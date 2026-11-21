//! SonoJot's offline speech recognition for phones, without Tauri or desktop dependencies.
//!
//! The pipeline for one recording of 16 kHz mono `f32` PCM:
//!
//! 1. [`AudioGateFilter`] runs SonoJot's RNNoise gate (the desktop `asr_filter.rs`, compiled here
//!    unchanged) over a copy of the audio, compensates its fixed 352-sample delay, and returns the
//!    speech regions of the *original* audio. The recognizer never hears the denoised copy.
//! 2. A [`Recognizer`] decodes each region. [`SherpaRecognizer`] loads the sherpa-onnx C API
//!    (`libsherpa-onnx-c-api.so`) at run time and supports Whisper, SenseVoice, transducer
//!    (Zipformer, Parakeet) and Moonshine models.
//! 3. [`postprocess`] removes non-speech annotations and decoder loops, fixes all-caps output and,
//!    for English, applies SonoJot's conservative spoken-to-written rules ([`written_form`]).
//!
//! Model files arrive as a pack: a directory with a `manifest.json` ([`ModelPackManifest`]) that
//! pins every file's size and SHA-256. [`SonojotEngine::open`] verifies the pack before it loads.
//!
//! Phones download packs listed in [`catalog`] (pinned files, the same pins as the desktop):
//! SonoJot Parakeet for English, Whisper Small for every other language.
//!
//! Mobile apps call the C ABI in [`ffi`] (`include/sonojot_mobile.h`). The rules that keep this
//! crate, the desktop app and the apps coherent are in `README.md` ("Rules").

#[path = "../../../rewrite/src-tauri/src/asr_filter.rs"]
mod asr_filter;

/// SonoJot's spoken-to-written rules, shared with the desktop app.
#[path = "../../../rewrite/src-tauri/src/written_form.rs"]
pub mod written_form;

pub mod catalog;
mod engine;
mod error;
pub mod ffi;
mod gate;
mod manifest;
pub mod postprocess;
mod runner;
mod sherpa;

pub use engine::{SegmentResult, SonojotConfig, SonojotEngine, TranscriptResult, MAX_AUDIO_SAMPLES};
pub use error::EngineError;
pub use gate::{AudioGateFilter, GateConfig, GateMode, GatedAudio};
pub use manifest::{
    language_of, whisper_language, ModelFamily, ModelOptions, ModelPackManifest, PackFile, RuntimeRequirement,
    VerificationReport, VerifyMode, MANIFEST_FILE, MANIFEST_SCHEMA, WHISPER_LANGUAGES,
};
pub use runner::Recognizer;
pub use sherpa::{SherpaRecognizer, SherpaRuntime, DEFAULT_RUNTIME_LIBRARY, RUNTIME_LIBRARY_SELF, SHERPA_ONNX_ABI};

/// Sample rate of every PCM buffer this crate accepts.
pub const SAMPLE_RATE: u32 = 16_000;
