//! Explicit user-confirmed heard forms and their written-form guard.
use super::{
    check_budget, edit_with_revision, protected_lexeme, tokens_with_budget, AppliedEdit,
    Checkpoint, Vocabulary,
};

/// The alias spelling is explicit user confirmation, never a fuzzy guess.
pub(crate) fn valid_alias(alias: &str) -> bool {
    let words: Vec<_> = alias
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    alias.len() <= 250
        && (1..=4).contains(&words.len())
        && alias
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '.'))
        && alias.chars().next().is_some_and(char::is_alphanumeric)
        && alias.chars().last().is_some_and(char::is_alphanumeric)
}

pub(super) fn confirmed_alias_edits(
    source: &str,
    vocabulary: &[Vocabulary],
    revision: &str,
    budget: Option<&dyn Checkpoint>,
) -> Result<Vec<AppliedEdit>, String> {
    if !vocabulary
        .iter()
        .any(|entry| entry.enabled && entry.kind != crate::domain::VocabularyKind::Hint && !entry.aliases.is_empty())
    {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let tokens = tokens_with_budget(source, budget)?;
    for entry in vocabulary
        .iter()
        .filter(|v| v.enabled && v.kind != crate::domain::VocabularyKind::Hint && !v.written.is_empty())
    {
        for alias in &entry.aliases {
            check_budget(budget)?;
            if !valid_alias(alias) {
                continue;
            }
            let parts: Vec<_> = alias
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            if parts.iter().any(|w| protected_lexeme(w))
                || entry.written.split_whitespace().any(protected_lexeme)
            {
                continue;
            }
            let pattern = alias
                .split('.')
                .map(|part| {
                    part.split([' ', '-'])
                        .filter(|w| !w.is_empty())
                        .map(|word| {
                            word.chars()
                                .map(|c| regex::escape(&c.to_string()))
                                .collect::<Vec<_>>()
                                .join(r"[\s-]*")
                        })
                        .collect::<Vec<_>>()
                        .join(r"[\s-]*")
                })
                .collect::<Vec<_>>()
                .join(r"(?:\s+dot\s+|\.)");
            let Ok(matcher) = regex::RegexBuilder::new(&format!(r"\b{pattern}\b"))
                .case_insensitive(true)
                .build()
            else {
                continue;
            };
            for found in matcher.find_iter(source) {
                check_budget(budget)?;
                if found.as_str().contains(['\n', '\r', '!', '?']) {
                    continue;
                }
                let start = found.start();
                let end = found.end();
                // Quotes and code remain literal. A full confirmed URL alias may replace a URL.
                if tokens.iter().any(|t| {
                    t.core_start < end
                        && t.core_end > start
                        && t.literal
                        && !(alias.contains('.')
                            && start <= t.core_start
                            && end >= t.core_end
                            && !source[t.start..t.end].contains(['`', '\"', '\'']))
                }) {
                    continue;
                }
                // The bundled lowercase list excludes personal and place names.
                // Keep these known recognition collisions context-gated too.
                let ambiguous = parts.len() == 1
                    && (crate::punctuation::common_lowercase(&alias.to_lowercase())
                        || matches!(alias.to_lowercase().as_str(), "quinn" | "seoul" | "jason"));
                if ambiguous {
                    let nearby = tokens
                        .iter()
                        .enumerate()
                        .find(|(_, t)| t.core_start <= start && t.core_end > start)
                        .is_some_and(|(index, _)| {
                            tokens[index.saturating_sub(4)..(index + 5).min(tokens.len())]
                                .iter()
                                .any(|t| {
                                    let gap = if t.core_end <= start {
                                        &source[t.core_end..start]
                                    } else if t.core_start >= end {
                                        &source[end..t.core_start]
                                    } else {
                                        return false;
                                    };
                                    !gap.contains(['.', '!', '?', '\n', '\r'])
                                        && entry.context.iter().any(|c| {
                                            c.eq_ignore_ascii_case(
                                                &source[t.core_start..t.core_end],
                                            )
                                        })
                                })
                        });
                    if !nearby {
                        continue;
                    }
                }
                candidates.push(edit_with_revision(
                    source,
                    start,
                    end,
                    entry.written.clone(),
                    "confirmed_vocabulary_occurrence",
                    revision.to_owned(),
                ));
            }
        }
    }
    // Conflicting destinations abstain. Longest confirmed phrase wins over a contained alias.
    candidates.sort_by_key(|e| (e.start_byte, std::cmp::Reverse(e.end_byte)));
    let mut edits: Vec<AppliedEdit> = Vec::new();
    for edit in &candidates {
        if candidates.iter().any(|other| {
            other.start_byte == edit.start_byte
                && other.end_byte == edit.end_byte
                && other.replacement != edit.replacement
        }) {
            continue;
        }
        if edits
            .last()
            .is_none_or(|last| last.end_byte <= edit.start_byte)
        {
            edits.push(edit.clone());
        }
    }
    Ok(edits)
}

pub(super) fn guard_alias_written_forms(
    source: &str,
    proposal: &str,
    vocabulary: &[Vocabulary],
) -> Result<(), String> {
    for entry in vocabulary
        .iter()
        .filter(|v| v.enabled && !v.aliases.is_empty() && !v.written.is_empty())
    {
        let pattern = format!(r"\b{}\b", regex::escape(&entry.written));
        let Ok(matcher) = regex::Regex::new(&pattern) else {
            continue;
        };
        if matcher.find_iter(source).count() != matcher.find_iter(proposal).count() {
            return Err("Cleanup changed a confirmed vocabulary written form.".into());
        }
    }
    Ok(())
}
