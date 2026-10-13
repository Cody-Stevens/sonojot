//! Native GPU validation probe (isolated temp roots only).
//!
//! GPU-worker owned. Drives the real app adapter path
//! (`asr::Recognizer` -> `gpu_asr::Engine`/`Stream` for GPU IDs,
//! `sherpa::Engine` for CPU IDs) against an isolated models root.
//! Never touches the daily app, settings, or live model root.
//!
//! Usage:
//!   gpu_native_probe <models_root> <wav> <expected_txt> <mode>
//! Modes: tiny | qwen | switch | cancel | ready
//!
//! - tiny/qwen: load, decode once, print transcript + timings + VRAM.
//! - switch: CPU parakeet -> GPU tiny -> GPU qwen -> CPU parakeet in ONE
//!   process, proving clean DLL handoff with no misreported provider.
//! - cancel: pre-cancelled + unknown-id fail-clear checks.
//! - ready: readiness + requested-provider labels without loading.
//!
//! All output is one JSON object on stdout.

use sonojot_core::asr;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn vram_used_mib() -> Option<u64> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()?
        .trim()
        .split([',', ' '])
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn expected_text(path: &Path, wav: &Path) -> String {
    // trans.txt holds "<file> <TEXT>" lines; match the wav file name token.
    let file = wav.file_name().and_then(|s| s.to_str()).unwrap_or("0.wav");
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for line in content.lines() {
        let line = line.trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        if parts.next() == Some(file) {
            return parts.next().unwrap_or("").trim().to_owned();
        }
    }
    content.trim().to_owned()
}

struct VramSampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<u64>>>,
}

impl VramSampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("gpu-native-vram-poll".into())
            .spawn(move || {
                let mut samples = Vec::new();
                while !flag.load(Ordering::Acquire) {
                    if let Some(mib) = vram_used_mib() {
                        samples.push(mib);
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                samples
            })
            .ok();
        Self { stop, handle }
    }
    fn finish(self) -> Vec<u64> {
        self.stop.store(true, Ordering::Release);
        self.handle.and_then(|h| h.join().ok()).unwrap_or_default()
    }
}

fn decode_once(root: &Path, model: &str, wav: &Path) -> serde_json::Value {
    let idle_before = vram_used_mib();
    let t_load = Instant::now();
    let load = asr::Recognizer::load(root, model);
    let load_ms = t_load.elapsed().as_millis();
    let mut recognizer = match load {
        Ok(r) => r,
        Err(error) => {
            return serde_json::json!({
                "model": model, "loaded": false, "error": error,
                "providerRequested": asr::provider_requested(model),
                "vramIdleBeforeMib": idle_before,
            })
        }
    };
    let sampler = VramSampler::start();
    let cancel = AtomicBool::new(false);
    let t_decode = Instant::now();
    let result = asr::transcribe_file_with(&mut recognizer, wav, &cancel);
    let decode_ms = t_decode.elapsed().as_millis();
    let samples = sampler.finish();
    let peak = samples.iter().copied().max();
    drop(recognizer);
    // Give the driver a beat to reclaim, then confirm release.
    std::thread::sleep(Duration::from_millis(1500));
    let idle_after = vram_used_mib();
    match result {
        Ok(text) => serde_json::json!({
            "model": model, "loaded": true,
            "providerRequested": asr::provider_requested(model),
            "backend": asr::backend_for(model),
            "text": text,
            "loadMs": load_ms, "decodeMs": decode_ms,
            "vramIdleBeforeMib": idle_before,
            "vramPeakDecodeMib": peak,
            "vramSamples": samples.len(),
            "vramIdleAfterMib": idle_after,
        }),
        Err(error) => serde_json::json!({
            "model": model, "loaded": true, "decodeError": error,
            "providerRequested": asr::provider_requested(model),
            "loadMs": load_ms, "decodeMs": decode_ms,
            "vramIdleBeforeMib": idle_before,
            "vramPeakDecodeMib": peak,
            "vramIdleAfterMib": idle_after,
        }),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("models root"));
    let wav = PathBuf::from(args.next().expect("wav path"));
    let expected_path = PathBuf::from(args.next().expect("expected transcript path"));
    let mode = args
        .next()
        .expect("mode: tiny | qwen | switch | cancel | ready");
    let expected = expected_text(&expected_path, &wav);
    let expected_norm = normalize(&expected);

    let report = match mode.as_str() {
        "tiny" => {
            let mut r = decode_once(&root, asr::GPU_WHISPER_TINY_ID, &wav);
            let text = r.get("text").and_then(|v| v.as_str()).unwrap_or("");
            r["transcriptMatch"] = serde_json::json!(normalize(text) == expected_norm);
            r["expected"] = serde_json::json!(expected);
            r
        }
        "qwen" => {
            let mut r = decode_once(&root, asr::GPU_QWEN_ID, &wav);
            let text = r.get("text").and_then(|v| v.as_str()).unwrap_or("");
            r["transcriptMatch"] = serde_json::json!(normalize(text) == expected_norm);
            r["expected"] = serde_json::json!(expected);
            r
        }
        "switch" => {
            // One process, four loads: CPU -> GPU tiny -> GPU qwen -> CPU.
            // Each stage drops its recognizer before the next loads, matching
            // the single-capture-owner lease discipline. Any basename aliasing
            // between the CPU and CUDA runtimes would surface here as a load
            // failure or a wrong transcript, never as silent relabelling:
            // provider labels come from the ID and VRAM proves execution.
            let cpu = "parakeet-tdt-0.6b-v2";
            let stages = [cpu, asr::GPU_WHISPER_TINY_ID, asr::GPU_QWEN_ID, cpu];
            let mut out = Vec::new();
            for model in stages {
                let mut r = decode_once(&root, model, &wav);
                let text = r.get("text").and_then(|v| v.as_str()).unwrap_or("");
                r["transcriptMatch"] = serde_json::json!(normalize(text) == expected_norm);
                out.push(r);
            }
            serde_json::json!({ "stages": out, "expected": expected })
        }
        "cancel" => {
            let cancelled = AtomicBool::new(true);
            let cancel_err =
                asr::transcribe_file(&root, &wav, asr::GPU_WHISPER_TINY_ID, &cancelled)
                    .err()
                    .unwrap_or_else(|| "UNEXPECTED SUCCESS".into());
            let unknown_err = asr::Recognizer::load(&root, "not-a-model")
                .err()
                .unwrap_or_else(|| "UNEXPECTED SUCCESS".into());
            let gpu_missing =
                asr::Recognizer::load(Path::new("missing-gpu-models-probe"), asr::GPU_QWEN_ID)
                    .err()
                    .unwrap_or_else(|| "UNEXPECTED SUCCESS".into());
            // Empty audio must stay Complete+empty, never Incomplete.
            let empty_wav = root.join("empty-16k.wav");
            write_empty_wav(&empty_wav);
            let empty_result = (|| {
                let mut rec = asr::Recognizer::load(&root, asr::GPU_WHISPER_TINY_ID)?;
                let stream = rec.stream()?;
                stream.finish_recognition().map(|r| {
                    serde_json::json!({
                        "text": r.text,
                        "complete": r.completeness
                            == sonojot_core::domain::RecognitionCompleteness::Complete,
                        "warnings": r.warnings,
                    })
                })
            })();
            let _ = std::fs::remove_file(&empty_wav);
            serde_json::json!({
                "preCancelledError": cancel_err,
                "unknownIdError": unknown_err,
                "missingRuntimeError": gpu_missing,
                "emptyAudio": empty_result.map_err(|e: String| e),
            })
        }
        "ready" => serde_json::json!({
            "qwenReady": asr::model_ready(&root, asr::GPU_QWEN_ID),
            "tinyReady": asr::model_ready(&root, asr::GPU_WHISPER_TINY_ID),
            "cpuReady": asr::model_ready(&root, "parakeet-tdt-0.6b-v2"),
            "qwenProviderRequested": asr::provider_requested(asr::GPU_QWEN_ID),
            "tinyProviderRequested": asr::provider_requested(asr::GPU_WHISPER_TINY_ID),
            "cpuProviderRequested": asr::provider_requested("parakeet-tdt-0.6b-v2"),
            "qwenBackend": asr::backend_for(asr::GPU_QWEN_ID),
            "qwenCompatibility": asr::compatibility_for(asr::GPU_QWEN_ID),
        }),
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    };
    println!("{}", serde_json::to_string(&report).unwrap());
}

fn write_empty_wav(path: &Path) {
    // 0.5 s of 16 kHz mono silence; short enough to stay fast on CUDA.
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("empty wav");
    for _ in 0..8000 {
        writer.write_sample(0i16).expect("sample");
    }
    writer.finalize().expect("finalize");
}
