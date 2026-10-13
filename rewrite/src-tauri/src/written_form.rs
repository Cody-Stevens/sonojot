//! Conservative spoken-to-written rendering. Span offsets are word indices
//! (end-exclusive); `apply` retains the original bytes outside replacements.
//! BIO input uses the same `split_whitespace` word boundaries as `spans`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

macro_rules! re {
    ($pattern:literal) => {{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| regex::Regex::new($pattern).expect("written-form regex"))
    }};
}

#[path = "written_form/models.rs"]
mod models;
#[path = "written_form/numbers.rs"]
mod numbers;
#[path = "written_form/proposals.rs"]
mod proposals;
#[path = "written_form/verbalize.rs"]
mod speech;
pub use proposals::{ModelProposer, RulesProposer, SpanProposer};
pub use speech::verbalize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Class {
    Cardinal,
    Decimal,
    Money,
    Percent,
    Magnitude,
    Year,
    Date,
    Time,
    Ordinal,
    Measure,
    Version,
    Model,
    Amp,
}

impl Class {
    pub const ALL: [Self; 13] = [
        Self::Cardinal,
        Self::Decimal,
        Self::Money,
        Self::Percent,
        Self::Magnitude,
        Self::Year,
        Self::Date,
        Self::Time,
        Self::Ordinal,
        Self::Measure,
        Self::Version,
        Self::Model,
        Self::Amp,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cardinal => "CARDINAL",
            Self::Decimal => "DECIMAL",
            Self::Money => "MONEY",
            Self::Percent => "PERCENT",
            Self::Magnitude => "MAGNITUDE",
            Self::Year => "YEAR",
            Self::Date => "DATE",
            Self::Time => "TIME",
            Self::Ordinal => "ORDINAL",
            Self::Measure => "MEASURE",
            Self::Version => "VERSION",
            Self::Model => "MODEL",
            Self::Amp => "AMP",
        }
    }
}
impl FromStr for Class {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|c| c.as_str() == value)
            .ok_or_else(|| format!("unknown written-form class: {value}"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BioLabel {
    O,
    B(Class),
    I(Class),
}
impl FromStr for BioLabel {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "O" {
            return Ok(Self::O);
        }
        if let Some(c) = value.strip_prefix("B-") {
            return c.parse().map(Self::B);
        }
        if let Some(c) = value.strip_prefix("I-") {
            return c.parse().map(Self::I);
        }
        Err(format!("invalid written-form BIO label: {value}"))
    }
}
impl Serialize for BioLabel {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::O => s.serialize_str("O"),
            Self::B(c) => s.serialize_str(&format!("B-{}", c.as_str())),
            Self::I(c) => s.serialize_str(&format!("I-{}", c.as_str())),
        }
    }
}
impl<'de> Deserialize<'de> for BioLabel {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

/// An accepted replacement, with end-exclusive whitespace-word offsets into
/// the original input. Byte-identical renders are omitted from edit metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub class: Class,
    pub rendered: String,
}

/// Present model labels select ROUTED_STRICT; absent labels use rules only.
/// Invalid alignment fails closed. Orphan I tags can license numeric rules
/// fallback, but never start a model span. A numeric rules span supported by a
/// non-O label may replace overlapping model spans only when all are unrenderable
/// and have the same class as the rule (SAME_CLASS).
pub fn spans(words: &[&str], labels: Option<&[BioLabel]>) -> Vec<Span> {
    match labels {
        Some(labels) => ModelProposer::new(labels).spans(words),
        None => RulesProposer.spans(words),
    }
}

fn tokens(text: &str) -> Vec<String> {
    let text = text.to_lowercase().replace('’', "'");
    let text = re!(r"\b([ap])\s*\.\s*m\.?").replace_all(&text, "${1}m");
    let text = re!(r"\b([ap])\s+m\b").replace_all(&text, "${1}m");
    // Preserve numeric signs. Number-word and model-name hyphens are separators.
    let text = re!(r"(\w)-(\w)").replace_all(&text, "$1 $2");
    let text = re!(r"(\w)-(\w)").replace_all(&text, "$1 $2");
    text.split_whitespace()
        .filter_map(|p| {
            let p = p
                .trim_matches(|c| ",;:!?\"()[]“”".contains(c))
                .trim_end_matches('.');
            (!p.is_empty()).then(|| p.to_owned())
        })
        .collect()
}

fn match_key(text: &str) -> String {
    let ts = tokens(text);
    let refs: Vec<_> = ts.iter().map(String::as_str).collect();
    numbers::grouped_digits(&refs)
        .into_iter()
        .filter(|t| t != "and")
        .map(|t| {
            if re!(r"^-?(?:\d+|\d{1,3}(?:,\d{3})+)(?:\.\d+)?$").is_match(&t) {
                speech::numeric_speech(&t).into_iter().next().unwrap_or(t)
            } else {
                t
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render a complete span only after parsing and reverse verbalization agree.
/// The caller supplies up to a few words of read-only adjacent context.
pub fn render_span(
    text: &str,
    class: Class,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<String> {
    render_with_guard(text, class, before, after, |core, candidate, class| {
        if class == Class::Model {
            return models::guard(core, candidate, before.unwrap_or(""), after.unwrap_or(""));
        }
        let key = match_key(core);
        verbalize(candidate).iter().any(|s| match_key(s) == key)
    })
}

fn render_with_guard(
    text: &str,
    class: Class,
    before: Option<&str>,
    after: Option<&str>,
    guard: impl FnOnce(&str, &str, Class) -> bool,
) -> Option<String> {
    if text.is_empty() || text.len() > 2048 {
        return None;
    }
    let start = text.find(|c: char| !c.is_whitespace() && !"\"([“".contains(c))?;
    let end = text.rfind(|c: char| !c.is_whitespace() && !".,;:!?\")]”".contains(c))?;
    let end = end + text[end..].chars().next()?.len_utf8();
    if start >= end {
        return None;
    }
    let core = &text[start..end];
    if class == Class::Model
        && tokens(core).last().is_some_and(|w| w == "extra")
        && after.is_some_and(|s| tokens(s).first().is_some_and(|w| w == "high"))
    {
        return None;
    }
    let ts = tokens(core);
    if ts.is_empty() {
        return None;
    }
    let refs: Vec<_> = ts.iter().map(String::as_str).collect();
    if matches!(
        class,
        Class::Cardinal
            | Class::Decimal
            | Class::Money
            | Class::Percent
            | Class::Magnitude
            | Class::Measure
    ) {
        let bt = tokens(before.unwrap_or(""));
        let at = tokens(after.unwrap_or(""));
        if bt.last().is_some_and(|w| w == "to")
            && bt.len() > 1
            && (numbers::scale(&bt[bt.len() - 2]).is_some()
                || numbers::number(&[&bt[bt.len() - 2]], false).is_some())
        {
            return None;
        }
        if at.first().is_some_and(|w| w == "to")
            && at.len() > 1
            && numbers::number(&[&at[1]], false).is_some()
            && refs.last().is_some_and(|w| {
                numbers::scale(w).is_some() || numbers::number(&[w], false).is_some()
            })
        {
            return None;
        }
    }
    let candidate = if class == Class::Model {
        models::render(core, before.unwrap_or(""), after.unwrap_or(""))
    } else {
        numbers::candidate(
            &refs,
            class,
            before.unwrap_or(""),
            after.unwrap_or(""),
            core,
        )
    }?;
    if !guard(core, &candidate, class) {
        return None;
    }
    Some(format!("{}{}{}", &text[..start], candidate, &text[end..]))
}

/// Apply only validated, unprotected spans. All whitespace and punctuation
/// outside those spans is copied from the original input, including UTF-8.
pub fn apply(text: &str, labels: Option<&[BioLabel]>) -> String {
    apply_with_edits(text, labels).0
}

/// Apply the selected proposer and return only replacements actually made.
pub fn apply_with_edits(text: &str, labels: Option<&[BioLabel]>) -> (String, Vec<Edit>) {
    let words: Vec<_> = text.split_whitespace().collect();
    let selected = spans(&words, labels);
    render_spans(text, &words, &selected)
}

/// Render preselected spans without proposing or routing again. This preserves
/// a caller's vocabulary/protection filtering. Spans must be ordered, disjoint
/// and within the input's whitespace-word bounds; invalid batches fail closed.
pub fn apply_spans(text: &str, spans: &[Span]) -> String {
    apply_spans_with_edits(text, spans).0
}

/// The preselected-span path with the same accepted-edit metadata as `apply`.
pub fn apply_spans_with_edits(text: &str, spans: &[Span]) -> (String, Vec<Edit>) {
    let words: Vec<_> = text.split_whitespace().collect();
    render_spans(text, &words, spans)
}

fn render_spans(text: &str, words: &[&str], spans: &[Span]) -> (String, Vec<Edit>) {
    let mut previous_end = 0;
    for span in spans {
        if span.start < previous_end || span.start >= span.end || span.end > words.len() {
            return (text.to_owned(), Vec::new());
        }
        previous_end = span.end;
    }
    if spans.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    let ranges: Vec<_> = words
        .iter()
        .map(|w| {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            start..start + w.len()
        })
        .collect();
    let protected = proposals::protected_ranges(text);
    let mut output = String::with_capacity(text.len());
    let mut edits = Vec::new();
    let mut cursor = 0;
    for span in spans {
        let start = ranges[span.start].start;
        let end = ranges[span.end - 1].end;
        if start < cursor || protected.iter().any(|r| r.start < end && r.end > start) {
            continue;
        }
        // Never erase a paragraph break, even if an external BIO head joins it.
        if text[start..end].contains(['\n', '\r']) {
            continue;
        }
        let before = words[span.start.saturating_sub(3)..span.start].join(" ");
        let after = words[span.end..words.len().min(span.end + 3)].join(" ");
        if let Some(replacement) =
            render_span(&text[start..end], span.class, Some(&before), Some(&after))
        {
            if replacement == text[start..end] {
                continue;
            }
            output.push_str(&text[cursor..start]);
            output.push_str(&replacement);
            cursor = end;
            edits.push(Edit {
                start: span.start,
                end: span.end,
                class: span.class,
                rendered: replacement,
            });
        }
    }
    output.push_str(&text[cursor..]);
    (output, edits)
}

#[cfg(test)]
#[path = "written_form/tests.rs"]
mod tests;
