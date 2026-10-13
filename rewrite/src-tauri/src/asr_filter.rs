//! Session-owned speech filtering. Construct a fresh filter per recording.
//!
//! Production uses RNNoise for Qwen gating and ignores evaluation overrides.
//! Evaluation reads SONOJOT_EVAL_ASR_FILTER once at construction. Never apply
//! this filter to the recording writer. In rnnoise-gate mode, decode ORIGINAL
//! audio; use the filtered copy only for VAD and confirmation.
//!
//! Every process call returns exactly its input length, independent of chunking.
//! RNNoise output is delayed by 352 samples at 16 kHz (22 ms): 160 samples to
//! assemble a frame, 160 from RNNoise overlap-add, and 16 each for the two FIRs.
//! Map a filtered index to original time by subtracting latency_samples(). At
//! finish, flush() supplies that delayed tail; it is padding, not capture time.
//! When retaining an aligned gate copy, concatenate process outputs and flush,
//! discard the first latency_samples(), then take the original sample count.

pub(crate) struct Filter {
    inner: Candidate,
}

impl Filter {
    pub(crate) fn from_env() -> Self {
        #[cfg(feature = "fidelity-eval")]
        {
            Self::named(&std::env::var("SONOJOT_EVAL_ASR_FILTER").unwrap_or_else(|_| "rnnoise-gate".into()))
        }
        #[cfg(not(feature = "fidelity-eval"))]
        Self { inner: Candidate::Rnnoise { filter: Box::new(Rnnoise::new()), gate_only: true } }
    }

    pub(crate) fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        match &mut self.inner {
            #[cfg(feature = "fidelity-eval")]
            Candidate::Off => samples.to_vec(),
            #[cfg(feature = "fidelity-eval")]
            Candidate::Highpass(filter) => samples.iter().map(|&x| filter.push(x)).collect(),
            Candidate::Rnnoise { filter, .. } => samples.iter().map(|&x| filter.push(x)).collect(),
        }
    }

    pub(crate) fn gate_only(&self) -> bool {
        matches!(&self.inner, Candidate::Rnnoise { gate_only: true, .. })
    }

    pub(crate) fn mode_name(&self) -> &str {
        match &self.inner {
            #[cfg(feature = "fidelity-eval")]
            Candidate::Off => "off",
            #[cfg(feature = "fidelity-eval")]
            Candidate::Highpass(_) => "highpass",
            Candidate::Rnnoise { gate_only: true, .. } => "rnnoise-gate",
            Candidate::Rnnoise { gate_only: false, .. } => "rnnoise-asr",
        }
    }

    /// Fixed transport delay, not frequency-dependent IIR phase delay.
    pub(crate) fn latency_samples(&self) -> usize {
        if matches!(&self.inner, Candidate::Rnnoise { .. }) { return 352; }
        0
    }

    /// Call once after the final real samples. Do not count this as capture.
    pub(crate) fn flush(&mut self) -> Vec<f32> {
        self.process(&vec![0.0; self.latency_samples()])
    }

    #[cfg(feature = "fidelity-eval")]
    fn named(name: &str) -> Self {
        let inner = match name {
            "off" => Candidate::Off,
            "highpass" => Candidate::Highpass(Highpass::new()),
            "rnnoise-asr" | "rnnoise-gate" => Candidate::Rnnoise {
                filter: Box::new(Rnnoise::new()), gate_only: name == "rnnoise-gate",
            },
            // A misspelt candidate must not silently produce baseline results.
            _ => panic!("Invalid SONOJOT_EVAL_ASR_FILTER: {name}"),
        };
        Self { inner }
    }
}

enum Candidate {
    #[cfg(feature = "fidelity-eval")]
    Off,
    #[cfg(feature = "fidelity-eval")]
    Highpass(Highpass),
    Rnnoise { filter: Box<Rnnoise>, gate_only: bool },
}

/// Bilinear-transform Butterworth high-pass, fc=80 Hz, fs=16000 Hz, Q=1/sqrt(2).
#[cfg(feature = "fidelity-eval")]
struct Highpass { b: [f64; 3], a: [f64; 2], z: [f64; 2] }

#[cfg(feature = "fidelity-eval")]
impl Highpass {
    fn new() -> Self {
        let k = (std::f64::consts::PI * 80.0 / 16_000.0).tan();
        let norm = 1.0 / (1.0 + std::f64::consts::SQRT_2 * k + k * k);
        Self {
            b: [norm, -2.0 * norm, norm],
            a: [2.0 * (k * k - 1.0) * norm,
                (1.0 - std::f64::consts::SQRT_2 * k + k * k) * norm],
            z: [0.0; 2],
        }
    }

    fn push(&mut self, input: f32) -> f32 {
        let x = finite(input) as f64;
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y as f32
    }
}

fn finite(sample: f32) -> f32 { if sample.is_finite() { sample } else { 0.0 } }

/// 97-tap Blackman-windowed sinc at 7.2 kHz / 48 kHz; DC gain one.
/// Both directions are causal. Zero insertion uses gain three to preserve level.
struct Fir { taps: [f32; 97], history: [f32; 97], head: usize }

impl Fir {
    fn new() -> Self {
        let mut taps = [0.0; 97];
        for (i, tap) in taps.iter_mut().enumerate() {
            let offset = i as f64 - 48.0;
            let sinc = if i == 48 { 0.3 } else {
                (std::f64::consts::PI * 0.3 * offset).sin() / (std::f64::consts::PI * offset)
            };
            let phase = std::f64::consts::TAU * i as f64 / 96.0;
            *tap = (sinc * (0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos())) as f32;
        }
        let total: f32 = taps.iter().sum();
        for tap in &mut taps { *tap /= total; }
        Self { taps, history: [0.0; 97], head: 0 }
    }

    fn push(&mut self, sample: f32) -> f32 {
        self.history[self.head] = sample;
        let mut value = 0.0;
        let mut cursor = self.head;
        for &tap in &self.taps {
            value += tap * self.history[cursor];
            cursor = if cursor == 0 { 96 } else { cursor - 1 };
        }
        self.head = if self.head == 96 { 0 } else { self.head + 1 };
        value
    }
}

struct Rnnoise {
    denoiser: Box<nnnoiseless::DenoiseState<'static>>,
    up: Fir,
    down: Fir,
    frame: [f32; 480],
    filled: usize,
    ready: std::collections::VecDeque<f32>,
}

impl Rnnoise {
    fn new() -> Self {
        Self {
            denoiser: nnnoiseless::DenoiseState::new(), up: Fir::new(), down: Fir::new(),
            frame: [0.0; 480], filled: 0,
            ready: std::iter::repeat_n(0.0, 160).collect(),
        }
    }

    fn push(&mut self, sample: f32) -> f32 {
        for phase in 0..3 {
            let input = if phase == 0 { finite(sample).clamp(-1.0, 1.0) * 3.0 } else { 0.0 };
            // nnnoiseless expects PCM16-scale floats, not normalized floats.
            self.frame[self.filled] = (self.up.push(input) * 32768.0).clamp(-32768.0, 32767.0);
            self.filled += 1;
        }
        if self.filled == 480 {
            let mut output = [0.0; 480];
            self.denoiser.process_frame(&mut output, &self.frame);
            for (i, sample) in output.into_iter().enumerate() {
                let filtered = self.down.push(finite(sample) / 32768.0);
                if i % 3 == 0 { self.ready.push_back(filtered); }
            }
            self.filled = 0;
        }
        // The fixed initial frame makes this independent of callback size.
        self.ready.pop_front().expect("RNNoise frame accounting")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "fidelity-eval"))]
    #[test]
    fn production_always_filters_the_gate_and_accounts_for_delayed_samples() {
        let mut filter = Filter::from_env();
        let input = [0.1, -0.2, f32::NAN, f32::INFINITY];
        let output = filter.process(&input);
        assert_eq!(input.len(), output.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert_eq!(filter.mode_name(), "rnnoise-gate");
        assert!(filter.gate_only());
        assert_eq!(filter.latency_samples(), 352);
        assert_eq!(filter.flush().len(), 352);
    }

    #[cfg(feature = "fidelity-eval")]
    mod evaluation {
        use super::*;

        fn signal(count: usize, hz: f32) -> Vec<f32> {
            (0..count).map(|i| (i as f32 * std::f32::consts::TAU * hz / 16_000.0).sin() * 0.2).collect()
        }

        fn rms(xs: &[f32]) -> f64 {
            (xs.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / xs.len() as f64).sqrt()
        }

        #[test]
        fn names_and_placements_are_explicit() {
            for name in ["off", "highpass", "rnnoise-asr", "rnnoise-gate"] {
                let filter = Filter::named(name);
                assert_eq!(filter.mode_name(), name);
                assert_eq!(filter.gate_only(), name == "rnnoise-gate");
                assert_eq!(filter.latency_samples(), if name.starts_with("rnnoise") { 352 } else { 0 });
            }
        }

        #[test]
        #[should_panic(expected = "Invalid SONOJOT_EVAL_ASR_FILTER")]
        fn unknown_candidate_cannot_masquerade_as_baseline() { Filter::named("rnnois"); }

        #[test]
        fn off_preserves_every_bit() {
            let input = [0.1, -0.0, f32::NAN, f32::INFINITY];
            assert_eq!(input.map(f32::to_bits), Filter::named("off").process(&input).iter().map(|x| x.to_bits()).collect::<Vec<_>>().as_slice());
        }

        #[test]
        fn highpass_has_butterworth_response_and_rejects_dc() {
            for (hz, expected) in [(20.0, 0.0624), (80.0, std::f64::consts::FRAC_1_SQRT_2), (1000.0, 1.0)] {
                let input = signal(32_000, hz);
                let output = Filter::named("highpass").process(&input);
                let gain = rms(&output[16_000..]) / rms(&input[16_000..]);
                assert!((gain - expected).abs() < 0.002, "{hz}: {gain}");
            }
            let output = Filter::named("highpass").process(&vec![0.5; 16_000]);
            assert!(rms(&output[8000..]) < 1e-10);
        }

        #[test]
        fn arbitrary_chunks_preserve_count_and_output() {
            let input = signal(3217, 230.0);
            for name in ["off", "highpass", "rnnoise-asr", "rnnoise-gate"] {
                let mut whole = Filter::named(name);
                let expected = whole.process(&input);
                for size in [1, 159, 160, 161, 512] {
                    let mut filter = Filter::named(name);
                    let mut actual = Vec::new();
                    for chunk in input.chunks(size) {
                        let output = filter.process(chunk);
                        assert_eq!(output.len(), chunk.len());
                        actual.extend(output);
                        assert!(filter.process(&[]).is_empty());
                    }
                    assert_eq!(actual, expected, "{name}, chunks {size}");
                    assert_eq!(filter.flush().len(), filter.latency_samples());
                }
            }
        }

        #[test]
        fn new_sessions_do_not_share_state() {
            for name in ["highpass", "rnnoise-asr", "rnnoise-gate"] {
                let mut previous = Filter::named(name);
                previous.process(&signal(4800, 700.0));
                let mut next = Filter::named(name);
                assert!(next.process(&vec![0.0; 4800]).iter().all(|&x| x == 0.0));
            }
        }

        #[test]
        fn gate_and_asr_share_audio_and_flush_recovers_partial_frames() {
            for count in [1, 159, 160, 161, 479, 480, 481] {
                let input = signal(count, 230.0);
                let mut asr = Filter::named("rnnoise-asr");
                let mut gate = Filter::named("rnnoise-gate");
                let mut actual = asr.process(&input);
                actual.extend(asr.flush());
                let mut padded = input.clone();
                padded.resize(count + gate.latency_samples(), 0.0);
                assert_eq!(actual, gate.process(&padded));
                assert_eq!(actual.len() - asr.latency_samples(), count);
                assert!(actual.iter().all(|x| x.is_finite()));
            }
        }

        #[test]
        fn invalid_samples_do_not_poison_filter_state() {
            for name in ["highpass", "rnnoise-asr"] {
                let mut input = signal(3200, 200.0);
                input[12] = f32::NAN; input[150] = f32::INFINITY;
                assert!(Filter::named(name).process(&input).iter().all(|x| x.is_finite()));
            }
        }

        #[test]
        fn resampler_delay_level_and_antialiasing() {
            let mut up = Fir::new();
            let mut down = Fir::new();
            let mut impulse = Vec::new();
            for i in 0..128 {
                for phase in 0..3 {
                    let value = down.push(up.push(if i == 0 && phase == 0 { 3.0 } else { 0.0 }));
                    if phase == 0 { impulse.push(value); }
                }
            }
            let peak = impulse.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
            assert_eq!(peak, 32);
            assert!((impulse.iter().sum::<f32>() - 1.0).abs() < 1e-4);
            for (hz, maximum) in [(1000.0, 1.01), (10_000.0, 0.001)] {
                let mut fir = Fir::new();
                let output: Vec<f32> = (0..4800).map(|i| fir.push((i as f32 * std::f32::consts::TAU * hz / 48_000.0).sin())).collect();
                let gain = rms(&output[2400..]) * std::f64::consts::SQRT_2;
                assert!(gain < maximum);
                if hz == 1000.0 { assert!(gain > 0.99); }
            }
        }
    }
}
