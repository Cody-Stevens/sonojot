use super::*;

// Word deletion and punctuation transfer follow the v4 reference render_clean.py.
const DELETE_INTRO: &[&str] = &[
    "again", "alright", "also", "anyway", "finally", "first", "hello", "hey", "hi", "honestly",
    "however", "no", "nope", "now", "oh", "ok", "okay", "please", "right", "second", "so", "sure",
    "thanks", "third", "well", "yeah", "yep", "yes",
];
const DELETE_CONTINUES: &[&str] = &[
    "a", "about", "also", "am", "an", "and", "any", "are", "as", "at", "be", "because", "been",
    "being", "both", "but", "by", "can", "could", "each", "either", "every", "for", "from", "get",
    "going", "gonna", "he", "he's", "her", "his", "i", "i'll", "i'm", "if", "in", "into", "is",
    "it's", "its", "just", "let's", "like", "make", "may", "might", "more", "most", "must", "my",
    "need", "neither", "no", "nor", "not", "of", "on", "onto", "or", "our", "really", "shall",
    "she", "she's", "should", "so", "some", "such", "than", "that", "that's", "the", "their",
    "then", "there's", "they", "they're", "to", "trying", "unless", "until", "very", "wanna",
    "want", "was", "we", "we'll", "we're", "were", "when", "where", "whether", "which", "while",
    "who", "whose", "will", "with", "would", "you'll", "you're", "your",
];
const DELETE_CORRECTION_MARKERS: &[&str] =
    &["actually", "mean", "no", "oops", "rather", "sorry", "wait"];
const DELETE_OPENERS: &[&str] = &[
    "alright",
    "also",
    "anyway",
    "anyways",
    "besides",
    "btw",
    "hey",
    "meanwhile",
    "oh",
    "ok",
    "okay",
    "separately",
    "thanks",
];
const DELETE_CLAUSE_START: &[&str] = &[
    "and", "because", "but", "he", "he's", "i", "i'd", "i'll", "i'm", "i've", "if", "it", "it'll",
    "it's", "or", "she", "she's", "so", "that's", "then", "there", "there's", "these", "they",
    "they'll", "they're", "they've", "this", "those", "we", "we'll", "we're", "we've", "when",
    "where", "which", "who", "you", "you'll", "you're", "you've",
];

pub(super) fn render_clean(
    doc: &Document<'_>,
    marks: &[Mark],
    delete: &[bool],
) -> Result<(String, Vec<bool>)> {
    let words = &doc.words;
    if marks.len() != words.len() || delete.len() != words.len() {
        return Err(Failure::Output);
    }
    let mut delete = delete.to_vec();
    // Protected literals and vocabulary are retained, then glued words are kept together.
    for (i, word) in words.iter().enumerate() {
        if word.protected {
            delete[i] = false;
        }
    }
    let mut start = 0;
    for end in 1..=words.len() {
        if end == words.len()
            || doc.text[words[end - 1].end..words[end].start]
                .chars()
                .any(char::is_whitespace)
        {
            let full = delete[start..end].iter().all(|d| *d);
            delete[start..end].fill(full);
            start = end;
        }
    }
    let lower: Vec<_> = words
        .iter()
        .map(|w| w.text.to_lowercase().replace('’', "'"))
        .collect();
    for i in 0..words.len().saturating_sub(1) {
        if matches!(
            (lower[i].as_str(), lower[i + 1].as_str()),
            ("you", "know") | ("i", "mean") | ("sort", "of") | ("kind", "of")
        ) && delete[i] != delete[i + 1]
        {
            delete[i] = false;
            delete[i + 1] = false;
        }
    }
    let kept: Vec<_> = delete.iter().map(|d| !d).collect();
    let indices: Vec<_> = (0..words.len()).filter(|&i| kept[i]).collect();
    if indices.is_empty() {
        return Ok((String::new(), kept));
    }
    let mut labels = marks.to_vec();
    let mut i = 0;
    while i < words.len() {
        if !delete[i] {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < words.len() && delete[j + 1] {
            j += 1;
        }
        if i > 0 {
            let p = i - 1;
            let n = j + 1;
            let m = labels[j];
            if n == words.len() {
                if m == Mark::Question {
                    labels[p] = m;
                }
            } else if !labels[p].terminal() {
                if m.terminal() {
                    let keep_break = !lower[i..=j]
                        .iter()
                        .any(|w| DELETE_CORRECTION_MARKERS.contains(&w.as_str()))
                        && lower[n] != lower[i]
                        && DELETE_OPENERS.contains(&lower[n].as_str())
                        && !DELETE_CONTINUES.contains(&lower[p].as_str());
                    if matches!(labels[p], Mark::Comma | Mark::Semicolon)
                        || (labels[p] == Mark::None && keep_break)
                    {
                        labels[p] = m;
                    }
                } else if labels[p] == Mark::Comma
                    && !DELETE_INTRO.contains(&lower[p].as_str())
                    && !DELETE_CLAUSE_START.contains(&lower[n].as_str())
                {
                    labels[p] = Mark::None;
                }
            }
        }
        i = j + 1;
    }
    let names: HashSet<_> = words
        .iter()
        .enumerate()
        .filter(|(i, w)| {
            *i > 0
                && w.text.chars().next().is_some_and(char::is_uppercase)
                && !doc.text[words[*i - 1].end..w.start].contains(['.', '?', '!', ':', '"', '“'])
        })
        .map(|(_, w)| w.text)
        .collect();
    let mut out = doc.text[..words[0].start].to_owned();
    let mut cap = true;
    // After an abbreviation's own period ("etc.", "U.S.", "p.m.") the next
    // word keeps the recognizer's casing, as in rebuild_gaps.
    let mut keep_case = false;
    for (k, &i) in indices.iter().enumerate() {
        let word = &words[i];
        let mut w = word.text.to_owned();
        let i_form = matches!(lower[i].as_str(), "i" | "i'm" | "i'll" | "i've" | "i'd");
        if !word.protected && doc.casing.is_some() && !i_form && cap {
            w = match super::casing::sentence_start_form(&w) {
                Some(form) => form.into(),
                None => {
                    let first = w.chars().next().unwrap();
                    first.to_uppercase().collect::<String>() + &w[first.len_utf8()..]
                }
            };
        } else if let (false, false, Some(casing)) = (word.protected, i_form, &doc.casing) {
            // The casing label decides; the word lists below are the fallback
            // for packages without one.
            let positional = !keep_case
                && (casing.bounds.get(i).copied().unwrap_or(false)
                    || (i > 0 && doc.text[words[i - 1].end..word.start].contains(['.', '?', '!'])));
            let texts: Vec<&str> = words.iter().map(|w| w.text).collect();
            let breaks: Vec<bool> = (0..texts.len())
                .map(|k| k > 0 && doc.gap(k - 1).contains(['.', '?', '!', ',', ';', ':']))
                .collect();
            let run = super::casing::capital_run(&texts, &breaks, i);
            if let Some(form) = super::casing::model_case(&w, casing.probs[i], positional, run, &casing.thresholds) {
                w = form;
            }
        } else if !word.protected {
            if i_form || cap {
                let first = w.chars().next().unwrap();
                w = first.to_uppercase().collect::<String>() + &w[first.len_utf8()..];
            // Same rule as rebuild_gaps: never lowercase an unfamiliar
            // title-cased word just because a sentence end was removed.
            } else if !keep_case
                && !names.contains(word.text)
                && !w.chars().skip(1).any(char::is_uppercase)
                && common_lowercase(&lower[i])
                && !doc.proper.contains(&lower[i])
            {
                w = w.to_lowercase();
            }
        }
        let gap = if i + 1 < words.len() {
            &doc.text[word.end..words[i + 1].start]
        } else {
            &doc.text[word.end..]
        };
        let last = k + 1 == indices.len();
        if !last && indices[k + 1] == i + 1 && !gap.chars().any(char::is_whitespace) {
            out += &w;
            out += gap;
            cap = false;
            keep_case = false;
            continue;
        }
        // An opening quote, bracket or leading apostrophe ("'cause") before
        // this word stays in front of it.
        if i > 0 {
            out += &gap_sides(&doc.text[words[i - 1].end..word.start]).2;
        }
        let (before, after, _) = gap_sides(gap);
        out += &w;
        // An abbreviation keeps its own period whatever the label says, as
        // rebuilt_gap keeps it on the rules path ("e.g. apples", "etc. for").
        let abbreviation = !last && doc.abbreviation(i);
        let final_gap = doc.text[words.last().unwrap().end..].trim();
        if last && !labels[i].terminal() && i + 1 == words.len() && !final_gap.is_empty() {
            out += final_gap;
        } else {
            out += &before;
            out += if abbreviation {
                "."
            } else if last && !labels[i].terminal() {
                "."
            } else {
                labels[i].text()
            };
            out += &after;
        }
        if !last {
            out.push(' ');
        }
        // The models label an abbreviation's own period "." too; it ends the
        // sentence only on the same evidence the rules path uses.
        cap = if abbreviation {
            doc.abbreviation_ends(i, gap, Some(labels[i]))
        } else {
            labels[i].terminal()
        };
        keep_case = abbreviation && !cap;
    }
    Ok((cleanup_house_style(&out), kept))
}

/// Quotes, brackets and apostrophes in a gap, by the side they attach to:
/// before the mark, after it, and in front of the next word. Closing marks
/// and possessive apostrophes ("the dogs' bowls") stay on the left.
fn gap_sides(gap: &str) -> (String, String, String) {
    let split = gap.find(char::is_whitespace).unwrap_or(gap.len());
    let (mut before, mut after, mut opening) = (String::new(), String::new(), String::new());
    let mut saw_mark = false;
    for ch in gap[..split].chars() {
        if editable_punctuation(ch) {
            saw_mark = true;
        } else if matches!(ch, '"' | '\'' | '“' | '”' | '‘' | '’' | '(' | ')' | '[' | ']' | '{' | '}') {
            if saw_mark { after.push(ch) } else { before.push(ch) }
        }
    }
    for ch in gap[split..].chars() {
        if editable_punctuation(ch) {
            saw_mark = true;
        } else if matches!(ch, ')' | ']' | '}' | '”') {
            if saw_mark { after.push(ch) } else { before.push(ch) }
        } else if matches!(ch, '"' | '\'' | '“' | '‘' | '’' | '(' | '[' | '{') {
            opening.push(ch);
        }
    }
    (before, after, opening)
}

#[cfg(test)]
mod cleanup_v4_parity {
    use super::*;
    #[test]
    fn synthetic_python_renderer_parity() {
        let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../evaluation/cleanup-v4/synthetic.json"
        ))
        .unwrap();
        let cancel = AtomicBool::new(false);
        let mut equal = 0;
        for row in &rows {
            let text = row["text"].as_str().unwrap();
            let doc = Document::new(text, &[], &cancel).unwrap();
            let marks: Vec<_> = row["punct"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| match m.as_str().unwrap() {
                    "." => Mark::Period,
                    "," => Mark::Comma,
                    "?" => Mark::Question,
                    ";" => Mark::Semicolon,
                    _ => Mark::None,
                })
                .collect();
            let delete: Vec<bool> = serde_json::from_value(row["delete"].clone()).unwrap();
            let (actual, _) = render_clean(&doc, &marks, &delete).unwrap();
            assert_eq!(
                actual,
                row["expected"].as_str().unwrap(),
                "synthetic case {equal}"
            );
            equal += 1;
        }
        assert_eq!(equal, rows.len());
    }
}

fn cleanup_house_style(text: &str) -> String {
    let mut dashes = String::new();
    let chars: Vec<_> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\u{2013}'
            && !(i > 0
                && i + 1 < chars.len()
                && chars[i - 1].is_ascii_digit()
                && chars[i + 1].is_ascii_digit())
        {
            dashes.push('\u{2014}');
        } else {
            dashes.push(c);
        }
    }
    let out = Regex::new(r"\s*(?:\x{2014}|--)\s*")
        .unwrap()
        .replace_all(&dashes, ", ");
    let out = Regex::new(r",\s*,").unwrap().replace_all(&out, ",");
    let out = Regex::new(r",\s*([.?!;:])")
        .unwrap()
        .replace_all(&out, "$1");
    let out = Regex::new(r"([.?!;:])\s*,")
        .unwrap()
        .replace_all(&out, "$1");
    Regex::new(r"^\s*,\s*")
        .unwrap()
        .replace_all(&out, "")
        .into_owned()
}
