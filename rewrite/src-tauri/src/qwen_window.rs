//! Bounded Qwen windows in original 16 kHz sample coordinates.
use std::{collections::VecDeque, ops::Range};

pub const RATE: usize = 16_000;
pub const PRE: u64 = 1_600;
pub const POST: u64 = 3_200;
pub const GAP: u64 = 9_600;
pub const BREAK: u64 = 24_000;
pub const HARD: u64 = 320_000;
const RETAIN: usize = 22 * RATE;

pub fn target_seconds() -> usize {
    target_seconds_for_model("")
}

pub fn target_seconds_for_model(model_id: &str) -> usize {
    let default = default_target_seconds(model_id);
    #[cfg(feature = "fidelity-eval")]
    { return select_target(std::env::var("SONOJOT_EVAL_QWEN_WINDOW_SECONDS").ok().as_deref(), default); }
    #[cfg(not(feature = "fidelity-eval"))]
    { default }
}

fn default_target_seconds(model_id: &str) -> usize {
    // The CPU decoder needs a shorter final window to meet the stop latency bound.
    if model_id == crate::llama_asr::CPU_MODEL_ID { 8 } else { 15 }
}

fn select_target(value: Option<&str>, default: usize) -> usize {
    match value { Some("0") => 0, Some("8") => 8, Some("15") => 15, _ => default }
}

pub fn vad_threshold() -> f32 {
    #[cfg(feature = "fidelity-eval")]
    { return select_threshold(std::env::var("SONOJOT_EVAL_VAD_THRESHOLD").ok().as_deref()); }
    #[cfg(not(feature = "fidelity-eval"))]
    { 0.50 }
}

fn select_threshold(value: Option<&str>) -> f32 {
    match value { Some("0.25") => 0.25, Some("0.40" | "0.4") => 0.40, _ => 0.50 }
}

#[derive(Debug)]
pub struct Window {
    pub audio: Vec<f32>,
    pub gate_audio: Vec<f32>,
    /// The source ranges, not timestamps in the gap-trimmed decoder input.
    pub ranges: Vec<Range<u64>>,
    pub start: u64,
    pub end: u64,
    pub reason: &'static str,
    pub recovered_tail: bool,
}

pub struct Assembler {
    audio: VecDeque<f32>,
    gate_audio: VecDeque<f32>,
    base: u64,
    end: u64,
    floor: u64,
    speech: Vec<Range<u64>>,
    ready: Vec<Window>,
    target: u64,
    retention: usize,
    recovered: bool,
    last_speech_end: u64,
}

impl Assembler {
    pub fn new(seconds: usize) -> Self {
        Self { audio: VecDeque::new(), gate_audio: VecDeque::new(), base: 0, end: 0,
            floor: 0, speech: Vec::new(), ready: Vec::new(), target: seconds as u64 * RATE as u64,
            // The existing baseline evaluator also supports 28-second sections.
            retention: if seconds == 0 { 30 * RATE } else { RETAIN },
            recovered: false, last_speech_end: 0 }
    }

    /// Feed bounded detector-sized blocks, then collect detector segments and
    /// call poll. Gate placement uses the paired ring without replacing ASR audio.
    pub fn append(&mut self, audio: &[f32], gate_audio: &[f32]) -> Result<(), String> {
        if audio.len() != gate_audio.len() { return Err("Qwen audio/gate alignment changed.".into()); }
        self.end = self.end.checked_add(audio.len() as u64).ok_or("Qwen audio offset overflowed.")?;
        self.audio.extend(audio);
        self.gate_audio.extend(gate_audio);
        let discard = self.audio.len().saturating_sub(self.retention);
        if discard > 0 {
            self.audio.drain(..discard);
            self.gate_audio.drain(..discard);
            self.base += discard as u64;
        }
        Ok(())
    }

    pub fn end(&self) -> u64 { self.end }

    pub fn exact(&self, start: u64, end: u64) -> Result<Window, String> {
        let end = end.min(self.end);
        if start < self.base || start > end { return Err("Qwen detector exceeded retained audio.".into()); }
        let range = start..end;
        Ok(Window { audio: self.copy(&self.audio, range.clone()),
            gate_audio: self.copy(&self.gate_audio, range.clone()), ranges: vec![range],
            start, end, reason: "baseline", recovered_tail: false })
    }

    pub fn hard_due(&self) -> bool {
        self.speech.first().is_some_and(|r| self.end >= self.start(r.start) + HARD)
    }

    fn start(&self, speech_start: u64) -> u64 {
        speech_start.saturating_sub(PRE).max(self.floor).max(self.base)
    }

    pub fn segment(&mut self, range: Range<u64>, recovered: bool) -> Result<(), String> {
        let mut start = range.start.max(self.floor);
        let end = range.end.min(self.end);
        if start >= end { return Ok(()); }
        if start < self.base { return Err("Qwen detector exceeded retained audio.".into()); }
        self.last_speech_end = self.last_speech_end.max(end);
        if !recovered && self.speech.last().is_some_and(|last| start.saturating_sub(last.end) >= BREAK) {
            self.emit("gap", None)?;
            start = start.max(self.floor);
        }
        while start < end {
            let first = self.speech.first().map_or(start, |r| r.start);
            let limit = self.start(first) + HARD;
            if start >= limit {
                self.emit("hard_cap", Some(limit))?;
                continue;
            }
            let part_end = end.min(limit);
            if let Some(last) = self.speech.last_mut().filter(|last| start <= last.end) {
                last.end = last.end.max(part_end);
            } else {
                self.speech.push(start..part_end);
            }
            self.recovered |= recovered;
            start = part_end;
            if part_end == limit { self.emit("hard_cap", Some(limit))?; }
        }
        Ok(())
    }

    pub fn poll(&mut self, finish: bool) -> Result<Vec<Window>, String> {
        self.poll_active(finish, false)
    }

    pub fn poll_active(&mut self, finish: bool, speaking: bool) -> Result<Vec<Window>, String> {
        if let (Some(first), Some(last)) = (self.speech.first(), self.speech.last()) {
            let reason = if finish { Some("stop") }
                else if self.hard_due() { Some("hard_cap") }
                else if !speaking && self.end >= last.end + BREAK { Some("gap") }
                else if last.end + POST - self.start(first.start) >= self.target && self.end >= last.end + POST { Some("target") }
                else { None };
            if let Some(reason) = reason { self.emit(reason, None)?; }
        }
        Ok(std::mem::take(&mut self.ready))
    }

    /// Only real audio after the last detector emission can qualify a missed tail.
    pub fn tail(&self) -> (u64, Vec<f32>) {
        let start = self.last_speech_end.max(self.floor).max(self.base);
        (start, self.copy(&self.gate_audio, start..self.end))
    }

    fn copy(&self, ring: &VecDeque<f32>, range: Range<u64>) -> Vec<f32> {
        ring.iter().skip((range.start - self.base) as usize)
            .take((range.end - range.start) as usize).copied().collect()
    }

    fn emit(&mut self, reason: &'static str, cap: Option<u64>) -> Result<(), String> {
        let Some(first) = self.speech.first() else { return Ok(()); };
        let start = self.start(first.start);
        let end = (self.speech.last().unwrap().end + POST).min(self.end)
            .min(cap.unwrap_or(start + HARD));
        if start < self.base || end > self.end { return Err("Qwen window left retained audio.".into()); }
        let mut ranges = Vec::new();
        let mut from = start;
        for pair in self.speech.windows(2) {
            if pair[1].start.saturating_sub(pair[0].end) > GAP {
                let left = pair[0].end + GAP / 2;
                ranges.push(from..left);
                from = pair[1].start - GAP / 2;
            }
        }
        if from < end { ranges.push(from..end); }
        let audio = ranges.iter().flat_map(|r| self.copy(&self.audio, r.clone())).collect();
        let gate_audio = ranges.iter().flat_map(|r| self.copy(&self.gate_audio, r.clone())).collect();
        self.ready.push(Window { audio, gate_audio, ranges, start, end, reason, recovered_tail: self.recovered });
        self.floor = end;
        self.speech.clear();
        self.recovered = false;
        Ok(())
    }
}

/// Segment a stream of detector probabilities using the same 250/500 ms
/// qualification as Windows Silero. Recurrence belongs to the platform detector.
#[derive(Default)]
pub struct Detector {
    end: u64,
    candidate: Option<u64>,
    last_positive: u64,
    active: bool,
}

impl Detector {
    pub fn speaking(&self) -> bool { self.candidate.is_some() }

    pub fn observe(&mut self, probability: f32, count: usize) -> Option<Range<u64>> {
        let start = self.end;
        self.end += count as u64;
        if probability >= vad_threshold() {
            let candidate = *self.candidate.get_or_insert(start);
            self.last_positive = self.end;
            self.active |= self.end - candidate >= 4_000;
        } else if !self.active {
            self.candidate = None;
        } else if self.end - self.last_positive >= 8_000 {
            return self.finish();
        }
        None
    }

    pub fn finish(&mut self) -> Option<Range<u64>> {
        let range = self.candidate.take().filter(|_| self.active).map(|s| s..self.last_positive);
        self.active = false;
        range
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fill(w: &mut Assembler, end: u64) {
        while w.end < end {
            let to = (w.end + 512).min(end);
            let samples: Vec<_> = (w.end..to).map(|s| s as f32).collect();
            w.append(&samples, &samples).unwrap();
        }
    }

    #[test]
    fn padding_uses_real_audio_and_internal_gap_removes_only_its_middle() {
        let mut w = Assembler::new(8);
        fill(&mut w, 10 * RATE as u64);
        w.segment(3_200..32_000, false).unwrap();
        w.segment(48_000..145_600, false).unwrap();
        let result = w.poll(false).unwrap().pop().unwrap();
        assert_eq!(result.reason, "target");
        assert_eq!(result.ranges, vec![1_600..36_800, 43_200..148_800]);
        assert_eq!((result.start, result.end), (1_600, 148_800));
        assert_eq!(result.audio[0], 1_600.0);
        assert_eq!(*result.audio.last().unwrap(), 148_799.0);
        assert_eq!(result.audio, result.gate_audio);
    }

    #[test]
    fn target_waits_for_real_postroll_stop_uses_only_available_samples() {
        let mut w = Assembler::new(8);
        fill(&mut w, 128_000);
        w.segment(0..128_000, false).unwrap();
        assert!(w.poll(false).unwrap().is_empty());
        fill(&mut w, 129_000);
        let result = w.poll(true).unwrap().pop().unwrap();
        assert_eq!(result.audio.len(), 129_000);
        assert_eq!(result.reason, "stop");
        assert!(w.poll(true).unwrap().is_empty());
    }

    #[test]
    fn long_pause_emits_without_waiting_for_next_speech() {
        let mut w = Assembler::new(15);
        fill(&mut w, 40_000);
        w.segment(0..16_000, false).unwrap();
        assert_eq!(w.poll(false).unwrap()[0].reason, "gap");
    }

    #[test]
    fn active_next_utterance_prevents_a_false_gap_and_target_waits_for_boundary() {
        let mut w = Assembler::new(8);
        fill(&mut w, 32_000);
        w.segment(0..16_000, false).unwrap();
        fill(&mut w, 9 * RATE as u64);
        assert!(w.poll_active(false, true).unwrap().is_empty());
        w.segment(24_000..140_000, false).unwrap();
        let result = w.poll(false).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].reason, "target");
        assert_eq!(result[0].start, 0);
    }

    #[test]
    fn a_reported_long_gap_separates_windows_and_new_pre_roll_has_original_offset() {
        let mut w = Assembler::new(15);
        fill(&mut w, 64_000);
        w.segment(0..16_000, false).unwrap();
        w.segment(40_000..50_000, false).unwrap();
        let result = w.poll(true).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].reason, "gap");
        assert_eq!((result[1].start, result[1].end), (38_400, 53_200));
        assert_eq!(result[1].audio[0], 38_400.0);
    }

    #[test]
    fn hard_cap_splits_a_long_emission_without_losing_or_repeating_samples() {
        let mut w = Assembler::new(15);
        fill(&mut w, HARD + 1_024);
        w.segment(0..HARD + 1_024, false).unwrap();
        let result = w.poll(true).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].audio.len(), HARD as usize);
        assert_eq!(result[1].start, HARD);
        assert_eq!(result[1].audio[0], HARD as f32);
        assert_eq!(result.iter().map(|r| r.audio.len()).sum::<usize>(), HARD as usize + 1_024);
    }

    #[test]
    fn recovered_short_tail_merges_into_open_window_and_keeps_offsets() {
        let mut w = Assembler::new(8);
        fill(&mut w, 64_000);
        w.segment(0..32_000, false).unwrap();
        let (at, tail) = w.tail();
        assert_eq!(at, 32_000);
        assert_eq!(tail[0], 32_000.0);
        w.segment(48_000..50_000, true).unwrap();
        let result = w.poll(true).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].recovered_tail);
        assert_eq!(result[0].end, 53_200);
    }

    #[test]
    fn paired_gate_audio_is_bounded_and_sessions_have_no_shared_samples() {
        let mut a = Assembler::new(8);
        for _ in 0..2_000 { a.append(&[0.5; 512], &[0.1; 512]).unwrap(); }
        assert_eq!(a.audio.len(), RETAIN);
        a.segment(a.end - 4_000..a.end, false).unwrap();
        let result = a.poll(true).unwrap();
        assert!(result[0].audio.iter().all(|&v| v == 0.5));
        assert!(result[0].gate_audio.iter().all(|&v| v == 0.1));
        let mut b = Assembler::new(8);
        assert_eq!(b.end(), 0);
        assert!(b.poll(true).unwrap().is_empty());
        assert!(b.tail().1.is_empty());
    }

    #[test]
    fn baseline_retains_the_supported_28_second_evaluation_section() {
        let mut windows = Assembler::new(0);
        fill(&mut windows, (29 * RATE) as u64);
        let section = windows.exact(0, (28 * RATE) as u64).unwrap();
        assert_eq!(section.audio.len(), 28 * RATE);
        assert_eq!(section.audio[0], 0.0);
        assert_eq!(section.audio.last(), Some(&((28 * RATE - 1) as f32)));
        assert!(windows.audio.len() <= 30 * RATE);
    }

    #[test]
    fn detector_keeps_short_tail_untriggered_and_resets_between_sessions() {
        let mut detector = Detector::default();
        for _ in 0..4 { assert!(detector.observe(0.9, 512).is_none()); }
        assert!(detector.finish().is_none());
        for _ in 0..8 { detector.observe(0.9, 512); }
        for _ in 0..15 { assert!(detector.observe(0.0, 512).is_none()); }
        assert_eq!(detector.observe(0.0, 512), Some(2_048..6_144));
        assert!(Detector::default().finish().is_none());
    }

    #[test]
    fn overrides_are_allowlisted() {
        assert_eq!(select_target(Some("0"), 15), 0);
        assert_eq!(select_target(Some("15"), 8), 15);
        assert_eq!(select_target(Some("8"), 15), 8);
        assert_eq!(select_target(Some("99"), 15), 15);
        assert_eq!(select_target(None, 8), 8);
        assert_eq!(select_threshold(Some("0.25")), 0.25);
        assert_eq!(select_threshold(Some("0.40")), 0.4);
        assert_eq!(select_threshold(Some("0.50")), 0.5);
        assert_eq!(select_threshold(Some("NaN")), 0.50);
    }

    #[test]
    fn cpu_and_accelerated_models_use_their_measured_window_targets() {
        assert_eq!(default_target_seconds(crate::llama_asr::CPU_MODEL_ID), 8);
        assert_eq!(default_target_seconds(crate::llama_asr::MODEL_ID), 15);
        assert_eq!(default_target_seconds(crate::llama_asr::METAL_Q8_MODEL_ID), 15);
        assert_eq!(default_target_seconds(crate::llama_asr::METAL_Q4_MODEL_ID), 15);
    }
}
