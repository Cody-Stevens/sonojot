//! Shared text-tracking for locators that read the focused field's text
//! transiently (Linux AT-SPI, macOS Accessibility, Windows classic Edit).
//!
//! Marks stay anchored to their words through typing, deletion, pasting, and
//! reordering elsewhere in the field. A mark is dropped only when its own word
//! is edited, extended, or removed (or, for a gap, when the user inserts a
//! word right into the gap).
//!
//! Privacy: this module keeps only the marked words and short same-line
//! neighbour snippets from the pasted text, never the rest of the field.

use super::locator::{needle, Edge, Span};

const CONTEXT_CHARS: usize = 24;

#[derive(Clone, Debug)]
pub struct TrackedMark {
    pub word: Vec<char>,
    pub edge: Edge,
    /// Character offset of `word` in the field, as last confirmed.
    pub at: Option<usize>,
    /// True once resolved via Keep or Replace so polling skips it.
    pub resolved: bool,
    /// Bounded replacement token observed between unchanged anchors when the
    /// user retypes a marked word, emitted only after stabilizing across two polls.
    pub retyped: Option<String>,
    pending_retype: Option<(String, u8)>,
    needle: Vec<char>,
    within: usize,
    left: Vec<char>,
    right: Vec<char>,
    /// Adjacent word across a gap edge, used to notice when the user fills the gap.
    gap_neighbor: Vec<char>,
}

pub struct WordTracker {
    pub marks: Vec<TrackedMark>,
    last_len: usize,
}

impl WordTracker {
    pub fn attach(text: &[char], pasted: &str, spans: &[Span], caret: usize) -> Option<Self> {
        let pasted_chars: Vec<char> = pasted.chars().collect();
        let start = paste_start(text, &pasted_chars, caret)?;
        let marks = spans
            .iter()
            .map(|span| mark_for(text, pasted, span, start))
            .collect();
        Some(Self { marks, last_len: text.len() })
    }

    /// Update every unresolved mark's character offset for the current field text.
    pub fn follow(&mut self, text: &[char]) {
        let len_delta = text.len() as isize - self.last_len as isize;
        self.last_len = text.len();
        for mark in &mut self.marks {
            if mark.resolved {
                continue;
            }
            let Some(old_at) = mark.at else { continue };
            match relocate(text, mark, old_at, len_delta) {
                Some(new_at) => {
                    mark.at = Some(new_at);
                    mark.pending_retype = None;
                    let (left, right) = context_around(text, new_at, mark.word.len());
                    mark.left = left;
                    mark.right = right;
                }
                None => {
                    if mark.edge == Edge::Word && mark.retyped.is_none() {
                        if let Some(candidate) = extract_bounded_retype(text, mark, old_at) {
                            if candidate.chars().count() >= 3 {
                                mark.retyped = Some(candidate);
                            }
                        }
                    }
                    mark.pending_retype = None;
                    mark.at = None;
                }
            }
        }
    }

    pub fn resolve(&mut self, index: usize) {
        if let Some(mark) = self.marks.get_mut(index) {
            mark.resolved = true;
        }
    }

    /// True when unresolved marks remain and none of their words exist anymore.
    pub fn all_unresolved_gone(&self) -> bool {
        let mut any_open = false;
        for mark in &self.marks {
            if !mark.resolved {
                any_open = true;
                if mark.at.is_some() {
                    return false;
                }
            }
        }
        any_open
    }
}

/// Where the pasted text starts: the copy that ends nearest before the caret,
/// else the last copy.
pub fn paste_start(text: &[char], pasted: &[char], caret: usize) -> Option<usize> {
    if pasted.is_empty() || pasted.len() > text.len() {
        return None;
    }
    let starts: Vec<usize> = (0..=text.len() - pasted.len())
        .filter(|&i| text[i..i + pasted.len()] == *pasted)
        .collect();
    starts
        .iter()
        .copied()
        .filter(|&i| i + pasted.len() <= caret)
        .max()
        .or_else(|| starts.last().copied())
}

/// True when `word` at `at` is not embedded inside a longer alphanumeric word.
pub fn word_boundary_ok(text: &[char], at: usize, word: &[char]) -> bool {
    let Some(end) = at.checked_add(word.len()) else { return false };
    if end > text.len() {
        return false;
    }
    if word.first().is_some_and(|c| c.is_alphanumeric())
        && at > 0
        && text[at - 1].is_alphanumeric()
    {
        return false;
    }
    if word.last().is_some_and(|c| c.is_alphanumeric())
        && end < text.len()
        && text[end].is_alphanumeric()
    {
        return false;
    }
    true
}

/// Convert a character range `[start, end)` in `text` to a UTF-16 `(location, length)` pair.
pub fn char_range_to_utf16(text: &[char], start: usize, end: usize) -> (usize, usize) {
    let s = start.min(text.len());
    let e = end.min(text.len()).max(s);
    let loc: usize = text[..s].iter().map(|c| c.len_utf16()).sum();
    let len: usize = text[s..e].iter().map(|c| c.len_utf16()).sum();
    (loc, len)
}

/// Convert a UTF-16 code-unit offset in `text` to a character offset.
pub fn utf16_to_char_offset(text: &[char], utf16_offset: usize) -> usize {
    let mut units = 0usize;
    for (i, c) in text.iter().enumerate() {
        if units >= utf16_offset {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn mark_for(field_text: &[char], pasted: &str, span: &Span, paste_offset: usize) -> TrackedMark {
    let (a, b) = needle(pasted, span.start, span.end);
    let chars_before = |byte: usize| pasted[..byte].chars().count();
    let word: Vec<char> = pasted[span.start..span.end].chars().collect();
    let at = paste_offset + chars_before(span.start);
    let (left, right) = context_around(field_text, at, word.len());
    let gap_neighbor = match span.edge {
        Edge::Word => Vec::new(),
        Edge::After => next_word_on_line(field_text, at + word.len()),
        Edge::Before => prev_word_on_line(field_text, at),
    };
    TrackedMark {
        needle: pasted[a..b].chars().collect(),
        within: chars_before(span.start) - chars_before(a),
        at: Some(at),
        resolved: false,
        retyped: None,
        pending_retype: None,
        word,
        edge: span.edge,
        left,
        right,
        gap_neighbor,
    }
}

/// Extract a bounded single-term replacement (1–3 words, ≤40 chars) when the
/// user retypes a marked word between unchanged left/right anchors on the same
/// line. Returns `None` on deletion, ambiguity, or edits outside the anchors.
fn extract_bounded_retype(text: &[char], mark: &TrackedMark, old_at: usize) -> Option<String> {
    let left_anchor: &[char] = if mark.left.len() >= 4 {
        &mark.left[mark.left.len().saturating_sub(12)..]
    } else {
        return None;
    };
    let right_anchor: &[char] = if mark.right.len() >= 4 {
        &mark.right[..mark.right.len().min(12)]
    } else {
        return None;
    };
    if left_anchor.iter().filter(|c| c.is_alphanumeric()).count() < 2
        || right_anchor.iter().filter(|c| c.is_alphanumeric()).count() < 2
    {
        return None;
    }
    let window_start = old_at.saturating_sub(64);
    let window_end = (old_at + mark.word.len() + 64).min(text.len());
    if window_start >= window_end {
        return None;
    }
    let slice = &text[window_start..window_end];
    let left_hits = find_all_exact(slice, left_anchor);
    if left_hits.len() != 1 {
        return None;
    }
    let repl_start = left_hits[0] + left_anchor.len();
    let right_hits: Vec<usize> = find_all_exact(&slice[repl_start..], right_anchor);
    if right_hits.len() != 1 {
        return None;
    }
    let repl_end = repl_start + right_hits[0];
    if repl_end <= repl_start || repl_end - repl_start > 40 {
        return None;
    }
    let middle = &slice[repl_start..repl_end];
    if middle.iter().any(|&c| c == '\n' || c == '\r' || c.is_control()) {
        return None;
    }
    let raw: String = middle.iter().collect();
    let trimmed = raw.trim();
    let orig: String = mark.word.iter().collect();
    if trimmed.is_empty()
        || trimmed == orig
        || trimmed.split_whitespace().count() > 3
        || !trimmed.chars().any(char::is_alphabetic)
    {
        return None;
    }
    Some(trimmed.to_owned())
}

fn context_around(text: &[char], at: usize, len: usize) -> (Vec<char>, Vec<char>) {
    let line_start = text[..at.min(text.len())]
        .iter()
        .rposition(|&c| c == '\n' || c == '\r')
        .map_or(0, |i| i + 1);
    let left_start = at.saturating_sub(CONTEXT_CHARS).max(line_start);
    let left = text[left_start..at.min(text.len())].to_vec();

    let after = (at + len).min(text.len());
    let line_end = text[after..]
        .iter()
        .position(|&c| c == '\n' || c == '\r')
        .map_or(text.len(), |i| after + i);
    let right_end = (after + CONTEXT_CHARS).min(line_end);
    let right = text[after..right_end].to_vec();
    (left, right)
}

fn next_word_on_line(text: &[char], mut i: usize) -> Vec<char> {
    while i < text.len() && text[i] != '\n' && text[i] != '\r' && !text[i].is_alphanumeric() {
        i += 1;
    }
    let start = i;
    while i < text.len() && text[i].is_alphanumeric() {
        i += 1;
    }
    text[start..i].to_vec()
}

fn prev_word_on_line(text: &[char], mut i: usize) -> Vec<char> {
    while i > 0 && text[i - 1] != '\n' && text[i - 1] != '\r' && !text[i - 1].is_alphanumeric() {
        i -= 1;
    }
    let end = i;
    while i > 0 && text[i - 1].is_alphanumeric() {
        i -= 1;
    }
    text[i..end].to_vec()
}

fn find_all_exact(text: &[char], what: &[char]) -> Vec<usize> {
    if what.is_empty() || what.len() > text.len() {
        return Vec::new();
    }
    (0..=text.len() - what.len())
        .filter(|&i| text[i..i + what.len()] == *what)
        .collect()
}

fn find_all_words(text: &[char], word: &[char]) -> Vec<usize> {
    if word.is_empty() || word.len() > text.len() {
        return Vec::new();
    }
    (0..=text.len() - word.len())
        .filter(|&i| text[i..i + word.len()] == *word && word_boundary_ok(text, i, word))
        .collect()
}

#[derive(Clone, Copy, Default)]
struct ContextMatch {
    left_chars: usize,
    right_chars: usize,
    left_alnum: usize,
    right_alnum: usize,
}

impl ContextMatch {
    fn total(self) -> usize {
        self.left_chars + self.right_chars
    }
}

fn match_context(text: &[char], at: usize, len: usize, left: &[char], right: &[char]) -> ContextMatch {
    let mut left_chars = 0usize;
    let mut left_alnum = 0usize;
    let max_left = left.len().min(at);
    for k in 1..=max_left {
        let c = text[at - k];
        if c == left[left.len() - k] {
            left_chars += 1;
            if c.is_alphanumeric() {
                left_alnum += 1;
            }
        } else {
            break;
        }
    }

    let mut right_chars = 0usize;
    let mut right_alnum = 0usize;
    let after = at + len;
    let max_right = right.len().min(text.len().saturating_sub(after));
    for k in 0..max_right {
        let c = text[after + k];
        if c == right[k] {
            right_chars += 1;
            if c.is_alphanumeric() {
                right_alnum += 1;
            }
        } else {
            break;
        }
    }

    ContextMatch { left_chars, right_chars, left_alnum, right_alnum }
}

fn has_alnum(slice: &[char]) -> bool {
    slice.iter().any(|c| c.is_alphanumeric())
}

fn gap_was_filled(text: &[char], at: usize, len: usize, edge: Edge, neighbor: &[char]) -> bool {
    if neighbor.is_empty() {
        return false;
    }
    match edge {
        Edge::Word => false,
        Edge::After => {
            let after = at + len;
            let immediate = next_word_on_line(text, after);
            if immediate.is_empty() || immediate == neighbor {
                return false;
            }
            // The original next word is still further along on the same line,
            // meaning the user typed another word between the anchor and it.
            let line_end = text[after..]
                .iter()
                .position(|&c| c == '\n' || c == '\r')
                .map_or(text.len(), |i| after + i)
                .min(after + 80);
            let slice = &text[after..line_end];
            find_all_words(slice, neighbor).into_iter().any(|pos| pos > 0)
        }
        Edge::Before => {
            let immediate = prev_word_on_line(text, at);
            if immediate.is_empty() || immediate == neighbor {
                return false;
            }
            let line_start = text[..at]
                .iter()
                .rposition(|&c| c == '\n' || c == '\r')
                .map_or(0, |i| i + 1)
                .max(at.saturating_sub(80));
            let slice = &text[line_start..at];
            !find_all_words(slice, neighbor).is_empty()
        }
    }
}

fn relocate(text: &[char], mark: &TrackedMark, old_at: usize, len_delta: isize) -> Option<usize> {
    let len = mark.word.len();
    let shift = len_delta.unsigned_abs();
    let expected = (old_at as isize + len_delta).max(0) as usize;

    let check_gap = |pos: usize| -> Option<usize> {
        if gap_was_filled(text, pos, len, mark.edge, &mark.gap_neighbor) {
            None
        } else {
            Some(pos)
        }
    };

    // Fast path: word is still at old_at with valid word boundaries.
    if text.get(old_at..old_at + len) == Some(&mark.word[..]) && word_boundary_ok(text, old_at, &mark.word) {
        // If text grew/shrank and the shifted position ALSO holds the exact
        // word with strictly stronger neighbour context (for example inserting
        // an identical phrase immediately before it), follow the original copy.
        if shift > 0
            && expected != old_at
            && text.get(expected..expected + len) == Some(&mark.word[..])
            && word_boundary_ok(text, expected, &mark.word)
        {
            let old_score = match_context(text, old_at, len, &mark.left, &mark.right);
            let new_score = match_context(text, expected, len, &mark.left, &mark.right);
            if new_score.total() > old_score.total() {
                return check_gap(expected);
            }
        }
        return check_gap(old_at);
    }

    let candidates = find_all_words(text, &mark.word);
    if candidates.is_empty() {
        return None;
    }

    // 1. Unique pasted phrase (only when it carries neighbour context).
    if mark.needle.len() > len {
        if let [only] = find_all_exact(text, &mark.needle)[..] {
            let pos = only + mark.within;
            if candidates.contains(&pos) {
                return check_gap(pos);
            }
        }
    }

    // 2. Candidates within the length shift since last poll: pick best context,
    // then closest to the expected shifted offset, then closest to old_at.
    let within_shift: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&i| i.abs_diff(old_at) <= shift)
        .collect();
    if !within_shift.is_empty() {
        let best = within_shift
            .into_iter()
            .max_by(|&a, &b| {
                let sa = match_context(text, a, len, &mark.left, &mark.right);
                let sb = match_context(text, b, len, &mark.left, &mark.right);
                sa.total()
                    .cmp(&sb.total())
                    .then_with(|| b.abs_diff(expected).cmp(&a.abs_diff(expected)))
                    .then_with(|| b.abs_diff(old_at).cmp(&a.abs_diff(old_at)))
            })?;
        return check_gap(best);
    }

    // 3. Reordered / combined edits where the word moved beyond `shift`:
    // require strong surrounding context (both sides when both sides existed,
    // or a multi-word / 6+ alnum-char match on one side, or a full word on the
    // only side that existed) so deleting a doubted word never jumps onto an
    // unrelated copy elsewhere in the document.
    let had_left = has_alnum(&mark.left);
    let had_right = has_alnum(&mark.right);
    let strong = candidates
        .into_iter()
        .filter_map(|i| {
            let m = match_context(text, i, len, &mark.left, &mark.right);
            let ok = match (had_left, had_right) {
                (true, true) => {
                    (m.left_alnum >= 2 && m.right_alnum >= 2)
                        || m.left_alnum >= 6
                        || m.right_alnum >= 6
                }
                (true, false) => m.left_alnum >= 3,
                (false, true) => m.right_alnum >= 3,
                (false, false) => false,
            };
            ok.then_some((i, m.total()))
        })
        .max_by(|&(ia, sa), &(ib, sb)| {
            sa.cmp(&sb).then_with(|| ib.abs_diff(old_at).cmp(&ia.abs_diff(old_at)))
        });

    strong.and_then(|(pos, _)| check_gap(pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    fn span_of(pasted: &str, word: &str, edge: Edge) -> Span {
        let start = pasted.find(word).unwrap();
        Span { start, end: start + word.len(), edge }
    }

    #[test]
    fn paste_nearest_before_caret_wins() {
        let text = chars("hello world. hello world.");
        assert_eq!(paste_start(&text, &chars("hello world."), 25), Some(13));
        assert_eq!(paste_start(&text, &chars("hello world."), 12), Some(0));
        assert_eq!(paste_start(&text, &chars("absent"), 25), None);
    }

    #[test]
    fn survives_typing_deleting_and_large_pastes_elsewhere() {
        let pasted = "please check the quarterly figures before Friday";
        let initial = chars(pasted);
        let spans = vec![
            span_of(pasted, "quarterly", Edge::Word),
            span_of(pasted, "Friday", Edge::Word),
        ];
        let mut t = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();
        assert_eq!(t.marks[0].at, Some(17));
        assert_eq!(t.marks[1].at, Some(42));

        // Large paste before the dictation shifts both marks.
        let prefix = "A".repeat(400) + " ";
        let edited = chars(&format!("{prefix}{pasted}"));
        t.follow(&edited);
        assert_eq!(t.marks[0].at, Some(401 + 17));
        assert_eq!(t.marks[1].at, Some(401 + 42));

        // Delete part of the prefix and edit between the two marks.
        let edited2 = chars("Hi! please check the quarterly revenue numbers before Friday evening");
        t.follow(&edited2);
        assert_eq!(t.marks[0].at, Some(21));
        assert_eq!(t.marks[1].at, Some(54));
    }

    #[test]
    fn survives_net_zero_shift_reordering() {
        let pasted = "please check the quarterly figures before Friday";
        let initial = chars(pasted);
        let spans = vec![span_of(pasted, "quarterly", Edge::Word)];
        let mut t = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();

        // Cut "before Friday" from the end and move it to the front: same total length!
        let reordered = chars("before Friday please check the quarterly figures");
        assert_eq!(reordered.len(), initial.len());
        t.follow(&reordered);
        assert_eq!(t.marks[0].at, Some(31));
    }

    #[test]
    fn ends_when_doubted_word_is_edited_extended_or_deleted() {
        let pasted = "send the plan to the team";
        let initial = chars(pasted);
        let spans = vec![span_of(pasted, "plan", Edge::Word)];

        // Extending "plan" -> "planet" must end the mark, not match prefix!
        let mut t1 = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();
        t1.follow(&chars("send the planet to the team"));
        assert_eq!(t1.marks[0].at, None);
        assert!(t1.all_unresolved_gone());

        // Retyping "plan" -> "draft" when another unrelated "plan" exists later.
        let with_extra = chars("send the plan to the team. the plan is later");
        let mut t2 = WordTracker::attach(&with_extra, pasted, &spans, 25).unwrap();
        assert_eq!(t2.marks[0].at, Some(9));
        t2.follow(&chars("send the draft to the team. the plan is later"));
        assert_eq!(t2.marks[0].at, None);
        assert_eq!(t2.marks[0].retyped.as_deref(), Some("draft"));

        // Deleting the doubted word captures no retype.
        let mut t3 = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();
        t3.follow(&chars("send the  to the team"));
        assert_eq!(t3.marks[0].at, None);
        assert_eq!(t3.marks[0].retyped, None);
    }

    #[test]
    fn repeated_word_tracks_its_own_occurrence() {
        let pasted = "the plan and the plan";
        let second = pasted.rfind("plan").unwrap();
        let spans = vec![Span { start: second, end: second + 4, edge: Edge::Word }];
        let initial = chars(pasted);
        let mut t = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();
        assert_eq!(t.marks[0].at, Some(17));

        let edited = chars("Note: the plan and the plan today");
        t.follow(&edited);
        assert_eq!(t.marks[0].at, Some(23));
    }

    #[test]
    fn gap_mark_survives_edits_elsewhere_and_ends_when_user_fills_gap() {
        let pasted = "since the vendor confirmed the dates";
        let initial = chars(pasted);
        let spans = vec![span_of(pasted, "vendor", Edge::After)];
        let mut t = WordTracker::attach(&initial, pasted, &spans, initial.len()).unwrap();

        // Edit elsewhere: gap stays.
        t.follow(&chars("Hi, since the vendor confirmed the new dates"));
        assert_eq!(t.marks[0].at, Some(14));

        // User inserts "has" right into the gap between "vendor" and "confirmed": gap ends.
        t.follow(&chars("Hi, since the vendor has confirmed the new dates"));
        assert_eq!(t.marks[0].at, None);
    }

    #[test]
    fn utf16_conversion_handles_surrogate_pairs() {
        let text = chars("Hi 👍 Castrel!");
        // "Hi " = 3, "👍" = 2 UTF-16 units, " " = 1 => "Castrel" starts at char 5, UTF-16 6.
        assert_eq!(char_range_to_utf16(&text, 5, 12), (6, 7));
        assert_eq!(utf16_to_char_offset(&text, 6), 5);
        assert_eq!(utf16_to_char_offset(&text, 3), 3);
        assert_eq!(utf16_to_char_offset(&text, 5), 4);
    }
}
