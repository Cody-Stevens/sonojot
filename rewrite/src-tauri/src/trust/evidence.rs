//! Recognition evidence for the trust layer: what the recognizer knew about
//! each word (confidence, timing, alternatives) and where speech had no words.
//!
//! Platform independent. Recognizers fill `RawWord`s for the raw transcript;
//! `project` carries them onto the cleaned text the user actually received.
//! Unknown stays unknown: a recognizer without confidence leaves it `None`.

use serde::{Deserialize, Serialize};

/// One recognizer word, in raw-transcript order (one per `words(raw)` core).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawWord {
    pub word: String,
    /// Probability in 0..=1, or `None` when the runtime cannot say.
    pub confidence: Option<f32>,
    /// Seconds on the dictation audio timeline.
    pub time: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AltSource {
    /// A personal vocabulary entry that sounds or is spelled close.
    Vocabulary,
    /// The decoder's own next-best candidate.
    NextGuess,
    /// What the recognizer heard before cleanup changed it.
    AsHeard,
    /// A second recognizer's word for the same audio.
    SecondModel,
}

impl AltSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vocabulary => "Your words",
            Self::NextGuess => "Next guess",
            Self::AsHeard => "As heard",
            Self::SecondModel => "Second model",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    pub text: String,
    pub source: AltSource,
}

/// Evidence for one word of a text, by byte range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordEvidence {
    pub start: usize,
    pub end: usize,
    pub confidence: Option<f32>,
    pub time: Option<(f32, f32)>,
    pub alternatives: Vec<Alternative>,
}

/// Speech with no emitted word. `after` is the index of the word before the
/// gap in `Evidence::words`, or `None` when the gap precedes the first word.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GapEvidence {
    pub after: Option<usize>,
    pub time: (f32, f32),
    /// Share of the gap's frames that carried speech-like energy, 0..=1.
    pub speech_share: f32,
    pub alternatives: Vec<Alternative>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub text: String,
    pub words: Vec<WordEvidence>,
    pub gaps: Vec<GapEvidence>,
}

impl Evidence {
    pub fn word(&self, index: usize) -> &str {
        let w = &self.words[index];
        &self.text[w.start..w.end]
    }
}

/// Word cores of a text, the same rule the confidence module uses.
pub fn word_ranges(text: &str) -> Vec<(usize, usize)> {
    crate::confidence::words(text)
}

/// Evidence for the raw transcript. Returns words without evidence when the
/// recognizer's word list does not line up one to one with the text.
pub fn from_raw(text: &str, raw: &[RawWord]) -> Evidence {
    let ranges = word_ranges(text);
    let aligned = raw.len() == ranges.len()
        && raw
            .iter()
            .zip(&ranges)
            .all(|(w, (a, b))| key(&w.word) == key(&text[*a..*b]));
    let words = ranges
        .iter()
        .enumerate()
        .map(|(i, &(start, end))| WordEvidence {
            start,
            end,
            confidence: aligned.then(|| raw[i].confidence).flatten(),
            time: aligned.then(|| raw[i].time).flatten(),
            alternatives: Vec::new(),
        })
        .collect();
    Evidence { text: text.to_owned(), words, gaps: Vec::new() }
}

/// Comparison key: lower case letters and digits only.
pub fn key(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Spelled-out numbers and digits compare equal when cleanup rewrote one as
/// the other ("thirty" / "30"). Only single words; anything else differs.
fn number_key(word: &str) -> Option<u32> {
    const SMALL: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen",
        "eighteen", "nineteen",
    ];
    const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    let k = key(word);
    if let Ok(n) = k.parse::<u32>() {
        return Some(n);
    }
    if let Some(i) = SMALL.iter().position(|w| *w == k) {
        return Some(i as u32);
    }
    TENS.iter().position(|w| *w == k).map(|i| (i as u32 + 2) * 10)
}

fn same(a: &str, b: &str) -> bool {
    key(a) == key(b) || matches!((number_key(a), number_key(b)), (Some(x), Some(y)) if x == y)
}

/// Carry raw evidence onto the cleaned text. Cleanup changes punctuation,
/// capitals, number forms and vocabulary spellings, so words are aligned by
/// longest common subsequence on a loose key. Unmatched cleaned words inside a
/// gap take the evidence of the raw words they replaced (the least confident
/// one, the union of their times) and offer the raw words as "As heard".
/// Words cleanup inserted carry no evidence. Raw gaps move with their word.
pub fn project(raw: &Evidence, cleaned: &str) -> Evidence {
    let ranges = word_ranges(cleaned);
    let src: Vec<&str> = (0..raw.words.len()).map(|i| raw.word(i)).collect();
    let dst: Vec<&str> = ranges.iter().map(|(a, b)| &cleaned[*a..*b]).collect();
    let mut words: Vec<WordEvidence> = ranges
        .iter()
        .map(|&(start, end)| WordEvidence { start, end, confidence: None, time: None, alternatives: Vec::new() })
        .collect();
    let (n, m) = (src.len(), dst.len());
    // raw index -> cleaned index of the word that carries it.
    let mut carried: Vec<Option<usize>> = vec![None; n];
    if n > 0 && m > 0 && n.saturating_mul(m) <= 4_000_000 {
        let mut dp = vec![0u32; (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[at(i, j)] = if same(src[i], dst[j]) {
                    dp[at(i + 1, j + 1)] + 1
                } else {
                    dp[at(i + 1, j)].max(dp[at(i, j + 1)])
                };
            }
        }
        let mut pairs = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if same(src[i], dst[j]) && dp[at(i, j)] == dp[at(i + 1, j + 1)] + 1 {
                pairs.push((i, j));
                i += 1;
                j += 1;
            } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
                i += 1;
            } else {
                j += 1;
            }
        }
        pairs.push((n, m));
        let (mut pi, mut pj) = (0usize, 0usize);
        for &(i, j) in &pairs {
            // The unmatched run raw[pi..i] became cleaned[pj..j].
            if pi < i && pj < j {
                let raw_run: Vec<usize> = (pi..i).collect();
                for (k, cj) in (pj..j).enumerate() {
                    // Equal runs pair one to one; otherwise every cleaned word
                    // takes the whole raw run.
                    let take: Vec<usize> = if i - pi == j - pj { vec![pi + k] } else { raw_run.clone() };
                    let ev = merge(raw, &take);
                    let words_cj = &mut words[cj];
                    words_cj.confidence = ev.0;
                    words_cj.time = ev.1;
                    let heard: Vec<&str> = take.iter().map(|r| src[*r]).collect();
                    let heard = heard.join(" ");
                    if key(&heard) != key(dst[cj]) && !heard.is_empty() {
                        push_alt(&mut words_cj.alternatives, &heard, AltSource::AsHeard);
                    }
                    for &r in &take {
                        carried[r] = Some(cj);
                        for alt in &raw.words[r].alternatives {
                            push_alt(&mut words_cj.alternatives, &alt.text, alt.source);
                        }
                    }
                }
            }
            if i < n && j < m {
                let w = &raw.words[i];
                words[j].confidence = w.confidence;
                words[j].time = w.time;
                for alt in &w.alternatives {
                    push_alt(&mut words[j].alternatives, &alt.text, alt.source);
                }
                // Same word, different written form ("thirty" -> "30").
                if src[i] != dst[j] && key(src[i]) != key(dst[j]) {
                    push_alt(&mut words[j].alternatives, src[i], AltSource::AsHeard);
                }
                carried[i] = Some(j);
            }
            pi = i + 1;
            pj = j + 1;
        }
    }
    let gaps = raw
        .gaps
        .iter()
        .filter_map(|g| {
            let after = match g.after {
                None => None,
                // The last carried word at or before the raw word.
                Some(r) => Some((0..=r.min(n.saturating_sub(1))).rev().find_map(|k| carried[k])?),
            };
            Some(GapEvidence { after, ..g.clone() })
        })
        .collect();
    Evidence { text: cleaned.to_owned(), words, gaps }
}

fn merge(raw: &Evidence, take: &[usize]) -> (Option<f32>, Option<(f32, f32)>) {
    let conf = take
        .iter()
        .map(|i| raw.words[*i].confidence)
        .try_fold(f32::INFINITY, |acc, c| c.map(|c| acc.min(c)))
        .filter(|c| c.is_finite());
    let times: Vec<(f32, f32)> = take.iter().filter_map(|i| raw.words[*i].time).collect();
    let time = (times.len() == take.len() && !times.is_empty())
        .then(|| (times.iter().map(|t| t.0).fold(f32::INFINITY, f32::min), times.iter().map(|t| t.1).fold(0.0, f32::max)));
    (conf, time)
}

fn push_alt(list: &mut Vec<Alternative>, text: &str, source: AltSource) {
    let text = text.trim();
    if text.is_empty() || list.iter().any(|a| key(&a.text) == key(text)) {
        return;
    }
    list.push(Alternative { text: text.to_owned(), source });
}

/// Frame energy in dBFS-free linear RMS, 20 ms frames at 16 kHz.
pub const FRAME_SECONDS: f32 = 0.02;

pub fn frame_rms(audio: &[f32]) -> Vec<f32> {
    audio
        .chunks(320)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len().max(1) as f32).sqrt())
        .collect()
}

/// Speech with no word: a stretch between word times (or before the first,
/// or after the last) at least `min_seconds` long where at least `min_share`
/// of the frames are above the speech floor. The floor adapts to the
/// recording: a fixed fraction of the loudest frames inside words.
pub fn detect_gaps(times: &[Option<(f32, f32)>], rms: &[f32], min_seconds: f32, min_share: f32) -> Vec<(Option<usize>, (f32, f32), f32)> {
    if times.is_empty() || times.iter().any(Option::is_none) || rms.is_empty() {
        return Vec::new();
    }
    let times: Vec<(f32, f32)> = times.iter().map(|t| t.unwrap()).collect();
    let frame = |t: f32| ((t / FRAME_SECONDS).max(0.0) as usize).min(rms.len());
    let mut voiced: Vec<f32> = times.iter().flat_map(|(a, b)| rms[frame(*a)..frame(*b).max(frame(*a))].iter().copied()).collect();
    if voiced.len() < 5 {
        return Vec::new();
    }
    voiced.sort_by(f32::total_cmp);
    // Median word energy; speech in a gap must reach a third of it.
    let floor = (voiced[voiced.len() / 2] / 3.0).max(0.004);
    let total = rms.len() as f32 * FRAME_SECONDS;
    let mut spans = Vec::new();
    spans.push((None, 0.0, times[0].0));
    for i in 0..times.len() {
        let next = times.get(i + 1).map(|t| t.0).unwrap_or(total);
        spans.push((Some(i), times[i].1, next));
    }
    spans
        .into_iter()
        .filter_map(|(after, a, b)| {
            // Keep a small margin so word edges do not count as a gap.
            let (a, b) = (a + 0.1, b - 0.1);
            if b - a < min_seconds {
                return None;
            }
            let frames = &rms[frame(a)..frame(b).max(frame(a))];
            if frames.is_empty() {
                return None;
            }
            let share = frames.iter().filter(|r| **r >= floor).count() as f32 / frames.len() as f32;
            (share >= min_share).then_some((after, (a, b), share))
        })
        .collect()
}

/// Vocabulary entries that sound or are spelled close to a word: same first
/// sound and a small edit distance on a simple sound key, or a small spelling
/// distance. Exact matches are not alternatives.
pub fn vocabulary_alternatives(word: &str, vocabulary: &[String]) -> Vec<Alternative> {
    let k = key(word);
    if k.len() < 3 {
        return Vec::new();
    }
    let sound = sound_key(&k);
    let mut found: Vec<(usize, &String)> = vocabulary
        .iter()
        .filter_map(|v| {
            let vk = key(v);
            if vk.is_empty() || vk == k || v.split_whitespace().count() > 3 {
                return None;
            }
            let spell = levenshtein(&k, &vk);
            let vs = sound_key(&vk);
            let sd = levenshtein(&sound, &vs);
            let longest = k.len().max(vk.len());
            let close_spelling = spell * 3 <= longest;
            let close_sound = sd * 2 <= sound.len().max(vs.len()).max(1) && spell * 2 <= longest;
            (close_spelling || close_sound).then_some((spell, v))
        })
        .collect();
    found.sort_by_key(|(d, _)| *d);
    found
        .into_iter()
        .take(2)
        .map(|(_, v)| Alternative { text: v.clone(), source: AltSource::Vocabulary })
        .collect()
}

/// A rough English sound key: letters folded into sound classes, vowels
/// dropped after the first letter, repeats collapsed.
fn sound_key(k: &str) -> String {
    let mut out = String::new();
    let mut last = ' ';
    for (i, c) in k.chars().enumerate() {
        let class = match c {
            'b' | 'p' => 'p',
            'c' | 'k' | 'q' | 'g' | 'x' => 'k',
            'd' | 't' => 't',
            'f' | 'v' => 'f',
            's' | 'z' => 's',
            'm' | 'n' => 'n',
            'l' => 'l',
            'r' => 'r',
            'j' => 'j',
            'h' | 'w' | 'y' => '_',
            'a' | 'e' | 'i' | 'o' | 'u' => {
                if i == 0 { 'a' } else { '_' }
            }
            d if d.is_ascii_digit() => d,
            _ => '_',
        };
        if class != '_' && class != last {
            out.push(class);
        }
        if class != '_' {
            last = class;
        }
    }
    out
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != b[j])).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(words: &[(&str, f32, f32, f32)]) -> Vec<RawWord> {
        words
            .iter()
            .map(|(w, c, a, b)| RawWord { word: (*w).into(), confidence: Some(*c), time: Some((*a, *b)) })
            .collect()
    }

    #[test]
    fn raw_evidence_needs_one_to_one_words() {
        let text = "the castrel rollout";
        let e = from_raw(text, &raw(&[("the", 0.9, 0.0, 0.2), ("castrel", 0.3, 0.2, 0.7), ("rollout", 0.95, 0.7, 1.1)]));
        assert_eq!(e.words[1].confidence, Some(0.3));
        assert_eq!(e.word(1), "castrel");
        let short = from_raw(text, &raw(&[("the", 0.9, 0.0, 0.2)]));
        assert!(short.words.iter().all(|w| w.confidence.is_none() && w.time.is_none()));
    }

    #[test]
    fn projection_survives_punctuation_capitals_and_numbers() {
        let raw_text = "ask margaret whether the room holds thirty people";
        let e = from_raw(
            raw_text,
            &raw(&[
                ("ask", 0.9, 0.0, 0.2),
                ("margaret", 0.4, 0.2, 0.6),
                ("whether", 0.9, 0.6, 0.8),
                ("the", 0.9, 0.8, 0.9),
                ("room", 0.9, 0.9, 1.1),
                ("holds", 0.9, 1.1, 1.3),
                ("thirty", 0.2, 1.3, 1.6),
                ("people", 0.9, 1.6, 1.9),
            ]),
        );
        let p = project(&e, "Ask Margaret whether the room holds 30 people.");
        assert_eq!(p.word(1), "Margaret");
        assert_eq!(p.words[1].confidence, Some(0.4));
        assert_eq!(p.word(6), "30");
        assert_eq!(p.words[6].confidence, Some(0.2));
        assert_eq!(p.words[6].time, Some((1.3, 1.6)));
        assert_eq!(p.words[6].alternatives, vec![Alternative { text: "thirty".into(), source: AltSource::AsHeard }]);
        assert!(p.words[1].alternatives.is_empty());
    }

    #[test]
    fn projection_handles_deletions_rewrites_and_insertions() {
        let e = from_raw(
            "um fix the cash eviction",
            &raw(&[("um", 0.5, 0.0, 0.2), ("fix", 0.9, 0.2, 0.4), ("the", 0.9, 0.4, 0.5), ("cash", 0.3, 0.5, 0.8), ("eviction", 0.9, 0.8, 1.2)]),
        );
        // Filler removed, vocabulary rewrote "cash" to "cache", cleanup added "now".
        let p = project(&e, "Fix the cache eviction now.");
        assert_eq!(p.word(2), "cache");
        assert_eq!(p.words[2].confidence, Some(0.3));
        assert_eq!(p.words[2].alternatives[0], Alternative { text: "cash".into(), source: AltSource::AsHeard });
        assert_eq!(p.words[4].confidence, None);
        assert_eq!(p.words[0].confidence, Some(0.9));
    }

    #[test]
    fn many_to_one_rewrite_takes_the_weakest_word() {
        let e = from_raw("ten k budget", &raw(&[("ten", 0.8, 0.0, 0.3), ("k", 0.4, 0.3, 0.5), ("budget", 0.9, 0.5, 0.9)]));
        let p = project(&e, "10K budget");
        assert_eq!(p.word(0), "10K");
        assert_eq!(p.words[0].confidence, Some(0.4));
        assert_eq!(p.words[0].time, Some((0.0, 0.5)));
    }

    #[test]
    fn gaps_follow_their_word_through_cleanup() {
        let mut e = from_raw("um the vendor confirmed", &raw(&[("um", 0.9, 0.0, 0.2), ("the", 0.9, 0.2, 0.3), ("vendor", 0.9, 0.3, 0.7), ("confirmed", 0.9, 1.9, 2.3)]));
        e.gaps.push(GapEvidence { after: Some(2), time: (0.8, 1.8), speech_share: 0.8, alternatives: vec![] });
        let p = project(&e, "The vendor confirmed.");
        assert_eq!(p.gaps[0].after, Some(1));
        assert_eq!(p.word(1), "vendor");
    }

    #[test]
    fn gap_detection_needs_energy_and_length() {
        // 3 s of audio: words 0-1 s and 2-3 s, loud speech 1.2-1.8 s in between.
        let mut rms = vec![0.0f32; 150];
        for f in rms.iter_mut().take(50) { *f = 0.2; }
        for f in rms.iter_mut().skip(100) { *f = 0.2; }
        let times = [Some((0.0, 1.0)), Some((2.0, 3.0))];
        assert!(detect_gaps(&times, &rms, 0.6, 0.5).is_empty(), "silence is not a gap");
        for f in rms.iter_mut().skip(55).take(35) { *f = 0.15; }
        let gaps = detect_gaps(&times, &rms, 0.6, 0.5);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].0, Some(0));
        assert!(detect_gaps(&[None, Some((2.0, 3.0))], &rms, 0.6, 0.5).is_empty(), "unknown timing never guesses");
    }

    #[test]
    fn vocabulary_suggests_close_entries_only() {
        let vocab = vec!["Kestrel".to_string(), "Marguerite".into(), "pH".into(), "Priya".into(), "cache".into()];
        let alts = |w: &str| vocabulary_alternatives(w, &vocab).into_iter().map(|a| a.text).collect::<Vec<_>>();
        assert_eq!(alts("Castrel"), vec!["Kestrel"]);
        assert_eq!(alts("Margaret"), vec!["Marguerite"]);
        assert_eq!(alts("cash"), vec!["cache"]);
        assert!(alts("Kestrel").is_empty(), "an exact match is no alternative");
        assert!(alts("rollout").is_empty());
        assert!(alts("the").is_empty());
    }
}
