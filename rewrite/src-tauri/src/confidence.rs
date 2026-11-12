//! Local recognizer confidence; never guess a word alignment.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordConfidence {
    pub word: String,
    pub probability: f64,
}
/// Markers did not help Opus 5.5 with the app prompt; 2026-11-01.
/// Keep confidence capture and storage, but send plain text to every model.
pub fn hints_help(_model: &str) -> bool {
    false
}
pub(crate) fn words(text: &str) -> Vec<(usize, usize)> {
    let re = regex::Regex::new(r"[\p{L}\p{N}](?:\S*[\p{L}\p{N}])?").unwrap();
    re.find_iter(text).map(|m| (m.start(), m.end())).collect()
}
pub fn from_tokens(text: &str, tokens: &serde_json::Value) -> Vec<WordConfidence> {
    let Some(tokens) = tokens.as_array() else {
        return vec![];
    };
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    for token in tokens {
        let Some(lp) = token["logprob"]
            .as_f64()
            .filter(|v| v.is_finite() && *v <= 0.)
        else {
            return vec![];
        };
        let start = bytes.len();
        if let Some(raw) = token["bytes"].as_array() {
            for b in raw {
                let Some(b) = b.as_u64().filter(|b| *b <= 255) else {
                    return vec![];
                };
                bytes.push(b as u8);
            }
        } else if let Some(t) = token["token"].as_str() {
            bytes.extend_from_slice(t.as_bytes());
        } else {
            return vec![];
        }
        spans.push((start, bytes.len(), lp.exp()));
    }
    if bytes != text.as_bytes() {
        return vec![];
    }
    let speech = crate::sherpa::qwen_transcript(text);
    let offset = text.len() - speech.len();
    regex::Regex::new(r"\S+")
        .unwrap()
        .find_iter(speech)
        .filter_map(|outer| {
            let cores = words(outer.as_str());
            let &(a, b) = cores.first()?;
            let ps: Vec<_> = spans
                .iter()
                .filter(|(s, e, _)| *s < offset + outer.end() && *e > offset + outer.start())
                .map(|(_, _, p)| *p)
                .collect();
            (!ps.is_empty()).then(|| WordConfidence {
                word: outer.as_str()[a..b].into(),
                probability: ps.iter().sum::<f64>() / ps.len() as f64,
            })
        })
        .collect()
}
/// Only identical lexical sequences are projected. Case and punctuation may
/// change, but any insertion, deletion, number rendering or rewrite drops all
/// markers. This deliberately sacrifices coverage rather than misaligning repeats.
pub fn project(source: &[WordConfidence], text: &str) -> Vec<WordConfidence> {
    let ranges = words(text);
    if source.len() != ranges.len()
        || source
            .iter()
            .zip(&ranges)
            .any(|(s, (a, b))| s.word.to_lowercase() != text[*a..*b].to_lowercase())
    {
        return vec![];
    }
    source
        .iter()
        .zip(ranges)
        .map(|(s, (a, b))| WordConfidence {
            word: text[a..b].into(),
            probability: s.probability,
        })
        .collect()
}
/// Cleanup deletes fillers and written form rewrites numbers, so the text the
/// proofreader sees rarely matches recognition word for word. Align the two by
/// longest common subsequence and keep a match only where it is unambiguous:
/// inside a run of at least two consecutive matches, or a word that occurs once
/// on each side. Unmatched words simply carry no confidence.
pub(crate) fn align(source: &[WordConfidence], text: &str) -> Vec<Option<f64>> {
    let ranges = words(text);
    let target: Vec<String> = ranges
        .iter()
        .map(|(a, b)| text[*a..*b].to_lowercase())
        .collect();
    let src: Vec<String> = source.iter().map(|s| s.word.to_lowercase()).collect();
    let (n, m) = (src.len(), target.len());
    let mut out = vec![None; m];
    if n == 0 || m == 0 || n.saturating_mul(m) > 4_200_000 {
        return out;
    }
    // Suffix LCS lengths; u16 is enough because MAX_WORDS bounds both sides.
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if src[i] == target[j] {
                dp[at(i + 1, j + 1)] + 1
            } else {
                dp[at(i + 1, j)].max(dp[at(i, j + 1)])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if src[i] == target[j] && dp[at(i, j)] == dp[at(i + 1, j + 1)] + 1 {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let count = |list: &[String], w: &str| list.iter().filter(|x| *x == w).count();
    for (k, &(i, j)) in pairs.iter().enumerate() {
        let joined = |a: Option<&(usize, usize)>, b: Option<&(usize, usize)>| matches!((a, b), (Some(x), Some(y)) if y.0 == x.0 + 1 && y.1 == x.1 + 1);
        let in_run = joined(k.checked_sub(1).and_then(|p| pairs.get(p)), pairs.get(k))
            || joined(pairs.get(k), pairs.get(k + 1));
        if in_run || (count(&src, &src[i]) == 1 && count(&target, &target[j]) == 1) {
            out[j] = Some(source[i].probability);
        }
    }
    out
}
pub fn mark(source: &[WordConfidence], text: &str) -> String {
    let mapped = align(source, text);
    let known: Vec<usize> = (0..mapped.len()).filter(|i| mapped[*i].is_some()).collect();
    // Too little survives alignment to rank words fairly: send no hints.
    if known.is_empty() || known.len() * 2 < mapped.len() {
        return text.into();
    }
    let mut ranked = known.clone();
    ranked.sort_by(|a, b| {
        mapped[*a]
            .unwrap()
            .total_cmp(&mapped[*b].unwrap())
            .then(a.cmp(b))
    });
    ranked.truncate(known.len().div_ceil(5));
    let mut out = String::new();
    let mut end = 0;
    for (i, (a, b)) in words(text).into_iter().enumerate() {
        out.push_str(&text[end..a]);
        if ranked.contains(&i) {
            out.push('\u{27e6}');
        }
        out.push_str(&text[a..b]);
        if ranked.contains(&i) {
            out.push('\u{27e7}');
        }
        end = b;
    }
    out.push_str(&text[end..]);
    out
}
pub fn strip(text: &str) -> String {
    text.replace(['\u{27e6}', '\u{27e7}'], "")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_tokens_and_projection() {
        let t = serde_json::json!([{"token":"hello", "logprob":-0.1},{"token":" world", "logprob":-1.0}]);
        let c = from_tokens("hello world", &t);
        assert_eq!(mark(&c, "Hello, world!"), "Hello, \u{27e6}world\u{27e7}!");
        assert!(project(&c, "hello new world").is_empty());
        assert!(project(&c, "world hello").is_empty());
        assert!(project(&c, "hello-world").is_empty());
        let mut missing = false;
        let mut accumulated = Vec::new();
        append(&mut accumulated, &mut missing, "unknown", vec![]);
        assert!(missing);
        assert!(from_tokens("different", &t).is_empty());
        assert_eq!(strip("\u{27e6}world\u{27e7}"), "world");
    }
    #[test]
    fn cleanup_deletions_and_rewrites_keep_hints_on_matched_words() {
        let c: Vec<_> = [
            ("so", 0.3),
            ("um", 0.1),
            ("the", 0.9),
            ("report", 0.95),
            ("is", 0.9),
            ("due", 0.9),
            ("on", 0.9),
            ("tuesday", 0.2),
            ("ten", 0.9),
            ("k", 0.9),
        ]
        .iter()
        .map(|(w, p)| WordConfidence {
            word: (*w).into(),
            probability: *p,
        })
        .collect();
        // Filler deleted, number rewritten: "Tuesday" still gets its hint.
        let marked = mark(&c, "So the report is due on Tuesday, 10K.");
        assert!(marked.contains("\u{27e6}Tuesday\u{27e7}"));
        assert!(!marked.contains("\u{27e6}10K"));
        // A doubled word removed by cleanup is ambiguous, so it is not matched alone.
        let d: Vec<_> = [("the", 0.1), ("the", 0.9), ("plan", 0.9)]
            .iter()
            .map(|(w, p)| WordConfidence {
                word: (*w).into(),
                probability: *p,
            })
            .collect();
        assert_eq!(align(&d, "The plan"), vec![None, Some(0.9)]);
        // Mostly rewritten text gets no hints at all.
        assert_eq!(
            mark(&c, "Completely different words here now."),
            "Completely different words here now."
        );
    }
}

/// A missing decode must not shift later confidence onto an earlier repeated word.
pub fn append(
    target: &mut Vec<WordConfidence>,
    missing: &mut bool,
    text: &str,
    incoming: Vec<WordConfidence>,
) {
    if !words(text).is_empty() && project(&incoming, text).is_empty() {
        *missing = true;
    }
    target.extend(incoming);
}

#[cfg(test)]
mod hint_policy_tests {
    #[test]
    fn markers_are_disabled_for_every_provider() {
        for model in ["anthropic/claude-opus-5.5", "anthropic/claude-sonnet-5.5", "anthropic/claude-haiku-5.5", "openai/gpt-6-luna", "openai/gpt-6-luna-pro", "local/model", "", "future/model"] {
            assert!(!super::hints_help(model));
        }
    }
}
