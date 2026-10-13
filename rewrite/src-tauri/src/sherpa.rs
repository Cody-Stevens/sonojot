//! Optional sherpa-onnx C API, loaded only after an optional model is installed
//! and a dictation or file job actually selects it.
use crate::{domain::RecognitionCompleteness, optional_asr};
use libloading::Library;
use std::{
    collections::BTreeMap,
    ffi::{c_char, CStr, CString},
    path::{Path, PathBuf},
    ptr,
};

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
#[repr(C)]
struct SileroVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}
#[repr(C)]
struct TenVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}
#[repr(C)]
struct VadModelConfig {
    silero_vad: SileroVadModelConfig,
    sample_rate: i32,
    num_threads: i32,
    provider: *const c_char,
    debug: i32,
    ten_vad: TenVadModelConfig,
}
#[repr(C)]
struct SpeechSegment {
    start: i32,
    samples: *mut f32,
    n: i32,
}
#[repr(C)]
struct OfflineRecognizerResult {
    text: *const c_char,
    // Prefix of the installed 1.13.8 C ABI. Do not infer a decoding finish
    // reason from token count; that API does not expose one.
    timestamps: *mut f32,
    count: i32,
    tokens: *const c_char,
    tokens_arr: *const *const c_char,
    json: *const c_char,
}

enum OfflineRecognizer {}
enum OfflineStreamHandle {}
enum VoiceActivityDetector {}

type CreateOffline =
    unsafe extern "C" fn(*const OfflineRecognizerConfig) -> *const OfflineRecognizer;
type DestroyOffline = unsafe extern "C" fn(*const OfflineRecognizer);
type CreateOfflineStream =
    unsafe extern "C" fn(*const OfflineRecognizer) -> *const OfflineStreamHandle;
type DestroyOfflineStream = unsafe extern "C" fn(*const OfflineStreamHandle);
type AcceptWaveform = unsafe extern "C" fn(*const OfflineStreamHandle, i32, *const f32, i32);
type DecodeOffline = unsafe extern "C" fn(*const OfflineRecognizer, *const OfflineStreamHandle);
type GetResult = unsafe extern "C" fn(*const OfflineStreamHandle) -> *const OfflineRecognizerResult;
type DestroyResult = unsafe extern "C" fn(*const OfflineRecognizerResult);
type CreateVad = unsafe extern "C" fn(*const VadModelConfig, f32) -> *const VoiceActivityDetector;
type DestroyVad = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadAccept = unsafe extern "C" fn(*const VoiceActivityDetector, *const f32, i32);
type VadEmpty = unsafe extern "C" fn(*const VoiceActivityDetector) -> i32;
type VadPop = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadFront = unsafe extern "C" fn(*const VoiceActivityDetector) -> *const SpeechSegment;
type DestroySegment = unsafe extern "C" fn(*const SpeechSegment);
type VadFlush = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadReset = unsafe extern "C" fn(*const VoiceActivityDetector);

struct Api {
    _library: Library,
    _dependencies: Vec<Library>,
    create_offline: CreateOffline,
    destroy_offline: DestroyOffline,
    create_stream: CreateOfflineStream,
    destroy_stream: DestroyOfflineStream,
    accept: AcceptWaveform,
    decode: DecodeOffline,
    get_result: GetResult,
    destroy_result: DestroyResult,
    create_vad: CreateVad,
    destroy_vad: DestroyVad,
    vad_accept: VadAccept,
    vad_empty: VadEmpty,
    vad_detected: VadEmpty,
    vad_pop: VadPop,
    vad_front: VadFront,
    destroy_segment: DestroySegment,
    vad_flush: VadFlush,
    vad_reset: VadReset,
}

impl Api {
    fn open(root: &Path) -> Result<Self, String> {
        let library_path = optional_asr::runtime_library(root);
        if !library_path.is_file() {
            return Err("The optional speech runtime is not installed.".into());
        }
        let mut dependencies = Vec::new();
        #[cfg(target_os = "windows")]
        for name in ["onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
            let dep = library_path.parent().unwrap_or(root).join(name);
            if dep.is_file() {
                dependencies.push(
                    unsafe { Library::new(&dep) }
                        .map_err(|e| format!("Cannot load {}: {e}", dep.display()))?,
                );
            }
        }
        let library = unsafe { Library::new(&library_path) }
            .map_err(|e| format!("Cannot load the optional speech runtime: {e}"))?;
        unsafe {
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("Optional speech runtime is missing {}: {e}", $name))?
                };
            }
            Ok(Self {
                create_offline: symbol!("SherpaOnnxCreateOfflineRecognizer", CreateOffline),
                destroy_offline: symbol!("SherpaOnnxDestroyOfflineRecognizer", DestroyOffline),
                create_stream: symbol!("SherpaOnnxCreateOfflineStream", CreateOfflineStream),
                destroy_stream: symbol!("SherpaOnnxDestroyOfflineStream", DestroyOfflineStream),
                accept: symbol!("SherpaOnnxAcceptWaveformOffline", AcceptWaveform),
                decode: symbol!("SherpaOnnxDecodeOfflineStream", DecodeOffline),
                get_result: symbol!("SherpaOnnxGetOfflineStreamResult", GetResult),
                destroy_result: symbol!("SherpaOnnxDestroyOfflineRecognizerResult", DestroyResult),
                create_vad: symbol!("SherpaOnnxCreateVoiceActivityDetector", CreateVad),
                destroy_vad: symbol!("SherpaOnnxDestroyVoiceActivityDetector", DestroyVad),
                vad_accept: symbol!("SherpaOnnxVoiceActivityDetectorAcceptWaveform", VadAccept),
                vad_empty: symbol!("SherpaOnnxVoiceActivityDetectorEmpty", VadEmpty),
                vad_detected: symbol!("SherpaOnnxVoiceActivityDetectorDetected", VadEmpty),
                vad_pop: symbol!("SherpaOnnxVoiceActivityDetectorPop", VadPop),
                vad_front: symbol!("SherpaOnnxVoiceActivityDetectorFront", VadFront),
                destroy_segment: symbol!("SherpaOnnxDestroySpeechSegment", DestroySegment),
                vad_flush: symbol!("SherpaOnnxVoiceActivityDetectorFlush", VadFlush),
                vad_reset: symbol!("SherpaOnnxVoiceActivityDetectorReset", VadReset),
                _library: library,
                _dependencies: dependencies,
            })
        }
    }
}

enum Decoder {
    Native(*const OfflineRecognizer),
    Llama(crate::llama_asr::Server),
}

pub struct Engine {
    api: Api,
    decoder: Decoder,
    vad: *const VoiceActivityDetector,
    _keep: Vec<CString>,
    pub model_id: String,
    pub requested_model: String,
}

impl Engine {
    pub fn load(root: &Path, requested: &str) -> Result<Self, String> {
        // Whisper checkpoints live in whisper_catalog but run on the same
        // pinned sherpa CPU runtime. Native Qwen int8 graphs also stay on CPU;
        // llama.cpp owns its CUDA decoder and uses this CPU runtime only for VAD.
        // Actual gpu_asr models must never fall back to the CPU decoder here.
        if crate::gpu_asr::is_gpu_model(requested) {
            return Err(format!(
                "GPU model {requested} needs the CUDA adapter; not a CPU fallback."
            ));
        }
        let whisper = crate::whisper_catalog::find(requested);
        let optional = optional_asr::find(requested);
        let llama = crate::llama_asr::is_llama_asr(requested);
        if whisper.is_none() && optional.is_none() && !llama {
            return Err(format!("Unsupported recognizer: {requested}"));
        }
        if llama {
            if !crate::llama_asr::model_ready(root, requested) {
                return Err(format!("{requested} is not installed"));
            }
        } else if let Some(w) = whisper {
            if !crate::whisper_catalog::model_ready(root, w.id) {
                return Err(format!("{requested} is not installed"));
            }
        } else if let Some(model) = optional {
            if !optional_asr::model_ready(root, model.id) {
                return Err(format!("{requested} is not installed"));
            }
        }
        let api = Api::open(root)?;
        let dir = if llama {
            crate::llama_asr::model_dir(root, requested)
        } else if whisper.is_some() {
            crate::whisper_catalog::model_dir(root, requested)
        } else {
            optional_asr::model_dir(root, requested)
        };
        let mut keep = Vec::new();
        let path = |keep: &mut Vec<CString>, value: PathBuf| -> Result<*const c_char, String> {
            keep.push(crate::asr::native_model_path(&value)?);
            Ok(keep.last().unwrap().as_ptr())
        };
        let intern = |keep: &mut Vec<CString>, value: &str| -> *const c_char {
            keep.push(CString::new(value).expect("static interned strings"));
            keep.last().unwrap().as_ptr()
        };
        let mut config: OfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = 16_000;
        config.feat_config.feature_dim = 128;
        config.model_config.num_threads = 2;
        config.model_config.provider = intern(&mut keep, "cpu");
        config.decoding_method = intern(&mut keep, "greedy_search");
        if crate::whisper_catalog::is_whisper(requested) {
            // Offline Whisper via sherpa: transcribe only, never translate.
            // `.en` checkpoints are English-only; multilingual checkpoints
            // auto-detect so non-English text is preserved, not translated.
            let whisper = crate::whisper_catalog::find(requested).unwrap();
            config.model_config.whisper.encoder = path(&mut keep, dir.join("encoder.int8.onnx"))?;
            config.model_config.whisper.decoder = path(&mut keep, dir.join("decoder.int8.onnx"))?;
            config.model_config.whisper.language = intern(
                &mut keep,
                crate::asr::sherpa_whisper_language(whisper.english_only),
            );
            config.model_config.whisper.task = intern(&mut keep, "transcribe");
            config.model_config.whisper.tail_paddings = 0;
            config.model_config.whisper.enable_token_timestamps = 0;
            config.model_config.whisper.enable_segment_timestamps = 0;
            config.model_config.tokens = path(&mut keep, dir.join("tokens.txt"))?;
            config.model_config.model_type = intern(&mut keep, "whisper");
        } else if requested == "parakeet-tdt-0.6b-v2" {
            config.model_config.transducer.encoder =
                path(&mut keep, dir.join("encoder.int8.onnx"))?;
            config.model_config.transducer.decoder =
                path(&mut keep, dir.join("decoder.int8.onnx"))?;
            config.model_config.transducer.joiner = path(&mut keep, dir.join("joiner.int8.onnx"))?;
            config.model_config.tokens = path(&mut keep, dir.join("tokens.txt"))?;
            config.model_config.model_type = intern(&mut keep, "nemo_transducer");
        } else if !llama {
            config.model_config.qwen3_asr.conv_frontend =
                path(&mut keep, dir.join("conv_frontend.onnx"))?;
            config.model_config.qwen3_asr.encoder = path(&mut keep, dir.join("encoder.int8.onnx"))?;
            config.model_config.qwen3_asr.decoder = path(&mut keep, dir.join("decoder.int8.onnx"))?;
            config.model_config.qwen3_asr.tokenizer = path(&mut keep, dir.join("tokenizer"))?;
            config.model_config.qwen3_asr.max_total_len = 512;
            config.model_config.qwen3_asr.max_new_tokens = 256;
            config.model_config.qwen3_asr.temperature = 1e-6;
            config.model_config.qwen3_asr.top_p = 0.8;
            config.model_config.qwen3_asr.seed = 42;
        }
        // Resolve every fallible path before allocating native resources.
        let mut vad_config: VadModelConfig = unsafe { std::mem::zeroed() };
        vad_config.silero_vad.model = path(&mut keep, optional_asr::vad_path(root))?;
        vad_config.silero_vad.threshold = if is_qwen(requested) { crate::qwen_window::vad_threshold() } else { 0.25 };
        vad_config.silero_vad.min_silence_duration = 0.5;
        vad_config.silero_vad.min_speech_duration = 0.25;
        vad_config.silero_vad.window_size = 512;
        vad_config.silero_vad.max_speech_duration = crate::asr::sherpa_section_seconds() as f32;
        vad_config.sample_rate = 16_000;
        vad_config.num_threads = 1;
        vad_config.provider = intern(&mut keep, "cpu");
        let decoder = if llama {
            Decoder::Llama(crate::llama_asr::Server::launch(root, requested)?)
        } else {
            let recognizer = unsafe { (api.create_offline)(&config) };
            if recognizer.is_null() {
                return Err(format!("Cannot initialize speech recognition: {requested}"));
            }
            Decoder::Native(recognizer)
        };
        let vad = unsafe { (api.create_vad)(&vad_config, 30.0) };
        if vad.is_null() {
            if let Decoder::Native(recognizer) = &decoder {
                unsafe { (api.destroy_offline)(*recognizer) };
            }
            // A llama server is dropped here and shuts down its owned child.
            return Err(
                "Cannot initialize voice-activity detection for the optional speech model.".into(),
            );
        }
        Ok(Self {
            api,
            decoder,
            vad,
            _keep: keep,
            model_id: requested.to_owned(),
            requested_model: requested.into(),
        })
    }

    pub fn stream(&mut self) -> Result<Stream<'_>, String> {
        unsafe { (self.api.vad_reset)(self.vad) };
        let qwen = is_qwen(&self.model_id);
        let seconds = crate::qwen_window::target_seconds_for_model(&self.model_id);
        Ok(Stream {
            engine: self,
            pending: Vec::new(),
            gate_pending: Vec::new(),
            qwen_input: qwen.then(QwenInput::new),
            windows: qwen.then(|| crate::qwen_window::Assembler::new(seconds)),
            merged_windows: seconds != 0,
            text_guard: crate::qwen_guard::Session::default(),
            ledger: BTreeMap::new(),
            original_text: None,
            next_id: 1,
            vad_samples_since_segment: 0,
            vad_samples: 0,
            vad_origin: 0,
            completeness: RecognitionCompleteness::Complete,
            warnings: Vec::new(),
            #[cfg(feature = "fidelity-eval")]
            segment_evidence: Vec::new(),
            #[cfg(feature = "fidelity-eval")]
            vad_evidence: VadEvidence::default(),
        })
    }

    #[cfg(not(feature = "fidelity-eval"))]
    fn transcribe(&self, samples: &[f32]) -> Result<crate::asr::RecognitionResult, String> {
        self.transcribe_observed(samples, None)
    }

    /// One server request or decode on a fresh native stream. Native token
    /// pieces are copied out before destruction; `tokens_arr` holds `count`
    /// NUL-terminated entries per the installed 1.13.8 C ABI.
    fn decode_once(&self, samples: &[f32]) -> Result<NativeSegment, String> {
        let recognizer = match &self.decoder {
            Decoder::Llama(server) => {
                return server.transcribe(samples).map(NativeSegment::from_llama_response);
            }
            Decoder::Native(recognizer) => *recognizer,
        };
        let stream = unsafe { (self.api.create_stream)(recognizer) };
        if stream.is_null() {
            return Err("Cannot start optional-model recognition.".into());
        }
        unsafe {
            (self.api.accept)(stream, 16_000, samples.as_ptr(), samples.len() as i32);
            (self.api.decode)(recognizer, stream);
        }
        let result = unsafe { (self.api.get_result)(stream) };
        let segment = if result.is_null() {
            NativeSegment::default()
        } else {
            let raw = unsafe { (*result).text };
            let text = if raw.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(raw) }
                    .to_string_lossy()
                    .into_owned()
            };
            let token_count = unsafe { (*result).count };
            let pieces = unsafe { (*result).tokens_arr };
            let token_copy = if !is_qwen(&self.model_id) {
                Ok(None)
            } else {
                checked_qwen_token_count(token_count).and_then(|count| {
                    if count == 0 {
                        return Ok(None);
                    }
                    if pieces.is_null() {
                        return Err(
                            "Optional-model recognition returned tokens without a token array."
                                .into(),
                        );
                    }
                    let pieces = unsafe { std::slice::from_raw_parts(pieces, count) };
                    let mut tokens = Vec::with_capacity(count);
                    for piece in pieces {
                        if piece.is_null() {
                            return Err(
                                "Optional-model recognition returned a null token piece.".into()
                            );
                        }
                        tokens.push(
                            unsafe { CStr::from_ptr(*piece) }
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                    Ok(Some(tokens))
                })
            };
            unsafe { (self.api.destroy_result)(result) };
            let tokens = match token_copy {
                Ok(tokens) => tokens,
                Err(error) => {
                    unsafe { (self.api.destroy_stream)(stream) };
                    return Err(error);
                }
            };
            NativeSegment {
                text,
                tokens,
                token_count,
                ..Default::default()
            }
        };
        unsafe { (self.api.destroy_stream)(stream) };
        Ok(segment)
    }

    fn transcribe_observed(
        &self,
        samples: &[f32],
        mut evidence: Option<&mut serde_json::Value>,
    ) -> Result<crate::asr::RecognitionResult, String> {
        #[cfg(windows)]
        if is_qwen(&self.model_id) && !confirm_speech(samples, evidence.as_deref_mut())? {
            return Ok(crate::asr::RecognitionResult { original_text: None, segments: Vec::new(), text: String::new(),
                completeness: RecognitionCompleteness::Complete, warnings: Vec::new() });
        }
        self.transcribe_qualified(samples, evidence)
    }

    fn transcribe_qualified(
        &self, samples: &[f32], evidence: Option<&mut serde_json::Value>,
    ) -> Result<crate::asr::RecognitionResult, String> {
        if let Decoder::Llama(server) = &self.decoder {
            return transcribe_llama(server, samples, evidence);
        }
        if is_qwen(&self.model_id) {
            return transcribe_qwen_native(samples, 256, || self.decode_once(samples)
                .map(|segment| (segment.text, segment.tokens, segment.token_count)), evidence);
        }
        transcribe_decoded(samples, false, || self.decode_once(samples), None, evidence)
    }
}

/// The same protocol, retry and truncation policy for Windows VAD and Mac
/// sectioning. The caller owns cancellation and the section's timestamps.
pub(crate) fn transcribe_llama(
    server: &crate::llama_asr::Server,
    samples: &[f32],
    evidence: Option<&mut serde_json::Value>,
) -> Result<crate::asr::RecognitionResult, String> {
    transcribe_decoded(samples, true,
        || server.transcribe(samples).map(NativeSegment::from_llama_response),
        Some(server), evidence)
}

/// Native adapters supply raw text, optional copied token pieces and the
/// native count. The caller still owns stream lifetime and cancellation;
/// this boundary owns protocol parsing, guards and the one permitted retry.
pub(crate) fn transcribe_qwen_native(
    samples: &[f32],
    max_new_tokens: u32,
    mut decode: impl FnMut() -> Result<(String, Option<Vec<String>>, i32), String>,
    evidence: Option<&mut serde_json::Value>,
) -> Result<crate::asr::RecognitionResult, String> {
    // The CPU and CUDA adapters have different configured native budgets.
    let mut details = serde_json::Value::Null;
    let observed = evidence.is_some();
    let result = transcribe_decoded(samples, true, || decode().map(|(text, tokens, token_count)| {
        NativeSegment { text, tokens, token_count, ..Default::default() }
    }), None, observed.then_some(&mut details))?;
    if let Some(evidence) = evidence {
        if details.get("native_token_count").is_some() {
            details["configured_max_new_tokens"] = serde_json::json!(max_new_tokens);
            details["token_limit_suspected"] = serde_json::json!(
                details["native_token_count"].as_u64().is_some_and(|count| count >= u64::from(max_new_tokens))
            );
        }
        *evidence = details;
    }
    Ok(result)
}

/// Qwen output budget for one 16 kHz mono section: eight tokens per second
/// plus room for the language header and a short answer. Integer arithmetic
/// keeps the ceiling exact and cannot overflow on an untrusted sample count.
pub(crate) fn qwen_max_tokens(sample_count: usize) -> u32 {
    (sample_count.min(60 * 16_000).div_ceil(2_000) as u32 + 32).clamp(32, 512)
}

/// Second presence check for speech already emitted by Windows Silero. Keep
/// this at the native adapter boundary: shared text guards also serve Mac,
/// whose caller owns its independent Silero/fallback gate.
#[cfg(any(windows, target_os = "macos", test))]
pub(crate) fn confirm_speech(samples: &[f32], evidence: Option<&mut serde_json::Value>) -> Result<bool, String> {
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_SPECTRAL_GATE").as_deref() == Ok("off") { return Ok(true); }
    let gate = crate::mac_speech::speech_gate::decide(
        crate::mac_speech::speech_gate::Mode::Confirmation, samples, None,
        || Err("Silero section already qualified.".into()),
    )?;
    if !gate.decode {
        if let Some(evidence) = evidence {
            *evidence = serde_json::json!({"decoded":false, "skipped":true,
                "spectral_confirmation":gate, "audio_samples":samples.len()});
        }
    }
    Ok(gate.decode)
}

pub(crate) fn qwen_guards_enabled() -> bool {
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_QWEN_GUARDS").as_deref() == Ok("off") {
        return false;
    }
    true
}

fn transcribe_decoded(
    samples: &[f32],
    qwen: bool,
    mut decode: impl FnMut() -> Result<NativeSegment, String>,
    server: Option<&crate::llama_asr::Server>,
    evidence: Option<&mut serde_json::Value>,
) -> Result<crate::asr::RecognitionResult, String> {
    if samples.is_empty() {
        return Ok(crate::asr::RecognitionResult {
            original_text: None,
            segments: Vec::new(),
            text: String::new(),
            completeness: RecognitionCompleteness::Complete,
            warnings: Vec::new(),
        });
    }
    let duration = samples.len() as f64 / 16_000.0;
    let first = decode();
    let segment;
    let mut protocol = "none";
    let mut token_provenance = serde_json::Value::Null;
    let parsed;
    let mut original_text = None;
    let mut completeness = RecognitionCompleteness::Complete;
    let mut warnings = Vec::new();
    let mut attempts = Vec::new();
    let mut output_guard = serde_json::Value::Null;
    if qwen {
        let resolved = qwen_resolve_decode(first, duration, decode);
        if evidence.is_some() {
            output_guard = serde_json::json!(resolved.boundary.guard);
        }
        segment = resolved.segment;
        protocol = resolved.protocol;
        token_provenance = serde_json::json!(resolved.boundary.token_provenance);
        original_text = resolved.boundary.original_text;
        parsed = resolved.boundary.text;
        completeness = resolved.completeness;
        warnings = resolved.warnings;
        attempts = resolved.attempts;
    } else {
        segment = first?;
        parsed = segment.text.clone();
    }
    if let Some(evidence) = evidence {
        let count = segment.token_count;
        let llama = server.is_some() || segment.finish_reason.is_some();
        let truncation = if attempts.iter().any(|attempt| attempt["finish_reason"] == "length") {
            Some(true)
        } else {
            segment.finish_reason.as_deref().map(|reason| reason == "length")
        };
        if let Some(server) = server {
            for attempt in &mut attempts {
                attempt["server_pid"] = serde_json::json!(server.pid());
                attempt["backend"] = serde_json::json!(server.backend());
            }
        }
        *evidence = serde_json::json!({
            "audio_samples":samples.len(),
            "duration_seconds":duration,
            "native_token_count":if llama {None} else {Some(count)},
            "completion_tokens":segment.completion_tokens,
            "finish_reason":segment.finish_reason,
            "configured_max_new_tokens":if llama {Some(qwen_max_tokens(samples.len()))} else if qwen {Some(256)} else {None},
            "token_limit_suspected":if llama {truncation == Some(true)} else {qwen && count>=256},
            "truncation_detected":truncation,
            "truncation_status":if llama {"server finish reason recorded per attempt"} else {
                "native finish reason unavailable; token count is diagnostic only"
            },
            "native_text":segment.text,
            "detected_language":if qwen && protocol != "none" { crate::qwen_guard::detected_language(&segment.text) } else { None },
            "native_tokens":segment.tokens,
            "parsed_text":&parsed,
            "protocol":protocol,
            "token_provenance":token_provenance,
            "qwen_output_guard":output_guard,
            "recognition_completeness":completeness,
            "warnings":&warnings,
        });
        if let Some(server) = server {
            evidence["server_pid"] = serde_json::json!(server.pid());
            evidence["backend"] = serde_json::json!(server.backend());
        }
        if !attempts.is_empty() {
            evidence["attempts"] = serde_json::json!(attempts);
        }
    }
    Ok(crate::asr::RecognitionResult {
        original_text,
        segments: Vec::new(),
        text: parsed,
        completeness,
        warnings,
    })
}

/// Output owned by the adapter after one native decode or server request.
#[derive(Default)]
struct NativeSegment {
    text: String,
    tokens: Option<Vec<String>>,
    token_count: i32,
    completion_tokens: Option<i64>,
    finish_reason: Option<String>,
}

impl NativeSegment {
    fn from_llama_response(
        (text, completion_tokens, finish_reason): (String, Option<i64>, String),
    ) -> Self {
        Self {
            text,
            completion_tokens,
            finish_reason: Some(finish_reason),
            ..Default::default()
        }
    }

    fn truncated(&self) -> bool {
        self.finish_reason.as_deref() == Some("length")
    }
}

fn is_qwen(id: &str) -> bool {
    id == "qwen3-asr-1.7b" || crate::llama_asr::is_llama_asr(id)
}

/// Outcome of parsing one native Qwen segment at the decoder/adapter boundary.
struct QwenBoundary {
    /// Speech after a leading protocol header is removed; raw text otherwise.
    text: String,
    original_text: Option<String>,
    /// "none" | "header_stripped" | "bare_marker_stripped"
    protocol: &'static str,
    /// Native evidence used at the boundary without inferring token kind from shape.
    token_provenance: &'static str,
    /// Conservative review reason when protocol and literal speech are indistinguishable.
    review_reason: Option<&'static str>,
    guard: QwenOutputGuard,
}

#[derive(Default, serde::Serialize)]
struct QwenOutputGuard {
    enabled: bool,
    action: &'static str,
    reason: Option<&'static str>,
    word_count: usize,
    repeated_words: usize,
    non_loop_words: usize,
    repeated_coverage: f64,
    loops: Vec<QwenLoop>,
    token_count: Option<u64>,
    token_count_source: &'static str,
    tokens_per_audio_second: Option<f64>,
    rate_token_allowance: f64,
    rate_exceeded: bool,
    rate_exemption: Option<&'static str>,
}

impl QwenOutputGuard {
    fn intervened(&self) -> bool {
        self.enabled && self.reason.is_some()
    }
}

#[derive(serde::Serialize)]
struct QwenLoop {
    start_word: usize,
    ngram_words: usize,
    repetitions: usize,
}

struct QwenWord {
    start: usize,
    normalized: String,
}

fn qwen_words(text: &str) -> Vec<QwenWord> {
    let mut words = Vec::new();
    let mut start = None;
    let mut normalized = String::new();
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((offset, ch)) = chars.next() {
        // Keep apostrophes inside words and punctuation inside numbers. A
        // decimal, version, date or time must not become a loop of its digits.
        let internal = start.is_some()
            && chars.peek().is_some_and(|(_, next)| {
                (matches!(ch, '\'' | '’') && next.is_alphanumeric())
                    || (matches!(ch, '.' | ',' | ':' | '/' | '-')
                        && previous.is_some_and(|prev: char| prev.is_numeric())
                        && next.is_numeric())
            });
        if ch.is_alphanumeric() || internal {
            start.get_or_insert(offset);
            normalized.extend(if ch == '’' { '\'' } else { ch }.to_lowercase());
        } else if let Some(start) = start.take() {
            words.push(QwenWord { start, normalized: std::mem::take(&mut normalized) });
        }
        previous = Some(ch);
    }
    if let Some(start) = start {
        words.push(QwenWord { start, normalized });
    }
    words
}

fn qwen_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff
        | 0xf900..=0xfaff | 0x20000..=0x323af | 0x1100..=0x11ff
        | 0x3130..=0x318f | 0xa960..=0xa97f | 0xac00..=0xd7ff | 0xff66..=0xff9f)
}

fn qwen_guard_boundary(
    segment: &NativeSegment,
    audio_seconds: f64,
    enabled: bool,
) -> QwenBoundary {
    let mut boundary = qwen_boundary_parse(&segment.text, segment.tokens.as_deref(), audio_seconds);
    let words = qwen_words(&boundary.text);
    let mut loops = Vec::new();
    let mut repeated_words = 0;
    let mut collapsed = String::new();
    let mut cursor = 0;
    let mut start = 0;
    while start < words.len() {
        let mut longest = None;
        for size in 1..=6.min((words.len() - start) / 4) {
            let mut end = start + size;
            while end + size <= words.len()
                && (0..size).all(|n| words[start + n].normalized == words[end + n].normalized)
            {
                end += size;
            }
            if (end - start) / size >= 4
                && longest.is_none_or(|(_, old_end)| end > old_end)
            {
                longest = Some((size, end));
            }
        }
        if let Some((size, end)) = longest {
            loops.push(QwenLoop {
                start_word: start,
                ngram_words: size,
                repetitions: (end - start) / size,
            });
            repeated_words += end - start;
            // Preserve the first occurrence's spelling and punctuation. Only
            // remove later occurrences, up to the next non-loop word.
            collapsed.push_str(&boundary.text[cursor..words[start + size].start]);
            cursor = words.get(end).map_or(boundary.text.len(), |word| word.start);
            start = end;
        } else {
            start += 1;
        }
    }
    let non_loop_words = words.len() - repeated_words;
    let coverage = if words.is_empty() { 0.0 } else { repeated_words as f64 / words.len() as f64 };
    let (token_count, token_count_source) = if segment.finish_reason.is_some() {
        (segment.completion_tokens.and_then(|n| u64::try_from(n).ok()), "completion_tokens")
    } else {
        (u64::try_from(segment.token_count).ok().filter(|n| *n > 0), "native_token_count")
    };
    let rate_token_allowance = audio_seconds * 20.0 + 32.0;
    let rate_exceeded = token_count.is_some_and(|n| n as f64 > rate_token_allowance);
    // Tokenization cost varies across scripts. A rate outlier is evidence,
    // not permission to erase CJK speech or a genuinely short answer.
    let rate_exemption = if boundary.text.chars().any(qwen_cjk) {
        Some("cjk_text")
    } else if words.len() <= 3
        && boundary.text.chars().filter(|ch| !ch.is_whitespace()).take(65).count() <= 64
    {
        Some("short_response")
    } else {
        None
    };
    let (action, reason) = if !loops.is_empty() {
        if segment.truncated() {
            ("drop", Some("repetition_at_token_limit"))
        } else if non_loop_words <= 3 || repeated_words * 5 >= words.len() * 4 {
            ("drop", Some("mostly_repetition"))
        } else {
            ("collapse", Some("consecutive_repetition"))
        }
    } else if rate_exceeded && rate_exemption.is_none() {
        ("review", Some("extreme_token_rate"))
    } else {
        ("none", None)
    };
    boundary.guard = QwenOutputGuard {
        enabled,
        action: if enabled { action } else { "bypassed" },
        reason,
        word_count: words.len(),
        repeated_words,
        non_loop_words,
        repeated_coverage: coverage,
        loops,
        token_count,
        token_count_source,
        tokens_per_audio_second: token_count.filter(|_| audio_seconds > 0.0)
            .map(|n| n as f64 / audio_seconds),
        rate_token_allowance,
        rate_exceeded,
        rate_exemption,
    };
    if enabled && matches!(action, "drop" | "collapse") {
        boundary.original_text = Some(boundary.text.clone());
    }
    if enabled && action == "drop" {
        boundary.text.clear();
    } else if enabled && action == "collapse" {
        collapsed.push_str(&boundary.text[cursor..]);
        boundary.text = collapsed.trim().to_owned();
    }
    boundary
}

struct QwenResolution {
    segment: NativeSegment,
    boundary: QwenBoundary,
    protocol: &'static str,
    completeness: RecognitionCompleteness,
    warnings: Vec<String>,
    attempts: Vec<serde_json::Value>,
}

const QWEN_MARKER: &str = "<asr_text>";
const MAX_QWEN_NATIVE_TOKENS: usize = 4_096;
/// Header-only output on audio longer than this is a protocol failure, not a
/// genuinely empty utterance.
const QWEN_HEADER_ONLY_SECONDS: f64 = 1.0;

/// Exercise the production response mapping and Qwen resolution without a
/// server. Ambiguous output replays the supplied response for the one retry.
#[cfg(test)]
pub(crate) fn llama_test_recognition(
    response: (String, Option<i64>, String),
) -> crate::asr::RecognitionResult {
    transcribe_decoded(&vec![0.1; 4 * 16_000], true,
        || Ok(NativeSegment::from_llama_response(response.clone())), None, None).unwrap()
}

/// Qwen emits a structural header at the start of each native segment. Parse
/// it only at that boundary: a bare marker, a known language name plus the
/// marker, or one alphabetic language-label word (`language`, `língua`,
/// `langue`, ...) plus a known language name plus the marker. More than two
/// words before the marker, or any marker appearing later, is spoken text.
/// When native token pieces exist they decide: the marker is protocol only as
/// a whole special token, not as sub-word pieces. Never search-and-delete the
/// marker from the middle of text.
fn qwen_boundary_parse(
    native: &str,
    tokens: Option<&[String]>,
    audio_seconds: f64,
) -> QwenBoundary {
    let tokens = tokens.filter(|t| !t.is_empty());
    let mut token_provenance = if tokens.is_some() {
        "native_token_pieces"
    } else {
        "text_only"
    };
    let mut protocol = "none";
    let mut text = native.to_owned();
    if let Some((_, bare)) = qwen_leading_header(native) {
        if tokens.map_or(true, qwen_marker_is_whole_token) {
            protocol = if bare {
                "bare_marker_stripped"
            } else {
                "header_stripped"
            };
            text = qwen_transcript(native).to_owned();
            if tokens.is_some() {
                token_provenance = "whole_marker_token";
            }
        }
    }
    let trimmed = text.trim_start();
    let mut review_reason =
        if protocol == "none" && trimmed.starts_with('<') && !trimmed.contains('>') {
            Some("unterminated angle-prefixed output is indistinguishable from literal speech")
        } else {
            None
        };
    // `language None<asr_text>` is Qwen's documented empty-audio sentinel, so
    // it is a genuine no-speech result at any duration and never retried.
    let empty_sentinel = qwen_leading_header(native).is_some_and(|_| {
        native
            .trim_start()
            .split_once(QWEN_MARKER)
            .and_then(|(prefix, _)| prefix.split_whitespace().last())
            .is_some_and(|language| language.eq_ignore_ascii_case("None"))
    });
    if review_reason.is_none()
        && protocol != "none"
        && !empty_sentinel
        && text.trim().is_empty()
        && audio_seconds > QWEN_HEADER_ONLY_SECONDS
    {
        review_reason = Some("protocol header contained no speech on substantial audio");
    }
    QwenBoundary {
        text,
        original_text: None,
        protocol,
        token_provenance,
        review_reason,
        guard: QwenOutputGuard::default(),
    }
}

fn qwen_resolve_decode<F>(
    first: Result<NativeSegment, String>,
    audio_seconds: f64,
    retry: F,
) -> QwenResolution
where
    F: FnOnce() -> Result<NativeSegment, String>,
{
    qwen_resolve_decode_with_guards(first, audio_seconds, qwen_guards_enabled(), retry)
}

fn qwen_resolve_decode_with_guards<F>(
    first: Result<NativeSegment, String>,
    audio_seconds: f64,
    guards_enabled: bool,
    retry: F,
) -> QwenResolution
where
    F: FnOnce() -> Result<NativeSegment, String>,
{
    let error = match first {
        Ok(segment) => return qwen_resolve_segment_with_guards(segment, audio_seconds, guards_enabled, retry),
        Err(error) => error,
    };
    let mut attempts = vec![serde_json::json!({"decode_error":&error})];
    let (segment, retry_error) = match retry() {
        Ok(segment) => (segment, None),
        Err(error) => (NativeSegment::default(), Some(error)),
    };
    let boundary = qwen_guard_boundary(&segment, audio_seconds, guards_enabled);
    if let Some(error) = &retry_error {
        attempts.push(serde_json::json!({"retry_error":error}));
    } else {
        attempts.push(qwen_attempt_evidence(&segment, &boundary));
    }
    if boundary.guard.intervened() {
        let truncated = segment.truncated();
        return qwen_guard_resolution(segment, boundary, attempts, truncated);
    }
    let recovered = retry_error.is_none()
        && boundary.review_reason.is_none()
        && !boundary.text.trim().is_empty();
    let truncated = segment.truncated();
    qwen_finish_resolution(QwenResolution {
        segment,
        boundary,
        protocol: if recovered {
            "decode_retry_ok"
        } else {
            "review_required"
        },
        completeness: if recovered {
            RecognitionCompleteness::Complete
        } else {
            RecognitionCompleteness::Incomplete
        },
        warnings: if recovered {
            Vec::new()
        } else {
            vec![format!("qwen3-asr-1.7b segment decode failed ({error}); one retry did not recover complete speech ({}). Available segments were retained for review.",
                retry_error.as_deref().unwrap_or("empty or ambiguous output"))]
        },
        attempts,
    }, truncated)
}

#[cfg(test)]
fn qwen_resolve_segment<F>(
    segment: NativeSegment,
    audio_seconds: f64,
    retry: F,
) -> QwenResolution
where
    F: FnOnce() -> Result<NativeSegment, String>,
{
    qwen_resolve_segment_with_guards(segment, audio_seconds, qwen_guards_enabled(), retry)
}

fn qwen_resolve_segment_with_guards<F>(
    mut segment: NativeSegment,
    audio_seconds: f64,
    guards_enabled: bool,
    retry: F,
) -> QwenResolution
where
    F: FnOnce() -> Result<NativeSegment, String>,
{
    let mut boundary = qwen_guard_boundary(&segment, audio_seconds, guards_enabled);
    let mut protocol = boundary.protocol;
    let mut completeness = RecognitionCompleteness::Complete;
    let mut warnings = Vec::new();
    let mut attempts = Vec::new();
    let mut truncated = segment.truncated();
    // A detected loop or rate failure never earns a fresh decode. In
    // particular, an ambiguous '<' prefix must not route around the guard.
    if boundary.guard.intervened() {
        attempts.push(qwen_attempt_evidence(&segment, &boundary));
        return qwen_guard_resolution(segment, boundary, attempts, truncated);
    }
    if boundary.review_reason.is_some() {
        attempts.push(qwen_attempt_evidence(&segment, &boundary));
        match retry() {
            Ok(next) => {
                truncated |= next.truncated();
                let next_boundary = qwen_guard_boundary(&next, audio_seconds, guards_enabled);
                attempts.push(qwen_attempt_evidence(&next, &next_boundary));
                // A dropped retry must not revive the first ambiguous text.
                if next_boundary.guard.intervened() {
                    return qwen_guard_resolution(next, next_boundary, attempts, truncated);
                }
                if next_boundary.review_reason.is_none() && !next_boundary.text.trim().is_empty() {
                    protocol = "review_retry_ok";
                } else {
                    let reason = next_boundary
                        .review_reason
                        .unwrap_or("fresh-stream retry returned no speech after ambiguous output");
                    warnings.push(format!(
                        "qwen3-asr-1.7b output still needs review after one fresh-stream retry ({reason}); available text was retained."
                    ));
                    protocol = "review_required";
                    completeness = RecognitionCompleteness::Incomplete;
                }
                // An empty retry keeps the first text and its finish metadata
                // together; both attempts remain in the evidence.
                if !next_boundary.text.trim().is_empty() {
                    segment = next;
                    boundary = next_boundary;
                }
            }
            Err(error) => {
                attempts.push(serde_json::json!({"retry_error":&error}));
                warnings.push(format!(
                    "qwen3-asr-1.7b review retry failed ({error}); available text was retained."
                ));
                protocol = "review_required";
                completeness = RecognitionCompleteness::Incomplete;
            }
        }
    }
    qwen_finish_resolution(QwenResolution {
        segment,
        boundary,
        protocol,
        completeness,
        warnings,
        attempts,
    }, truncated)
}

fn qwen_guard_resolution(
    segment: NativeSegment,
    mut boundary: QwenBoundary,
    attempts: Vec<serde_json::Value>,
    truncated: bool,
) -> QwenResolution {
    // A length limit on either attempt plus repetition is a hallucination;
    // retaining the collapsed loop would still publish suspect text.
    if truncated && !boundary.guard.loops.is_empty() {
        boundary.text.clear();
        boundary.guard.action = "drop";
        boundary.guard.reason = Some("repetition_at_token_limit");
    }
    let warning = match boundary.guard.action {
        "drop" => "Qwen3-ASR returned a repeating loop. This section was omitted; review the recording for missing speech.",
        "collapse" => "Qwen3-ASR returned a repeating loop. Repeated text was reduced to its first occurrence; review the recording.",
        _ => "Qwen3-ASR returned unusually many tokens for this audio. Available text was retained for review.",
    };
    qwen_finish_resolution(QwenResolution {
        segment,
        boundary,
        protocol: "output_guard",
        completeness: RecognitionCompleteness::Incomplete,
        warnings: vec![warning.into()],
        attempts,
    }, truncated)
}

fn qwen_finish_resolution(mut resolved: QwenResolution, truncated: bool) -> QwenResolution {
    // A retry cannot erase an explicit length limit reported for this audio.
    // Keep the text and require review even if the retry looks well-formed.
    if truncated {
        resolved.completeness = RecognitionCompleteness::Incomplete;
        resolved.protocol = "review_required";
        resolved.warnings.push(if resolved.boundary.text.trim().is_empty() {
            "Qwen3-ASR reached its output token limit. Review the recording for missing speech."
        } else {
            "Qwen3-ASR reached its output token limit. Available text was retained; review the recording for missing speech."
        }.into());
    }
    resolved
}

/// A leading header is a bare marker, or at most two words ending in a known
/// language name immediately before the marker. With two words the first must
/// be one of the decoder's known language-label words; arbitrary spoken text
/// before a language name is not treated as protocol.
fn qwen_leading_header(native: &str) -> Option<(&str, bool)> {
    let (prefix, speech) = native.trim_start().split_once(QWEN_MARKER)?;
    let words: Vec<&str> = prefix.split_whitespace().collect();
    match words.as_slice() {
        [] => Some((speech, true)),
        [language] if qwen_known_language(language) => Some((speech, false)),
        [label, language] if qwen_language_label(label) && qwen_known_language(language) => {
            Some((speech, false))
        }
        _ => None,
    }
}

/// The marker is protocol only when it is one whole native token near the
/// start; sub-word pieces like `<`, `asr`, `_`, `text`, `>` stay in spoken text.
fn qwen_marker_is_whole_token(tokens: &[String]) -> bool {
    let Some(index) = tokens.iter().position(|t| t.trim() == QWEN_MARKER) else {
        return false;
    };
    // Bound by words, not piece count: the observed `língua English` header
    // is five native pieces (` l`, `ng`, `ua`, ` English`, `<asr_text>`).
    tokens[..index].concat().split_whitespace().count() <= 2
}

fn qwen_language_label(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "language" | "língua" | "langue" | "lingua"
    )
}

fn checked_qwen_token_count(count: i32) -> Result<usize, String> {
    let count = usize::try_from(count)
        .map_err(|_| "Optional-model recognition returned a negative token count.".to_owned())?;
    if count > MAX_QWEN_NATIVE_TOKENS
        || count > isize::MAX as usize / std::mem::size_of::<*const c_char>()
    {
        return Err("Optional-model recognition returned an invalid token count.".into());
    }
    Ok(count)
}

fn qwen_known_language(word: &str) -> bool {
    QWEN_LANGUAGES
        .iter()
        .any(|known| word.eq_ignore_ascii_case(known))
}

/// Per-attempt evidence so a retry keeps the ambiguous first decode visible.
fn qwen_attempt_evidence(segment: &NativeSegment, boundary: &QwenBoundary) -> serde_json::Value {
    serde_json::json!({
        "native_text":segment.text,
        "native_tokens":segment.tokens,
        "native_token_count":if segment.finish_reason.is_some() {None} else {Some(segment.token_count)},
        "completion_tokens":segment.completion_tokens,
        "finish_reason":segment.finish_reason,
        "truncation_detected":segment.finish_reason.as_deref().map(|reason| reason == "length"),
        "parsed_text":boundary.text,
        "protocol":boundary.protocol,
        "token_provenance":boundary.token_provenance,
        "review_reason":boundary.review_reason,
        "qwen_output_guard":boundary.guard,
    })
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if !self.vad.is_null() {
                (self.api.destroy_vad)(self.vad);
                self.vad = ptr::null();
            }
            if let Decoder::Native(recognizer) = &mut self.decoder {
                if !recognizer.is_null() {
                    (self.api.destroy_offline)(*recognizer);
                    *recognizer = ptr::null();
                }
            }
        }
    }
}

/// Align delayed filters with the original source before either
/// detector or decoder receives them. The saved recording never enters here.
pub(crate) struct QwenInput {
    filter: crate::asr_filter::Filter,
    originals: std::collections::VecDeque<f32>,
    discard: usize,
}

impl QwenInput {
    pub(crate) fn mode_name(&self) -> &str { self.filter.mode_name() }
    pub(crate) fn gate_only(&self) -> bool { self.filter.gate_only() }

    pub(crate) fn new() -> Self {
        let filter = crate::asr_filter::Filter::from_env();
        let discard = filter.latency_samples();
        Self { filter, originals: std::collections::VecDeque::new(), discard }
    }

    pub(crate) fn process(&mut self, samples: &[f32], finish: bool) -> (Vec<f32>, Vec<f32>) {
        self.originals.extend(samples);
        let mut filtered = self.filter.process(samples);
        if finish { filtered.extend(self.filter.flush()); }
        self.align(filtered, self.filter.gate_only())
    }

    fn align(&mut self, filtered: Vec<f32>, gate_only: bool) -> (Vec<f32>, Vec<f32>) {
        let skip = self.discard.min(filtered.len());
        self.discard -= skip;
        let mut audio = Vec::with_capacity(filtered.len() - skip);
        let mut gate = Vec::with_capacity(filtered.len() - skip);
        for value in filtered.into_iter().skip(skip) {
            let Some(original) = self.originals.pop_front() else { break; };
            audio.push(if gate_only { original } else { value });
            gate.push(value);
        }
        (audio, gate)
    }
}

/// Update all existing ledger rows because later language evidence can revise
/// an earlier fragment. Keep empty rows so IDs and original timestamps survive.
pub(crate) fn refresh_qwen_guard(
    session: &crate::qwen_guard::Session,
    ledger: &mut BTreeMap<u64, crate::asr::Segment>,
    original: &mut Option<String>,
) -> Vec<serde_json::Value> {
    let decisions = session.decisions();
    let mut trace = Vec::with_capacity(decisions.len());
    for decision in decisions {
        if let Some(segment) = ledger.get_mut(&decision.id) { segment.text = decision.text.trim().to_owned(); }
        trace.push(serde_json::json!({"id":decision.id, "text":decision.text,
            "original_text":decision.original, "detected_language":decision.language,
            "dominant_language_in_other_windows":decision.dominant, "reason":decision.reason}));
    }
    *original = session.original_text();
    trace
}

pub struct Stream<'a> {
    engine: &'a mut Engine,
    pending: Vec<f32>,
    gate_pending: Vec<f32>,
    qwen_input: Option<QwenInput>,
    windows: Option<crate::qwen_window::Assembler>,
    merged_windows: bool,
    text_guard: crate::qwen_guard::Session,
    ledger: BTreeMap<u64, crate::asr::Segment>,
    original_text: Option<String>,
    next_id: u64,
    vad_samples_since_segment: usize,
    vad_samples: u64,
    vad_origin: u64,
    completeness: RecognitionCompleteness,
    /// Visible recognition warnings recorded at the adapter boundary, for
    /// example ambiguous output retained for review after one retry.
    warnings: Vec<String>,
    #[cfg(feature = "fidelity-eval")]
    segment_evidence: Vec<serde_json::Value>,
    #[cfg(feature = "fidelity-eval")]
    vad_evidence: VadEvidence,
}

#[cfg(feature = "fidelity-eval")]
#[derive(Default, serde::Serialize)]
struct VadEvidence {
    input_samples: u64,
    padding_samples: usize,
    sections_emitted: usize,
    sections_decoded: usize,
    /// Only actual emitted segments with no usable samples. Silence rejected
    /// internally by Silero has no section identity and is never counted here.
    sections_skipped: usize,
    decoded_audio_samples: u64,
}

impl Stream<'_> {
    pub fn add(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        if sample_rate == 0 || sample_rate > 384_000 {
            return Err("Invalid input sample rate.".into());
        }
        let samples = resample(samples, sample_rate, 16_000);
        if let Some(input) = &mut self.qwen_input {
            let (audio, gate) = input.process(&samples, false);
            self.pending.extend(audio);
            self.gate_pending.extend(gate);
        } else {
            self.pending.extend(samples);
        }
        self.drain_vad(false)
    }

    pub fn update(&mut self) -> Result<String, String> {
        self.drain_vad(false)?;
        Ok(crate::asr::assemble_segments(&self.ledger, false))
    }

    pub(crate) fn original_text(&self) -> Option<&str> {
        self.original_text.as_deref()
    }

    /// Recognition warnings recorded while decoding this stream's segments.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn finish(self) -> Result<String, String> {
        self.finish_recognition()?.into_complete_text()
    }

    pub fn finish_recognition(mut self) -> Result<crate::asr::RecognitionResult, String> {
        self.drain_vad(true)?;
        Ok(crate::asr::RecognitionResult {
            original_text: self.original_text,
            segments: crate::asr::ordered_segments(&self.ledger),
            text: crate::asr::assemble_segments(&self.ledger, false),
            completeness: self.completeness,
            warnings: self.warnings,
        })
    }

    #[cfg(feature = "fidelity-eval")]
    pub fn finish_observed(
        mut self,
    ) -> Result<(crate::asr::RecognitionResult, serde_json::Value), String> {
        self.drain_vad(true)?;
        let recognition = crate::asr::RecognitionResult {
            original_text: self.original_text,
            segments: crate::asr::ordered_segments(&self.ledger),
            text: crate::asr::assemble_segments(&self.ledger, false),
            completeness: self.completeness,
            warnings: self.warnings,
        };
        let details = serde_json::json!({"segments":self.segment_evidence,
            "warnings":&recognition.warnings,
            "recognition_completeness":recognition.completeness,
            "all_segments_complete":recognition.completeness == RecognitionCompleteness::Complete,
            "timestamp_basis":"VAD sample offsets, including resets",
            "speech_gate":"silero_vad",
            "qwen_output_guards_enabled":if is_qwen(&self.engine.model_id) {Some(qwen_guards_enabled())} else {None},
            "qwen_window_seconds":if self.windows.is_some() {Some(crate::qwen_window::target_seconds_for_model(&self.engine.model_id))} else {None},
            "vad_threshold":if self.windows.is_some() {crate::qwen_window::vad_threshold()} else {0.25},
            "asr_filter":self.qwen_input.as_ref().map(QwenInput::mode_name),
            "untriggered_tail_requires_original_confirmation":self.qwen_input.as_ref().is_some_and(QwenInput::gate_only),
            "vad":self.vad_evidence,
            "sections_decoded":self.vad_evidence.sections_decoded,
            "sections_skipped":self.vad_evidence.sections_skipped,
            "vad_discarded_section_count":serde_json::Value::Null,
            "section_count_basis":"sections_emitted counts VAD segments; decoded/skipped counts decoder windows (or baseline segments). VAD-discarded audio has no section count.",
            "truncation_status":if matches!(&self.engine.decoder, Decoder::Llama(_)) {
                "server finish reason recorded per attempt"
            } else {"native finish reason unavailable"}});
        Ok((recognition, details))
    }

    fn drain_vad(&mut self, flush: bool) -> Result<(), String> {
        let window = 512usize;
        if flush {
            if let Some(input) = &mut self.qwen_input {
                let (audio, gate) = input.process(&[], true);
                self.pending.extend(audio);
                self.gate_pending.extend(gate);
            }
        }
        while self.pending.len() >= window {
            let chunk: Vec<f32> = self.pending.drain(..window).collect();
            let gate: Option<Vec<f32>> = self.qwen_input.is_some()
                .then(|| self.gate_pending.drain(..window).collect());
            let gate = gate.as_deref().unwrap_or(&chunk);
            if let Some(windows) = &mut self.windows { windows.append(&chunk, gate)?; }
            unsafe {
                (self.engine.api.vad_accept)(self.engine.vad, gate.as_ptr(), gate.len() as i32);
            }
            self.vad_samples_since_segment += chunk.len();
            self.vad_samples += chunk.len() as u64;
            #[cfg(feature = "fidelity-eval")]
            { self.vad_evidence.input_samples += chunk.len() as u64; }
            self.collect_segments()?;
            // sherpa's max_speech_duration only raises the VAD threshold; it
            // is not a hard segment cap. Bound continuous speech explicitly
            // so Qwen never receives an ever-growing audio context.
            if self.windows.as_ref().is_some_and(|w| w.hard_due())
                || self.vad_samples_since_segment >= if self.windows.is_some() && self.merged_windows { 20 * 16_000 }
                    else { crate::asr::sherpa_section_seconds() * 16_000 } {
                unsafe { (self.engine.api.vad_flush)(self.engine.vad) };
                self.collect_segments()?;
                unsafe { (self.engine.api.vad_reset)(self.engine.vad) };
                self.vad_origin = self.vad_samples;
                self.vad_samples_since_segment = 0;
            }
            self.decode_windows(false)?;
        }
        if flush {
            // sherpa's Flush ignores an incomplete VAD window. Pad only that
            // final window (at most 31ms) so the last captured samples survive.
            if !self.pending.is_empty() {
                let real_count = self.pending.len();
                #[cfg(feature = "fidelity-eval")]
                {
                    self.vad_evidence.input_samples += self.pending.len() as u64;
                    self.vad_evidence.padding_samples += window - self.pending.len();
                }
                if let Some(windows) = &mut self.windows {
                    windows.append(&self.pending, &self.gate_pending)?;
                }
                let mut final_frame = if self.qwen_input.is_some() {
                    std::mem::take(&mut self.gate_pending)
                } else { std::mem::take(&mut self.pending) };
                final_frame.resize(window, 0.0);
                unsafe {
                    (self.engine.api.vad_accept)(
                        self.engine.vad,
                        final_frame.as_ptr(),
                        final_frame.len() as i32,
                    );
                }
                self.pending.clear();
                self.vad_samples += real_count as u64;
            }
            unsafe { (self.engine.api.vad_flush)(self.engine.vad) };
            self.collect_segments()?;
            #[cfg(any(windows, target_os = "macos", test))]
            let original_confirmation = self.qwen_input.as_ref().is_some_and(QwenInput::gate_only);
            #[cfg(any(windows, target_os = "macos", test))]
            if let Some(windows) = self.windows.as_mut().filter(|_| self.merged_windows) {
                let (origin, tail) = windows.tail();
                let original = original_confirmation.then(|| windows.exact(origin, windows.end())
                    .map(|window| window.audio)).transpose()?;
                if let Some(range) = crate::mac_speech::speech_gate::confirmed_tail_with_original(&tail, original.as_deref(), None)? {
                    windows.segment(origin + range.start as u64..origin + range.end as u64, true)?;
                }
            }
            self.decode_windows(true)?;
        }
        Ok(())
    }

    fn collect_segments(&mut self) -> Result<(), String> {
        while unsafe { (self.engine.api.vad_empty)(self.engine.vad) } == 0 {
            let segment = unsafe { (self.engine.api.vad_front)(self.engine.vad) };
            if segment.is_null() {
                break;
            }
            let (samples, count, start) = unsafe { ((*segment).samples, (*segment).n, (*segment).start) };
            let start_seconds = (self.vad_origin + start.max(0) as u64) as f32 / 16_000.0;
            let end_seconds = start_seconds + count.max(0) as f32 / 16_000.0;
            let audio = if samples.is_null() || count <= 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(samples, count as usize) }.to_vec()
            };
            unsafe {
                (self.engine.api.destroy_segment)(segment);
                (self.engine.api.vad_pop)(self.engine.vad);
            }
            #[cfg(feature = "fidelity-eval")]
            { self.vad_evidence.sections_emitted += 1; }
            // Flush/reset only drains Silero's speech queue; neither submits
            // the pending buffer directly to ASR. An invalid empty emission
            // must not invoke the decoder or fabricate a speech section.
            if audio.is_empty() {
                #[cfg(feature = "fidelity-eval")]
                {
                    self.vad_evidence.sections_skipped += 1;
                    self.segment_evidence.push(serde_json::json!({
                        "audio_samples":0,
                        "decoded":false,
                        "skipped":true,
                        "skip_reason":"empty_vad_segment",
                        "start_seconds":start_seconds,
                        "end_seconds":end_seconds,
                    }));
                }
                continue;
            }
            if let Some(windows) = &mut self.windows {
                let from = self.vad_origin + start.max(0) as u64;
                let to = from + count.max(0) as u64;
                if self.merged_windows {
                    windows.segment(from..to, false)?;
                } else {
                    let window = windows.exact(from, to)?;
                    self.decode_qwen_window(window)?;
                }
                self.vad_samples_since_segment = 0;
                continue;
            }
            #[cfg(feature = "fidelity-eval")]
            {
                let mut evidence = serde_json::Value::Null;
                let recognition = self
                    .engine
                    .transcribe_observed(&audio, Some(&mut evidence))?;
                evidence["text"] = serde_json::json!(&recognition.text);
                if evidence["skipped"] == true {
                    self.vad_evidence.sections_skipped += 1;
                } else {
                    self.vad_evidence.sections_decoded += 1;
                    self.vad_evidence.decoded_audio_samples += audio.len() as u64;
                    evidence["decoded"] = serde_json::json!(true);
                    evidence["skipped"] = serde_json::json!(false);
                }
                evidence["speech_gate"] = serde_json::json!("silero_vad");
                evidence["start_seconds"] = serde_json::json!(start_seconds);
                evidence["end_seconds"] = serde_json::json!(end_seconds);
                evidence["complete"] = serde_json::json!(
                    recognition.completeness == RecognitionCompleteness::Complete
                );
                self.segment_evidence.push(evidence);
                if recognition.completeness == RecognitionCompleteness::Incomplete {
                    self.completeness = RecognitionCompleteness::Incomplete;
                }
                crate::asr::retain_original_section(&mut self.original_text,
                    || crate::asr::assemble_segments(&self.ledger, false), &recognition);
                crate::asr::append_recognition_warnings(&mut self.warnings, recognition.warnings);
                self.push_text(&recognition.text, start_seconds, end_seconds,
                    recognition.completeness == RecognitionCompleteness::Complete)?;
            }
            #[cfg(not(feature = "fidelity-eval"))]
            {
                let recognition = self.engine.transcribe(&audio)?;
                if recognition.completeness == RecognitionCompleteness::Incomplete {
                    self.completeness = RecognitionCompleteness::Incomplete;
                }
                crate::asr::retain_original_section(&mut self.original_text,
                    || crate::asr::assemble_segments(&self.ledger, false), &recognition);
                crate::asr::append_recognition_warnings(&mut self.warnings, recognition.warnings);
                self.push_text(&recognition.text, start_seconds, end_seconds,
                    recognition.completeness == RecognitionCompleteness::Complete)?;
            }
            self.vad_samples_since_segment = 0;
        }
        Ok(())
    }

    fn decode_windows(&mut self, finish: bool) -> Result<(), String> {
        if !self.merged_windows || self.windows.is_none() { return Ok(()); }
        let speaking = unsafe { (self.engine.api.vad_detected)(self.engine.vad) } != 0;
        let ready = match &mut self.windows {
            Some(windows) => windows.poll_active(finish, speaking)?,
            None => return Ok(()),
        };
        for window in ready { self.decode_qwen_window(window)?; }
        Ok(())
    }

    fn decode_qwen_window(&mut self, window: crate::qwen_window::Window) -> Result<(), String> {
        let mut evidence = serde_json::Value::Null;
        #[cfg(any(windows, target_os = "macos", test))]
        let qualified = confirm_speech(&window.gate_audio, Some(&mut evidence))?;
        #[cfg(not(any(windows, target_os = "macos", test)))]
        let qualified = true;
        if qualified {
            let recognition = self.engine.transcribe_qualified(&window.audio, Some(&mut evidence))?;
            let id = self.next_id;
            self.next_id += 1;
            let complete = recognition.completeness == RecognitionCompleteness::Complete;
            if !complete { self.completeness = RecognitionCompleteness::Incomplete; }
            crate::asr::append_recognition_warnings(&mut self.warnings, recognition.warnings);
            self.ledger.insert(id, crate::asr::Segment { id,
                start_seconds: window.start as f32 / 16_000.0,
                end_seconds: Some(window.end as f32 / 16_000.0), speaker: None,
                text: recognition.text.trim().to_owned(), complete });
            if qwen_guards_enabled() {
                self.text_guard.push(id, &recognition.text, recognition.original_text.as_deref(),
                    evidence["detected_language"].as_str());
                let trace = refresh_qwen_guard(&self.text_guard, &mut self.ledger, &mut self.original_text);
                evidence["qwen_fragment_guard"] = trace.last().cloned().unwrap_or_default();
                #[cfg(feature = "fidelity-eval")]
                for previous in &mut self.segment_evidence {
                    if let Some(decision) = trace.iter().find(|d| d["id"] == previous["window_id"]) {
                        previous["qwen_fragment_guard"] = decision.clone();
                        previous["text"] = decision["text"].clone();
                    }
                }
            }
            evidence["window_id"] = serde_json::json!(id);
            evidence["text"] = serde_json::json!(self.ledger[&id].text);
            evidence["complete"] = serde_json::json!(complete);
        }
        #[cfg(feature = "fidelity-eval")]
        {
            if qualified {
                self.vad_evidence.sections_decoded += 1;
                self.vad_evidence.decoded_audio_samples += window.audio.len() as u64;
            } else { self.vad_evidence.sections_skipped += 1; }
            evidence["decoded"] = serde_json::json!(qualified);
            evidence["skipped"] = serde_json::json!(!qualified);
            evidence["start_seconds"] = serde_json::json!(window.start as f64 / 16_000.0);
            evidence["end_seconds"] = serde_json::json!(window.end as f64 / 16_000.0);
            evidence["source_sample_ranges"] = serde_json::json!(window.ranges.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>());
            evidence["boundary"] = serde_json::json!(window.reason);
            evidence["recovered_tail"] = serde_json::json!(window.recovered_tail);
            self.segment_evidence.push(evidence);
        }
        Ok(())
    }

    fn push_text(&mut self, text: &str, start_seconds: f32, end_seconds: f32, complete: bool) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.ledger.insert(
            id,
            crate::asr::Segment {
                id,
                start_seconds,
                end_seconds: Some(end_seconds),
                speaker: None,
                text: text.to_owned(),
                complete,
            },
        );
        Ok(())
    }
}

/// Qwen emits a language header before each independent audio segment.
/// Text-level handling removes only a leading header shape confirmed by
/// `qwen_leading_header`; it never searches through spoken text. A LEADING
/// bare marker at the native boundary is protocol, which is why
/// `qwen_boundary_parse` owns the decision and this function only extracts the
/// speech after a confirmed header.
fn qwen_transcript(raw: &str) -> &str {
    match qwen_leading_header(raw) {
        Some((speech, _)) => speech.trim_start(),
        None => raw,
    }
}

// Names from Qwen3-ASR's output protocol, plus its empty-audio sentinel.
const QWEN_LANGUAGES: &[&str] = &[
    "Chinese",
    "English",
    "Cantonese",
    "Arabic",
    "German",
    "French",
    "Spanish",
    "Portuguese",
    "Indonesian",
    "Italian",
    "Korean",
    "Russian",
    "Thai",
    "Vietnamese",
    "Japanese",
    "Turkish",
    "Hindi",
    "Malay",
    "Dutch",
    "Swedish",
    "Danish",
    "Finnish",
    "Polish",
    "Czech",
    "Filipino",
    "Persian",
    "Greek",
    "Romanian",
    "Hungarian",
    "Macedonian",
    "None",
];

pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || from == to {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = ((samples.len() as f64) / ratio).floor() as usize;
    (0..n)
        .map(|i| {
            let src = i as f64 * ratio;
            let i0 = src.floor() as usize;
            let frac = (src - i0 as f64) as f32;
            let a = samples.get(i0).copied().unwrap_or(0.0);
            let b = samples.get(i0 + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_gate_alignment_retains_original_audio_and_flushes_the_real_tail() {
        let mut input = QwenInput::new();
        input.discard = 352;
        let original: Vec<_> = (0..1_000).map(|i| i as f32 / 1_000.0).collect();
        input.originals.extend(&original);
        let first = input.align(vec![0.1; 1_000], true);
        assert_eq!(first.0, original[..648]);
        assert_eq!(first.1, vec![0.1; 648]);
        let last = input.align(vec![0.2; 352], true);
        assert_eq!(last.0, original[648..]);
        assert_eq!(last.1, vec![0.2; 352]);
        assert!(input.originals.is_empty());
        assert_eq!(input.discard, 0);
        let fresh = QwenInput::new();
        assert!(fresh.originals.is_empty());
    }

    #[test]
    fn late_language_decision_updates_ledger_but_never_replaces_original_recognition() {
        let mut session = crate::qwen_guard::Session::default();
        let mut ledger = BTreeMap::new();
        let mut original = None;
        for (id, text, language) in [(1, "Obrigado.", "Portuguese"),
            (2, "Please preserve all the words I actually said in this recording.", "English")] {
            ledger.insert(id, crate::asr::Segment { id, start_seconds: id as f32,
                end_seconds: Some(id as f32 + 0.5), speaker: None, text: text.into(), complete: true });
            session.push(id, text, None, Some(language));
            refresh_qwen_guard(&session, &mut ledger, &mut original);
        }
        assert_eq!(ledger[&1].text, "");
        assert_eq!(ledger[&1].start_seconds, 1.0);
        assert_eq!(original.as_deref(), Some("Obrigado. Please preserve all the words I actually said in this recording."));
        let fresh = crate::qwen_guard::Session::default();
        let mut next_ledger = BTreeMap::new();
        let mut next_original = None;
        assert!(refresh_qwen_guard(&fresh, &mut next_ledger, &mut next_original).is_empty());
        assert_eq!(next_original, None);
    }

    #[test]
    fn gpu_ids_never_fall_back_to_cpu_adapter() {
        for gpu in ["qwen3-asr-1.7b-fp16-cuda", "whisper-tiny-fp32-cuda"] {
            let err = match Engine::load(std::path::Path::new("missing-test-models"), gpu) {
                Ok(_) => panic!("{gpu} must not load on the CPU adapter"),
                Err(err) => err,
            };
            assert!(err.contains("CUDA adapter"), "{gpu}: {err}");
            assert!(!err.contains("Unsupported recognizer"), "{gpu}");
        }
    }

    #[test]
    fn qwen_headers_are_removed_at_each_segment_boundary() {
        let chunks = [
            "First sentence.",
            "Language English<asr_text>I feel like we should try.",
            "language Chinese\n<asr_text>你好。",
        ];
        assert_eq!(
            chunks.map(qwen_transcript).join(" "),
            "First sentence. I feel like we should try. 你好。"
        );
        assert_eq!(
            qwen_transcript("  LANGUAGE English<asr_text>  Keep this."),
            "Keep this."
        );
        assert_eq!(qwen_transcript("language None<asr_text>"), "");
    }

    #[test]
    fn qwen_header_parser_preserves_spoken_text_and_literals() {
        for text in [
            "Language English is selected.",
            "Say Language English<asr_text> literally.",
            "`Language English<asr_text>` is a token.",
            "Language English is selected<asr_text>keep all this",
            "Language unknown<asr_text>keep all this",
            // A LEADING bare marker at the native boundary is protocol and is
            // stripped by qwen_boundary_parse; text-stage code only ever strips
            // a leading header, so a non-leading mention stays spoken text.
            "The <asr_text> is the marker.",
            "  Just ordinary speech.  ",
        ] {
            assert_eq!(qwen_transcript(text), text);
        }
    }

    #[test]
    fn qwen_boundary_strips_leading_protocol_headers() {
        let boundary = qwen_boundary_parse(
            "<asr_text>It was with great opposition, after two or three hours.",
            None,
            10.7,
        );
        assert_eq!(boundary.protocol, "bare_marker_stripped");
        assert_eq!(boundary.token_provenance, "text_only");
        assert_eq!(
            boundary.text,
            "It was with great opposition, after two or three hours."
        );
        assert!(boundary.review_reason.is_none());

        let boundary = qwen_boundary_parse("língua English<asr_text>Well, sir,", None, 3.5);
        assert_eq!(boundary.protocol, "header_stripped");
        assert_eq!(boundary.text, "Well, sir,");

        let boundary = qwen_boundary_parse(
            "Language English<asr_text>I feel like we should try.",
            None,
            5.0,
        );
        assert_eq!(boundary.protocol, "header_stripped");
        assert_eq!(boundary.text, "I feel like we should try.");

        // The empty-audio sentinel on genuinely short audio is empty speech,
        // not malformed output.
        let boundary = qwen_boundary_parse("LANGUAGE None<asr_text>", None, 0.4);
        assert_eq!(boundary.protocol, "header_stripped");
        assert_eq!(boundary.text, "");
        assert!(boundary.review_reason.is_none());
    }

    #[test]
    fn qwen_boundary_preserves_nonleading_markers_and_longer_prefixes() {
        for raw in [
            // Three words before the marker: not a protocol header.
            "Say Language English<asr_text> literally.",
            "The <asr_text> is the marker.",
            "Language English is selected<asr_text>keep all this",
            "Language unknown<asr_text>keep all this",
            "`Language English<asr_text>` is a token.",
            "Say English<asr_text> literally.",
        ] {
            let boundary = qwen_boundary_parse(raw, None, 5.0);
            assert_eq!(boundary.protocol, "none", "{raw}");
            assert_eq!(boundary.text, raw, "{raw}");
            assert!(boundary.review_reason.is_none(), "{raw}");
        }
    }

    #[test]
    fn qwen_boundary_preserves_ambiguous_angle_literals_for_review() {
        for (raw, pieces) in [("< 3", ["<", " 3"]), ("<T", ["<", "T"]), ("<<", ["<", "<"])] {
            let tokens = pieces.map(str::to_owned);
            for duration in [0.4, 4.25] {
                let boundary = qwen_boundary_parse(raw, Some(&tokens), duration);
                assert_eq!(boundary.text, raw);
                assert_eq!(boundary.protocol, "none");
                assert_eq!(boundary.token_provenance, "native_token_pieces");
                assert_eq!(
                    boundary.review_reason,
                    Some("unterminated angle-prefixed output is indistinguishable from literal speech")
                );
            }
        }
        for raw in ["<T>", "value < 3"] {
            let boundary = qwen_boundary_parse(raw, None, 4.25);
            assert_eq!(boundary.text, raw);
            assert!(boundary.review_reason.is_none());
        }
    }

    #[test]
    fn qwen_boundary_keeps_known_ambiguous_failure_explicit() {
        // `<thö`: two native tokens over 4.25 s of audio, no closing marker.
        let boundary = qwen_boundary_parse("<thö", None, 4.25);
        assert_eq!(
            boundary.review_reason,
            Some("unterminated angle-prefixed output is indistinguishable from literal speech")
        );

        let tokens = vec!["<th".to_string(), "ö".to_string()];
        let boundary = qwen_boundary_parse("<thö", Some(&tokens), 4.25);
        assert_eq!(boundary.text, "<thö");
        assert!(boundary.review_reason.is_some());

        // A short marker-like utterance is retained just like longer output;
        // duration does not establish whether the text was literal speech.
        let boundary = qwen_boundary_parse("<thö", None, 0.4);
        assert_eq!(boundary.text, "<thö");
        assert!(boundary.review_reason.is_some());
        // Native token pieces are diagnostic and do not prove token kind.
        let tokens = vec!["<th".to_string(), "ö".to_string()];
        let boundary = qwen_boundary_parse("<thö", Some(&tokens), 0.4);
        assert_eq!(boundary.token_provenance, "native_token_pieces");
        assert!(boundary.review_reason.is_some());

        // A different token split cannot clear text-level ambiguity.
        let tokens = vec!["thö".to_string(), "<".to_string()];
        let boundary = qwen_boundary_parse("<thö", Some(&tokens), 4.25);
        assert!(boundary.review_reason.is_some());

        // A genuinely short ordinary utterance passes through unchanged.
        let boundary = qwen_boundary_parse("Yes.", None, 0.6);
        assert_eq!(boundary.text, "Yes.");
        assert!(boundary.review_reason.is_none());

        // A header with no speech on substantial audio needs review; a short
        // header-only result remains the recognizer's empty result.
        let boundary = qwen_boundary_parse("Language English<asr_text>", None, 4.0);
        assert_eq!(
            boundary.review_reason,
            Some("protocol header contained no speech on substantial audio")
        );
        let boundary = qwen_boundary_parse("Language English<asr_text>", None, 0.8);
        assert!(boundary.review_reason.is_none());
        let boundary = qwen_boundary_parse("<asr_text>", None, 4.0);
        assert!(boundary.review_reason.is_some());
        // The documented empty-audio sentinel is a genuine no-speech result
        // at any duration; it is never retried or reported as incomplete.
        let boundary = qwen_boundary_parse("language None<asr_text>", None, 4.0);
        assert_eq!(boundary.text, "");
        assert!(boundary.review_reason.is_none());
    }

    #[test]
    fn qwen_retry_route_distinguishes_recovery_from_review_required() {
        let recovered = qwen_resolve_segment(
            NativeSegment {
                text: "<thö".into(),
                tokens: Some(vec!["<th".into(), "ö".into()]),
                token_count: 2,
                ..Default::default()
            },
            4.25,
            || {
                Ok(NativeSegment {
                    text: "Language English<asr_text>Recovered complete speech.".into(),
                    tokens: None,
                    token_count: 6,
                    ..Default::default()
                })
            },
        );
        assert_eq!(recovered.boundary.text, "Recovered complete speech.");
        assert_eq!(recovered.protocol, "review_retry_ok");
        assert_eq!(recovered.completeness, RecognitionCompleteness::Complete);
        assert!(recovered.warnings.is_empty());
        assert_eq!(recovered.attempts.len(), 2);

        let retained = qwen_resolve_segment(
            NativeSegment {
                text: "<thö".into(),
                tokens: Some(vec!["<th".into(), "ö".into()]),
                token_count: 2,
                ..Default::default()
            },
            4.25,
            || {
                Ok(NativeSegment {
                    text: "<thö".into(),
                    tokens: Some(vec!["<th".into(), "ö".into()]),
                    token_count: 2,
                    ..Default::default()
                })
            },
        );
        assert_eq!(retained.boundary.text, "<thö");
        assert_eq!(retained.protocol, "review_required");
        assert_eq!(retained.completeness, RecognitionCompleteness::Incomplete);
        assert_eq!(retained.warnings.len(), 1);
        assert_eq!(retained.attempts.len(), 2);

        let empty = qwen_resolve_segment(
            NativeSegment {
                text: "Language English<asr_text>".into(),
                tokens: None,
                token_count: 3,
                ..Default::default()
            },
            4.25,
            || {
                Ok(NativeSegment {
                    text: "Language English<asr_text>".into(),
                    tokens: None,
                    token_count: 3,
                    ..Default::default()
                })
            },
        );
        assert!(empty.boundary.text.is_empty());
        assert_eq!(empty.completeness, RecognitionCompleteness::Incomplete);
        assert_eq!(empty.attempts.len(), 2);

        let sentinel = qwen_resolve_segment(
            NativeSegment {
                text: "Language None<asr_text>".into(),
                tokens: None,
                token_count: 3,
                ..Default::default()
            },
            4.25,
            || panic!("empty-audio sentinel must not retry"),
        );
        assert!(sentinel.boundary.text.is_empty());
        assert_eq!(sentinel.completeness, RecognitionCompleteness::Complete);
        assert!(sentinel.attempts.is_empty());

        let retry_error = qwen_resolve_segment(
            NativeSegment {
                text: "< 3".into(),
                tokens: Some(vec!["<".into(), " 3".into()]),
                token_count: 2,
                ..Default::default()
            },
            0.4,
            || Err("injected retry failure".into()),
        );
        assert_eq!(retry_error.boundary.text, "< 3");
        assert_eq!(
            retry_error.completeness,
            RecognitionCompleteness::Incomplete
        );
        assert_eq!(retry_error.attempts.len(), 2);
        assert_eq!(
            retry_error.attempts[1]["retry_error"],
            "injected retry failure"
        );
    }

    #[test]
    fn qwen_boundary_uses_native_token_provenance() {
        // The exact native pieces recorded for libri-7902-96594-0008: the
        // label word is split into three pieces, so the whole marker is the
        // fifth piece and must still count as a two-word header.
        let observed: Vec<String> = [
            " lí",
            "ng",
            "ua",
            " English",
            "<asr_text>",
            "Well",
            ",",
            " sir",
            ",\"",
            " said",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let boundary = qwen_boundary_parse(
            "língua English<asr_text>Well, sir,\" said",
            Some(&observed),
            3.5,
        );
        assert_eq!(boundary.protocol, "header_stripped");
        assert_eq!(boundary.text, "Well, sir,\" said");
        // The marker as sub-word pieces is spoken text, not protocol.
        let pieces: Vec<String> = ["língua", " English", "<", "asr", "_", "text", ">", " Well"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let boundary =
            qwen_boundary_parse("língua English<asr_text>Well, sir,", Some(&pieces), 3.5);
        assert_eq!(boundary.protocol, "none");
        assert_eq!(boundary.text, "língua English<asr_text>Well, sir,");
        assert_eq!(boundary.token_provenance, "native_token_pieces");

        // The marker as one whole special token is protocol.
        let pieces: Vec<String> = ["língua", " English", "<asr_text>", " Well", ", sir", ","]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let boundary =
            qwen_boundary_parse("língua English<asr_text>Well, sir,", Some(&pieces), 3.5);
        assert_eq!(boundary.protocol, "header_stripped");
        assert_eq!(boundary.text, "Well, sir,");
        assert_eq!(boundary.token_provenance, "whole_marker_token");
    }

    #[test]
    fn first_decode_error_gets_one_retry_and_cannot_become_blank_recovery() {
        let calls = std::cell::Cell::new(0);
        let failed = qwen_resolve_decode(Err("first decode failed".into()), 4.0, || {
            calls.set(calls.get() + 1);
            Err("retry failed".into())
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(failed.completeness, RecognitionCompleteness::Incomplete);
        assert!(failed.boundary.text.is_empty());
        assert_eq!(failed.attempts.len(), 2);
        let blank = qwen_resolve_decode(Err("first decode failed".into()), 4.0, || {
            Ok(NativeSegment::default())
        });
        assert_eq!(blank.completeness, RecognitionCompleteness::Incomplete);
        let recovered = qwen_resolve_decode(Err("first decode failed".into()), 4.0, || {
            Ok(NativeSegment {
                text: "Keep the full tail.".into(),
                ..Default::default()
            })
        });
        assert_eq!(recovered.completeness, RecognitionCompleteness::Complete);
        assert_eq!(recovered.protocol, "decode_retry_ok");
        assert_eq!(recovered.boundary.text, "Keep the full tail.");
        assert!(recovered.warnings.is_empty());
    }

    #[test]
    fn qwen_native_token_count_is_bounded_before_slice_creation() {
        assert_eq!(checked_qwen_token_count(0).unwrap(), 0);
        assert_eq!(checked_qwen_token_count(256).unwrap(), 256);
        assert!(checked_qwen_token_count(-1).is_err());
        assert!(checked_qwen_token_count(4_097).is_err());
    }

    fn guarded_response(text: &str, tokens: i64, reason: &str, seconds: f64) -> QwenResolution {
        qwen_resolve_decode_with_guards(
            Ok(NativeSegment::from_llama_response((text.into(), Some(tokens), reason.into()))),
            seconds,
            true,
            || panic!("ordinary or guarded output must not retry"),
        )
    }

    #[test]
    fn qwen_original_speech_survives_guards_and_retry_resolution() {
        for (raw, guarded, changed) in [
            ("go go go go", "", true),
            ("Keep these four words. go go go go", "Keep these four words. go", true),
            ("Keep these four words.", "Keep these four words.", false),
        ] {
            for retry in [false, true] {
                let mut calls = 0;
                let result = transcribe_decoded(&vec![0.1; 64_000], true, || {
                    calls += 1;
                    if retry && calls == 1 { return Err("decode failed".into()); }
                    Ok(NativeSegment::from_llama_response((
                        format!("language English<asr_text>{raw}"), Some(12), "stop".into(),
                    )))
                }, None, None).unwrap();
                assert_eq!(calls, if retry { 2 } else { 1 });
                assert_eq!(result.text, guarded);
                assert_eq!(result.original_text(), raw);
                assert_eq!(result.original_text.is_some(), changed);
                assert_eq!(result.completeness == RecognitionCompleteness::Incomplete, changed);
            }
        }
        // A length-limited first attempt changes a collapsed retry into a drop.
        let raw = "Keep these four words. go go go go";
        let mut calls = 0;
        let result = transcribe_decoded(&vec![0.1; 64_000], true, || {
            calls += 1;
            Ok(NativeSegment::from_llama_response(if calls == 1 {
                ("< 3".into(), Some(2), "length".into())
            } else { (raw.into(), Some(12), "stop".into()) }))
        }, None, None).unwrap();
        assert!(result.text.is_empty());
        assert_eq!(result.original_text(), raw);
        assert_eq!(calls, 2);
    }

    #[test]
    fn qwen_duration_token_cap_ceil_and_saturation() {
        for (samples, expected) in [
            (0, 32), (1, 33), (1_999, 33), (2_000, 33), (2_001, 34),
            (16_000, 40), (16_001, 41), (64_000, 64), (320_000, 192),
            (448_000, 256), (959_999, 512), (960_000, 512),
            (960_001, 512), (usize::MAX, 512),
        ] {
            assert_eq!(qwen_max_tokens(samples), expected, "{samples} samples");
        }
    }

    #[test]
    fn qwen_portuguese_loop_is_omitted_without_retry_and_keeps_raw_evidence() {
        let raw = "language Portuguese<asr_text>Obrigado por assistir. OBRIGADO por assistir! Obrigado por assistir? Obrigado por assistir.";
        let mut evidence = serde_json::Value::Null;
        let mut calls = 0;
        let recognition = transcribe_decoded(&vec![0.1; 64_000], true, || {
            calls += 1;
            Ok(NativeSegment::from_llama_response((raw.into(), Some(40), "stop".into())))
        }, None, Some(&mut evidence)).unwrap();
        assert_eq!(calls, 1);
        assert!(recognition.text.is_empty());
        assert_eq!(recognition.completeness, RecognitionCompleteness::Incomplete);
        assert!(recognition.warnings[0].contains("omitted"));
        assert_eq!(evidence["native_text"], raw);
        assert_eq!(evidence["attempts"][0]["native_text"], raw);
        assert_eq!(evidence["qwen_output_guard"]["action"], "drop");
        assert_eq!(evidence["qwen_output_guard"]["loops"][0]["ngram_words"], 3);
        assert_eq!(evidence["qwen_output_guard"]["loops"][0]["repetitions"], 4);
        assert_eq!(evidence["configured_max_new_tokens"], 64);
    }

    #[test]
    fn qwen_loops_cover_one_through_six_words_but_not_three_repetitions() {
        let vocabulary = ["um", "dois", "três", "quatro", "cinco", "seis"];
        for size in 1..=6 {
            let phrase = vocabulary[..size].join(" ");
            let looped = format!("{phrase}. {phrase}! {phrase}? {phrase}.");
            let result = guarded_response(&looped, 30, "stop", 4.0);
            assert!(result.boundary.text.is_empty(), "{looped}");
            assert_eq!(result.boundary.guard.loops[0].ngram_words, size);
            assert_eq!(result.boundary.guard.loops[0].repetitions, 4);
            for repetitions in 1..4 {
                let text = vec![phrase.as_str(); repetitions].join(". ");
                let result = guarded_response(&text, 30, "stop", 4.0);
                assert_eq!(result.boundary.text, text);
                assert!(result.warnings.is_empty());
                assert_eq!(result.completeness, RecognitionCompleteness::Complete);
            }
        }
        let text = ["one two three four five six seven"; 4].join(". ");
        assert_eq!(guarded_response(&text, 40, "stop", 4.0).boundary.text, text);
    }

    #[test]
    fn qwen_partial_loop_preserves_first_spelling_punctuation_and_surrounding_words() {
        let text = "Please keep these opening words. É isso! é ISSO? É isso, é isso. Then keep the final sentence.";
        let result = guarded_response(text, 40, "stop", 5.0);
        assert_eq!(result.boundary.text,
            "Please keep these opening words. É isso! Then keep the final sentence.");
        assert_eq!(result.boundary.guard.action, "collapse");
        assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
        assert!(result.warnings[0].contains("first occurrence"));
        assert_eq!(result.segment.text, text);

        let result = guarded_response("Keep all four words. yes,YES!yes?Yes.", 20, "stop", 4.0);
        assert_eq!(result.boundary.text, "Keep all four words. yes,");
        let result = guarded_response(
            "Keep all four words. yes yes yes yes. Save these four too. no no no no.",
            25, "stop", 4.0,
        );
        assert_eq!(result.boundary.text, "Keep all four words. yes Save these four too. no");
        assert_eq!(result.boundary.guard.loops.len(), 2);
    }

    #[test]
    fn qwen_mostly_loop_thresholds_are_fixed_at_three_words_or_eighty_percent() {
        for (prefix, copies, drop) in [
            ("Keep these words", 4, true),
            ("Keep these four words", 4, false),
            ("Keep these four words", 15, false),
            ("Keep these four words", 16, true),
        ] {
            let raw = format!("{prefix}. {}", vec!["echo"; copies].join(" "));
            let result = guarded_response(&raw, 30, "stop", 4.0);
            assert_eq!(result.boundary.text.is_empty(), drop, "{raw}");
        }
    }

    #[test]
    fn qwen_token_limit_plus_even_partial_loop_drops_the_section() {
        let raw = "Keep these four words. go go go go. Keep these final words too.";
        assert!(!guarded_response(raw, 35, "stop", 4.0).boundary.text.is_empty());
        let result = guarded_response(raw, 35, "length", 4.0);
        assert!(result.boundary.text.is_empty());
        assert_eq!(result.boundary.guard.reason, Some("repetition_at_token_limit"));
        assert!(result.warnings.iter().any(|warning| warning.contains("token limit")));
        assert!(result.warnings.iter().all(|warning| !warning.contains("text was retained")));
        // Truncation alone keeps the useful words for review.
        let result = guarded_response("Keep these four words", 35, "length", 4.0);
        assert_eq!(result.boundary.text, "Keep these four words");
        assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
    }

    #[test]
    fn qwen_rate_guard_records_exact_allowance_and_never_erases_rate_only_text() {
        let text = "Keep all of these ordinary words.";
        let at_limit = guarded_response(text, 112, "stop", 4.0);
        assert!(!at_limit.boundary.guard.rate_exceeded);
        assert!(at_limit.warnings.is_empty());
        let excessive = guarded_response(text, 113, "stop", 4.0);
        assert!(excessive.boundary.guard.rate_exceeded);
        assert_eq!(excessive.boundary.guard.rate_token_allowance, 112.0);
        assert_eq!(excessive.boundary.guard.tokens_per_audio_second, Some(28.25));
        assert_eq!(excessive.boundary.guard.action, "review");
        assert_eq!(excessive.boundary.text, text);
        assert_eq!(excessive.completeness, RecognitionCompleteness::Incomplete);
        assert_eq!(excessive.attempts.len(), 1);
        assert_eq!(excessive.segment.text, text);
        // A single very long generated word is not a short spoken answer.
        let garbage = "x".repeat(512);
        let result = guarded_response(&garbage, 512, "stop", 0.05);
        assert_eq!(result.boundary.guard.action, "review");
        assert_eq!(result.boundary.text, garbage);
        let native = qwen_resolve_decode_with_guards(Ok(NativeSegment {
            text: text.into(), token_count: 113, ..Default::default()
        }), 4.0, true, || panic!("native rate failure must not retry"));
        assert_eq!(native.boundary.guard.action, "review");
        assert_eq!(native.boundary.guard.token_count_source, "native_token_count");
        // Missing or invalid usage is not inferred from characters or bytes.
        for count in [None, Some(-1)] {
            let result = qwen_resolve_decode_with_guards(
                Ok(NativeSegment::from_llama_response((text.into(), count, "stop".into()))),
                0.1, true, || panic!("must not retry missing usage"),
            );
            assert!(!result.boundary.guard.rate_exceeded);
            assert!(result.warnings.is_empty());
        }
    }

    #[test]
    fn qwen_preserves_ordinary_numbers_short_answers_and_cjk_rate_outliers() {
        for text in [
            "Yes.", "No, thanks.", "1234567890123456789012345678901234567890",
            "1.1.1.1.1", "1,000,000,000,000", "2026-09-28", "<asr_text>",
            "今天 天气 很好 我们 下午 去 公园。", "今日は 良い 天気 なので 公園に 行きます。",
            "오늘 날씨가 좋아서 우리는 공원으로 갑니다.",
        ] {
            let result = guarded_response(text, 512, "stop", 0.05);
            assert_eq!(result.boundary.text, qwen_transcript(text));
            assert!(result.warnings.is_empty(), "{text}");
            assert_eq!(result.completeness, RecognitionCompleteness::Complete);
            assert!(result.boundary.guard.rate_exceeded);
            assert!(result.boundary.guard.rate_exemption.is_some());
        }
        for text in [
            "One two three four five six seven eight nine ten.",
            "Call 512 555 0132 at 12:30 on 2026-09-28; use version 1.1.1.1.",
            "I said yes, then she said yes, and they finally agreed.",
            "Go go go. Stop here, please.",
            "Turn left. Turn left. Turn left. Then continue north.",
            "Say <asr_text> literally. Keep the protocol example intact.",
        ] {
            let result = guarded_response(text, 40, "stop", 4.0);
            assert_eq!(result.boundary.text, text);
            assert!(result.warnings.is_empty(), "{text}");
        }
    }

    #[test]
    fn qwen_retry_guards_cannot_revive_ambiguous_or_looped_text() {
        let looped = "language English<asr_text>go go go go";
        for first in [Err("HTTP 503".into()), Ok(NativeSegment {
            text: "< 3".into(), ..Default::default()
        })] {
            let result = qwen_resolve_decode_with_guards(first, 4.0, true, || {
                Ok(NativeSegment::from_llama_response((looped.into(), Some(8), "stop".into())))
            });
            assert!(result.boundary.text.is_empty());
            assert_eq!(result.segment.text, looped);
            assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
            assert_eq!(result.attempts.len(), 2);
            assert_eq!(result.attempts[1]["native_text"], looped);
        }
        // Repetition wins over the existing ambiguous-angle retry route.
        let result = guarded_response("<go go go go", 8, "stop", 4.0);
        assert!(result.boundary.text.is_empty());
        assert_eq!(result.attempts.len(), 1);
        // A length limit from the first attempt still invalidates a partial
        // loop in a retry, even when that retry itself stopped normally.
        let result = qwen_resolve_decode_with_guards(
            Ok(NativeSegment::from_llama_response(("< 3".into(), Some(2), "length".into()))),
            4.0, true, || Ok(NativeSegment::from_llama_response((
                "Keep all four words. go go go go".into(), Some(10), "stop".into(),
            ))),
        );
        assert!(result.boundary.text.is_empty());
        assert_eq!(result.boundary.guard.reason, Some("repetition_at_token_limit"));
    }

    #[test]
    fn qwen_guard_bypass_records_detection_without_changing_baseline_retries() {
        let raw = "language Portuguese<asr_text>Sim. SIM! sim? sim.";
        let result = qwen_resolve_decode_with_guards(
            Ok(NativeSegment::from_llama_response((raw.into(), Some(60), "length".into()))),
            0.5, false, || panic!("baseline length does not retry"),
        );
        assert_eq!(result.boundary.text, "Sim. SIM! sim? sim.");
        assert!(!result.boundary.guard.enabled);
        assert_eq!(result.boundary.guard.action, "bypassed");
        assert_eq!(result.boundary.guard.loops.len(), 1);
        assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
        assert_eq!(result.warnings.len(), 1);
        let result = qwen_resolve_decode_with_guards(
            Err("decode failed".into()), 4.0, false,
            || Ok(NativeSegment::from_llama_response((raw.into(), Some(8), "stop".into()))),
        );
        assert_eq!(result.protocol, "decode_retry_ok");
        assert_eq!(result.boundary.text, "Sim. SIM! sim? sim.");
        assert_eq!(result.attempts[1]["qwen_output_guard"]["enabled"], false);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn qwen_guard_environment_override_is_eval_only() {
        let expected = !cfg!(feature = "fidelity-eval")
            || std::env::var("SONOJOT_EVAL_QWEN_GUARDS").as_deref() != Ok("off");
        assert_eq!(qwen_guards_enabled(), expected);
        let raw = "echo echo echo echo";
        let mut evidence = serde_json::Value::Null;
        let result = transcribe_decoded(&vec![0.1; 16_000], true, || Ok(NativeSegment {
            text: raw.into(), token_count: 8, ..Default::default()
        }), None, Some(&mut evidence)).unwrap();
        assert_eq!(result.text, if expected { "" } else { raw });
        assert_eq!(evidence["qwen_output_guard"]["enabled"], expected);
    }

    #[test]
    fn qwen_empty_audio_never_invokes_decode_or_retry() {
        let result = transcribe_decoded(&[], true,
            || panic!("empty audio must not decode"), None, None).unwrap();
        assert!(result.text.is_empty());
        assert!(result.warnings.is_empty());
        assert_eq!(result.completeness, RecognitionCompleteness::Complete);
    }

    #[test]
    fn qwen_native_and_llama_share_guards_but_other_recognizers_do_not() {
        for qwen in [false, true] {
            let raw = "go go go go";
            let mut evidence = serde_json::Value::Null;
            let samples = vec![0.1; 16_000];
            let result = if qwen {
                transcribe_qwen_native(&samples, 256, || Ok((raw.into(), None, 256)), Some(&mut evidence))
            } else {
                transcribe_decoded(&samples, false, || Ok(NativeSegment {
                    text: raw.into(), token_count: 256, ..Default::default()
                }), None, Some(&mut evidence))
            }.unwrap();
            assert_eq!(result.text, if qwen { "" } else { raw });
            assert_eq!(evidence["native_text"], raw);
            if qwen {
                assert_eq!(evidence["configured_max_new_tokens"], 256);
                assert_eq!(evidence["qwen_output_guard"]["token_count_source"], "native_token_count");
                assert_eq!(evidence["qwen_output_guard"]["rate_exceeded"], true);
                assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
            } else {
                assert!(evidence["qwen_output_guard"].is_null());
                assert!(result.warnings.is_empty());
            }
        }
    }

    #[test]
    fn llama_qwen_uses_shared_boundary_and_explicit_finish_reason() {
        assert!(is_qwen("qwen3-asr-1.7b"));
        assert!(is_qwen("qwen3-asr-1.7b-q8-cuda"));
        assert!(!is_qwen("parakeet-tdt-0.6b-v2"));
        for (raw, count, reason, expected, completeness) in [
            (
                "language English<asr_text>Keep the complete sentence.",
                Some(8),
                "stop",
                "Keep the complete sentence.",
                RecognitionCompleteness::Complete,
            ),
            (
                "language English<asr_text>Keep the partial sentence",
                None,
                "length",
                "Keep the partial sentence",
                RecognitionCompleteness::Incomplete,
            ),
            (
                "language None<asr_text>",
                Some(3),
                "length",
                "",
                RecognitionCompleteness::Incomplete,
            ),
            (
                "Say <asr_text> literally.",
                None,
                "stop",
                "Say <asr_text> literally.",
                RecognitionCompleteness::Complete,
            ),
        ] {
            let recognition = llama_test_recognition((raw.into(), count, reason.into()));
            assert_eq!(recognition.text, expected);
            assert_eq!(recognition.completeness, completeness);
            if completeness == RecognitionCompleteness::Incomplete {
                assert!(recognition.warnings.iter().any(|warning| warning.contains("token limit")));
                assert!(recognition.into_complete_text().is_err());
            } else {
                assert!(recognition.warnings.is_empty());
            }
        }
    }

    #[test]
    fn shared_llama_adapter_retains_retry_finish_evidence_and_incomplete_text() {
        let mut responses = [
            ("<thö".into(), Some(512), "length".into()),
            ("language English<asr_text>Recovered words.".into(), Some(4), "stop".into()),
        ].into_iter();
        let mut evidence = serde_json::Value::Null;
        let result = transcribe_decoded(&vec![0.1; 4 * 16_000], true,
            || Ok(NativeSegment::from_llama_response(responses.next().unwrap())),
            None, Some(&mut evidence)).unwrap();
        assert_eq!(result.text, "Recovered words.");
        assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
        assert!(result.warnings.iter().any(|warning| warning.contains("token limit")));
        assert_eq!(evidence["audio_samples"], 64_000);
        assert_eq!(evidence["configured_max_new_tokens"], 64);
        assert_eq!(evidence["truncation_detected"], true);
        assert_eq!(evidence["attempts"][0]["finish_reason"], "length");
        assert_eq!(evidence["attempts"][1]["finish_reason"], "stop");
        assert!(responses.next().is_none());
    }

    #[test]
    fn llama_length_on_either_review_attempt_stays_incomplete() {
        for (first_reason, next_reason) in [("length", "stop"), ("stop", "length")] {
            let first = NativeSegment::from_llama_response((
                "< 3".into(), Some(2), first_reason.into(),
            ));
            let resolved = qwen_resolve_decode(Ok(first), 4.0, || {
                Ok(NativeSegment::from_llama_response((
                    "language English<asr_text>Retained speech.".into(),
                    Some(4),
                    next_reason.into(),
                )))
            });
            assert_eq!(resolved.boundary.text, "Retained speech.");
            assert_eq!(resolved.completeness, RecognitionCompleteness::Incomplete);
            assert_eq!(resolved.protocol, "review_required");
            assert!(resolved.warnings.iter().any(|warning| warning.contains("token limit")));
            assert_eq!(resolved.attempts.len(), 2);
            assert_eq!(resolved.attempts[0]["finish_reason"], first_reason);
            assert_eq!(resolved.attempts[0]["completion_tokens"], 2);
            assert_eq!(resolved.attempts[1]["finish_reason"], next_reason);
            assert_eq!(resolved.attempts[1]["completion_tokens"], 4);
            assert!(resolved.attempts[0]["native_tokens"].is_null());
            assert!(resolved.attempts[0]["native_token_count"].is_null());
        }
    }

    #[test]
    fn llama_empty_or_failed_retry_keeps_truncated_text_and_metadata() {
        for retry_fails in [false, true] {
            let first = NativeSegment::from_llama_response((
                "< 3".into(), Some(2), "length".into(),
            ));
            let resolved = qwen_resolve_decode(Ok(first), 4.0, || {
                if retry_fails {
                    Err("HTTP 500".into())
                } else {
                    Ok(NativeSegment::from_llama_response((
                        String::new(), Some(0), "stop".into(),
                    )))
                }
            });
            assert_eq!(resolved.boundary.text, "< 3");
            assert_eq!(resolved.segment.text, "< 3");
            assert_eq!(resolved.segment.finish_reason.as_deref(), Some("length"));
            assert_eq!(resolved.segment.completion_tokens, Some(2));
            assert_eq!(resolved.completeness, RecognitionCompleteness::Incomplete);
            assert!(resolved.warnings.iter().any(|warning| warning.contains("token limit")));
        }
    }

    #[test]
    fn llama_http_error_retry_cannot_hide_length_or_blank_output() {
        for (text, reason) in [("Partial speech", "length"), ("", "stop")] {
            let resolved = qwen_resolve_decode(Err("HTTP 503".into()), 4.0, || {
                Ok(NativeSegment::from_llama_response((
                    text.into(), None, reason.into(),
                )))
            });
            assert_eq!(resolved.boundary.text, text);
            assert_eq!(resolved.completeness, RecognitionCompleteness::Incomplete);
            assert!(!resolved.warnings.is_empty());
            assert_eq!(resolved.attempts[0]["decode_error"], "HTTP 503");
            assert_eq!(resolved.attempts[1]["finish_reason"], reason);
            assert!(resolved.attempts[1]["completion_tokens"].is_null());
        }
    }

    #[test]
    #[ignore = "requires installed Qwen ASR and SONOJOT_ASR_SAMPLE"]
    fn installed_qwen_transcribes_from_canonical_store_path() {
        let root = std::fs::canonicalize(
            crate::environment::var_os("MODELS").expect("set SONOJOT_MODELS"),
        )
        .unwrap();
        let wav = PathBuf::from(
            crate::environment::var_os("ASR_SAMPLE").expect("set SONOJOT_ASR_SAMPLE"),
        );
        let started = std::time::Instant::now();
        let text = crate::asr::transcribe_file(
            &root,
            &wav,
            "qwen3-asr-1.7b",
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("installed Qwen must accept the canonical store path");
        assert!(!text.trim().is_empty(), "Qwen returned no words");
        eprintln!(
            "Qwen sample finished in {:?} ({} characters)",
            started.elapsed(),
            text.len()
        );
    }

    #[test]
    fn resample_keeps_same_rate_samples() {
        let samples = [0.0, 0.5, -1.0];
        assert_eq!(resample(&samples, 16_000, 16_000), samples);
        assert!(resample(&samples, 48_000, 16_000).len() <= 2);
    }

    #[test]
    fn live_optional_asr_transcribes_sample_wav_when_staged() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.live-asr");
        let wav = root.join("sample.wav");
        if !crate::optional_asr::model_ready(&root, "parakeet-tdt-0.6b-v2") || !wav.is_file() {
            eprintln!(
                "Skipping live optional ASR: stage rewrite/.live-asr with optional models and sample.wav"
            );
            return;
        }
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let parakeet = crate::asr::transcribe_file(&root, &wav, "parakeet-tdt-0.6b-v2", &cancel)
            .expect("Parakeet TDT 0.6B v2 should transcribe the staged sample");
        println!("parakeet-tdt-0.6b-v2: {parakeet:?}");
        assert!(
            !parakeet.trim().is_empty(),
            "Parakeet returned empty text for the sample wav"
        );
        if crate::optional_asr::model_ready(&root, "qwen3-asr-1.7b") {
            let qwen = crate::asr::transcribe_file(&root, &wav, "qwen3-asr-1.7b", &cancel)
                .expect("Qwen3-ASR 1.7B should transcribe the staged sample");
            println!("qwen3-asr-1.7b: {qwen:?}");
            assert!(
                !qwen.trim().is_empty(),
                "Qwen3-ASR returned empty text for the sample wav"
            );
        }
    }
}
