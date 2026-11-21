//! Conservative transcript cleanup. Every pass either removes text the speaker did not say
//! (model control tokens, non-speech captions, decoder loops) or changes only case and spacing.
//! English then goes through SonoJot's spoken-to-written rules ([`crate::written_form`]), which
//! leave quotes, code, links and anything ambiguous untouched.

use std::sync::OnceLock;

use regex::Regex;

/// A word group repeated back to back this many times is a decoder loop, not speech. Matches
/// SonoJot's desktop collapse threshold.
pub const LOOP_REPETITIONS: usize = 8;
const LONGEST_LOOP_WORDS: usize = 6;

fn control_tokens() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // SenseVoice and Whisper special tokens: <|en|>, <|NEUTRAL|>, <|nospeech|>, <|endoftext|>…
    RE.get_or_init(|| Regex::new(r"<\|[^|<>]{0,32}\|>").expect("control token regex"))
}

fn non_speech() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)[\[(（【]\s*(?:blank[ _]audio|no[ _]speech|silence|music(?:\s+playing)?|background\s+(?:music|noise)|noise|inaudible|applause|laughter|laughs?|coughs?|coughing|sighs?|breathing|static|wind|beep(?:s|ing)?|sound|sounds)\s*[\])）】]|[♪♫🎵🎶]+",
        )
        .expect("non-speech regex")
    })
}

/// Cleans one recognizer output: control tokens and non-speech captions removed, whitespace
/// collapsed.
pub fn clean_segment(raw: &str) -> String {
    let text = control_tokens().replace_all(raw, " ");
    let text = non_speech().replace_all(&text, " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Joins segment texts with single spaces.
pub fn join<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    parts.into_iter().flat_map(str::split_whitespace).collect::<Vec<_>>().join(" ")
}

/// The finished transcript: loops collapsed, all-caps output recased, spacing tidied and, for
/// English when `written_form` is set, numbers, dates, money and terms in written form.
pub fn finish(text: &str, language: &str, written_form: bool) -> String {
    let text = collapse_loops(text);
    let text = fix_case(&text, language);
    let text = tidy_spacing(&text);
    if written_form && language == "en" {
        crate::written_form::apply(&text, None)
    } else {
        text
    }
}

fn loop_key(word: &str) -> String {
    word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Keeps one copy of any group of up to six words repeated back to back at least
/// [`LOOP_REPETITIONS`] times.
pub fn collapse_loops(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let keys: Vec<String> = words.iter().map(|w| loop_key(w)).collect();
    let mut output: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    'outer: while i < words.len() {
        for n in 1..=LONGEST_LOOP_WORDS {
            if i + n * LOOP_REPETITIONS > words.len() || keys[i..i + n].iter().all(String::is_empty) {
                continue;
            }
            let mut repeats = 1;
            while i + (repeats + 1) * n <= words.len()
                && keys[i + repeats * n..i + (repeats + 1) * n] == keys[i..i + n]
            {
                repeats += 1;
            }
            if repeats >= LOOP_REPETITIONS {
                output.extend_from_slice(&words[i..i + n]);
                i += repeats * n;
                continue 'outer;
            }
        }
        output.push(words[i]);
        i += 1;
    }
    output.join(" ")
}

fn lower(c: char, language: &str, out: &mut String) {
    match (language, c) {
        ("tr" | "az", 'I') => out.push('ı'),
        ("tr" | "az", 'İ') => out.push('i'),
        _ => out.extend(c.to_lowercase()),
    }
}

fn upper(c: char, language: &str, out: &mut String) {
    match (language, c) {
        ("tr" | "az", 'i') => out.push('İ'),
        _ => out.extend(c.to_uppercase()),
    }
}

/// Some transducers emit ALL CAPS. When every cased letter is upper case, lower-case the text
/// (Turkish dotted and dotless i kept apart), capitalize sentence starts and, in English, "I".
/// Text that already mixes case is returned unchanged.
pub fn fix_case(text: &str, language: &str) -> String {
    let cased: Vec<char> = text.chars().filter(|c| c.is_lowercase() || c.is_uppercase()).collect();
    if cased.len() < 2 || cased.iter().any(|c| c.is_lowercase()) {
        return text.to_owned();
    }
    let mut lowered = String::with_capacity(text.len());
    for c in text.chars() {
        lower(c, language, &mut lowered);
    }
    let mut output = String::with_capacity(lowered.len());
    let mut sentence_start = true;
    for word in lowered.split(' ') {
        if !output.is_empty() {
            output.push(' ');
        }
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
        let english_i = language == "en" && (bare == "i" || bare.starts_with("i'"));
        let mut chars = word.chars();
        let mut done = false;
        for c in chars.by_ref() {
            if !done && c.is_alphabetic() && (sentence_start || english_i) {
                upper(c, language, &mut output);
                done = true;
            } else {
                output.push(c);
            }
            if c.is_alphabetic() {
                done = true;
            }
        }
        if word.chars().any(char::is_alphanumeric) {
            sentence_start = false;
        }
        if word.ends_with(['.', '!', '?', '؟']) {
            sentence_start = true;
        }
    }
    output
}

/// No space before closing punctuation (Latin and Arabic), single spaces elsewhere.
pub fn tidy_spacing(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::with_capacity(collapsed.len());
    let mut chars = collapsed.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' && chars.peek().is_some_and(|next| matches!(next, ',' | '.' | '!' | '?' | ';' | ':' | '…' | '،' | '؛' | '؟')) {
            continue;
        }
        output.push(c);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_tokens_and_captions_are_removed() {
        assert_eq!(clean_segment("<|en|><|NEUTRAL|><|Speech|> hello there"), "hello there");
        assert_eq!(clean_segment("[BLANK_AUDIO]"), "");
        assert_eq!(clean_segment(" (music) ♪♪ we start [Music] now (inaudible) "), "we start now");
        assert_eq!(clean_segment("meet at [the office]"), "meet at [the office]");
        assert_eq!(clean_segment("مرحبا （音乐） بك"), "مرحبا （音乐） بك");
    }

    #[test]
    fn decoder_loops_collapse_but_real_repetition_stays() {
        let looped = format!("so {} anyway", ["thank you."; 12].join(" "));
        assert_eq!(collapse_loops(&looped), "so thank you. anyway");
        assert_eq!(collapse_loops(&["no"; 9].join(" ")), "no");
        let emphasis = "no no no no I said no";
        assert_eq!(collapse_loops(emphasis), emphasis);
        let seven = ["go team"; 7].join(" ");
        assert_eq!(collapse_loops(&seven), seven);
        assert_eq!(collapse_loops(""), "");
    }

    #[test]
    fn all_caps_output_is_recased_per_language() {
        assert_eq!(fix_case("HELLO WORLD. I'M HERE AND I KNOW", "en"), "Hello world. I'm here and I know");
        assert_eq!(fix_case("Already Mixed case", "en"), "Already Mixed case");
        // Turkish keeps dotted and dotless i apart in both directions.
        assert_eq!(fix_case("IŞIK İSTANBUL'DA. İYİ", "tr"), "Işık istanbul'da. İyi");
        // Arabic has no case.
        assert_eq!(fix_case("مرحبا بكم", "ar"), "مرحبا بكم");
        assert_eq!(fix_case("A", "en"), "A");
    }

    #[test]
    fn spacing_is_tidied_for_latin_and_arabic_punctuation() {
        assert_eq!(tidy_spacing("hello , world !  ok ?"), "hello, world! ok?");
        assert_eq!(tidy_spacing("مرحبا ، كيف حالك ؟"), "مرحبا، كيف حالك؟");
    }

    #[test]
    fn english_gets_written_form_other_languages_do_not() {
        let english = finish("i paid twenty five dollars", "en", true);
        assert!(english.contains("$25"), "{english}");
        assert_eq!(finish("i paid twenty five dollars", "en", false), "i paid twenty five dollars");
        assert_eq!(finish("yirmi beş lira ödedim", "tr", true), "yirmi beş lira ödedim");
    }

    #[test]
    fn segments_join_with_single_spaces() {
        assert_eq!(join(["hello ", "", "  world  again"]), "hello world again");
    }
}
