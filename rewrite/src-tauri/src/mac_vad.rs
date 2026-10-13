//! Silero speech presence and optional section boundaries on the recognition worker.
//! Include on Windows / Apple Silicon only (the platforms with `ort`).
//! No downloading, desktop-startup initialization, or diarization model required.

use crate::{diarization_model, optional_asr};
use super::{audio::check_cancel, speech_gate::Probabilities};
use ort::{
    session::Session,
    value::{Outlet, Tensor, TensorElementType, ValueType},
};
use sha2::{Digest, Sha256};
use std::{
    ffi::CStr,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex},
};

const WINDOW: usize = 512;
const STATE_LEN: usize = 2 * 64;
const MIN_SPEECH_SAMPLES: usize = 4_000; // 0.25 s at 16 kHz
const MIN_SILENCE_SAMPLES: usize = 8_000; // 0.5 s at 16 kHz

pub(super) struct Silero {
    session: Session,
    stream: Stream,
}

impl Silero {
    /// Load only the existing pinned model and CPU runtime. Call on the worker,
    /// before any other `ort` user in this process; errors select the fallback.
    pub(super) fn load(root: &Path) -> Result<Self, String> {
        // Both pins must pass before touching any ORT API, including tensors.
        let model = read_model(&optional_asr::vad_path(root))?;
        let runtime = verified_runtime(root)?;
        initialize_runtime(&runtime)?;

        let session = Session::builder()
            .map_err(ort_error)?
            .with_no_environment_execution_providers()
            .map_err(ort_error)?
            .with_execution_providers([ort::ep::CPU::default().build().error_on_failure()])
            .map_err(ort_error)?
            .with_independent_thread_pool()
            .map_err(ort_error)?
            .with_intra_threads(1)
            .map_err(ort_error)?
            .with_inter_threads(1)
            .map_err(ort_error)?
            .with_parallel_execution(false)
            .map_err(ort_error)?
            .with_intra_op_spinning(false)
            .map_err(ort_error)?
            .with_inter_op_spinning(false)
            .map_err(ort_error)?
            .commit_from_memory(&model)
            .map_err(ort_error)?;
        validate_schema(&session)?;
        let mut silero = Self {
            session,
            stream: Stream::default(),
        };
        // Exercise graph/output compatibility now, so a load failure can select
        // the fallback before recording. This zero frame is not recording state.
        infer(&mut silero.session, &mut silero.stream)?;
        silero.reset();
        Ok(silero)
    }

    /// Start a new recording. Do not reset at a section cut: recurrence and
    /// pending samples must survive the parent's carried-audio sectioning.
    pub(super) fn reset(&mut self) {
        self.stream.reset();
    }

    /// Consume one mono 16 kHz sample; infer once per 512 samples (32 ms).
    /// `true` means qualified speech ended, and stays true during quiet until
    /// the next inferred speech frame. Incomplete frames use the last decision.
    pub(super) fn push(&mut self, sample: f32) -> Result<bool, String> {
        if let Some(probability) = self.push_probability(sample)? {
            return self.stream.gate.observe(probability);
        }
        Ok(self.stream.gate.ended)
    }

    pub(super) fn push_probability(&mut self, sample: f32) -> Result<Option<f32>, String> {
        if self.stream.append(sample)? {
            let decision = infer(&mut self.session, &mut self.stream);
            match decision {
                Ok(probability) => return Ok(Some(probability)),
                Err(error) => {
                    self.stream.failed = true;
                    return Err(error);
                }
            }
        }
        Ok(None)
    }

    /// Analyze only the submitted section with fresh recurrence. The boundary
    /// stream may already include carried lookahead; none of its state or
    /// positive frames can authorize this decode. Its state is left untouched.
    pub(super) fn analyze_section(
        &mut self,
        samples: &[f32],
        cancel: Option<&AtomicBool>,
    ) -> Result<Probabilities, String> {
        analyze_section_with(samples, cancel, |stream| infer(&mut self.session, stream))
    }
}

fn analyze_section_with(
    samples: &[f32],
    cancel: Option<&AtomicBool>,
    mut infer_frame: impl FnMut(&mut Stream) -> Result<f32, String>,
) -> Result<Probabilities, String> {
    let mut stream = Stream::default();
    let mut probabilities = Probabilities::default();
    for frame in samples.chunks(WINDOW) {
        check_cancel(cancel)?;
        if frame.iter().any(|sample| !sample.is_finite()) {
            return Err("Silero received a non-finite audio sample.".into());
        }
        // Pad only the inference copy. Positive duration counts real samples,
        // so a partial tail cannot qualify on invented silence or stale bytes.
        stream.frame.fill(0.0);
        stream.frame[..frame.len()].copy_from_slice(frame);
        let probability = infer_frame(&mut stream)?;
        probabilities.observe(probability, frame.len())?;
    }
    check_cancel(cancel)?;
    Ok(probabilities)
}

fn ort_error(error: impl std::fmt::Display) -> String {
    format!("Silero ONNX Runtime: {error}")
}

fn read_model(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        format!(
            "Optional Silero model unavailable at {}: {e}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() != optional_asr::VAD_BYTES {
        return Err("Optional Silero model does not match its pinned size.".into());
    }
    let file =
        File::open(path).map_err(|e| format!("Cannot open the optional Silero model: {e}"))?;
    // Read/hash the same bounded bytes handed to ORT, not a file later reopened.
    let mut bytes = Vec::with_capacity(optional_asr::VAD_BYTES as usize);
    file.take(optional_asr::VAD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read the optional Silero model: {e}"))?;
    if bytes.len() as u64 != optional_asr::VAD_BYTES
        || format!("{:x}", Sha256::digest(&bytes)) != optional_asr::VAD_SHA256
    {
        return Err("Optional Silero model does not match its pinned checksum.".into());
    }
    Ok(bytes)
}

fn verified_runtime(root: &Path) -> Result<PathBuf, String> {
    let path = runtime_path(root)
        .canonicalize()
        .map_err(|e| format!("Optional Silero runtime unavailable: {e}"))?;
    // Deliberately check only the runtime library, not installed()/runtime_ready():
    // those also require unrelated diarization weights or the sherpa package.
    if !optional_asr::file_matches(&path, diarization_model::runtime_library_pin())? {
        return Err(format!(
            "Optional Silero runtime absent or does not match its pin: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn runtime_path(root: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        optional_asr::mac_speech_runtime_library(root)
    } else {
        diarization_model::runtime_library(root)
    }
}

struct RuntimeOwner {
    path: PathBuf,
    // Static ownership is intentional: ort stores function pointers globally,
    // including its process-exit environment destructor. Never unload this DLL.
    _library: libloading::Library,
    initialized: Result<(), String>,
}

static RUNTIME: Mutex<Option<RuntimeOwner>> = Mutex::new(None);

fn initialize_runtime(path: &Path) -> Result<(), String> {
    let mut owner = RUNTIME
        .lock()
        .map_err(|_| "Silero runtime initialization lock is poisoned.".to_string())?;
    if let Some(runtime) = owner.as_ref() {
        if runtime.path != path {
            return Err(
                "Silero runtime is already owned at a different path; restart the worker.".into(),
            );
        }
        return runtime.initialized.clone();
    }

    // rc.13 init_from returns load errors, but its loader asserts a non-null API
    // base, and api() later expects GetApi to succeed. catch_unwind cannot help
    // with this app's release panic=abort. Check the pinned library explicitly,
    // then use rc.13's supported custom-loading set_api; never enter lazy loading.
    // SAFETY: path is the verified platform library. Its constructors are native
    // code; this is not a sandbox. Keep the handle alive as long as its API is used.
    let library = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("Cannot load optional Silero runtime: {e}"))?;
    // SAFETY: OrtGetApiBase's ABI matches ort-sys rc.13 on both target platforms.
    let getter = unsafe {
        library
            .get::<unsafe extern "system" fn() -> *const ort::sys::OrtApiBase>(b"OrtGetApiBase\0")
    }
    .map_err(|e| format!("Silero runtime has no OrtGetApiBase: {e}"))?;
    // SAFETY: the symbol comes from the pinned runtime and library is still live.
    let base = unsafe { getter().as_ref() }.ok_or("Silero runtime returned a null API base.")?;
    // SAFETY: a valid OrtApiBase owns this null-terminated version string.
    let version = unsafe { (base.GetVersionString)() };
    if version.is_null() {
        return Err("Silero runtime returned a null version string.".into());
    }
    let version = unsafe { CStr::from_ptr(version) }
        .to_str()
        .map_err(|_| "Silero runtime returned an invalid version string.".to_string())?;
    validate_runtime_version(version)?;
    // SAFETY: request exactly the API compiled into ort-sys (17 here). A runtime
    // without that API returns null. No ort::api()/Environment/Session used yet.
    let api = unsafe { (base.GetApi)(ort::sys::ORT_API_VERSION).as_ref() }
        .ok_or("Silero runtime does not provide the required ORT API.")?;
    if !ort::set_api(api.clone()) {
        return Err(
            "Another owner already initialized the ORT API; Silero requires its pinned runtime."
                .into(),
        );
    }
    // From set_api success onward, even a later failure must retain the handle.
    *owner = Some(RuntimeOwner {
        path: path.to_owned(),
        _library: library,
        initialized: Err("Silero runtime initialization did not complete.".into()),
    });
    let configured = ort::init()
        .with_name("sonojot-silero")
        .with_telemetry(false)
        .commit();
    let initialized = if configured {
        ort::environment::Environment::current()
            .map(|_| ())
            .map_err(ort_error)
    } else {
        Err("Another owner already configured the ORT environment; restart the worker.".into())
    };
    if let Some(runtime) = owner.as_mut() {
        runtime.initialized = initialized.clone();
    }
    initialized
}

fn validate_runtime_version(version: &str) -> Result<(), String> {
    let mut parts = version.split('.');
    if parts.next() != Some("1")
        || parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .is_none_or(|minor| minor < ort::sys::ORT_API_VERSION)
    {
        return Err(format!("Unsupported Silero runtime version: {version}"));
    }
    Ok(())
}

fn validate_schema(session: &Session) -> Result<(), String> {
    // The exact pinned bytes identify k2-fsa's Silero v4, 16 kHz-only export:
    // x/h/c -> prob/new_h/new_c. This is not the v5 state/sr + context schema.
    // Confirmed from the local pinned ONNX protobuf and sherpa-onnx v1.13.8:
    // https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/sherpa-onnx/csrc/silero-vad-model.cc
    let inputs = [
        ("x", &[1, 512][..]),
        ("h", &[2, 1, 64][..]),
        ("c", &[2, 1, 64][..]),
    ];
    let outputs = [
        ("prob", &[1, 1][..]),
        ("new_h", &[2, 1, 64][..]),
        ("new_c", &[2, 1, 64][..]),
    ];
    for (outlets, expected, dynamic) in [
        (session.inputs(), &inputs, false),
        (session.outputs(), &outputs, true),
    ] {
        if outlets.len() != expected.len() {
            return Err("Unexpected pinned Silero input/output count.".into());
        }
        for (name, shape) in expected {
            let outlet = outlets
                .iter()
                .find(|outlet| outlet.name() == *name)
                .ok_or_else(|| format!("Silero graph is missing {name}."))?;
            validate_outlet(outlet, shape, dynamic)?;
        }
    }
    Ok(())
}

fn validate_outlet(outlet: &Outlet, expected: &[i64], dynamic: bool) -> Result<(), String> {
    if let ValueType::Tensor {
        ty: TensorElementType::Float32,
        shape,
        ..
    } = outlet.dtype()
    {
        if shape.len() == expected.len()
            && shape
                .iter()
                .zip(expected)
                .all(|(&actual, &wanted)| actual == wanted || (dynamic && actual == -1))
        {
            return Ok(());
        }
    }
    Err(format!(
        "Unexpected Silero tensor schema for {}.",
        outlet.name()
    ))
}

fn infer(session: &mut Session, stream: &mut Stream) -> Result<f32, String> {
    let x = Tensor::from_array(([1, WINDOW], stream.frame.to_vec())).map_err(ort_error)?;
    let h = Tensor::from_array(([2usize, 1, 64], stream.h.to_vec())).map_err(ort_error)?;
    let c = Tensor::from_array(([2usize, 1, 64], stream.c.to_vec())).map_err(ort_error)?;
    let outputs = session
        .run(ort::inputs!["x" => x, "h" => h, "c" => c])
        .map_err(ort_error)?;
    let probability = outputs
        .get("prob")
        .ok_or("Silero returned no probability.")?
        .try_extract_tensor::<f32>()
        .map_err(ort_error)?;
    let h = outputs
        .get("new_h")
        .ok_or("Silero returned no hidden state.")?
        .try_extract_tensor::<f32>()
        .map_err(ort_error)?;
    let c = outputs
        .get("new_c")
        .ok_or("Silero returned no cell state.")?
        .try_extract_tensor::<f32>()
        .map_err(ort_error)?;
    validate_tensor("prob", probability.0, probability.1, &[1, 1], 1)?;
    validate_tensor("new_h", h.0, h.1, &[2, 1, 64], STATE_LEN)?;
    validate_tensor("new_c", c.0, c.1, &[2, 1, 64], STATE_LEN)?;
    let probability = probability.1[0];
    validate_probability(probability)?;
    // Commit recurrence only after every output passes validation.
    stream.h.copy_from_slice(h.1);
    stream.c.copy_from_slice(c.1);
    Ok(probability)
}

fn validate_tensor(
    name: &str,
    shape: &[i64],
    data: &[f32],
    expected: &[i64],
    len: usize,
) -> Result<(), String> {
    if shape != expected || data.len() != len || data.iter().any(|v| !v.is_finite()) {
        return Err(format!("Silero returned invalid {name} shape or data."));
    }
    Ok(())
}

fn validate_probability(probability: f32) -> Result<(), String> {
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err("Silero returned an invalid speech probability.".into());
    }
    Ok(())
}

struct Stream {
    frame: [f32; WINDOW],
    filled: usize,
    h: [f32; STATE_LEN],
    c: [f32; STATE_LEN],
    gate: Gate,
    failed: bool,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            frame: [0.0; WINDOW],
            filled: 0,
            h: [0.0; STATE_LEN],
            c: [0.0; STATE_LEN],
            gate: Gate::default(),
            failed: false,
        }
    }
}

impl Stream {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn append(&mut self, sample: f32) -> Result<bool, String> {
        if self.failed {
            return Err("Silero failed for this recording; use energy sectioning.".into());
        }
        if !sample.is_finite() {
            self.failed = true;
            return Err("Silero received a non-finite audio sample.".into());
        }
        self.frame[self.filled] = sample;
        self.filled += 1;
        if self.filled == WINDOW {
            self.filled = 0;
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Default)]
struct Gate {
    speech_samples: usize,
    silence_samples: usize,
    speaking: bool,
    ended: bool,
}

impl Gate {
    fn observe(&mut self, probability: f32) -> Result<bool, String> {
        validate_probability(probability)?;
        if probability >= crate::qwen_window::vad_threshold() {
            self.ended = false;
            self.silence_samples = 0;
            self.speech_samples = (self.speech_samples + WINDOW).min(MIN_SPEECH_SAMPLES);
            if self.speech_samples >= MIN_SPEECH_SAMPLES {
                self.speaking = true;
            }
        } else if self.speaking {
            self.silence_samples = (self.silence_samples + WINDOW).min(MIN_SILENCE_SAMPLES);
            if self.silence_samples >= MIN_SILENCE_SAMPLES {
                self.speaking = false;
                self.speech_samples = 0;
                self.ended = true;
            }
        } else {
            self.speech_samples = 0;
        }
        Ok(self.ended)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_inference_has_fresh_recurrence_and_only_the_submitted_audio() {
        let mut audio = vec![0.0; 4 * WINDOW];
        audio.extend(vec![0.5; 4 * WINDOW]); // Future speech is outside the first cut.
        let analyze = |samples: &[f32]| {
            let mut frames = 0;
            analyze_section_with(samples, None, |stream| {
                assert_eq!(stream.h[0], frames as f32);
                assert_eq!(stream.c[0], -(frames as f32));
                frames += 1;
                stream.h[0] = frames as f32;
                stream.c[0] = -(frames as f32);
                Ok(if stream.frame.iter().any(|&s| s > 0.0) { 0.8 } else { 0.0 })
            }).unwrap()
        };
        assert!(!analyze(&audio[..4 * WINDOW]).speech());
        assert!(analyze(&audio[4 * WINDOW..]).speech());
        assert!(!analyze(&audio[..4 * WINDOW]).speech()); // No stale positive state.
    }

    #[test]
    fn partial_section_frame_is_zero_padded_and_counts_only_real_samples() {
        let samples = vec![0.3; 2 * WINDOW + WINDOW - 1];
        let mut calls = 0;
        let probabilities = analyze_section_with(&samples, None, |stream| {
            calls += 1;
            if calls == 3 {
                assert_eq!(stream.frame[WINDOW - 1], 0.0);
                assert!(stream.frame[..WINDOW - 1].iter().all(|&s| s == 0.3));
            }
            Ok(0.9)
        }).unwrap();
        assert_eq!(probabilities.samples, samples.len());
        assert_eq!(probabilities.positive_samples, samples.len());
        assert_eq!(calls, 3);
        assert!(!probabilities.speech()); // Padding cannot supply the missing sample.
    }

    #[test]
    fn section_error_after_positive_frames_does_not_return_a_positive_decision() {
        let samples = vec![0.2; 4 * WINDOW];
        let mut calls = 0;
        assert!(analyze_section_with(&samples, None, |_| {
            calls += 1;
            if calls == 4 { Err("inference failure".into()) } else { Ok(0.9) }
        }).is_err());
        assert_eq!(calls, 4); // Check the whole section even after qualification.
        assert!(analyze_section_with(&samples, None, |_| Ok(f32::NAN)).is_err());
        assert!(analyze_section_with(&[f32::INFINITY], None, |_| panic!("invalid input")).is_err());
    }

    #[test]
    fn section_cancellation_stops_before_and_between_inferences() {
        use std::sync::atomic::Ordering;
        let cancel = AtomicBool::new(true);
        let samples = vec![0.2; 4 * WINDOW];
        assert!(analyze_section_with(&samples, Some(&cancel), |_| panic!("cancelled")).is_err());
        cancel.store(false, Ordering::Release);
        let mut calls = 0;
        let result = analyze_section_with(&samples, Some(&cancel), |_| {
            calls += 1;
            cancel.store(true, Ordering::Release);
            Ok(0.9)
        });
        assert_eq!(calls, 1);
        assert_eq!(result.unwrap_err(), "Transcription cancelled.");
    }

    fn frames(gate: &mut Gate, probability: f32, count: usize, expected: bool) {
        for _ in 0..count {
            assert_eq!(gate.observe(probability).unwrap(), expected);
        }
    }

    #[test]
    fn gate_requires_consecutive_minimum_speech_before_a_boundary() {
        let mut gate = Gate::default();
        let threshold = crate::qwen_window::vad_threshold();
        frames(&mut gate, 0.0, 100, false);
        frames(&mut gate, threshold, 7, false); // 224 ms is too short.
        frames(&mut gate, 0.0, 20, false);
        frames(&mut gate, threshold, 7, false); // Earlier speech cannot accumulate.
        frames(&mut gate, 0.0, 20, false);
        frames(&mut gate, threshold, 8, false); // 256 ms qualifies.
        frames(&mut gate, 0.0, 15, false); // 480 ms is too short.
        frames(&mut gate, 0.0, 1, true); // 512 ms ends qualified speech.
    }

    #[test]
    fn gate_silence_must_be_contiguous_and_end_persists_until_renewed_speech() {
        let mut gate = Gate::default();
        let threshold = crate::qwen_window::vad_threshold();
        frames(&mut gate, 0.8, 8, false);
        frames(&mut gate, 0.1, 15, false);
        frames(&mut gate, 0.8, 1, false);
        frames(&mut gate, 0.1, 15, false);
        frames(&mut gate, 0.1, 1, true);
        frames(&mut gate, 0.1, 100, true); // Parent may still be below MIN 3 s.
        frames(&mut gate, threshold, 1, false);
        frames(&mut gate, 0.1, 20, false); // A short new burst is not qualified.
        frames(&mut gate, threshold, 8, false);
        frames(&mut gate, 0.1, 15, false);
        frames(&mut gate, 0.1, 1, true);
    }

    #[test]
    fn buffering_consumes_every_sample_and_preserves_quiet_decision() {
        let mut stream = Stream::default();
        stream.gate.ended = true;
        for frame in 0..3 {
            for index in 0..WINDOW {
                let sample = (frame * WINDOW + index) as f32 / 2_000.0;
                assert_eq!(stream.append(sample).unwrap(), index == WINDOW - 1);
                assert_eq!(stream.frame[index], sample);
                assert!(stream.gate.ended);
            }
        }
        assert_eq!(stream.filled, 0);
    }

    #[test]
    fn reset_state_clears_partial_frame_recurrence_gate_and_failure() {
        let mut stream = Stream::default();
        stream.append(0.5).unwrap();
        stream.h.fill(1.0);
        stream.c.fill(-1.0);
        frames(&mut stream.gate, 0.8, 8, false);
        frames(&mut stream.gate, 0.0, 15, false);
        frames(&mut stream.gate, 0.0, 1, true);
        assert!(stream.append(f32::NAN).is_err());
        assert!(stream.append(0.0).is_err());
        // Silero::reset delegates here; this needs no ORT session.
        stream.reset();
        assert_eq!(stream.filled, 0);
        assert!(stream
            .frame
            .iter()
            .chain(&stream.h)
            .chain(&stream.c)
            .all(|v| *v == 0.0));
        assert!(!stream.failed);
        frames(&mut stream.gate, 0.0, 100, false);
        assert!(!stream.append(0.0).unwrap());
        frames(&mut stream.gate, 0.8, 8, false);
        stream.reset(); // Reset during active speech must also prevent a stale end.
        frames(&mut stream.gate, 0.0, 100, false);
    }

    #[test]
    fn invalid_probabilities_and_recurrent_outputs_are_rejected() {
        let mut gate = Gate::default();
        for probability in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.001, 1.001] {
            assert!(gate.observe(probability).is_err());
        }
        assert_eq!(gate.speech_samples, 0);
        let mut state = [0.0; STATE_LEN];
        assert!(validate_tensor("h", &[2, 1, 64], &state, &[2, 1, 64], STATE_LEN).is_ok());
        assert!(validate_tensor("h", &[1, 2, 64], &state, &[2, 1, 64], STATE_LEN).is_err());
        assert!(validate_tensor("h", &[2, 1, 64], &state[..127], &[2, 1, 64], STATE_LEN).is_err());
        state[100] = f32::INFINITY;
        assert!(validate_tensor("h", &[2, 1, 64], &state, &[2, 1, 64], STATE_LEN).is_err());
        assert!(validate_tensor("prob", &[1], &[0.5], &[1, 1], 1).is_err());
        assert!(validate_tensor("prob", &[1, 1], &[f32::NAN], &[1, 1], 1).is_err());
    }

    #[test]
    fn runtime_version_requires_the_compiled_api() {
        assert!(validate_runtime_version("1.28.2").is_ok());
        assert!(validate_runtime_version(&format!("1.{}.0", ort::sys::ORT_API_VERSION)).is_ok());
        for version in ["", "garbage", "1.x.0", "1.16.0", "2.28.2"] {
            assert!(validate_runtime_version(version).is_err());
        }
    }

    #[test]
    fn absent_or_invalid_optional_files_fail_before_ort_access() {
        let root = std::env::temp_dir().join(format!("sonojot-mac-vad-{}", crate::store::new_id()));
        assert!(Silero::load(&root).is_err());
        assert!(verified_runtime(&root).is_err());
        let path = optional_asr::vad_path(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid").unwrap();
        assert!(Silero::load(&root).is_err());
        // Correct length still requires the exact checksum. No ORT needed.
        File::create(&path)
            .unwrap()
            .set_len(optional_asr::VAD_BYTES)
            .unwrap();
        assert!(Silero::load(&root).is_err());
        let runtime = runtime_path(&root);
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&runtime, b"invalid").unwrap();
        assert!(verified_runtime(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
