//! Slow level assist for the speech detector's copy of file and meeting audio.
//!
//! Silero (v4, as pinned) scores speech by absolute level: a meeting microphone
//! 15-20 dB quieter than usual stays below its threshold for whole minutes,
//! while stationary room noise does not trigger it even 30 dB louder. So the
//! detector, and only the detector, gets a copy raised towards a fixed speech
//! level. The recogniser always decodes the original samples.
//!
//! Speech level is the 95th percentile of 32 ms frame power, measured after an
//! 80-100 Hz high-pass (hum and rumble dominate a quiet microphone's raw RMS),
//! over the last 30 seconds. Gain = target - level, between 0 and +24 dB, and
//! never so high that the loudest frame of the last half second would exceed
//! -15 dBFS. It rises at most 0.5 dB per frame and falls at once, so speech
//! after a long silence is not over-driven; a raised sample is held to 0.9. It never
//! attenuates, so normal and loud audio reach the detector unchanged.
//! Deterministic and causal: each frame's gain comes from earlier frames only,
//! and frames sit at fixed 512-sample positions, so output does not depend on
//! how callers chunk the audio. It can run before Qwen's RNNoise gate filter,
//! whose suppression would otherwise erase quiet speech first.

use std::collections::VecDeque;

pub(crate) const FRAME: usize = 512;
const TARGET_DB: f32 = -30.0;
const MAX_GAIN_DB: f32 = 24.0;
const STEP_DB: f32 = 0.5;
const WINDOW_FRAMES: usize = 30 * 16_000 / FRAME;
const PEAK_LIMIT: f32 = 0.9;
const CEILING_DB: f32 = -15.0;
const RECENT_FRAMES: usize = 16;
/// Power histogram in 0.5 dB bins from -120 dBFS to 0 dBFS.
const BINS: usize = 240;

/// Production always levels. Evaluation builds read SONOJOT_EVAL_LEVEL_ASSIST:
/// `off` restores the unlevelled detector; `decode` also sends levelled audio
/// to non-Qwen recognisers (Qwen always decodes the original).
pub(crate) fn assist_enabled() -> bool {
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_LEVEL_ASSIST").as_deref() == Ok("off") { return false; }
    true
}

pub(crate) fn decode_levelled() -> bool {
    #[cfg(feature = "fidelity-eval")]
    if std::env::var("SONOJOT_EVAL_LEVEL_ASSIST").as_deref() == Ok("decode") { return true; }
    false
}

pub(crate) struct DetectorLevel {
    highpass: Highpass,
    histogram: [u32; BINS],
    frames: VecDeque<u8>,
    recent: VecDeque<f32>,
    gain_db: f32,
    gain: f32,
    frame_power: f64,
    frame_samples: usize,
}

impl Default for DetectorLevel {
    fn default() -> Self {
        Self { highpass: Highpass::new(), histogram: [0; BINS], frames: VecDeque::with_capacity(WINDOW_FRAMES + 1),
            recent: VecDeque::with_capacity(RECENT_FRAMES + 1), gain_db: 0.0,
            gain: 1.0, frame_power: 0.0, frame_samples: 0 }
    }
}

impl DetectorLevel {
    /// Any number of samples in, the same number out.
    pub(crate) fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples.len());
        for &sample in samples {
            let sample = if sample.is_finite() { sample } else { 0.0 };
            let filtered = f64::from(self.highpass.push(sample));
            self.frame_power += filtered * filtered;
            self.frame_samples += 1;
            let raised = sample * self.gain;
            out.push(if raised.abs() > PEAK_LIMIT.max(sample.abs()) {
                PEAK_LIMIT.max(sample.abs()).copysign(sample)
            } else { raised });
            if self.frame_samples == FRAME { self.end_frame(); }
        }
        out
    }

    fn end_frame(&mut self) {
        let db = (10.0 * (self.frame_power / FRAME as f64 + 1e-12).log10()) as f32;
        self.frame_power = 0.0;
        self.frame_samples = 0;
        let bin = ((db + 120.0) * 2.0).clamp(0.0, (BINS - 1) as f32) as u8;
        self.histogram[bin as usize] += 1;
        self.frames.push_back(bin);
        if self.frames.len() > WINDOW_FRAMES {
            let old = self.frames.pop_front().unwrap();
            self.histogram[old as usize] -= 1;
        }
        self.recent.push_back(db);
        if self.recent.len() > RECENT_FRAMES { self.recent.pop_front(); }
        let loudest = self.recent.iter().copied().fold(f32::MIN, f32::max);
        let wanted = (TARGET_DB - self.level_db()).min(CEILING_DB - loudest).clamp(0.0, MAX_GAIN_DB);
        self.gain_db = if wanted < self.gain_db { wanted } else { (self.gain_db + STEP_DB).min(wanted) };
        self.gain = 10f32.powf(self.gain_db / 20.0);
    }

    /// 95th percentile of high-passed frame power over the window, in dBFS.
    pub(crate) fn level_db(&self) -> f32 {
        let wanted = (self.frames.len() as f64 * 0.95).ceil() as u32;
        let mut seen = 0;
        for (bin, &count) in self.histogram.iter().enumerate() {
            seen += count;
            if seen >= wanted.max(1) { return bin as f32 / 2.0 - 120.0 + 0.25; }
        }
        -120.0
    }

    #[cfg(any(test, feature = "fidelity-eval"))]
    pub(crate) fn gain_db(&self) -> f32 { self.gain_db }
}

/// Second-order Butterworth high-pass at 100 Hz for 16 kHz input.
struct Highpass { b: [f64; 3], a: [f64; 2], z: [f64; 2] }

impl Highpass {
    fn new() -> Self {
        let k = (std::f64::consts::PI * 100.0 / 16_000.0).tan();
        let norm = 1.0 / (1.0 + std::f64::consts::SQRT_2 * k + k * k);
        Self { b: [norm, -2.0 * norm, norm],
            a: [2.0 * (k * k - 1.0) * norm, (1.0 - std::f64::consts::SQRT_2 * k + k * k) * norm], z: [0.0; 2] }
    }

    fn push(&mut self, input: f32) -> f32 {
        let x = f64::from(input);
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y as f32
    }
}

/// Original samples in detector coordinates, so a detector segment cut from
/// the levelled copy can be decoded from the untouched source instead.
#[derive(Default)]
pub(crate) struct OriginalRing {
    samples: VecDeque<f32>,
    base: u64,
}

impl OriginalRing {
    /// Comfortably longer than the detector's 30 s buffer and 20 s sections.
    const RETAIN: usize = 64 * 16_000;

    pub(crate) fn push(&mut self, samples: &[f32]) {
        self.samples.extend(samples);
        let drop = self.samples.len().saturating_sub(Self::RETAIN);
        if drop > 0 { self.samples.drain(..drop); self.base += drop as u64; }
    }

    /// The original samples for `count` detector samples starting at `start`.
    /// None if they are no longer (or not yet) retained.
    pub(crate) fn get(&self, start: u64, count: usize) -> Option<Vec<f32>> {
        let offset = usize::try_from(start.checked_sub(self.base)?).ok()?;
        if offset.checked_add(count)? > self.samples.len() { return None; }
        Some(self.samples.range(offset..offset + count).copied().collect())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic speech-shaped test signal: 1.2 s voiced bursts (a 140 Hz
    /// harmonic series with syllable-rate amplitude modulation and a little
    /// noise) separated by 0.8 s pauses, at roughly -26 dBFS speech level.
    pub(crate) fn speech_like(seconds: f32, attenuation_db: f32, seed: u64) -> Vec<f32> {
        let mut noise = Lcg(seed);
        let scale = 10f32.powf(-attenuation_db / 20.0);
        let n = (seconds * 16_000.0) as usize;
        (0..n).map(|i| {
            let t = i as f32 / 16_000.0;
            let cycle = t % 2.0;
            let burst = if cycle < 1.2 { (std::f32::consts::PI * cycle / 1.2).sin().powf(0.3) } else { 0.0 };
            let syllable = 0.55 + 0.45 * (std::f32::consts::TAU * 4.0 * t).sin();
            let pitch = 140.0 * (1.0 + 0.05 * (std::f32::consts::TAU * 0.7 * t).sin());
            let mut voiced = 0.0;
            for h in 1..=24 {
                let f = pitch * h as f32;
                // A rough formant envelope: strong 300-900 Hz, a second peak near 1.8 kHz.
                let env = (-((f - 600.0) / 450.0).powi(2)).exp() + 0.5 * (-((f - 1800.0) / 500.0).powi(2)).exp() + 0.05;
                voiced += env * (std::f32::consts::TAU * f * t).sin() / h as f32;
            }
            scale * (0.08 * burst * syllable * voiced + 0.0005 * noise.next())
        }).collect()
    }

    /// Steady broadband background noise at an RMS level in dBFS.
    pub(crate) fn steady_noise(seconds: f32, level_db: f32, seed: u64) -> Vec<f32> {
        let mut noise = Lcg(seed);
        let amp = 10f32.powf(level_db / 20.0) * 3f32.sqrt();
        (0..(seconds * 16_000.0) as usize).map(|_| amp * noise.next()).collect()
    }

    pub(crate) struct Lcg(pub u64);
    impl Lcg {
        /// Uniform in [-1, 1).
        pub(crate) fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    fn run(samples: &[f32]) -> (Vec<f32>, DetectorLevel) {
        let mut level = DetectorLevel::default();
        let out = samples.chunks(FRAME).flat_map(|f| level.process(f)).collect();
        (out, level)
    }

    fn rms_db(x: &[f32]) -> f32 {
        10.0 * (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64 + 1e-12).log10() as f32
    }

    #[test]
    fn normal_speech_reaches_the_detector_unchanged() {
        let speech = speech_like(20.0, 0.0, 1);
        let (out, level) = run(&speech);
        assert!(level.level_db() > TARGET_DB, "test speech level {}", level.level_db());
        // The first frames have no history; after that the copy is exact.
        assert_eq!(out[3 * 16_000..], speech[3 * 16_000..]);
    }

    #[test]
    fn quiet_speech_is_raised_by_its_attenuation_up_to_the_cap() {
        let reference = run(&speech_like(30.0, 0.0, 2)).1.level_db();
        for attenuation in [7.0f32, 12.0, 18.0] {
            let quiet = speech_like(30.0, attenuation, 2);
            let (out, level) = run(&quiet);
            let raised = rms_db(&out[16_000 * 20..]) - rms_db(&quiet[16_000 * 20..]);
            let expected = (TARGET_DB - (reference - attenuation)).clamp(0.0, MAX_GAIN_DB);
            assert!((level.gain_db() - expected).abs() < 1.0, "{attenuation} dB: gain {} want {expected}", level.gain_db());
            assert!((raised - expected).abs() < 1.0, "{attenuation} dB: raised {raised} want {expected}");
        }
    }

    #[test]
    fn gain_never_clips_and_never_attenuates() {
        let mut loud = speech_like(10.0, 0.0, 3);
        loud.iter_mut().for_each(|v| *v = (*v * 12.0).clamp(-1.0, 1.0));
        let mut x = vec![0.0; 16_000 * 40];
        x.extend(&loud);
        let (out, _) = run(&x);
        assert!(out.iter().all(|v| v.abs() <= 1.0));
        for (o, i) in out.iter().zip(&x) { assert!(o.abs() + 1e-7 >= i.abs()); }
    }

    #[test]
    fn speech_after_a_long_silence_is_not_over_driven() {
        let mut x = steady_noise(40.0, -70.0, 5);
        let speech = speech_like(10.0, 0.0, 6);
        x.extend(&speech);
        let (out, _) = run(&x);
        let start = 40 * 16_000;
        // Within the first two frames of speech the gain has fallen back.
        let late = &out[start + 2 * FRAME..start + 16_000];
        let clipped = late.iter().filter(|v| v.abs() >= PEAK_LIMIT).count();
        assert_eq!(clipped, 0);
        // Raised at most to the ceiling (high-pass frame power), not +24 dB.
        assert!(rms_db(late) <= CEILING_DB + 1.0, "{} dBFS", rms_db(late));
    }

    #[test]
    fn same_input_gives_identical_output() {
        let x = speech_like(12.0, 15.0, 4);
        assert_eq!(run(&x).0, run(&x).0);
        let chunked: Vec<f32> = {
            let mut level = DetectorLevel::default();
            x.chunks(16_000).flat_map(|f| level.process(f)).collect()
        };
        let ragged: Vec<f32> = {
            let mut level = DetectorLevel::default();
            let mut out = Vec::new();
            let mut at = 0;
            for size in [1usize, 7, 511, 513, 4_000].iter().cycle() {
                if at >= x.len() { break; }
                let end = (at + size).min(x.len());
                out.extend(level.process(&x[at..end]));
                at = end;
            }
            out
        };
        assert_eq!(ragged, chunked);
        assert_eq!(chunked, run(&x).0);
    }

    #[test]
    fn original_ring_returns_exact_source_samples_in_detector_coordinates() {
        let mut ring = OriginalRing::default();
        let x: Vec<f32> = (0..OriginalRing::RETAIN as u64 + 5_000).map(|i| i as f32).collect();
        for chunk in x.chunks(FRAME) { ring.push(chunk); }
        assert_eq!(ring.get(4_999, 1), None);
        let got = ring.get(10_000, 512).unwrap();
        assert_eq!(got[0], 10_000.0);
        assert_eq!(got[511], 10_511.0);
        assert_eq!(ring.get(x.len() as u64 - 10, 11), None);
    }
}
