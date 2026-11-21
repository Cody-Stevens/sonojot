//! Where the pasted words are on screen, and how to change one. The platform
//! layer sits behind [`Locator`]; everything here is plain data.
//!
//! Privacy rule for every implementation: read only the text needed to locate
//! the words SonoJot just pasted (and notice edits to them) and their screen
//! positions. Never keep, store or log text from the host app.

use serde::Serialize;

/// Physical screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn contains_rect(&self, other: &Rect, slack: i32) -> bool {
        other.x >= self.x - slack
            && other.y >= self.y - slack
            && other.right() <= self.right() + slack
            && other.bottom() <= self.bottom() + slack
    }
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then_some(Rect { x, y, w: r - x, h: b - y })
    }
}

/// What a located field allows. Doubtful words show only as inline highlights
/// on the words themselves; when an app does not expose trustworthy positions,
/// nothing is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Marks drawn on the words; a fix replaces the word in place.
    Inline,
    /// Password fields, terminals, elevated windows, or apps without
    /// trustworthy word positions: show nothing at all.
    Nothing,
}

impl Capability {
    pub fn inline(self) -> bool {
        self == Self::Inline
    }
}

/// Where the user is relative to the pasted field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Focus {
    InField,
    /// Another field or window has the keyboard. SonoJot's own overlay counts
    /// as in field while it holds focus for the keyboard flow.
    Away,
    /// The field or its window no longer exists.
    Gone,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "rects")]
pub enum Place {
    Visible(Vec<Rect>),
    /// Scrolled out of view or clipped by the field.
    Hidden,
    /// The word no longer reads as pasted: the user edited or deleted it.
    Changed,
}

/// One look at the field.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub focus: Focus,
    /// One per requested span, in request order.
    pub marks: Vec<Place>,
    /// Bounded replacement token when a marked word was retyped in place
    /// between unchanged anchors (aligned with `marks`, empty when unused).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retyped: Vec<Option<String>>,
    /// The visible field.
    pub field: Option<Rect>,
    /// The pasted words can no longer be found (sent, cleared, undone).
    pub text_gone: bool,
}

/// A word to find inside the pasted text. A gap is located through a
/// neighbouring word: `After` the word before it, or `Before` the first word.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub edge: Edge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Replace the word itself.
    Word,
    /// Insert at the end of the word.
    After,
    /// Insert at the start of the word.
    Before,
}

/// The identity of the field SonoJot pasted into, captured at dictation start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    /// Platform window handle of the focused control and its top window.
    pub focused: usize,
    pub window: usize,
    pub process: u32,
}

pub struct Request<'a> {
    pub text: &'a str,
    pub spans: &'a [Span],
    pub target: &'a Target,
}

/// A located field. Owned by the trust thread; never shared.
pub trait Tracked {
    fn capability(&self) -> Capability;
    /// Look again. Cheap enough for ten calls a second while marks show.
    fn poll(&mut self) -> Frame;
    /// Replace span `index` (as requested) with `text`, or insert `text` at
    /// its edge for a gap.
    fn replace(&mut self, index: usize, text: &str) -> Result<(), String>;
    /// Mark span `index` as resolved (Kept or Replaced) so polling skips it.
    fn resolve(&mut self, _index: usize) {}
    /// Give keyboard focus back to the field (after a keyboard popover).
    fn refocus(&mut self) -> bool;
}

pub trait Locator {
    /// Find the pasted text in the target field. `Nothing` comes back as a
    /// tracked field with that capability, never as an error.
    fn attach(&mut self, request: &Request) -> Result<Box<dyn Tracked>, String>;
}

/// Nothing to show (password, terminal, elevated, or positions unavailable).
pub struct Inert;

impl Tracked for Inert {
    fn capability(&self) -> Capability {
        Capability::Nothing
    }
    fn poll(&mut self) -> Frame {
        Frame { focus: Focus::Gone, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true }
    }
    fn replace(&mut self, _: usize, _: &str) -> Result<(), String> {
        Err("unsupported".into())
    }
    fn refocus(&mut self) -> bool {
        false
    }
}

/// Inline marks are skipped when any visible located word fails the check:
/// the point at the word's rectangle must hit the word itself (rich editors
/// can be characters off), the rectangle must be a plausible text size, and it
/// must lie inside the visible field.
pub fn trustworthy(rects: &[Rect], field: Option<&Rect>, hit_inside_word: bool) -> bool {
    let Some(first) = rects.first() else {
        return false;
    };
    let sane = rects.iter().all(|r| r.w > 0 && r.h >= 6 && r.h <= 400 && r.w <= 4000);
    let inside = field.is_none_or(|f| rects.iter().all(|r| f.contains_rect(r, 4)));
    sane && inside && hit_inside_word && first.w > 0
}

/// The byte span of the text around a word, within its own line, used as
/// the search needle for the word. Grows word by word until it occurs once in
/// the pasted text, so repeated words find their own occurrence.
pub fn needle(text: &str, start: usize, end: usize) -> (usize, usize) {
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let (mut a, mut b) = (start, end);
    loop {
        if text.matches(&text[a..b]).count() <= 1 || (a == line_start && b == line_end) {
            return (a, b);
        }
        if a > line_start {
            a = text[line_start..a].trim_end().rfind(' ').map_or(line_start, |i| line_start + i + 1);
        }
        if b < line_end {
            b = text[b..line_end].trim_start_matches(' ').find(' ').map_or(line_end, |i| {
                let skipped = text[b..line_end].len() - text[b..line_end].trim_start_matches(' ').len();
                b + skipped + i
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatch_check_drops_offset_rich_editors() {
        let field = Rect { x: 0, y: 0, w: 800, h: 400 };
        let word = vec![Rect { x: 100, y: 50, w: 60, h: 20 }];
        assert!(trustworthy(&word, Some(&field), true));
        assert!(!trustworthy(&word, Some(&field), false), "the point hit another word");
        assert!(!trustworthy(&[], Some(&field), true), "no rectangles");
        assert!(!trustworthy(&[Rect { x: 100, y: 50, w: 0, h: 20 }], Some(&field), true), "empty width (VS Code)");
        assert!(!trustworthy(&[Rect { x: 790, y: 50, w: 60, h: 20 }], Some(&field), true), "outside the field");
        assert!(!trustworthy(&[Rect { x: 1, y: 1, w: 60, h: 900 }], None, true), "not a text line");
    }

    #[test]
    fn needles_are_unique_and_stay_on_their_line() {
        let text = "the plan and the plan\nnew line the plan";
        let second = text.find(" the plan\n").unwrap() + 5;
        let (a, b) = needle(text, second, second + 4);
        assert_eq!(&text[a..b], "and the plan");
        assert_eq!(text.matches(&text[a..b]).count(), 1);
        let unique = text.find("new").unwrap();
        assert_eq!(needle(text, unique, unique + 3), (unique, unique + 3));
        let last = text.rfind("plan").unwrap();
        let (a, b) = needle(text, last, last + 4);
        assert!(!text[a..b].contains('\n'));
    }

    #[test]
    fn rect_intersection_clips_scrollable_fields() {
        let outer = Rect { x: 10, y: 20, w: 300, h: 200 };
        let inner = Rect { x: 10, y: -100, w: 300, h: 900 };
        assert_eq!(outer.intersect(&inner), Some(outer));
        let disjoint = Rect { x: 500, y: 500, w: 50, h: 50 };
        assert_eq!(outer.intersect(&disjoint), None);
    }
}
