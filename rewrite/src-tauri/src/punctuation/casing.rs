//! Per-word casing from a package with a casing label (sonojot-punct-v4 and later).
//!
//! The model reads every word lower-cased, as before, plus two side inputs per
//! word: the recognizer's own case of the word and whether the word starts a
//! separately decoded audio piece. It returns P(lower), P(Capitalised) and
//! P(UPPER). The host applies a choice only when it clears the manifest's
//! threshold for that kind of change. Lowering a capital the recognizer wrote
//! inside a sentence needs the strictest threshold; a capital that is only
//! positional (after a sentence end the model removed, or at a piece start) has
//! its own threshold. Protected words, vocabulary, "I", sentence starts and
//! mixed-case words never reach this code, and only the case of a word changes.
use super::*;

/// Hint ids: 1 + 2 * (hint - 1) + piece_start, hint 1 unknown, 2 lower,
/// 3 Capitalised, 4 UPPER, 5 mixed. Special and mark tokens carry 0.
pub(crate) const CASE_HINT_ENCODING: &str = "Every subword token of a word carries 1 + 2 * (hint - 1) + piece_start, where hint is the recognizer's case of the word (1 unknown, 2 lower, 3 Capitalised, 4 UPPER, 5 mixed) and piece_start is 1 when the word begins a separately decoded audio piece. CLS, SEP and recognizer-mark tokens carry 0. Read case_logits at the same last-subword position as logits.";
pub(crate) const CASE_LABELS: [&str; 3] = ["lower", "Capitalised", "UPPER"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseThresholds {
    /// Add a capital to a lower-case word.
    pub raise: f32,
    /// Write a 2 to 6 letter word in capitals (acronyms).
    pub upper: f32,
    /// Lower a capital that only marks a removed sentence end or a piece start.
    pub lower_positional: f32,
    /// Lower any other capital the recognizer wrote.
    pub lower: f32,
}

impl CaseThresholds {
    pub(crate) fn valid(&self) -> bool {
        [self.raise, self.upper, self.lower_positional, self.lower]
            .iter()
            .all(|t| t.is_finite() && *t > 0.5 && *t <= 1.0)
            && self.lower >= self.raise
    }
}

/// Casing output for one document, aligned to `Document::words`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Casing {
    pub probs: Vec<[f32; 3]>,
    pub bounds: Vec<bool>,
    pub thresholds: CaseThresholds,
}

/// 0 lower, 1 Capitalised, 2 UPPER, 3 mixed; None without letters.
fn class(word: &str) -> Option<u8> {
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    let first = *letters.first()?;
    if letters.iter().all(|c| c.is_lowercase()) {
        return Some(0);
    }
    if letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase()) {
        return Some(2);
    }
    if first.is_uppercase() && letters[1..].iter().all(|c| c.is_lowercase()) {
        return Some(1);
    }
    Some(3)
}

pub(crate) fn word_hint(word: &str) -> i64 {
    class(word).map_or(1, |c| i64::from(c) + 2)
}

pub(crate) fn hint_id(word: &str, piece_start: bool) -> i64 {
    1 + 2 * (word_hint(word) - 1) + i64::from(piece_start)
}

/// Brand and product names whose capitals are inside the word. Applied by rule
/// when a casing package is active; the letters never change.
pub(crate) fn mixed_case(lower: &str) -> Option<&'static str> {
    static FORMS: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    FORMS
        .get_or_init(|| {
            include_str!("../punctuation_mixed_case.txt")
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|form| (form.to_lowercase(), form))
                .collect()
        })
        .get(lower)
        .copied()
}

/// Closed-class words (pronouns, conjunctions, determiners, prepositions,
/// auxiliaries, discourse words). Inside a sentence and outside a capital run
/// their capital is a recognizer artefact, never a name, so it is lowered by rule.
pub(crate) fn function_word(lower: &str) -> bool {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| include_str!("../punctuation_function_words.txt").lines().collect())
        .contains(lower.replace('’', "'").as_str())
}

fn upper_first(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

const CONNECTORS: &[&str] = &["a", "an", "and", "at", "de", "for", "in", "of", "on", "the", "to", "&"];

/// Whether word `index` sits in a run of capitalised words ("Black Lives
/// Matter", "Five Nights at Freddy's"), looking past up to two connector words.
/// Such capitals are never lowered: they are names, whatever the model says.
/// `breaks[i]` is true when punctuation stands before word i; a run never
/// crosses it, so a sentence-final name does not shield the next word.
pub(crate) fn capital_run(words: &[&str], breaks: &[bool], index: usize) -> bool {
    let broken = |a: usize, b: usize| (a.min(b) + 1..=a.max(b)).any(|k| breaks.get(k).copied().unwrap_or(false));
    let capital = |w: &str| {
        w.chars().next().is_some_and(char::is_uppercase)
            && !I_FORMS.contains(&w.to_lowercase().replace('’', "'").as_str())
    };
    let connector = |w: &str| CONNECTORS.contains(&w.to_lowercase().as_str());
    for step in [-1_isize, 1] {
        let mut j = index as isize + step;
        let mut skipped = 0;
        while j >= 0 && (j as usize) < words.len() && connector(words[j as usize]) && skipped < 2 {
            j += step;
            skipped += 1;
        }
        if j >= 0 && (j as usize) < words.len() && capital(words[j as usize]) && !broken(index, j as usize) {
            return true;
        }
    }
    false
}

/// The spelling the model chooses for a word that is not at a sentence start,
/// or None to keep the word as it is. `run`: the word is in a capital run.
pub(crate) fn model_case(word: &str, probs: [f32; 3], positional: bool, run: bool, t: &CaseThresholds) -> Option<String> {
    if word.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let lower = word.to_lowercase();
    if let Some(form) = mixed_case(&lower.replace('’', "'")) {
        return (form != word && form.to_lowercase() == lower).then(|| form.into());
    }
    if !run && class(word) == Some(1) && word.chars().count() > 1 && function_word(&lower) {
        return Some(lower);
    }
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    let acronym = (2..=6).contains(&letters) && probs[2] >= t.upper;
    match class(word)? {
        0 if acronym => Some(word.to_uppercase()),
        0 if probs[1] >= t.raise => Some(upper_first(word)),
        // Rewriting a capital the recognizer chose is as strict as lowering one.
        1 if acronym && probs[2] >= t.lower => Some(word.to_uppercase()),
        // A single capital letter mid-sentence is usually a label ("option A").
        1 if letters == 1 && !positional => None,
        1 if !run && probs[0] >= if positional { t.lower_positional } else { t.lower } => Some(lower),
        2 if !run && probs[0] >= t.lower => Some(lower),
        2 if !run && probs[1] >= t.lower => Some(upper_first(&lower)),
        _ => None,
    }
}

/// The mixed-case form at a sentence start ("iPhone" stays "iPhone").
pub(crate) fn sentence_start_form(word: &str) -> Option<&'static str> {
    mixed_case(&word.to_lowercase().replace('’', "'"))
}

thread_local! {
    static PIECES: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with the recognizer's separately decoded pieces available to the
/// punctuation stage on this thread. The pieces never change any text.
pub(crate) fn with_pieces<R>(pieces: &[&str], f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PIECES.with(|cell| *cell.borrow_mut() = None);
        }
    }
    PIECES.with(|cell| *cell.borrow_mut() = Some(pieces.iter().map(|s| (*s).to_owned()).collect()));
    let _reset = Reset;
    f()
}

/// Piece-start flags for the document words, by aligning them in order to the
/// recognizer's pieces. Cleanup rules may have removed or changed a few words;
/// unmatched words get no flag. Without pieces every flag is false.
pub(crate) fn piece_bounds(words: &[&str]) -> Vec<bool> {
    PIECES.with(|cell| {
        let pieces = cell.borrow();
        let Some(pieces) = pieces.as_ref() else {
            return vec![false; words.len()];
        };
        bounds_for(words, pieces)
    })
}

pub(crate) fn bounds_for(words: &[&str], pieces: &[String]) -> Vec<bool> {
    let mut source: Vec<(String, bool)> = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        for (k, word) in word_regex().find_iter(piece).enumerate() {
            source.push((word.as_str().to_lowercase(), k == 0 && index > 0));
        }
    }
    let mut out = vec![false; words.len()];
    let mut next = 0;
    for (i, word) in words.iter().enumerate() {
        let lower = word.to_lowercase();
        if let Some(offset) = source[next.min(source.len())..].iter().take(8).position(|(w, _)| *w == lower) {
            out[i] = source[next + offset].1;
            next += offset + 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: CaseThresholds = CaseThresholds { raise: 0.7, upper: 0.8, lower_positional: 0.6, lower: 0.97 };

    #[test]
    fn raising_needs_less_than_lowering() {
        assert_eq!(model_case("paris", [0.2, 0.8, 0.0], false, false, &T).as_deref(), Some("Paris"));
        assert_eq!(model_case("Paris", [0.8, 0.2, 0.0], false, false, &T), None);
        assert_eq!(model_case("Paris", [0.8, 0.2, 0.0], true, false, &T).as_deref(), Some("paris"));
        assert_eq!(model_case("Paris", [0.99, 0.01, 0.0], false, false, &T).as_deref(), Some("paris"));
        assert_eq!(model_case("Paris", [0.99, 0.01, 0.0], true, true, &T), None);
        assert_eq!(model_case("api", [0.1, 0.0, 0.9], false, false, &T).as_deref(), Some("API"));
        assert_eq!(model_case("extraordinary", [0.1, 0.0, 0.9], false, false, &T), None);
        assert_eq!(model_case("version2", [0.0, 1.0, 0.0], false, false, &T), None);
        assert_eq!(model_case("Rust", [0.05, 0.1, 0.85], false, false, &T), None);
        assert_eq!(model_case("Nasa", [0.0, 0.01, 0.99], false, false, &T).as_deref(), Some("NASA"));
        assert_eq!(model_case("Because", [0.3, 0.7, 0.0], false, false, &T).as_deref(), Some("because"));
        assert_eq!(model_case("The", [0.3, 0.7, 0.0], false, true, &T), None);
        assert_eq!(model_case("Will", [0.3, 0.7, 0.0], false, false, &T), None);
        assert_eq!(model_case("A", [0.999, 0.001, 0.0], false, false, &T), None);
        assert_eq!(model_case("A", [0.999, 0.001, 0.0], true, false, &T).as_deref(), Some("a"));
        let words = ["we", "support", "Black", "Lives", "Matter", "and", "the", "Elimination", "of", "All", "Forms"];
        let none = [false; 11];
        assert!(capital_run(&words, &none, 2) && capital_run(&words, &none, 4) && capital_run(&words, &none, 7));
        assert!(!capital_run(&["file", "a", "feature", "Request", "for", "the", "export"], &none, 3));
        assert!(!capital_run(&["then", "I", "Said", "it"], &none, 2));
        // "... to Paris. And then": the period breaks the run.
        assert!(!capital_run(&["to", "Paris", "And", "then"], &[false, false, true, false], 2));
    }

    #[test]
    fn mixed_case_forms_come_from_the_list() {
        assert_eq!(model_case("github", [1.0, 0.0, 0.0], false, false, &T).as_deref(), Some("GitHub"));
        assert_eq!(model_case("Iphone", [0.0, 1.0, 0.0], true, false, &T).as_deref(), Some("iPhone"));
        assert_eq!(model_case("GitHub", [1.0, 0.0, 0.0], false, false, &T), None);
        assert_eq!(sentence_start_form("iphone"), Some("iPhone"));
    }

    #[test]
    fn hints_encode_case_and_piece_start() {
        assert_eq!(hint_id("hello", false), 3);
        assert_eq!(hint_id("Hello", true), 6);
        assert_eq!(hint_id("NASA", false), 7);
        assert_eq!(hint_id("GitHub", false), 9);
        assert_eq!(hint_id("42", true), 2);
    }

    /// Invented sentences only. `p` gives the model's probabilities by word.
    fn rebuilt(text: &str, vocabulary: &[Vocabulary], p: &dyn Fn(&str) -> [f32; 3], bounds: &[usize], clean: bool) -> String {
        let cancel = AtomicBool::new(false);
        let mut doc = Document::new(text, vocabulary, &cancel).unwrap();
        let n = doc.words.len();
        let probs = doc.words.iter().map(|w| p(&w.text.to_lowercase())).collect();
        let bounds = (0..n).map(|i| bounds.contains(&i)).collect();
        doc.casing = Some(Casing { probs, bounds, thresholds: T });
        let mut marks = vec![Mark::None; n];
        marks[n - 1] = Mark::Period;
        if clean {
            render_clean(&doc, &marks, &vec![false; n]).unwrap().0
        } else {
            doc.rebuild_with_case(Some(&marks), &cancel, true).unwrap()
        }
    }

    fn model(word: &str) -> [f32; 3] {
        match word {
            "maria" | "paris" => [0.1, 0.9, 0.0],
            "nasa" => [0.0, 0.01, 0.99],
            "summit" | "request" => [0.9, 0.1, 0.0],
            "apple" => [0.999, 0.001, 0.0],
            _ => [0.999, 0.001, 0.0],
        }
    }

    #[test]
    fn renderers_apply_the_model_choice_with_asymmetric_thresholds() {
        for clean in [false, true] {
            assert_eq!(rebuilt("we met maria in paris", &[], &model, &[], clean), "We met Maria in Paris.");
            assert_eq!(rebuilt("i think nasa uses github", &[], &model, &[], clean), "I think NASA uses GitHub.");
            // A mid-sentence capital survives a moderately confident "lower".
            assert_eq!(rebuilt("the Summit starts now", &[], &model, &[], clean), "The Summit starts now.");
            // A very confident one is lowered, unless it is part of a capital run.
            assert_eq!(rebuilt("we ate an Apple today", &[], &model, &[], clean), "We ate an apple today.");
            assert_eq!(rebuilt("we ate an Apple Pie today", &[], &model, &[], clean), "We ate an Apple Pie today.");
            // A capital after a removed recognizer period or at a piece start is positional.
            assert_eq!(rebuilt("we need the. Summit", &[], &model, &[], clean), "We need the summit.");
            assert_eq!(rebuilt("file a feature Request now", &[], &model, &[3], clean), "File a feature request now.");
            assert_eq!(rebuilt("file a feature Request now", &[], &model, &[], clean), "File a feature Request now.");
            // iPhone keeps its form at a sentence start.
            assert_eq!(rebuilt("iphone sales grew", &[], &model, &[], clean), "iPhone sales grew.");
        }
    }

    #[test]
    fn vocabulary_and_protected_words_never_change_case() {
        let vocabulary = [Vocabulary {
            kind: crate::domain::VocabularyKind::Replace, aliases: Vec::new(),
            heard: "Apple".into(), written: "Apple".into(), enabled: true, context: vec![],
        }];
        for clean in [false, true] {
            assert_eq!(rebuilt("we ate an Apple today", &vocabulary, &model, &[], clean), "We ate an Apple today.");
            assert_eq!(rebuilt("see Example_Name and api.Example.com", &[], &model, &[], clean),
                "See Example_Name and api.Example.com.");
        }
    }

    #[test]
    fn bounds_follow_pieces_through_small_edits() {
        let pieces = vec!["um we met at the".to_owned(), "Summit today".to_owned(), "and then left".to_owned()];
        let words = ["We", "met", "at", "the", "Summit", "today", "and", "then", "left"];
        assert_eq!(bounds_for(&words, &pieces), vec![false, false, false, false, true, false, true, false, false]);
        let flags = with_pieces(&["one two", "Three"], || piece_bounds(&["one", "two", "Three"]));
        assert_eq!(flags, vec![false, false, true]);
        assert_eq!(piece_bounds(&["one"]), vec![false]);
    }
}
