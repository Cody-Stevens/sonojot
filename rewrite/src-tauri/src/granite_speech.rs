//! Meeting-only adapter. The meeting owns the ASR lease and the worker; raw
//! recognition reaches the same explicit transcript-cleanup action as Qwen.
use crate::asr::{RecognitionResult, Segment};
use crate::domain::RecognitionCompleteness;
use crate::granite_worker::GraniteWorker;
use std::{path::Path, sync::atomic::AtomicBool};

pub(crate) fn is_granite(model: &str) -> bool {
    model == crate::granite_model::GRANITE_ID
}

pub(crate) fn transcribe_file(
    worker: &mut GraniteWorker,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<RecognitionResult, String> {
    transcribe_with(path, cancel, |samples| worker.transcribe(samples, cancel))
}

fn transcribe_with(
    path: &Path,
    cancel: &AtomicBool,
    mut recognize: impl FnMut(&[f32]) -> Result<String, String>,
) -> Result<RecognitionResult, String> {
    let mut result = RecognitionResult {
        text: String::new(),
        original_text: None,
        segments: Vec::new(),
        completeness: RecognitionCompleteness::Complete,
        warnings: Vec::new(),
    };
    for_sections(path, cancel, |samples, start, end| {
        crate::ai::check_cancel(cancel)?;
        if samples.iter().all(|sample| sample.abs() <= 0.000_001) {
            return Ok(());
        }
        let text = recognize_bounded(samples, cancel, &mut recognize)?;
        crate::ai::check_cancel(cancel)?;
        crate::store::validate_text(&text, "Transcript section")?;
        if result
            .text
            .len()
            .saturating_add(text.len())
            .saturating_add(1)
            > crate::store::MAX_TEXT_BYTES
        {
            return Err("The Granite transcript reached its saved-text limit.".into());
        }
        if !text.trim().is_empty() {
            if !result.text.is_empty() {
                result.text.push(' ');
            }
            result.text.push_str(&text);
            result.segments.push(Segment {
                id: result.segments.len() as u64,
                start_seconds: start as f32 / 16_000.0,
                end_seconds: Some(end as f32 / 16_000.0),
                speaker: None,
                text,
                complete: true,
            });
        }
        Ok(())
    })?;
    Ok(result)
}

// NAR can reject unusually dense input below its advisory 384-second ceiling.
// Retry only that explicit failure; crashes and cancellation fail the job.
fn recognize_bounded(
    samples: &[f32],
    cancel: &AtomicBool,
    recognize: &mut impl FnMut(&[f32]) -> Result<String, String>,
) -> Result<String, String> {
    crate::ai::check_cancel(cancel)?;
    match recognize(samples) {
        Ok(text) => Ok(text),
        Err(error) if samples.len() > 16_000 && input_too_long(&error) => {
            crate::ai::check_cancel(cancel)?;
            let middle = samples.len() / 2;
            let first = recognize_bounded(&samples[..middle], cancel, recognize)?;
            let second = recognize_bounded(&samples[middle..], cancel, recognize)?;
            Ok([first, second]
                .into_iter()
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" "))
        }
        Err(error) => Err(error),
    }
}

fn input_too_long(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("input too long")
        || error.contains("audio too long")
        || error.contains("context length")
        || error.contains("context limit")
}

/// Reuse Qwen's bounded window buffer and detector with its spectral fallback.
/// Carry every original sample across cuts instead of trimming by VAD evidence:
/// a spectral detector can miss quiet words within otherwise clear speech.
/// Pauses choose boundaries; they never authorize dropping source audio. The
/// 20-second cap and source offsets also apply in fidelity-eval builds.
#[cfg(any(windows, target_os = "macos", test))]
fn for_sections(
    path: &Path,
    cancel: &AtomicBool,
    mut visit: impl FnMut(&[f32], u64, u64) -> Result<(), String>,
) -> Result<(), String> {
    use crate::qwen_window::{Assembler, Detector, HARD, RATE};
    let mut windows = Assembler::new(20);
    let mut detector = Detector::default();
    let mut frame = Vec::with_capacity(512);
    let mut offset = 0u64;
    let mut accept = |frame: &[f32]| -> Result<(), String> {
        crate::ai::check_cancel(cancel)?;
        windows.append(frame, frame)?;
        let positive =
            crate::mac_speech::speech_gate::fallback(frame, Some(cancel))?.positive_samples > 0;
        let pause = detector
            .observe(if positive { 1.0 } else { 0.0 }, frame.len())
            .is_some();
        let pending = windows.end() - offset;
        if (pause && pending >= 3 * RATE as u64) || pending >= HARD {
            let end = windows.end().min(offset + HARD);
            let window = windows.exact(offset, end)?;
            visit(&window.audio, offset, end)?;
            offset = end;
        }
        Ok(())
    };
    crate::meeting_speech::for_mono_seconds(path, cancel, |samples| {
        for &sample in samples {
            frame.push(sample);
            if frame.len() == 512 {
                accept(&frame)?;
                frame.clear();
            }
        }
        Ok(true)
    })?;
    if !frame.is_empty() {
        accept(&frame)?;
    }
    drop(accept);
    if windows.end() > offset {
        crate::ai::check_cancel(cancel)?;
        let window = windows.exact(offset, windows.end())?;
        visit(&window.audio, offset, window.end)?;
    }
    crate::ai::check_cancel(cancel)
}

#[cfg(not(any(windows, target_os = "macos", test)))]
fn for_sections(
    _: &Path,
    _: &AtomicBool,
    _: impl FnMut(&[f32], u64, u64) -> Result<(), String>,
) -> Result<(), String> {
    Err("Granite Speech requires Windows NVIDIA CUDA or Apple Silicon Metal.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn recording(seconds: usize) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("granite-sections-{}.wav", uuid::Uuid::new_v4()));
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..seconds * 16_000 {
            // Speech-like audio and a pause exercise carried boundary samples.
            let sample = if (21 * 16_000..24 * 16_000).contains(&i) {
                0
            } else {
                (6000.0 * (i as f32 * std::f32::consts::TAU * 300.0 / 16_000.0).sin()) as i16
            };
            wav.write_sample(sample).unwrap();
        }
        wav.finalize().unwrap();
        path
    }

    #[test]
    fn section_boundaries_preserve_every_original_sample() {
        let path = recording(45);
        let source: Vec<f32> = hound::WavReader::open(&path)
            .unwrap()
            .samples::<i16>()
            .map(|sample| sample.unwrap() as f32 / 32768.0)
            .collect();
        let mut retained = Vec::new();
        for_sections(&path, &AtomicBool::new(false), |samples, start, end| {
            assert_eq!(start, retained.len() as u64);
            assert_eq!(end - start, samples.len() as u64);
            assert!(samples.len() <= 20 * 16_000);
            retained.extend_from_slice(samples);
            Ok(())
        })
        .unwrap();
        assert_eq!(retained, source);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn chunks_keep_source_times_and_raw_text_without_automatic_cleanup() {
        let path = recording(45);
        let mut lengths = Vec::new();
        let result = transcribe_with(&path, &AtomicBool::new(false), |samples| {
            lengths.push(samples.len());
            Ok("raw lowercase words".into())
        })
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(lengths.len() >= 3);
        assert!(lengths.iter().all(|len| *len <= 20 * 16_000));
        assert_eq!(result.original_text, None);
        assert!(result.text.starts_with("raw lowercase words"));
        assert!(result.segments.last().unwrap().end_seconds.unwrap() > 44.0);
        for pair in result.segments.windows(2) {
            assert!(pair[0].end_seconds.unwrap() <= pair[1].start_seconds);
        }
    }

    #[test]
    fn cancellation_and_worker_crash_never_publish_partial_transcripts() {
        let path = recording(45);
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result = transcribe_with(&path, &cancel, |_| {
            calls += 1;
            if calls == 1 {
                Ok("first chunk".into())
            } else {
                Err("Granite worker exited unexpectedly.".into())
            }
        });
        assert!(result.unwrap_err().contains("worker exited"));
        let result = transcribe_with(&path, &cancel, |_| {
            cancel.store(true, Ordering::Release);
            Ok("must not publish".into())
        });
        assert!(result.unwrap_err().contains("cancelled"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn dense_audio_retries_smaller_sections_but_crashes_do_not_retry() {
        let cancel = AtomicBool::new(false);
        let mut sizes = Vec::new();
        let result = recognize_bounded(&vec![0.1; 4 * 16_000], &cancel, &mut |samples| {
            sizes.push(samples.len());
            if samples.len() > 2 * 16_000 {
                Err("input too long".into())
            } else {
                Ok("words".into())
            }
        })
        .unwrap();
        assert_eq!(result, "words words");
        assert_eq!(sizes, [64_000, 32_000, 32_000]);
        let mut calls = 0;
        assert!(
            recognize_bounded(&vec![0.1; 4 * 16_000], &cancel, &mut |_| {
                calls += 1;
                Err("worker crashed".into())
            })
            .is_err()
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn meeting_dispatch_uses_only_the_exact_granite_id() {
        assert!(is_granite(crate::granite_model::GRANITE_ID));
        for id in [
            crate::llama_asr::MODEL_ID,
            "moonshine-medium",
            "whisper-tiny",
            "openrouter",
            "granite",
        ] {
            assert!(!is_granite(id));
        }
    }
}
