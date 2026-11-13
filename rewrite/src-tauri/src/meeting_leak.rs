//! Keeps the other participants' voices out of the microphone transcript.
//!
//! On loudspeakers, far-end speech reaches the microphone and is transcribed a
//! second time as "You". The system-audio stem holds exactly what was played,
//! so its transcript says what can leak and when. A leaked phrase shows up as
//! a run of microphone words that repeat system-audio words, in the same order,
//! at the same moment. Only the matching words of such runs are removed; the
//! user's own words between them, isolated repeats and anything said a moment
//! later are kept. Measurements: `docs/research/MeetingLeakFilter.md`.
//!
//! Echo cancellation on the signal was measured as well and is not used: the
//! one real leaky meeting reached the microphone through a non-linear path
//! (device processing) that no linear canceller could model.

use crate::asr::{RecognitionResult, Segment};

#[derive(Clone, Copy)]
struct Rules {
    /// Largest distance in seconds between a microphone word and its
    /// system-audio twin, after the measured offset between the channels.
    window: f32,
    /// A run: `need` matches among `span` consecutive microphone words.
    span: usize,
    need: usize,
    /// The twins must play in the same order. Only meaningful with real
    /// word times; spread times are too coarse to order repeated words.
    ordered: bool,
}

/// Both channels carry word times (SonoJot Parakeet).
const TIMED: Rules = Rules { window: 0.5, span: 4, need: 3, ordered: true };
/// Word times are spread evenly over each segment (Qwen, Granite, online
/// models): a wider window, and every word of a longer run must match.
const SPREAD: Rules = Rules { window: SPREAD_WINDOW, span: SPREAD_RUN, need: SPREAD_RUN, ordered: false };
const SPREAD_WINDOW: f32 = 6.0;
const SPREAD_RUN: usize = 6;
/// The offset between the channels comes from runs found with a wide window,
/// once there are enough of them.
const CALIBRATION_WINDOW: f32 = 2.0;
const CALIBRATION_WORDS: usize = 20;
const MAX_OFFSET: f32 = 1.5;

#[derive(Debug)]
struct Word {
    time: f32,
    key: String,
    segment: usize,
    /// Byte range of the word inside its segment's text.
    range: (usize, usize),
}

/// Whether a two-source meeting removes far-end words from the microphone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EchoRemoval {
    /// No saved choice: only when both stems carry the recogniser's word
    /// times (SonoJot Parakeet on the CPU). The spread-time rule can remove
    /// words the user repeats within a few seconds, so it waits for a choice.
    Automatic,
    On,
    Off,
}

impl EchoRemoval {
    pub(crate) fn from_setting(choice: Option<bool>) -> Self {
        match choice { None => Self::Automatic, Some(true) => Self::On, Some(false) => Self::Off }
    }
}

/// Removes far-end words from `mic` and returns how many were removed. The
/// unfiltered microphone text stays in `original_text`.
pub(crate) fn remove_far_end_words(mic: &mut RecognitionResult, system: &RecognitionResult, mode: EchoRemoval) -> usize {
    if mode == EchoRemoval::Off { return 0; }
    let (mic_words, mic_timed) = timed_words(mic);
    let (system_words, system_timed) = timed_words(system);
    if mic_words.len() < 3 || system_words.len() < 3 { return 0; }
    if mode == EchoRemoval::Automatic && !(mic_timed && system_timed) { return 0; }
    let rules = if mic_timed && system_timed { TIMED } else { SPREAD };
    let offset = channel_offset(&mic_words, &system_words, &rules);
    let drop = leaked(&mic_words, &system_words, &rules, offset, None);
    let removed = drop.iter().filter(|&&d| d).count();
    if removed > 0 { remove(mic, &mic_words, &drop); }
    removed
}

fn key(word: &str) -> String {
    word.chars().flat_map(char::to_lowercase)
        .map(|c| if c == '’' { '\'' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '\'').collect()
}

/// Segment order as `asr::assemble_segments` joins them into `text`.
fn segment_order(segments: &[Segment]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| segments[a].start_seconds.total_cmp(&segments[b].start_seconds)
        .then(segments[a].id.cmp(&segments[b].id)));
    order
}

/// Words with times: the recogniser's own (`raw_words`) when they line up with
/// the segments, otherwise spread evenly over each segment. The flag says
/// whether the recogniser's times were used.
fn timed_words(result: &RecognitionResult) -> (Vec<Word>, bool) {
    let mut words = Vec::new();
    for index in segment_order(&result.segments) {
        let segment = &result.segments[index];
        if segment.text.is_empty() { continue; }
        let spans = crate::confidence::words(&segment.text);
        let start = segment.start_seconds;
        let end = segment.end_seconds.unwrap_or(start).max(start);
        let count = spans.len().max(1) as f32;
        for (i, (a, b)) in spans.into_iter().enumerate() {
            words.push(Word {
                time: start + (end - start) * (i as f32 + 0.5) / count,
                key: key(&segment.text[a..b]),
                segment: index,
                range: (a, b),
            });
        }
    }
    let timed = !words.is_empty() && result.raw_words.len() == words.len()
        && result.raw_words.iter().zip(&words).all(|(raw, word)| raw.time.is_some() && key(&raw.word) == word.key);
    if timed {
        for (word, raw) in words.iter_mut().zip(&result.raw_words) {
            word.time = raw.time.map_or(word.time, |(start, _)| start);
        }
    }
    words.retain(|word| !word.key.is_empty());
    (words, timed)
}

/// Median time from system word to microphone word over runs found with a
/// wide window; zero when there are too few to trust.
fn channel_offset(mic: &[Word], system: &[Word], rules: &Rules) -> f32 {
    let wide = Rules { window: CALIBRATION_WINDOW.max(rules.window), ..*rules };
    let mut pairs = Vec::new();
    leaked(mic, system, &wide, 0.0, Some(&mut pairs));
    if pairs.len() < CALIBRATION_WORDS { return 0.0; }
    pairs.sort_by(f32::total_cmp);
    pairs[pairs.len() / 2].clamp(-MAX_OFFSET, MAX_OFFSET)
}

/// Which microphone words belong to leaked runs. `pairs` collects the time
/// differences of the matched words.
fn leaked(mic: &[Word], system: &[Word], rules: &Rules, offset: f32, mut pairs: Option<&mut Vec<f32>>) -> Vec<bool> {
    let mut times: std::collections::HashMap<&str, Vec<f32>> = std::collections::HashMap::new();
    for word in system { times.entry(word.key.as_str()).or_default().push(word.time); }
    let twin: Vec<Option<f32>> = mic.iter().map(|word| {
        times.get(word.key.as_str())?.iter().copied()
            .filter(|t| (word.time - t - offset).abs() <= rules.window)
            .min_by(|a, b| (word.time - a - offset).abs().total_cmp(&(word.time - b - offset).abs()))
    }).collect();
    let mut drop = vec![false; mic.len()];
    let span = rules.span.min(mic.len());
    for start in 0..=mic.len() - span {
        let run = &twin[start..start + span];
        let hits: Vec<f32> = run.iter().flatten().copied().collect();
        // The twins must play in the same order: a phrase, not scattered words.
        if hits.len() >= rules.need && (!rules.ordered || hits.windows(2).all(|w| w[1] >= w[0] - 0.05)) {
            for i in start..start + span {
                if let Some(t) = twin[i] {
                    if !drop[i] {
                        if let Some(pairs) = pairs.as_deref_mut() { pairs.push(mic[i].time - t); }
                    }
                    drop[i] = true;
                }
            }
        }
    }
    drop
}

/// Deletes the dropped words (with their attached punctuation) from the
/// segments, drops segments left without words, and keeps timed words and
/// confidences aligned with the new text.
fn remove(mic: &mut RecognitionResult, words: &[Word], drop: &[bool]) {
    let before = mic.text.clone();
    let total = crate::confidence::words(&before).len();
    let mut cut: Vec<Vec<(usize, usize)>> = vec![Vec::new(); mic.segments.len()];
    for (word, &d) in words.iter().zip(drop) {
        if d { cut[word.segment].push(word.range); }
    }
    // Indices into `text` words, for raw words and confidences.
    let mut dropped_text_words = Vec::new();
    let mut position = 0;
    for index in segment_order(&mic.segments) {
        let segment = &mic.segments[index];
        if segment.text.is_empty() { continue; }
        for range in crate::confidence::words(&segment.text) {
            if cut[index].contains(&range) { dropped_text_words.push(position); }
            position += 1;
        }
    }
    for (segment, ranges) in mic.segments.iter_mut().zip(&cut) {
        if ranges.is_empty() { continue; }
        let mut kept = Vec::new();
        let mut offset = 0;
        for token in segment.text.split_whitespace() {
            let start = offset + segment.text[offset..].find(token).unwrap_or(0);
            offset = start + token.len();
            if !ranges.iter().any(|&(a, b)| a >= start && b <= offset) { kept.push(token); }
        }
        segment.text = if kept.iter().any(|t| t.chars().any(char::is_alphanumeric)) { kept.join(" ") } else { String::new() };
    }
    mic.segments.retain(|segment| !segment.text.is_empty());
    let keep = |items: usize| -> Option<Vec<bool>> {
        (items == total).then(|| (0..total).map(|i| dropped_text_words.binary_search(&i).is_err()).collect())
    };
    match keep(mic.raw_words.len()) {
        Some(mask) => { let mut m = mask.into_iter(); mic.raw_words.retain(|_| m.next().unwrap_or(true)); }
        None => mic.raw_words.clear(),
    }
    match keep(mic.word_confidence.len()) {
        Some(mask) => { let mut m = mask.into_iter(); mic.word_confidence.retain(|_| m.next().unwrap_or(true)); }
        None => mic.word_confidence.clear(),
    }
    let order = segment_order(&mic.segments);
    mic.text = order.iter().map(|&i| mic.segments[i].text.as_str()).collect::<Vec<_>>().join(" ");
    if mic.original_text.is_none() { mic.original_text = Some(before); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RecognitionCompleteness;
    use crate::trust::evidence::RawWord;

    /// One segment per (start, text); words one per 0.3 s from the segment start.
    fn recognition(parts: &[(f32, &str)], timed: bool) -> RecognitionResult {
        let mut segments = Vec::new();
        let mut raw_words = Vec::new();
        for (id, &(start, text)) in parts.iter().enumerate() {
            let count = crate::confidence::words(text).len();
            segments.push(Segment { id: id as u64, start_seconds: start, end_seconds: Some(start + 0.3 * count as f32),
                speaker: None, text: text.into(), complete: true });
            for (i, (a, b)) in crate::confidence::words(text).into_iter().enumerate() {
                let t = start + 0.3 * i as f32;
                raw_words.push(RawWord { word: text[a..b].into(), confidence: None, time: Some((t, t + 0.25)) });
            }
        }
        let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
        RecognitionResult { word_confidence: Vec::new(), raw_words: if timed { raw_words } else { Vec::new() }, text,
            original_text: None, segments, completeness: RecognitionCompleteness::Complete, warnings: Vec::new() }
    }

    const FAR: &str = "Shall we move the launch to Thursday so the design team can finish the pricing page";

    #[test]
    fn a_leaked_phrase_is_removed_and_the_users_words_stay() {
        let system = recognition(&[(10.0, FAR)], true);
        let mut mic = recognition(&[(2.0, "I think the build is ready."),
            (10.05, "Shall we move the launch to Thursday so the design team can finish the pricing page"),
            (20.0, "Yes, Thursday works for me.")], true);
        let removed = remove_far_end_words(&mut mic, &system, EchoRemoval::On);
        assert_eq!(removed, 16);
        assert_eq!(mic.text, "I think the build is ready. Yes, Thursday works for me.");
        assert_eq!(mic.segments.len(), 2);
        assert_eq!(mic.raw_words.len(), crate::confidence::words(&mic.text).len());
        assert!(mic.original_text.as_deref().unwrap().contains("pricing page"));
    }

    #[test]
    fn words_spoken_over_the_leak_are_kept() {
        let system = recognition(&[(10.0, FAR)], true);
        // The user talks over the far end; their words fall between the leaked ones.
        let mut mic = recognition(&[(10.0, "Shall we move hold on the launch to")], true);
        mic.raw_words = ["Shall", "we", "move", "hold", "on", "the", "launch", "to"].iter()
            .zip([10.0, 10.3, 10.6, 10.7, 10.8, 10.9, 11.2, 11.5])
            .map(|(w, t)| RawWord { word: (*w).into(), confidence: None, time: Some((t, t + 0.2)) }).collect();
        mic.text = mic.segments[0].text.clone();
        remove_far_end_words(&mut mic, &system, EchoRemoval::On);
        assert_eq!(mic.text, "hold on");
    }

    #[test]
    fn repeating_the_far_end_a_moment_later_is_kept() {
        let system = recognition(&[(10.0, FAR)], true);
        let mut mic = recognition(&[(13.0, "move the launch to Thursday")], true);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 0);
        assert_eq!(mic.text, "move the launch to Thursday");
        assert_eq!(mic.original_text, None);
    }

    #[test]
    fn isolated_common_words_are_kept() {
        let system = recognition(&[(10.0, "yes I think so and then we can go")], true);
        let mut mic = recognition(&[(10.1, "yes we should check the logs first")], true);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 0);
    }

    #[test]
    fn headphones_change_nothing() {
        // No leak: the user and the far end take turns, different words.
        let system = recognition(&[(0.0, FAR), (30.0, "Great, see you all on Thursday then")], true);
        let mut mic = recognition(&[(8.0, "Sounds good, I will update the roadmap"), (25.0, "Thanks everyone")], true);
        let before = mic.clone();
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 0);
        assert_eq!(mic, before);
    }

    #[test]
    fn segment_times_alone_need_a_longer_exact_run() {
        // No word times (Qwen): six or more consecutive matches are removed...
        let system = recognition(&[(10.0, FAR)], false);
        let mut mic = recognition(&[(9.0, "okay the design team can finish the pricing page today")], false);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 8);
        assert_eq!(mic.text, "okay today");
        // ...shorter coincidences are not.
        let mut mic = recognition(&[(9.0, "okay the design team is out today")], false);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 0);
    }

    #[test]
    fn without_a_choice_only_word_timed_recognition_is_filtered() {
        let leaked = "okay the design team can finish the pricing page today";
        // Word times on both channels (SonoJot Parakeet): filtered automatically.
        let system = recognition(&[(10.0, FAR)], true);
        let mut mic = recognition(&[(10.05, FAR)], true);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::Automatic), 16);
        // Segment times only (Qwen): untouched unless switched on.
        let system = recognition(&[(10.0, FAR)], false);
        let mut mic = recognition(&[(9.0, leaked)], false);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::Automatic), 0);
        assert_eq!(mic.original_text, None);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), 8);
        // Off always wins.
        let system = recognition(&[(10.0, FAR)], true);
        let mut mic = recognition(&[(10.05, FAR)], true);
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::Off), 0);
        assert_eq!(EchoRemoval::from_setting(None), EchoRemoval::Automatic);
        assert_eq!(EchoRemoval::from_setting(Some(false)), EchoRemoval::Off);
    }

    #[test]
    fn a_constant_device_delay_is_learned() {
        // Leaked words arrive 1.2 s after the system audio (Bluetooth speaker).
        let lines: Vec<String> = (0..8).map(|i| format!("{FAR} number {i}")).collect();
        let system_parts: Vec<(f32, &str)> = lines.iter().enumerate().map(|(i, l)| (i as f32 * 20.0, l.as_str())).collect();
        let mic_parts: Vec<(f32, &str)> = lines.iter().enumerate().map(|(i, l)| (i as f32 * 20.0 + 1.2, l.as_str())).collect();
        let system = recognition(&system_parts, true);
        let mut mic = recognition(&mic_parts, true);
        let total = crate::confidence::words(&mic.text).len();
        assert_eq!(remove_far_end_words(&mut mic, &system, EchoRemoval::On), total);
        assert!(mic.text.is_empty() && mic.segments.is_empty());
    }
}
