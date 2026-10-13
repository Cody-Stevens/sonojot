//! Apple Silicon sectioning with whisper.cpp or the bundled llama.cpp Metal
//! decoder. Native state belongs to the existing recognition worker.
use std::path::{Path, PathBuf};

#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), all(test, windows)))]
#[path = "mac_vad.rs"]
mod vad;

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
mod vad {
    pub struct Silero;
    impl Silero {
        pub fn load(_: &std::path::Path) -> Result<Self, String> {
            Err("Silero requires Apple Silicon.".into())
        }
        pub fn reset(&mut self) {}
        pub fn push(&mut self, _: f32) -> Result<bool, String> { Ok(false) }
        pub fn push_probability(&mut self, _: f32) -> Result<Option<f32>, String> {
            Err("Silero requires Apple Silicon.".into())
        }
        pub fn analyze_section(
            &mut self,
            _: &[f32],
            _: Option<&std::sync::atomic::AtomicBool>,
        ) -> Result<super::speech_gate::Probabilities, String> {
            Err("Silero requires Apple Silicon.".into())
        }
    }
}

pub const REPO: &str = "ggerganov/whisper.cpp";
pub const BACKEND: &str = "whisper.cpp via whisper-rs 0.16.0 (CPU + Metal)";
pub const COMPATIBILITY: &str =
    "Apple Silicon macOS; CPU + Metal GPU. Initialization is checked; GPU execution is not measured.";

#[derive(Clone, Copy)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub revision: &'static str,
    /// Estimated total worker memory, including shared CPU/GPU allocations.
    /// These are planning envelopes, not measurements on the current Mac.
    pub ram_min_bytes: u64,
    pub ram_max_bytes: u64,
}

// Hugging Face LFS size/SHA256 metadata verified at this immutable revision:
// https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1
const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const MODELS: &[Model] = &[
    Model {
        id: "whisper-small-metal",
        name: "Whisper Small · Metal",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        revision: REVISION,
        ram_min_bytes: 1_000_000_000,
        ram_max_bytes: 2_000_000_000,
    },
    Model {
        id: "whisper-turbo-metal",
        name: "Whisper Turbo · Metal",
        file: "ggml-large-v3-turbo.bin",
        bytes: 1_624_555_275,
        sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
        revision: REVISION,
        ram_min_bytes: 2_500_000_000,
        ram_max_bytes: 4_000_000_000,
    },
];

// Evaluation candidates use the exact production worker and decoder. Keep
// them out of the desktop catalog until measured quality warrants a product
// decision. The immutable revision and file verification match MODELS above.
#[cfg(feature = "fidelity-eval")]
const EVALUATION_MODELS: &[Model] = &[
    Model {
        id: "whisper-small-en-metal",
        name: "Whisper Small English · Metal (evaluation)",
        file: "ggml-small.en.bin",
        bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        revision: REVISION,
        ram_min_bytes: 1_000_000_000,
        ram_max_bytes: 2_000_000_000,
    },
    Model {
        id: "whisper-tiny-en-metal",
        name: "Whisper Tiny English · Metal (evaluation)",
        file: "ggml-tiny.en.bin",
        bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        revision: REVISION,
        ram_min_bytes: 300_000_000,
        ram_max_bytes: 800_000_000,
    },
    Model {
        id: "whisper-base-en-metal",
        name: "Whisper Base English · Metal (evaluation)",
        file: "ggml-base.en.bin",
        bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        revision: REVISION,
        ram_min_bytes: 500_000_000,
        ram_max_bytes: 1_000_000_000,
    },
];

pub fn catalog() -> &'static [Model] {
    MODELS
}

pub fn find(id: &str) -> Option<&'static Model> {
    let production = MODELS.iter().find(|model| model.id == id);
    #[cfg(feature = "fidelity-eval")]
    {
        production.or_else(|| EVALUATION_MODELS.iter().find(|model| model.id == id))
    }
    #[cfg(not(feature = "fidelity-eval"))]
    production
}

pub fn model_dir(root: &Path, id: &str) -> PathBuf {
    root.join("optional").join(id)
}

pub fn model_ready(root: &Path, id: &str) -> bool {
    model_status(root, id, crate::model_verification::Check::Blocking).ready()
}

#[cfg(any(target_os = "macos", test))]
fn production_sectioner(id: &str) -> &'static str {
    if crate::llama_asr::is_metal(id) {
        if crate::qwen_window::target_seconds() == 0 { "energy" } else { "qwen_windows" }
    } else { "baseline" }
}

pub(crate) fn model_status(root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Status::Missing;
    }
    let Some(model) = find(id) else {
        return Status::Missing;
    };
    check.file(root, &model_dir(root, id).join(model.file), model.bytes, model.sha256)
}

#[cfg(any(windows, target_os = "macos", test))]
mod audio {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub const RATE: usize = 16_000;
    pub const MAX_SAMPLES: usize = 20 * RATE;
    pub const NATIVE_PAD_SAMPLES: usize = RATE / 5;
    const MIN_SAMPLES: usize = 3 * RATE;
    const QUIET_SAMPLES: usize = 3 * RATE / 5;
    const FRAME_SAMPLES: usize = RATE / 100;
    const NOISE_FRAMES: usize = 800;
    const SEARCH_FRAMES: usize = 800;
    const WINDOW_FRAMES: usize = 20;

    /// Linear interpolation with a rational phase shared by every callback.
    /// Only the previous input sample is retained. A final fractional sample
    /// interval is held to its endpoint, adding at most one 16 kHz frame.
    #[derive(Default)]
    pub struct Resampler {
        rate: Option<u32>,
        input_samples: u64,
        next_position: u64,
        previous: f32,
    }

    impl Resampler {
        pub fn add(
            &mut self,
            samples: &[f32],
            rate: u32,
            mut emit: impl FnMut(f32) -> Result<(), String>,
        ) -> Result<(), String> {
            if !(1..=384_000).contains(&rate) {
                return Err("Invalid input sample rate.".into());
            }
            if self.rate.is_some_and(|previous| previous != rate) {
                return Err("The input sample rate changed during recognition.".into());
            }
            if samples.is_empty() {
                return Ok(());
            }
            self.rate = Some(rate);
            for &sample in samples {
                if !sample.is_finite() || sample.abs() > 1.0 {
                    return Err("Invalid normalized audio.".into());
                }
                let position = self
                    .input_samples
                    .checked_mul(RATE as u64)
                    .ok_or("Recognition audio duration overflowed.")?;
                while self.next_position <= position {
                    let value = if self.input_samples == 0 {
                        sample
                    } else {
                        let previous_position = position - RATE as u64;
                        let fraction =
                            (self.next_position - previous_position) as f32 / RATE as f32;
                        self.previous + (sample - self.previous) * fraction
                    };
                    emit(value)?;
                    self.next_position = self
                        .next_position
                        .checked_add(u64::from(rate))
                        .ok_or("Recognition audio duration overflowed.")?;
                }
                self.previous = sample;
                self.input_samples += 1;
            }
            Ok(())
        }

        pub fn finish(
            &mut self,
            mut emit: impl FnMut(f32) -> Result<(), String>,
        ) -> Result<(), String> {
            let Some(rate) = self.rate else {
                return Ok(());
            };
            let end = self
                .input_samples
                .checked_mul(RATE as u64)
                .ok_or("Recognition audio duration overflowed.")?;
            while self.next_position < end {
                emit(self.previous)?;
                self.next_position = self
                    .next_position
                    .checked_add(u64::from(rate))
                    .ok_or("Recognition audio duration overflowed.")?;
            }
            Ok(())
        }
    }

    pub struct AudioBuffer {
        pub samples: Vec<f32>,
        max_samples: usize,
        quiet_samples: usize,
        frame_power: f64,
        frame_peak: f32,
        powers: Vec<f64>,
        noise: VecDeque<f64>,
        quiet_power: f64,
        boundary: Option<usize>,
        forced: bool,
        legacy: bool,
    }

    impl Default for AudioBuffer {
        fn default() -> Self {
            #[cfg(feature = "fidelity-eval")]
            let max_samples = if std::env::var("SONOJOT_EVAL_MAX_CHUNK_SECONDS").as_deref() == Ok("28") {
                28 * RATE
            } else {
                MAX_SAMPLES
            };
            #[cfg(not(feature = "fidelity-eval"))]
            let max_samples = MAX_SAMPLES;
            Self::with_max_samples(max_samples)
        }
    }

    impl AudioBuffer {
        pub(super) fn with_max_samples(max_samples: usize) -> Self {
            Self {
                samples: Vec::with_capacity(max_samples + NATIVE_PAD_SAMPLES),
                max_samples,
                quiet_samples: 0,
                frame_power: 0.0,
                frame_peak: 0.0,
                powers: Vec::with_capacity(max_samples / FRAME_SAMPLES),
                noise: VecDeque::with_capacity(NOISE_FRAMES),
                quiet_power: 0.003_f64.powi(2),
                boundary: None,
                forced: false,
                legacy: false,
            }
        }
        pub fn max_samples(&self) -> usize {
            self.max_samples
        }

        pub fn push(&mut self, sample: f32) -> bool {
            self.push_with_pause(sample, None)
        }

        // A VAD decision replaces only pause detection. The same bounded buffer,
        // fallback cut and sample ownership serve both decoders and candidates.
        pub fn push_with_pause(&mut self, sample: f32, vad_pause: Option<bool>) -> bool {
            self.samples.push(sample);
            self.frame_power += f64::from(sample).powi(2);
            self.frame_peak = self.frame_peak.max(sample.abs());
            if self.samples.len() % FRAME_SAMPLES == 0 {
                let power = self.frame_power / FRAME_SAMPLES as f64;
                self.powers.push(power);
                self.noise.push_back(power);
                if self.noise.len() > NOISE_FRAMES {
                    self.noise.pop_front();
                }
                if self.powers.len() % 10 == 0 {
                    let mut sorted: Vec<_> = self.noise.iter().copied().collect();
                    sorted.sort_unstable_by(f64::total_cmp);
                    let low = sorted[(sorted.len() - 1) / 20];
                    let high = sorted[(sorted.len() - 1) * 4 / 5];
                    // The upper-quantile guard keeps sustained speech from
                    // becoming its own noise floor. Neither threshold removes
                    // audio, and very quiet input retains the absolute floor.
                    self.quiet_power = (low * 4.0).min(high * 0.1).max(0.003_f64.powi(2));
                }
                let quiet = if self.legacy {
                    power <= 0.003_f64.powi(2) && self.frame_peak <= 0.015
                } else {
                    power <= self.quiet_power
                };
                if quiet {
                    self.quiet_samples += FRAME_SAMPLES;
                } else {
                    self.quiet_samples = 0;
                }
                self.frame_power = 0.0;
                self.frame_peak = 0.0;
            }
            let pause = vad_pause.unwrap_or(self.quiet_samples >= QUIET_SAMPLES);
            if self.samples.len() >= MIN_SAMPLES && self.samples.len() % FRAME_SAMPLES == 0 && pause {
                self.boundary = Some(self.samples.len());
                self.forced = false;
            } else if self.samples.len() >= self.max_samples {
                self.boundary = Some(if self.legacy { self.max_samples } else { self.quietest_cut() });
                self.forced = true;
            }
            self.boundary.is_some()
        }

        pub fn forced_boundary(&self) -> bool {
            self.forced
        }

        fn quietest_cut(&self) -> usize {
            let end = self.powers.len();
            let start = end.saturating_sub(SEARCH_FRAMES).max(MIN_SAMPLES / FRAME_SAMPLES);
            if end < start + WINDOW_FRAMES {
                return self.max_samples;
            }
            let mut sum: f64 = self.powers[start..start + WINDOW_FRAMES].iter().sum();
            let mut best = sum;
            let mut best_start = start;
            let mut highest = sum;
            for at in start + 1..=end - WINDOW_FRAMES {
                sum += self.powers[at + WINDOW_FRAMES - 1] - self.powers[at - 1];
                highest = highest.max(sum);
                if sum <= best {
                    best = sum;
                    best_start = at;
                }
            }
            // A flat signal supplies no quieter location. Avoid shortening
            // every section by 100 ms solely because of a numerical tie.
            if best >= highest * 0.999 {
                self.max_samples
            } else {
                (best_start + WINDOW_FRAMES / 2) * FRAME_SAMPLES
            }
        }

        pub fn section_len(&self) -> usize {
            self.boundary.unwrap_or(self.samples.len())
        }

        pub fn boundary_kind(&self) -> &'static str {
            if self.forced { "maximum" } else if self.boundary.is_some() { "pause" } else { "finish" }
        }

        pub fn set_legacy(&mut self, legacy: bool) {
            self.legacy = legacy;
        }

        pub fn consume(&mut self, count: usize) {
            self.samples.drain(..count);
            self.powers.drain(..count / FRAME_SAMPLES);
            self.boundary = None;
            self.forced = false;
            // A maximum cut can leave several seconds already analyzed. Keep
            // their powers and trailing quiet count, without feeding them twice
            // into the noise estimator or the VAD's recurrent state.
            self.quiet_samples = self.powers.iter().rev()
                .take_while(|&&power| power <= self.quiet_power).count() * FRAME_SAMPLES;
            // Normal boundaries and maximum cuts are frame-aligned; finish may
            // consume a partial frame and always empties the remaining buffer.
            if self.samples.is_empty() {
                self.frame_power = 0.0;
                self.frame_peak = 0.0;
            }
        }

        #[cfg(test)]
        pub fn clear(&mut self) {
            self.consume(self.section_len());
            self.quiet_samples = 0;
        }
    }

    pub fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), String> {
        if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            Err("Transcription cancelled.".into())
        } else {
            Ok(())
        }
    }

    /// Keep the existing digital-silence guard, including evaluation bypasses.
    pub fn digital_silence(samples: &[f32]) -> bool {
        samples.iter().all(|sample| sample.abs() <= 0.000_001)
    }
}

#[cfg(any(windows, target_os = "macos", test))]
pub(crate) mod speech_gate {
    use super::audio::{check_cancel, digital_silence};
    use std::sync::{atomic::AtomicBool, OnceLock};

    // Presence is deliberately shorter than the sectioner's 250 ms minimum:
    // brief replies can qualify without waiting for a whole boundary event.
    pub const MIN_POSITIVE_SAMPLES: usize = 1_536; // 96 ms at 16 kHz
    const FFT_SIZE: usize = 512;
    const MIN_RMS: f64 = 0.001;
    const MAX_FLATNESS: f64 = 0.35;
    const MIN_BAND_FRACTION: f64 = 0.15;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Mode { Silero, Fallback, Confirmation, Off, NotApplied }

    impl Mode {
        pub fn for_model(id: &str) -> Self {
            let qwen = crate::llama_asr::is_metal(id);
            #[cfg(feature = "fidelity-eval")]
            let selection = std::env::var(if qwen {
                "SONOJOT_EVAL_QWEN_GATE"
            } else {
                "SONOJOT_EVAL_WHISPER_GATE"
            }).ok();
            #[cfg(not(feature = "fidelity-eval"))]
            let selection: Option<String> = None;
            Self::select(qwen, selection.as_deref())
        }

        fn select(qwen: bool, _selection: Option<&str>) -> Self {
            #[cfg(feature = "fidelity-eval")]
            match _selection {
                Some("off") => return Self::Off,
                Some("fallback") => return Self::Fallback,
                Some("silero" | "on") => return Self::Silero,
                _ => {}
            }
            if qwen { Self::Silero } else { Self::NotApplied }
        }
    }

    #[derive(Default, Debug, serde::Serialize)]
    pub struct Probabilities {
        pub frames: usize,
        pub samples: usize,
        pub positive_samples: usize,
        pub longest_positive_run_samples: usize,
        pub min_probability: Option<f32>,
        pub max_probability: Option<f32>,
        pub mean_probability: f64,
        #[serde(skip)]
        run: usize,
    }

    impl Probabilities {
        pub fn observe(&mut self, probability: f32, samples: usize) -> Result<(), String> {
            if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                return Err("Silero returned an invalid speech probability.".into());
            }
            self.frames += 1;
            self.samples += samples;
            self.mean_probability += (f64::from(probability) - self.mean_probability)
                * samples as f64 / self.samples as f64;
            self.min_probability = Some(self.min_probability.unwrap_or(probability).min(probability));
            self.max_probability = Some(self.max_probability.unwrap_or(probability).max(probability));
            if probability >= crate::qwen_window::vad_threshold() {
                self.positive_samples += samples;
                self.run += samples;
                self.longest_positive_run_samples = self.longest_positive_run_samples.max(self.run);
            } else {
                self.run = 0;
            }
            Ok(())
        }

        pub fn speech(&self) -> bool {
            self.longest_positive_run_samples >= MIN_POSITIVE_SAMPLES
        }
    }

    #[derive(Default, Debug, serde::Serialize)]
    pub struct Fallback {
        pub frames: usize,
        pub energetic_frames: usize,
        pub positive_samples: usize,
        pub longest_positive_run_samples: usize,
        pub max_rms: f64,
        pub min_flatness: Option<f64>,
        pub mean_flatness: Option<f64>,
        pub max_band_fraction: f64,
        pub min_preemphasized_flatness: Option<f64>,
        pub max_preemphasized_band_fraction: f64,
    }

    #[derive(Debug, serde::Serialize)]
    pub struct Decision {
        pub requested: Mode,
        pub method: &'static str,
        pub decode: bool,
        pub reason: &'static str,
        pub probabilities: Option<Probabilities>,
        pub fallback: Option<Fallback>,
        pub failure: Option<String>,
        pub probability_threshold: f32,
        pub min_positive_samples: usize,
        pub fallback_min_rms: f64,
        pub fallback_max_flatness: f64,
        pub fallback_min_band_fraction: f64,
    }

    pub fn decide(
        mode: Mode,
        samples: &[f32],
        cancel: Option<&AtomicBool>,
        silero: impl FnOnce() -> Result<Probabilities, String>,
    ) -> Result<Decision, String> {
        check_cancel(cancel)?;
        let mut decision = Decision {
            requested: mode, method: "digital_silence", decode: false, reason: "digital_silence",
            probabilities: None, fallback: None, failure: None,
            probability_threshold: crate::qwen_window::vad_threshold(),
            // Silero already required 250 ms of speech for these Windows
            // sections. Two structured frames confirm it without losing brief
            // noisy words; standalone fallback still requires three frames.
            min_positive_samples: if mode == Mode::Confirmation { 1_024 } else { MIN_POSITIVE_SAMPLES },
            fallback_min_rms: MIN_RMS, fallback_max_flatness: MAX_FLATNESS,
            fallback_min_band_fraction: MIN_BAND_FRACTION,
        };
        if digital_silence(samples) {
            return Ok(decision);
        }
        if matches!(mode, Mode::Off | Mode::NotApplied) {
            decision.method = if mode == Mode::Off { "evaluation_bypass" } else { "not_applied" };
            decision.reason = decision.method;
            decision.decode = true;
            return Ok(decision);
        }
        if mode == Mode::Silero {
            let result = silero();
            // Cancellation is never converted into a fallback authorization.
            check_cancel(cancel)?;
            match result {
                Ok(probabilities) => {
                    decision.decode = probabilities.speech();
                    decision.method = "silero";
                    decision.reason = if decision.decode { "speech_present" } else { "no_qualified_speech" };
                    decision.probabilities = Some(probabilities);
                    return Ok(decision);
                }
                Err(error) => decision.failure = Some(error),
            }
        }
        let fallback = fallback(samples, cancel)?;
        decision.decode = fallback.longest_positive_run_samples >= decision.min_positive_samples;
        decision.method = "energy_spectral_flatness";
        decision.reason = if decision.decode { "speech_like_energy" } else { "no_speech_like_energy" };
        decision.fallback = Some(fallback);
        Ok(decision)
    }

    /// Locate speech evidence in an untriggered final tail. At least 100 ms of
    /// consecutive real samples must qualify; padding cannot supply evidence.
    pub(crate) fn confirmed_tail(
        samples: &[f32], cancel: Option<&AtomicBool>,
    ) -> Result<Option<std::ops::Range<usize>>, String> {
        let mut run = 0;
        let mut start = None;
        let mut end = 0;
        for (index, frame) in samples.chunks(FFT_SIZE).enumerate() {
            let at = index * FFT_SIZE;
            // The final partial frame may use preceding real context for its
            // spectrum, but counts only its own samples toward qualification.
            let analysis = if frame.len() < FFT_SIZE && at + frame.len() >= FFT_SIZE {
                &samples[at + frame.len() - FFT_SIZE..at + frame.len()]
            } else { frame };
            let positive = fallback(analysis, cancel)?.positive_samples > 0;
            if positive {
                run += frame.len();
                if run >= 1_600 {
                    start.get_or_insert(at + frame.len() - run);
                    end = at + frame.len();
                }
            } else { run = 0; }
        }
        Ok(start.map(|start| start..end))
    }

    /// Denoising can turn stationary noise into structured spectral output.
    /// Never recover an untriggered tail from that evidence alone.
    pub(crate) fn confirmed_tail_with_original(
        samples: &[f32], original: Option<&[f32]>, cancel: Option<&AtomicBool>,
    ) -> Result<Option<std::ops::Range<usize>>, String> {
        if original.is_some_and(|source| source.len() != samples.len()) {
            return Err("Final-tail source and gate samples are misaligned.".into());
        }
        let Some(range) = confirmed_tail(samples, cancel)? else { return Ok(None); };
        if let Some(source) = original {
            if confirmed_tail(&source[range.clone()], cancel)?.is_none() { return Ok(None); }
        }
        Ok(Some(range))
    }

    pub(crate) fn fallback(samples: &[f32], cancel: Option<&AtomicBool>) -> Result<Fallback, String> {
        let mut result = Fallback::default();
        let mut run = 0;
        for frame in samples.chunks(FFT_SIZE) {
            check_cancel(cancel)?;
            if frame.iter().any(|sample| !sample.is_finite()) {
                return Err("Speech gate received invalid audio.".into());
            }
            result.frames += 1;
            let mean = frame.iter().map(|&x| f64::from(x)).sum::<f64>() / frame.len() as f64;
            let rms = (frame.iter().map(|&x| (f64::from(x) - mean).powi(2)).sum::<f64>()
                / frame.len() as f64).sqrt();
            result.max_rms = result.max_rms.max(rms);
            // Very short zero-padded tails cannot establish spectral structure.
            let positive = if rms >= MIN_RMS && frame.len() >= FFT_SIZE / 2 {
                let (flatness, band_fraction) = spectrum(frame, mean, false);
                // Differencing flattens the spectral slope of pink/brown and
                // fan noise. Require structure in both views: differencing
                // alone can make white noise look artificially non-flat.
                let (emphasized_flatness, emphasized_band) = spectrum(frame, mean, true);
                result.energetic_frames += 1;
                result.min_flatness = Some(result.min_flatness.unwrap_or(flatness).min(flatness));
                let average = result.mean_flatness.get_or_insert(0.0);
                *average += (flatness - *average) / result.energetic_frames as f64;
                result.max_band_fraction = result.max_band_fraction.max(band_fraction);
                result.min_preemphasized_flatness = Some(result.min_preemphasized_flatness
                    .unwrap_or(emphasized_flatness).min(emphasized_flatness));
                result.max_preemphasized_band_fraction = result.max_preemphasized_band_fraction.max(emphasized_band);
                flatness <= MAX_FLATNESS && band_fraction >= MIN_BAND_FRACTION
                    && emphasized_flatness <= MAX_FLATNESS && emphasized_band >= MIN_BAND_FRACTION
            } else {
                false
            };
            if positive {
                result.positive_samples += frame.len();
                run += frame.len();
                result.longest_positive_run_samples = result.longest_positive_run_samples.max(run);
            } else {
                run = 0;
            }
        }
        check_cancel(cancel)?;
        Ok(result)
    }

    // Fixed 512-point radix-2 FFT: O(N log N), two stack arrays, no dependency
    // or recording-length allocation. Only the analysis copy is windowed.
    fn spectrum(frame: &[f32], mean: f64, preemphasis: bool) -> (f64, f64) {
        use std::f64::consts::TAU;
        static HANN: OnceLock<[f64; FFT_SIZE]> = OnceLock::new();
        let window = HANN.get_or_init(|| std::array::from_fn(|i| {
            0.5 - 0.5 * (TAU * i as f64 / (FFT_SIZE - 1) as f64).cos()
        }));
        let mut re = [0.0; FFT_SIZE];
        let mut im = [0.0; FFT_SIZE];
        for (i, &sample) in frame.iter().enumerate() {
            let reverse = i.reverse_bits() >> (usize::BITS - FFT_SIZE.ilog2());
            let centered = f64::from(sample) - mean;
            let value = if preemphasis {
                if i == 0 { 0.0 } else { centered - 0.97 * (f64::from(frame[i - 1]) - mean) }
            } else { centered };
            re[reverse] = value * window[i];
        }
        let mut width = 2;
        while width <= FFT_SIZE {
            let (sin, cos) = (-TAU / width as f64).sin_cos();
            for start in (0..FFT_SIZE).step_by(width) {
                let (mut wr, mut wi) = (1.0, 0.0);
                for offset in 0..width / 2 {
                    let left = start + offset;
                    let right = left + width / 2;
                    let tr = wr * re[right] - wi * im[right];
                    let ti = wr * im[right] + wi * re[right];
                    re[right] = re[left] - tr;
                    im[right] = im[left] - ti;
                    re[left] += tr;
                    im[left] += ti;
                    (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
                }
            }
            width *= 2;
        }
        let mut total = 0.0;
        let mut band = 0.0;
        let mut logs = 0.0;
        let first_bin = if preemphasis { 8 } else { 4 };
        let bins = (129 - first_bin) as f64;
        // 125--4000 Hz avoids DC/hum and out-of-band noise. A flat spectrum
        // approaches 1; harmonic speech approaches 0. This is a conservative
        // fallback, not a substitute for a trained speech classifier.
        for bin in 1..=FFT_SIZE / 2 {
            let power = re[bin].powi(2) + im[bin].powi(2);
            total += power;
            if (first_bin..=128).contains(&bin) {
                band += power;
                logs += power.max(1e-20).ln();
            }
        }
        let flatness = ((logs / bins).exp() / (band / bins).max(1e-20)).clamp(0.0, 1.0);
        (flatness, band / total.max(1e-20))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::Ordering;

        fn harmonics(count: usize, amplitude: f32) -> Vec<f32> {
            (0..count).map(|i| {
                let phase = std::f32::consts::TAU * 187.5 * i as f32 / 16_000.0;
                amplitude * (phase.sin() + 0.5 * (3.0 * phase).sin() + 0.25 * (5.0 * phase).sin())
            }).collect()
        }

        #[test]
        fn final_tail_requires_one_hundred_ms_of_real_confirmation_audio() {
            for count in [0, 512, 1_024, 1_536, 1_599] {
                assert!(confirmed_tail(&harmonics(count, 0.1), None).unwrap().is_none(), "{count}");
            }
            for count in [1_600, 2_048, 3_900] {
                assert_eq!(confirmed_tail(&harmonics(count, 0.1), None).unwrap(), Some(0..count));
            }
            assert!(confirmed_tail(&vec![0.0; 32_000], None).unwrap().is_none());
        }

        #[test]
        fn filtered_tail_cannot_invent_speech_absent_from_original_audio() {
            let filtered = harmonics(4_000, 0.1);
            let original_noise = noise(4_000);
            assert!(confirmed_tail(&filtered, None).unwrap().is_some());
            assert!(confirmed_tail_with_original(&filtered, Some(&original_noise), None).unwrap().is_none());
            assert_eq!(confirmed_tail_with_original(&filtered, Some(&filtered), None).unwrap(), Some(0..4_000));
            assert!(confirmed_tail_with_original(&filtered, Some(&filtered[..3_999]), None).is_err());
            assert!(confirmed_tail_with_original(&filtered, None, None).unwrap().is_some());
        }

        #[test]
        fn final_tail_recovery_keeps_a_short_word_after_a_pause_at_each_stop_offset() {
            for after_ms in [0, 30, 60] {
                let mut samples = vec![0.0; 16_384];
                samples.extend(harmonics(2_048, 0.1));
                samples.extend(vec![0.0; after_ms * 16]);
                let speech = confirmed_tail(&samples, None).unwrap().unwrap();
                assert_eq!(speech.start, 16_384);
                assert!(speech.end >= 18_432);
                assert!(speech.end <= samples.len());
                let mut windows = crate::qwen_window::Assembler::new(8);
                windows.append(&samples, &samples).unwrap();
                windows.segment(0..8_000, false).unwrap();
                windows.segment(speech.start as u64..speech.end as u64, true).unwrap();
                let result = windows.poll(true).unwrap();
                assert_eq!(result.len(), 1);
                assert!(result[0].recovered_tail);
                assert_eq!(result[0].end, samples.len() as u64);
            }
        }

        fn noise(count: usize) -> Vec<f32> {
            let mut seed = 0x1234_5678_u32;
            (0..count).map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f64 / u32::MAX as f64 * 0.2 - 0.1) as f32
            }).collect()
        }

        #[test]
        fn fallback_rejects_silence_dc_broadband_noise_and_isolated_clicks() {
            let mut click = vec![0.0; 16_000];
            click[230] = 1.0;
            for samples in [vec![0.0; 16_000], vec![0.2; 16_000], noise(16_000), click] {
                let decision = decide(Mode::Fallback, &samples, None, || panic!("no Silero")).unwrap();
                assert!(!decision.decode, "{decision:?}");
            }
        }

        #[test]
        fn fallback_rejects_colored_and_fan_like_noise() {
            let white = noise(16_000 * 3);
            let mut state = 0.0;
            let brown: Vec<f32> = white.iter().map(|sample| {
                state = 0.98 * state + sample;
                state
            }).collect();
            let fan: Vec<f32> = brown.iter().enumerate().map(|(i, sample)| {
                let phase = std::f32::consts::TAU * 60.0 * i as f32 / 16_000.0;
                0.7 * sample + 0.025 * phase.sin() + 0.012 * (phase * 2.0).sin()
            }).collect();
            for samples in [brown, fan] {
                let decision = decide(Mode::Fallback, &samples, None, || panic!("no Silero")).unwrap();
                assert!(!decision.decode, "{decision:?}");
            }
        }

        #[test]
        fn silero_confirmation_accepts_two_structured_frames_but_not_isolated_frames() {
            assert!(decide(Mode::Confirmation, &harmonics(1_024, 0.1), None, || panic!("already gated")).unwrap().decode);
            assert!(!decide(Mode::Confirmation, &harmonics(512, 0.1), None, || panic!("already gated")).unwrap().decode);
            assert!(!decide(Mode::Fallback, &harmonics(1_024, 0.1), None, || panic!("standalone fallback")).unwrap().decode);
        }

        #[test]
        fn fallback_accepts_quiet_harmonics_but_requires_sustained_energy_in_band() {
            let samples = harmonics(16_000, 0.003);
            let original = samples.clone();
            let decision = decide(Mode::Fallback, &samples, None, || panic!("no Silero")).unwrap();
            assert!(decision.decode, "{decision:?}");
            assert_eq!(samples, original);
            assert!(decision.fallback.unwrap().min_flatness.unwrap() < MAX_FLATNESS);
            let high = (0..16_000).map(|i| if i % 2 == 0 { 0.2 } else { -0.2 }).collect::<Vec<_>>();
            for rejected in [harmonics(16_000, 0.0001), harmonics(1_000, 0.1), high] {
                assert!(!decide(Mode::Fallback, &rejected, None, || panic!("no Silero")).unwrap().decode);
            }
        }

        #[test]
        fn fallback_never_carries_speech_from_another_section() {
            let mut samples = noise(4_096);
            samples.extend(harmonics(4_096, 0.1));
            assert!(fallback(&samples[..4_096], None).unwrap().longest_positive_run_samples < MIN_POSITIVE_SAMPLES);
            assert!(fallback(&samples[4_096..], None).unwrap().longest_positive_run_samples >= MIN_POSITIVE_SAMPLES);
            assert!(fallback(&samples[..4_096], None).unwrap().longest_positive_run_samples < MIN_POSITIVE_SAMPLES);
        }

        #[test]
        fn silero_failure_always_uses_fallback_and_negative_silero_is_authoritative() {
            let samples = noise(16_000);
            let decision = decide(Mode::Silero, &samples, None, || Err("broken runtime".into())).unwrap();
            assert_eq!(decision.method, "energy_spectral_flatness");
            assert_eq!(decision.failure.as_deref(), Some("broken runtime"));
            assert!(!decision.decode);
            assert!(decision.fallback.is_some());
            let samples = harmonics(16_000, 0.2);
            let decision = decide(Mode::Silero, &samples, None, || {
                let mut probabilities = Probabilities::default();
                probabilities.observe(0.05, samples.len())?;
                Ok(probabilities)
            }).unwrap();
            assert!(!decision.decode);
            assert_eq!(decision.method, "silero");
            assert!(decision.fallback.is_none());
            assert!(decide(Mode::Silero, &samples, None, || Err("inference failed".into())).unwrap().decode);
        }

        #[test]
        fn probability_qualification_requires_contiguous_real_samples() {
            let mut p = Probabilities::default();
            for _ in 0..8 {
                p.observe(0.8, 512).unwrap();
                p.observe(0.0, 512).unwrap();
            }
            assert!(!p.speech());
            assert_eq!(p.positive_samples, 4_096);
            let threshold = crate::qwen_window::vad_threshold();
            p.observe(threshold, MIN_POSITIVE_SAMPLES - 1).unwrap();
            assert!(!p.speech());
            p.observe(threshold, 1).unwrap();
            assert!(p.speech());
            assert_eq!(p.min_probability, Some(0.0));
            assert_eq!(p.max_probability, Some(0.8));
            let expected = (0.8_f32 as f64 * 4_096.0 + f64::from(threshold) * MIN_POSITIVE_SAMPLES as f64)
                / (8_192 + MIN_POSITIVE_SAMPLES) as f64;
            assert!((p.mean_probability - expected).abs() < 1e-10);
            assert!(p.observe(f32::NAN, 512).is_err());
        }

        #[test]
        fn bypass_and_whisper_preserve_the_original_digital_silence_guard() {
            for mode in [Mode::Off, Mode::NotApplied] {
                assert!(decide(mode, &noise(2_000), None, || panic!("bypass")).unwrap().decode);
                assert!(!decide(mode, &[0.0; 512], None, || panic!("silence")).unwrap().decode);
            }
        }

        #[test]
        fn gate_selection_limits_overrides_to_evaluation_builds() {
            assert_eq!(Mode::select(true, None), Mode::Silero);
            assert_eq!(Mode::select(false, None), Mode::NotApplied);
            for (selection, expected) in [("off", Mode::Off), ("fallback", Mode::Fallback), ("on", Mode::Silero)] {
                if cfg!(feature = "fidelity-eval") {
                    assert_eq!(Mode::select(true, Some(selection)), expected);
                    assert_eq!(Mode::select(false, Some(selection)), expected);
                } else {
                    assert_eq!(Mode::select(true, Some(selection)), Mode::Silero);
                    assert_eq!(Mode::select(false, Some(selection)), Mode::NotApplied);
                }
            }
            assert_eq!(Mode::select(true, Some("unknown")), Mode::Silero);
        }

        #[test]
        fn cancellation_is_not_converted_to_fallback_or_a_bypass() {
            let samples = harmonics(16_000, 0.2);
            let cancel = AtomicBool::new(true);
            for mode in [Mode::Silero, Mode::Fallback, Mode::Off, Mode::NotApplied] {
                assert!(decide(mode, &samples, Some(&cancel), || panic!("cancelled")).is_err());
            }
            cancel.store(false, Ordering::Release);
            let result = decide(Mode::Silero, &samples, Some(&cancel), || {
                cancel.store(true, Ordering::Release);
                Err("inference interrupted".into())
            });
            assert_eq!(result.unwrap_err(), "Transcription cancelled.");
            assert!(fallback(&samples, Some(&cancel)).is_err());
        }

        #[test]
        fn fft_flatness_and_band_energy_match_a_direct_dft() {
            let frame = noise(FFT_SIZE);
            let mean = frame.iter().map(|&v| f64::from(v)).sum::<f64>() / FFT_SIZE as f64;
            let (actual_flatness, actual_fraction) = spectrum(&frame, mean, false);
            let (mut total, mut band, mut logs) = (0.0, 0.0, 0.0);
            for bin in 1..=FFT_SIZE / 2 {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, &value) in frame.iter().enumerate() {
                    let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (FFT_SIZE - 1) as f64).cos();
                    let angle = std::f64::consts::TAU * bin as f64 * i as f64 / FFT_SIZE as f64;
                    re += (f64::from(value) - mean) * window * angle.cos();
                    im -= (f64::from(value) - mean) * window * angle.sin();
                }
                let power = re * re + im * im;
                total += power;
                if (4..=128).contains(&bin) {
                    band += power;
                    logs += power.ln();
                }
            }
            assert!((actual_flatness - (logs / 125.0).exp() / (band / 125.0)).abs() < 1e-10);
            assert!((actual_fraction - band / total).abs() < 1e-10);
        }
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::{audio::*, *};
    use crate::asr::{RecognitionResult, Segment};
    use std::{
        collections::BTreeMap,
        ffi::{c_char, c_uint, c_void, CStr},
        io::Write,
        sync::atomic::{AtomicBool, Ordering},
    };
    use whisper_rs::{
        FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    };

    static METAL_SELECTED: AtomicBool = AtomicBool::new(false);
    static METAL_FAILED: AtomicBool = AtomicBool::new(false);

    // whisper.cpp can return a valid CPU context after GPU initialization
    // fails. The pinned backend emits these messages around device init; a
    // successful state plus selection and no failure verifies initialization.
    // This is deliberately not GPU execution/offload evidence.
    unsafe extern "C" fn native_log(_: c_uint, message: *const c_char, _: *mut c_void) {
        if message.is_null() {
            return;
        }
        let bytes = unsafe { CStr::from_ptr(message) }.to_bytes();
        record_initialization(bytes);
        let _ = std::io::stderr().lock().write_all(bytes);
    }

    fn record_initialization(bytes: &[u8]) {
        if contains(bytes, b"whisper_backend_init_gpu: using Metal backend") {
            METAL_SELECTED.store(true, Ordering::Release);
        }
        if contains(
            bytes,
            b"whisper_backend_init_gpu: failed to initialize Metal backend",
        ) {
            METAL_FAILED.store(true, Ordering::Release);
        }
    }

    fn contains(bytes: &[u8], needle: &[u8]) -> bool {
        bytes.windows(needle.len()).any(|part| part == needle)
    }

    pub struct Engine {
        // WhisperState retains its context through Arc; one state is reused
        // between dictations, then both are released with the owned worker.
        decoder: Decoder,
        vad: Option<vad::Silero>,
        sectioner: &'static str,
        vad_fallback: Option<String>,
        speech_gate: speech_gate::Mode,
        pub model_id: String,
        pub requested_model: String,
    }

    enum Decoder {
        Whisper(WhisperState),
        Llama(crate::llama_asr::Server),
    }

    impl Engine {
        pub fn load(root: &Path, id: &str) -> Result<Self, String> {
            if !crate::recognition::is_worker() {
                return Err("Metal recognition requires an isolated worker.".into());
            }
            if !cfg!(target_arch = "aarch64") {
                return Err("These Metal speech models require an Apple Silicon Mac.".into());
            }
            // Qwen always has a speech presence gate in production. Boundary
            // selection is independent; Whisper keeps its original behavior.
            // This worker never installs or downloads missing optional assets.
            #[cfg(feature = "fidelity-eval")]
            let requested = std::env::var("SONOJOT_EVAL_MAC_SECTIONER")
                .unwrap_or_else(|_| production_sectioner(id).into());
            #[cfg(not(feature = "fidelity-eval"))]
            let requested = production_sectioner(id);
            let speech_gate = speech_gate::Mode::for_model(id);
            let sectioner = match &*requested {
                "silero" => "silero",
                "baseline" => "baseline",
                "qwen_windows" => "qwen_windows",
                _ => "energy",
            };
            let (vad, sectioner, vad_fallback) = if sectioner == "silero"
                || speech_gate == speech_gate::Mode::Silero
            {
                match vad::Silero::load(root) {
                    Ok(vad) => (Some(vad), sectioner, None),
                    Err(error) => (None, if sectioner == "silero" { "energy" } else { sectioner }, Some(error)),
                }
            } else {
                (None, sectioner, None)
            };
            if crate::llama_asr::is_metal(id) {
                return Ok(Self {
                    decoder: Decoder::Llama(crate::llama_asr::Server::launch(root, id)?),
                    vad,
                    sectioner,
                    vad_fallback,
                    speech_gate,
                    model_id: id.into(),
                    requested_model: id.into(),
                });
            }
            let model = find(id).ok_or_else(|| format!("Unsupported Metal recognizer: {id}"))?;
            if !model_ready(root, id) {
                return Err(format!(
                    "{} is not installed or failed verification.",
                    model.name
                ));
            }
            METAL_SELECTED.store(false, Ordering::Release);
            METAL_FAILED.store(false, Ordering::Release);
            // This worker owns the only Whisper engine and callback. It uses
            // static atomics, has no borrowed userdata and never unwinds.
            unsafe { whisper_rs::set_log_callback(Some(native_log), std::ptr::null_mut()) };
            let mut parameters = WhisperContextParameters::default();
            parameters.use_gpu(true).flash_attn(true);
            let context =
                WhisperContext::new_with_params(model_dir(root, id).join(model.file), parameters)
                    .map_err(|error| format!("Cannot load {}: {error}", model.name))?;
            let state = context
                .create_state()
                .map_err(|error| format!("Cannot initialize {}: {error}", model.name))?;
            if !METAL_SELECTED.load(Ordering::Acquire) || METAL_FAILED.load(Ordering::Acquire) {
                return Err("Whisper could not initialize Metal acceleration on this Mac.".into());
            }
            Ok(Self {
                decoder: Decoder::Whisper(state),
                vad,
                sectioner,
                vad_fallback,
                speech_gate,
                model_id: id.into(),
                requested_model: id.into(),
            })
        }

        pub fn stream<'a>(&'a mut self, cancel: Option<&'a AtomicBool>) -> Stream<'a> {
            let mut pending = AudioBuffer::default();
            pending.set_legacy(self.sectioner == "baseline");
            if let Some(vad) = &mut self.vad {
                vad.reset();
            }
            let qwen = (matches!(&self.decoder, Decoder::Llama(_))
                && crate::qwen_window::target_seconds() != 0).then(QwenStream::new);
            Stream {
                engine: self,
                cancel,
                resampler: Resampler::default(),
                pending,
                qwen,
                output: Output::default(),
            }
        }

        fn fail_vad(&mut self, error: String) {
            self.vad = None;
            if self.sectioner == "silero" {
                self.sectioner = "energy";
            }
            self.vad_fallback = Some(error);
        }

        fn gate_section(
            &mut self,
            samples: &[f32],
            cancel: Option<&AtomicBool>,
        ) -> Result<speech_gate::Decision, String> {
            speech_gate::decide(self.speech_gate, samples, cancel, || {
                let result = match &mut self.vad {
                    Some(vad) => vad.analyze_section(samples, cancel),
                    None => return Err(self.vad_fallback.clone()
                        .unwrap_or_else(|| "Silero is unavailable.".into())),
                };
                check_cancel(cancel)?;
                if let Err(error) = &result {
                    // An inference failure disables Silero for this worker.
                    // The current and every later section still run a gate.
                    self.fail_vad(error.clone());
                }
                result
            })
        }
    }

    #[derive(Default)]
    struct Output {
        ledger: BTreeMap<u64, Segment>,
        original_text: Option<String>,
        samples_consumed: u64,
        warnings: Vec<String>,
        incomplete: bool,
        forced_boundary_warned: bool,
        text_guard: crate::qwen_guard::Session,
        #[cfg(feature = "fidelity-eval")]
        section_evidence: Vec<serde_json::Value>,
    }

    pub struct Stream<'a> {
        engine: &'a mut Engine,
        cancel: Option<&'a AtomicBool>,
        resampler: Resampler,
        pending: AudioBuffer,
        qwen: Option<QwenStream>,
        output: Output,
    }

    impl Stream<'_> {
        pub fn add(&mut self, samples: &[f32], rate: u32) -> Result<(), String> {
            check_cancel(self.cancel)?;
            let Self {
                engine,
                cancel,
                resampler,
                pending,
                qwen,
                output,
            } = self;
            if let Some(qwen) = qwen {
                let mut converted = Vec::new();
                resampler.add(samples, rate, |sample| { converted.push(sample); Ok(()) })?;
                return qwen.add(engine, output, &converted, false, *cancel);
            }
            resampler.add(samples, rate, |sample| {
                if push_sample(engine, pending, sample, *cancel)? {
                    decode(engine, pending, output, *cancel)?;
                }
                Ok(())
            })
        }

        pub fn update(&mut self) -> Result<String, String> {
            check_cancel(self.cancel)?;
            Ok(crate::asr::assemble_segments(&self.output.ledger, false))
        }

        pub(crate) fn original_text(&self) -> Option<&str> {
            self.output.original_text.as_deref()
        }

        pub fn warnings(&self) -> &[String] {
            &self.output.warnings
        }

        pub fn finish_recognition(mut self) -> Result<RecognitionResult, String> {
            self.flush()?;
            Ok(self.result())
        }

        fn flush(&mut self) -> Result<(), String> {
            check_cancel(self.cancel)?;
            let Self {
                engine,
                cancel,
                resampler,
                pending,
                qwen,
                output,
            } = self;
            if let Some(qwen) = qwen {
                let mut converted = Vec::new();
                resampler.finish(|sample| { converted.push(sample); Ok(()) })?;
                return qwen.add(engine, output, &converted, true, *cancel);
            }
            resampler.finish(|sample| {
                if push_sample(engine, pending, sample, *cancel)? {
                    decode(engine, pending, output, *cancel)?;
                }
                Ok(())
            })?;
            decode(engine, pending, output, *cancel)
        }

        fn result(self) -> RecognitionResult {
            let mut recognition = crate::asr::finish_recognition(&self.output.ledger);
            // A failed/empty section has no ledger row but still makes the
            // recording incomplete; later successful sections cannot erase it.
            if self.output.incomplete {
                recognition.completeness = crate::domain::RecognitionCompleteness::Incomplete;
            }
            recognition.original_text = self.output.original_text;
            crate::asr::append_recognition_warnings(&mut recognition.warnings, self.output.warnings);
            recognition
        }

        #[cfg(feature = "fidelity-eval")]
        pub fn finish_observed(mut self) -> Result<(RecognitionResult, serde_json::Value), String> {
            self.flush()?;
            let samples = self.output.samples_consumed;
            let llama = matches!(&self.engine.decoder, Decoder::Llama(_));
            let maximum_buffer_samples = self.pending.max_samples()
                + if llama { 0 } else { NATIVE_PAD_SAMPLES };
            let maximum_buffer_samples = if self.qwen.is_some() { 22 * RATE } else { maximum_buffer_samples };
            let filter = self.qwen.as_ref().map(|qwen| qwen.input.mode_name().to_owned());
            let original_tail_confirmation = self.qwen.as_ref().is_some_and(|qwen| qwen.input.gate_only());
            let window_seconds = llama.then(crate::qwen_window::target_seconds);
            let section_evidence = std::mem::take(&mut self.output.section_evidence);
            let sectioner = self.engine.sectioner;
            let vad_fallback = self.engine.vad_fallback.clone();
            let speech_gate = self.engine.speech_gate;
            let decoded_sections = section_evidence.iter().filter(|s| s["decoded"].as_bool() == Some(true)).count();
            let decoded_samples: u64 = section_evidence.iter()
                .filter(|s| s["decoded"].as_bool() == Some(true)).filter_map(|s| s["samples"].as_u64()).sum();
            let recognition = self.result();
            let details = serde_json::json!({
                "segments": &recognition.segments,
                "section_evidence": section_evidence,
                "sectioner": sectioner,
                "qwen_window_seconds": window_seconds,
                "asr_filter": filter,
                "untriggered_tail_requires_original_confirmation": original_tail_confirmation,
                "vad_threshold": if llama {Some(crate::qwen_window::vad_threshold())} else {None},
                "vad_fallback": vad_fallback,
                "speech_gate_requested": speech_gate,
                "sections_decoded": decoded_sections,
                "sections_skipped": section_evidence.len() - decoded_sections,
                "decoded_samples": decoded_samples,
                "skipped_samples": samples.saturating_sub(decoded_samples),
                "warnings": &recognition.warnings,
                "recognition_completeness": recognition.completeness,
                "all_segments_complete": recognition.completeness == crate::domain::RecognitionCompleteness::Complete,
                "timestamp_basis": if llama { "original 16 kHz input offsets; source_sample_ranges records trimmed gaps" }
                    else { "Whisper timestamps plus cumulative 16 kHz input samples" },
                "input_samples_consumed": samples,
                "maximum_buffer_samples": maximum_buffer_samples,
                "truncation_status": if llama { "bounded windows contain real source audio; server finish reason recorded per attempt" }
                    else { "all buffered audio accounted for; decoded sections include native tail padding; native finish reason unavailable" },
                "initialized_provider": "metal",
                "observed_execution": if llama { Some("llama-server confirmed GPU layer offload") } else { None },
            });
            Ok((recognition, details))
        }
    }

    struct QwenStream {
        input: crate::sherpa::QwenInput,
        windows: crate::qwen_window::Assembler,
        detector: crate::qwen_window::Detector,
        audio: Vec<f32>,
        gate: Vec<f32>,
        since_segment: usize,
    }

    impl QwenStream {
        fn new() -> Self {
            Self { input: crate::sherpa::QwenInput::new(),
                windows: crate::qwen_window::Assembler::new(crate::qwen_window::target_seconds()),
                detector: crate::qwen_window::Detector::default(),
                audio: Vec::new(), gate: Vec::new(), since_segment: 0 }
        }

        fn add(&mut self, engine: &mut Engine, output: &mut Output, samples: &[f32],
            finish: bool, cancel: Option<&AtomicBool>) -> Result<(), String> {
            let (audio, gate) = self.input.process(samples, finish);
            self.audio.extend(audio);
            self.gate.extend(gate);
            while self.audio.len() >= 512 || (finish && !self.audio.is_empty()) {
                check_cancel(cancel)?;
                let count = self.audio.len().min(512);
                let audio: Vec<_> = self.audio.drain(..count).collect();
                let gate: Vec<_> = self.gate.drain(..count).collect();
                self.windows.append(&audio, &gate)?;
                let probability = if let Some(vad) = &mut engine.vad {
                    let result = (|| {
                        let mut probability = None;
                        for sample in gate.iter().copied().chain(std::iter::repeat(0.0)).take(512) {
                            if let Some(value) = vad.push_probability(sample)? { probability = Some(value); }
                        }
                        probability.ok_or_else(|| "Silero frame did not produce a probability.".to_owned())
                    })();
                    match result {
                        Ok(value) => Some(value),
                        Err(error) => { engine.fail_vad(error); None }
                    }
                } else { None };
                let probability = match probability {
                    Some(value) => value,
                    None => if speech_gate::fallback(&gate, cancel)?.positive_samples > 0 { 1.0 } else { 0.0 },
                };
                self.since_segment += count;
                if let Some(segment) = self.detector.observe(probability, count) {
                    self.windows.segment(segment, false)?;
                    self.since_segment = 0;
                }
                if self.windows.hard_due() || self.since_segment >= 20 * RATE {
                    if let Some(segment) = self.detector.finish() { self.windows.segment(segment, false)?; }
                    self.since_segment = 0;
                }
                for window in self.windows.poll_active(false, self.detector.speaking())? {
                    decode_qwen_window(engine, output, window, cancel)?;
                }
            }
            if finish {
                if let Some(segment) = self.detector.finish() { self.windows.segment(segment, false)?; }
                let (origin, tail) = self.windows.tail();
                let original = self.input.gate_only().then(|| self.windows.exact(origin, self.windows.end())
                    .map(|window| window.audio)).transpose()?;
                if let Some(range) = speech_gate::confirmed_tail_with_original(&tail, original.as_deref(), cancel)? {
                    self.windows.segment(origin + range.start as u64..origin + range.end as u64, true)?;
                }
                for window in self.windows.poll(true)? { decode_qwen_window(engine, output, window, cancel)?; }
            }
            output.samples_consumed = self.windows.end();
            Ok(())
        }
    }

    fn decode_qwen_window(engine: &mut Engine, output: &mut Output,
        window: crate::qwen_window::Window, cancel: Option<&AtomicBool>) -> Result<(), String> {
        check_cancel(cancel)?;
        // Detector segments already qualified speech. Use confirmation on the
        // aligned gate copy; the decoder keeps original audio in gate-only mode.
        let mode = if engine.speech_gate == speech_gate::Mode::Off { speech_gate::Mode::Off }
            else { speech_gate::Mode::Confirmation };
        let gate = speech_gate::decide(mode, &window.gate_audio, cancel,
            || Err("Streaming detector already qualified speech.".into()))?;
        let mut evidence = serde_json::json!({"decoded":gate.decode, "skipped":!gate.decode,
            "speech_gate_evidence":gate, "samples":window.audio.len(),
            "start_seconds":window.start as f64 / RATE as f64,
            "end_seconds":window.end as f64 / RATE as f64, "boundary":window.reason,
            "recovered_tail":window.recovered_tail,
            "source_sample_ranges":window.ranges.iter().map(|r| [r.start, r.end]).collect::<Vec<_>>()});
        if gate.decode {
            if window.reason == "hard_cap" && !output.forced_boundary_warned {
                output.warnings.push("Continuous speech reached the 20-second section limit; check words at section boundaries.".into());
                output.forced_boundary_warned = true;
            }
            if let Decoder::Llama(server) = &engine.decoder {
                let mut native = serde_json::Value::Null;
                let recognition = crate::sherpa::transcribe_llama(server, &window.audio, Some(&mut native))?;
                check_cancel(cancel)?;
                let id = output.ledger.len() as u64;
                let complete = recognition.completeness == crate::domain::RecognitionCompleteness::Complete;
                output.incomplete |= !complete;
                crate::asr::append_recognition_warnings(&mut output.warnings, recognition.warnings);
                output.ledger.insert(id, Segment { id, speaker: None,
                    start_seconds: window.start as f32 / RATE as f32,
                    end_seconds: Some(window.end as f32 / RATE as f32),
                    text: recognition.text.clone(), complete });
                if crate::sherpa::qwen_guards_enabled() {
                    output.text_guard.push(id, &recognition.text, recognition.original_text.as_deref(),
                        native["detected_language"].as_str());
                    let trace = crate::sherpa::refresh_qwen_guard(&output.text_guard, &mut output.ledger, &mut output.original_text);
                    evidence["qwen_fragment_guard"] = trace.last().cloned().unwrap_or_default();
                    #[cfg(feature = "fidelity-eval")]
                    for previous in &mut output.section_evidence {
                        if let Some(decision) = trace.iter().find(|d| d["id"] == previous["window_id"]) {
                            previous["qwen_fragment_guard"] = decision.clone();
                            previous["text"] = decision["text"].clone();
                        }
                    }
                }
                evidence.as_object_mut().unwrap().extend(native.as_object().cloned().unwrap_or_default());
                evidence["window_id"] = serde_json::json!(id);
                evidence["text"] = serde_json::json!(output.ledger[&id].text);
            }
        }
        #[cfg(feature = "fidelity-eval")]
        output.section_evidence.push(evidence);
        Ok(())
    }

    fn push_sample(
        engine: &mut Engine,
        pending: &mut AudioBuffer,
        sample: f32,
        cancel: Option<&AtomicBool>,
    ) -> Result<bool, String> {
        check_cancel(cancel)?;
        let pause = if engine.sectioner == "silero" {
            match &mut engine.vad {
                Some(vad) => match vad.push(sample) {
                    Ok(pause) => Some(pause),
                    Err(error) => {
                        // Energy history has been updated throughout this recording,
                        // so failure changes detectors without resetting audio/offsets.
                        engine.fail_vad(error);
                        None
                    }
                },
                None => None,
            }
        } else {
            None
        };
        Ok(pending.push_with_pause(sample, pause))
    }

    unsafe extern "C" fn abort_decode(data: *mut c_void) -> bool {
        // The optional AtomicBool is borrowed for the stream's lifetime and
        // the native callback runs only during the synchronous full() call.
        unsafe { &*data.cast::<AtomicBool>() }.load(Ordering::Acquire)
    }

    fn decode(
        engine: &mut Engine,
        pending: &mut AudioBuffer,
        output: &mut Output,
        cancel: Option<&AtomicBool>,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        let count = pending.section_len();
        if count == 0 {
            return Ok(());
        }
        #[cfg(feature = "fidelity-eval")]
        let gate_started = std::time::Instant::now();
        let gate = engine.gate_section(&pending.samples[..count], cancel)?;
        check_cancel(cancel)?;
        #[cfg(feature = "fidelity-eval")]
        let gate_ms = gate_started.elapsed().as_secs_f64() * 1_000.0;
        if gate.decode && pending.forced_boundary() && !output.forced_boundary_warned {
            output.warnings.push(format!("Continuous speech reached the {}-second section limit; check words at section boundaries.", pending.max_samples() / RATE));
            output.forced_boundary_warned = true;
        }
        #[cfg(feature = "fidelity-eval")]
        let mut section = serde_json::json!({
            "start_seconds": output.samples_consumed as f64 / RATE as f64,
            "end_seconds": (output.samples_consumed + count as u64) as f64 / RATE as f64,
            "samples": count,
            "forced_boundary": pending.forced_boundary(),
            "boundary": pending.boundary_kind(),
            "digital_silence": digital_silence(&pending.samples[..count]),
            "sectioner": engine.sectioner,
            "decoded": gate.decode,
            "skipped": !gate.decode,
            "reason": gate.reason,
            "speech_gate": gate.method,
            "speech_gate_evidence": &gate,
            "speech_gate_ms": gate_ms,
        });
        if gate.decode {
            match &mut engine.decoder {
                Decoder::Llama(server) => {
                    // Whisper's 200 ms pad compensates for its window cutoff.
                    // Qwen receives the exact section: no audio is added or lost.
                    let mut evidence = serde_json::Value::Null;
                    let recognition = crate::sherpa::transcribe_llama(server, &pending.samples[..count], Some(&mut evidence))?;
                    check_cancel(cancel)?;
                    let id = output.ledger.len() as u64;
                    let complete = recognition.completeness == crate::domain::RecognitionCompleteness::Complete;
                    if crate::sherpa::qwen_guards_enabled() {
                        output.text_guard.push(id, &recognition.text, recognition.original_text.as_deref(),
                            evidence["detected_language"].as_str());
                    }
                    #[cfg(feature = "fidelity-eval")]
                    {
                        section.as_object_mut().unwrap().extend(evidence.as_object().cloned().unwrap_or_default());
                    }
                    append_llama_result(output, recognition, count);
                    if crate::sherpa::qwen_guards_enabled() {
                        output.ledger.entry(id).or_insert_with(|| Segment { id, speaker: None,
                            start_seconds: output.samples_consumed as f32 / RATE as f32,
                            end_seconds: Some((output.samples_consumed + count as u64) as f32 / RATE as f32),
                            text: String::new(), complete });
                        let trace = crate::sherpa::refresh_qwen_guard(&output.text_guard, &mut output.ledger, &mut output.original_text);
                        #[cfg(feature = "fidelity-eval")]
                        {
                            section["window_id"] = serde_json::json!(id);
                            section["qwen_fragment_guard"] = trace.last().cloned().unwrap_or_default();
                            section["text"] = serde_json::json!(output.ledger[&id].text);
                            for previous in &mut output.section_evidence {
                                if let Some(decision) = trace.iter().find(|d| d["id"] == previous["window_id"]) {
                                    previous["qwen_fragment_guard"] = decision.clone();
                                    previous["text"] = decision["text"].clone();
                                }
                            }
                        }
                        #[cfg(not(feature = "fidelity-eval"))]
                        let _ = trace;
                    }
                }
                Decoder::Whisper(state) => {
                    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                    params.set_n_threads(
                        std::thread::available_parallelism().map_or(2, |count| count.get().min(4)) as i32,
                    );
                    params.set_translate(false);
                    #[cfg(not(feature = "fidelity-eval"))]
                    params.set_language(None);
                    #[cfg(feature = "fidelity-eval")]
                    params.set_language(
                        if std::env::var("SONOJOT_EVAL_ASR_LANGUAGE").as_deref() == Ok("en") {
                            Some("en")
                        } else {
                            None
                        },
                    );
                    params.set_no_context(true);
                    params.set_no_timestamps(false);
                    params.set_print_special(false);
                    params.set_print_progress(false);
                    params.set_print_realtime(false);
                    params.set_print_timestamps(false);
                    params.set_suppress_blank(true);
                    params.set_no_speech_thold(0.6);
                    params.set_max_tokens(0);
                    if let Some(cancel) = cancel {
                        // full() does not retain parameters or callback userdata.
                        unsafe {
                            params.set_abort_callback(Some(abort_decode));
                            params.set_abort_callback_user_data(
                                (cancel as *const AtomicBool).cast_mut().cast(),
                            );
                        }
                    }
                    // whisper.cpp stops with <=100 ms left in a window. Add 200 ms
                    // of silence so that cutoff cannot swallow the real audio tail,
                    // including tiny final utterances. Counts/timestamps exclude it.
                    let mut padded = pending.samples[..count].to_vec();
                    padded.resize(count + NATIVE_PAD_SAMPLES, 0.0);
                    state
                        .full(params, &padded)
                        .map_err(|error| format!("Metal speech recognition failed: {error}"))?;
                    check_cancel(cancel)?;
                    let origin = output.samples_consumed as f64 / RATE as f64;
                    let duration = count as f64 / RATE as f64;
                    for segment in state.as_iter() {
                        let text = segment
                            .to_str()
                            .map_err(|error| format!("Whisper returned invalid text: {error}"))?
                            .to_owned();
                        if text.is_empty() {
                            continue;
                        }
                        let start = (segment.start_timestamp() as f64 / 100.0).clamp(0.0, duration);
                        let end = (segment.end_timestamp() as f64 / 100.0).clamp(start, duration);
                        let id = output.ledger.len() as u64;
                        output.ledger.insert(
                            id,
                            Segment {
                                id,
                                start_seconds: (origin + start) as f32,
                                end_seconds: Some((origin + end) as f32),
                                speaker: None,
                                text,
                                complete: true,
                            },
                        );
                    }
                }
            }
        }
        check_cancel(cancel)?;
        #[cfg(feature = "fidelity-eval")]
        output.section_evidence.push(section);
        output.samples_consumed = output
            .samples_consumed
            .checked_add(count as u64)
            .ok_or("Recognition audio duration overflowed.")?;
        pending.consume(count);
        Ok(())
    }

    fn append_llama_result(output: &mut Output, recognition: RecognitionResult, count: usize) {
        let complete = recognition.completeness == crate::domain::RecognitionCompleteness::Complete;
        output.incomplete |= !complete;
        crate::asr::retain_original_section(&mut output.original_text,
            || crate::asr::assemble_segments(&output.ledger, false), &recognition);
        crate::asr::append_recognition_warnings(&mut output.warnings, recognition.warnings);
        if recognition.text.trim().is_empty() {
            return;
        }
        let id = output.ledger.len() as u64;
        output.ledger.insert(id, Segment {
            id,
            start_seconds: output.samples_consumed as f32 / RATE as f32,
            end_seconds: Some((output.samples_consumed + count as u64) as f32 / RATE as f32),
            speaker: None,
            text: recognition.text,
            complete,
        });
    }

    #[cfg(test)]
    mod decoder_tests {
        use super::*;

        #[test]
        fn qwen_dropped_section_preserves_original_before_later_usable_speech() {
            let mut output = Output::default();
            append_llama_result(&mut output, crate::sherpa::llama_test_recognition((
                "go go go go".into(), Some(8), "stop".into(),
            )), RATE);
            assert!(output.ledger.is_empty());
            assert!(output.incomplete);
            output.samples_consumed += RATE as u64;
            append_llama_result(&mut output, crate::sherpa::llama_test_recognition((
                "Useful speech.".into(), Some(8), "stop".into(),
            )), RATE);
            assert_eq!(crate::asr::assemble_segments(&output.ledger, false), "Useful speech.");
            assert_eq!(output.original_text.as_deref(), Some("go go go go Useful speech."));
            assert_eq!(output.ledger[&0].start_seconds, 1.0);
            assert!(output.incomplete);
        }

        #[test]
        fn qwen_sections_keep_sample_timestamps_and_incomplete_empty_results() {
            let mut output = Output { samples_consumed: 2 * RATE as u64, ..Default::default() };
            let response = ("language English<asr_text>Hello.".into(), Some(3), "stop".into());
            append_llama_result(&mut output, crate::sherpa::llama_test_recognition(response), RATE);
            assert_eq!(output.ledger[&0].text, "Hello.");
            assert_eq!(output.ledger[&0].start_seconds, 2.0);
            assert_eq!(output.ledger[&0].end_seconds, Some(3.0));
            assert!(!output.incomplete);
            let empty = ("language English<asr_text>".into(), Some(512), "length".into());
            append_llama_result(&mut output, crate::sherpa::llama_test_recognition(empty), RATE);
            assert_eq!(output.ledger.len(), 1);
            assert!(output.incomplete);
            assert!(output.warnings.iter().any(|warning| warning.contains("token limit")));
        }
    }
}

#[cfg(target_os = "macos")]
pub use native::{Engine, Stream};

#[cfg(test)]
mod tests {
    use super::{audio::*, *};

    fn resample(input: &[f32], rate: u32, fragment: usize) -> Vec<f32> {
        let mut resampler = Resampler::default();
        let mut output = Vec::new();
        for chunk in input.chunks(fragment) {
            resampler
                .add(chunk, rate, |sample| {
                    output.push(sample);
                    Ok(())
                })
                .unwrap();
        }
        resampler
            .finish(|sample| {
                output.push(sample);
                Ok(())
            })
            .unwrap();
        output
    }

    #[test]
    fn fragmented_capture_keeps_resampling_phase_and_final_tail() {
        for (rate, size) in [(48_000, 480), (44_100, 137), (8_000, 13), (16_000, 1)] {
            let input: Vec<_> = (0..rate + 7)
                .map(|i| (i as f32 / 23.0).sin() * 0.3)
                .collect();
            let expected = resample(&input, rate, input.len());
            assert_eq!(resample(&input, rate, size), expected);
            assert_eq!(resample(&input, rate, 1), expected);
            assert_eq!(expected.len(), (input.len() * RATE).div_ceil(rate as usize));
        }
    }

    #[test]
    fn resampler_rejects_format_changes_and_bad_samples() {
        let mut resampler = Resampler::default();
        assert!(resampler.add(&[0.0], 0, |_| Ok(())).is_err());
        resampler.add(&[0.1], 48_000, |_| Ok(())).unwrap();
        assert!(resampler.add(&[0.1], 44_100, |_| Ok(())).is_err());
        assert!(resampler.add(&[f32::NAN], 48_000, |_| Ok(())).is_err());
    }

    #[test]
    fn continuous_speech_is_bounded_and_every_sample_survives() {
        let mut pending = AudioBuffer::default();
        let total = MAX_SAMPLES * 3 + 17;
        let mut decoded = 0;
        for _ in 0..total {
            if pending.push(0.2) {
                assert_eq!(pending.samples.len(), MAX_SAMPLES);
                assert!(pending.forced_boundary());
                decoded += pending.samples.len();
                pending.clear();
            }
            assert!(pending.samples.len() <= MAX_SAMPLES);
        }
        assert_eq!(decoded + pending.samples.len(), total);
        assert_eq!(pending.samples.capacity(), MAX_SAMPLES + NATIVE_PAD_SAMPLES);
    }

    #[test]
    fn quiet_pause_chooses_boundary_without_discarding_audio() {
        let mut pending = AudioBuffer::default();
        for _ in 0..3 * RATE {
            assert!(!pending.push(0.2));
        }
        for _ in 0..3 * RATE / 5 - 1 {
            assert!(!pending.push(0.0));
        }
        assert!(pending.push(0.0));
        assert!(!pending.forced_boundary());
        assert_eq!(pending.samples.len(), 3 * RATE + 3 * RATE / 5);
        assert!(digital_silence(&[0.0; 160]));
        assert!(!digital_silence(&[0.0001; 160]));
    }

    #[test]
    fn background_noise_pause_is_detected_without_treating_speech_as_noise() {
        let mut pending = AudioBuffer::default();
        for _ in 0..4 * RATE {
            assert!(!pending.push(0.2));
        }
        let mut end = None;
        for i in 0..RATE {
            // Well above the original fixed RMS and peak thresholds.
            if pending.push(if i % 2 == 0 { 0.02 } else { -0.02 }) {
                end = Some(i);
                break;
            }
        }
        assert!(end.is_some(), "a one-second noisy pause should close the section");
        assert!(!pending.forced_boundary());
        assert!(pending.section_len() >= 4 * RATE + RATE / 2);
    }

    #[test]
    fn maximum_uses_recent_quietest_window_and_carries_exact_remainder() {
        let input: Vec<_> = (0..MAX_SAMPLES).map(|i| {
            if (15 * RATE..15 * RATE + RATE / 3).contains(&i) { 0.01 } else { 0.2 }
        }).collect();
        let mut pending = AudioBuffer::default();
        for (i, &sample) in input.iter().enumerate() {
            assert_eq!(pending.push(sample), i + 1 == input.len());
        }
        assert!(pending.forced_boundary());
        let cut = pending.section_len();
        assert!((15 * RATE..15 * RATE + RATE / 3).contains(&cut));
        let mut reconstructed = pending.samples[..cut].to_vec();
        pending.consume(cut);
        assert_eq!(pending.samples, input[cut..]);
        assert_eq!(pending.boundary_kind(), "finish");
        reconstructed.extend_from_slice(&pending.samples);
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn maximum_search_ignores_older_quieter_audio() {
        let mut pending = AudioBuffer::default();
        for i in 0..MAX_SAMPLES {
            let sample = if (10 * RATE..10 * RATE + RATE / 3).contains(&i) {
                0.005
            } else if (17 * RATE..17 * RATE + RATE / 3).contains(&i) {
                0.03
            } else {
                0.2
            };
            assert_eq!(pending.push(sample), i + 1 == MAX_SAMPLES);
        }
        assert!((17 * RATE..17 * RATE + RATE / 3).contains(&pending.section_len()));
    }

    #[test]
    fn vad_pause_still_respects_minimum_and_frame_alignment() {
        let mut pending = AudioBuffer::default();
        for _ in 0..3 * RATE - 1 {
            assert!(!pending.push_with_pause(0.1, Some(true)));
        }
        assert!(pending.push_with_pause(0.1, Some(true)));
        pending.consume(pending.section_len());
        for _ in 0..3 * RATE + 1 {
            assert!(!pending.push_with_pause(0.1, Some(false)));
        }
        for _ in 0..158 {
            assert!(!pending.push_with_pause(0.1, Some(true)));
        }
        assert!(pending.push_with_pause(0.1, Some(true)));
        assert_eq!(pending.section_len() % 160, 0);
    }

    #[test]
    fn carried_sections_have_contiguous_offsets_and_obey_minimum_and_maximum() {
        let input: Vec<_> = (0..3 * MAX_SAMPLES + 73).map(|i| {
            if i % (17 * RATE) < RATE / 3 { 0.01 } else { 0.2 }
        }).collect();
        let mut pending = AudioBuffer::default();
        let mut reconstructed = Vec::new();
        let mut origin = 0;
        let mut intervals = Vec::new();
        for &sample in &input {
            if pending.push(sample) {
                let count = pending.section_len();
                assert!((3 * RATE..=MAX_SAMPLES).contains(&count));
                reconstructed.extend_from_slice(&pending.samples[..count]);
                intervals.push((origin, origin + count));
                origin += count;
                pending.consume(count);
            }
            assert!(pending.samples.len() <= MAX_SAMPLES);
        }
        intervals.push((origin, origin + pending.section_len()));
        reconstructed.extend_from_slice(&pending.samples);
        for pair in intervals.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
            assert_eq!(pair[0].1 as f64 / RATE as f64, pair[1].0 as f64 / RATE as f64);
        }
        assert_eq!(intervals.last().unwrap().1, input.len());
        assert_eq!(reconstructed, input);
    }

    #[test]
    fn quiet_input_waits_for_minimum_and_finish_keeps_short_tail() {
        let mut pending = AudioBuffer::default();
        for _ in 0..3 * RATE - 1 {
            assert!(!pending.push(0.0));
        }
        assert!(pending.push(0.0));
        assert_eq!(pending.section_len(), 3 * RATE);
        pending.consume(pending.section_len());
        for _ in 0..17 {
            assert!(!pending.push(0.001));
        }
        assert_eq!(pending.section_len(), 17);
        pending.consume(17);
        assert!(pending.samples.is_empty());
    }

    #[test]
    fn cancellation_stops_before_consuming_a_section_or_its_remainder() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancel = AtomicBool::new(false);
        let mut pending = AudioBuffer::default();
        for i in 0..MAX_SAMPLES {
            pending.push(if (15 * RATE..15 * RATE + RATE / 3).contains(&i) { 0.01 } else { 0.2 });
        }
        let cut = pending.section_len();
        assert!(cut < MAX_SAMPLES);
        check_cancel(Some(&cancel)).unwrap();
        cancel.store(true, Ordering::Release);
        let consumed = (|| -> Result<(), String> {
            check_cancel(Some(&cancel))?;
            pending.consume(pending.section_len());
            Ok(())
        })();
        assert_eq!(consumed.unwrap_err(), "Transcription cancelled.");
        assert_eq!(pending.section_len(), cut);
        assert_eq!(pending.samples.len(), MAX_SAMPLES);
        cancel.store(false, Ordering::Release);
        check_cancel(Some(&cancel)).unwrap();
        pending.consume(cut);
        cancel.store(true, Ordering::Release);
        assert!(check_cancel(Some(&cancel)).is_err());
        assert_eq!(pending.samples.len(), MAX_SAMPLES - cut);
    }

    #[test]
    fn experimental_window_keeps_every_sample_and_stays_below_whisper_context() {
        for seconds in [20, 28] {
            let maximum = seconds * RATE;
            let mut pending = AudioBuffer::with_max_samples(maximum);
            let total = 2 * maximum + 17;
            let mut consumed = 0;
            for _ in 0..total {
                if pending.push(0.2) {
                    assert!(pending.forced_boundary());
                    assert_eq!(pending.samples.len(), maximum);
                    consumed += pending.samples.len();
                    pending.clear();
                }
            }
            assert_eq!(consumed + pending.samples.len(), total);
            assert_eq!(pending.max_samples(), maximum);
            assert!(maximum + NATIVE_PAD_SAMPLES < 30 * RATE);
            assert_eq!(pending.samples.capacity(), maximum + NATIVE_PAD_SAMPLES);
        }
    }

    #[test]
    fn catalog_is_disjoint_and_missing_files_are_not_ready() {
        for model in catalog() {
            assert!(
                model.sha256.len() == 64
                    && model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            );
            assert!(model.revision.len() == 40 && model.bytes > 0);
            assert!(!crate::gpu_asr::is_gpu_model(model.id));
            assert!(!crate::whisper_catalog::is_whisper(model.id));
            assert_eq!(crate::asr::provider_requested(model.id), "metal");
            assert_eq!(crate::asr::backend_for(model.id), BACKEND);
            assert_eq!(crate::asr::compatibility_for(model.id), COMPATIBILITY);
            let root = Path::new("missing-metal-model-fixture");
            assert_eq!(
                crate::asr::model_directory(root, model.id),
                model_dir(root, model.id)
            );
            assert!(!crate::asr::model_ready(root, model.id));
        }
    }

    #[test]
    fn qwen_window_selection_preserves_whisper_and_the_evaluation_baseline() {
        let qwen = if crate::qwen_window::target_seconds() == 0 { "energy" } else { "qwen_windows" };
        assert_eq!(production_sectioner("qwen3-asr-1.7b-q8-metal"), qwen);
        assert_eq!(production_sectioner("qwen3-asr-1.7b-q4-metal"), qwen);
        assert_eq!(production_sectioner("whisper-small-metal"), "baseline");
        assert_eq!(production_sectioner("whisper-turbo-metal"), "baseline");
    }
}
