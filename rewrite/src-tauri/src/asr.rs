//! Moonshine's native C ABI, dynamically loaded from the app's model bundle.
//! Reference: https://github.com/moonshine-ai/moonshine/blob/main/core/moonshine-c-api.h
use crate::domain::RecognitionCompleteness;
use libloading::Library;
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

const HEADER_VERSION: i32 = 30000;
const REQUIRED_FILES: &[&str] = &[
    "encoder.ort",
    "adapter.ort",
    "cross_kv.ort",
    "decoder_kv.ort",
    "streaming_config.json",
    "tokenizer.bin",
];

/// paths.json may contain absolute development paths; packaged bundles fall
/// back to paths relative to the supplied model root.
pub fn bundle_path(root: &Path, key: &str, relative: &str) -> PathBuf {
    if let Ok(bytes) = std::fs::read(root.join("paths.json")) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let alias = match key {
                "moonshine_medium_dir" => "moonshine_medium_model_dir",
                "moonshine_small_dir" => "moonshine_small_model_dir",
                _ => key,
            };
            let candidate = value
                .get(key)
                .or_else(|| value.get(alias))
                .or_else(|| value.get("paths").and_then(|v| v.get(key)))
                .and_then(|v| v.as_str());
            if let Some(candidate) = candidate {
                let path = PathBuf::from(candidate);
                let resolved = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                if resolved.exists() {
                    return resolved;
                }
            }
        }
    }
    root.join(relative)
}
/// Disjoint GPU IDs (owned by `gpu_asr.rs`, never edited here).
/// Re-exported literals; tests assert they stay verbatim with
/// `gpu_asr::QWEN_FP16_CUDA_ID` / `WHISPER_TINY_FP32_CUDA_ID`.
pub const GPU_QWEN_ID: &str = "qwen3-asr-1.7b-fp16-cuda";
pub const GPU_WHISPER_TINY_ID: &str = "whisper-tiny-fp32-cuda";

pub fn is_gpu_model(id: &str) -> bool {
    // Single source of truth is the GPU module; the match mirrors it so the
    // routing order stays explicit and disjointness is testable here.
    crate::gpu_asr::is_gpu_model(id)
}

/// Requested provider from the selected ID alone (selection, not execution).
/// CUDA and Metal IDs are disjoint; unknown IDs never imply GPU execution.
pub fn provider_requested(id: &str) -> &'static str {
    if crate::mac_speech::find(id).is_some() {
        "metal"
    } else if let Some(model) = crate::llama_asr::find(id) {
        model.runtime.provider()
    } else if is_gpu_model(id) {
        "cuda"
    } else {
        "cpu"
    }
}

/// Verbatim backend label for the ID (for `acceleration.rs`/Setup display).
/// Never relabels CPU execution as GPU. GPU strings delegate to `gpu_asr` so
/// labels cannot drift.
pub fn backend_for(id: &str) -> &'static str {
    if cfg!(target_os = "linux") && crate::llama_asr::find(id).is_some_and(|m| matches!(m.runtime, crate::llama_asr::Runtime::Cpu)) {
        return "llama.cpp b10964 (Qwen3-ASR CPU)";
    }
    if crate::mac_speech::find(id).is_some() {
        crate::mac_speech::BACKEND
    } else if let Some(model) = crate::llama_asr::find(id) {
        model.backend
    } else if let Some(model) = crate::gpu_asr::find(id) {
        model.backend
    } else if crate::whisper_catalog::is_whisper(id) {
        crate::whisper_catalog::BACKEND
    } else if crate::optional_asr::is_optional(id) {
        "sherpa-onnx-1.13.8-cpu (Offline, transcribe, CPU provider)"
    } else {
        "moonshine-ort-cpu"
    }
}

/// Verbatim compatibility label for the ID (for `acceleration.rs`/Setup).
pub fn compatibility_for(id: &str) -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && (matches!(id, "moonshine-medium" | "moonshine-small" | crate::live_preview::MODEL_ID)
            || crate::optional_asr::is_optional(id) || crate::whisper_catalog::is_whisper(id)
            || crate::llama_asr::find(id).is_some_and(|m| matches!(m.runtime, crate::llama_asr::Runtime::Cpu)))
    {
        return "Linux x64 CPU via the pinned model runtime. Speech GPU offload is not enabled.";
    }
    if crate::mac_speech::find(id).is_some() {
        crate::mac_speech::COMPATIBILITY
    } else if let Some(model) = crate::llama_asr::find(id) {
        model.compatibility
    } else if is_gpu_model(id) {
        crate::gpu_asr::CUDA_COMPATIBILITY
    } else if crate::whisper_catalog::is_whisper(id) {
        crate::whisper_catalog::COMPATIBILITY
    } else if crate::optional_asr::is_optional(id) {
        "Windows x64 CPU via bundled sherpa-onnx v1.13.8; NVIDIA CUDA not used by this adapter; AMD has no CUDA path"
    } else {
        "Windows x64 CPU via bundled ONNX runtime; no GPU offload; never labeled as GPU."
    }
}

pub fn model_directory(root: &Path, id: &str) -> PathBuf {
    model_directory_checked(root, id, crate::model_verification::Check::Blocking)
}

pub(crate) fn model_display_directory(root: &Path, id: &str) -> PathBuf {
    model_directory_checked(root, id, crate::model_verification::Check::Background)
}

fn model_directory_checked(root: &Path, id: &str, check: crate::model_verification::Check) -> PathBuf {
    if crate::mac_speech::find(id).is_some() {
        crate::mac_speech::model_dir(root, id)
    } else if crate::llama_asr::is_llama_asr(id) {
        crate::llama_asr::model_dir(root, id)
    } else if is_gpu_model(id) {
        crate::gpu_asr::model_dir(root, id)
    } else if crate::whisper_catalog::is_whisper(id) {
        crate::whisper_catalog::model_dir(root, id)
    } else if crate::optional_asr::is_optional(id) || id == crate::live_preview::MODEL_ID {
        // The live-preview model is never bundled: only verified managed files.
        crate::optional_asr::model_dir(root, id)
    } else {
        let bundled = if id == "moonshine-small" {
            bundle_path(
                root,
                "moonshine_small_dir",
                "moonshine/models/small-streaming-123m-en",
            )
        } else {
            bundle_path(
                root,
                "moonshine_medium_dir",
                "moonshine/models/medium-streaming-245m-en",
            )
        };
        // Preserve working bundled/located models. Only a verified managed
        // copy can replace missing files; partial downloads are never loaded.
        if !moonshine_files_present(&bundled) {
            let managed = crate::model_manager::moonshine_managed_status(root, id, check);
            if managed != crate::model_verification::Status::Missing {
                return crate::optional_asr::model_dir(root, id);
            }
        }
        bundled
    }
}
fn runtime_directory(root: &Path) -> PathBuf {
    let platform = if cfg!(target_os = "windows") {
        "win-x64"
    } else if cfg!(target_os = "macos") {
        "mac-arm64"
    } else {
        "linux-x64"
    };
    bundle_path(
        root,
        "moonshine_runtime_dir",
        &format!("moonshine/runtime/{platform}"),
    )
}
pub fn runtime_path(root: &Path) -> PathBuf {
    let dir = runtime_directory(root);
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["moonshine.dll", "moonshine_voice.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmoonshine.dylib", "libmoonshine_voice.dylib"]
    } else {
        &["libmoonshine.so", "libmoonshine_voice.so"]
    };
    let declared = bundle_path(
        root,
        "moonshine_dll",
        &format!("moonshine/runtime/win-x64/lib/{}", names[0]),
    );
    if declared.is_file() {
        return declared;
    }
    names
        .iter()
        .flat_map(|name| [dir.join(name), dir.join("lib").join(name)])
        .find(|p| p.is_file())
        .unwrap_or_else(|| dir.join("lib").join(names[0]))
}
pub fn model_ready(root: &Path, id: &str) -> bool {
    model_status(root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    // Artifact-specific readiness: GPU requires the CUDA runtime (all DLLs +
    // VAD); CPU requires the CPU runtime. Raw filesize alone never claims
    // SHA-verified (cached stamp + install-time hash; UI polls this).
    // No CPU fallback for GPU IDs and vice versa.
    if crate::mac_speech::find(id).is_some() {
        return crate::mac_speech::model_status(root, id, check);
    }
    if crate::llama_asr::is_llama_asr(id) {
        return crate::llama_asr::model_status(root, id, check);
    }
    if is_gpu_model(id) {
        return crate::gpu_asr::model_status(root, id, check);
    }
    if crate::whisper_catalog::is_whisper(id) {
        return crate::whisper_catalog::model_status(root, id, check);
    }
    if crate::optional_asr::is_optional(id) {
        return crate::optional_asr::model_status(root, id, check);
    }
    if id == crate::live_preview::MODEL_ID {
        return Status::all([
            moonshine_runtime_status(root, check),
            crate::model_manager::moonshine_managed_status(root, id, check),
        ]);
    }
    if !matches!(id, "moonshine-medium" | "moonshine-small") {
        return Status::Missing;
    }
    let dir = model_directory_checked(root, id, check);
    let files = if dir == crate::optional_asr::model_dir(root, id) {
        crate::model_manager::moonshine_managed_status(root, id, check)
    } else {
        Status::present(moonshine_files_present(&dir))
    };
    Status::all([moonshine_runtime_status(root, check), files])
}

fn moonshine_runtime_status(root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    if crate::model_manager::linux_runtime::AVAILABLE {
        crate::model_manager::linux_runtime::moonshine_status(root, check)
    } else {
        crate::model_verification::Status::present(runtime_path(root).is_file())
    }
}

fn moonshine_files_present(dir: &Path) -> bool {
    REQUIRED_FILES.iter().all(|f| nonempty(&dir.join(f)))
        && (nonempty(&dir.join("frontend.ort"))
            || (nonempty(&dir.join("frontend.model.ort"))
                && nonempty(&dir.join("frontend.weights.ort"))))
}
fn nonempty(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Longest continuous VAD section sent to a sherpa recognizer. Production is
/// fixed at 20 s; evaluation builds accept 28 s from `fidelity_eval`.
pub(crate) fn sherpa_section_seconds() -> usize {
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_MAX_CHUNK_SECONDS").as_deref() == Ok("28") {
        return 28;
    }
    20
}

/// Whisper language for sherpa. `.en` checkpoints are English-only;
/// multilingual checkpoints auto-detect in production. Evaluation builds may
/// request explicit English to compare against detection.
pub(crate) fn sherpa_whisper_language(english_only: bool) -> &'static str {
    if english_only {
        return "en";
    }
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_ASR_LANGUAGE").as_deref() == Ok("en") {
        return "en";
    }
    ""
}

pub(crate) fn native_model_path(path: &Path) -> Result<CString, String> {
    let text = path.to_str().ok_or("Model path is not valid UTF-8.")?;
    // Moonshine and sherpa's Qwen tokenizer join filenames with '/'. Windows verbatim paths disable
    // separator normalization, so `\\?\C:\model/tokenizer.bin` cannot be
    // opened even though Rust can inspect the model directory. Convert only
    // this native-library argument; keep canonical store paths unchanged.
    #[cfg(target_os = "windows")]
    let text = if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", unc)
    } else if let Some(drive) = text.strip_prefix(r"\\?\") {
        if drive.as_bytes().get(1) != Some(&b':') {
            return Err("Speech recognition requires a drive or UNC model path.".into());
        }
        drive.to_owned()
    } else {
        text.to_owned()
    };
    CString::new(text).map_err(|_| "Model path contains an invalid null byte.".into())
}

#[repr(C)]
struct OptionC {
    name: *const c_char,
    value: *const c_char,
}
#[repr(C)]
struct LineC {
    text: *const c_char,
    audio_data: *const f32,
    audio_data_count: usize,
    start_time: f32,
    duration: f32,
    id: u64,
    is_complete: i8,
    is_updated: i8,
    is_new: i8,
    has_text_changed: i8,
    have_speakers_changed: i8,
    speaker_spans: *const c_void,
    speaker_span_count: u64,
    last_transcription_latency_ms: u32,
    words: *const c_void,
    word_count: u64,
}
#[repr(C)]
struct TranscriptC {
    lines: *const LineC,
    line_count: u64,
}
type Load = unsafe extern "C" fn(*const c_char, u32, *const OptionC, u64, i32) -> i32;
type Free = unsafe extern "C" fn(i32);
type Create = unsafe extern "C" fn(i32, u32) -> i32;
type StreamAction = unsafe extern "C" fn(i32, i32) -> i32;
type AddAudio = unsafe extern "C" fn(i32, i32, *const f32, u64, i32, u32) -> i32;
type Transcribe = unsafe extern "C" fn(i32, i32, u32, *mut *mut TranscriptC) -> i32;
type ErrorString = unsafe extern "C" fn(i32) -> *const c_char;

struct Api {
    // Libraries outlive every copied function pointer and transcriber handle.
    _library: Library,
    _dependencies: Vec<Library>,
    load: Load,
    free: Free,
    create: Create,
    free_stream: StreamAction,
    start: StreamAction,
    stop: StreamAction,
    add: AddAudio,
    transcribe: Transcribe,
    error: ErrorString,
}
impl Api {
    fn open(root: &Path) -> Result<Self, String> {
        let path = runtime_path(root);
        if !path.is_file() {
            return Err(format!(
                "Moonshine native runtime is missing: {}",
                path.display()
            ));
        }
        // Preload sibling dependencies by absolute path. Never alter PATH or
        // the process-wide DLL search directory from a worker thread.
        let mut dependencies = Vec::new();
        #[cfg(target_os = "windows")]
        for name in ["onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
            let dep = path.parent().unwrap_or(root).join(name);
            if dep.is_file() {
                dependencies.push(
                    unsafe { Library::new(&dep) }
                        .map_err(|e| format!("Cannot load {}: {e}", dep.display()))?,
                );
            }
        }
        let library =
            unsafe { Library::new(&path) }.map_err(|e| format!("Cannot load Moonshine: {e}"))?;
        unsafe {
            let version = *library
                .get::<unsafe extern "C" fn() -> i32>(b"moonshine_get_version\0")
                .map_err(|e| e.to_string())?;
            let version = version();
            if version / 10000 != HEADER_VERSION / 10000 {
                return Err(format!("Moonshine ABI {version} is incompatible with the bundled API {HEADER_VERSION}; install a matching native runtime."));
            }
            macro_rules! symbol {
                ($name:literal,$ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                };
            }
            Ok(Self {
                load: symbol!("moonshine_load_transcriber_from_files", Load),
                free: symbol!("moonshine_free_transcriber", Free),
                create: symbol!("moonshine_create_stream", Create),
                free_stream: symbol!("moonshine_free_stream", StreamAction),
                start: symbol!("moonshine_start_stream", StreamAction),
                stop: symbol!("moonshine_stop_stream", StreamAction),
                add: symbol!("moonshine_transcribe_add_audio_to_stream", AddAudio),
                transcribe: symbol!("moonshine_transcribe_stream", Transcribe),
                error: symbol!("moonshine_error_to_string", ErrorString),
                _library: library,
                _dependencies: dependencies,
            })
        }
    }
    fn check(&self, code: i32) -> Result<(), String> {
        if code == 0 {
            return Ok(());
        }
        let text = unsafe { (self.error)(code) };
        let message = if text.is_null() {
            format!("error {code}")
        } else {
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        };
        Err(format!("Moonshine: {message}"))
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub id: u64,
    pub start_seconds: f32,
    #[serde(default)]
    pub end_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecognitionResult {
    /// Guarded recognition used by cleanup, previews and publication.
    pub text: String,
    /// Speech before output guards, after protocol parsing. Absent when it
    /// equals `text`, so ordinary recognition does not duplicate a transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_text: Option<String>,
    #[serde(default)]
    pub segments: Vec<Segment>,
    pub completeness: RecognitionCompleteness,
    pub warnings: Vec<String>,
}

impl RecognitionResult {
    pub(crate) fn original_text(&self) -> &str {
        self.original_text.as_deref().unwrap_or(&self.text)
    }

    /// Legacy string-only consumers cannot carry a review requirement. Keep
    /// them fail-closed; dictation/evaluation use the typed result instead.
    pub(crate) fn into_complete_text(self) -> Result<String, String> {
        if self.completeness == RecognitionCompleteness::Incomplete {
            return Err("Recognition is incomplete. Retry transcription or review the recording before using its text.".into());
        }
        Ok(self.text)
    }
}

/// Append every section, including one whose guarded text is empty. Allocate
/// the original transcript only on the first changed section; the caller
/// supplies the guarded prefix before adding that section to its ledger.
pub(crate) fn retain_original_section(
    original: &mut Option<String>,
    preceding: impl FnOnce() -> String,
    section: &RecognitionResult,
) {
    if original.is_none() && section.original_text.is_some() {
        *original = Some(preceding());
    }
    if let Some(original) = original {
        let text = section.original_text();
        if !text.is_empty() {
            if !original.is_empty() { original.push(' '); }
            original.push_str(text);
        }
    }
}

/// Repeated section guards should not overflow the worker's warning envelope.
/// Keep distinct warnings in encounter order and reserve one slot for overflow.
pub(crate) fn append_recognition_warnings(target: &mut Vec<String>, incoming: impl IntoIterator<Item = String>) {
    const OVERFLOW: &str = "Additional recognition warnings occurred; review the recording before using its text.";
    for warning in incoming {
        if target.iter().any(|existing| existing == &warning) { continue; }
        if target.len() < 63 { target.push(warning); }
        else if !target.iter().any(|existing| existing == OVERFLOW) { target.push(OVERFLOW.into()); }
    }
}

enum Backend {
    Remote(crate::recognition::Remote),
    Moonshine { api: Api, handle: i32 },
    Sherpa(crate::sherpa::Engine),
    Gpu(crate::gpu_asr::Engine),
    #[cfg(target_os = "macos")]
    Metal(crate::mac_speech::Engine),
}

pub struct Recognizer {
    backend: Backend,
    pub model_id: String,
    pub requested_model: String,
}
impl Recognizer {
    pub(crate) fn is_alive(&mut self) -> bool {
        match &mut self.backend {
            Backend::Remote(remote) => remote.is_alive(),
            _ => true,
        }
    }
    /// PID and exit value of an exited worker (`None` while it runs).
    pub(crate) fn worker_exit(&mut self) -> Option<(u32, u64)> {
        match &mut self.backend {
            Backend::Remote(remote) => remote.exit(),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn kill_worker_for_test(&mut self) {
        if let Backend::Remote(remote) = &mut self.backend {
            remote.kill_worker_for_test();
        }
    }
    #[cfg(test)]
    pub(crate) fn worker_pid_for_test(&self) -> u32 {
        match &self.backend {
            Backend::Remote(remote) => remote.pid_for_test(),
            _ => 0,
        }
    }
    pub fn load(root: &Path, requested: &str) -> Result<Self, String> {
        Self::load_interruptible(root, requested, &AtomicBool::new(false))
    }

    pub fn load_interruptible(
        root: &Path,
        requested: &str,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        if crate::recognition::is_worker() {
            return Self::load_native(root, requested);
        }
        let remote = crate::recognition::Remote::load(root, requested, cancel)?;
        Ok(Self {
            backend: Backend::Remote(remote),
            model_id: requested.to_owned(),
            requested_model: requested.to_owned(),
        })
    }

    /// A disposable helper recognizer in its own below-normal-priority worker
    /// (the live preview). One load attempt; no replay or crash recovery.
    pub(crate) fn load_background(
        root: &Path,
        requested: &str,
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let remote = crate::recognition::Remote::load_background(root, requested, cancel)?;
        Ok(Self {
            backend: Backend::Remote(remote),
            model_id: requested.to_owned(),
            requested_model: requested.to_owned(),
        })
    }

    pub(crate) fn load_native(root: &Path, requested: &str) -> Result<Self, String> {
        if !crate::recognition::is_worker() {
            return Err("Native recognition must run in its owned worker process.".into());
        }
        if crate::mac_speech::find(requested).is_some() || crate::llama_asr::is_metal(requested) {
            #[cfg(target_os = "macos")]
            {
                let engine = crate::mac_speech::Engine::load(root, requested)?;
                return Ok(Self {
                    model_id: engine.model_id.clone(),
                    requested_model: engine.requested_model.clone(),
                    backend: Backend::Metal(engine),
                });
            }
            #[cfg(not(target_os = "macos"))]
            return Err("Metal speech recognition requires an Apple Silicon Mac.".into());
        }
        // Windows llama.cpp decodes audio while sherpa retains CPU VAD.
        // This ID stays outside is_gpu_model, which names the sherpa CUDA adapter.
        if crate::llama_asr::is_llama_asr(requested) {
            let engine = crate::sherpa::Engine::load(root, requested)?;
            return Ok(Self {
                model_id: engine.model_id.clone(),
                requested_model: engine.requested_model.clone(),
                backend: Backend::Sherpa(engine),
            });
        }
        // Sherpa GPU IDs are disjoint. Artifact-specific: requires the CUDA
        // runtime + GPU weights. Failure errors; never falls back to CPU.
        if is_gpu_model(requested) {
            let engine = crate::gpu_asr::Engine::load(root, requested)?;
            return Ok(Self {
                model_id: engine.model_id.clone(),
                requested_model: engine.requested_model.clone(),
                backend: Backend::Gpu(engine),
            });
        }
        if crate::whisper_catalog::is_whisper(requested)
            || crate::optional_asr::is_optional(requested)
        {
            let engine = crate::sherpa::Engine::load(root, requested)?;
            return Ok(Self {
                model_id: engine.model_id.clone(),
                requested_model: engine.requested_model.clone(),
                backend: Backend::Sherpa(engine),
            });
        }
        if !matches!(requested, "moonshine-medium" | "moonshine-small")
            && requested != crate::live_preview::MODEL_ID
        {
            return Err(format!("Unsupported recognizer: {requested}"));
        }
        let api = Api::open(root)?;
        // A missing or failed model must not silently change recognition.
        let candidates = [requested];
        let mut errors = Vec::new();
        for id in candidates {
            if !model_ready(root, id) {
                errors.push(format!("{id} is not installed"));
                continue;
            }
            let path = native_model_path(&model_directory(root, id))?;
            let mut names = vec![
                CString::new("ort_providers").unwrap(),
                CString::new("identify_speakers").unwrap(),
                // We publish text and never consume native per-segment PCM.
                // Let Moonshine release completed audio during long streams.
                CString::new("return_audio_data").unwrap(),
            ];
            let mut values = vec![
                CString::new("CPU").unwrap(),
                CString::new("false").unwrap(),
                CString::new("false").unwrap(),
            ];
            if id == crate::live_preview::MODEL_ID {
                // Moonshine skips passes until this much new audio arrives
                // (default 0.5 s); the live preview asks more often.
                names.push(CString::new("transcription_interval").unwrap());
                values.push(CString::new(crate::live_preview::TRANSCRIPTION_INTERVAL).unwrap());
            }
            let options: Vec<OptionC> = names
                .iter()
                .zip(values.iter())
                .map(|(name, value)| OptionC {
                    name: name.as_ptr(),
                    value: value.as_ptr(),
                })
                .collect();
            let handle = unsafe {
                (api.load)(
                    path.as_ptr(),
                    // MOONSHINE_MODEL_ARCH_{TINY,SMALL,MEDIUM}_STREAMING.
                    match id {
                        crate::live_preview::MODEL_ID => 2,
                        "moonshine-small" => 4,
                        _ => 5,
                    },
                    options.as_ptr(),
                    options.len() as u64,
                    HEADER_VERSION,
                )
            };
            if handle >= 0 {
                return Ok(Self {
                    backend: Backend::Moonshine { api, handle },
                    model_id: id.to_owned(),
                    requested_model: requested.to_owned(),
                });
            }
            errors.push(api.check(handle).unwrap_err());
        }
        Err(format!(
            "Cannot initialize speech recognition: {}",
            errors.join("; ")
        ))
    }
    /// Initialized backend provider: `"cuda"` or `"metal"` for the respective
    /// GPU adapter; `"cpu"` otherwise. This names what was INITIALIZED /
    /// REQUESTED (ID selection + successful `Engine::load`), NOT measured GPU
    /// execution: the native worker supplies no numeric offload proof for
    /// speech (unlike text `parse_offload` layer counts), so this must never
    /// be presented as observed execution. Use `provider_requested(id)` for
    /// selection identity and `execution_proof()` (always `None` in this
    /// build) for the honest execution slot. Never labels CPU as GPU.
    pub fn observed_provider(&self) -> &'static str {
        self.initialized_provider()
    }

    /// Honest name for the above: what the backend was initialized with.
    pub fn initialized_provider(&self) -> &'static str {
        match &self.backend {
            Backend::Remote(_) => provider_requested(&self.model_id),
            Backend::Gpu(_) => "cuda",
            Backend::Sherpa(_) if crate::llama_asr::is_llama_asr(&self.model_id) => {
                provider_requested(&self.model_id)
            }
            #[cfg(target_os = "macos")]
            Backend::Metal(_) => "metal",
            _ => "cpu",
        }
    }

    /// Measured GPU execution proof for speech. Always `None` in this build:
    /// the GPU adapter reports no numeric device/offload evidence, so UI must
    /// render initialized/requested (see `initialized_provider`) and keep the
    /// observed-execution slot empty rather than relabelling init as proof.
    /// Hook: if the native worker later publishes per-decode device ids or
    /// VRAM deltas, return them here and surface via `acceleration::AsrRuntime`.
    pub fn execution_proof(&self) -> Option<String> {
        None
    }
    /// Verbatim backend label for the loaded model (for acceleration UI).
    pub fn backend(&self) -> &'static str {
        backend_for(&self.model_id)
    }
    /// Verbatim compatibility label for the loaded model.
    pub fn compatibility(&self) -> &'static str {
        compatibility_for(&self.model_id)
    }
    pub fn stream(&mut self) -> Result<Stream<'_>, String> {
        self.stream_with_cancel(None)
    }
    pub fn stream_interruptible<'a>(
        &'a mut self,
        cancel: &'a AtomicBool,
    ) -> Result<Stream<'a>, String> {
        self.stream_with_cancel(Some(cancel))
    }
    fn stream_with_cancel<'a>(
        &'a mut self,
        cancel: Option<&'a AtomicBool>,
    ) -> Result<Stream<'a>, String> {
        match &mut self.backend {
            Backend::Remote(remote) => Ok(Stream::Remote(remote.stream(cancel)?)),
            Backend::Gpu(engine) => Ok(Stream::Gpu(engine.stream()?)),
            #[cfg(target_os = "macos")]
            Backend::Metal(engine) => Ok(Stream::Metal(engine.stream(cancel))),
            Backend::Sherpa(engine) => Ok(Stream::Sherpa(engine.stream()?)),
            Backend::Moonshine { api, handle } => {
                let transcriber = *handle;
                let stream_handle = unsafe { (api.create)(transcriber, 0) };
                if stream_handle < 0 {
                    return Err(api.check(stream_handle).unwrap_err());
                }
                if let Err(error) = api.check(unsafe { (api.start)(transcriber, stream_handle) }) {
                    unsafe {
                        (api.free_stream)(transcriber, stream_handle);
                    }
                    return Err(error);
                }
                Ok(Stream::Moonshine(MoonshineStream {
                    api,
                    transcriber,
                    handle: stream_handle,
                    ledger: BTreeMap::new(),
                    stopped: false,
                    pending_audio_seconds: 0.0,
                }))
            }
        }
    }
}
impl Drop for Recognizer {
    fn drop(&mut self) {
        if let Backend::Moonshine { api, handle } = &self.backend {
            unsafe {
                (api.free)(*handle);
            }
        }
    }
}

pub enum Stream<'a> {
    Remote(crate::recognition::RemoteStream<'a>),
    Moonshine(MoonshineStream<'a>),
    Sherpa(crate::sherpa::Stream<'a>),
    Gpu(crate::gpu_asr::Stream<'a>),
    #[cfg(target_os = "macos")]
    Metal(crate::mac_speech::Stream<'a>),
}

pub struct MoonshineStream<'a> {
    api: &'a Api,
    transcriber: i32,
    handle: i32,
    ledger: BTreeMap<u64, Segment>,
    stopped: bool,
    pending_audio_seconds: f64,
}
impl Stream<'_> {
    pub(crate) fn failed(&self) -> bool {
        matches!(self, Stream::Remote(s) if s.failed())
    }
    /// Evaluation replays must stop feeding a failed worker, like the capture owner.
    #[cfg(feature = "fidelity-eval")]
    pub fn evaluation_failed(&self) -> bool {
        self.failed()
    }
    pub(crate) fn original_text(&self) -> Option<&str> {
        match self {
            Stream::Remote(stream) => stream.original_text(),
            Stream::Sherpa(stream) => stream.original_text(),
            Stream::Gpu(stream) => stream.original_text(),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.original_text(),
            Stream::Moonshine(_) => None,
        }
    }
    /// Warnings recorded while decoding this stream's segments (GPU/CPU).
    pub fn warnings(&self) -> Vec<String> {
        match self {
            Stream::Remote(stream) => stream.warnings(),
            Stream::Sherpa(stream) => stream.warnings().to_vec(),
            Stream::Gpu(stream) => stream.warnings().to_vec(),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.warnings().to_vec(),
            Stream::Moonshine(_) => Vec::new(),
        }
    }
    #[cfg(feature = "fidelity-eval")]
    pub fn finish_observed(self) -> Result<(RecognitionResult, serde_json::Value), String> {
        match self {
            Stream::Remote(stream) => stream.finish(true).map(|(result, details)| {
                let details = details.unwrap_or_else(|| serde_json::json!({
                    "warnings":result.warnings,"recognition_completeness":result.completeness,
                    "all_segments_complete":result.completeness == RecognitionCompleteness::Complete,
                    "observation":"isolated worker; segment telemetry unavailable in this worker build"
                }));
                (result, details)
            }),
            Stream::Sherpa(stream) => stream.finish_observed(),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.finish_observed(),
            Stream::Gpu(stream) => stream.finish_observed(),
            Stream::Moonshine(mut stream) => {
                stream
                    .api
                    .check(unsafe { (stream.api.stop)(stream.transcriber, stream.handle) })?;
                stream.stopped = true;
                stream.decode(true)?;
                let recognition = finish_recognition(&stream.ledger);
                let segments: Vec<_> = stream.ledger.values().map(|s| serde_json::json!({
                    "id":s.id,"start_seconds":s.start_seconds,"text":s.text,"complete":s.complete,
                    "duration_seconds":null,"truncation_detected":null
                })).collect();
                let details = serde_json::json!({"segments":segments,"warnings":recognition.warnings,
                    "recognition_completeness":recognition.completeness,
                    "all_segments_complete":recognition.completeness == RecognitionCompleteness::Complete,
                    "truncation_status":"native finish reason unavailable; completion flags checked"});
                Ok((recognition, details))
            }
        }
    }
    pub fn add(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        match self {
            Stream::Remote(stream) => stream.add(samples, sample_rate),
            Stream::Sherpa(stream) => stream.add(samples, sample_rate),
            Stream::Gpu(stream) => stream.add(samples, sample_rate),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.add(samples, sample_rate),
            Stream::Moonshine(stream) => stream.add(samples, sample_rate),
        }
    }
    /// End of the newest recognized segment on the stream's own audio
    /// timeline (seconds of input from the first sample). Text from `update`
    /// covers audio up to here; a live preview shows only what comes after.
    /// Metal reports none: the Mac has no live-preview model yet.
    pub(crate) fn committed_seconds(&self) -> Option<f32> {
        match self {
            Stream::Remote(stream) => stream.committed_seconds(),
            Stream::Sherpa(stream) => stream.committed_seconds(),
            Stream::Gpu(stream) => stream.committed_seconds(),
            #[cfg(target_os = "macos")]
            Stream::Metal(_) => None,
            Stream::Moonshine(stream) => ledger_end(&stream.ledger),
        }
    }
    pub fn update(&mut self) -> Result<String, String> {
        match self {
            Stream::Remote(stream) => stream.update(),
            Stream::Sherpa(stream) => stream.update(),
            Stream::Gpu(stream) => stream.update(),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.update(),
            Stream::Moonshine(stream) => stream.update(),
        }
    }
    pub fn finish(self) -> Result<String, String> {
        self.finish_recognition()?.into_complete_text()
    }
    pub fn finish_recognition(self) -> Result<RecognitionResult, String> {
        match self {
            Stream::Remote(stream) => stream.finish(false).map(|(result, _)| result),
            Stream::Sherpa(stream) => stream.finish_recognition(),
            Stream::Gpu(stream) => stream.finish_recognition(),
            #[cfg(target_os = "macos")]
            Stream::Metal(stream) => stream.finish_recognition(),
            Stream::Moonshine(stream) => stream.finish_recognition(),
        }
    }
}
impl MoonshineStream<'_> {
    pub fn add(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        if self.stopped {
            return Err("Recognition stream is already stopped.".into());
        }
        if sample_rate == 0 || sample_rate > 384_000 {
            return Err("Invalid input sample rate.".into());
        }
        // Decode by audio duration as well as the UI cadence. Draining a capture
        // backlog after Stop must not accumulate the whole session inside ASR.
        for chunk in samples.chunks(sample_rate as usize) {
            self.api.check(unsafe {
                (self.api.add)(
                    self.transcriber,
                    self.handle,
                    chunk.as_ptr(),
                    chunk.len() as u64,
                    sample_rate as i32,
                    0,
                )
            })?;
            self.pending_audio_seconds += chunk.len() as f64 / sample_rate as f64;
            if self.pending_audio_seconds >= 1.0 {
                self.decode(false)?;
            }
        }
        Ok(())
    }
    pub fn update(&mut self) -> Result<String, String> {
        self.decode(false)?;
        Ok(self.assemble(false))
    }
    pub fn finish_recognition(mut self) -> Result<RecognitionResult, String> {
        self.api
            .check(unsafe { (self.api.stop)(self.transcriber, self.handle) })?;
        self.stopped = true;
        // Stop preserves the pending audio. FORCE_UPDATE drains it, including
        // utterances shorter than the normal streaming cadence.
        self.decode(true)?;
        Ok(finish_recognition(&self.ledger))
    }
    fn decode(&mut self, force: bool) -> Result<(), String> {
        let mut transcript = ptr::null_mut();
        self.api.check(unsafe {
            (self.api.transcribe)(
                self.transcriber,
                self.handle,
                u32::from(force),
                &mut transcript,
            )
        })?;
        self.pending_audio_seconds = 0.0;
        if transcript.is_null() {
            return Ok(());
        }
        let transcript = unsafe { &*transcript };
        if transcript.line_count > (isize::MAX as usize / std::mem::size_of::<LineC>()) as u64
            || (transcript.line_count != 0 && transcript.lines.is_null())
        {
            return Err("Moonshine returned an invalid transcript.".into());
        }
        if transcript.line_count == 0 {
            return Ok(());
        }
        // All library-owned strings are copied before another native call.
        for line in
            unsafe { std::slice::from_raw_parts(transcript.lines, transcript.line_count as usize) }
        {
            let text = if line.text.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(line.text) }
                    .to_str()
                    .map_err(|_| "Moonshine returned invalid UTF-8 text.")?
                    .to_owned()
            };
            self.ledger.insert(
                line.id,
                Segment {
                    id: line.id,
                    start_seconds: line.start_time,
                    end_seconds: Some(line.start_time + line.duration.max(0.0)),
                    speaker: None,
                    text,
                    complete: line.is_complete != 0,
                },
            );
        }
        Ok(())
    }
    fn assemble(&self, final_only: bool) -> String {
        assemble_segments(&self.ledger, final_only)
    }
}

pub(crate) fn finish_recognition(ledger: &BTreeMap<u64, Segment>) -> RecognitionResult {
    let incomplete = ledger.values().any(|line| !line.complete);
    RecognitionResult {
        original_text: None,
        text: assemble_segments(ledger, false),
        segments: ordered_segments(ledger),
        completeness: if incomplete {
            RecognitionCompleteness::Incomplete
        } else {
            RecognitionCompleteness::Complete
        },
        warnings: if incomplete {
            vec!["Recognition returned an unfinished final segment; available text was retained for review.".into()]
        } else {
            Vec::new()
        },
    }
}

/// Latest segment end in a ledger, or `None` before the first segment.
pub(crate) fn ledger_end(ledger: &BTreeMap<u64, Segment>) -> Option<f32> {
    ledger
        .values()
        .map(|segment| segment.end_seconds.unwrap_or(segment.start_seconds))
        .filter(|end| end.is_finite())
        .reduce(f32::max)
}

pub(crate) fn ordered_segments(ledger: &BTreeMap<u64, Segment>) -> Vec<Segment> {
    let mut segments: Vec<_> = ledger.values().cloned().collect();
    segments.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds).then(a.id.cmp(&b.id)));
    segments
}

pub(crate) fn assemble_segments(ledger: &BTreeMap<u64, Segment>, final_only: bool) -> String {
    let mut lines: Vec<_> = ledger
        .values()
        .filter(|l| !final_only || l.complete)
        .collect();
    lines.sort_by(|a, b| {
        a.start_seconds
            .total_cmp(&b.start_seconds)
            .then(a.id.cmp(&b.id))
    });
    // Segment text is immutable. The only assembly addition is one ASCII
    // space between nonempty segments. Revisions replace by stable ID.
    lines
        .into_iter()
        .filter(|l| !l.text.is_empty())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
impl Drop for MoonshineStream<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.api.free_stream)(self.transcriber, self.handle);
        }
    }
}

pub fn transcribe_file(
    root: &Path,
    path: &Path,
    model: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    let mut recognizer = Recognizer::load_interruptible(root, model, cancel)?;
    transcribe_file_with(&mut recognizer, path, cancel)
}
pub fn transcribe_file_with(
    recognizer: &mut Recognizer,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<String, String> {
    transcribe_file_recognition_with(recognizer, path, cancel)?.into_complete_text()
}

/// Typed file result for publication owners that can retain review-required
/// raw text. Legacy string APIs continue to reject incomplete recognition.
pub fn transcribe_file_recognition(
    root: &Path,
    path: &Path,
    model: &str,
    cancel: &AtomicBool,
) -> Result<RecognitionResult, String> {
    let mut recognizer = Recognizer::load_interruptible(root, model, cancel)?;
    transcribe_file_recognition_with(&mut recognizer, path, cancel)
}

pub fn transcribe_file_recognition_with(
    recognizer: &mut Recognizer,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<RecognitionResult, String> {
    transcribe_file_impl(recognizer, path, cancel, None)
}

#[cfg(feature = "fidelity-eval")]
pub fn transcribe_file_observed(
    recognizer: &mut Recognizer,
    path: &Path,
    cancel: &AtomicBool,
    trace: &mut serde_json::Value,
) -> Result<RecognitionResult, String> {
    transcribe_file_impl(recognizer, path, cancel, Some(trace))
}

fn transcribe_file_impl(
    recognizer: &mut Recognizer,
    path: &Path,
    cancel: &AtomicBool,
    mut trace: Option<&mut serde_json::Value>,
) -> Result<RecognitionResult, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    let (spec, values) = wav_samples(path)?;
    if let Some(trace) = trace.as_deref_mut() {
        *trace = serde_json::json!({"requested_model":recognizer.requested_model,
            "resolved_model":recognizer.model_id,"sample_rate":spec.sample_rate,
            "channels":spec.channels,"input_frames_consumed":0,"finalized":false});
    }
    let mut stream = recognizer.stream_interruptible(cancel)?;
    let mut chunk = Vec::with_capacity(spec.sample_rate as usize);
    let mut channel_count = 0u16;
    let mut sum = 0.0;
    let mut frames = 0u64;
    for value in values {
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        let value = value?;
        sum += if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        channel_count += 1;
        if channel_count == spec.channels {
            frames += 1;
            chunk.push(sum / spec.channels as f32);
            sum = 0.0;
            channel_count = 0;
            if chunk.len() >= spec.sample_rate as usize {
                // add drains native audio without rebuilding the entire text
                // once per second; only final publication needs that String.
                stream.add(&chunk, spec.sample_rate)?;
                chunk.clear();
                if stream.failed() {
                    break;
                }
            }
        }
    }
    if channel_count != 0 {
        return Err("WAV ends with an incomplete interleaved audio frame.".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    if !chunk.is_empty() {
        stream.add(&chunk, spec.sample_rate)?;
    }
    if let Some(trace) = trace.as_deref_mut() {
        trace["input_frames_consumed"] = serde_json::json!(frames);
    }
    #[cfg(feature = "fidelity-eval")]
    let result = if let Some(trace) = trace.as_deref_mut() {
        let (recognition, details) = stream.finish_observed()?;
        trace["warnings"] = details
            .get("warnings")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        trace["recognition_completeness"] = serde_json::json!(recognition.completeness);
        trace["recognition"] = details;
        trace["finalized"] = serde_json::json!(true);
        recognition
    } else {
        stream.finish_recognition()?
    };
    #[cfg(not(feature = "fidelity-eval"))]
    let result = stream.finish_recognition()?;
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    Ok(result)
}

type WavSamples = Box<dyn Iterator<Item = Result<f32, String>>>;

// Expose existing owners only to the opt-in headless replay binary. Production
// capture, queue limits, WAV conversion and finalization remain unchanged.
#[cfg(feature = "fidelity-eval")]
pub use crate::audio_backlog::AudioBacklog as EvaluationAudioBacklog;

#[cfg(feature = "fidelity-eval")]
pub fn evaluation_backlog_peak_bytes(backlog: &EvaluationAudioBacklog) -> u64 {
    backlog.peak_bytes()
}

#[cfg(feature = "fidelity-eval")]
pub fn evaluation_wav_samples(path: &Path) -> Result<(hound::WavSpec, u64, WavSamples), String> {
    let info = crate::recording::read_wav_info(path)?;
    if info.block_align == 0 || info.data_bytes % u64::from(info.block_align) != 0 {
        return Err("WAV contains an incomplete audio frame.".into());
    }
    let frames = info.data_bytes / u64::from(info.block_align);
    let (spec, samples) = wav_samples(path)?;
    if spec.channels == 0 || !(1..=384_000).contains(&spec.sample_rate) {
        return Err("Invalid WAV audio format.".into());
    }
    Ok((spec, frames, samples))
}

pub(crate) fn wav_samples(path: &Path) -> Result<(hound::WavSpec, WavSamples), String> {
    let info = crate::recording::read_wav_info(path)?;
    if info.rf64 {
        // Capture and the media converter produce PCM16. hound does not read
        // RF64, so stream those samples using the validated 64-bit data length.
        if info.format != 1
            || info.bits_per_sample != 16
            || u32::from(info.block_align) != u32::from(info.channels) * 2
            || u64::from(info.byte_rate)
                != u64::from(info.sample_rate) * u64::from(info.block_align)
            || !(1..=384_000).contains(&info.sample_rate)
        {
            return Err("RF64 audio must use 16-bit PCM; import other media through the app's media converter.".into());
        }
        let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
        reader
            .seek(SeekFrom::Start(info.data_offset))
            .map_err(|e| e.to_string())?;
        let mut remaining = info.data_bytes / 2;
        let values = std::iter::from_fn(move || {
            if remaining == 0 {
                return None;
            }
            let mut bytes = [0u8; 2];
            match reader.read_exact(&mut bytes) {
                Ok(()) => {
                    remaining -= 1;
                    Some(Ok(f32::from(i16::from_le_bytes(bytes)) / 32768.0))
                }
                Err(error) => {
                    remaining = 0;
                    Some(Err(format!(
                        "RF64 audio ended before its declared length: {error}"
                    )))
                }
            }
        });
        return Ok((
            hound::WavSpec {
                channels: info.channels,
                sample_rate: info.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
            Box::new(values),
        ));
    }
    let reader = hound::WavReader::open(path).map_err(|e| {
        format!("Cannot read WAV audio: {e}. Import other media through the app's media converter.")
    })?;
    let spec = reader.spec();
    if spec.channels == 0 || !(1..=384_000).contains(&spec.sample_rate) {
        return Err("Invalid WAV audio format.".into());
    }
    if spec.sample_format == hound::SampleFormat::Int && !(1..=32).contains(&spec.bits_per_sample) {
        return Err("Unsupported PCM bit depth.".into());
    }
    let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
    let values: WavSamples = if spec.sample_format == hound::SampleFormat::Float {
        Box::new(
            reader
                .into_samples::<f32>()
                .map(|v| v.map_err(|e| e.to_string())),
        )
    } else {
        Box::new(
            reader
                .into_samples::<i32>()
                .map(move |v| v.map(|s| s as f32 / scale).map_err(|e| e.to_string())),
        )
    };
    Ok((spec, values))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_recognition_keeps_asr_ellipses_and_fillers_exactly() {
        let raw = "Um... And then … preserve `x...y` and 'um...'";
        let ledger = BTreeMap::from([(
            1,
            Segment {
                id: 1,
                start_seconds: 0.0,
                end_seconds: None,
                speaker: None,
                text: raw.into(),
                complete: true,
            },
        )]);
        let recognition = finish_recognition(&ledger);
        assert_eq!(recognition.text, raw);
        assert_eq!(recognition.completeness, RecognitionCompleteness::Complete);
        assert!(recognition.warnings.is_empty());
        // Preview truncation happens only in the UI. Assembly neither adds
        // an omission marker nor strips punctuation supplied by recognition.
        assert_eq!(assemble_segments(&ledger, false), raw);
    }

    #[test]
    fn incomplete_recognition_preserves_before_failure_after_and_full_tail() {
        let mut ledger = BTreeMap::new();
        for (id, start, text, complete) in [
            (10, 0.0, "Keep the first instruction.", true),
            (4, 599.0, "<thö", false),
            (9, 601.0, "Do not remove the backup.", true),
            (1, 86_400.0, "Final words: café, 保留.", true),
        ] {
            ledger.insert(
                id,
                Segment {
                    id,
                    start_seconds: start,
                    end_seconds: None,
                    speaker: None,
                    text: text.into(),
                    complete,
                },
            );
        }
        let recognition = finish_recognition(&ledger);
        assert_eq!(
            recognition.text,
            "Keep the first instruction. <thö Do not remove the backup. Final words: café, 保留."
        );
        assert_eq!(
            recognition.completeness,
            RecognitionCompleteness::Incomplete
        );
        assert_eq!(recognition.warnings.len(), 1);
        assert_eq!(recognition.segments.iter().map(|s| s.start_seconds).collect::<Vec<_>>(),
            vec![0.0, 599.0, 601.0, 86_400.0]);
        assert!(!recognition.segments[1].complete);
        let serialized = serde_json::to_vec(&recognition).unwrap();
        assert_eq!(serde_json::from_slice::<RecognitionResult>(&serialized).unwrap(), recognition);
        ledger.get_mut(&4).unwrap().complete = true;
        let recognition = finish_recognition(&ledger);
        assert_eq!(recognition.completeness, RecognitionCompleteness::Complete);
        assert!(recognition.text.ends_with("Final words: café, 保留."));
    }

    #[test]
    fn repeated_section_warnings_stay_within_worker_envelope() {
        let mut warnings = Vec::new();
        append_recognition_warnings(&mut warnings, (0..150).map(|_| "Repeated speech omitted.".into()));
        assert_eq!(warnings, ["Repeated speech omitted."]);
        append_recognition_warnings(&mut warnings, (0..100).map(|i| format!("Different warning {i}")));
        assert_eq!(warnings.len(), 64);
        assert!(warnings.last().unwrap().starts_with("Additional recognition warnings"));
    }

    #[test]
    fn original_sections_preserve_dropped_and_changed_text_without_clean_duplicates() {
        let mut original = None;
        let mut published = String::new();
        for (guarded, unmodified) in [
            ("First.", None),
            ("", Some("echo echo echo echo")),
            ("Middle words remain. go", Some("Middle words remain. go go go go")),
            ("Last.", None),
        ] {
            let section = RecognitionResult {
                text: guarded.into(), original_text: unmodified.map(str::to_owned),
                segments: Vec::new(), completeness: RecognitionCompleteness::Incomplete,
                warnings: Vec::new(),
            };
            retain_original_section(&mut original, || published.clone(), &section);
            if !guarded.is_empty() {
                if !published.is_empty() { published.push(' '); }
                published.push_str(guarded);
            }
            if published == "First." && unmodified.is_none() { assert!(original.is_none()); }
        }
        assert_eq!(published, "First. Middle words remain. go Last.");
        assert_eq!(original.as_deref(), Some("First. echo echo echo echo Middle words remain. go go go go Last."));
        let clean = RecognitionResult {
            text: "Clean.".into(), original_text: None, segments: Vec::new(),
            completeness: RecognitionCompleteness::Complete, warnings: Vec::new(),
        };
        let mut unchanged = None;
        retain_original_section(&mut unchanged, || panic!("clean text must not be copied"), &clean);
        assert!(unchanged.is_none());
        assert!(serde_json::to_value(&clean).unwrap().get("original_text").is_none());
    }

    #[test]
    fn wholly_dropped_sections_still_accumulate_original_text() {
        let mut original = None;
        for raw in ["echo echo echo echo", "again again again again"] {
            let section = RecognitionResult {
                text: String::new(), original_text: Some(raw.into()), segments: Vec::new(),
                completeness: RecognitionCompleteness::Incomplete, warnings: vec!["omitted".into()],
            };
            retain_original_section(&mut original, String::new, &section);
        }
        assert_eq!(original.as_deref(), Some("echo echo echo echo again again again again"));
    }

    #[test]
    fn empty_unfinished_segment_is_typed_incomplete_without_invented_text() {
        let legacy: RecognitionResult = serde_json::from_str(
            r#"{"text":"Old worker result","completeness":"complete","warnings":[]}"#,
        ).unwrap();
        assert!(legacy.segments.is_empty());
        let ledger = BTreeMap::from([(
            1,
            Segment {
                id: 1,
                start_seconds: 0.0,
                end_seconds: None,
                speaker: None,
                text: String::new(),
                complete: false,
            },
        )]);
        let recognition = finish_recognition(&ledger);
        assert!(recognition.text.is_empty());
        assert_eq!(
            recognition.completeness,
            RecognitionCompleteness::Incomplete
        );
        assert_eq!(recognition.warnings.len(), 1);
        assert!(recognition.into_complete_text().is_err());
        assert_eq!(
            RecognitionResult {
                original_text: None,
                segments: Vec::new(),
                text: "Complete tail.".into(),
                completeness: RecognitionCompleteness::Complete,
                warnings: Vec::new(),
            }
            .into_complete_text()
            .unwrap(),
            "Complete tail."
        );
    }

    #[test]
    fn rf64_samples_stream_and_truncation_is_rejected() {
        let path =
            std::env::temp_dir().join(format!("sonojot-rf64-asr-{}.wav", uuid::Uuid::new_v4()));
        let mut writer = crate::recording::PcmWriter::create_at_rate(&path, 16_000).unwrap();
        writer.write_samples(&[0.0, 0.5, -1.0]).unwrap();
        writer.checkpoint().unwrap();
        drop(writer);
        let mut bytes = std::fs::read(&path).unwrap();
        let size = bytes.len() as u64;
        bytes[..4].copy_from_slice(b"RF64");
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[12..16].copy_from_slice(b"ds64");
        bytes[20..28].copy_from_slice(&(size - 8).to_le_bytes());
        bytes[28..36].copy_from_slice(&6u64.to_le_bytes());
        bytes[36..44].copy_from_slice(&3u64.to_le_bytes());
        bytes[76..80].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();
        let (spec, values) = wav_samples(&path).unwrap();
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(
            values.collect::<Result<Vec<_>, _>>().unwrap(),
            vec![0.0, 0.5, -32767.0 / 32768.0]
        );
        assert_eq!(
            crate::recording::wav_duration(&path).unwrap(),
            3.0 / 16_000.0
        );
        bytes.pop();
        std::fs::write(&path, bytes).unwrap();
        assert!(wav_samples(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn native_model_paths_allow_windows_filename_appending() {
        assert_eq!(
            native_model_path(Path::new(r"\\?\C:\Users\Sam\Model Cache\medium"))
                .unwrap()
                .to_str()
                .unwrap(),
            r"C:\Users\Sam\Model Cache\medium"
        );
        assert_eq!(
            native_model_path(Path::new(r"\\?\UNC\server\share\medium"))
                .unwrap()
                .to_str()
                .unwrap(),
            r"\\server\share\medium"
        );
        assert_eq!(
            native_model_path(Path::new(r"C:\Model Cache\medium"))
                .unwrap()
                .to_str()
                .unwrap(),
            r"C:\Model Cache\medium"
        );
    }
    #[test]
    fn abi_layout_matches_64_bit_header() {
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(std::mem::size_of::<LineC>(), 88);
            assert_eq!(std::mem::offset_of!(LineC, speaker_spans), 48);
            assert_eq!(std::mem::size_of::<TranscriptC>(), 16);
        }
    }
    #[test]
    fn missing_models_are_not_ready() {
        assert!(!model_ready(
            Path::new("missing-test-models"),
            "moonshine-medium"
        ));
        assert!(!model_ready(Path::new("."), "unknown"));
    }

    #[test]
    fn llama_asr_routes_to_distinct_artifacts_and_cuda_provider() {
        let root = Path::new("missing-llama-asr-test-models");
        let id = crate::llama_asr::MODEL_ID;
        assert!(!is_gpu_model(id)); // The sherpa CUDA guard must not reject it.
        assert!(!crate::optional_asr::is_optional(id));
        assert!(!crate::whisper_catalog::is_whisper(id));
        assert_eq!(provider_requested(id), "cuda");
        assert!(backend_for(id).contains("llama.cpp b10900 CUDA 13.3"));
        assert!(compatibility_for(id).contains("NVIDIA"));
        assert_eq!(model_directory(root, id), root.join("optional").join(id));
        assert_ne!(model_directory(root, id), model_directory(root, GPU_QWEN_ID));
        assert!(!model_ready(root, id));
    }

    #[test]
    fn metal_qwen_routes_keep_platform_backend_and_artifact_identity() {
        let root = Path::new("missing-metal-routing-fixture");
        for id in [crate::llama_asr::METAL_Q8_MODEL_ID, crate::llama_asr::METAL_Q4_MODEL_ID] {
            assert!(crate::llama_asr::is_metal(id));
            assert!(!is_gpu_model(id));
            assert_eq!(provider_requested(id), "metal");
            assert!(backend_for(id).starts_with("llama.cpp b10964 Metal (Qwen3-ASR"));
            assert!(backend_for(id).contains("sectioning local"));
            assert!(compatibility_for(id).contains("Apple Silicon"));
            assert_eq!(model_directory(root, id), crate::llama_asr::model_dir(root, id));
            assert!(!model_ready(root, id));
        }
    }

    #[test]
    fn gpu_ids_are_disjoint_from_cpu_catalogs() {
        // Literals stay verbatim with the GPU module (single source of truth).
        assert_eq!(GPU_QWEN_ID, crate::gpu_asr::QWEN_FP16_CUDA_ID);
        assert_eq!(
            GPU_WHISPER_TINY_ID,
            crate::gpu_asr::WHISPER_TINY_FP32_CUDA_ID
        );
        assert_eq!(backend_for(GPU_QWEN_ID), crate::gpu_asr::QWEN_CUDA_BACKEND);
        assert_eq!(
            backend_for(GPU_WHISPER_TINY_ID),
            crate::gpu_asr::WHISPER_CUDA_BACKEND
        );
        assert_eq!(
            compatibility_for(GPU_QWEN_ID),
            crate::gpu_asr::CUDA_COMPATIBILITY
        );
        for gpu in [GPU_QWEN_ID, GPU_WHISPER_TINY_ID] {
            assert!(is_gpu_model(gpu), "{gpu}");
            assert!(!crate::whisper_catalog::is_whisper(gpu), "{gpu}");
            assert!(!crate::optional_asr::is_optional(gpu), "{gpu}");
            assert_eq!(provider_requested(gpu), "cuda");
            assert!(backend_for(gpu).contains("CUDA provider"));
            assert!(compatibility_for(gpu).contains("NVIDIA CUDA"));
            assert!(compatibility_for(gpu).contains("AMD has no CUDA path"));
            // Artifact-specific readiness: missing root is never ready, never
            // falls back to CPU.
            assert!(!model_ready(Path::new("missing-test-models"), gpu));
        }
        for cpu in [
            "whisper-tiny",
            "whisper-large-v3-turbo",
            "parakeet-tdt-0.6b-v2",
            "qwen3-asr-1.7b",
            "moonshine-medium",
        ] {
            assert!(!is_gpu_model(cpu), "{cpu}");
            assert_eq!(provider_requested(cpu), "cpu");
            assert!(!backend_for(cpu).contains("CUDA provider"), "{cpu}");
            assert!(!compatibility_for(cpu).contains("NVIDIA CUDA 13"), "{cpu}");
        }
        // Prefix sharing is not identity: FP32 CUDA tiny is not CPU whisper.
        assert!(!crate::whisper_catalog::is_whisper(GPU_WHISPER_TINY_ID));
        assert!(is_gpu_model(GPU_WHISPER_TINY_ID));
        assert!(!is_gpu_model("whisper-tiny"));
        assert_eq!(provider_requested("unknown-id"), "cpu");
        assert_eq!(backend_for("unknown-id"), "moonshine-ort-cpu");
    }

    #[test]
    fn gpu_model_directory_is_artifact_specific() {
        let root = Path::new(r"C:\Models");
        for gpu in [GPU_QWEN_ID, GPU_WHISPER_TINY_ID] {
            let dir = model_directory(root, gpu);
            assert!(dir.starts_with(root.join("optional")), "{gpu}");
            assert!(dir.ends_with(gpu), "{gpu}");
            // CPU dirs for the same family are different artifacts.
            assert_ne!(dir, model_directory(root, "whisper-tiny"));
            assert_ne!(dir, model_directory(root, "qwen3-asr-1.7b"));
        }
    }

    #[test]
    fn transcribe_file_honours_pre_cancel_for_every_backend() {
        use std::sync::atomic::AtomicBool;
        let cancel = AtomicBool::new(true);
        // Each branch must refuse before touching native libraries, so the
        // meeting file path keeps its cancellation guard for GPU too.
        for id in [
            GPU_QWEN_ID,
            GPU_WHISPER_TINY_ID,
            "whisper-tiny",
            "qwen3-asr-1.7b",
            "moonshine-medium",
        ] {
            let err = transcribe_file(
                Path::new("missing-test-models"),
                Path::new("missing.wav"),
                id,
                &cancel,
            )
            .unwrap_err();
            assert!(
                err.contains("cancelled") || err.contains("Cancelled"),
                "{id}: {err}"
            );
        }
    }

    #[test]
    fn incomplete_recognition_never_publishes_via_typed_result() {
        // Completeness boundary shared by capture owner and meeting filepath:
        // Incomplete must fail closed through into_complete_text.
        let incomplete = RecognitionResult {
            original_text: None,
            segments: Vec::new(),
            text: "partial tail".into(),
            completeness: RecognitionCompleteness::Incomplete,
            warnings: vec!["review".into()],
        };
        assert!(incomplete.into_complete_text().is_err());
        let complete = RecognitionResult {
            original_text: None,
            segments: Vec::new(),
            text: "full tail".into(),
            completeness: RecognitionCompleteness::Complete,
            warnings: Vec::new(),
        };
        assert_eq!(complete.into_complete_text().unwrap(), "full tail");
    }
}
