//! Session-local fragment decisions. Native/protocol-parsed originals are immutable.
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Session { windows: Vec<Entry> }

struct Entry {
    id: u64,
    text: String,
    original: String,
    previously_guarded: bool,
    language: Option<String>,
}

#[derive(Debug)]
pub struct Decision {
    pub id: u64,
    pub text: String,
    pub original: String,
    pub language: Option<String>,
    pub dominant: Option<String>,
    pub reason: Option<&'static str>,
}

/// Use only the model's explicit protocol header, never guess from Latin text.
pub fn detected_language(native: &str) -> Option<String> {
    let prefix = native.trim_start().split_once("<asr_text>")?.0;
    let words: Vec<_> = prefix.split_whitespace().collect();
    let header = match words.as_slice() {
        [language] => *language,
        [label, language] if matches!(label.to_lowercase().as_str(), "language" | "língua" | "langue" | "lingua") => *language,
        _ => return None,
    };
    if header.is_empty() || header.eq_ignore_ascii_case("none")
        || !header.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '-') { return None; }
    Some(header.to_lowercase())
}

impl Session {
    pub fn push(&mut self, id: u64, text: &str, original: Option<&str>, language: Option<&str>) {
        self.windows.push(Entry { id, text: text.to_owned(), original: original.unwrap_or(text).to_owned(),
            previously_guarded: original.is_some(), language: language.map(|s| s.to_lowercase()) });
    }

    /// Revisit earlier windows when the dominant language is established later.
    /// A window never contributes its own words to evidence used to suppress it.
    pub fn decisions(&self) -> Vec<Decision> {
        let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
        for entry in &self.windows {
            if let Some(language) = entry.language.as_deref() {
                if !annotation_only(&entry.text) {
                    *totals.entry(language).or_default() += units(&entry.text);
                }
            }
        }
        self.windows.iter().map(|entry| {
            let mut others = totals.clone();
            if let Some(language) = entry.language.as_deref() {
                if !annotation_only(&entry.text) {
                    if let Some(count) = others.get_mut(language) { *count -= units(&entry.text); }
                }
            }
            let mut ranked: Vec<_> = others.iter().filter(|(_, n)| **n >= 10).collect();
            ranked.sort_by(|a, b| b.1.cmp(a.1));
            let dominant = ranked.first().filter(|first| ranked.get(1).is_none_or(|next| first.1 > next.1))
                .map(|(language, _)| (**language).to_owned());
            let different = entry.language.as_ref().zip(dominant.as_ref()).is_some_and(|(a, b)| a != b);
            let same_context = entry.language.as_deref().is_some_and(|language| others.get(language).copied().unwrap_or(0) > 0);
            let stripped = strip_annotations(&entry.text);
            let cjk_interjection = interjection_only(&entry.text);
            let reason = if different && short(&entry.text) && !entry.text.trim().is_empty() {
                Some("short_cross_language_fragment")
            } else if (stripped.trim().is_empty() && !entry.text.trim().is_empty())
                || (cjk_interjection && (different || !same_context)) {
                Some("isolated_non_speech")
            } else if different && stripped != entry.text {
                Some("cross_language_annotation")
            } else { None };
            let text = match reason {
                Some("cross_language_annotation") => stripped,
                Some(_) => String::new(),
                None => entry.text.clone(),
            };
            Decision { id: entry.id, text, original: entry.original.clone(), language: entry.language.clone(), dominant, reason }
        }).collect()
    }

    pub fn original_text(&self) -> Option<String> {
        let changed = self.windows.iter().any(|e| e.previously_guarded)
            || self.decisions().iter().any(|d| d.reason.is_some());
        changed.then(|| self.windows.iter().map(|e| e.original.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "))
    }
}

fn cjk(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af
        | 0x3040..=0x30ff | 0xac00..=0xd7af)
}

fn units(text: &str) -> usize {
    // Match the existing Qwen guard's units: alphabetic words and individual
    // CJK characters, so unspaced dictation can establish language context.
    let mut count = 0;
    let mut word = false;
    for ch in text.chars() {
        if cjk(ch) { count += 1; word = false; }
        else if ch.is_alphanumeric() {
            if !word { count += 1; }
            word = true;
        } else if ch != '\'' && ch != '’' { word = false; }
    }
    count
}

fn short(text: &str) -> bool {
    units(text) <= 3 || (text.chars().any(cjk) && units(text) <= 4)
}

fn interjection_only(text: &str) -> bool {
    let letters: String = text.chars().filter(|c| c.is_alphanumeric()).collect();
    !letters.is_empty() && letters.chars().all(|c| matches!(c, '咳' | '啊' | '嗯' | '对' | '呃' | '唔'))
}

fn annotation_only(text: &str) -> bool {
    interjection_only(text) || (!text.trim().is_empty() && strip_annotations(text).trim().is_empty())
}

fn annotation(label: &str) -> bool {
    matches!(label.trim().to_lowercase().as_str(), "cough" | "coughs" | "coughing" | "laugh" | "laughs"
        | "laughter" | "laughing" | "music" | "applause" | "sigh" | "sighs" | "sneeze" | "sneezes"
        | "breathing" | "silence" | "noise" | "咳" | "咳嗽" | "笑声" | "音乐")
}

fn strip_annotations(text: &str) -> String {
    let mut result = String::new();
    let mut at = 0;
    let mut changed = false;
    while at < text.len() {
        let ch = text[at..].chars().next().unwrap();
        let close = match ch { '*' => Some('*'), '(' => Some(')'), '[' => Some(']'), _ => None };
        let before = text[..at].chars().next_back();
        if let Some(close) = close.filter(|_| before.is_none_or(|c| !c.is_alphanumeric())) {
            let rest = at + ch.len_utf8();
            if let Some(length) = text[rest..].find(close) {
                let end = rest + length + close.len_utf8();
                let after = text[end..].chars().next();
                if after.is_none_or(|c| !c.is_alphanumeric()) && annotation(&text[rest..rest + length]) {
                    changed = true;
                    at = end;
                    continue;
                }
            }
        }
        result.push(ch);
        at += ch.len_utf8();
    }
    if !changed { return text.to_owned(); }
    let result = result.split_whitespace().collect::<Vec<_>>().join(" ");
    if result.chars().all(|c| !c.is_alphanumeric()) { String::new() } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ENGLISH: &str = "Please keep every word that I actually said in this recording.";

    #[test]
    fn late_dominant_language_revisits_earlier_short_windows_and_preserves_original() {
        let mut s = Session::default();
        s.push(1, "Obrigado.", None, Some("Portuguese"));
        assert_eq!(s.decisions()[0].text, "Obrigado.");
        s.push(2, ENGLISH, None, Some("English"));
        assert_eq!(s.decisions()[0].reason, Some("short_cross_language_fragment"));
        assert_eq!(s.original_text(), Some(format!("Obrigado. {ENGLISH}")));
        assert_eq!(s.decisions()[0].original, "Obrigado.");
    }

    #[test]
    fn ten_words_must_come_from_other_windows_and_unknown_language_is_kept() {
        let mut s = Session::default();
        s.push(1, "one two three four five six seven eight nine", None, Some("English"));
        s.push(2, "Sim.", None, Some("Portuguese"));
        assert!(s.decisions().iter().all(|d| d.reason.is_none()));
        s.push(3, "ten", None, Some("English"));
        assert!(s.decisions()[1].text.is_empty());
        s.push(4, "Sim.", None, None);
        assert_eq!(s.decisions()[3].text, "Sim.");
    }

    #[test]
    fn full_spanish_portuguese_and_chinese_dictations_are_untouched() {
        for (language, text) in [
            ("Spanish", "Quiero conservar todas las palabras de este mensaje para poder revisarlo mañana."),
            ("Portuguese", "Quero manter todas as palavras desta mensagem para poder conferir tudo amanhã."),
            ("Chinese", "请保留我在这次录音中说出的每一个词语以便稍后核对。"),
        ] {
            let mut s = Session::default();
            s.push(1, text, None, Some(language));
            assert_eq!(s.decisions()[0].text, text);
            assert_eq!(s.original_text(), None);
        }
    }

    #[test]
    fn same_language_context_restores_legitimate_chinese_interjections() {
        let mut s = Session::default();
        s.push(1, "对。", None, Some("Chinese"));
        assert!(s.decisions()[0].text.is_empty());
        let text = "请保留我在这次录音中说出的每一个词语。";
        s.push(2, text, None, Some("Chinese"));
        s.push(3, "嗯。", None, Some("Chinese"));
        assert_eq!(s.decisions()[0].text, "对。");
        assert_eq!(s.decisions()[2].text, "嗯。");
        assert_eq!(s.original_text(), None);
    }

    #[test]
    fn annotations_require_whole_tokens_and_permitted_context() {
        for text in ["*cough*", "*Cough*.", "(laughs)", "[music]", "咳咳咳。", "啊。"] {
            let mut s = Session::default();
            s.push(1, text, None, Some("Chinese"));
            assert!(s.decisions()[0].text.is_empty(), "{text}");
            assert_eq!(s.original_text().as_deref(), Some(text));
        }
        for text in ["Please *cough* keep this.", "cough", "coughing", "a*cough*b", "[musical]", "对这个问题我有不同的看法。"] {
            let mut s = Session::default();
            s.push(1, text, None, Some("English"));
            assert_eq!(s.decisions()[0].text, text);
        }
    }

    #[test]
    fn substantial_language_switches_survive_and_short_cjk_is_counted_by_character() {
        let mut s = Session::default();
        s.push(1, ENGLISH, None, Some("English"));
        s.push(2, "Esta frase cambia de idioma y debe conservarse.", None, Some("Spanish"));
        s.push(3, "你好世界", None, Some("Chinese"));
        s.push(4, "我们继续讨论这个问题。", None, Some("Chinese"));
        let d = s.decisions();
        assert!(!d[1].text.is_empty());
        assert!(d[2].text.is_empty());
        assert!(!d[3].text.is_empty());
    }

    #[test]
    fn earlier_guards_original_and_new_session_are_preserved_independently() {
        let mut s = Session::default();
        s.push(1, "", Some("go go go go"), Some("English"));
        s.push(2, ENGLISH, None, Some("English"));
        assert_eq!(s.original_text(), Some(format!("go go go go {ENGLISH}")));
        let next = Session::default();
        assert!(next.decisions().is_empty());
        assert_eq!(next.original_text(), None);
    }

    #[test]
    fn detected_language_requires_a_leading_protocol_header() {
        assert_eq!(detected_language("language Portuguese<asr_text>Olá"), Some("portuguese".into()));
        assert_eq!(detected_language("language None<asr_text>"), None);
        assert_eq!(detected_language("LANGUAGE English<asr_text>Hello"), Some("english".into()));
        assert_eq!(detected_language("Say language English<asr_text>"), None);
    }
}
