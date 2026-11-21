//! Speech gating for whole recordings: find where speech is, then decode only those regions of the
//! original audio, each short enough for a 30-second model window.
//!
//! The RNNoise filter is SonoJot's desktop `asr_filter.rs`, unchanged. As on the desktop, its output
//! is used only to decide where speech is; the recognizer always receives the original samples.

use crate::{asr_filter::Filter, EngineError, SAMPLE_RATE};
use serde::{Deserialize, Serialize};
use std::ops::Range;

const SAMPLES_PER_MS: usize = SAMPLE_RATE as usize / 1000;

/// How regions are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateMode {
    /// Gate on the energy of an RNNoise-denoised copy.
    #[default]
    Rnnoise,
    /// No gate: decode everything, cut only where a region exceeds the model window.
    Off,
}

/// Gate tuning. Defaults suit phone memos; every field can be set in [`crate::SonojotConfig`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GateConfig {
    pub mode: GateMode,
    /// Analysis frame length.
    pub frame_ms: u32,
    /// The speech threshold sits this far above the recording's noise floor (its 10th-percentile
    /// frame level on the denoised copy)…
    pub above_noise_db: f32,
    /// …but never below this level…
    pub floor_dbfs: f32,
    /// …and never above this one, so a recording that is speech throughout still passes.
    pub ceiling_dbfs: f32,
    /// Speech runs shorter than this are dropped (clicks, taps).
    pub min_speech_ms: u32,
    /// Pauses shorter than this stay inside one region.
    pub min_silence_ms: u32,
    /// Audio kept before and after each region, so word onsets and endings are not clipped.
    pub padding_ms: u32,
    /// Longest region handed to the recognizer; longer ones are cut at their quietest frame.
    pub max_segment_ms: u32,
    /// If the gate finds no speech but the original audio reaches this level, the audible span is
    /// decoded anyway. The gate never turns audible audio into "no speech" on its own.
    pub audible_dbfs: f32,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            mode: GateMode::Rnnoise,
            frame_ms: 30,
            above_noise_db: 10.0,
            floor_dbfs: -55.0,
            ceiling_dbfs: -38.0,
            min_speech_ms: 150,
            min_silence_ms: 600,
            padding_ms: 300,
            max_segment_ms: 28_000,
            audible_dbfs: -40.0,
        }
    }
}

impl GateConfig {
    pub fn validate(&self) -> Result<(), EngineError> {
        let bad = |detail: &str| Err(EngineError::InvalidArgument(format!("gate: {detail}")));
        if !(10..=100).contains(&self.frame_ms) {
            return bad("frame_ms must be 10–100");
        }
        if self.max_segment_ms < 1_000 || self.max_segment_ms > 120_000 {
            return bad("max_segment_ms must be 1000–120000");
        }
        if 2 * self.padding_ms >= self.max_segment_ms {
            return bad("padding_ms must be less than half of max_segment_ms");
        }
        let levels = [self.above_noise_db, self.floor_dbfs, self.ceiling_dbfs, self.audible_dbfs];
        if levels.iter().any(|x| !x.is_finite()) || self.floor_dbfs > self.ceiling_dbfs {
            return bad("levels must be finite and floor_dbfs <= ceiling_dbfs");
        }
        Ok(())
    }

    fn frame_samples(&self) -> usize {
        self.frame_ms as usize * SAMPLES_PER_MS
    }
}

/// Where to decode, in sample indices of the original audio.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GatedAudio {
    /// Ordered, disjoint regions, each at most `max_segment_ms` long.
    pub segments: Vec<Range<usize>>,
    /// The speech threshold used, or `None` when the gate was off.
    pub threshold_dbfs: Option<f32>,
    /// True when the gate heard no speech but kept the audible span (see [`GateConfig::audible_dbfs`]).
    pub fail_open: bool,
    /// `rnnoise-gate` or `off`.
    pub mode: String,
}

impl GatedAudio {
    pub fn speech_samples(&self) -> usize {
        self.segments.iter().map(|r| r.len()).sum()
    }
}

/// RNNoise speech gate with SonoJot's latency compensation. Holds no audio state between calls:
/// every call builds a fresh filter, as the desktop does per recording.
#[derive(Clone, Debug)]
pub struct AudioGateFilter {
    config: GateConfig,
}

impl AudioGateFilter {
    pub fn new(config: GateConfig) -> Result<Self, EngineError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &GateConfig {
        &self.config
    }

    /// RNNoise's fixed transport delay at 16 kHz (352 samples, 22 ms).
    pub fn latency_samples() -> usize {
        Filter::from_env().latency_samples()
    }

    /// The denoised gate copy, shifted back by the filter's delay so index `i` describes the same
    /// moment as `samples[i]`. Same length as the input; never feed it to a recognizer.
    pub fn denoise_aligned(samples: &[f32]) -> Vec<f32> {
        let mut filter = Filter::from_env();
        debug_assert!(filter.gate_only());
        let latency = filter.latency_samples();
        let mut output = filter.process(samples);
        output.extend(filter.flush());
        output.drain(..latency.min(output.len()));
        output.truncate(samples.len());
        output
    }

    /// Finds the regions of `samples` (16 kHz mono) worth decoding.
    pub fn analyze(&self, samples: &[f32]) -> GatedAudio {
        let frame = self.config.frame_samples();
        let max_segment = self.config.max_segment_ms as usize * SAMPLES_PER_MS;
        let original_db = frame_levels(samples, frame);
        if self.config.mode == GateMode::Off {
            let whole: Vec<Range<usize>> = std::iter::once(0..samples.len()).filter(|r| !r.is_empty()).collect();
            return GatedAudio {
                segments: split_long(whole, &original_db, frame, max_segment),
                threshold_dbfs: None,
                fail_open: false,
                mode: "off".into(),
            };
        }

        let mode = Filter::from_env().mode_name().to_owned();
        if samples.is_empty() {
            return GatedAudio { segments: Vec::new(), threshold_dbfs: None, fail_open: false, mode };
        }
        let gate_db = frame_levels(&Self::denoise_aligned(samples), frame);
        let threshold = threshold_for(&gate_db, &self.config);
        let speech: Vec<bool> = gate_db.iter().map(|&db| db >= threshold).collect();
        let mut regions = regions_from_frames(&speech, &self.config, samples.len());
        let mut fail_open = false;
        if regions.is_empty() {
            let audible: Vec<bool> = original_db.iter().map(|&db| db >= self.config.audible_dbfs).collect();
            if let (Some(first), Some(last)) =
                (audible.iter().position(|&a| a), audible.iter().rposition(|&a| a))
            {
                let pad = self.config.padding_ms as usize * SAMPLES_PER_MS;
                let start = (first * frame).saturating_sub(pad);
                let end = ((last + 1) * frame + pad).min(samples.len());
                regions = std::iter::once(start..end).collect();
                fail_open = true;
            }
        }
        GatedAudio {
            segments: split_long(regions, &gate_db, frame, max_segment),
            threshold_dbfs: Some(threshold),
            fail_open,
            mode,
        }
    }
}

/// RMS level of each frame in dBFS; non-finite samples count as silence. The last partial frame
/// is measured over the samples it has.
fn frame_levels(samples: &[f32], frame: usize) -> Vec<f32> {
    samples
        .chunks(frame)
        .map(|chunk| {
            let energy: f64 = chunk
                .iter()
                .map(|&x| if x.is_finite() { (x as f64).powi(2) } else { 0.0 })
                .sum::<f64>()
                / chunk.len() as f64;
            (10.0 * (energy + 1e-12).log10()) as f32
        })
        .collect()
}

fn threshold_for(levels: &[f32], config: &GateConfig) -> f32 {
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(-120.0);
    (noise + config.above_noise_db).clamp(config.floor_dbfs, config.ceiling_dbfs)
}

/// Speech frames → padded sample regions: join pauses shorter than `min_silence_ms`, drop runs
/// shorter than `min_speech_ms`, pad, clamp to the audio and merge overlaps.
fn regions_from_frames(speech: &[bool], config: &GateConfig, total: usize) -> Vec<Range<usize>> {
    let frame = config.frame_samples();
    let frames_for = |ms: u32| (ms as usize).div_ceil(config.frame_ms as usize);
    let min_gap = frames_for(config.min_silence_ms);
    let min_run = frames_for(config.min_speech_ms).max(1);

    let mut runs: Vec<Range<usize>> = Vec::new();
    let mut index = 0;
    while index < speech.len() {
        if !speech[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < speech.len() && speech[index] {
            index += 1;
        }
        match runs.last_mut() {
            Some(last) if start - last.end < min_gap => last.end = index,
            _ => runs.push(start..index),
        }
    }

    let pad = config.padding_ms as usize * SAMPLES_PER_MS;
    let mut regions: Vec<Range<usize>> = Vec::new();
    for run in runs.into_iter().filter(|run| run.len() >= min_run) {
        let start = (run.start * frame).saturating_sub(pad);
        let end = (run.end * frame + pad).min(total);
        match regions.last_mut() {
            Some(last) if start <= last.end => last.end = last.end.max(end),
            _ if start < end => regions.push(start..end),
            _ => {}
        }
    }
    regions
}

/// Cuts every region longer than `max` at the quietest frame boundary in its second half-window,
/// so a cut falls in a pause where there is one.
fn split_long(regions: Vec<Range<usize>>, levels: &[f32], frame: usize, max: usize) -> Vec<Range<usize>> {
    let mut output = Vec::with_capacity(regions.len());
    for region in regions {
        let mut start = region.start;
        while region.end - start > max {
            let earliest = start + max / 2;
            let latest = start + max;
            let first_frame = earliest.div_ceil(frame);
            let last_frame = latest / frame;
            let cut = (first_frame..last_frame)
                .filter(|&f| f < levels.len())
                .min_by(|&a, &b| levels[a].total_cmp(&levels[b]))
                .map(|f| f * frame)
                .filter(|&cut| cut > start && cut <= latest)
                .unwrap_or(latest);
            output.push(start..cut);
            start = cut;
        }
        if start < region.end {
            output.push(start..region.end);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = 16_000;

    fn noise(count: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed.max(1);
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    /// A rough voiced sound: a gliding 120–220 Hz harmonic stack with a 4 Hz syllable envelope.
    fn voiced(count: usize, amplitude: f32) -> Vec<f32> {
        let mut phase = 0.0f32;
        (0..count)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let pitch = 170.0 + 50.0 * (t * 1.3 * std::f32::consts::TAU).sin();
                phase += pitch / SR as f32;
                let harmonics: f32 = (1..=12)
                    .map(|h| (phase * h as f32 * std::f32::consts::TAU).sin() / h as f32)
                    .sum();
                let envelope = 0.6 + 0.4 * (t * 4.0 * std::f32::consts::TAU).sin();
                harmonics * envelope * amplitude * 0.5
            })
            .collect()
    }

    fn config() -> GateConfig {
        GateConfig::default()
    }

    #[test]
    fn default_config_is_valid_and_bad_configs_are_refused() {
        assert!(config().validate().is_ok());
        for broken in [
            GateConfig { frame_ms: 5, ..config() },
            GateConfig { max_segment_ms: 500, ..config() },
            GateConfig { padding_ms: 14_000, ..config() },
            GateConfig { floor_dbfs: -20.0, ceiling_dbfs: -30.0, ..config() },
            GateConfig { audible_dbfs: f32::NAN, ..config() },
        ] {
            assert!(matches!(AudioGateFilter::new(broken), Err(EngineError::InvalidArgument(_))));
        }
    }

    #[test]
    fn latency_matches_the_desktop_filter() {
        assert_eq!(AudioGateFilter::latency_samples(), 352);
    }

    #[test]
    fn aligned_copy_keeps_length_and_is_finite_for_any_input() {
        for count in [0, 1, 351, 352, 353, 480, 4_801] {
            let mut input = noise(count, 0.3, 7);
            if count > 2 {
                input[1] = f32::NAN;
                input[2] = f32::INFINITY;
            }
            let output = AudioGateFilter::denoise_aligned(&input);
            assert_eq!(output.len(), count);
            assert!(output.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn empty_and_silent_audio_have_no_regions() {
        let gate = AudioGateFilter::new(config()).unwrap();
        let empty = gate.analyze(&[]);
        assert!(empty.segments.is_empty());
        assert_eq!(empty.mode, "rnnoise-gate");
        let silent = gate.analyze(&vec![0.0; SR * 3]);
        assert!(silent.segments.is_empty());
        assert!(!silent.fail_open);
        let hiss = gate.analyze(&noise(SR * 3, 0.0005, 3));
        assert!(hiss.segments.is_empty(), "{hiss:?}");
    }

    #[test]
    fn audible_audio_is_never_dropped() {
        // Whatever RNNoise makes of a synthetic voice, an audible burst must reach the recognizer.
        let gate = AudioGateFilter::new(config()).unwrap();
        let mut audio = vec![0.0; SR];
        audio.extend(voiced(SR, 0.3));
        audio.extend(vec![0.0; SR]);
        let gated = gate.analyze(&audio);
        assert!(!gated.segments.is_empty());
        let covered = |i: usize| gated.segments.iter().any(|r| r.contains(&i));
        assert!(covered(SR + SR / 2), "{gated:?}");
        assert!(gated.segments.iter().all(|r| r.end <= audio.len()));
        assert!(!covered(SR / 4) && !covered(audio.len() - SR / 4), "{gated:?}");
    }

    #[test]
    fn regions_join_short_pauses_drop_clicks_and_pad() {
        let c = GateConfig { frame_ms: 10, min_speech_ms: 50, min_silence_ms: 100, padding_ms: 20, ..config() };
        let frame = c.frame_samples();
        let mut speech = vec![false; 200];
        speech[10..30].fill(true); // run A
        speech[35..40].fill(true); // 5-frame pause from A: joins it
        speech[100..102].fill(true); // 2-frame click: dropped
        speech[150..170].fill(true); // run B
        let regions = regions_from_frames(&speech, &c, 200 * frame);
        let pad = 20 * SAMPLES_PER_MS;
        assert_eq!(regions, vec![10 * frame - pad..40 * frame + pad, 150 * frame - pad..170 * frame + pad]);
    }

    #[test]
    fn padding_is_clamped_and_overlaps_merge() {
        let c = GateConfig { frame_ms: 10, min_speech_ms: 10, min_silence_ms: 10, padding_ms: 300, ..config() };
        let frame = c.frame_samples();
        let mut speech = vec![false; 100];
        speech[0..5].fill(true);
        speech[40..45].fill(true); // separate run, but padding overlaps the first
        speech[95..100].fill(true);
        let regions = regions_from_frames(&speech, &c, 100 * frame);
        assert_eq!(regions, vec![0..100 * frame]);
    }

    #[test]
    fn long_regions_are_cut_in_their_quietest_frame() {
        let frame = 160;
        let max = 100 * frame;
        let mut levels = vec![-20.0; 260];
        levels[80] = -70.0; // a pause inside the second half of the first window
        levels[170] = -60.0;
        let cut = split_long(std::iter::once(0..260 * frame).collect(), &levels, frame, max);
        assert_eq!(cut, vec![0..80 * frame, 80 * frame..170 * frame, 170 * frame..260 * frame]);
        assert!(cut.iter().all(|r| r.len() <= max));
        // Without any quieter frame the cut still makes progress and respects the limit.
        let flat = split_long(std::iter::once(0..1_000 * frame).collect(), &vec![-20.0; 1_000], frame, max);
        assert!(flat.iter().all(|r| !r.is_empty() && r.len() <= max));
        assert_eq!(flat.first().unwrap().start, 0);
        assert_eq!(flat.last().unwrap().end, 1_000 * frame);
        assert!(flat.windows(2).all(|w| w[0].end == w[1].start));
    }

    #[test]
    fn gate_off_decodes_everything_in_model_sized_pieces() {
        let gate = AudioGateFilter::new(GateConfig { mode: GateMode::Off, max_segment_ms: 2_000, padding_ms: 0, ..config() }).unwrap();
        let audio = noise(SR * 5, 0.1, 9);
        let gated = gate.analyze(&audio);
        assert_eq!(gated.mode, "off");
        assert_eq!(gated.threshold_dbfs, None);
        assert_eq!(gated.speech_samples(), audio.len());
        assert!(gated.segments.iter().all(|r| r.len() <= 2 * SR));
    }

    #[test]
    fn threshold_follows_noise_within_its_limits() {
        let c = config();
        assert_eq!(threshold_for(&[-90.0; 50], &c), c.floor_dbfs);
        assert_eq!(threshold_for(&[-10.0; 50], &c), c.ceiling_dbfs);
        assert_eq!(threshold_for(&[-50.0; 50], &c), -40.0);
        assert_eq!(threshold_for(&[], &c), c.floor_dbfs);
    }
}
