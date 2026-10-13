//! Small transparent local-learning foundation (Round 6, privacy owner).
//!
//! Local-only reuse of user-authored corrections without retraining weights,
//! cloud upload, or background scraping. Explicit `VocabularyEntry` items stay
//! separate and are never pruned by history deletion; learned entries are
//! source-traceable (`source_ids`), scoped, inspectable, reversible, and
//! pruned when their support disappears. Only independently authored manual
//! entries may be pinned; pinning cannot retain deleted source text.
//!
//! Rejection policy (conservative, per VOCABULARY_LEARNING.md):
//! - digits, spelled-out quantities, instruction/negation/direction/time words
//! - paragraph rewrites, insertions/deletions, or fewer than two retained words
//! - aliases over three words/64 bytes, or beyond a small spelling distance
//! Whole-sentence rewrites, insertions/deletions-only, and casing-only
//! no-ops beyond a narrow alias are not learned (count 0) but the explicit
//! record correction itself still saves.

use crate::domain::{CleanupMode, LearnKind, LearnedEntry, VocabularyEntry};

pub const MAX_LEARNED_ENTRIES: usize = 256;
/// Alias kept for the concurrent Store draft (`learning::MAX_ENTRIES`).
pub const MAX_ENTRIES: usize = MAX_LEARNED_ENTRIES;
pub const MAX_LEARNED_TEXT_CHARS: usize = 120;
pub const MAX_LEARNED_WORDS: usize = 3;
pub const MAX_RELEVANT_ENTRIES: usize = 12;
pub const RELEVANT_CHAR_BUDGET: usize = 1536;

const DIRECTION_WORDS: &[&str] = &[
    "left",
    "right",
    "up",
    "down",
    "above",
    "below",
    "top",
    "bottom",
    "front",
    "back",
    "forward",
    "backward",
    "north",
    "south",
    "east",
    "west",
    "before",
    "after",
    "next",
    "previous",
    "first",
    "second",
    "third",
    "today",
    "tomorrow",
    "yesterday",
    "morning",
    "evening",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn normalize_scope(scope: &Option<String>) -> Option<String> {
    scope
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn validate_learned_entry(entry: &LearnedEntry) -> Result<(), String> {
    if entry.id.is_empty()
        || entry.id.len() > 128
        || !entry
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid learned entry identifier.".into());
    }
    let text = entry.text.trim();
    if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err(
            "Learned text must contain between 1 and 4,096 bytes without control characters."
                .into(),
        );
    }
    if text.chars().count() > MAX_LEARNED_TEXT_CHARS {
        return Err(format!(
            "Learned entries are limited to {MAX_LEARNED_TEXT_CHARS} characters; save longer edits as explicit vocabulary or keep them in the record."
        ));
    }
    if let Some(heard) = entry.heard.as_deref() {
        let heard = heard.trim();
        if heard.is_empty() || heard.len() > 4096 || heard.chars().any(char::is_control) {
            return Err(
                "Learned heard phrase must contain between 1 and 4,096 bytes without control characters."
                    .into(),
            );
        }
        if heard.chars().count() > MAX_LEARNED_TEXT_CHARS {
            return Err(format!(
                "Learned heard phrases are limited to {MAX_LEARNED_TEXT_CHARS} characters."
            ));
        }
        if heard.eq_ignore_ascii_case(text) && entry.kind.is_lexical() {
            // Identical ignoring case is at most a casing hint; still storable,
            // but callers should prefer count-0 for pure no-ops. Keep valid.
        }
    } else if entry.kind == LearnKind::Correction {
        return Err("Correction entries require a heard phrase.".into());
    }
    if entry
        .scope
        .as_deref()
        .is_some_and(|s| s.len() > 1024 || s.chars().any(char::is_control))
    {
        return Err(
            "Learned scopes must be 1,024 bytes or fewer without control characters.".into(),
        );
    }
    if entry.source_type.trim().is_empty()
        || entry.source_type.len() > 64
        || entry.source_type.chars().any(char::is_control)
    {
        return Err("Learned source type must be 1-64 bytes without control characters.".into());
    }
    if entry.source_ids.len() > 32 {
        return Err("Learned entries track at most 32 source records.".into());
    }
    for id in &entry.source_ids {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("Invalid learned source identifier.".into());
        }
    }
    match entry.source_type.as_str() {
        "dictationCorrection"
            if !entry.source_ids.is_empty()
                && !entry.user_pinned
                && entry.kind == LearnKind::Correction => {}
        "manual" if entry.source_ids.is_empty() && entry.user_pinned => {}
        _ => {
            return Err(
                "Keep source-derived learning separate from independently saved vocabulary.".into(),
            )
        }
    }
    if entry.kind == LearnKind::Correction
        && !safe_alias(entry.heard.as_deref().unwrap_or(""), text)
    {
        return Err(
            "Learn a short spelling correction, not a quantity or instruction change.".into(),
        );
    }
    if entry.kind == LearnKind::Canonical
        && entry.heard.as_deref().is_some_and(|heard| {
            heard.split_whitespace().collect::<String>().to_lowercase()
                != text.split_whitespace().collect::<String>().to_lowercase()
        })
    {
        return Err("Canonical entries supply spelling. Use a scoped correction for a different heard form.".into());
    }
    Ok(())
}

pub fn validate_learning(entries: &[LearnedEntry]) -> Result<(), String> {
    if entries.len() > MAX_LEARNED_ENTRIES {
        return Err(format!(
            "Learned memory is limited to {MAX_LEARNED_ENTRIES} entries."
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let mut mappings = std::collections::HashSet::new();
    for entry in entries {
        if !ids.insert(&entry.id) {
            return Err("Learned entry identifiers must be unique.".into());
        }
        validate_learned_entry(entry)?;
        if entry.enabled
            && entry.kind == LearnKind::Correction
            && !mappings.insert((
                entry.heard.as_deref().unwrap_or("").to_lowercase(),
                entry.scope.as_deref().unwrap_or("").to_lowercase(),
            ))
        {
            return Err(
                "Resolve conflicting corrections within the same scope before enabling them."
                    .into(),
            );
        }
    }
    Ok(())
}

fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

fn direction_set(text: &str) -> std::collections::HashSet<String> {
    let words = word_set(text);
    DIRECTION_WORDS
        .iter()
        .filter(|w| words.contains(**w))
        .map(|w| w.to_string())
        .collect()
}

fn has_digit(text: &str) -> bool {
    text.bytes().any(|b| b.is_ascii_digit())
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// True when the span pair must not become a reusable alias.
pub fn is_rejected_pair(heard: &str, written: &str) -> bool {
    let heard = heard.trim();
    let written = written.trim();
    if heard.is_empty() || written.is_empty() {
        return true;
    }
    if heard.eq_ignore_ascii_case(written) {
        // Pure casing/whitespace no-op: nothing reusable to learn.
        return true;
    }
    for span in [heard, written] {
        if span.contains('\n') || span.contains('\r') {
            return true;
        }
        if span.chars().count() > MAX_LEARNED_TEXT_CHARS {
            return true;
        }
        if word_count(span) > MAX_LEARNED_WORDS {
            return true;
        }
        if has_digit(span) {
            return true;
        }
    }
    // Direction/time tokens differing between sides likely change intent.
    if direction_set(heard) != direction_set(written) {
        return true;
    }
    !safe_alias(heard, written)
}

fn protected(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "no" | "not"
            | "never"
            | "none"
            | "only"
            | "all"
            | "any"
            | "if"
            | "unless"
            | "and"
            | "or"
            | "can"
            | "cannot"
            | "cant"
            | "do"
            | "dont"
            | "does"
            | "must"
            | "should"
            | "would"
            | "will"
            | "wont"
            | "may"
            | "before"
            | "after"
            | "until"
            | "increase"
            | "decrease"
            | "enable"
            | "disable"
            | "allow"
            | "deny"
            | "start"
            | "stop"
            | "keep"
            | "delete"
            | "remove"
            | "save"
            | "send"
            | "open"
            | "close"
            | "on"
            | "off"
            | "zero"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "ten"
            | "eleven"
            | "twelve"
            | "thirteen"
            | "fourteen"
            | "fifteen"
            | "sixteen"
            | "seventeen"
            | "eighteen"
            | "nineteen"
            | "twenty"
            | "thirty"
            | "forty"
            | "fifty"
            | "sixty"
            | "seventy"
            | "eighty"
            | "ninety"
            | "hundred"
            | "thousand"
            | "million"
            | "billion"
            | "half"
            | "quarter"
            | "percent"
            | "seconds"
            | "minutes"
            | "hours"
            | "days"
            | "meter"
            | "meters"
            | "liters"
            | "grams"
            | "kilograms"
            | "inches"
            | "feet"
    ) || DIRECTION_WORDS.contains(&word.to_lowercase().as_str())
}

pub fn safe_alias(heard: &str, written: &str) -> bool {
    if [heard, written].iter().any(|s| {
        s.len() > 64
            || s.trim().is_empty()
            || s.split_whitespace().count() > MAX_LEARNED_WORDS
            || !s.chars().all(|c| c.is_alphabetic() || c == ' ')
            || s.split_whitespace().any(protected)
    }) {
        return false;
    }
    let compact = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphabetic())
            .flat_map(char::to_lowercase)
            .collect::<Vec<_>>()
    };
    let a = compact(heard);
    let b = compact(written);
    if a.len() < 3 || b.len() < 3 {
        return false;
    }
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb))
                .min(row[j] + 1)
                .min(old + 1);
            diagonal = old;
        }
    }
    row[b.len()] <= 3.min(a.len().max(b.len()) / 3)
}

/// Extract a single narrow substitution between the stored finished text and
/// the user's corrected text. Word-level common prefix/suffix; insertion- or
/// deletion-only edits, multi-span rewrites, and rejected pairs yield `None`.
pub fn extract_narrow_correction(old_text: &str, new_text: &str) -> Option<(String, String)> {
    if old_text.trim() == new_text.trim()
        || old_text.contains(['\n', '\r'])
        || new_text.contains(['\n', '\r'])
    {
        return None;
    }
    let old_words: Vec<&str> = words(old_text);
    let new_words: Vec<&str> = words(new_text);
    if old_words.is_empty() || new_words.is_empty() {
        return None;
    }
    let mut prefix = 0;
    while prefix < old_words.len()
        && prefix < new_words.len()
        && old_words[prefix] == new_words[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_words.len() - prefix
        && suffix < new_words.len() - prefix
        && old_words[old_words.len() - 1 - suffix] == new_words[new_words.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_span = old_words[prefix..old_words.len() - suffix].join(" ");
    let new_span = new_words[prefix..new_words.len() - suffix].join(" ");
    if prefix + suffix < 2 {
        return None;
    }
    if old_span.trim().is_empty() || new_span.trim().is_empty() {
        return None;
    }
    // More than one disjoint edit collapses into a span covering unchanged
    // words; reject spans that embed long unchanged middles by requiring the
    // span itself to stay narrow (checked in is_rejected_pair) and the total
    // changed words to stay small.
    if is_rejected_pair(&old_span, &new_span) {
        return None;
    }
    Some((old_span, new_span))
}

/// Build the learned entry for an explicit correction save. Returns `None`
/// when the edit is not a reusable alias (caller records `learnedCount: 0`).
pub fn correction_entry(
    source_id: &str,
    old_published: &str,
    corrected_text: &str,
    scope: Option<String>,
) -> Option<LearnedEntry> {
    let (heard, written) = extract_narrow_correction(old_published, corrected_text)?;
    Some(LearnedEntry {
        id: crate::store::new_id(),
        kind: LearnKind::Correction,
        text: written,
        heard: Some(heard),
        scope: normalize_scope(&scope),
        source_ids: vec![source_id.into()],
        source_type: "dictationCorrection".into(),
        enabled: true,
        user_pinned: false,
    })
}

/// Remove `deleted_ids` from every entry's `source_ids`. Entries left with no
/// support are dropped; independently authored manual entries remain.
/// Never retains excerpts of deleted content: only the concise
/// heard/written fact remains only while other sources support it.
pub fn prune_deleted_sources(
    entries: Vec<LearnedEntry>,
    deleted_ids: &[String],
) -> Vec<LearnedEntry> {
    if deleted_ids.is_empty() {
        return entries;
    }
    let deleted: std::collections::HashSet<&str> = deleted_ids.iter().map(String::as_str).collect();
    entries
        .into_iter()
        .filter_map(|mut entry| {
            if entry
                .source_ids
                .iter()
                .any(|id| deleted.contains(id.as_str()))
            {
                entry.source_ids.retain(|id| !deleted.contains(id.as_str()));
            }
            if entry.source_ids.is_empty() && !(entry.source_type == "manual" && entry.user_pinned)
            {
                None
            } else {
                Some(entry)
            }
        })
        .collect()
}

/// Small relevant subset for the current dictation, under a char budget
/// (~4 chars/token). Enabled only; pinned first, then more-supported, then
/// shorter. No embeddings; phrase lookup happens in cleanup via host rules.
pub fn select_relevant(entries: &[LearnedEntry], max_entries: usize) -> Vec<LearnedEntry> {
    let max_entries = max_entries.min(MAX_RELEVANT_ENTRIES);
    let mut ranked: Vec<&LearnedEntry> = entries.iter().filter(|e| e.enabled).collect();
    ranked.sort_by(|a, b| {
        b.user_pinned
            .cmp(&a.user_pinned)
            .then_with(|| b.source_ids.len().cmp(&a.source_ids.len()))
            .then_with(|| a.text.len().cmp(&b.text.len()))
    });
    let mut used = 0usize;
    let mut out = Vec::new();
    for entry in ranked.into_iter().take(max_entries) {
        let cost = entry.text.len()
            + entry.heard.as_deref().map_or(0, str::len)
            + entry.scope.as_deref().map_or(0, str::len)
            + 16;
        if used + cost > RELEVANT_CHAR_BUDGET {
            continue;
        }
        used += cost;
        out.push(entry.clone());
    }
    out
}

/// Lexical entries (correction/canonical) as host-rule vocabulary for Clean.
/// Unscoped entries carry empty context, which the host permits only for
/// casing-level application; scoped entries carry scope words as context.
pub fn to_cleanup_vocabulary(entries: &[LearnedEntry]) -> Vec<crate::cleanup::Vocabulary> {
    entries
        .iter()
        .filter(|e| e.enabled && e.kind.is_lexical())
        .filter_map(|e| {
            let heard = e.heard.as_deref()?.trim();
            let written = e.text.trim();
            if heard.is_empty() || written.is_empty() {
                return None;
            }
            let context = e
                .scope
                .as_deref()
                .map(|s| {
                    s.split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Some(crate::cleanup::Vocabulary {
                heard: heard.into(),
                written: written.into(),
                enabled: true,
                context,
            })
        })
        .collect()
}

/// Scoped lexical entries as confirmed structured-cleanup entries with
/// explicit-user provenance. Context/style kinds never become replacements;
/// Format-only never receives lexical substitution through this path.
pub fn to_confirmed_vocabulary(
    entries: &[LearnedEntry],
) -> Vec<crate::structured_clean::VocabularyEntry> {
    entries
        .iter()
        .filter(|e| e.enabled && e.kind.is_lexical())
        .filter_map(|e| {
            let scope = normalize_scope(&e.scope)?;
            Some(crate::structured_clean::VocabularyEntry {
                id: Some(e.id.clone()),
                canonical: e.text.trim().into(),
                aliases: vec![e.heard.as_deref()?.trim().into()],
                scope,
                status: crate::structured_clean::VocabularyStatus::Confirmed,
                provenance: crate::structured_clean::VocabularyProvenance::ExplicitUserConfirmation,
            })
        })
        .collect()
}

/// Context/style hints as plain data strings for the context packet. Never
/// fact-edit commands; presentation-only where compatible.
pub fn context_hints(entries: &[LearnedEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| {
            e.enabled
                && matches!(e.kind, LearnKind::Context | LearnKind::Style)
                && !e.text.trim().is_empty()
        })
        .take(10)
        .map(|e| e.text.trim().to_owned())
        .collect()
}

// --- Compat aliases for the concurrent Store draft (same behavior) ---

/// Alias: `learning::validate` == `validate_learning`.
pub fn validate(entries: &[LearnedEntry]) -> Result<(), String> {
    validate_learning(entries)
}

/// Alias: `learning::narrow_correction` == `extract_narrow_correction`.
pub fn narrow_correction(old_text: &str, new_text: &str) -> Option<(String, String)> {
    extract_narrow_correction(old_text, new_text)
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
        .collect()
}

fn locally_supported(raw: &str, heard: &str, scope: Option<&str>) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    let tokens: Vec<_> = words(raw).into_iter().map(str::to_lowercase).collect();
    let raw_tokens: Vec<_> = raw.split_whitespace().collect();
    let alias: Vec<_> = words(heard).into_iter().map(str::to_lowercase).collect();
    let cues: Vec<_> = words(scope).into_iter().map(str::to_lowercase).collect();
    let model_scope = cues.join(" ") == "ai model discussion";
    if alias.is_empty() || cues.is_empty() || (!model_scope && cues.iter().any(|c| c.len() < 3)) {
        return false;
    }
    let mut found = false;
    // Host vocabulary is global within the request. If even one occurrence
    // lacks scope evidence, abstain from this alias for the whole request.
    for (index, window) in tokens.windows(alias.len()).enumerate() {
        if window != alias {
            continue;
        }
        found = true;
        let mut nearby = Vec::new();
        for previous in (index.saturating_sub(4)..index).rev() {
            if raw_tokens[previous].contains(['.', '?', '!', ';']) {
                break;
            }
            nearby.push(tokens[previous].as_str());
        }
        if !raw_tokens[index + alias.len() - 1].contains(['.', '?', '!', ';']) {
            for next in index + alias.len()..(index + alias.len() + 4).min(tokens.len()) {
                nearby.push(tokens[next].as_str());
                if raw_tokens[next].contains(['.', '?', '!', ';']) {
                    break;
                }
            }
        }
        let supported = if model_scope {
            nearby.iter().any(|word| {
                matches!(
                    *word,
                    "model" | "models" | "llm" | "asr" | "checkpoint" | "inference" | "parameters"
                )
            })
        } else {
            cues.iter().all(|cue| nearby.contains(&cue.as_str()))
        };
        if !supported {
            return false;
        }
    }
    found
}

/// Small relevant subset for cleanup, honoring the mode contract:
/// Clean may use scoped lexical corrections; Format-only and As-heard
/// (Original) never receive lexical substitution through this path.
/// Only enabled lexical entries whose heard phrase occurs in `raw` are
/// returned, ranked pinned/support/short under `byte_budget`.
pub fn relevant(
    entries: &[LearnedEntry],
    raw: &str,
    mode: CleanupMode,
    byte_budget: usize,
) -> Vec<VocabularyEntry> {
    if !matches!(mode, CleanupMode::Clean) || byte_budget == 0 || raw.trim().is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<&LearnedEntry> = entries
        .iter()
        .filter(|e| {
            e.enabled
                && e.kind.is_lexical()
                && validate_learned_entry(e).is_ok()
                && e.heard
                    .as_deref()
                    .or_else(|| (e.kind == LearnKind::Canonical).then_some(e.text.as_str()))
                    .is_some_and(|h| {
                        !h.trim().is_empty() && locally_supported(raw, h, e.scope.as_deref())
                    })
                && !e.text.trim().is_empty()
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.user_pinned
            .cmp(&a.user_pinned)
            .then_with(|| b.source_ids.len().cmp(&a.source_ids.len()))
            .then_with(|| a.text.len().cmp(&b.text.len()))
    });
    let mut used = 0usize;
    let mut out = Vec::new();
    for entry in ranked {
        let heard = entry.heard.as_deref().unwrap_or(&entry.text).trim();
        let written = entry.text.trim();
        let cost = heard.len() + written.len() + entry.scope.as_deref().map_or(0, str::len) + 16;
        if used + cost > byte_budget.min(RELEVANT_CHAR_BUDGET) {
            continue;
        }
        used += cost;
        out.push(VocabularyEntry {
            id: entry.id.clone(),
            heard: heard.into(),
            written: written.into(),
            enabled: true,
            scope: normalize_scope(&entry.scope),
        });
        if out.len() >= MAX_RELEVANT_ENTRIES {
            break;
        }
    }
    let conflicts: std::collections::HashSet<_> = out
        .iter()
        .filter(|entry| {
            out.iter().any(|other| {
                other.heard.eq_ignore_ascii_case(&entry.heard) && other.written != entry.written
            })
        })
        .map(|e| e.heard.to_lowercase())
        .collect();
    out.retain(|e| !conflicts.contains(&e.heard.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(heard: &str, text: &str) -> LearnedEntry {
        LearnedEntry {
            id: "abc123".into(),
            kind: LearnKind::Correction,
            text: text.into(),
            heard: Some(heard.into()),
            scope: None,
            source_ids: vec!["rec1".into()],
            source_type: "dictationCorrection".into(),
            enabled: true,
            user_pinned: false,
        }
    }

    #[test]
    fn rejects_numbers_directions_and_paragraphs() {
        assert!(is_rejected_pair("grock", "Grok") == false);
        assert!(!is_rejected_pair("Sono Jot", "SonoJot"));
        assert!(is_rejected_pair("version 4", "version 5"));
        assert!(is_rejected_pair("500", "5000"));
        assert!(is_rejected_pair("go left", "go right"));
        assert!(is_rejected_pair("see you today", "see you tomorrow"));
        assert!(is_rejected_pair("on monday", "on tuesday"));
        assert!(is_rejected_pair("first line\nsecond line", "replacement"));
        assert!(is_rejected_pair(
            "this is a very long rewritten sentence with many changed words entirely",
            "something else"
        ));
        assert!(is_rejected_pair("same", "same"));
        for (heard, written) in [
            ("four", "five"),
            ("enable", "disable"),
            ("can", "cannot"),
            ("send", "save"),
        ] {
            assert!(is_rejected_pair(heard, written));
        }
    }

    #[test]
    fn narrow_extraction_keeps_single_substitution_only() {
        let (h, w) =
            extract_narrow_correction("ask grock about the model", "ask Grok about the model")
                .unwrap();
        assert_eq!((h.as_str(), w.as_str()), ("grock", "Grok"));
        assert!(extract_narrow_correction("same text here", "same text here").is_none());
        // Insertion-only has no reusable alias.
        assert!(extract_narrow_correction("keep this", "keep this please now").is_none());
        // Number change is not learned.
        assert!(extract_narrow_correction(
            "set limit to twelve actually fifteen",
            "set limit to 15"
        )
        .is_none());
        // Paragraph rewrite is not learned.
        assert!(extract_narrow_correction(
            "first".repeat(40).as_str(),
            "second".repeat(40).as_str()
        )
        .is_none());
    }

    #[test]
    fn prune_drops_orphans_but_keeps_pins() {
        let pinned = LearnedEntry {
            user_pinned: true,
            source_type: "manual".into(),
            source_ids: vec![],
            ..entry("grock", "Grok")
        };
        let orphan = LearnedEntry {
            source_ids: vec!["gone".into()],
            ..entry("quinn", "Qwen")
        };
        let kept = LearnedEntry {
            source_ids: vec!["gone".into(), "alive".into()],
            ..entry("acme", "ACME")
        };
        let out = prune_deleted_sources(vec![pinned, orphan, kept], &["gone".into()]);
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .any(|e| e.heard.as_deref() == Some("grock") && e.source_ids.is_empty()));
        assert!(out.iter().any(
            |e| e.heard.as_deref() == Some("acme") && e.source_ids == vec!["alive".to_string()]
        ));
    }

    #[test]
    fn validation_enforces_small_entries() {
        assert!(validate_learned_entry(&entry("grock", "Grok")).is_ok());
        assert!(validate_learned_entry(&LearnedEntry {
            text: "x".repeat(200),
            ..entry("grock", "Grok")
        })
        .is_err());
        let many: Vec<LearnedEntry> = (0..600)
            .map(|i| LearnedEntry {
                id: format!("id{i}"),
                ..entry("grock", "Grok")
            })
            .collect();
        assert!(validate_learning(&many).is_err());
    }

    #[test]
    fn relevant_subset_respects_budget_and_pin_order() {
        let mut entries = Vec::new();
        for i in 0..60 {
            entries.push(LearnedEntry {
                id: format!("id{i:03}"),
                text: "Qwen".into(),
                heard: Some("quinn".into()),
                enabled: true,
                user_pinned: i == 59,
                source_ids: vec!["s".into()],
                ..LearnedEntry::default()
            });
        }
        let picked = select_relevant(&entries, 50);
        assert!(picked.len() <= MAX_RELEVANT_ENTRIES);
        assert!(picked[0].user_pinned);
        let total: usize = picked.iter().map(|e| e.text.len() + 16).sum();
        assert!(total <= RELEVANT_CHAR_BUDGET + 256);
    }

    #[test]
    fn retrieval_requires_local_scope_for_every_occurrence_and_mode() {
        let entries = vec![LearnedEntry {
            scope: Some("project".into()),
            ..entry("Velmora", "Velmorae")
        }];
        assert_eq!(
            relevant(
                &entries,
                "Open the Velmora project",
                CleanupMode::Clean,
                200
            )
            .len(),
            1
        );
        for raw in [
            "Call Velmora tomorrow",
            "Open the Velmora project. Later we went out to meet Velmora",
            "Call Velmora. The project is ready",
            "We finished the project. Call Velmora",
        ] {
            assert!(relevant(&entries, raw, CleanupMode::Clean, 200).is_empty());
        }
        for mode in [CleanupMode::Original, CleanupMode::Format] {
            assert!(relevant(&entries, "Open the Velmora project", mode, 200).is_empty());
        }
        assert!(relevant(&entries, "Open the Velmora project", CleanupMode::Clean, 1).is_empty());
        let disabled = vec![LearnedEntry {
            enabled: false,
            ..entries[0].clone()
        }];
        assert!(relevant(
            &disabled,
            "Open the Velmora project",
            CleanupMode::Clean,
            200
        )
        .is_empty());
        let model = vec![LearnedEntry {
            scope: Some("AI model discussion".into()),
            ..entry("Velmora", "Velmorae")
        }];
        assert_eq!(
            relevant(&model, "Run the Velmora model", CleanupMode::Clean, 200).len(),
            1
        );
        assert!(relevant(&model, "Call Velmora tomorrow", CleanupMode::Clean, 200).is_empty());
    }
}
