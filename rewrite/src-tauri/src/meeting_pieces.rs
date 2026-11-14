//! Pause-cut pieces for file and meeting recognition.
//!
//! Measured on Cody's meeting microphone (2026-11), decoding only detector
//! sections of up to 20 s lost about one word in twenty that whole-chunk
//! decoding keeps; 60 s pause-cut pieces recover nearly all of it for both
//! SonoJot Parakeet and Qwen without hurting the public meeting sets
//! (docs/research/MeetingQuietSpeech.md). Live dictation does not use this.
//!
//! Instead of decoding only the spans the speech detector marked, the whole
//! recording is cut at detector-silent pauses into pieces of at most `max`
//! seconds, and every sample between two cuts is decoded. The detector only
//! chooses cut points and lets long true silence (no detector speech for
//! `SKIP` seconds) be dropped, keeping `PAD` seconds next to speech. Pieces
//! with no detector speech at all are not decoded. All offsets are 16 kHz
//! samples from the start of the stream.

use std::ops::Range;

const RATE: u64 = 16_000;
/// Silence longer than this with no detector speech is not decoded.
const SKIP: u64 = 6 * RATE;
/// Audio kept beside speech when silence is dropped.
const PAD: u64 = RATE;
/// A cut lands in the largest detector pause in the last two thirds of a piece.
const MIN_PIECE_FRACTION: u64 = 3;

pub(crate) struct Piece {
    pub start: u64,
    pub end: u64,
    pub audio: Vec<f32>,
    pub gate: Vec<f32>,
}

pub(crate) struct Pieces {
    max: u64,
    /// Close at the first detector pause from this length (== max: never early).
    target: u64,
    step: Option<u64>,
    /// End of the speech covered by the last `early` copy.
    spoken_to: u64,
    start: u64,
    audio: Vec<f32>,
    gate: Vec<f32>,
    speech: Vec<Range<u64>>,
}

/// Longest piece. Qwen's server context (2,048 tokens) fits 60 s of audio
/// (~780 tokens) plus its 512-token output cap.
pub(crate) const DEFAULT_SECONDS: u64 = 60;

/// Production always cuts 60 s pieces. Evaluation builds read
/// SONOJOT_EVAL_PIECE_SECONDS: `0` restores detector sections, 5-300 sets
/// the piece length.
pub(crate) fn piece_seconds() -> Option<u64> {
    #[cfg(feature = "fidelity-eval")]
    if let Some(seconds) = std::env::var("SONOJOT_EVAL_PIECE_SECONDS").ok().and_then(|v| v.parse::<u64>().ok()) {
        return (5..=900).contains(&seconds).then_some(seconds);
    }
    Some(DEFAULT_SECONDS)
}

/// How live dictation cuts its audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DictationCut {
    /// A piece closes at the first detector pause once it is this long.
    pub target: u64,
    /// Hard limit: a piece still open here is cut at its longest pause.
    pub max: u64,
    /// While the piece is open, decode it again at a pause after this much new
    /// speech, so Stop only waits for the speech after the last such pause.
    /// `None` never decodes an open piece early.
    pub step: Option<u64>,
}

/// SonoJot and stock Parakeet on the CPU: close from 20 s, at most 30 s.
/// Measured November 2026 (docs/research/DictationPieceLength.md).
const PARAKEET_CUT: DictationCut = DictationCut { target: 20, max: 30, step: Some(3) };
/// Qwen3-ASR on a graphics card or Apple GPU: close from 40 s, at most 60 s,
/// the longest audio its 2,048-token server context fits with its output.
const QWEN_GPU_CUT: DictationCut = DictationCut { target: 40, max: 60, step: Some(8) };

/// Live dictation cutting for a sherpa CPU or llama-server model. `None`
/// decodes each detector section on its own (cut at every pause of 0.5 s or
/// more, at most 20 s), as Qwen on the CPU and other models still do.
/// Evaluation builds read SONOJOT_EVAL_DICTATION_CUT: `today` (sections),
/// `pieces:N` (pieces of up to N s, no early close or early decode, the
/// first measurement) or `cut:TARGET:MAX:STEP` (STEP 0 = no early decode).
pub(crate) fn dictation_cut(model_id: &str) -> Option<DictationCut> {
    #[cfg(feature = "fidelity-eval")]
    if let Ok(value) = std::env::var("SONOJOT_EVAL_DICTATION_CUT") {
        return eval_cut(&value).unwrap_or_else(|| default_cut(model_id));
    }
    default_cut(model_id)
}

fn default_cut(model_id: &str) -> Option<DictationCut> {
    if crate::llama_asr::is_llama_asr(model_id) {
        let gpu = crate::llama_asr::find(model_id).is_some_and(|m| m.runtime.provider() != "cpu");
        return gpu.then_some(QWEN_GPU_CUT);
    }
    (model_id.starts_with("parakeet-") && !model_id.ends_with("-cuda")).then_some(PARAKEET_CUT)
}

#[cfg(feature = "fidelity-eval")]
fn eval_cut(value: &str) -> Option<Option<DictationCut>> {
    if value == "today" || value.starts_with("pause:") { return Some(None); }
    if let Some(n) = value.strip_prefix("pieces:").and_then(|n| n.parse::<u64>().ok()).filter(|n| (5..=900).contains(n)) {
        return Some(Some(DictationCut { target: n, max: n, step: None }));
    }
    let parts: Vec<u64> = value.strip_prefix("cut:")?.split(':').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    let [target, max, step] = parts[..] else { return None };
    (5 <= target && target <= max && max <= 900).then_some(Some(DictationCut { target, max, step: (step > 0).then_some(step) }))
}

/// Shortest pause the speech detector cuts at. Evaluation builds read
/// SONOJOT_EVAL_DICTATION_CUT=`pause:S` (S = 0.3..5 s).
pub(crate) fn detector_min_silence() -> f32 {
    #[cfg(feature = "fidelity-eval")]
    if let Some(s) = std::env::var("SONOJOT_EVAL_DICTATION_CUT").ok().as_deref().and_then(|v| v.strip_prefix("pause:")) {
        if let Some(s) = s.parse::<f32>().ok().filter(|s| (0.3..=5.0).contains(s)) { return s; }
    }
    0.5
}

impl Pieces {
    pub(crate) fn new(max_seconds: u64) -> Self {
        Self { max: max_seconds * RATE, target: max_seconds * RATE, step: None, spoken_to: 0,
            start: 0, audio: Vec::new(), gate: Vec::new(), speech: Vec::new() }
    }

    /// Live dictation pieces: see `DictationCut`.
    pub(crate) fn dictation(cut: DictationCut) -> Self {
        let max = cut.max.max(1) * RATE;
        Self { max, target: (cut.target * RATE).min(max), step: cut.step.map(|s| s.max(1) * RATE),
            ..Self::new(cut.max) }
    }

    /// First sample of the open piece.
    pub(crate) fn start(&self) -> u64 { self.start }

    /// The open piece up to the middle of the current detector pause, to decode
    /// now while the speaker continues: only when the detector is in a pause and
    /// at least `step` of speech arrived since the last such copy. A piece that
    /// later closes at the same point, or Stop, can reuse that text.
    pub(crate) fn early(&mut self, speaking: bool) -> Option<Piece> {
        let step = self.step?;
        let last_end = self.speech.last()?.end;
        let end = self.end();
        if speaking || end <= last_end || last_end < self.start.max(self.spoken_to) + step { return None; }
        self.spoken_to = last_end;
        let cut = pause_middle(last_end, end);
        let len = (cut - self.start) as usize;
        Some(Piece { start: self.start, end: cut, audio: self.audio[..len].to_vec(), gate: self.gate[..len].to_vec() })
    }

    /// Close the open piece at `cut` whose text the caller already has.
    pub(crate) fn close_at(&mut self, cut: u64) -> Option<Piece> {
        self.emit(cut)
    }

    fn end(&self) -> u64 { self.start + self.audio.len() as u64 }

    /// Original samples and the detector's copy of the same span.
    pub(crate) fn push(&mut self, audio: &[f32], gate: &[f32]) {
        debug_assert_eq!(audio.len(), gate.len());
        self.audio.extend(audio);
        self.gate.extend(gate);
    }

    /// A span the detector marked as speech.
    pub(crate) fn speech(&mut self, range: Range<u64>) {
        let range = range.start.max(self.start)..range.end.min(self.end());
        if range.start >= range.end { return; }
        match self.speech.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => self.speech.push(range),
        }
    }

    /// Pieces ready to decode. `speaking` means the detector is inside an
    /// utterance it has not emitted yet, so audio after the last emitted
    /// speech may still be speech and cannot hold a cut or be dropped.
    pub(crate) fn poll(&mut self, speaking: bool, finish: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        let open = speaking && !finish;
        loop {
            let end = self.end();
            let Some(last_end) = self.speech.last().map(|r| r.end) else {
                // No speech yet: drop leading silence, keeping PAD before what follows.
                if finish { self.drop_to(end); } else if !open && end - self.start > SKIP { self.drop_to(end - PAD); }
                break;
            };
            // Dictation: close at the first pause once the piece is long enough.
            if !open && !finish && self.target < self.max && end > last_end && last_end - self.start >= self.target {
                if let Some(piece) = self.emit(pause_middle(last_end, end)) { out.push(piece); }
                continue;
            }
            let closing = finish || (!open && end - last_end >= SKIP);
            let cut_at = if closing { (last_end + PAD).min(end) } else { end };
            if cut_at - self.start > self.max || (!closing && cut_at - self.start == self.max) {
                let known = if open { last_end } else { end };
                let cut = self.choose_cut(known);
                if let Some(piece) = self.emit(cut) { out.push(piece); }
                continue;
            }
            if closing {
                if let Some(piece) = self.emit(cut_at) { out.push(piece); }
                let resume = if finish { end } else { end - PAD };
                self.drop_to(resume.max(self.start));
                continue;
            }
            break;
        }
        out
    }

    /// Middle of the longest detector pause whose middle lies in the last two
    /// thirds of the piece; a hard cut at `max` only when there is no pause.
    fn choose_cut(&self, known: u64) -> u64 {
        let low = self.start + self.max / MIN_PIECE_FRACTION;
        let high = self.start + self.max;
        let mut best: Option<(u64, u64)> = None;
        let mut gap_start = self.start;
        let gaps = self.speech.iter().map(|r| (r.start, r.end)).chain(std::iter::once((known, known)));
        for (speech_start, speech_end) in gaps {
            if speech_start > gap_start {
                let middle = gap_start + (speech_start - gap_start) / 2;
                let length = speech_start - gap_start;
                if (low..=high).contains(&middle) && best.is_none_or(|(l, _)| length > l) {
                    best = Some((length, middle));
                }
            }
            gap_start = gap_start.max(speech_end);
        }
        best.map_or(high, |(_, middle)| middle)
    }

    fn emit(&mut self, cut: u64) -> Option<Piece> {
        let cut = cut.clamp(self.start, self.end());
        if cut == self.start { return None; }
        let has_speech = self.speech.iter().any(|r| r.start < cut);
        let len = (cut - self.start) as usize;
        let piece = Piece { start: self.start, end: cut,
            audio: self.audio.drain(..len).collect(), gate: self.gate.drain(..len).collect() };
        self.start = cut;
        self.speech.retain_mut(|r| { r.start = r.start.max(cut); r.start < r.end });
        has_speech.then_some(piece)
    }

    fn drop_to(&mut self, at: u64) {
        let at = at.clamp(self.start, self.end());
        let len = (at - self.start) as usize;
        self.audio.drain(..len);
        self.gate.drain(..len);
        self.start = at;
        self.speech.retain_mut(|r| { r.start = r.start.max(at); r.start < r.end });
    }
}

/// Middle of the pause heard so far after speech ending at `last_end`.
fn pause_middle(last_end: u64, end: u64) -> u64 {
    last_end + (end - last_end) / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(p: &mut Pieces, seconds: u64) {
        let n = (seconds * RATE) as usize;
        let audio: Vec<f32> = (0..n).map(|i| (p.end() + i as u64) as f32).collect();
        p.push(&audio, &audio);
    }

    fn s(a: f64, b: f64) -> Range<u64> { (a * RATE as f64) as u64..(b * RATE as f64) as u64 }

    #[test]
    fn every_sample_between_cuts_is_decoded_once_and_cuts_land_in_pauses() {
        let mut p = Pieces::new(30);
        let mut pieces = Vec::new();
        // Speech every 4 s for 2.5 s, 100 s long: pauses of 1.5 s.
        for second in 0..100u64 {
            feed(&mut p, 1);
            // Like Silero: a span is emitted half a second after it ends, and
            // the detector reports "speaking" while inside one.
            if second % 4 == 2 { p.speech(s(second as f64 - 2.0, second as f64 + 0.5)); }
            let speaking = matches!((second + 1) % 4, 1 | 2);
            pieces.extend(p.poll(speaking, false));
        }
        pieces.extend(p.poll(false, true));
        assert!(pieces.len() >= 4);
        let mut at = pieces[0].start;
        for piece in &pieces {
            assert_eq!(piece.start, at, "no audio between pieces is skipped");
            assert_eq!(piece.audio.len() as u64, piece.end - piece.start);
            assert_eq!(piece.audio[0], piece.start as f32);
            assert!(piece.end - piece.start <= 30 * RATE);
            at = piece.end;
            let in_pause = (piece.end % (4 * RATE)) as f64 / RATE as f64;
            if piece.end != pieces.last().unwrap().end {
                assert!(in_pause > 2.5 && in_pause < 4.0, "cut at {in_pause} s into the cycle");
            }
        }
    }

    #[test]
    fn long_silence_is_skipped_but_kept_beside_speech_and_silent_pieces_are_not_decoded() {
        let mut p = Pieces::new(60);
        let mut pieces = Vec::new();
        feed(&mut p, 20); // leading silence
        pieces.extend(p.poll(false, false));
        feed(&mut p, 5); p.speech(s(20.0, 25.0));
        feed(&mut p, 30); // long silence
        pieces.extend(p.poll(false, false));
        feed(&mut p, 5); p.speech(s(55.0, 60.0));
        pieces.extend(p.poll(false, true));
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].start, 19 * RATE, "one second kept before speech");
        assert_eq!(pieces[0].end, 26 * RATE, "one second kept after speech");
        assert_eq!(pieces[1].start, 54 * RATE);
        assert_eq!(pieces[1].end, 60 * RATE);
        let mut silent = Pieces::new(60);
        feed(&mut silent, 90);
        assert!(silent.poll(false, false).is_empty());
        assert!(silent.poll(false, true).is_empty());
    }

    #[test]
    fn open_speech_is_never_cut_or_dropped_and_continuous_speech_is_hard_cut_at_max() {
        let mut p = Pieces::new(30);
        feed(&mut p, 2);
        p.speech(s(0.0, 2.0));
        feed(&mut p, 10);
        assert!(p.poll(true, false).is_empty(), "detector still inside an utterance");
        let mut q = Pieces::new(30);
        feed(&mut q, 45);
        q.speech(s(0.0, 45.0));
        let pieces = q.poll(false, true);
        assert_eq!(pieces.iter().map(|p| p.end - p.start).collect::<Vec<_>>(), [30 * RATE, 15 * RATE]);
    }

    /// Dictation-like audio: 2.5 s of speech then a 1.5 s pause, repeated;
    /// spans are reported half a second after they end, as Silero does.
    fn dictate(p: &mut Pieces, seconds: u64, mut each: impl FnMut(&mut Pieces, bool)) {
        for second in 0..seconds {
            feed(p, 1);
            if second % 4 == 2 { p.speech(s(second as f64 - 2.0, second as f64 + 0.5)); }
            let speaking = matches!((second + 1) % 4, 1 | 2);
            each(p, speaking);
        }
    }

    const CUT: DictationCut = DictationCut { target: 20, max: 30, step: Some(3) };

    #[test]
    fn dictation_pieces_close_at_the_first_pause_from_the_target_and_never_pass_the_limit() {
        let mut p = Pieces::dictation(CUT);
        let mut pieces = Vec::new();
        dictate(&mut p, 120, |p, speaking| pieces.extend(p.poll(speaking, false)));
        pieces.extend(p.poll(false, true));
        let mut at = pieces[0].start;
        for piece in &pieces {
            assert_eq!(piece.start, at, "no audio between pieces is skipped");
            at = piece.end;
            let len = piece.end - piece.start;
            assert!(len <= 30 * RATE);
            if piece.end != pieces.last().unwrap().end {
                assert!(len >= 20 * RATE && len < 25 * RATE, "closed {} s in", len / RATE);
                let in_pause = (piece.end % (4 * RATE)) as f64 / RATE as f64;
                assert!(in_pause > 2.5 && in_pause < 4.0, "cut at {in_pause} s into the cycle");
            }
        }
        // Unbroken speech is still cut at the hard limit.
        let mut q = Pieces::dictation(CUT);
        feed(&mut q, 45);
        q.speech(s(0.0, 45.0));
        let lens: Vec<u64> = q.poll(false, true).iter().map(|p| p.end - p.start).collect();
        assert_eq!(lens, [30 * RATE, 15 * RATE]);
    }

    #[test]
    fn early_copies_come_at_pauses_after_enough_new_speech_and_stop_reuses_them() {
        let mut p = Pieces::dictation(CUT);
        let mut early = Vec::new();
        dictate(&mut p, 16, |p, speaking| {
            assert!(p.poll(speaking, false).is_empty());
            if let Some(piece) = p.early(speaking) {
                assert!(!speaking, "never while the speaker is mid-utterance");
                early.push(piece);
            }
        });
        // Speech ends at 2.5, 6.5, 10.5, 14.5 s; a copy needs 3 s of new speech,
        // so the first comes at the 6.5 s pause.
        assert_eq!(early.len(), 3, "{:?}", early.iter().map(|e| e.end).collect::<Vec<_>>());
        for copy in &early {
            assert_eq!(copy.start, 0);
            assert_eq!(copy.audio.len() as u64, copy.end);
            let in_pause = (copy.end % (4 * RATE)) as f64 / RATE as f64;
            assert!(in_pause > 2.5 && in_pause < 4.0);
        }
        // Stop: close at the last copy; only audio after it remains to decode.
        let last = early.last().unwrap().end;
        let closed = p.close_at(last).unwrap();
        assert_eq!((closed.start, closed.end), (0, last));
        let rest = p.poll(false, true);
        assert!(rest.iter().all(|piece| piece.start >= last));
        // Meeting pieces never decode an open piece early.
        let mut meeting = Pieces::new(60);
        dictate(&mut meeting, 16, |p, speaking| assert!(p.early(speaking).is_none()));
    }

    #[test]
    fn dictation_cut_follows_the_model() {
        assert_eq!(default_cut("parakeet-tdt-0.6b-v2-sonojot-ft2"), Some(CUT));
        assert_eq!(default_cut("parakeet-tdt-0.6b-v2"), Some(CUT));
        assert_eq!(default_cut("parakeet-tdt-0.6b-v2-fp16-cuda"), None, "GPU Parakeet keeps 20 s sections");
        let qwen = default_cut(crate::llama_asr::MODEL_ID).unwrap();
        assert_eq!((qwen.target, qwen.max), (40, 60));
        assert_eq!(default_cut(crate::llama_asr::METAL_Q8_MODEL_ID).map(|c| c.max), Some(60));
        assert_eq!(default_cut(crate::llama_asr::CPU_MODEL_ID), None, "Qwen on the CPU keeps its windows");
        assert_eq!(default_cut("whisper-small"), None);
    }
}
