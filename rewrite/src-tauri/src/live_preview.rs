//! Live preview: a tiny streaming recognizer that runs beside the primary
//! dictation model so the screen tab shows words while the user speaks.
//!
//! Display only. The preview text travels in its own `Event::Preview` and
//! `Snapshot::preview_text`; it never reaches the recognition result,
//! history, cleanup, the clipboard or paste, and an error never promotes it.
//!
//! Audio: the backlog producer offers each captured chunk to a bounded queue
//! (`Tap::offer`, `try_send`). Capture and the primary are never slowed; when
//! the preview falls behind, its chunks are dropped and the preview restarts at
//! the next audio it receives.
//!
//! Alignment: the primary reports how far its committed segments reach
//! (`asr::Stream::committed_seconds`). When that point advances, the preview
//! restarts its stream there and re-feeds the audio after it from a short
//! ring. Preview text therefore covers only speech the primary has not
//! committed yet, and the screen tab shows committed text followed by it.
//!
//! Cost: Moonshine Tiny Streaming (MIT, 34M parameters, 45 MB) in its own
//! below-normal-priority worker process, ONNX Runtime forced to one thread
//! (`MOONSHINE_ORT_SINGLE_THREAD`, set for every recognition worker), CPU only.
//! Resource leases: see `resource_manager::admit_live_preview`.
use crate::speech::Event;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const MODEL_ID: &str = "moonshine-tiny-streaming";
pub(crate) const MODEL_NAME: &str = "Moonshine Tiny Streaming";
/// Pinned on 2026-10-14 from the Moonshine CDN path compiled into the bundled
/// runtime (`tiny-streaming-en/quantized_26_08_21`, the same revision as the
/// Small and Medium streaming weights). MIT: Moonshine LICENSE, section 1.
pub(crate) const CDN_BASE: &str =
    "https://download.moonshine.ai/model/tiny-streaming-en/quantized_26_08_21";
pub(crate) const FILES: &[(&str, u64, &str)] = &[
    ("adapter.ort", 1_319_664, "22ecc949e146c49667fda28d102d4e30749a107dc88a396292aa8f277ef1347c"),
    ("cross_kv.ort", 1_287_544, "143a36667b8d05fd9d04e8c337b7ee121f37ef299aea6b3d82bdb3d3401950b4"),
    ("decoder_kv.ort", 32_583_720, "8852553f312adb6c9aa4d17418015049b30f412209ee569d336548c0044627de"),
    ("encoder.ort", 7_675_440, "a8414e1a5dedf9f2093d7680601dd8a9b0433e7020260eafe0e370ead91134ca"),
    ("frontend.model.ort", 23_344, "5121b561417b638afce0c6c31b760e37c93cf97f80d9b0031aad1fe7b6f25d61"),
    ("frontend.weights.ort", 2_093_464, "217da24ac6f522ebf02da8ef288e77d1ac68d50d4a6821433182e4fbf4204bbd"),
    ("streaming_config.json", 509, "74fe5ddebd63b17caf59e8a3b18c17547ff7bce1642050edbb1c3962674f8950"),
    ("tokenizer.bin", 249_974, "6884b35fd6377d4c4d32336a0bc152f36b64d1e45b6503683cdc238250a8472d"),
];

const RATE: u32 = 16_000;
/// Moonshine option: seconds of new audio between transcription passes.
pub(crate) const TRANSCRIPTION_INTERVAL: &str = "0.2";
/// About 5 s of 10 ms device callbacks; beyond that the preview drops audio.
const QUEUE_CHUNKS: usize = 512;
/// Audio is sent to the worker in batches of at least this much.
const SEND_SECONDS: f64 = 0.1;
/// Uncommitted audio kept for a restart. Older audio is shown by neither.
const RING_SECONDS: f64 = 45.0;
/// At most ten preview updates per second.
const EMIT_INTERVAL: Duration = Duration::from_millis(100);
/// Commit advances smaller than this do not restart the preview.
const COMMIT_EPSILON: f64 = 0.05;

static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// True while any preview worker may hold the model files open.
pub(crate) fn running() -> bool {
    RUNNING.load(Ordering::Acquire) > 0
}

/// Whether to start a preview beside `primary`. A primary that is itself a
/// Moonshine streaming model already shows live text, so it gets none.
pub(crate) fn wanted(root: &Path, enabled: bool, primary: &str) -> bool {
    enabled
        && cfg!(all(any(target_os = "windows", target_os = "linux"), target_arch = "x86_64"))
        && !matches!(primary, "moonshine-medium" | "moonshine-small" | MODEL_ID)
        && crate::asr::model_status(root, MODEL_ID, crate::model_verification::Check::Background)
            .ready()
        && crate::resource_manager::admit_live_preview()
}

/// Producer side, owned by the audio backlog worker. Never blocks.
pub(crate) struct Tap {
    tx: SyncSender<Chunk>,
    next_frame: u64,
}

struct Chunk {
    first_frame: u64,
    rate: u32,
    samples: Vec<f32>,
}

impl Tap {
    pub(crate) fn offer(&mut self, samples: &[f32], rate: u32) {
        let first_frame = self.next_frame;
        self.next_frame += samples.len() as u64;
        match self.tx.try_send(Chunk { first_frame, rate, samples: samples.to_vec() }) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

type Callback = Arc<dyn Fn(Event) + Send + Sync>;

struct Commit {
    /// Seconds on the capture timeline; `None` means "everything so far".
    seconds: Option<f64>,
    generation: u64,
}

struct Shared {
    commit: Mutex<Commit>,
    stop: AtomicBool,
}

/// Consumer side, owned by the dictation job. Dropping it stops the preview.
pub(crate) struct Preview {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
    callback: Callback,
    session_id: u64,
}

impl Preview {
    /// Start the preview thread; returns the tap to install in the backlog.
    pub(crate) fn start(
        root: PathBuf,
        session_id: u64,
        callback: Callback,
        cue: crate::audio::CueMask,
        cancel: Arc<AtomicBool>,
    ) -> Option<(Self, Tap)> {
        let (tx, rx) = mpsc::sync_channel(QUEUE_CHUNKS);
        let shared = Arc::new(Shared {
            commit: Mutex::new(Commit { seconds: Some(0.0), generation: 0 }),
            stop: AtomicBool::new(false),
        });
        let worker = shared.clone();
        let events = callback.clone();
        RUNNING.fetch_add(1, Ordering::AcqRel);
        let thread = thread::Builder::new()
            .name("dictation-live-preview".into())
            .spawn(move || {
                let _running = RunningGuard;
                run(&root, rx, &worker, &callback, session_id, &cue, &cancel);
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(_) => {
                RUNNING.fetch_sub(1, Ordering::AcqRel);
                return None;
            }
        };
        Some((
            Self { shared, thread: Some(thread), callback: events, session_id },
            Tap { tx, next_frame: 0 },
        ))
    }

    /// The primary's committed text now covers audio up to `seconds`.
    /// `publish` sends the primary's interim text while the commit lock is
    /// held. When the commit point advances, the old draft is cleared first
    /// and no draft for the older point can follow; otherwise it stays.
    pub(crate) fn commit(&self, seconds: Option<f32>, publish: impl FnOnce()) {
        let Ok(mut commit) = self.shared.commit.lock() else { return publish() };
        let seconds = seconds.map(f64::from);
        let advanced = match (commit.seconds, seconds) {
            (Some(old), Some(new)) => new > old + COMMIT_EPSILON,
            (_, None) => true,
            (None, Some(_)) => true,
        };
        if advanced {
            commit.seconds = seconds;
            commit.generation += 1;
            (self.callback)(Event::Preview { session_id: self.session_id, text: String::new() });
        }
        publish();
    }

    /// Stop decoding; the last preview text stays until the result replaces it.
    pub(crate) fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
        // The thread exits within one receive timeout; it owns the worker
        // process and drops it on exit. Do not wait for that here.
        drop(self.thread.take());
    }
}

struct RunningGuard;
impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Phase-continuous linear resampler to 16 kHz mono.
#[derive(Default)]
struct Resampler {
    rate: u32,
    position: f64,
    last: Option<f32>,
}

impl Resampler {
    fn push(&mut self, input: &[f32], rate: u32, output: &mut Vec<f32>) {
        if rate == 0 || input.is_empty() {
            return;
        }
        if rate != self.rate {
            // Position 1.0 is the first input sample; 0.0 is the previous
            // chunk's last sample.
            *self = Self { rate, position: 1.0, last: None };
        }
        let clean = |v: f32| if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
        if rate == RATE {
            output.extend(input.iter().map(|v| clean(*v)));
            return;
        }
        let step = f64::from(rate) / f64::from(RATE);
        let last = self.last.unwrap_or(input[0]);
        let at = |k: usize| clean(if k == 0 { last } else { input[k - 1] });
        let length = input.len() as f64;
        while self.position < length {
            let base = self.position.floor();
            let frac = (self.position - base) as f32;
            let (a, b) = (at(base as usize), at(base as usize + 1));
            output.push(a + (b - a) * frac);
            self.position += step;
        }
        self.position -= length;
        self.last = input.last().copied();
    }
}

/// Recent 16 kHz audio on the capture timeline.
struct Ring {
    samples: VecDeque<f32>,
    /// Capture-timeline seconds of the newest sample's end.
    end: f64,
}

impl Ring {
    fn start(&self) -> f64 {
        self.end - self.samples.len() as f64 / f64::from(RATE)
    }
    fn push(&mut self, samples: &[f32], end: f64) {
        self.samples.extend(samples);
        self.end = end;
        let limit = (RING_SECONDS * f64::from(RATE)) as usize;
        if self.samples.len() > limit {
            let excess = self.samples.len() - limit;
            self.samples.drain(..excess);
        }
    }
    fn clear_to(&mut self, end: f64) {
        self.samples.clear();
        self.end = end;
    }
    /// Samples from `from` seconds to the end.
    fn since(&self, from: f64) -> Vec<f32> {
        let skip = ((from - self.start()).max(0.0) * f64::from(RATE)).round() as usize;
        self.samples.iter().skip(skip).copied().collect()
    }
}

fn run(
    root: &Path,
    rx: Receiver<Chunk>,
    shared: &Shared,
    callback: &Callback,
    session_id: u64,
    cue: &crate::audio::CueMask,
    cancel: &AtomicBool,
) {
    let started = Instant::now();
    let Ok(mut recognizer) = crate::asr::Recognizer::load_background(root, MODEL_ID, cancel) else {
        crate::recognition_diagnostics::event("preview-load-failed", MODEL_ID, std::process::id(), session_id, started.elapsed().as_millis() as u64, 0);
        return;
    };
    crate::recognition_diagnostics::event("preview-ready", MODEL_ID, std::process::id(), session_id, started.elapsed().as_millis() as u64, 0);
    let mut resampler = Resampler::default();
    let mut ring = Ring { samples: VecDeque::new(), end: 0.0 };
    let mut expected_frame = 0u64;
    let mut generation = 0u64;
    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut converted = Vec::new();
    // Capture-timeline seconds of the last audio sent to the current stream.
    let mut fed = 0.0f64;
    // One stream per commit point. The worker stays loaded between them.
    'streams: loop {
        let Ok(mut stream) = recognizer.stream_interruptible(cancel) else { break };
        let mut shown = String::new();
        let mut pending: Option<String> = None;
        let mut restart = false;
        // Capture ended: send the last audio and show its text, then stop.
        let mut draining = false;
        let ended = loop {
            if cancel.load(Ordering::Acquire) || shared.stop.load(Ordering::Acquire) {
                break true;
            }
            let first = match rx.recv_timeout(Duration::from_millis(30)) {
                Ok(chunk) => Some(chunk),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    draining = true;
                    None
                }
            };
            for mut chunk in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                if chunk.rate == 0 {
                    continue;
                }
                let rate = f64::from(chunk.rate);
                if chunk.first_frame != expected_frame {
                    // Dropped chunks: what the stream heard is no longer contiguous.
                    ring.clear_to(chunk.first_frame as f64 / rate);
                    resampler = Resampler::default();
                    restart = true;
                }
                expected_frame = chunk.first_frame + chunk.samples.len() as u64;
                cue.apply(&mut chunk.samples, chunk.first_frame);
                converted.clear();
                resampler.push(&chunk.samples, chunk.rate, &mut converted);
                ring.push(&converted, expected_frame as f64 / rate);
            }
            let Ok(commit) = shared.commit.lock() else { break true };
            if commit.generation != generation || restart {
                generation = commit.generation;
                // A primary ahead of the preview waits for audio past its commit.
                fed = commit.seconds.unwrap_or(ring.end).max(ring.start());
                break false;
            }
            drop(commit);
            if ring.end - fed >= SEND_SECONDS || (draining && ring.end > fed) {
                let audio = ring.since(fed);
                fed = ring.end;
                if stream.add(&audio, RATE).is_err() || stream.failed() {
                    break true;
                }
                match stream.update() {
                    Ok(text) if text != shown => pending = Some(text),
                    Ok(_) => {}
                    Err(_) => break true,
                }
            }
            if pending.is_some() && (draining || last_emit.elapsed() >= EMIT_INTERVAL) {
                let Ok(commit) = shared.commit.lock() else { break true };
                if commit.generation == generation {
                    let text = pending.take().unwrap_or_default();
                    shown.clone_from(&text);
                    last_emit = Instant::now();
                    callback(Event::Preview { session_id, text });
                }
            }
            if draining {
                break true;
            }
        };
        // Finish (not drop): dropping an unfinished remote stream ends the
        // worker; a finished one lets it exit through Shutdown or restart.
        let finished = stream.finish_recognition().is_ok();
        if ended || !finished {
            break 'streams;
        }
    }
    drop(recognizer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_keeps_phase_across_chunks() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0) - 0.5).collect();
        let mut whole = Vec::new();
        Resampler::default().push(&input, 48_000, &mut whole);
        let mut pieces = Vec::new();
        let mut resampler = Resampler::default();
        for chunk in input.chunks(441) {
            resampler.push(chunk, 48_000, &mut pieces);
        }
        assert!((whole.len() as i64 - 16_000).abs() <= 1, "{}", whole.len());
        assert_eq!(whole.len(), pieces.len());
        for (a, b) in whole.iter().zip(&pieces) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn ring_slices_by_capture_time_and_forgets_old_audio() {
        let mut ring = Ring { samples: VecDeque::new(), end: 0.0 };
        ring.push(&vec![0.0; 16_000], 1.0);
        ring.push(&vec![1.0; 16_000], 2.0);
        assert_eq!(ring.since(1.5).len(), 8_000);
        assert!(ring.since(1.5).iter().all(|v| *v == 1.0));
        assert_eq!(ring.since(-3.0).len(), 32_000);
        ring.push(&vec![0.5; (RING_SECONDS as usize) * 16_000], 2.0 + RING_SECONDS);
        assert!((ring.start() - 2.0).abs() < 1e-6);
    }

    #[test]
    fn tap_never_blocks_and_counts_dropped_frames() {
        let (tx, rx) = mpsc::sync_channel(1);
        let mut tap = Tap { tx, next_frame: 0 };
        tap.offer(&[0.0; 10], 16_000);
        tap.offer(&[0.0; 10], 16_000); // full: dropped, still counted
        tap.offer(&[0.0; 10], 16_000);
        assert_eq!(rx.try_recv().unwrap().first_frame, 0);
        assert!(rx.try_recv().is_err());
        tap.offer(&[0.0; 10], 16_000);
        assert_eq!(rx.try_recv().unwrap().first_frame, 30);
    }

    #[test]
    fn commits_publish_in_order_and_only_advances_restart() {
        let shared = Arc::new(Shared {
            commit: Mutex::new(Commit { seconds: Some(0.0), generation: 0 }),
            stop: AtomicBool::new(false),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let callback: Callback = Arc::new(move |event| {
            if let Event::Preview { text, .. } = event {
                sink.lock().unwrap().push(format!("draft:{text}"));
            }
        });
        let preview = Preview { shared: shared.clone(), thread: None, callback, session_id: 3 };
        let interim = |label: &'static str| {
            let events = events.clone();
            move || events.lock().unwrap().push(label.to_string())
        };
        preview.commit(Some(0.01), interim("same point"));
        assert_eq!(shared.commit.lock().unwrap().generation, 0);
        preview.commit(Some(2.0), interim("advanced"));
        preview.commit(None, interim("unknown point"));
        assert_eq!(shared.commit.lock().unwrap().generation, 2);
        // An unmoved commit keeps the draft; an advance clears it before the
        // newly committed text is published.
        assert_eq!(
            *events.lock().unwrap(),
            ["same point", "draft:", "advanced", "draft:", "unknown point"]
        );
    }

    #[test]
    fn moonshine_primaries_and_disabled_setting_get_no_preview() {
        let root = std::env::temp_dir().join("sonojot-live-preview-none");
        for primary in ["moonshine-medium", "moonshine-small", MODEL_ID] {
            assert!(!wanted(&root, true, primary));
        }
        assert!(!wanted(&root, false, "qwen3-asr-1.7b-q8-cuda"));
        // Not installed: everything behaves as before.
        assert!(!wanted(&root, true, "qwen3-asr-1.7b-q8-cuda"));
    }

    #[test]
    fn pinned_files_match_the_moonshine_bundle_layout() {
        let names: Vec<_> = FILES.iter().map(|(name, _, _)| *name).collect();
        for required in ["encoder.ort", "adapter.ort", "cross_kv.ort", "decoder_kv.ort", "streaming_config.json", "tokenizer.bin", "frontend.model.ort", "frontend.weights.ort"] {
            assert!(names.contains(&required), "{required}");
        }
        assert!(FILES.iter().all(|(_, bytes, sha)| *bytes > 0 && sha.len() == 64));
        let total: u64 = FILES.iter().map(|(_, bytes, _)| bytes).sum();
        assert_eq!(total, 45_233_659);
    }

    /// Opt-in real-audio bench. Needs a debug `sonojot` binary beside the
    /// test executable (`cargo build`), a models root whose runtime and
    /// `optional/moonshine-tiny-streaming` are installed, and a mono WAV.
    /// Feeds the WAV in real time, 10 ms per chunk, and writes every preview
    /// update plus the worker's peak working set and CPU time as JSON.
    /// SONOJOT_PREVIEW_BENCH_COMMIT_EVERY=<seconds> also simulates a primary
    /// committing everything older than one second at that interval.
    #[test]
    #[ignore = "opt-in: SONOJOT_PREVIEW_BENCH_ROOT, _WAV and _OUT"]
    fn bench_real_audio() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
        let root = PathBuf::from(var("SONOJOT_PREVIEW_BENCH_ROOT"));
        let mut reader = hound::WavReader::open(var("SONOJOT_PREVIEW_BENCH_WAV")).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "mono WAV");
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader.samples::<i32>().map(|v| v.unwrap() as f32 / scale).collect()
            }
            hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        };
        let commit_every: Option<f64> = std::env::var("SONOJOT_PREVIEW_BENCH_COMMIT_EVERY").ok().and_then(|v| v.parse().ok());
        let rate = spec.sample_rate;
        let started = Instant::now();
        let events = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let sink = events.clone();
        let callback: Callback = Arc::new(move |event| {
            if let Event::Preview { text, .. } = event {
                sink.lock().unwrap().push(serde_json::json!({"wall": started.elapsed().as_secs_f64(), "text": text}));
            }
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let (preview, mut tap) = Preview::start(root, 1, callback, crate::audio::CueMask::none(), cancel.clone()).unwrap();
        let chunk = (rate / 100) as usize;
        let mut next_commit = commit_every;
        let mut commits = Vec::new();
        // Optional real primary in its own worker, driven like speech.rs:
        // update every 650 ms and commit when its text changes.
        let primary_id = std::env::var("SONOJOT_PREVIEW_BENCH_PRIMARY").ok();
        let mut primary = primary_id.as_deref().map(|id| {
            crate::asr::Recognizer::load(&PathBuf::from(var("SONOJOT_PREVIEW_BENCH_ROOT")), id).unwrap()
        });
        let mut primary_stream = primary.as_mut().map(|recognizer| recognizer.stream().unwrap());
        let mut interims = Vec::new();
        let mut last_interim = String::new();
        let mut last_update = Instant::now();
        for (index, piece) in samples.chunks(chunk).enumerate() {
            let due = started + Duration::from_secs_f64(index as f64 * chunk as f64 / f64::from(rate));
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            tap.offer(piece, rate);
            if let Some(stream) = primary_stream.as_mut() {
                stream.add(piece, rate).unwrap();
                if last_update.elapsed() >= Duration::from_millis(650) {
                    last_update = Instant::now();
                    let text = stream.update().unwrap();
                    if text != last_interim {
                        let committed = stream.committed_seconds();
                        let at = started.elapsed().as_secs_f64();
                        preview.commit(committed, || {});
                        interims.push(serde_json::json!({"wall": at, "text": text, "committed": committed}));
                        last_interim = text;
                    }
                }
            }
            let audio = (index + 1) as f64 * chunk as f64 / f64::from(rate);
            if let Some(at) = next_commit.filter(|at| audio >= *at) {
                preview.commit(Some((audio - 1.0) as f32), || {});
                commits.push(audio - 1.0);
                next_commit = Some(at + commit_every.unwrap());
            }
        }
        thread::sleep(Duration::from_secs(2));
        let query = format!(
            "Get-CimInstance Win32_Process -Filter 'ParentProcessId={}' | Select-Object ProcessId,Name,PeakWorkingSetSize,WorkingSetSize,PeakPageFileUsage,KernelModeTime,UserModeTime | ConvertTo-Json",
            std::process::id()
        );
        let workers = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &query]).output().unwrap();
        let wall = started.elapsed().as_secs_f64();
        drop(tap);
        drop(preview);
        if let Some(stream) = primary_stream.take() {
            let _ = stream.finish_recognition();
        }
        thread::sleep(Duration::from_secs(1));
        let report = serde_json::json!({
            "audio_seconds": samples.len() as f64 / f64::from(rate),
            "wall_seconds_at_sample": wall,
            "sample_rate": rate,
            "commits": commits,
            "primary": primary_id,
            "interims": interims,
            "workers": serde_json::from_slice::<serde_json::Value>(&workers.stdout).unwrap_or(serde_json::Value::Null),
            "events": *events.lock().unwrap(),
        });
        std::fs::write(var("SONOJOT_PREVIEW_BENCH_OUT"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        assert!(!events.lock().unwrap().is_empty(), "the preview produced no text");
    }
}
