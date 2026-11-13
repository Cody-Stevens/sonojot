//! Which words and gaps to mark: a pure function of evidence and the user's
//! sensitivity. All numbers live in [`Thresholds`]; tune them there.

use super::evidence::{AltSource, Alternative, Evidence};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sensitivity {
    Fewer,
    #[default]
    Balanced,
    More,
}

impl Sensitivity {
    fn index(self) -> usize {
        match self {
            Self::Fewer => 0,
            Self::Balanced => 1,
            Self::More => 2,
        }
    }
}

/// Every tunable number of the decision, indexed by sensitivity
/// (Fewer, Balanced, More). Measured values replace these later.
#[derive(Clone, Debug, PartialEq)]
pub struct Thresholds {
    /// A word is doubtful below this recognizer confidence.
    pub word_confidence: [f32; 3],
    /// A word with unknown confidence is doubtful when it is this close to a
    /// personal vocabulary entry (and only at these sensitivities).
    pub vocabulary_only: [bool; 3],
    /// A gap needs at least this many seconds of speech without words.
    pub gap_seconds: [f32; 3],
    /// Share of a gap's frames that must carry speech.
    pub gap_speech_share: f32,
    /// At most this share of the words may be marked (gaps count as words).
    pub max_share: f32,
    /// Hard cap on marks for one dictation, whatever its length.
    pub max_marks: usize,
    /// Ranking weight of a gap against a word with confidence 0.
    pub gap_weight: f32,
    /// Ranking score of a vocabulary-only doubt.
    pub vocabulary_weight: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            word_confidence: [0.35, 0.55, 0.72],
            vocabulary_only: [false, true, true],
            gap_seconds: [1.2, 0.9, 0.7],
            gap_speech_share: 0.5,
            max_share: 0.06,
            max_marks: 6,
            gap_weight: 0.7,
            vocabulary_weight: 0.5,
        }
    }
}

pub fn thresholds() -> Thresholds {
    Thresholds::default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkKind {
    Word,
    Gap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub kind: MarkKind,
    /// Byte range in the pasted text. A gap is an empty range at the end of
    /// the word before it (or at 0 before the first word).
    pub start: usize,
    pub end: usize,
    /// The word as pasted; for a gap, the word before it (may be empty).
    pub heard: String,
    pub alternatives: Vec<Alternative>,
    /// Seconds on the dictation audio timeline, when known.
    pub audio: Option<(f32, f32)>,
    /// Higher is more doubtful. Used for ranking only.
    pub score: f32,
}

/// The marks for one dictation, in text order. Never more than
/// `max(1, words * max_share)` and never more than `max_marks`; when more
/// qualify, only the most doubtful are kept.
pub fn decide(evidence: &Evidence, sensitivity: Sensitivity, t: &Thresholds) -> Vec<Mark> {
    let s = sensitivity.index();
    let mut marks = Vec::new();
    for (i, w) in evidence.words.iter().enumerate() {
        let word = evidence.word(i);
        if word.chars().all(|c| !c.is_alphanumeric()) {
            continue;
        }
        let score = match w.confidence {
            Some(c) if c < t.word_confidence[s] => 1.0 - c,
            Some(_) => continue,
            None if t.vocabulary_only[s] && w.alternatives.iter().any(|a| a.source == AltSource::Vocabulary) => t.vocabulary_weight,
            None => continue,
        };
        marks.push(Mark {
            kind: MarkKind::Word,
            start: w.start,
            end: w.end,
            heard: word.to_owned(),
            alternatives: w.alternatives.clone(),
            audio: w.time,
            score,
        });
    }
    for g in &evidence.gaps {
        if g.time.1 - g.time.0 < t.gap_seconds[s] || g.speech_share < t.gap_speech_share {
            continue;
        }
        let (at, heard) = match g.after {
            Some(i) if i < evidence.words.len() => (evidence.words[i].end, evidence.word(i).to_owned()),
            Some(_) => continue,
            None => (evidence.words.first().map_or(0, |w| w.start), String::new()),
        };
        marks.push(Mark {
            kind: MarkKind::Gap,
            start: at,
            end: at,
            heard,
            alternatives: g.alternatives.clone(),
            audio: Some(g.time),
            score: t.gap_weight * g.speech_share,
        });
    }
    let cap = cap(evidence.words.len(), t);
    if marks.len() > cap {
        marks.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.start.cmp(&b.start)));
        marks.truncate(cap);
    }
    marks.sort_by_key(|m| (m.start, m.kind == MarkKind::Word));
    marks
}

fn cap(words: usize, t: &Thresholds) -> usize {
    ((words as f32 * t.max_share).floor() as usize).max(1).min(t.max_marks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::evidence::{from_raw, GapEvidence, RawWord};

    fn evidence(text: &str, confidence: &[f32]) -> Evidence {
        let words: Vec<RawWord> = crate::trust::evidence::word_ranges(text)
            .iter()
            .zip(confidence)
            .enumerate()
            .map(|(i, ((a, b), c))| RawWord { word: text[*a..*b].into(), confidence: Some(*c), time: Some((i as f32 * 0.3, i as f32 * 0.3 + 0.3)) })
            .collect();
        from_raw(text, &words)
    }

    fn sixty_words(low: &[(usize, f32)]) -> Evidence {
        let text = (0..60).map(|i| format!("word{i}")).collect::<Vec<_>>().join(" ");
        let mut c = vec![0.97f32; 60];
        for (i, v) in low {
            c[*i] = *v;
        }
        evidence(&text, &c)
    }

    #[test]
    fn confident_text_gets_no_marks() {
        assert!(decide(&sixty_words(&[]), Sensitivity::More, &Thresholds::default()).is_empty());
    }

    #[test]
    fn sensitivity_orders_the_thresholds() {
        let e = sixty_words(&[(5, 0.2), (20, 0.5), (40, 0.65)]);
        let t = Thresholds::default();
        let count = |s| decide(&e, s, &t).len();
        assert_eq!(count(Sensitivity::Fewer), 1);
        assert_eq!(count(Sensitivity::Balanced), 2);
        assert_eq!(count(Sensitivity::More), 3);
    }

    #[test]
    fn a_bad_dictation_is_never_painted_yellow() {
        // Every word doubtful: only the cap survives, the weakest first.
        let low: Vec<(usize, f32)> = (0..60).map(|i| (i, 0.1 + i as f32 * 0.001)).collect();
        let marks = decide(&sixty_words(&low), Sensitivity::More, &Thresholds::default());
        assert_eq!(marks.len(), 3, "6% of 60 words");
        assert_eq!(marks[0].heard, "word0");
        assert!(marks.windows(2).all(|m| m[0].start < m[1].start), "text order");
        let short = evidence("one two three", &[0.1, 0.1, 0.1]);
        assert_eq!(decide(&short, Sensitivity::More, &Thresholds::default()).len(), 1, "at least one");
        let huge: Vec<(usize, f32)> = (0..60).map(|i| (i, 0.1)).collect();
        let t = Thresholds { max_share: 1.0, ..Thresholds::default() };
        assert_eq!(decide(&sixty_words(&huge), Sensitivity::More, &t).len(), 6, "hard cap");
    }

    #[test]
    fn unknown_confidence_is_not_a_doubt() {
        let mut e = sixty_words(&[]);
        for w in &mut e.words {
            w.confidence = None;
        }
        assert!(decide(&e, Sensitivity::More, &Thresholds::default()).is_empty());
        e.words[3].alternatives.push(Alternative { text: "Kestrel".into(), source: AltSource::Vocabulary });
        assert!(decide(&e, Sensitivity::Fewer, &Thresholds::default()).is_empty());
        let m = decide(&e, Sensitivity::Balanced, &Thresholds::default());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].alternatives[0].text, "Kestrel");
    }

    #[test]
    fn gaps_mark_the_end_of_the_word_before() {
        let mut e = evidence("since the vendor confirmed the dates", &[0.9; 6]);
        e.gaps.push(GapEvidence { after: Some(2), time: (1.0, 2.1), speech_share: 0.9, alternatives: vec![] });
        e.gaps.push(GapEvidence { after: Some(4), time: (3.0, 3.5), speech_share: 0.9, alternatives: vec![] });
        let m = decide(&e, Sensitivity::Balanced, &Thresholds { max_share: 1.0, ..Thresholds::default() });
        assert_eq!(m.len(), 1, "the short gap is ignored");
        assert_eq!(m[0].kind, MarkKind::Gap);
        assert_eq!(m[0].heard, "vendor");
        assert_eq!(&e.text[..m[0].start], "since the vendor");
        assert_eq!(m[0].start, m[0].end);
        assert_eq!(m[0].audio, Some((1.0, 2.1)));
        e.gaps[0].speech_share = 0.2;
        assert!(decide(&e, Sensitivity::More, &Thresholds::default()).is_empty(), "quiet gaps are pauses");
    }

    #[test]
    fn punctuation_only_words_are_skipped() {
        let mut e = evidence("hello", &[0.1]);
        e.text = "hello".into();
        assert_eq!(decide(&e, Sensitivity::Balanced, &Thresholds::default()).len(), 1);
    }
}
