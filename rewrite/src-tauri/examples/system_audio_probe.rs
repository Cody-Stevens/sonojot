//! Explicit system-audio capture check. Never opens the microphone, settings,
//! meeting library or dictation history; writes a file only for `--tone`.
//!
//! Usage (from rewrite/src-tauri):
//!   cargo run --example system_audio_probe -- status
//!   cargo run --example system_audio_probe -- request
//!   cargo run --example system_audio_probe -- tone /tmp/tone.wav 8
//!   cargo run --example system_audio_probe -- capture 4 [--force]
//!
//! `capture` prints one JSON line and exits 0 only when non-silent audio
//! arrived. Play something (for example `afplay /tmp/tone.wav`) meanwhile.
//! `--force` skips the permission check to observe what an unpermitted tap
//! delivers on macOS.
use sonojot_core::system_audio;
use std::{
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Stats {
    chunks: u64,
    frames: u64,
    sum_squares: f64,
    peak: f32,
    nonzero: u64,
    discontinuities: u64,
    sample_rate: u32,
    first_at: Option<Duration>,
    end_at: Duration,
}

fn write_tone(path: &str, seconds: u32) -> std::io::Result<()> {
    let rate = 48_000u32;
    let frames = rate * seconds;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 2);
    let data = frames * 2;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for frame in 0..frames {
        let phase = frame as f64 * 440.0 * std::f64::consts::TAU / rate as f64;
        let value = (phase.sin() * 0.3 * i16::MAX as f64) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("status") => println!("{{\"status\":\"{:?}\"}}", system_audio::status()),
        Some("request") => {
            let before = system_audio::request_permission();
            std::thread::sleep(Duration::from_secs(3));
            println!(
                "{{\"before\":\"{before:?}\",\"after\":\"{:?}\"}}",
                system_audio::status()
            );
        }
        Some("tone") => {
            let path = args.get(1).expect("Pass the WAV path");
            let seconds = args.get(2).and_then(|value| value.parse().ok()).unwrap_or(8);
            write_tone(path, seconds).expect("Cannot write the tone");
        }
        Some("capture") => {
            let seconds: f64 = args.get(1).and_then(|value| value.parse().ok()).unwrap_or(4.0);
            let force = args.iter().any(|arg| arg == "--force");
            let status = system_audio::status();
            let stats = Arc::new(Mutex::new(Stats::default()));
            let sink = stats.clone();
            let (errors, error_rx) = mpsc::channel();
            let epoch = Instant::now();
            let on_chunk = move |chunk: system_audio::SystemAudioChunk| {
                let mut stats = sink.lock().unwrap();
                stats.chunks += 1;
                stats.frames += chunk.samples.len() as u64;
                stats.sample_rate = chunk.sample_rate;
                stats.discontinuities += u64::from(chunk.discontinuity);
                stats.first_at.get_or_insert(chunk.captured_at);
                stats.end_at = chunk.captured_at
                    + Duration::from_secs_f64(chunk.samples.len() as f64 / chunk.sample_rate as f64);
                for value in chunk.samples {
                    stats.sum_squares += f64::from(value * value);
                    stats.peak = stats.peak.max(value.abs());
                    stats.nonzero += u64::from(value != 0.0);
                }
            };
            let on_error = move |error: String| {
                let _ = errors.send(error);
            };
            let started = if force {
                system_audio::SystemAudioCapture::start_ignoring_permission(epoch, on_chunk, on_error)
            } else {
                system_audio::SystemAudioCapture::start(epoch, on_chunk, on_error)
            };
            let capture = match started {
                Ok(capture) => capture,
                Err(error) => {
                    println!(
                        "{{\"status\":\"{status:?}\",\"started\":false,\"error\":{}}}",
                        serde_json::Value::from(error)
                    );
                    std::process::exit(3);
                }
            };
            std::thread::sleep(Duration::from_secs_f64(seconds));
            let stopped = capture.stop();
            let stats = stats.lock().unwrap();
            let rms = if stats.frames == 0 {
                0.0
            } else {
                (stats.sum_squares / stats.frames as f64).sqrt()
            };
            let covered = stats
                .first_at
                .map_or(0.0, |first| stats.end_at.saturating_sub(first).as_secs_f64());
            let audio_seconds = if stats.sample_rate == 0 {
                0.0
            } else {
                stats.frames as f64 / stats.sample_rate as f64
            };
            let errors: Vec<String> = error_rx.try_iter().collect();
            println!(
                "{}",
                serde_json::json!({
                    "status": format!("{status:?}"),
                    "forced": force,
                    "started": true,
                    "stop": stopped.err(),
                    "errors": errors,
                    "sampleRate": stats.sample_rate,
                    "chunks": stats.chunks,
                    "audioSeconds": audio_seconds,
                    "timelineSeconds": covered,
                    "firstAtMs": stats.first_at.map(|at| at.as_millis() as u64),
                    "nonzeroSamples": stats.nonzero,
                    "rms": rms,
                    "peak": stats.peak,
                    "discontinuities": stats.discontinuities,
                })
            );
            std::process::exit(if stats.peak > 0.01 { 0 } else { 2 });
        }
        _ => {
            eprintln!("Usage: system_audio_probe status | request | tone <path> [seconds] | capture [seconds] [--force]");
            std::process::exit(64);
        }
    }
}
