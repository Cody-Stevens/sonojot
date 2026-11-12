//! Protected terms (`terms.json`): brand names and jargon that written form
//! must not rewrite. A span overlapping a cued term is declined by
//! `render_span`; `render_terms` re-spells a cued term only when its letters
//! and digits equal the canonical spelling. Mirrors `wf_render.py` exactly.

use serde::Deserialize;
use std::ops::Range;
use std::sync::OnceLock;

const TRIM: &str = ".,;:!?\"()[]{}“”'";
const LEAD: &str = "\"'([{“‘";
const TAIL: &str = ".,;:!?\"')]}”’";
const JOINERS: &str = " \t-.&+'";
const NAME_SKIP: [&str; 11] = ["and", "of", "the", "to", "in", "on", "for", "a", "an", "at", "by"];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Cue {
    /// Recognizer name casing; a one-word form equal to the written form needs none.
    Name,
    /// No capitals: jargon that a capitalized lexicon name would otherwise claim.
    Lower,
    Any,
}

fn default_cue() -> Cue {
    Cue::Name
}

#[derive(Deserialize)]
struct Table {
    terms: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    written: String,
    spoken: Vec<String>,
    #[serde(default = "default_cue")]
    case: Cue,
    #[serde(default)]
    blocks_models: bool,
}

#[derive(Debug)]
pub(super) struct Term {
    pub written: String,
    pub forms: Vec<Vec<String>>,
    pub case: Cue,
    pub blocks_models: bool,
}

pub(super) fn table() -> &'static [Term] {
    static TERMS: OnceLock<Vec<Term>> = OnceLock::new();
    TERMS.get_or_init(|| {
        let table: Table =
            serde_json::from_str(include_str!("terms.json")).expect("embedded protected terms");
        table
            .terms
            .into_iter()
            .map(|raw| {
                let forms: Vec<Vec<String>> = raw
                    .spoken
                    .iter()
                    .map(|form| tokens(form).into_iter().map(|t| t.lower).collect())
                    .collect();
                assert!(
                    forms.iter().all(|f| (1..=4).contains(&f.len())),
                    "protected term spoken forms need 1 to 4 words"
                );
                Term {
                    written: raw.written,
                    forms,
                    case: raw.case,
                    blocks_models: raw.blocks_models,
                }
            })
            .collect()
    })
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub lower: String,
    pub original: String,
    pub word: usize,
}

/// Lowercase/original pieces per hyphen-separated part of each whitespace word,
/// with outer marks removed and the word index retained.
pub(super) fn tokens(text: &str) -> Vec<Token> {
    let mut result = Vec::new();
    for (word, w) in text.split_whitespace().enumerate() {
        for piece in w.split('-') {
            let piece = piece.replace('’', "'");
            let core = piece.trim_matches(|c| TRIM.contains(c));
            if !core.is_empty() {
                result.push(Token {
                    lower: core.to_lowercase(),
                    original: core.to_owned(),
                    word,
                });
            }
        }
    }
    result
}

pub(super) fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

fn cued(term: &Term, matched: &[Token], form: &[String]) -> bool {
    match term.case {
        Cue::Any => true,
        Cue::Lower => !matched
            .iter()
            .any(|t| t.original.chars().any(char::is_uppercase)),
        Cue::Name => {
            if form.len() == 1 && form[0] == fold(&term.written) {
                return true;
            }
            let named: Vec<_> = matched
                .iter()
                .filter(|t| {
                    t.original.chars().next().is_some_and(char::is_alphabetic)
                        && !NAME_SKIP.contains(&t.lower.as_str())
                })
                .collect();
            !named.is_empty()
                && named
                    .iter()
                    .all(|t| t.original.chars().next().is_some_and(char::is_uppercase))
        }
    }
}

/// (length, term) of every cued term starting at token `p`, in table order.
fn matches_at(seq: &[Token], p: usize) -> Vec<(usize, &'static Term)> {
    let mut found = Vec::new();
    for term in table() {
        for form in &term.forms {
            let n = form.len();
            if p + n > seq.len() || (0..n).any(|k| seq[p + k].lower != form[k]) {
                continue;
            }
            if cued(term, &seq[p..p + n], form) {
                found.push((n, term));
            }
        }
    }
    found
}

/// True when a cued term overlaps the span words. A MODEL span is blocked only
/// when a `blocks_models` term covers all of it, so anchored names such as
/// "GPT five codec" and "codecs CLI" keep their lexicon rendering.
pub(super) fn protected(text: &str, before: &str, after: &str, model: bool) -> bool {
    let span = tokens(text);
    if span.is_empty() {
        return false;
    }
    let mut seq = tokens(before);
    let low = seq.len();
    let high = low + span.len();
    seq.extend(span);
    seq.extend(tokens(after));
    (0..seq.len()).any(|p| {
        matches_at(&seq, p)
            .into_iter()
            .any(|(n, term)| {
                if model {
                    term.blocks_models && p <= low && p + n >= high
                } else {
                    p < high && p + n > low
                }
            })
    })
}

/// Leftmost-longest whole-word canonical spelling of cued terms. A match is
/// consumed even when it is not re-spelled. Only case, spacing and joining
/// marks may change; quotes, code, links and `extra` byte ranges stay as is.
pub(super) fn render(text: &str, extra: &[Range<usize>]) -> String {
    let seq = tokens(text);
    let words: Vec<Range<usize>> = text
        .split_whitespace()
        .map(|w| {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            start..start + w.len()
        })
        .collect();
    let mut ranges = super::proposals::protected_ranges(text);
    ranges.extend(extra.iter().cloned());
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut p = 0;
    while p < seq.len() {
        if p > 0 && seq[p - 1].word == seq[p].word {
            p += 1;
            continue;
        }
        let mut best: Option<(usize, &Term)> = None;
        for (n, term) in matches_at(&seq, p) {
            if seq.get(p + n).is_some_and(|t| t.word == seq[p + n - 1].word) {
                continue;
            }
            if best.is_none_or(|(m, _)| n > m) {
                best = Some((n, term));
            }
        }
        let Some((n, term)) = best else {
            p += 1;
            continue;
        };
        let start = words[seq[p].word].start;
        let end = words[seq[p + n - 1].word].end;
        p += n;
        let region = &text[start..end];
        let after_lead = region.trim_start_matches(|c| LEAD.contains(c));
        let core = after_lead.trim_end_matches(|c| TAIL.contains(c));
        let begin = start + (region.len() - after_lead.len());
        let finish = begin + core.len();
        if core.is_empty()
            || core == term.written
            || ranges.iter().any(|r| r.start < end && r.end > start)
            || !core
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || JOINERS.contains(c))
            || fold(core) != fold(&term.written)
        {
            continue;
        }
        output.push_str(&text[cursor..begin]);
        output.push_str(&term.written);
        cursor = finish;
    }
    output.push_str(&text[cursor..]);
    output
}

/// Respell a multi-word or hyphen/dot-split match of a joined name from the
/// user's Vocabulary ("Next js" -> "Next.js"). Only names without spaces
/// qualify; letters and digits must be identical and only case, spacing and
/// joining marks change. At most four words; leftmost-longest; quotes, code,
/// links and `extra` byte ranges stay as is. No fuzzy matching.
pub(super) fn respell_joined(text: &str, names: &[&str], extra: &[Range<usize>]) -> String {
    let names: Vec<(&str, String)> = names
        .iter()
        .filter(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-.&+'".contains(c))
        })
        .map(|name| (*name, fold(name)))
        .filter(|(_, folded)| folded.len() >= 2)
        .collect();
    if names.is_empty() {
        return text.to_owned();
    }
    let words: Vec<Range<usize>> = text
        .split_whitespace()
        .map(|w| {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            start..start + w.len()
        })
        .collect();
    let mut ranges = super::proposals::protected_ranges(text);
    ranges.extend(extra.iter().cloned());
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut i = 0;
    'words: while i < words.len() {
        for len in (1..=4.min(words.len() - i)).rev() {
            let (start, end) = (words[i].start, words[i + len - 1].end);
            let region = &text[start..end];
            let after_lead = region.trim_start_matches(|c| LEAD.contains(c));
            let core = after_lead.trim_end_matches(|c| TAIL.contains(c));
            // A single plain word is a casing correction, handled by cleanup.
            if core.is_empty()
                || !core.contains([' ', '\t', '-', '.'])
                || !core
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || JOINERS.contains(c))
                || ranges.iter().any(|r| r.start < end && r.end > start)
            {
                continue;
            }
            let folded = fold(core);
            let Some((name, _)) = names.iter().find(|(_, f)| *f == folded) else {
                continue;
            };
            if core != *name {
                let begin = start + (region.len() - after_lead.len());
                output.push_str(&text[cursor..begin]);
                output.push_str(name);
                cursor = begin + core.len();
            }
            i += len;
            continue 'words;
        }
        i += 1;
    }
    output.push_str(&text[cursor..]);
    output
}
