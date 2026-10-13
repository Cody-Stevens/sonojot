use super::{models, numbers, render_span, tokens, BioLabel, Class, Span};
use std::ops::Range;

pub trait SpanProposer {
    fn spans(&self, words: &[&str]) -> Vec<Span>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RulesProposer;
impl RulesProposer {
    pub fn spans(&self, words: &[&str]) -> Vec<Span> {
        <Self as SpanProposer>::spans(self, words)
    }
}
/// ROUTED_STRICT: lexicon rules own MODEL/VERSION/AMP; the model owns numeric
/// spans. SAME_CLASS fallback replaces only unrenderable spans of the rule's class.
#[derive(Clone, Copy, Debug)]
pub struct ModelProposer<'a> {
    labels: &'a [BioLabel],
}
impl<'a> ModelProposer<'a> {
    pub fn new(labels: &'a [BioLabel]) -> Self {
        Self { labels }
    }
    pub fn spans(&self, words: &[&str]) -> Vec<Span> {
        <Self as SpanProposer>::spans(self, words)
    }
}
impl SpanProposer for ModelProposer<'_> {
    fn spans(&self, words: &[&str]) -> Vec<Span> {
        if words.len() != self.labels.len() {
            return vec![];
        }

        // Keep original classes even for rejected or displaced spans. A numeric
        // fallback requires every overlap to be unrenderable and the same class.
        let model = complete_bio_spans(self.labels);
        let rules = RulesProposer.spans(words);
        let mut model_covered = vec![false; words.len()];
        let mut model_classes = vec![None; words.len()];
        for span in &model {
            model_classes[span.start..span.end].fill(Some(span.class));
            let before = words[span.start.saturating_sub(3)..span.start].join(" ");
            let after = words[span.end..words.len().min(span.end + 3)].join(" ");
            if render_span(
                &words[span.start..span.end].join(" "),
                span.class,
                Some(&before),
                Some(&after),
            )
            .is_some()
            {
                model_covered[span.start..span.end].fill(true);
            }
        }
        let mut special_covered = vec![false; words.len()];
        let mut routed = Vec::new();
        for span in rules.iter().filter(|span| special_class(span.class)) {
            special_covered[span.start..span.end].fill(true);
            routed.push(*span);
        }
        // A cross-route collision rejects the whole model span. Never trim or
        // split it to rescue a numeric fragment around a special rules span.
        routed.extend(model.into_iter().filter(|span| {
            !special_class(span.class) && !special_covered[span.start..span.end].iter().any(|v| *v)
        }));
        let fallback: Vec<_> = rules
            .into_iter()
            .filter(|span| {
                !special_class(span.class)
                    && !model_covered[span.start..span.end].iter().any(|v| *v)
                    && model_classes[span.start..span.end]
                        .iter()
                        .all(|class| class.is_none_or(|class| class == span.class))
                    && self.labels[span.start..span.end]
                        .iter()
                        .any(|label| *label != BioLabel::O)
            })
            .collect();
        // Replace an unrenderable model span whole; retaining it alongside an
        // overlapping fallback would make the selected batch invalid.
        routed.retain(|span| {
            !fallback
                .iter()
                .any(|rule| span.start < rule.end && rule.start < span.end)
        });
        routed.extend(fallback);
        routed.sort_unstable_by_key(|span| span.start);
        routed
    }
}

fn special_class(class: Class) -> bool {
    matches!(class, Class::Model | Class::Version | Class::Amp)
}

/// Orphan or class-mismatched I tags never start or extend a complete span.
fn complete_bio_spans(labels: &[BioLabel]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < labels.len() {
        let BioLabel::B(class) = labels[i] else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while labels.get(end) == Some(&BioLabel::I(class)) {
            end += 1;
        }
        spans.push(Span {
            start: i,
            end,
            class,
        });
        i = end;
    }
    spans
}

fn clean(w: &str) -> String {
    tokens(w).join(" ")
}
fn numeric_start(w: &str) -> bool {
    let t = tokens(w);
    t.first().is_some_and(|s| {
        numbers::numeric_word(s)
            && !matches!(
                s.as_str(),
                "and" | "hundred" | "thousand" | "million" | "billion" | "trillion"
            )
    })
}
fn plain_number(w: &str) -> bool {
    let t = tokens(w);
    !t.is_empty() && t.iter().all(|s| numbers::numeric_word(s))
}
fn cents_end(words: &[&str], start: usize) -> Option<usize> {
    let mut number_seen = false;
    for (i, word) in words.iter().enumerate().skip(start).take(20) {
        let word = clean(word);
        if matches!(word.as_str(), "cent" | "cents") {
            return number_seen.then_some(i + 1);
        }
        if !plain_number(&word) {
            return None;
        }
        number_seen |= word != "and";
    }
    None
}
fn suppressed(words: &[&str], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let prev = clean(words[i - 1]);
    if matches!(
        prev.as_str(),
        "several" | "few" | "couple" | "many" | "hundreds" | "thousands" | "millions"
    ) {
        return true;
    }
    // Never rescue a suffix of a malformed/approximate number as a new entity.
    if plain_number(words[i - 1]) && !matches!(prev.as_str(), "and" | "a") {
        return true;
    }
    if prev == "and" && i >= 2 && plain_number(words[i - 2]) {
        return true;
    }
    false
}
fn range_end(words: &[&str], start: usize) -> Option<usize> {
    let mut end = start;
    let mut range = false;
    while end < words.len() && end < start + 28 {
        let w = clean(words[end]);
        if w == "to" {
            if end + 1 >= words.len() || !numeric_start(words[end + 1]) {
                break;
            }
            range = true;
        } else if !plain_number(words[end])
            && !matches!(
                w.as_str(),
                "dollar"
                    | "dollars"
                    | "euro"
                    | "euros"
                    | "cent"
                    | "cents"
                    | "percent"
                    | "buck"
                    | "bucks"
                    | "pound"
                    | "pounds"
            )
        {
            break;
        }
        end += 1;
    }
    range.then_some(end)
}
impl SpanProposer for RulesProposer {
    fn spans(&self, words: &[&str]) -> Vec<Span> {
        let mut result = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let first = clean(words[i]);
            let prefix = words[i..words.len().min(i + 5)].join(" ");
            let normalized = clean(&prefix);
            // Common idioms cannot license a numeric/model rewrite.
            if normalized.starts_with("cloud nine")
                || normalized.starts_with("eleventh hour")
                || normalized.starts_with("the fourth of july")
                || normalized.starts_with("fourth of july")
                || first == "codex"
                    && words[i.saturating_sub(3)..words.len().min(i + 12)]
                        .join(" ")
                        .to_lowercase()
                        .contains("museum")
            {
                i += 1;
                continue;
            }
            let model = models::anchor(&prefix)
                && !(matches!(first.as_str(), "oh" | "o" | "0")
                    && i > 0
                    && plain_number(words[i - 1]));
            let version = matches!(first.as_str(), "version" | "v" | ".net" | "python");
            let date = numbers::month(&first).is_some()
                || first == "the"
                || tokens(&first)
                    .last()
                    .is_some_and(|s| numbers::ordinal_word(s).is_some());
            let numeric = numeric_start(words[i]) && !suppressed(words, i);
            let complete_range_end = numeric.then(|| range_end(words, i)).flatten();
            if numeric {
                let mut failed_money_end = None;
                for ci in i + 1..words.len().min(i + 20) {
                    if matches!(
                        clean(words[ci]).as_str(),
                        "dollar" | "dollars" | "euro" | "euros"
                    ) {
                        if let Some(end) = cents_end(words, ci + 1) {
                            if render_span(&words[i..end].join(" "), Class::Money, None, None)
                                .is_none()
                            {
                                failed_money_end = Some(end);
                            }
                        }
                        break;
                    }
                    if !plain_number(words[ci]) {
                        break;
                    }
                }
                if let Some(end) = failed_money_end {
                    i = end;
                    continue;
                }
            }
            let amp = first.len() == 1 && first.as_bytes()[0].is_ascii_alphabetic();
            if !model && !version && !date && !numeric && !amp {
                i += 1;
                continue;
            }
            let before = words[i.saturating_sub(3)..i].join(" ");
            let prev = i
                .checked_sub(1)
                .map(|i| clean(words[i]))
                .unwrap_or_default();
            let cued_year = matches!(prev.as_str(), "in" | "since" | "fiscal" | "year")
                || numbers::month(&prev).is_some();
            let mut best = None;
            for end in (i + 1..=words.len().min(i + 28)).rev() {
                if complete_range_end.is_some_and(|required| end < required) {
                    continue;
                }
                let text = words[i..end].join(" ");
                if text.len() > 2048 {
                    continue;
                }
                let last = clean(words[end - 1]);
                let after = words[end..words.len().min(end + 3)].join(" ");
                let mut classes = Vec::with_capacity(8);
                if model {
                    classes.push(Class::Model);
                }
                if version {
                    classes.push(Class::Version);
                }
                if date {
                    classes.push(Class::Date);
                }
                if amp {
                    classes.push(Class::Amp);
                }
                if numeric {
                    if matches!(
                        last.as_str(),
                        "dollar"
                            | "dollars"
                            | "euro"
                            | "euros"
                            | "cent"
                            | "cents"
                            | "buck"
                            | "bucks"
                            | "pound"
                            | "pounds"
                    ) {
                        classes.push(Class::Money);
                    }
                    if last == "percent" {
                        classes.push(Class::Percent);
                    }
                    if matches!(last.as_str(), "k" | "m" | "x") {
                        classes.push(Class::Magnitude);
                    }
                    if numbers::unit(&last) || numbers::duration(&last) {
                        classes.push(Class::Measure);
                    }
                    if matches!(last.as_str(), "am" | "pm")
                        || prev == "at"
                        || end >= i + 2
                            && matches!(clean(words[end - 2]).as_str(), "a" | "p")
                            && last == "m"
                    {
                        classes.push(Class::Time);
                    }
                    if cued_year {
                        classes.push(Class::Year);
                    }
                    classes.extend([Class::Decimal, Class::Ordinal, Class::Cardinal]);
                }
                for class in classes {
                    if numbers::month(&prev).is_some()
                        && matches!(class, Class::Cardinal | Class::Ordinal | Class::Decimal)
                    {
                        // The preceding month failed to form a complete date;
                        // only a valid year may be recovered after it.
                        continue;
                    }
                    if class == Class::Ordinal
                        && prev == "a"
                        && tokens(&after).first().is_some_and(|w| w == "of")
                    {
                        continue;
                    }
                    if class == Class::Money
                        && end < words.len()
                        && matches!(last.as_str(), "dollar" | "dollars" | "euro" | "euros")
                        && cents_end(words, end).is_some()
                    {
                        // Do not publish the dollar prefix of an invalid cents
                        // clause such as "two dollars and one hundred cents".
                        continue;
                    }
                    if end < words.len()
                        && clean(words[end]) == "and"
                        && (end + 1 == words.len() || clean(words[end + 1]) == "and")
                        && matches!(class, Class::Cardinal | Class::Decimal | Class::Ordinal)
                    {
                        continue;
                    }
                    if class == Class::Measure
                        && matches!(last.as_str(), "gig" | "gigs")
                        && tokens(&after).iter().any(|w| {
                            matches!(
                                w.as_str(),
                                "weekend" | "tonight" | "band" | "concert" | "venue"
                            )
                        })
                    {
                        continue;
                    }
                    // A failed complete numeric expression must not be truncated
                    // just before its next numeric word or magnitude.
                    if end < words.len()
                        && plain_number(words[end])
                        && (clean(words[end]) != "and"
                            || end + 1 < words.len() && numeric_start(words[end + 1]))
                        && matches!(
                            class,
                            Class::Cardinal
                                | Class::Decimal
                                | Class::Year
                                | Class::Ordinal
                                | Class::Version
                                | Class::Model
                                | Class::Date
                        )
                        && !(matches!(class, Class::Model | Class::Version | Class::Date)
                            && numbers::ordinal_word(&clean(words[end])).is_some())
                    {
                        continue;
                    }
                    if render_span(&text, class, Some(&before), Some(&after)).is_some() {
                        best = Some(Span {
                            start: i,
                            end,
                            class,
                        });
                        break;
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            if let Some(span) = best {
                i = span.end;
                result.push(span);
            } else {
                i = complete_range_end.unwrap_or(i + 1);
            }
        }
        result
    }
}

/// Explicit literals are protected before replacing spans. Apostrophes inside
/// words are not quote delimiters. An unclosed code fence protects to EOF.
pub(super) fn protected_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let chars: Vec<_> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let closer = match c {
            '`' => '`',
            '"' => '"',
            '“' => '”',
            '‘' => '’',
            '\'' if i == 0 || !chars[i - 1].1.is_alphanumeric() => '\'',
            _ => {
                i += 1;
                continue;
            }
        };
        let fence = c == '`' && text[start..].starts_with("```");
        let open_len = if fence { 3 } else { c.len_utf8() };
        let tail = &text[start + open_len..];
        let end = if fence {
            tail.find("```").map(|j| start + open_len + j + 3)
        } else {
            tail.char_indices()
                .find(|(j, ch)| {
                    *ch == closer
                        && (closer != '\''
                            || tail[*j + 1..]
                                .chars()
                                .next()
                                .is_none_or(|ch| !ch.is_alphanumeric()))
                })
                .map(|(j, ch)| start + open_len + j + ch.len_utf8())
        };
        if let Some(end) = end.or_else(|| (c == '`').then_some(text.len())) {
            ranges.push(start..end);
            while i < chars.len() && chars[i].0 < end {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    for w in text.split_whitespace() {
        let literal = w.contains("://")
            || w.starts_with("www.")
            || w.contains('@')
            || w.contains(['\\', '/', '_'])
            || re!(r"(?i)\b[a-z0-9-]+\.(?:com|org|net|io|ai|dev|edu|gov|co|app|md|rs|py|js|ts)\b")
                .is_match(w);
        if literal {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            ranges.push(start..start + w.len());
        }
    }
    ranges
}
