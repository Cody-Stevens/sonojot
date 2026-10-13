//! One-shot diarization IPC. Only the child enters the ONNX-backed engine.
use crate::{
    diarization::Turn,
    store::{atomic_write, new_id, read_json, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    version: u32,
    turns: Vec<Turn>,
}

/// Dispatched before Tauri, including when launched by a Rust test harness.
pub fn dispatch_worker() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--diarization-worker")) {
        return false;
    }
    let paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if paths.len() != 3 {
        eprintln!("Diarization worker requires models, audio and output paths.");
        std::process::exit(2);
    }
    let result = crate::diarization::diarize_file(&paths[0], &paths[1]).and_then(|turns| {
        let bytes = serde_json::to_vec(&Output { version: 1, turns })
            .map_err(|e| format!("Cannot encode speaker labels: {e}"))?;
        atomic_write(&paths[2], &bytes)
    });
    if let Err(error) = &result {
        // The parent never needs to drain a stderr pipe while native code runs.
        let message: String = error
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(2048)
            .collect();
        let _ = atomic_write(&paths[2].with_extension("error"), message.as_bytes());
    }
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

struct TemporaryOutput(PathBuf);
impl TemporaryOutput {
    fn new(store: &Store) -> Result<Self, String> {
        let area = store.root().join("temp");
        fs::create_dir_all(&area).map_err(|e| format!("Cannot create meeting temp area: {e}"))?;
        store.check_owned(&area)?;
        let directory = area.join(format!("diarization-{}", new_id()));
        fs::create_dir(&directory)
            .map_err(|e| format!("Cannot create speaker label temp area: {e}"))?;
        Ok(Self(directory))
    }
    fn output(&self) -> PathBuf {
        self.0.join("out.json")
    }
}
impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        // Also removes an interrupted atomic write, inside this job's private folder.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validate_output(output: Output, seconds: f64) -> Result<Vec<Turn>, String> {
    if output.version != 1
        || output.turns.iter().any(|turn| {
            !turn.start.is_finite()
                || !turn.end.is_finite()
                || turn.start < 0.0
                || turn.end <= turn.start
                || turn.end > seconds + 0.02
                || turn.speaker >= 8
        })
        || output
            .turns
            .windows(2)
            .any(|pair| (pair[0].start, pair[0].speaker) > (pair[1].start, pair[1].speaker))
    {
        return Err("Invalid speaker labels from diarization worker.".into());
    }
    Ok(output.turns)
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
fn wait_for_worker(
    child: &mut crate::recognition::OwnedChild,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<std::process::ExitStatus, String> {
    loop {
        crate::ai::check_cancel(cancel)?;
        if Instant::now() >= deadline {
            return Err("Speaker labeling exceeded its time limit.".into());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("Cannot wait for speaker labeling: {e}"))?
        {
            crate::ai::check_cancel(cancel)?;
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(crate) fn run(
    models: &Path,
    audio: &Path,
    store: &Store,
    cancel: &AtomicBool,
) -> Result<Vec<Turn>, String> {
    crate::ai::check_cancel(cancel)?;
    #[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
    {
        let _ = (models, audio, store);
        Err("Speaker labeling is not available on this platform.".into())
    }
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    {
        let seconds = crate::recording::wav_duration(audio)?;
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("Invalid audio duration for speaker labeling.".into());
        }
        // Declaration order matters: the child is killed before its output is deleted.
        let temporary = TemporaryOutput::new(store)?;
        let output = temporary.output();
        let mut command = Command::new(crate::recognition::worker_executable()?);
        command
            .arg("--diarization-worker")
            .arg(models)
            .arg(audio)
            .arg(&output);
        let mut child = crate::recognition::OwnedChild::spawn(&mut command)?;
        let status = wait_for_worker(
            &mut child,
            cancel,
            Instant::now() + Duration::from_secs_f64(120.0 + 0.1 * seconds),
        )?;
        if !status.success() {
            let message = fs::read_to_string(output.with_extension("error"))
                .ok()
                .map(|text| text.chars().take(2048).collect::<String>())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "Diarization worker stopped without speaker labels.".into());
            return Err(message);
        }
        let turns = validate_output(read_json(&output)?, seconds)?;
        crate::ai::check_cancel(cancel)?;
        Ok(turns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_worker_results() {
        let output = |version, start, end, speaker| Output {
            version,
            turns: vec![Turn {
                start,
                end,
                speaker,
            }],
        };
        assert!(validate_output(output(1, 0.0, 1.01, 7), 1.0).is_ok());
        for bad in [
            output(2, 0.0, 1.0, 0),
            output(1, -0.01, 1.0, 0),
            output(1, 1.0, 0.0, 0),
            output(1, 0.0, f64::NAN, 0),
            output(1, 0.0, 2.0, 0),
            output(1, 0.0, 1.0, 8),
        ] {
            assert!(validate_output(bad, 1.0).is_err());
        }
    }

    #[test]
    fn temp_outputs_are_removed_with_partial_writes() {
        let root = std::env::temp_dir().join(format!("sonojot-diar-temp-{}", new_id()));
        let store = Store::open(Some(root.clone())).unwrap();
        let path;
        {
            let temporary = TemporaryOutput::new(&store).unwrap();
            path = temporary.0.clone();
            fs::write(temporary.output(), b"{}").unwrap();
            fs::write(path.join(".interrupted.tmp"), b"partial").unwrap();
        }
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn worker_wait_is_cancelable_and_has_a_deadline() {
        use std::sync::atomic::Ordering;
        for cancel_now in [true, false] {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]);
            let mut child = crate::recognition::OwnedChild::spawn(&mut command).unwrap();
            let cancel = AtomicBool::new(false);
            cancel.store(cancel_now, Ordering::Release);
            let started = Instant::now();
            assert!(
                wait_for_worker(&mut child, &cancel, started + Duration::from_millis(100)).is_err()
            );
            child.terminate();
            assert!(child.try_wait().unwrap().is_some());
            assert!(started.elapsed() < Duration::from_secs(3));
        }
    }

    /// Real worker parity; no saved meeting or installed application is changed.
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    #[ignore]
    fn real_worker_matches_python_activity() {
        #[derive(Deserialize)]
        struct Golden {
            #[serde(rename = "Start")]
            start: f64,
            #[serde(rename = "End")]
            end: f64,
            #[serde(rename = "Speaker")]
            speaker: usize,
        }
        let wav = PathBuf::from(std::env::var_os("SONOJOT_DIAR_WAV").expect("SONOJOT_DIAR_WAV"));
        let golden =
            PathBuf::from(std::env::var_os("SONOJOT_DIAR_GOLDEN").expect("SONOJOT_DIAR_GOLDEN"));
        let models =
            PathBuf::from(std::env::var_os("EAVESDROPPER_MODELS").expect("EAVESDROPPER_MODELS"));
        let golden: Vec<Golden> = read_json(&golden).unwrap();
        let root = std::env::temp_dir().join(format!("sonojot-diar-parity-{}", new_id()));
        let store = Store::open(Some(root.clone())).unwrap();
        let started = Instant::now();
        let actual = run(&models, &wav, &store, &AtomicBool::new(false));
        fs::remove_dir_all(root).unwrap();
        let actual = actual.unwrap();
        let seconds = crate::recording::wav_duration(&wav).unwrap();
        let frames = 1 + (seconds * 100.0).floor() as usize;
        let mut expected = vec![[false; 8]; frames];
        let mut observed = expected.clone();
        for (start, end, speaker) in golden.iter().map(|t| (t.start, t.end, t.speaker)) {
            for frame in
                &mut expected[(start * 100.0).round() as usize..(end * 100.0).round() as usize]
            {
                frame[speaker] = true;
            }
        }
        for turn in &actual {
            for frame in &mut observed
                [(turn.start * 100.0).round() as usize..(turn.end * 100.0).round() as usize]
            {
                frame[turn.speaker] = true;
            }
        }
        let mut equal = [0usize; 8];
        for (expected, observed) in expected.iter().zip(&observed) {
            for speaker in 0..8 {
                equal[speaker] += usize::from(expected[speaker] == observed[speaker]);
            }
        }
        let agreement = equal.iter().sum::<usize>() as f64 / (frames * 8) as f64;
        eprintln!(
            "{}: {:.6}% agreement, {} frames x 8 speakers, {} turns, {:.2}s; per speaker {:?}",
            wav.display(),
            agreement * 100.0,
            frames,
            actual.len(),
            started.elapsed().as_secs_f64(),
            equal.map(|count| 100.0 * count as f64 / frames as f64)
        );
        assert!(
            agreement >= 0.995,
            "Frame agreement below 99.5%: {agreement}"
        );
    }
}
