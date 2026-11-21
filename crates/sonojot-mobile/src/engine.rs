use crate::{
    language_of, postprocess, AudioGateFilter, EngineError, GateConfig, ModelPackManifest, Recognizer,
    SherpaRecognizer, SherpaRuntime, VerifyMode, DEFAULT_RUNTIME_LIBRARY, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, path::PathBuf};

/// How to open an engine. Deserialized from JSON at the C ABI; unknown fields are refused so a
/// misspelt setting fails loudly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SonojotConfig {
    /// The model pack directory (holding `manifest.json`).
    pub pack_dir: PathBuf,
    /// The language to transcribe, as a locale tag (`en-US`, `ar-JO`, `tr-TR`).
    pub locale: String,
    /// The sherpa-onnx C API: a file name for the platform loader, a path, or
    /// [`crate::RUNTIME_LIBRARY_SELF`] when it is linked into the app.
    pub runtime_library: String,
    /// Decoder threads (1–8).
    pub num_threads: i32,
    /// Must be [`VerifyMode::Full`]: opening always hashes every file before native code reads
    /// it, and `sizes` is refused. Size-only checks belong to `sonojot_pack_inspect`. The field
    /// stays so configs that spell out `"verify": "full"` keep working.
    pub verify: VerifyMode,
    pub gate: GateConfig,
    /// Apply SonoJot's spoken-to-written rules to English transcripts.
    pub written_form: bool,
}

impl Default for SonojotConfig {
    fn default() -> Self {
        Self {
            pack_dir: PathBuf::new(),
            locale: String::new(),
            runtime_library: DEFAULT_RUNTIME_LIBRARY.into(),
            num_threads: 2,
            verify: VerifyMode::Full,
            gate: GateConfig::default(),
            written_form: true,
        }
    }
}

/// One decoded region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SegmentResult {
    pub start_ms: u64,
    pub end_ms: u64,
    /// After [`postprocess::clean_segment`].
    pub text: String,
}

/// A finished transcript.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TranscriptResult {
    /// The cleaned transcript; empty when no speech was recognized.
    pub text: String,
    /// What the recognizer returned, joined, before any cleanup.
    pub raw_text: String,
    /// The language decoded (`en`, `ar`, `tr`…).
    pub language: String,
    /// The model pack's id.
    pub model_id: String,
    pub segments: Vec<SegmentResult>,
    pub audio_ms: u64,
    /// Audio the recognizer actually heard.
    pub speech_ms: u64,
    /// `rnnoise-gate` or `off`.
    pub gate_mode: String,
    /// The gate found no speech but passed audible audio through anyway.
    pub gate_fail_open: bool,
}

/// Offline transcription of whole recordings with one model pack.
pub struct SonojotEngine {
    recognizer: Box<dyn Recognizer>,
    gate: AudioGateFilter,
    manifest: ModelPackManifest,
    language: String,
    written_form: bool,
}

impl std::fmt::Debug for SonojotEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SonojotEngine")
            .field("model_id", &self.manifest.id)
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

fn ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

/// The longest recording one call accepts: 30 minutes. Transcription keeps a few copies of the
/// audio (bytes, samples, the gate's denoised copy), so a much longer buffer could exhaust a
/// phone's memory and abort the app. Split longer recordings.
pub const MAX_AUDIO_SAMPLES: usize = 30 * 60 * SAMPLE_RATE as usize;

fn check_length(samples: usize) -> Result<(), EngineError> {
    if samples > MAX_AUDIO_SAMPLES {
        return Err(EngineError::InvalidAudio(format!(
            "{} s of audio is longer than the {} s limit; split the recording",
            samples / SAMPLE_RATE as usize,
            MAX_AUDIO_SAMPLES / SAMPLE_RATE as usize
        )));
    }
    Ok(())
}

impl SonojotEngine {
    /// Loads the pack's manifest, checks it covers the locale, loads the sherpa-onnx runtime,
    /// verifies the pack's files and creates the recognizer, in that order (cheapest first).
    pub fn open(config: SonojotConfig) -> Result<Self, EngineError> {
        Self::check_config(&config)?;
        let manifest = ModelPackManifest::load(&config.pack_dir)?;
        let language = Self::language_for(&config, &manifest)?;
        let runtime = SherpaRuntime::open(&config.runtime_library)?;
        manifest.verify(&config.pack_dir, VerifyMode::Full)?;
        let recognizer = SherpaRecognizer::new(runtime, &manifest, &config.pack_dir, &language, config.num_threads)?;
        Self::with_recognizer(&config, manifest, Box::new(recognizer))
    }

    /// An engine around any [`Recognizer`] (another runtime, or a test double). The manifest is
    /// validated but its files are not read.
    pub fn with_recognizer(
        config: &SonojotConfig,
        manifest: ModelPackManifest,
        recognizer: Box<dyn Recognizer>,
    ) -> Result<Self, EngineError> {
        Self::check_config(config)?;
        manifest.validate()?;
        let language = Self::language_for(config, &manifest)?;
        let mut gate = config.gate.clone();
        for limit in [manifest.options.max_segment_ms, recognizer.max_segment_ms()].into_iter().flatten() {
            gate.max_segment_ms = gate.max_segment_ms.min(limit);
        }
        gate.padding_ms = gate.padding_ms.min(gate.max_segment_ms / 4);
        Ok(Self { recognizer, gate: AudioGateFilter::new(gate)?, manifest, language, written_form: config.written_form })
    }

    fn check_config(config: &SonojotConfig) -> Result<(), EngineError> {
        if !(1..=8).contains(&config.num_threads) {
            return Err(EngineError::InvalidArgument("num_threads must be 1–8".into()));
        }
        // A corrupt file of the right size would reach ONNX Runtime unhashed, and sherpa-onnx may
        // end the process on bad model input instead of returning an error.
        if config.verify != VerifyMode::Full {
            return Err(EngineError::InvalidArgument(
                "verify must be \"full\" to open an engine; use sonojot_pack_inspect for size-only checks".into(),
            ));
        }
        config.gate.validate()
    }

    fn language_for(config: &SonojotConfig, manifest: &ModelPackManifest) -> Result<String, EngineError> {
        let language = language_of(&config.locale);
        // A pack for exactly one language may be used without naming it.
        let language = if language.is_empty() && manifest.languages.len() == 1 && manifest.languages[0] != "*" {
            manifest.languages[0].clone()
        } else {
            language
        };
        if !manifest.supports(&language) {
            return Err(EngineError::UnsupportedLocale { locale: config.locale.clone(), pack: manifest.id.clone() });
        }
        Ok(language)
    }

    pub fn manifest(&self) -> &ModelPackManifest {
        &self.manifest
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn gate(&self) -> &AudioGateFilter {
        &self.gate
    }

    /// Transcribes a whole recording of 16 kHz mono samples, at most [`MAX_AUDIO_SAMPLES`].
    /// Non-finite samples are read as silence. Returns an empty `text` when nothing was recognized.
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<TranscriptResult, EngineError> {
        check_length(samples.len())?;
        let audio: Cow<'_, [f32]> = if samples.iter().all(|x| x.is_finite()) {
            Cow::Borrowed(samples)
        } else {
            Cow::Owned(samples.iter().map(|&x| if x.is_finite() { x } else { 0.0 }).collect())
        };
        let gated = self.gate.analyze(&audio);
        let mut raw_parts = Vec::with_capacity(gated.segments.len());
        let mut segments = Vec::with_capacity(gated.segments.len());
        for range in &gated.segments {
            let raw = self.recognizer.decode(&audio[range.clone()])?;
            let text = postprocess::clean_segment(&raw);
            if !text.is_empty() {
                segments.push(SegmentResult { start_ms: ms(range.start), end_ms: ms(range.end), text });
            }
            raw_parts.push(raw);
        }
        let raw_text = postprocess::join(raw_parts.iter().map(String::as_str));
        let joined = postprocess::join(segments.iter().map(|s| s.text.as_str()));
        Ok(TranscriptResult {
            text: postprocess::finish(&joined, &self.language, self.written_form),
            raw_text,
            language: self.language.clone(),
            model_id: self.recognizer.model_id().to_owned(),
            segments,
            audio_ms: ms(audio.len()),
            speech_ms: ms(gated.speech_samples()),
            gate_mode: gated.mode,
            gate_fail_open: gated.fail_open,
        })
    }

    /// [`Self::transcribe`] for little-endian `f32` bytes, the format Android audio code usually
    /// hands across a JNI or JNA boundary.
    pub fn transcribe_f32le(&mut self, bytes: &[u8]) -> Result<TranscriptResult, EngineError> {
        if bytes.len() % 4 != 0 {
            return Err(EngineError::InvalidAudio(format!("{} bytes is not a whole number of f32 samples", bytes.len())));
        }
        // Before the samples are copied out of the bytes.
        check_length(bytes.len() / 4)?;
        let samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        self.transcribe(&samples)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        manifest::tests::{write_pack, TempDir},
        GateMode, ModelFamily,
    };
    use std::sync::{Arc, Mutex};

    /// Records every region it is given and answers from a script.
    pub(crate) struct ScriptedRecognizer {
        pub heard: Arc<Mutex<Vec<usize>>>,
        pub replies: Vec<Result<String, EngineError>>,
        pub window: Option<u32>,
    }

    impl Recognizer for ScriptedRecognizer {
        fn decode(&mut self, samples: &[f32]) -> Result<String, EngineError> {
            self.heard.lock().unwrap().push(samples.len());
            if self.replies.is_empty() { Ok(String::new()) } else { self.replies.remove(0) }
        }
        fn model_id(&self) -> &str {
            "scripted"
        }
        fn max_segment_ms(&self) -> Option<u32> {
            self.window
        }
    }

    pub(crate) fn scripted(replies: &[&str]) -> (Box<ScriptedRecognizer>, Arc<Mutex<Vec<usize>>>) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let replies = replies.iter().map(|r| Ok(r.to_string())).collect();
        (Box::new(ScriptedRecognizer { heard: heard.clone(), replies, window: None }), heard)
    }

    fn pack(dir: &TempDir, languages: &[&str]) -> ModelPackManifest {
        write_pack(&dir.0, ModelFamily::Whisper, languages, &["encoder", "decoder", "tokens"])
    }

    fn config(locale: &str) -> SonojotConfig {
        SonojotConfig { locale: locale.into(), gate: GateConfig { mode: GateMode::Off, ..GateConfig::default() }, ..SonojotConfig::default() }
    }

    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize).map(|i| (i as f32 * 0.07).sin() * 0.3).collect()
    }

    #[test]
    fn config_json_has_defaults_and_refuses_typos() {
        let parsed: SonojotConfig = serde_json::from_str(r#"{"pack_dir":"/p","locale":"ar-JO"}"#).unwrap();
        assert_eq!(parsed.num_threads, 2);
        assert_eq!(parsed.verify, VerifyMode::Full);
        assert_eq!(parsed.runtime_library, DEFAULT_RUNTIME_LIBRARY);
        assert!(parsed.written_form);
        assert!(serde_json::from_str::<SonojotConfig>(r#"{"pack_dir":"/p","locale":"en","threads":4}"#).is_err());
        let gate: SonojotConfig = serde_json::from_str(r#"{"gate":{"mode":"off","padding_ms":100}}"#).unwrap();
        assert_eq!(gate.gate.mode, GateMode::Off);
        assert_eq!(gate.gate.frame_ms, 30);
    }

    #[test]
    fn locales_must_be_covered_by_the_pack() {
        let dir = TempDir::new("engine-locale");
        let english = pack(&dir, &["en"]);
        let (recognizer, _) = scripted(&[]);
        let error = SonojotEngine::with_recognizer(&config("ar-JO"), english.clone(), recognizer).unwrap_err();
        assert_eq!(error.code(), "unsupported_locale");
        let (recognizer, _) = scripted(&[]);
        let engine = SonojotEngine::with_recognizer(&config(""), english, recognizer).unwrap();
        assert_eq!(engine.language(), "en");
        let multi = pack(&dir, &["*"]);
        for locale in ["ar-SA", "tr-TR", "en-GB"] {
            let (recognizer, _) = scripted(&[]);
            let engine = SonojotEngine::with_recognizer(&config(locale), multi.clone(), recognizer).unwrap();
            assert_eq!(engine.language(), &locale[..2]);
        }
        let (recognizer, _) = scripted(&[]);
        assert_eq!(SonojotEngine::with_recognizer(&config(""), multi, recognizer).unwrap_err().code(), "unsupported_locale");
    }

    #[test]
    fn long_audio_reaches_the_recognizer_in_model_sized_regions_and_is_cleaned() {
        let dir = TempDir::new("engine-regions");
        let (mut recognizer, heard) = scripted(&["<|en|> I PAID", "[BLANK_AUDIO]", "TWENTY FIVE DOLLARS"]);
        recognizer.window = Some(20_000);
        let mut engine = SonojotEngine::with_recognizer(&config("en-US"), pack(&dir, &["en"]), recognizer).unwrap();
        assert_eq!(engine.gate().config().max_segment_ms, 20_000);
        let audio = tone(50.0);
        let result = engine.transcribe(&audio).unwrap();
        let heard = heard.lock().unwrap().clone();
        assert_eq!(heard.iter().sum::<usize>(), audio.len());
        assert!(heard.iter().all(|&n| n <= 20 * 16_000), "{heard:?}");
        assert_eq!(result.raw_text, "<|en|> I PAID [BLANK_AUDIO] TWENTY FIVE DOLLARS");
        assert_eq!(result.segments.len(), 2);
        assert!(result.text.starts_with("I paid "), "{}", result.text);
        assert!(result.text.contains("$25"), "{}", result.text);
        assert_eq!(result.audio_ms, 50_000);
        assert_eq!(result.speech_ms, 50_000);
        assert_eq!(result.model_id, "scripted");
        assert_eq!(result.gate_mode, "off");
    }

    #[test]
    fn rnnoise_gate_skips_silence_and_keeps_audible_audio() {
        let dir = TempDir::new("engine-gate");
        let (recognizer, heard) = scripted(&["مرحبا"]);
        let cfg = SonojotConfig { locale: "ar".into(), ..SonojotConfig::default() };
        let mut engine = SonojotEngine::with_recognizer(&cfg, pack(&dir, &["ar", "tr"]), recognizer).unwrap();
        let silent = engine.transcribe(&vec![0.0; 48_000]).unwrap();
        assert_eq!(silent.text, "");
        assert!(heard.lock().unwrap().is_empty());
        let mut audio = vec![0.0; 32_000];
        audio.extend(tone(1.0));
        audio.extend(vec![0.0; 32_000]);
        let spoken = engine.transcribe(&audio).unwrap();
        assert_eq!(spoken.text, "مرحبا");
        assert_eq!(spoken.gate_mode, "rnnoise-gate");
        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 1);
        assert!(heard[0] >= 16_000 && heard[0] < audio.len(), "{heard:?}");
    }

    #[test]
    fn little_endian_bytes_and_bad_samples_are_handled() {
        let dir = TempDir::new("engine-bytes");
        let (recognizer, heard) = scripted(&["ok"]);
        let mut engine = SonojotEngine::with_recognizer(&config("tr"), pack(&dir, &["*"]), recognizer).unwrap();
        let mut samples = tone(0.5);
        samples[10] = f32::NAN;
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(engine.transcribe_f32le(&bytes).unwrap().text, "ok");
        assert_eq!(heard.lock().unwrap().as_slice(), &[8_000]);
        assert_eq!(engine.transcribe_f32le(&bytes[..5]).unwrap_err().code(), "invalid_audio");
        assert_eq!(engine.transcribe_f32le(&[]).unwrap().text, "");
    }

    #[test]
    fn recordings_over_thirty_minutes_are_refused_before_copying() {
        let dir = TempDir::new("engine-long");
        let (recognizer, heard) = scripted(&[]);
        let mut engine = SonojotEngine::with_recognizer(&config("en"), pack(&dir, &["en"]), recognizer).unwrap();
        // Zeroed allocations stay untouched (and so cheap) because the length is checked first.
        let samples = vec![0.0f32; MAX_AUDIO_SAMPLES + 1];
        assert_eq!(engine.transcribe(&samples).unwrap_err().code(), "invalid_audio");
        drop(samples);
        let bytes = vec![0u8; (MAX_AUDIO_SAMPLES + 1) * 4];
        assert_eq!(engine.transcribe_f32le(&bytes).unwrap_err().code(), "invalid_audio");
        assert!(heard.lock().unwrap().is_empty());
        assert!(check_length(MAX_AUDIO_SAMPLES).is_ok());
    }

    #[test]
    fn recognizer_errors_propagate() {
        let dir = TempDir::new("engine-error");
        let heard = Arc::new(Mutex::new(Vec::new()));
        let recognizer = Box::new(ScriptedRecognizer { heard, replies: vec![Err(EngineError::Decode("boom".into()))], window: None });
        let mut engine = SonojotEngine::with_recognizer(&config("en"), pack(&dir, &["en"]), recognizer).unwrap();
        assert_eq!(engine.transcribe(&tone(1.0)).unwrap_err(), EngineError::Decode("boom".into()));
    }

    #[test]
    fn open_reports_missing_pack_before_touching_the_runtime() {
        let dir = TempDir::new("engine-open");
        let cfg = SonojotConfig { pack_dir: dir.0.clone(), locale: "en".into(), runtime_library: "libnope.so".into(), ..SonojotConfig::default() };
        assert_eq!(SonojotEngine::open(cfg.clone()).unwrap_err().code(), "pack_missing");
        pack(&dir, &["en"]);
        assert_eq!(SonojotEngine::open(SonojotConfig { locale: "ar".into(), ..cfg.clone() }).unwrap_err().code(), "unsupported_locale");
        assert_eq!(SonojotEngine::open(cfg.clone()).unwrap_err().code(), "runtime_missing");
        assert_eq!(SonojotEngine::open(SonojotConfig { num_threads: 0, ..cfg.clone() }).unwrap_err().code(), "invalid_argument");
    }

    #[test]
    fn engines_only_open_fully_hashed_packs() {
        // "full" spelled out (as the Android app sends it) still parses; "sizes" is refused
        // before the pack or the runtime is touched.
        let full: SonojotConfig = serde_json::from_str(r#"{"pack_dir":"/p","locale":"en","verify":"full"}"#).unwrap();
        assert_eq!(full.verify, VerifyMode::Full);
        let sizes: SonojotConfig = serde_json::from_str(r#"{"pack_dir":"/nowhere","locale":"en","verify":"sizes"}"#).unwrap();
        let error = SonojotEngine::open(sizes.clone()).unwrap_err();
        assert_eq!(error.code(), "invalid_argument", "{error}");
        assert!(error.to_string().contains("full"), "{error}");
        let dir = TempDir::new("engine-sizes");
        let (recognizer, _) = scripted(&[]);
        let error = SonojotEngine::with_recognizer(&sizes, pack(&dir, &["en"]), recognizer).unwrap_err();
        assert_eq!(error.code(), "invalid_argument");
    }
}
