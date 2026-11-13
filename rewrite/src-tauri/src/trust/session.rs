//! The last pasted dictation and its marks: what is open, kept or fixed,
//! whether the marks are showing, and when everything is let go. No platform
//! code and no clock of its own; callers pass `now`.

use super::decision::{Mark, MarkKind};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Open,
    Kept,
    Replaced,
    /// The user changed the word themselves.
    Edited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Visibility {
    Shown,
    /// The user moved to another field or window; marks come back with them.
    Away,
    /// Hidden on request or after the idle limit; "bring back" restores them.
    Dismissed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMark {
    pub id: u32,
    #[serde(flatten)]
    pub mark: Mark,
    pub status: Status,
}

/// Why a session ended; for metadata-only logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    AllResolved,
    TextGone,
    FieldGone,
    Replaced,
    Expired,
    Off,
}

impl End {
    pub fn code(self) -> &'static str {
        match self {
            Self::AllResolved => "all_resolved",
            Self::TextGone => "text_gone",
            Self::FieldGone => "field_gone",
            Self::Replaced => "new_dictation",
            Self::Expired => "expired",
            Self::Off => "off",
        }
    }
}

/// How long marks stay without the user coming back to the field.
pub const AWAY_LIMIT: Duration = Duration::from_secs(10 * 60);
/// Dismissed marks can be brought back for this long.
pub const DISMISSED_LIMIT: Duration = Duration::from_secs(30 * 60);

pub struct Session {
    pub id: u64,
    pub text: String,
    pub marks: Vec<SessionMark>,
    pub visibility: Visibility,
    audio: Option<Arc<Vec<f32>>>,
    since: Instant,
}

impl Session {
    pub fn new(id: u64, text: String, marks: Vec<Mark>, audio: Option<Arc<Vec<f32>>>, now: Instant) -> Self {
        let marks = marks
            .into_iter()
            .enumerate()
            .map(|(i, mark)| SessionMark { id: i as u32 + 1, mark, status: Status::Open })
            .collect();
        Self { id, text, marks, visibility: Visibility::Shown, audio, since: now }
    }

    pub fn open(&self) -> impl Iterator<Item = &SessionMark> {
        self.marks.iter().filter(|m| m.status == Status::Open)
    }

    pub fn open_count(&self) -> usize {
        self.open().count()
    }

    pub fn get(&self, id: u32) -> Option<&SessionMark> {
        self.marks.iter().find(|m| m.id == id)
    }

    /// Nothing left to check: the audio goes now.
    pub fn finished(&self) -> bool {
        self.open_count() == 0
    }

    fn resolve(&mut self, id: u32, status: Status) -> bool {
        let Some(m) = self.marks.iter_mut().find(|m| m.id == id && m.status == Status::Open) else {
            return false;
        };
        m.status = status;
        if self.finished() {
            self.audio = None;
        }
        true
    }

    pub fn keep(&mut self, id: u32) -> bool {
        self.resolve(id, Status::Kept)
    }

    pub fn edited(&mut self, id: u32) -> bool {
        self.resolve(id, Status::Edited)
    }

    /// The word (or gap) was replaced by `with` in the host field. The pasted
    /// text and every later mark shift to match what is now in the field.
    pub fn replaced(&mut self, id: u32, with: &str) -> bool {
        let Some(index) = self.marks.iter().position(|m| m.id == id && m.status == Status::Open) else {
            return false;
        };
        let (start, end, kind) = {
            let m = &self.marks[index].mark;
            (m.start, m.end, m.kind)
        };
        let inserted = match kind {
            MarkKind::Word => with.to_owned(),
            MarkKind::Gap => gap_insertion(&self.text, start, with),
        };
        if end > self.text.len() || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return false;
        }
        self.text.replace_range(start..end, &inserted);
        let delta = inserted.len() as isize - (end - start) as isize;
        for m in &mut self.marks {
            if m.mark.start >= end && m.id != id {
                m.mark.start = (m.mark.start as isize + delta) as usize;
                m.mark.end = (m.mark.end as isize + delta) as usize;
            }
        }
        let m = &mut self.marks[index].mark;
        m.end = start + inserted.len();
        m.heard = inserted.trim().to_owned();
        self.resolve(id, Status::Replaced)
    }

    pub fn left(&mut self) {
        if self.visibility == Visibility::Shown {
            self.visibility = Visibility::Away;
            self.since = Instant::now();
        }
    }

    pub fn left_at(&mut self, now: Instant) {
        if self.visibility == Visibility::Shown {
            self.visibility = Visibility::Away;
            self.since = now;
        }
    }

    /// The user is back in the field: marks they left return, dismissed ones stay hidden.
    pub fn returned(&mut self) -> bool {
        if self.visibility == Visibility::Away {
            self.visibility = Visibility::Shown;
            return true;
        }
        false
    }

    pub fn dismiss(&mut self, now: Instant) {
        self.visibility = Visibility::Dismissed;
        self.since = now;
    }

    /// "Bring back": shows the open marks again. False when none are left.
    pub fn restore(&mut self) -> bool {
        if self.finished() {
            return false;
        }
        self.visibility = Visibility::Shown;
        true
    }

    pub fn expired(&self, now: Instant) -> bool {
        match self.visibility {
            Visibility::Shown => false,
            Visibility::Away => now.duration_since(self.since) >= AWAY_LIMIT,
            Visibility::Dismissed => now.duration_since(self.since) >= DISMISSED_LIMIT,
        }
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// The audio around a mark, padded so a word is heard in context, as
    /// 16 kHz mono samples plus where the mark sits inside the snippet.
    pub fn snippet(&self, id: u32) -> Option<Snippet> {
        let audio = self.audio.as_ref()?;
        let (a, b) = self.get(id)?.mark.audio?;
        snippet(audio, a, b)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    #[serde(skip)]
    pub samples: Vec<f32>,
    /// Where the doubtful span starts and ends inside the snippet, seconds.
    pub mark_start: f32,
    pub mark_end: f32,
    pub seconds: f32,
}

pub const SNIPPET_PAD: f32 = 0.9;
pub const SNIPPET_MAX: f32 = 6.0;

pub fn snippet(audio: &[f32], a: f32, b: f32) -> Option<Snippet> {
    let total = audio.len() as f32 / 16_000.0;
    if !(a.is_finite() && b.is_finite()) || b <= a || a >= total {
        return None;
    }
    let b = b.min(a + SNIPPET_MAX - 2.0 * SNIPPET_PAD).min(total);
    let from = (a - SNIPPET_PAD).max(0.0);
    let to = (b + SNIPPET_PAD).min(total);
    let samples = audio[(from * 16_000.0) as usize..((to * 16_000.0) as usize).min(audio.len())].to_vec();
    Some(Snippet { seconds: samples.len() as f32 / 16_000.0, samples, mark_start: a - from, mark_end: b - from })
}

/// Text to insert at a gap so spacing stays natural: a space before the new
/// word unless the gap sits at the very start.
pub fn gap_insertion(text: &str, at: usize, word: &str) -> String {
    let word = word.trim();
    let before = text[..at].chars().next_back();
    let after = text[at..].chars().next();
    match (before, after) {
        (None, _) => format!("{word} "),
        (Some(c), _) if c.is_whitespace() => format!("{word} "),
        (_, Some(c)) if c.is_whitespace() || c.is_ascii_punctuation() => format!(" {word}"),
        _ => format!(" {word} "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::decision::MarkKind;

    fn mark(text: &str, word: &str, kind: MarkKind) -> Mark {
        let start = text.find(word).unwrap();
        let (start, end) = if kind == MarkKind::Gap { (start + word.len(), start + word.len()) } else { (start, start + word.len()) };
        Mark { kind, start, end, heard: word.into(), alternatives: vec![], audio: Some((1.0, 1.4)), score: 0.5 }
    }

    fn session() -> Session {
        let text = "add the Castrel rollout since the vendor confirmed the dates";
        Session::new(
            7,
            text.into(),
            vec![mark(text, "Castrel", MarkKind::Word), mark(text, "vendor", MarkKind::Gap), mark(text, "dates", MarkKind::Word)],
            Some(Arc::new(vec![0.1; 16_000 * 3])),
            Instant::now(),
        )
    }

    #[test]
    fn keep_and_replace_resolve_and_audio_goes_with_the_last_mark() {
        let mut s = session();
        assert_eq!(s.open_count(), 3);
        assert!(s.keep(1));
        assert!(!s.keep(1), "already resolved");
        assert!(s.has_audio());
        assert!(s.replaced(2, "has"));
        assert_eq!(s.text, "add the Castrel rollout since the vendor has confirmed the dates");
        let dates = s.get(3).unwrap().mark.clone();
        assert_eq!(&s.text[dates.start..dates.end], "dates", "later marks shift");
        assert!(s.replaced(3, "data"));
        assert!(s.text.ends_with("the data"));
        assert!(s.finished());
        assert!(!s.has_audio(), "audio is discarded when the marks are gone");
        assert!(!s.restore(), "nothing to bring back");
    }

    #[test]
    fn replacing_a_word_moves_the_marks_after_it() {
        let mut s = session();
        assert!(s.replaced(1, "Kestrel"));
        assert_eq!(&s.text[..23], "add the Kestrel rollout");
        let gap = &s.get(2).unwrap().mark;
        assert_eq!(&s.text[..gap.start], "add the Kestrel rollout since the vendor");
    }

    #[test]
    fn leaving_and_returning_and_bringing_back() {
        let mut s = session();
        let t0 = Instant::now();
        s.left_at(t0);
        assert_eq!(s.visibility, Visibility::Away);
        assert!(s.returned());
        assert_eq!(s.visibility, Visibility::Shown);
        s.dismiss(t0);
        assert!(!s.returned(), "dismissed marks stay hidden when the user comes back");
        assert!(s.restore());
        assert_eq!(s.visibility, Visibility::Shown);
        assert!(!s.expired(t0 + Duration::from_secs(3600)), "shown marks never expire");
        s.left_at(t0);
        assert!(!s.expired(t0 + Duration::from_secs(60)));
        assert!(s.expired(t0 + AWAY_LIMIT));
        s.edited(1);
        assert_eq!(s.get(1).unwrap().status, Status::Edited);
    }

    #[test]
    fn snippets_pad_and_clamp() {
        let audio = vec![0.0f32; 16_000 * 4];
        let sn = snippet(&audio, 0.3, 0.8).unwrap();
        assert!((sn.mark_start - 0.3).abs() < 1e-3, "clamped at the start");
        assert!((sn.seconds - 1.7).abs() < 1e-2);
        let end = snippet(&audio, 3.5, 3.9).unwrap();
        assert!((end.seconds - 1.4).abs() < 1e-2, "clamped at the end");
        let long = snippet(&audio, 0.0, 4.0).unwrap();
        assert!(long.seconds <= SNIPPET_MAX + 0.01);
        assert!(snippet(&audio, 5.0, 6.0).is_none());
        assert!(snippet(&audio, 1.0, 1.0).is_none());
    }

    #[test]
    fn gap_insertion_spacing() {
        assert_eq!(gap_insertion("the vendor confirmed", 10, "has"), " has");
        assert_eq!(gap_insertion("the vendor, then", 10, "has"), " has");
        assert_eq!(gap_insertion("confirmed", 0, "It"), "It ");
        assert_eq!(gap_insertion("a b", 2, "x"), "x ");
    }
}
