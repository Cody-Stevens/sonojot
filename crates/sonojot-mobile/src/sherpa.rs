//! The sherpa-onnx offline recognizer through its C API, loaded at run time with `libloading`.
//!
//! The `#[repr(C)]` structs below are the sherpa-onnx 1.13.8 layout, the same release and field
//! order that SonoJot's desktop runner (`rewrite/src-tauri/src/sherpa.rs`) uses. A different
//! release can reorder or extend them, so [`SherpaRuntime::open`] refuses any library whose
//! `SherpaOnnxGetVersionStr` is not [`SHERPA_ONNX_ABI`].

use crate::{whisper_language, EngineError, ModelFamily, ModelPackManifest, Recognizer, SAMPLE_RATE};
use libloading::Library;
use std::{
    ffi::{c_char, CStr, CString},
    path::Path,
    sync::Arc,
};

/// The sherpa-onnx release whose C structs this module mirrors.
pub const SHERPA_ONNX_ABI: &str = "1.13.8";

/// File name `dlopen` looks for when the app passes no path. On Android the library ships in the
/// APK's `jniLibs` next to `libonnxruntime.so`, so the name alone resolves.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub const DEFAULT_RUNTIME_LIBRARY: &str = "libsherpa-onnx-c-api.dylib";
#[cfg(windows)]
pub const DEFAULT_RUNTIME_LIBRARY: &str = "sherpa-onnx-c-api.dll";
#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
pub const DEFAULT_RUNTIME_LIBRARY: &str = "libsherpa-onnx-c-api.so";

/// `runtime_library` value that resolves the sherpa-onnx C API from the running program instead
/// of loading a library: for iOS, where the runtime is linked statically. The app must keep the
/// `SherpaOnnx*` symbols from being stripped or dead-stripped.
pub const RUNTIME_LIBRARY_SELF: &str = "self";

#[repr(C)]
struct FeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}
#[repr(C)]
struct HomophoneReplacerConfig {
    dict_dir: *const c_char,
    lexicon: *const c_char,
    rule_fsts: *const c_char,
}
#[repr(C)]
struct OfflineTransducerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    joiner: *const c_char,
}
#[repr(C)]
struct OfflineParaformerModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineNemoEncDecCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineWhisperModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    task: *const c_char,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}
#[repr(C)]
struct OfflineCanaryModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    src_lang: *const c_char,
    tgt_lang: *const c_char,
    use_pnc: i32,
}
#[repr(C)]
struct OfflineCohereTranscribeModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    use_punct: i32,
    use_itn: i32,
}
#[repr(C)]
struct OfflineFireRedAsrModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
}
#[repr(C)]
struct OfflineFireRedAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineMoonshineModelConfig {
    preprocessor: *const c_char,
    encoder: *const c_char,
    uncached_decoder: *const c_char,
    cached_decoder: *const c_char,
    merged_decoder: *const c_char,
}
#[repr(C)]
struct OfflineTdnnModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineLMConfig {
    model: *const c_char,
    scale: f32,
}
#[repr(C)]
struct OfflineSenseVoiceModelConfig {
    model: *const c_char,
    language: *const c_char,
    use_itn: i32,
}
#[repr(C)]
struct OfflineDolphinModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineZipformerCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineWenetCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineOmnilingualAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineFunASRNanoModelConfig {
    encoder_adaptor: *const c_char,
    llm: *const c_char,
    embedding: *const c_char,
    tokenizer: *const c_char,
    system_prompt: *const c_char,
    user_prompt: *const c_char,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    language: *const c_char,
    itn: i32,
    hotwords: *const c_char,
}
#[repr(C)]
struct OfflineQwen3ASRModelConfig {
    conv_frontend: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    tokenizer: *const c_char,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    hotwords: *const c_char,
}
#[repr(C)]
struct OfflineMedAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineModelConfig {
    transducer: OfflineTransducerModelConfig,
    paraformer: OfflineParaformerModelConfig,
    nemo_ctc: OfflineNemoEncDecCtcModelConfig,
    whisper: OfflineWhisperModelConfig,
    tdnn: OfflineTdnnModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    telespeech_ctc: *const c_char,
    sense_voice: OfflineSenseVoiceModelConfig,
    moonshine: OfflineMoonshineModelConfig,
    fire_red_asr: OfflineFireRedAsrModelConfig,
    dolphin: OfflineDolphinModelConfig,
    zipformer_ctc: OfflineZipformerCtcModelConfig,
    canary: OfflineCanaryModelConfig,
    wenet_ctc: OfflineWenetCtcModelConfig,
    omnilingual: OfflineOmnilingualAsrCtcModelConfig,
    medasr: OfflineMedAsrCtcModelConfig,
    funasr_nano: OfflineFunASRNanoModelConfig,
    fire_red_asr_ctc: OfflineFireRedAsrCtcModelConfig,
    qwen3_asr: OfflineQwen3ASRModelConfig,
    cohere_transcribe: OfflineCohereTranscribeModelConfig,
}
#[repr(C)]
struct OfflineRecognizerConfig {
    feat_config: FeatureConfig,
    model_config: OfflineModelConfig,
    lm_config: OfflineLMConfig,
    decoding_method: *const c_char,
    max_active_paths: i32,
    hotwords_file: *const c_char,
    hotwords_score: f32,
    rule_fsts: *const c_char,
    rule_fars: *const c_char,
    blank_penalty: f32,
    hr: HomophoneReplacerConfig,
}
/// Prefix of the 1.13.8 result struct; only `text` is read.
#[repr(C)]
struct OfflineRecognizerResult {
    text: *const c_char,
    timestamps: *mut f32,
    count: i32,
    tokens: *const c_char,
    tokens_arr: *const *const c_char,
    json: *const c_char,
}

enum OfflineRecognizer {}
enum OfflineStream {}

// sherpa-onnx is C++ behind a C API. "C-unwind" makes a C++ exception that escapes it defined
// behaviour (Rust aborts cleanly) rather than undefined behaviour in the Rust frames above.
type CreateOffline = unsafe extern "C-unwind" fn(*const OfflineRecognizerConfig) -> *const OfflineRecognizer;
type DestroyOffline = unsafe extern "C-unwind" fn(*const OfflineRecognizer);
type CreateStream = unsafe extern "C-unwind" fn(*const OfflineRecognizer) -> *const OfflineStream;
type DestroyStream = unsafe extern "C-unwind" fn(*const OfflineStream);
type AcceptWaveform = unsafe extern "C-unwind" fn(*const OfflineStream, i32, *const f32, i32);
type DecodeStream = unsafe extern "C-unwind" fn(*const OfflineRecognizer, *const OfflineStream);
type GetResult = unsafe extern "C-unwind" fn(*const OfflineStream) -> *const OfflineRecognizerResult;
type DestroyResult = unsafe extern "C-unwind" fn(*const OfflineRecognizerResult);
type GetVersion = unsafe extern "C-unwind" fn() -> *const c_char;

/// A loaded sherpa-onnx C API. Shared by every recognizer created from it.
pub struct SherpaRuntime {
    create_offline: CreateOffline,
    destroy_offline: DestroyOffline,
    create_stream: CreateStream,
    destroy_stream: DestroyStream,
    accept: AcceptWaveform,
    decode: DecodeStream,
    get_result: GetResult,
    destroy_result: DestroyResult,
    version: String,
    // Last, so the function pointers above are never used after the library unloads.
    _library: Library,
}

impl std::fmt::Debug for SherpaRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SherpaRuntime").field("version", &self.version).finish_non_exhaustive()
    }
}

impl SherpaRuntime {
    /// Loads `library`: a file name resolved by the platform loader, a path, or
    /// [`RUNTIME_LIBRARY_SELF`] for a runtime linked into the running program.
    pub fn open(library: &str) -> Result<Arc<Self>, EngineError> {
        if library.is_empty() || library.contains('\0') {
            return Err(EngineError::InvalidArgument("runtime library name is empty or contains NUL".into()));
        }
        let loaded = if library == RUNTIME_LIBRARY_SELF {
            this_program()
        } else {
            // SAFETY: loading runs the library's initializers. The app chooses this library; it
            // is the sherpa-onnx C API it ships, not a downloaded file.
            unsafe { Library::new(library) }
        }
        .map_err(|e| EngineError::RuntimeMissing(format!("{library}: {e}")))?;
        // SAFETY: each symbol is cast to the signature declared by the 1.13.8 `c-api.h`; the
        // version check below rejects libraries whose structs may differ.
        unsafe {
            let version = match loaded.get::<GetVersion>(b"SherpaOnnxGetVersionStr\0") {
                Ok(get) => {
                    let raw = get();
                    if raw.is_null() { String::new() } else { CStr::from_ptr(raw).to_string_lossy().into_owned() }
                }
                Err(_) => "unknown (no SherpaOnnxGetVersionStr)".into(),
            };
            if version.trim() != SHERPA_ONNX_ABI {
                return Err(EngineError::RuntimeVersion { expected: SHERPA_ONNX_ABI.into(), found: version });
            }
            macro_rules! symbol {
                ($name:literal) => {
                    *loaded
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|_| EngineError::RuntimeSymbol($name.into()))?
                };
            }
            Ok(Arc::new(Self {
                create_offline: symbol!("SherpaOnnxCreateOfflineRecognizer"),
                destroy_offline: symbol!("SherpaOnnxDestroyOfflineRecognizer"),
                create_stream: symbol!("SherpaOnnxCreateOfflineStream"),
                destroy_stream: symbol!("SherpaOnnxDestroyOfflineStream"),
                accept: symbol!("SherpaOnnxAcceptWaveformOffline"),
                decode: symbol!("SherpaOnnxDecodeOfflineStream"),
                get_result: symbol!("SherpaOnnxGetOfflineStreamResult"),
                destroy_result: symbol!("SherpaOnnxDestroyOfflineRecognizerResult"),
                version,
                _library: loaded,
            }))
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// The running program's own symbols (the static-linking case).
fn this_program() -> Result<Library, libloading::Error> {
    #[cfg(unix)]
    return Ok(libloading::os::unix::Library::this().into());
    #[cfg(windows)]
    return libloading::os::windows::Library::this().map(Into::into);
}

/// Owns C strings for the duration of recognizer creation.
struct Strings(Vec<CString>);

impl Strings {
    fn text(&mut self, value: &str) -> Result<*const c_char, EngineError> {
        let value = CString::new(value).map_err(|_| EngineError::PackInvalid("string contains NUL".into()))?;
        self.0.push(value);
        Ok(self.0.last().expect("just pushed").as_ptr())
    }

    fn file(&mut self, manifest: &ModelPackManifest, dir: &Path, role: &str) -> Result<*const c_char, EngineError> {
        let path = manifest
            .path_of(dir, role)
            .ok_or_else(|| EngineError::PackInvalid(format!("no {role} file")))?;
        let text = path.to_str().ok_or_else(|| EngineError::PackInvalid("model path is not UTF-8".into()))?;
        self.text(text)
    }
}

/// Languages SenseVoice accepts as a decoder hint.
const SENSE_VOICE_LANGUAGES: [&str; 5] = ["zh", "en", "ja", "ko", "yue"];

/// The decoder language passed to the model: the pack's fixed language, else the request's
/// language when the model knows it, else automatic detection.
///
/// For Whisper the result is always a token from [`crate::WHISPER_LANGUAGES`] or empty
/// (automatic detection): sherpa-onnx 1.13.8 ends the process on any other value.
fn decoder_language(manifest: &ModelPackManifest, language: &str) -> String {
    let whisper = |language: &str| whisper_language(language).unwrap_or_default().to_owned();
    if let Some(fixed) = &manifest.options.language {
        return match manifest.family {
            ModelFamily::Whisper => whisper(fixed),
            _ => fixed.clone(),
        };
    }
    match manifest.family {
        ModelFamily::Whisper if manifest.languages == ["en"] => "en".into(),
        // Name the language for multilingual Whisper: auto-detection on short memos can pick a
        // neighbouring language. The task stays "transcribe", so nothing is translated. A
        // language Whisper has no token for falls back to detection.
        ModelFamily::Whisper if manifest.supports(language) => whisper(language),
        ModelFamily::Whisper => String::new(),
        ModelFamily::SenseVoice if SENSE_VOICE_LANGUAGES.contains(&language) => language.into(),
        ModelFamily::SenseVoice => "auto".into(),
        _ => String::new(),
    }
}

/// One sherpa-onnx offline recognizer for one model pack.
pub struct SherpaRecognizer {
    runtime: Arc<SherpaRuntime>,
    recognizer: *const OfflineRecognizer,
    model_id: String,
    max_segment_ms: Option<u32>,
}

// SAFETY: the recognizer pointer is owned by this value alone, and the sherpa-onnx offline
// recognizer has no thread affinity. Decoding takes `&mut self`, so calls never overlap.
unsafe impl Send for SherpaRecognizer {}

impl SherpaRecognizer {
    /// Creates a recognizer from a validated manifest. Verify the pack's files first
    /// ([`ModelPackManifest::verify`]); this reads them directly.
    pub fn new(
        runtime: Arc<SherpaRuntime>,
        manifest: &ModelPackManifest,
        dir: &Path,
        language: &str,
        num_threads: i32,
    ) -> Result<Self, EngineError> {
        manifest.validate()?;
        let mut strings = Strings(Vec::new());
        // SAFETY: an all-zero config is sherpa's "unset" for every field (null strings, 0 numbers).
        let mut config: OfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        let options = &manifest.options;
        let nemo = options.model_type.as_deref() == Some("nemo_transducer");
        config.feat_config.sample_rate = SAMPLE_RATE as i32;
        config.feat_config.feature_dim = options.feature_dim.unwrap_or(if nemo { 128 } else { 80 });
        config.decoding_method = strings.text("greedy_search")?;
        config.max_active_paths = 4;
        let model = &mut config.model_config;
        model.num_threads = num_threads.clamp(1, 8);
        model.provider = strings.text("cpu")?;
        model.tokens = strings.file(manifest, dir, "tokens")?;
        let decoder_language = decoder_language(manifest, language);
        match manifest.family {
            ModelFamily::Whisper => {
                model.whisper.encoder = strings.file(manifest, dir, "encoder")?;
                model.whisper.decoder = strings.file(manifest, dir, "decoder")?;
                model.whisper.language = strings.text(&decoder_language)?;
                model.whisper.task = strings.text("transcribe")?;
                model.model_type = strings.text("whisper")?;
            }
            ModelFamily::SenseVoice => {
                model.sense_voice.model = strings.file(manifest, dir, "model")?;
                model.sense_voice.language = strings.text(&decoder_language)?;
                model.sense_voice.use_itn = i32::from(options.use_itn.unwrap_or(true));
            }
            ModelFamily::Transducer => {
                model.transducer.encoder = strings.file(manifest, dir, "encoder")?;
                model.transducer.decoder = strings.file(manifest, dir, "decoder")?;
                model.transducer.joiner = strings.file(manifest, dir, "joiner")?;
                model.model_type = strings.text(options.model_type.as_deref().unwrap_or("transducer"))?;
            }
            ModelFamily::Moonshine => {
                model.moonshine.encoder = strings.file(manifest, dir, "encoder")?;
                if manifest.file("merged_decoder").is_some() {
                    model.moonshine.merged_decoder = strings.file(manifest, dir, "merged_decoder")?;
                } else {
                    model.moonshine.preprocessor = strings.file(manifest, dir, "preprocessor")?;
                    model.moonshine.uncached_decoder = strings.file(manifest, dir, "uncached_decoder")?;
                    model.moonshine.cached_decoder = strings.file(manifest, dir, "cached_decoder")?;
                }
            }
        }
        // SAFETY: `config` and every string it points to (`strings`) outlive this call; sherpa
        // copies what it keeps.
        let recognizer = unsafe { (runtime.create_offline)(&config) };
        drop(strings);
        if recognizer.is_null() {
            return Err(EngineError::Init(format!("sherpa-onnx rejected {} ({:?})", manifest.id, manifest.family)));
        }
        let window = match manifest.family {
            ModelFamily::Whisper => Some(28_000),
            _ => None,
        };
        Ok(Self {
            runtime,
            recognizer,
            model_id: manifest.id.clone(),
            max_segment_ms: match (options.max_segment_ms, window) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        })
    }
}

impl Recognizer for SherpaRecognizer {
    fn decode(&mut self, samples: &[f32]) -> Result<String, EngineError> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let count = i32::try_from(samples.len()).map_err(|_| EngineError::InvalidAudio("segment too long".into()))?;
        let api = &self.runtime;
        // SAFETY: the stream belongs to this live recognizer and is destroyed before returning;
        // `samples` outlives `accept`, which copies them. The result's text is copied before the
        // result is destroyed.
        unsafe {
            let stream = (api.create_stream)(self.recognizer);
            if stream.is_null() {
                return Err(EngineError::Decode("cannot create a recognition stream".into()));
            }
            (api.accept)(stream, SAMPLE_RATE as i32, samples.as_ptr(), count);
            (api.decode)(self.recognizer, stream);
            let result = (api.get_result)(stream);
            let text = if result.is_null() || (*result).text.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*result).text).to_string_lossy().into_owned()
            };
            if !result.is_null() {
                (api.destroy_result)(result);
            }
            (api.destroy_stream)(stream);
            Ok(text)
        }
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_segment_ms(&self) -> Option<u32> {
        self.max_segment_ms
    }
}

impl Drop for SherpaRecognizer {
    fn drop(&mut self) {
        // SAFETY: created by this runtime, destroyed exactly once; the Arc keeps the library loaded.
        unsafe { (self.runtime.destroy_offline)(self.recognizer) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::{write_pack, TempDir};

    #[test]
    fn missing_runtime_is_reported_not_fatal() {
        let error = SherpaRuntime::open("libsonojot-no-such-runtime.so").unwrap_err();
        assert_eq!(error.code(), "runtime_missing");
        assert_eq!(SherpaRuntime::open("").unwrap_err().code(), "invalid_argument");
    }

    #[test]
    fn the_running_program_can_stand_in_for_a_static_runtime() {
        // This test binary resolves symbols but has no sherpa-onnx, so the version check refuses it.
        let error = SherpaRuntime::open(RUNTIME_LIBRARY_SELF).unwrap_err();
        assert_eq!(error.code(), "runtime_version", "{error}");
    }

    #[test]
    fn whisper_never_receives_a_language_it_does_not_know() {
        use crate::{language_of, WHISPER_LANGUAGES};
        let dir = TempDir::new("whisper-tokens");
        let mut multi = write_pack(&dir.0, ModelFamily::Whisper, &["*"], &["encoder", "decoder", "tokens"]);
        for (locale, expected) in [("nb-NO", "no"), ("fil-PH", "tl"), ("iw-IL", "he"), ("in-ID", "id"), ("ji", "yi"),
            ("jv-ID", "jw"), ("haw", "haw"), ("ckb-IQ", ""), ("yue-HK", ""), ("zz", ""), ("", "")]
        {
            assert_eq!(decoder_language(&multi, &language_of(locale)), expected, "{locale}");
        }
        let in_table = |token: String| token.is_empty() || WHISPER_LANGUAGES.contains(&token.as_str());
        // Every code language_of can produce: all one-, two- and three-letter ASCII tags.
        let letters: Vec<char> = ('a'..='z').collect();
        let mut tags: Vec<String> = Vec::new();
        for &a in &letters {
            tags.push(a.to_string());
            for &b in &letters {
                tags.push(format!("{a}{b}"));
                tags.extend(letters.iter().map(|c| format!("{a}{b}{c}")));
            }
        }
        for tag in &tags {
            let token = decoder_language(&multi, &language_of(tag));
            assert!(in_table(token.clone()), "{tag} -> {token:?}");
        }
        // A pack that fixes its language: validation refuses unknown tokens, and the decoder
        // maps whatever passes (including aliases such as nb).
        for fixed in ["auto", "nb", "fil", "ar"] {
            multi.options.language = Some(fixed.into());
            multi.validate().unwrap();
            assert!(in_table(decoder_language(&multi, "en")), "{fixed}");
        }
        let regional = write_pack(&dir.0, ModelFamily::Whisper, &["nb", "fil"], &["encoder", "decoder", "tokens"]);
        assert_eq!(decoder_language(&regional, "nb"), "no");
        assert_eq!(decoder_language(&regional, "fil"), "tl");
    }

    #[test]
    fn decoder_language_never_translates_and_respects_the_pack() {
        let dir = TempDir::new("language");
        let mut multi = write_pack(&dir.0, ModelFamily::Whisper, &["*"], &["encoder", "decoder", "tokens"]);
        assert_eq!(decoder_language(&multi, "ar"), "ar");
        assert_eq!(decoder_language(&multi, ""), "");
        multi.options.language = Some("auto".into());
        assert_eq!(decoder_language(&multi, "ar"), "");
        let english = write_pack(&dir.0, ModelFamily::Whisper, &["en"], &["encoder", "decoder", "tokens"]);
        assert_eq!(decoder_language(&english, "en"), "en");
        let sense = write_pack(&dir.0, ModelFamily::SenseVoice, &["en", "zh"], &["model", "tokens"]);
        assert_eq!(decoder_language(&sense, "en"), "en");
        assert_eq!(decoder_language(&sense, "tr"), "auto");
        let moonshine = write_pack(&dir.0, ModelFamily::Moonshine, &["en"], &["encoder", "merged_decoder", "tokens"]);
        assert_eq!(decoder_language(&moonshine, "en"), "");
    }

    #[test]
    fn struct_sizes_match_the_1_13_8_layout_on_64_bit_targets() {
        // Pointer-sized fields dominate; a field added or dropped by mistake changes the size.
        if std::mem::size_of::<usize>() == 8 {
            assert_eq!(std::mem::size_of::<OfflineWhisperModelConfig>(), 48);
            assert_eq!(std::mem::size_of::<OfflineFunASRNanoModelConfig>(), 88);
            assert_eq!(std::mem::size_of::<OfflineQwen3ASRModelConfig>(), 64);
            assert_eq!(std::mem::size_of::<OfflineModelConfig>(), 504);
            assert_eq!(std::mem::size_of::<OfflineRecognizerConfig>(), 608);
        }
    }
}
