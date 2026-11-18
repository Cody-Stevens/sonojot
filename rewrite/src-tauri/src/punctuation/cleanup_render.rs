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
        let previous = (i > 0).then(|| &doc.text[words[i - 1].end..word.start]);
        // Literal content after a deleted word: the pieces standing on their own
        // and the piece glued to this word stay; the deleted word's own piece
        // ("%" after a deleted "20") went with it.
        if let Some(previous) = previous.filter(|g| literal_gap(g) && !(k > 0 && indices[k - 1] == i - 1)) {
            let (_, free, glued) = literal_pieces(previous, false);
            for piece in free {
                out += piece;
                out.push(' ');
            }
            out += glued;
        }
        let literal = literal_gap(gap);
        let spaced = gap.chars().any(char::is_whitespace);
        // A gap inside a protected span (`keep as spoken`) is never edited,
        // as rebuilt_gap leaves it on the rules path.
        let inside_span = i + 1 < words.len()
            && doc.preserved.iter().any(|&(start, stop)| start <= word.end && stop > word.end && stop >= words[i + 1].start);
        if !last && indices[k + 1] == i + 1 && (literal || inside_span || !spaced) {
            // Glued words stay together, and literal content between two kept
            // words is written as it was, as render_gap leaves it on the rules path.
            out += &w;
            out += gap;
            cap = (literal || inside_span) && spaced && gap.contains(['.', '?', '!']);
            keep_case = false;
            continue;
        }
        // An opening quote, bracket or leading apostrophe ("'cause") before
        // this word stays in front of it.
        if let Some(previous) = previous.filter(|g| !literal_gap(g)) {
            out += &gap_sides(previous).2;
        }
        let (before, after, free) = if literal {
            literal_sides(gap, i + 1 == words.len())
        } else {
            let (before, after, _) = gap_sides(gap);
            (before, after, Vec::new())
        };
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
            for piece in free {
                out.push(' ');
                out += piece;
            }
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

/// A literal gap's pieces between whitespace: the piece glued to the word
/// before it, the pieces standing on their own, and the piece glued to the
/// word after it (none at the end of the text).
fn literal_pieces(gap: &str, end: bool) -> (&str, Vec<&str>, &str) {
    let mut pieces: Vec<&str> = gap.split(char::is_whitespace).filter(|p| !p.is_empty()).collect();
    let before = if !gap.starts_with(char::is_whitespace) && !pieces.is_empty() { pieces.remove(0) } else { "" };
    let after = if !end && !gap.ends_with(char::is_whitespace) { pieces.pop().unwrap_or("") } else { "" };
    (before, pieces, after)
}

/// A literal gap after a kept word whose next word is deleted (or after the
/// last word): the piece glued to the kept word stays, split around its
/// editable punctuation as render_gap splits it, and the pieces standing on
/// their own follow. The piece glued to the deleted word goes with it.
fn literal_sides(gap: &str, end: bool) -> (String, String, Vec<&str>) {
    let (glued, free, _) = literal_pieces(gap, end);
    let (mut before, mut after, mut saw_mark) = (String::new(), String::new(), false);
    for ch in glued.chars() {
        if editable_punctuation(ch) {
            saw_mark = true;
        } else if saw_mark {
            after.push(ch);
        } else {
            before.push(ch);
        }
    }
    (before, after, free)
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

#[cfg(test)]
mod literal_gaps {
    use super::*;

    /// Render with no model punctuation, deleting the words listed by index.
    fn render(text: &str, deleted: &[usize]) -> String {
        render_with(text, deleted, false)
    }

    /// `unprotect` clears literal protection first, to reach a deleted word
    /// that has a piece glued to it.
    fn render_with(text: &str, deleted: &[usize], unprotect: bool) -> String {
        let cancel = AtomicBool::new(false);
        let mut doc = Document::new(text, &[], &cancel).unwrap();
        if unprotect {
            for word in &mut doc.words {
                word.protected = false;
            }
        }
        let marks = vec![Mark::None; doc.words.len()];
        let delete: Vec<bool> = (0..doc.words.len()).map(|i| deleted.contains(&i)).collect();
        render_clean(&doc, &marks, &delete).unwrap().0
    }

    #[test]
    fn keeps_literal_content_between_kept_words() {
        assert_eq!(render("which is 20% of it", &[]), "Which is 20% of it.");
        assert_eq!(render("hello \u{1F600} world", &[]), "Hello \u{1F600} world.");
        assert_eq!(render("keep `ten dollars` as spoken", &[]), "Keep `ten dollars` as spoken.");
        assert_eq!(render("keep cafe\u{301} unchanged", &[]), "Keep cafe\u{301} unchanged.");
        assert_eq!(render("The word is `do not delete um`.", &[]), "The word is `do not delete um`.");
    }

    #[test]
    fn model_punctuation_stays_out_of_a_protected_span() {
        let cancel = AtomicBool::new(false);
        let doc = Document::new("The word is `do not delete um`.", &[], &cancel).unwrap();
        let mut marks = vec![Mark::None; doc.words.len()];
        marks[5] = Mark::Comma; // "delete,"
        let delete = vec![false; doc.words.len()];
        assert_eq!(render_clean(&doc, &marks, &delete).unwrap().0, "The word is `do not delete um`.");
    }

    #[test]
    fn a_word_with_a_glued_piece_is_a_protected_literal() {
        // "20%" is a literal, so the renderer keeps it even when the model marks it.
        assert_eq!(render("it is um 20% no 30% of it", &[2, 3, 4]), "It is 20% 30% of it.");
    }

    #[test]
    fn a_piece_glued_to_a_deleted_word_goes_with_it() {
        // Without protection: "um", "20" and "no" go, and "20"'s "%" with them.
        assert_eq!(render_with("it is um 20% no 30% of it", &[2, 3, 4], true), "It is 30% of it.");
        // The kept word's own piece stays when the word after it goes.
        assert_eq!(render("it costs 20% um", &[3]), "It costs 20%.");
    }

    #[test]
    fn a_free_standing_piece_stays_unless_both_neighbours_go() {
        assert_eq!(render("hello um \u{1F600} world", &[1]), "Hello \u{1F600} world.");
        assert_eq!(render("hello \u{1F600} um world", &[1]), "Hello \u{1F600} world.");
        assert_eq!(render("hello um \u{1F600} uh world", &[1, 2]), "Hello world.");
    }

    #[test]
    fn gaps_without_literal_content_render_as_before() {
        assert_eq!(render("um I need the \"blue\" folder", &[0]), "I need the \"blue\" folder.");
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
