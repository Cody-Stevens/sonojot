//! Local vocabulary learning ledger and typed consumer planner.
//!
//! Confirmed words live in `vocabulary.json` (schema 1, user-owned, encrypted
//! sync). Observed evidence, unconfirmed suggestions, active local hints/aliases
//! and device-keyed suppression digests live in `lexicon.json` (local-only,
//! source-traceable, pruned when supporting sources are deleted).

use crate::domain::{CleanupMode, LearnedEntry, LearnedKind, VocabularyEntry, VocabularyKind};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap, HashSet};
use time::OffsetDateTime;

type HmacSha256 = Hmac<Sha256>;

pub const LEXICON_SCHEMA_VERSION: u32 = 1;
pub const MAX_OBSERVED: usize = 2_000;
pub const MAX_SUGGESTED: usize = 256;
pub const MAX_ACTIVE: usize = 512;
pub const MAX_SUPPRESSIONS: usize = 1_000;
pub const MAX_SOURCES_PER_ENTRY: usize = 32;
pub const MAX_NEAR_CUES: usize = 6;
pub const MAX_LEARNED_WORDS: usize = 3;
pub const MAX_LEARNED_BYTES: usize = 64;
pub const MAX_LOCAL_PROOFREADER_HINTS: usize = 32;
pub const MAX_ONLINE_PROOFREADER_HINTS: usize = 24;
pub const MAX_CLEANUP_LEARNED_ENTRIES: usize = 24;
pub const CLEANUP_CHAR_BUDGET: usize = 2048;

pub const OBSERVED_EXPIRE_DAYS: u32 = 45;
pub const SUGGESTED_EXPIRE_DAYS: u32 = 90;
pub const ACTIVE_DEMOTE_DAYS: u32 = 180;
pub const PROBATION_USES: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TermKind {
    /// Known written spelling only; never replaces another word.
    Hint,
    /// Same letters and token boundaries, approved casing (e.g. "sonojot" -> "SonoJot").
    Spelling,
    /// Heard -> written substitution (gated by `HeardRisk` and `Gate`).
    Alias,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Gate {
    Always,
    Near(Vec<String>),
    Topic(String),
    Meeting(String),
}

impl Default for Gate {
    fn default() -> Self {
        Self::Always
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeardRisk {
    Nonword,
    Ordinary,
    Protected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    Observed,
    Suggested,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    OverlayChoice,
    OverlayKeep,
    VoiceFix,
    HostRetype,
    FixWord,
    HistoryEdit,
    MeetingEdit,
    SpeakerName,
    Passive,
    Review,
    Manual,
    Migrated,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "camelCase")]
pub enum SourceRef {
    Dictation(String),
    Meeting(String),
}

impl SourceRef {
    pub fn id(&self) -> &str {
        match self {
            Self::Dictation(id) | Self::Meeting(id) => id,
        }
    }
    pub fn is_dictation(&self) -> bool {
        matches!(self, Self::Dictation(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub source: SourceRef,
    pub kind: SourceKind,
    pub day: u32,
    #[serde(default)]
    pub weight: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IgnoreTarget {
    Mapping,
    Term,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suppression {
    pub target: IgnoreTarget,
    pub keyed_digest: [u8; 32],
    pub day: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyContextEntry {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub scope: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LexiconEntry {
    pub id: String,
    pub kind: TermKind,
    pub written: String,
    #[serde(default)]
    pub heard: Option<String>,
    #[serde(default)]
    pub gate: Gate,
    pub tier: Tier,
    pub risk: HeardRisk,
    #[serde(default)]
    pub fixes: u16,
    #[serde(default)]
    pub keeps: u16,
    #[serde(default)]
    pub applied: u16,
    #[serde(default)]
    pub reverted: u16,
    #[serde(default)]
    pub probation_left: u8,
    #[serde(default)]
    pub sources: Vec<EvidenceRef>,
    #[serde(default)]
    pub near_cues: Vec<(String, u8)>,
    #[serde(default)]
    pub promoted_to: Option<String>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub last_positive_day: u32,
    #[serde(default)]
    pub last_negative_day: u32,
    #[serde(default)]
    pub reason_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LexiconFile {
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default = "random_key")]
    pub suppression_key: [u8; 32],
    #[serde(default)]
    pub entries: Vec<LexiconEntry>,
    #[serde(default)]
    pub suppressions: Vec<Suppression>,
    #[serde(default)]
    pub legacy: Vec<LegacyContextEntry>,
    #[serde(default)]
    pub usage: HashMap<String, UsageCounter>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCounter {
    pub applied: u32,
    pub last_day: u32,
    #[serde(default)]
    pub reverted: u32,
}

fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

impl Default for LexiconFile {
    fn default() -> Self {
        Self {
            schema_version: LEXICON_SCHEMA_VERSION,
            revision: 1,
            suppression_key: random_key(),
            entries: Vec::new(),
            suppressions: Vec::new(),
            legacy: Vec::new(),
            usage: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FixOutcome {
    Ignored,
    EvidenceAdded,
    Suggested,
    Activated,
}

/// Typed consumer projection for one utterance or passage.
#[derive(Clone, Debug, Default)]
pub struct LexiconPlan {
    pub cleanup_vocabulary: Vec<crate::cleanup::Vocabulary>,
    pub confirmed_structured: Vec<crate::structured_clean::VocabularyEntry>,
    pub local_proofreader_hints: Vec<String>,
    pub online_proofreader_hints: Vec<String>,
    /// Confirmed written spellings eligible for vocabulary-only marks and false-mark suppression.
    pub trust_confirmed_words: Vec<String>,
    /// Active local hints/spellings: rank alternatives on already-doubted words and suppress
    /// false marks above 0.25 confidence, but never create vocabulary-only marks.
    pub trust_active_hints: Vec<String>,
    /// Suggested (heard, written) pairs offered as alternatives on already-doubted words.
    pub trust_suggested_pairs: Vec<(String, String)>,
    /// Auto-active non-word aliases currently under probation (first N uses).
    pub probation_pairs: Vec<(String, String)>,
}

// --- UI Projection Types ---

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub date: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedItemView {
    pub id: String,
    pub kind: TermKind,
    pub heard: Option<String>,
    pub written: String,
    pub tier: Tier,
    pub risk: HeardRisk,
    pub near_cues: Vec<String>,
    pub fixes: u16,
    pub keeps: u16,
    pub applied: u16,
    pub reverted: u16,
    pub probation_left: u8,
    pub reason: String,
    pub sources: Vec<SourceSummary>,
    pub revision: u64,
    pub promoted_to: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellingClusterView {
    pub key: String,
    pub variants: Vec<String>,
    pub total_sources: usize,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedWordMeta {
    pub id: String,
    pub applied: u32,
    pub last_day: u32,
    pub needs_check: bool,
    pub inert_legacy: bool,
    pub extra_aliases: Vec<LearnedItemView>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordsOverview {
    pub words: Vec<VocabularyEntry>,
    pub word_meta: HashMap<String, ConfirmedWordMeta>,
    pub active_learned: Vec<LearnedItemView>,
    pub review_items: Vec<LearnedItemView>,
    pub clusters: Vec<SpellingClusterView>,
    pub suppressed_count: usize,
    pub lexicon_revision: u64,
}

// --- Day clock ---

pub fn today_day() -> u32 {
    let secs = OffsetDateTime::now_utc().unix_timestamp();
    (secs.max(0) as u64 / 86_400) as u32
}

// --- Unified guardrails ---

const PROTECTED_WORDS: &[&str] = &[
    // Negations, quantifiers, conditionals, modals, logical connectors
    "no", "not", "never", "none", "neither", "nor", "only", "all", "any", "if", "unless",
    "until", "except", "without", "and", "or", "can", "cannot", "cant", "can't", "could",
    "do", "dont", "don't", "does", "doesn't", "didn't", "must", "mustn't", "should",
    "would", "wouldn't", "will", "wont", "won't", "isn't", "wasn't", "may", "might",
    "maybe", "perhaps", "probably", "possibly",
    // Action / instruction verbs
    "increase", "decrease", "enable", "disable", "allow", "deny", "start", "stop", "keep",
    "delete", "remove", "save", "send", "open", "close", "on", "off",
    // Numbers and quantities
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen",
    "eighteen", "nineteen", "twenty", "thirty", "forty", "fifty", "sixty", "seventy",
    "eighty", "ninety", "hundred", "thousand", "million", "billion", "half", "quarter",
    "percent",
    // Units
    "seconds", "minutes", "hours", "days", "meter", "meters", "liters", "grams",
    "kilograms", "inches", "feet",
    // Directions, order, days, months
    "left", "right", "up", "down", "above", "below", "top", "bottom", "front", "back",
    "forward", "backward", "north", "south", "east", "west", "before", "after", "next",
    "previous", "first", "second", "third", "today", "tomorrow", "yesterday", "morning",
    "evening", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
    "january", "february", "march", "april", "june", "july", "august", "september",
    "october", "november", "december",
];

const KNOWN_COLLISIONS: &[&str] = &["quinn", "seoul", "jason", "may", "us"];

pub fn is_protected_word(word: &str) -> bool {
    let lower = word.trim().to_lowercase();
    let lower = lower.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    if lower.is_empty() {
        return true;
    }
    if lower.bytes().any(|b| b.is_ascii_digit()) {
        return true;
    }
    PROTECTED_WORDS.contains(&lower)
}

pub fn is_ordinary_word(word: &str) -> bool {
    let lower = word.trim().to_lowercase();
    let lower = lower.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    if lower.is_empty() {
        return false;
    }
    crate::punctuation::common_lowercase(lower) || KNOWN_COLLISIONS.contains(&lower)
}

pub fn classify_heard(heard: &str) -> HeardRisk {
    let heard = heard.trim();
    if heard.is_empty()
        || heard.len() > MAX_LEARNED_BYTES
        || heard.chars().any(char::is_control)
        || heard.bytes().any(|b| b.is_ascii_digit())
    {
        return HeardRisk::Protected;
    }
    let parts: Vec<&str> = heard.split_whitespace().collect();
    if parts.is_empty() || parts.len() > MAX_LEARNED_WORDS {
        return HeardRisk::Protected;
    }
    if parts.iter().any(|w| is_protected_word(w)) {
        return HeardRisk::Protected;
    }
    if parts.iter().all(|w| is_ordinary_word(w)) {
        return HeardRisk::Ordinary;
    }
    HeardRisk::Nonword
}

pub fn valid_written(written: &str) -> bool {
    let written = written.trim();
    if written.is_empty()
        || written.len() > MAX_LEARNED_BYTES
        || written.chars().any(char::is_control)
    {
        return false;
    }
    let parts: Vec<&str> = written.split_whitespace().collect();
    if parts.is_empty() || parts.len() > MAX_LEARNED_WORDS {
        return false;
    }
    if parts.iter().any(|w| is_protected_word(w)) {
        return false;
    }
    written.chars().any(char::is_alphabetic)
}

/// True when both sides are lowercase ordinary English words with different
/// letters (e.g. "affect" -> "effect"), indicating a grammar/wording fix rather
/// than a vocabulary term.
pub fn is_ordinary_grammar_swap(heard: &str, written: &str) -> bool {
    let h = heard.trim();
    let w = written.trim();
    if h.eq_ignore_ascii_case(w) {
        return false;
    }
    let w_has_special_case = w.chars().any(char::is_uppercase)
        || w.contains(['-', '.', '/', '+']);
    if w_has_special_case {
        return false;
    }
    let h_parts: Vec<&str> = h.split_whitespace().collect();
    let w_parts: Vec<&str> = w.split_whitespace().collect();
    h_parts.iter().all(|p| is_ordinary_word(p)) && w_parts.iter().all(|p| is_ordinary_word(p))
}

pub fn normalize_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn sound_key(text: &str) -> String {
    let k = normalize_key(text);
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
                if i == 0 {
                    'a'
                } else {
                    '_'
                }
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
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

pub fn safe_alias_distance(heard: &str, written: &str) -> bool {
    let a = normalize_key(heard);
    let b = normalize_key(written);
    if a.len() < 2 || b.len() < 2 {
        return false;
    }
    if a == b {
        return true;
    }
    let dist = levenshtein(&a, &b);
    let longest = a.len().max(b.len());
    if dist <= 3.min((longest + 1) / 2) && dist <= longest.saturating_sub(1) {
        return true;
    }
    // Also permit phonetically very close pairs (e.g. "kuber net ease" -> "Kubernetes").
    let sa = sound_key(&a);
    let sb = sound_key(&b);
    let sd = levenshtein(&sa, &sb);
    let slen = sa.len().max(sb.len());
    slen >= 3 && sd <= 2 && sd * 3 <= slen && dist <= (longest * 2) / 3
}

pub fn determine_kind(heard: &str, written: &str) -> TermKind {
    if normalize_key(heard) == normalize_key(written)
        && heard.trim().split_whitespace().count() == written.trim().split_whitespace().count()
    {
        TermKind::Spelling
    } else {
        TermKind::Alias
    }
}

// --- Device-keyed HMAC suppressions ---

pub fn suppression_digest(
    key: &[u8; 32],
    target: IgnoreTarget,
    heard: Option<&str>,
    written: &str,
) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    match target {
        IgnoreTarget::Mapping => {
            mac.update(b"sj-suppress-v1:mapping:");
            mac.update(heard.unwrap_or("").trim().to_lowercase().as_bytes());
            mac.update(b"\x1f");
            mac.update(written.trim().to_lowercase().as_bytes());
        }
        IgnoreTarget::Term => {
            mac.update(b"sj-suppress-v1:term:");
            mac.update(normalize_key(written).as_bytes());
        }
    }
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

pub fn is_suppressed(lexicon: &LexiconFile, heard: Option<&str>, written: &str) -> bool {
    let term_digest = suppression_digest(&lexicon.suppression_key, IgnoreTarget::Term, None, written);
    if lexicon
        .suppressions
        .iter()
        .any(|s| s.target == IgnoreTarget::Term && s.keyed_digest == term_digest)
    {
        return true;
    }
    if let Some(h) = heard {
        let map_digest =
            suppression_digest(&lexicon.suppression_key, IgnoreTarget::Mapping, Some(h), written);
        if lexicon
            .suppressions
            .iter()
            .any(|s| s.target == IgnoreTarget::Mapping && s.keyed_digest == map_digest)
        {
            return true;
        }
    }
    false
}

pub fn add_suppression(
    lexicon: &mut LexiconFile,
    target: IgnoreTarget,
    heard: Option<&str>,
    written: &str,
    day: u32,
) {
    let digest = suppression_digest(&lexicon.suppression_key, target, heard, written);
    if let Some(existing) = lexicon
        .suppressions
        .iter_mut()
        .find(|s| s.target == target && s.keyed_digest == digest)
    {
        existing.day = day;
        return;
    }
    lexicon.suppressions.push(Suppression {
        target,
        keyed_digest: digest,
        day,
    });
    if lexicon.suppressions.len() > MAX_SUPPRESSIONS {
        lexicon.suppressions.sort_by_key(|s| std::cmp::Reverse(s.day));
        lexicon.suppressions.truncate(MAX_SUPPRESSIONS);
    }
    lexicon.revision = lexicon.revision.saturating_add(1);
}

pub fn clear_suppression(lexicon: &mut LexiconFile, heard: Option<&str>, written: &str) -> bool {
    let before = lexicon.suppressions.len();
    let term_digest = suppression_digest(&lexicon.suppression_key, IgnoreTarget::Term, None, written);
    let map_digest = heard.map(|h| {
        suppression_digest(&lexicon.suppression_key, IgnoreTarget::Mapping, Some(h), written)
    });
    lexicon.suppressions.retain(|s| {
        if s.target == IgnoreTarget::Term && s.keyed_digest == term_digest {
            return false;
        }
        if let Some(md) = map_digest {
            if s.target == IgnoreTarget::Mapping && s.keyed_digest == md {
                return false;
            }
        }
        true
    });
    let changed = lexicon.suppressions.len() != before;
    if changed {
        lexicon.revision = lexicon.revision.saturating_add(1);
    }
    changed
}

// --- Diff & nearby-cue extraction ---

fn trim_word_punct(s: &str) -> &str {
    s.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '\'' | '’' | '-' | '+' | '.'))
}

fn tokenize_words(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(trim_word_punct)
        .filter(|w| !w.is_empty())
        .collect()
}

/// Extract a single narrow substitution (including casing fixes and single-word
/// dictations) together with nearby same-sentence context cues.
pub fn extract_diff_pair(old_text: &str, new_text: &str) -> Option<(String, String, Vec<String>)> {
    let old_trimmed = old_text.trim();
    let new_trimmed = new_text.trim();
    if old_trimmed == new_trimmed
        || old_trimmed.contains(['\n', '\r'])
        || new_trimmed.contains(['\n', '\r'])
    {
        return None;
    }
    let old_words = tokenize_words(old_trimmed);
    let new_words = tokenize_words(new_trimmed);
    if old_words.is_empty() || new_words.is_empty() {
        return None;
    }
    let mut prefix = 0usize;
    while prefix < old_words.len()
        && prefix < new_words.len()
        && old_words[prefix] == new_words[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old_words.len() - prefix
        && suffix < new_words.len() - prefix
        && old_words[old_words.len() - 1 - suffix] == new_words[new_words.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_span = old_words[prefix..old_words.len() - suffix].join(" ");
    let new_span = new_words[prefix..new_words.len() - suffix].join(" ");
    if old_span.is_empty() || new_span.is_empty() || old_span == new_span {
        return None;
    }
    if old_span.split_whitespace().count() > MAX_LEARNED_WORDS
        || new_span.split_whitespace().count() > MAX_LEARNED_WORDS
    {
        return None;
    }
    // Reject full-sentence rewrites of multi-word utterances where almost nothing stayed fixed.
    if old_words.len() >= 4 && prefix + suffix < 2 {
        return None;
    }
    let nearby = collect_nearby_from_tokens(&old_words, prefix, old_words.len() - suffix);
    Some((old_span, new_span, nearby))
}

fn collect_nearby_from_tokens(tokens: &[&str], start: usize, end: usize) -> Vec<String> {
    let mut cues = Vec::new();
    let left = start.saturating_sub(4)..start;
    let right = end..(end + 4).min(tokens.len());
    for idx in left.chain(right) {
        let lower = tokens[idx].to_lowercase();
        let clean = lower.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.len() >= 4
            && clean.chars().all(char::is_alphabetic)
            && !is_protected_word(clean)
            && !is_stopword_cue(clean)
            && !cues.iter().any(|c| c == clean)
        {
            cues.push(clean.to_owned());
        }
    }
    cues
}

pub fn extract_nearby_cues(text: &str, phrase: &str) -> Vec<String> {
    let tokens = tokenize_words(text);
    let needle: Vec<String> = tokenize_words(phrase)
        .into_iter()
        .map(str::to_lowercase)
        .collect();
    if needle.is_empty() || tokens.len() < needle.len() {
        return Vec::new();
    }
    let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    for i in 0..=lower.len() - needle.len() {
        if lower[i..i + needle.len()] == needle[..] {
            return collect_nearby_from_tokens(&tokens, i, i + needle.len());
        }
    }
    Vec::new()
}

fn is_stopword_cue(w: &str) -> bool {
    matches!(
        w,
        "that" | "this" | "with" | "from" | "have" | "been" | "were" | "they" | "them"
            | "their" | "there" | "what" | "when" | "where" | "which" | "while" | "about"
            | "into" | "just" | "like" | "make" | "made" | "more" | "some" | "than"
            | "then" | "very" | "your" | "also" | "here" | "need" | "want" | "know"
            | "think" | "said" | "says" | "going" | "really" | "actually" | "basically"
            | "please" | "thanks" | "hello" | "okay" | "sure" | "yeah"
    )
}

// --- Signal ingestion ---

fn distinct_source_count(sources: &[EvidenceRef]) -> usize {
    sources
        .iter()
        .filter(|s| s.weight > 0)
        .map(|s| &s.source)
        .collect::<HashSet<_>>()
        .len()
}

fn distinct_positive_days(sources: &[EvidenceRef]) -> usize {
    sources
        .iter()
        .filter(|s| s.weight > 0)
        .map(|s| s.day)
        .collect::<HashSet<_>>()
        .len()
}

fn positive_fix_sources(sources: &[EvidenceRef]) -> usize {
    sources
        .iter()
        .filter(|s| {
            s.weight >= 3
                && matches!(
                    s.kind,
                    SourceKind::OverlayChoice
                        | SourceKind::VoiceFix
                        | SourceKind::HostRetype
                        | SourceKind::FixWord
                        | SourceKind::HistoryEdit
                        | SourceKind::MeetingEdit
                )
        })
        .map(|s| &s.source)
        .collect::<HashSet<_>>()
        .len()
}

fn add_or_update_evidence(entry: &mut LexiconEntry, ev: EvidenceRef) -> bool {
    if let Some(existing) = entry.sources.iter_mut().find(|s| s.source == ev.source) {
        if ev.weight > existing.weight || (ev.weight < 0 && existing.weight < 0) {
            *existing = ev;
            return true;
        }
        return false;
    }
    entry.sources.push(ev);
    if entry.sources.len() > MAX_SOURCES_PER_ENTRY {
        entry.sources.sort_by_key(|s| std::cmp::Reverse(s.day));
        entry.sources.truncate(MAX_SOURCES_PER_ENTRY);
    }
    true
}

fn merge_near_cues(target: &mut Vec<(String, u8)>, cues: &[String]) {
    for cue in cues {
        let lower = cue.trim().to_lowercase();
        if lower.len() < 3 || !lower.chars().all(char::is_alphanumeric) {
            continue;
        }
        if let Some(item) = target.iter_mut().find(|(w, _)| *w == lower) {
            item.1 = item.1.saturating_add(1);
        } else {
            target.push((lower, 1));
        }
    }
    target.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    target.truncate(MAX_NEAR_CUES);
}

fn has_conflict(lexicon: &LexiconFile, heard: &str, written: &str, self_id: &str) -> bool {
    lexicon.entries.iter().any(|other| {
        other.id != self_id
            && other.tier != Tier::Observed
            && other
                .heard
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(heard))
            && other.written != written
    })
}

fn recompute_entry_tier(entry: &mut LexiconEntry, has_destination_conflict: bool) {
    let fix_sources = positive_fix_sources(&entry.sources);
    let distinct_sources = distinct_source_count(&entry.sources);
    let distinct_days = distinct_positive_days(&entry.sources);
    let keep_sources = entry
        .sources
        .iter()
        .filter(|s| s.kind == SourceKind::OverlayKeep && s.weight > 0)
        .map(|s| &s.source)
        .collect::<HashSet<_>>()
        .len();
    let speaker_source = entry.sources.iter().any(|s| s.kind == SourceKind::SpeakerName);
    let review_source = entry.sources.iter().any(|s| s.kind == SourceKind::Review);
    let migrated_source = entry.sources.iter().any(|s| s.kind == SourceKind::Migrated);

    match entry.kind {
        TermKind::Spelling => {
            if fix_sources >= 1 && entry.reverted == 0 && !has_destination_conflict {
                entry.tier = Tier::Active;
            } else if fix_sources >= 1 || review_source || migrated_source {
                entry.tier = Tier::Suggested;
            } else {
                entry.tier = Tier::Observed;
            }
        }
        TermKind::Alias => {
            if entry.risk == HeardRisk::Nonword
                && fix_sources >= 2
                && entry.reverted == 0
                && !has_destination_conflict
            {
                if entry.tier != Tier::Active {
                    entry.probation_left = PROBATION_USES;
                }
                entry.tier = Tier::Active;
            } else if fix_sources >= 1 || review_source || migrated_source {
                entry.tier = Tier::Suggested;
            } else {
                entry.tier = Tier::Observed;
            }
        }
        TermKind::Hint => {
            if speaker_source || keep_sources >= 2 {
                entry.tier = Tier::Active;
            } else if (distinct_sources >= 3 && distinct_days >= 2)
                || fix_sources >= 1
                || review_source
                || migrated_source
            {
                entry.tier = Tier::Suggested;
            } else {
                entry.tier = Tier::Observed;
            }
        }
    }
}

/// Record an explicit heard -> written correction from Trust Layer, FixWord,
/// History edit, or Meeting transcript edit.
pub fn record_fix(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    source: SourceRef,
    kind: SourceKind,
    heard: &str,
    written: &str,
    nearby: &[String],
    day: u32,
) -> FixOutcome {
    let heard = heard.trim();
    let written = written.trim();
    if heard.is_empty() || written.is_empty() || heard == written {
        return FixOutcome::Ignored;
    }
    if !valid_written(written) {
        return FixOutcome::Ignored;
    }
    let risk = classify_heard(heard);
    if risk == HeardRisk::Protected {
        return FixOutcome::Ignored;
    }
    if is_ordinary_grammar_swap(heard, written) {
        return FixOutcome::Ignored;
    }
    if !safe_alias_distance(heard, written) {
        return FixOutcome::Ignored;
    }
    if is_suppressed(lexicon, Some(heard), written) {
        return FixOutcome::Ignored;
    }

    // If this exact mapping is already confirmed in vocabulary.json, nothing new to learn.
    let linked_vocab = confirmed.iter().find(|v| {
        v.enabled && v.written == written && v.kind != VocabularyKind::Hint
    });
    if let Some(v) = linked_vocab {
        if v.heard.eq_ignore_ascii_case(heard)
            || v.aliases.iter().any(|a| a.eq_ignore_ascii_case(heard))
        {
            return FixOutcome::Ignored;
        }
    }

    let term_kind = determine_kind(heard, written);
    let promoted_to = linked_vocab.map(|v| v.id.clone());

    let idx = lexicon.entries.iter().position(|e| {
        e.kind == term_kind
            && e.written == written
            && e.heard
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(heard))
    });

    let entry_id = if let Some(i) = idx {
        let entry = &mut lexicon.entries[i];
        let added = add_or_update_evidence(
            entry,
            EvidenceRef {
                source,
                kind,
                day,
                weight: 3,
            },
        );
        if added {
            entry.fixes = entry.fixes.saturating_add(1);
        }
        entry.last_positive_day = day;
        entry.promoted_to = promoted_to;
        if risk == HeardRisk::Ordinary {
            merge_near_cues(&mut entry.near_cues, nearby);
        }
        entry.revision = entry.revision.saturating_add(1);
        entry.id.clone()
    } else {
        let id = crate::store::new_id();
        let mut near_cues = Vec::new();
        if risk == HeardRisk::Ordinary {
            merge_near_cues(&mut near_cues, nearby);
        }
        let gate = if risk == HeardRisk::Ordinary {
            Gate::Near(near_cues.iter().map(|(w, _)| w.clone()).collect())
        } else {
            Gate::Always
        };
        lexicon.entries.push(LexiconEntry {
            id: id.clone(),
            kind: term_kind,
            written: written.to_owned(),
            heard: Some(heard.to_owned()),
            gate,
            tier: Tier::Observed,
            risk,
            fixes: 1,
            keeps: 0,
            applied: 0,
            reverted: 0,
            probation_left: 0,
            sources: vec![EvidenceRef {
                source,
                kind,
                day,
                weight: 3,
            }],
            near_cues,
            promoted_to,
            revision: 1,
            last_positive_day: day,
            last_negative_day: 0,
            reason_hint: None,
        });
        id
    };

    let conflict = has_conflict(lexicon, heard, written, &entry_id);
    let Some(entry) = lexicon.entries.iter_mut().find(|e| e.id == entry_id) else {
        return FixOutcome::Ignored;
    };
    let prev_tier = entry.tier;
    recompute_entry_tier(entry, conflict);
    let next_tier = entry.tier;
    lexicon.revision = lexicon.revision.saturating_add(1);

    match (prev_tier, next_tier) {
        (_, Tier::Active) if prev_tier != Tier::Active => FixOutcome::Activated,
        (_, Tier::Active) => FixOutcome::EvidenceAdded,
        (Tier::Observed, Tier::Suggested) => FixOutcome::Suggested,
        _ => FixOutcome::EvidenceAdded,
    }
}

/// Record user choosing Keep on a single Doubtful Words mark (never KeepAll).
pub fn record_keep(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    source: SourceRef,
    word: &str,
    offered_alternatives: &[String],
    day: u32,
) -> bool {
    let mut changed = false;
    let word = word.trim();
    if word.is_empty() {
        return false;
    }

    // 1. Weak negative vote against any unconfirmed offered alternative for this word.
    for alt in offered_alternatives {
        if let Some(entry) = lexicon.entries.iter_mut().find(|e| {
            e.written == *alt
                && e.heard
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(word))
        }) {
            if add_or_update_evidence(
                entry,
                EvidenceRef {
                    source: source.clone(),
                    kind: SourceKind::OverlayKeep,
                    day,
                    weight: -1,
                },
            ) {
                entry.last_negative_day = day;
                entry.revision = entry.revision.saturating_add(1);
                changed = true;
            }
        }
    }

    // 2. If the kept word is an unknown/OOV word, Keeping it confirms its spelling.
    if word.len() >= 3
        && valid_written(word)
        && !is_ordinary_word(word)
        && !is_suppressed(lexicon, None, word)
        && !confirmed
            .iter()
            .any(|v| v.enabled && v.written.eq_ignore_ascii_case(word))
    {
        if let Some(entry) = lexicon
            .entries
            .iter_mut()
            .find(|e| e.kind == TermKind::Hint && e.written.eq_ignore_ascii_case(word))
        {
            if add_or_update_evidence(
                entry,
                EvidenceRef {
                    source,
                    kind: SourceKind::OverlayKeep,
                    day,
                    weight: 2,
                },
            ) {
                entry.keeps = entry.keeps.saturating_add(1);
                entry.last_positive_day = day;
                entry.revision = entry.revision.saturating_add(1);
                recompute_entry_tier(entry, false);
                changed = true;
            }
        } else {
            let mut entry = LexiconEntry {
                id: crate::store::new_id(),
                kind: TermKind::Hint,
                written: word.to_owned(),
                heard: None,
                gate: Gate::Always,
                tier: Tier::Observed,
                risk: HeardRisk::Nonword,
                fixes: 0,
                keeps: 1,
                applied: 0,
                reverted: 0,
                probation_left: 0,
                sources: vec![EvidenceRef {
                    source,
                    kind: SourceKind::OverlayKeep,
                    day,
                    weight: 2,
                }],
                near_cues: Vec::new(),
                promoted_to: None,
                revision: 1,
                last_positive_day: day,
                last_negative_day: 0,
                reason_hint: None,
            };
            recompute_entry_tier(&mut entry, false);
            lexicon.entries.push(entry);
            changed = true;
        }
    }

    if changed {
        lexicon.revision = lexicon.revision.saturating_add(1);
    }
    changed
}

/// Record user choosing "As heard" to undo an automatic replacement.
pub fn record_revert(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    heard: &str,
    written: &str,
    day: u32,
) -> bool {
    let heard = heard.trim();
    let written = written.trim();
    if heard.is_empty() || written.is_empty() {
        return false;
    }

    // Check if this was a Confirmed word -> never auto-disable, just record revert counter for CHECK stamp.
    if let Some(v) = confirmed.iter().find(|v| {
        v.enabled
            && v.written == written
            && (v.heard.eq_ignore_ascii_case(heard)
                || v.aliases.iter().any(|a| a.eq_ignore_ascii_case(heard)))
    }) {
        let counter = lexicon.usage.entry(v.id.clone()).or_default();
        counter.reverted = counter.reverted.saturating_add(1);
        counter.last_day = day;
        lexicon.revision = lexicon.revision.saturating_add(1);
        return true;
    }

    // Otherwise check local lexicon entries.
    if let Some(pos) = lexicon.entries.iter().position(|e| {
        e.written == written
            && e.heard
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case(heard))
    }) {
        let should_suppress = {
            let entry = &mut lexicon.entries[pos];
            entry.reverted = entry.reverted.saturating_add(1);
            entry.probation_left = 0;
            entry.last_negative_day = day;
            entry.revision = entry.revision.saturating_add(1);
            if entry.reverted >= 2 {
                true
            } else {
                entry.tier = Tier::Suggested;
                false
            }
        };
        if should_suppress {
            lexicon.entries.remove(pos);
            add_suppression(lexicon, IgnoreTarget::Mapping, Some(heard), written, day);
        } else {
            lexicon.revision = lexicon.revision.saturating_add(1);
        }
        return true;
    }
    false
}

/// Record a user-entered meeting speaker name as an immediate Hint.
pub fn record_speaker_name(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    meeting_id: &str,
    name: &str,
    day: u32,
) -> bool {
    let name = name.trim();
    if name.len() < 2
        || name.len() > MAX_LEARNED_BYTES
        || !valid_written(name)
        || is_generated_speaker_label(name)
        || is_suppressed(lexicon, None, name)
    {
        return false;
    }
    if confirmed
        .iter()
        .any(|v| v.enabled && v.written.eq_ignore_ascii_case(name))
    {
        return false;
    }

    let source = SourceRef::Meeting(meeting_id.to_owned());
    if let Some(entry) = lexicon
        .entries
        .iter_mut()
        .find(|e| e.kind == TermKind::Hint && e.written.eq_ignore_ascii_case(name))
    {
        let added = add_or_update_evidence(
            entry,
            EvidenceRef {
                source,
                kind: SourceKind::SpeakerName,
                day,
                weight: 3,
            },
        );
        if added {
            entry.last_positive_day = day;
            entry.revision = entry.revision.saturating_add(1);
            // Seen across multiple meetings -> upgrade gate from single meeting to global.
            if distinct_source_count(&entry.sources) >= 2 {
                entry.gate = Gate::Always;
            }
            recompute_entry_tier(entry, false);
            lexicon.revision = lexicon.revision.saturating_add(1);
        }
        return added;
    }

    lexicon.entries.push(LexiconEntry {
        id: crate::store::new_id(),
        kind: TermKind::Hint,
        written: name.to_owned(),
        heard: None,
        gate: Gate::Meeting(meeting_id.to_owned()),
        tier: Tier::Active,
        risk: HeardRisk::Nonword,
        fixes: 0,
        keeps: 0,
        applied: 0,
        reverted: 0,
        probation_left: 0,
        sources: vec![EvidenceRef {
            source,
            kind: SourceKind::SpeakerName,
            day,
            weight: 3,
        }],
        near_cues: Vec::new(),
        promoted_to: None,
        revision: 1,
        last_positive_day: day,
        last_negative_day: 0,
        reason_hint: Some("speaker_name".into()),
    });
    lexicon.revision = lexicon.revision.saturating_add(1);
    true
}

fn is_generated_speaker_label(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    matches!(lower.as_str(), "you" | "other" | "speaker" | "unknown")
        || lower
            .strip_prefix("speaker ")
            .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
}

/// Scan a completed dictation or meeting transcript for distinctive recurring
/// words (OOV vs `common_lowercase`, camelCase/mixed-case, acronyms). Counts
/// at most once per normalized term per `SourceRef`.
pub fn observe_passive_text(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    source: SourceRef,
    text: &str,
    day: u32,
) -> bool {
    let candidates = extract_passive_terms(text);
    if candidates.is_empty() {
        return false;
    }
    let confirmed_keys: HashSet<String> = confirmed
        .iter()
        .filter(|v| v.enabled)
        .map(|v| normalize_key(&v.written))
        .collect();

    let mut changed = false;
    for term in candidates {
        let key = normalize_key(&term);
        if confirmed_keys.contains(&key) || is_suppressed(lexicon, None, &term) {
            continue;
        }
        if let Some(entry) = lexicon
            .entries
            .iter_mut()
            .find(|e| e.kind == TermKind::Hint && e.written.eq_ignore_ascii_case(&term))
        {
            if add_or_update_evidence(
                entry,
                EvidenceRef {
                    source: source.clone(),
                    kind: SourceKind::Passive,
                    day,
                    weight: 1,
                },
            ) {
                // Prefer the richer mixed-case/capitalized surface form when observed.
                if case_richness(&term) > case_richness(&entry.written) {
                    entry.written = term;
                }
                entry.last_positive_day = day;
                entry.revision = entry.revision.saturating_add(1);
                recompute_entry_tier(entry, false);
                changed = true;
            }
        } else {
            let mut entry = LexiconEntry {
                id: crate::store::new_id(),
                kind: TermKind::Hint,
                written: term,
                heard: None,
                gate: Gate::Always,
                tier: Tier::Observed,
                risk: HeardRisk::Nonword,
                fixes: 0,
                keeps: 0,
                applied: 0,
                reverted: 0,
                probation_left: 0,
                sources: vec![EvidenceRef {
                    source: source.clone(),
                    kind: SourceKind::Passive,
                    day,
                    weight: 1,
                }],
                near_cues: Vec::new(),
                promoted_to: None,
                revision: 1,
                last_positive_day: day,
                last_negative_day: 0,
                reason_hint: None,
            };
            recompute_entry_tier(&mut entry, false);
            lexicon.entries.push(entry);
            changed = true;
        }
    }

    if changed {
        decay_and_prune(lexicon, day);
        lexicon.revision = lexicon.revision.saturating_add(1);
    }
    changed
}

fn case_richness(word: &str) -> u8 {
    let has_upper = word.chars().any(char::is_uppercase);
    let has_lower = word.chars().any(char::is_lowercase);
    let internal_upper = word.chars().skip(1).any(char::is_uppercase);
    if internal_upper && has_lower {
        3 // camelCase / PascalCase e.g. SonoJot, iOS
    } else if has_upper && !has_lower {
        2 // Acronym e.g. SQL, CUDA
    } else if has_upper {
        1 // Title Case
    } else {
        0
    }
}

fn extract_passive_terms(text: &str) -> Vec<String> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut at_sentence_start = true;
    for raw_tok in text.split_whitespace() {
        let trimmed = raw_tok.trim_matches(|c: char| {
            !c.is_alphanumeric() && !matches!(c, '-' | '\'' | '’' | '.')
        });
        let ends_sentence = raw_tok
            .trim_end_matches(['"', '”', '\'', '’', ')', ']'])
            .ends_with(['.', '?', '!']);

        let clean = trimmed.trim_end_matches(['.', '\'', '’']);
        if is_eligible_passive_token(clean, at_sentence_start) {
            let key = normalize_key(clean);
            seen.entry(key)
                .and_modify(|existing| {
                    if case_richness(clean) > case_richness(existing) {
                        *existing = clean.to_owned();
                    }
                })
                .or_insert_with(|| clean.to_owned());
        }
        at_sentence_start = ends_sentence;
    }
    seen.into_values().collect()
}

fn is_eligible_passive_token(token: &str, at_sentence_start: bool) -> bool {
    if token.len() < 3 || token.len() > 40 {
        return false;
    }
    // Skip URLs, emails, paths, code identifiers, digits.
    if token.contains(['@', '/', '\\', '_', ':']) || token.bytes().any(|b| b.is_ascii_digit()) {
        return false;
    }
    if !token
        .chars()
        .all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '’'))
    {
        return false;
    }
    if is_protected_word(token) {
        return false;
    }
    let lower = token.to_lowercase();
    let internal_upper = token.chars().skip(1).any(char::is_uppercase);
    let has_lower = token.chars().any(char::is_lowercase);
    let is_acronym = token.len() >= 2 && token.len() <= 6 && !has_lower && token.chars().all(char::is_alphabetic);

    // Distinctive mixed-case (SonoJot, iOS, MacBook) is always interesting, even at sentence start.
    if internal_upper && has_lower {
        return true;
    }
    // Acronym not in ordinary words (e.g. TDT, ONNX, VRAM).
    if is_acronym && !is_ordinary_word(&lower) {
        return true;
    }
    // Out-of-lexicon word (not in 49k common_lowercase).
    let in_dict = is_ordinary_word(&lower)
        || token
            .split('-')
            .all(|part| part.len() >= 2 && is_ordinary_word(part));
    if !in_dict {
        // Require at least 4 chars for plain all-lowercase OOVs to cut typos/fragments.
        return token.len() >= 4 || (!at_sentence_start && token.chars().next().is_some_and(char::is_uppercase));
    }
    false
}

/// Route on-demand LLM vocabulary suggestions into persistent Review entries.
pub fn record_review_proposals(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    proposals: &[(String, String)],
    source_id: Option<&str>,
    day: u32,
) -> usize {
    let mut added = 0usize;
    let source = source_id
        .map(|id| SourceRef::Dictation(id.to_owned()))
        .unwrap_or_else(|| SourceRef::Dictation("review".into()));

    for (heard, written) in proposals {
        let heard = heard.trim();
        let written = written.trim();
        if heard.is_empty()
            || written.is_empty()
            || heard == written
            || !valid_written(written)
            || classify_heard(heard) == HeardRisk::Protected
            || is_ordinary_grammar_swap(heard, written)
            || is_suppressed(lexicon, Some(heard), written)
        {
            continue;
        }
        if confirmed
            .iter()
            .any(|v| v.heard.eq_ignore_ascii_case(heard) || v.written == written)
        {
            continue;
        }
        if lexicon.entries.iter().any(|e| {
            e.written == written
                && e.heard
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(heard))
        }) {
            continue;
        }
        let risk = classify_heard(heard);
        let term_kind = determine_kind(heard, written);
        lexicon.entries.push(LexiconEntry {
            id: crate::store::new_id(),
            kind: term_kind,
            written: written.to_owned(),
            heard: Some(heard.to_owned()),
            gate: if risk == HeardRisk::Ordinary {
                Gate::Near(Vec::new())
            } else {
                Gate::Always
            },
            tier: Tier::Suggested,
            risk,
            fixes: 0,
            keeps: 0,
            applied: 0,
            reverted: 0,
            probation_left: 0,
            sources: vec![EvidenceRef {
                source: source.clone(),
                kind: SourceKind::Review,
                day,
                weight: 0,
            }],
            near_cues: Vec::new(),
            promoted_to: None,
            revision: 1,
            last_positive_day: day,
            last_negative_day: 0,
            reason_hint: Some("review_proposal".into()),
        });
        added += 1;
    }
    if added > 0 {
        lexicon.revision = lexicon.revision.saturating_add(1);
    }
    added
}

/// Record that a cleanup run applied these (heard, written) pairs.
/// Updates usage stats for Confirmed words and decrements probation for auto-Active aliases.
pub fn note_applied_pairs(
    lexicon: &mut LexiconFile,
    confirmed: &[VocabularyEntry],
    pairs: &[(String, String)],
    day: u32,
) -> bool {
    let mut changed = false;
    for (heard, written) in pairs {
        if let Some(v) = confirmed.iter().find(|v| {
            v.enabled
                && v.written == *written
                && (v.heard.eq_ignore_ascii_case(heard)
                    || v.aliases.iter().any(|a| a.eq_ignore_ascii_case(heard)))
        }) {
            let counter = lexicon.usage.entry(v.id.clone()).or_default();
            counter.applied = counter.applied.saturating_add(1);
            counter.last_day = day;
            changed = true;
            continue;
        }
        if let Some(entry) = lexicon.entries.iter_mut().find(|e| {
            e.tier == Tier::Active
                && e.written == *written
                && e.heard
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(heard))
        }) {
            entry.applied = entry.applied.saturating_add(1);
            entry.probation_left = entry.probation_left.saturating_sub(1);
            entry.last_positive_day = day;
            changed = true;
        }
    }
    if changed {
        lexicon.revision = lexicon.revision.saturating_add(1);
    }
    changed
}

// --- Retraction, decay, caps, and migration ---

pub fn retract_source(lexicon: &mut LexiconFile, source: &SourceRef) -> bool {
    let before_len = lexicon.entries.len();
    let mut changed = false;
    lexicon.entries.retain_mut(|entry| {
        let orig = entry.sources.len();
        entry.sources.retain(|ev| &ev.source != source);
        if entry.sources.len() != orig {
            changed = true;
            entry.fixes = positive_fix_sources(&entry.sources) as u16;
            entry.keeps = entry
                .sources
                .iter()
                .filter(|s| s.kind == SourceKind::OverlayKeep && s.weight > 0)
                .count() as u16;
            entry.revision = entry.revision.saturating_add(1);
            recompute_entry_tier(entry, false);
        }
        !entry.sources.is_empty()
    });
    if changed || lexicon.entries.len() != before_len {
        lexicon.revision = lexicon.revision.saturating_add(1);
        true
    } else {
        false
    }
}

pub fn retract_all_dictations(lexicon: &mut LexiconFile) -> bool {
    let before_len = lexicon.entries.len();
    let mut changed = false;
    lexicon.entries.retain_mut(|entry| {
        let orig = entry.sources.len();
        entry.sources.retain(|ev| !ev.source.is_dictation());
        if entry.sources.len() != orig {
            changed = true;
            entry.fixes = positive_fix_sources(&entry.sources) as u16;
            entry.keeps = entry
                .sources
                .iter()
                .filter(|s| s.kind == SourceKind::OverlayKeep && s.weight > 0)
                .count() as u16;
            entry.revision = entry.revision.saturating_add(1);
            recompute_entry_tier(entry, false);
        }
        !entry.sources.is_empty()
    });
    if changed || lexicon.entries.len() != before_len {
        lexicon.revision = lexicon.revision.saturating_add(1);
        true
    } else {
        false
    }
}

pub fn decay_and_prune(lexicon: &mut LexiconFile, today: u32) {
    // 1. Expire old unconfirmed entries / demote stale active entries.
    lexicon.entries.retain_mut(|entry| {
        let age = today.saturating_sub(entry.last_positive_day);
        match entry.tier {
            Tier::Observed => age <= OBSERVED_EXPIRE_DAYS,
            Tier::Suggested => age <= SUGGESTED_EXPIRE_DAYS,
            Tier::Active => {
                if age > ACTIVE_DEMOTE_DAYS {
                    entry.tier = Tier::Suggested;
                }
                true
            }
        }
    });

    // 2. Enforce per-tier caps, evicting lowest-evidence oldest entries first.
    let enforce = |entries: &mut Vec<LexiconEntry>, tier: Tier, cap: usize| {
        let count = entries.iter().filter(|e| e.tier == tier).count();
        if count <= cap {
            return;
        }
        let mut tier_indices: Vec<(usize, u32, usize)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tier == tier)
            .map(|(i, e)| (i, e.last_positive_day, e.sources.len() + usize::from(e.fixes) * 3))
            .collect();
        // Sort worst-first: fewest sources/fixes, then oldest day.
        tier_indices.sort_by_key(|&(_, day, score)| (score, day));
        let drop_set: HashSet<usize> = tier_indices
            .into_iter()
            .take(count - cap)
            .map(|(i, _, _)| i)
            .collect();
        let mut idx = 0usize;
        entries.retain(|_| {
            let keep = !drop_set.contains(&idx);
            idx += 1;
            keep
        });
    };

    enforce(&mut lexicon.entries, Tier::Observed, MAX_OBSERVED);
    enforce(&mut lexicon.entries, Tier::Suggested, MAX_SUGGESTED);
    enforce(&mut lexicon.entries, Tier::Active, MAX_ACTIVE);
}

/// Migrate legacy `learning.json` entries into a fresh `LexiconFile`.
/// Returns `(lexicon, promoted_manual_vocabulary)` where independently authored
/// pinned manual lexical entries can be saved to `vocabulary.json`.
pub fn migrate_legacy_learning(
    entries: Vec<LearnedEntry>,
    today: u32,
) -> (LexiconFile, Vec<VocabularyEntry>) {
    let mut lexicon = LexiconFile::default();
    let mut manual_vocab = Vec::new();

    for entry in entries {
        match entry.kind {
            LearnedKind::Context | LearnedKind::Style => {
                lexicon.legacy.push(LegacyContextEntry {
                    id: entry.id,
                    kind: entry.kind.as_str().to_owned(),
                    text: entry.text,
                    scope: entry.scope,
                    enabled: entry.enabled,
                });
                continue;
            }
            LearnedKind::Correction | LearnedKind::Canonical => {}
        }

        // Independently authored pinned manual entry -> promote to VocabularyEntry.
        if entry.source_type == "manual" && entry.user_pinned && entry.source_ids.is_empty() {
            let written = entry.text.trim().to_owned();
            let heard = entry.heard.as_deref().unwrap_or(&written).trim().to_owned();
            if !written.is_empty() && !heard.is_empty() {
                let kind = if entry.kind == LearnedKind::Canonical && entry.heard.is_none() {
                    VocabularyKind::Hint
                } else {
                    VocabularyKind::Replace
                };
                manual_vocab.push(VocabularyEntry {
                    kind,
                    id: entry.id,
                    heard: if kind == VocabularyKind::Hint {
                        written.clone()
                    } else {
                        heard.clone()
                    },
                    written,
                    enabled: entry.enabled,
                    aliases: if kind == VocabularyKind::Hint {
                        Vec::new()
                    } else {
                        vec![heard]
                    },
                    context: Vec::new(),
                    scope: if kind == VocabularyKind::Hint {
                        None
                    } else {
                        entry.scope
                    },
                });
            }
            continue;
        }

        let written = entry.text.trim();
        let heard = entry.heard.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if written.is_empty() {
            continue;
        }

        // Disabled derived entries become local suppressions.
        if !entry.enabled {
            add_suppression(
                &mut lexicon,
                if heard.is_some() {
                    IgnoreTarget::Mapping
                } else {
                    IgnoreTarget::Term
                },
                heard,
                written,
                today,
            );
            continue;
        }

        // Enabled derived entries enter Review (Suggested) with surviving source IDs so
        // users can confirm or dismiss them explicitly.
        let sources: Vec<EvidenceRef> = entry
            .source_ids
            .into_iter()
            .take(MAX_SOURCES_PER_ENTRY)
            .map(|id| EvidenceRef {
                source: SourceRef::Dictation(id),
                kind: SourceKind::Migrated,
                day: today,
                weight: 3,
            })
            .collect();
        if sources.is_empty() {
            continue;
        }
        let risk = heard.map_or(HeardRisk::Nonword, classify_heard);
        let term_kind = match heard {
            Some(h) => determine_kind(h, written),
            None => TermKind::Hint,
        };
        let gate = match entry.scope.filter(|s| !s.trim().is_empty()) {
            Some(scope) => Gate::Topic(scope),
            None if risk == HeardRisk::Ordinary => Gate::Near(Vec::new()),
            None => Gate::Always,
        };
        lexicon.entries.push(LexiconEntry {
            id: entry.id,
            kind: term_kind,
            written: written.to_owned(),
            heard: heard.map(str::to_owned),
            gate,
            tier: Tier::Suggested,
            risk,
            fixes: sources.len() as u16,
            keeps: 0,
            applied: 0,
            reverted: 0,
            probation_left: 0,
            sources,
            near_cues: Vec::new(),
            promoted_to: None,
            revision: 1,
            last_positive_day: today,
            last_negative_day: 0,
            reason_hint: Some("migrated".into()),
        });
    }

    (lexicon, manual_vocab)
}

// --- Typed consumer planner ---

/// Build the exact consumer projections for one dictation or meeting passage.
/// Scoped Confirmed entries go only to `confirmed_structured` (unless they also
/// have explicit `context` words) so they never bypass their scope check.
pub fn plan(
    lexicon: &LexiconFile,
    confirmed: &[VocabularyEntry],
    raw_text: &str,
    mode: CleanupMode,
    learning_enabled: bool,
    meeting_id: Option<&str>,
) -> LexiconPlan {
    let mut out = LexiconPlan::default();
    let mut confirmed_written_all = Vec::new();
    let mut local_hint_pool = Vec::new();
    let mut seen_heard: HashSet<String> = HashSet::new();

    // 1. Confirmed entries from vocabulary.json (always active, regardless of learning toggle).
    for entry in confirmed.iter().filter(|e| e.enabled) {
        confirmed_written_all.push(entry.written.clone());
        local_hint_pool.push(entry.written.clone());
        out.trust_confirmed_words.push(entry.written.clone());

        let has_scope = entry
            .scope
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());

        if has_scope && entry.kind != VocabularyKind::Hint {
            let scope = entry.scope.as_deref().unwrap().trim().to_owned();
            let mut aliases = vec![entry.heard.clone()];
            for a in &entry.aliases {
                if !aliases.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                    aliases.push(a.clone());
                }
            }
            out.confirmed_structured
                .push(crate::structured_clean::VocabularyEntry {
                    id: Some(entry.id.clone()),
                    canonical: entry.written.clone(),
                    aliases,
                    scope,
                    status: crate::structured_clean::VocabularyStatus::Confirmed,
                    provenance:
                        crate::structured_clean::VocabularyProvenance::ExplicitUserConfirmation,
                });
            // Only also expose as a hint for casing/proofreader, NOT as an unscoped alias,
            // unless the user explicitly supplied nearby context words.
            if !entry.context.is_empty() {
                out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                    kind: entry.kind,
                    aliases: entry.aliases.clone(),
                    heard: entry.heard.clone(),
                    written: entry.written.clone(),
                    enabled: true,
                    context: entry.context.clone(),
                });
            } else {
                out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                    kind: VocabularyKind::Hint,
                    aliases: Vec::new(),
                    heard: entry.written.clone(),
                    written: entry.written.clone(),
                    enabled: true,
                    context: Vec::new(),
                });
            }
        } else {
            if entry.kind != VocabularyKind::Hint {
                seen_heard.insert(entry.heard.to_lowercase());
                for a in &entry.aliases {
                    seen_heard.insert(a.to_lowercase());
                }
            }
            out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                kind: entry.kind,
                aliases: entry.aliases.clone(),
                heard: entry.heard.clone(),
                written: entry.written.clone(),
                enabled: true,
                context: entry.context.clone(),
            });
        }
    }

    // 2. Local lexicon entries (only when learning_enabled is true).
    if learning_enabled {
        let mut active_ranked: Vec<&LexiconEntry> = lexicon
            .entries
            .iter()
            .filter(|e| e.tier == Tier::Active && gate_allows_surface(&e.gate, meeting_id))
            .collect();
        active_ranked.sort_by(|a, b| {
            b.fixes
                .cmp(&a.fixes)
                .then_with(|| b.sources.len().cmp(&a.sources.len()))
                .then_with(|| b.last_positive_day.cmp(&a.last_positive_day))
        });

        let mut added_cleanup = 0usize;
        let mut used_chars = 0usize;

        for entry in active_ranked {
            out.trust_active_hints.push(entry.written.clone());
            local_hint_pool.push(entry.written.clone());

            match entry.kind {
                TermKind::Hint => {
                    // Hints protect exact spellings in cleanup & feed local cleanup hints only.
                    if added_cleanup < MAX_CLEANUP_LEARNED_ENTRIES
                        && used_chars + entry.written.len() + 16 <= CLEANUP_CHAR_BUDGET
                    {
                        used_chars += entry.written.len() + 16;
                        added_cleanup += 1;
                        out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                            kind: VocabularyKind::Hint,
                            aliases: Vec::new(),
                            heard: entry.written.clone(),
                            written: entry.written.clone(),
                            enabled: true,
                            context: Vec::new(),
                        });
                    }
                }
                TermKind::Spelling => {
                    // Casing-only spelling fixes apply in Format and Clean (never Original).
                    if matches!(mode, CleanupMode::Clean | CleanupMode::Format) {
                        if let Some(heard) = entry.heard.as_deref() {
                            if !seen_heard.contains(&heard.to_lowercase())
                                && added_cleanup < MAX_CLEANUP_LEARNED_ENTRIES
                                && used_chars + heard.len() + entry.written.len() + 16
                                    <= CLEANUP_CHAR_BUDGET
                            {
                                seen_heard.insert(heard.to_lowercase());
                                used_chars += heard.len() + entry.written.len() + 16;
                                added_cleanup += 1;
                                out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                                    kind: VocabularyKind::Replace,
                                    aliases: vec![heard.to_owned()],
                                    heard: heard.to_owned(),
                                    written: entry.written.clone(),
                                    enabled: true,
                                    context: Vec::new(),
                                });
                            }
                        }
                    }
                }
                TermKind::Alias => {
                    // Auto-active aliases ONLY for Nonword heard forms, in Clean mode only.
                    if mode == CleanupMode::Clean && entry.risk == HeardRisk::Nonword {
                        if let Some(heard) = entry.heard.as_deref() {
                            if !seen_heard.contains(&heard.to_lowercase())
                                && !has_conflict(lexicon, heard, &entry.written, &entry.id)
                                && added_cleanup < MAX_CLEANUP_LEARNED_ENTRIES
                                && used_chars + heard.len() + entry.written.len() + 16
                                    <= CLEANUP_CHAR_BUDGET
                            {
                                seen_heard.insert(heard.to_lowercase());
                                used_chars += heard.len() + entry.written.len() + 16;
                                added_cleanup += 1;
                                out.cleanup_vocabulary.push(crate::cleanup::Vocabulary {
                                    kind: VocabularyKind::Replace,
                                    aliases: vec![heard.to_owned()],
                                    heard: heard.to_owned(),
                                    written: entry.written.clone(),
                                    enabled: true,
                                    context: Vec::new(),
                                });
                                if entry.probation_left > 0 {
                                    out.probation_pairs
                                        .push((heard.to_owned(), entry.written.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Suggested pairs can rank as Doubtful Words alternatives on already-doubted words.
        for entry in lexicon
            .entries
            .iter()
            .filter(|e| e.tier == Tier::Suggested && gate_allows_surface(&e.gate, meeting_id))
        {
            if let Some(heard) = entry.heard.as_deref() {
                out.trust_suggested_pairs
                    .push((heard.to_owned(), entry.written.clone()));
            }
        }
    }

    // 3. Relevance-filtered proofreader lists (online = Confirmed ONLY; local = Confirmed + Active).
    out.online_proofreader_hints =
        relevance_filter_hints(&confirmed_written_all, raw_text, MAX_ONLINE_PROOFREADER_HINTS);
    out.local_proofreader_hints =
        relevance_filter_hints(&local_hint_pool, raw_text, MAX_LOCAL_PROOFREADER_HINTS);

    dedup_preserve(&mut out.trust_confirmed_words);
    dedup_preserve(&mut out.trust_active_hints);
    out
}

fn gate_allows_surface(gate: &Gate, meeting_id: Option<&str>) -> bool {
    match gate {
        Gate::Meeting(id) => meeting_id.is_some_and(|m| m == id),
        _ => true,
    }
}

fn dedup_preserve(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.to_lowercase()));
}

/// Filter vocabulary hints to terms that match or sound close to a token in
/// `utterance`, capped at `max_count`. When `utterance` is empty, returns up
/// to `max_count` unique terms.
pub fn relevance_filter_hints(
    candidates: &[String],
    utterance: &str,
    max_count: usize,
) -> Vec<String> {
    if candidates.is_empty() || max_count == 0 {
        return Vec::new();
    }
    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for c in candidates {
        let trimmed = c.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_lowercase()) {
            unique.push(trimmed.to_owned());
        }
    }
    if unique.len() <= max_count {
        return unique;
    }
    let tokens: Vec<String> = tokenize_words(utterance)
        .into_iter()
        .map(normalize_key)
        .filter(|t| t.len() >= 2)
        .collect();
    if tokens.is_empty() {
        unique.truncate(max_count);
        return unique;
    }
    let token_sounds: Vec<String> = tokens.iter().map(|t| sound_key(t)).collect();
    let mut scored: Vec<(u8, usize, String)> = Vec::new();

    for cand in unique {
        let ck = normalize_key(&cand);
        if ck.len() < 2 {
            continue;
        }
        let cs = sound_key(&ck);
        let mut best_rank: Option<(u8, usize)> = None;

        // Check single tokens and adjacent bigrams.
        for (i, tok) in tokens.iter().enumerate() {
            if *tok == ck {
                best_rank = Some((0, 0));
                break;
            }
            let spell = levenshtein(tok, &ck);
            let longest = tok.len().max(ck.len());
            if spell * 3 <= longest {
                let candidate_rank = (1, spell);
                if best_rank.is_none_or(|b| candidate_rank < b) {
                    best_rank = Some(candidate_rank);
                }
            } else {
                let sd = levenshtein(&token_sounds[i], &cs);
                let slen = token_sounds[i].len().max(cs.len()).max(1);
                if sd * 2 <= slen && spell * 2 <= longest {
                    let candidate_rank = (2, spell);
                    if best_rank.is_none_or(|b| candidate_rank < b) {
                        best_rank = Some(candidate_rank);
                    }
                }
            }
            // Adjacent bigram check for joined names like "sono jot" vs "SonoJot".
            if i + 1 < tokens.len() {
                let joined = format!("{}{}", tok, tokens[i + 1]);
                let bspell = levenshtein(&joined, &ck);
                let blong = joined.len().max(ck.len());
                if bspell * 3 <= blong {
                    let candidate_rank = (1, bspell);
                    if best_rank.is_none_or(|b| candidate_rank < b) {
                        best_rank = Some(candidate_rank);
                    }
                }
            }
        }

        if let Some((tier, dist)) = best_rank {
            scored.push((tier, dist, cand));
        }
    }

    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)).then_with(|| a.2.cmp(&b.2)));
    scored
        .into_iter()
        .take(max_count)
        .map(|(_, _, c)| c)
        .collect()
}

// --- Phonetic spelling clusters for Review ---

/// Group ≥2 near-sounding passive Hint entries across ≥3 total sources into
/// one "Heard a few ways" cluster card for Review.
pub fn build_clusters(
    lexicon: &LexiconFile,
    confirmed: &[VocabularyEntry],
) -> Vec<SpellingClusterView> {
    let confirmed_sounds: HashSet<String> = confirmed
        .iter()
        .filter(|v| v.enabled)
        .map(|v| sound_key(&v.written))
        .collect();

    let mut groups: BTreeMap<String, Vec<&LexiconEntry>> = BTreeMap::new();
    for entry in lexicon.entries.iter().filter(|e| {
        e.kind == TermKind::Hint
            && matches!(e.tier, Tier::Observed | Tier::Suggested)
            && e.heard.is_none()
            && !is_suppressed(lexicon, None, &e.written)
    }) {
        let sk = sound_key(&entry.written);
        if sk.len() < 3 || confirmed_sounds.contains(&sk) {
            continue;
        }
        groups.entry(sk).or_default().push(entry);
    }

    let mut clusters = Vec::new();
    for (sk, group) in groups {
        let mut variants: Vec<(String, usize)> = Vec::new();
        let mut unique_sources: HashSet<&SourceRef> = HashSet::new();
        for entry in &group {
            for s in &entry.sources {
                unique_sources.insert(&s.source);
            }
            if !variants
                .iter()
                .any(|(w, _)| w.eq_ignore_ascii_case(&entry.written))
            {
                variants.push((entry.written.clone(), entry.sources.len()));
            }
        }
        if variants.len() >= 2 && unique_sources.len() >= 3 {
            variants.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let total_sources = unique_sources.len();
            clusters.push(SpellingClusterView {
                key: sk,
                variants: variants.into_iter().take(4).map(|(w, _)| w).collect(),
                total_sources,
                reason: format!("Heard a few ways across {total_sources} recordings"),
            });
        }
    }
    clusters.sort_by(|a, b| b.total_sources.cmp(&a.total_sources));
    clusters.truncate(8);
    clusters
}

/// Build the full `WordsOverview` read model for the Words UI.
pub fn build_overview<F>(
    lexicon: &LexiconFile,
    words: &[VocabularyEntry],
    resolve_source: F,
) -> WordsOverview
where
    F: Fn(&SourceRef) -> Option<SourceSummary>,
{
    let clusters = build_clusters(lexicon, words);
    let clustered_words: HashSet<String> = clusters
        .iter()
        .flat_map(|c| c.variants.iter().map(|v| v.to_lowercase()))
        .collect();

    let to_view = |entry: &LexiconEntry| -> LearnedItemView {
        let sources: Vec<SourceSummary> = entry
            .sources
            .iter()
            .filter_map(|s| resolve_source(&s.source))
            .take(5)
            .collect();
        let reason = human_reason(entry, &sources);
        LearnedItemView {
            id: entry.id.clone(),
            kind: entry.kind,
            heard: entry.heard.clone(),
            written: entry.written.clone(),
            tier: entry.tier,
            risk: entry.risk,
            near_cues: entry.near_cues.iter().map(|(w, _)| w.clone()).collect(),
            fixes: entry.fixes,
            keeps: entry.keeps,
            applied: entry.applied,
            reverted: entry.reverted,
            probation_left: entry.probation_left,
            reason,
            sources,
            revision: entry.revision,
            promoted_to: entry.promoted_to.clone(),
        }
    };

    let mut word_meta: HashMap<String, ConfirmedWordMeta> = HashMap::new();
    for word in words {
        let usage = lexicon.usage.get(&word.id).cloned().unwrap_or_default();
        let inert_legacy = word.kind == VocabularyKind::Replace
            && word.aliases.is_empty()
            && !word.heard.eq_ignore_ascii_case(&word.written)
            && word.context.is_empty()
            && word.scope.is_none();
        let extra_aliases = lexicon
            .entries
            .iter()
            .filter(|e| e.promoted_to.as_deref() == Some(&word.id) && e.tier != Tier::Observed)
            .map(to_view)
            .collect();
        word_meta.insert(
            word.id.clone(),
            ConfirmedWordMeta {
                id: word.id.clone(),
                applied: usage.applied,
                last_day: usage.last_day,
                needs_check: usage.reverted > 0,
                inert_legacy,
                extra_aliases,
            },
        );
    }

    let mut active_learned: Vec<LearnedItemView> = lexicon
        .entries
        .iter()
        .filter(|e| e.tier == Tier::Active && e.promoted_to.is_none())
        .map(to_view)
        .collect();
    active_learned.sort_by(|a, b| b.fixes.cmp(&a.fixes).then_with(|| a.written.cmp(&b.written)));

    let mut review_items: Vec<LearnedItemView> = lexicon
        .entries
        .iter()
        .filter(|e| {
            e.tier == Tier::Suggested
                && !(e.kind == TermKind::Hint
                    && clustered_words.contains(&e.written.to_lowercase()))
        })
        .map(to_view)
        .collect();
    review_items.sort_by(|a, b| {
        b.fixes
            .cmp(&a.fixes)
            .then_with(|| b.sources.len().cmp(&a.sources.len()))
            .then_with(|| a.written.cmp(&b.written))
    });

    WordsOverview {
        words: words.to_vec(),
        word_meta,
        active_learned,
        review_items,
        clusters,
        suppressed_count: lexicon.suppressions.len(),
        lexicon_revision: lexicon.revision,
    }
}

fn human_reason(entry: &LexiconEntry, sources: &[SourceSummary]) -> String {
    if entry.reason_hint.as_deref() == Some("speaker_name") {
        return "Speaker name from a meeting".into();
    }
    if entry.reason_hint.as_deref() == Some("review_proposal") {
        return "Found in recent dictations".into();
    }
    if entry.reason_hint.as_deref() == Some("migrated") {
        return "Carried over from earlier learning".into();
    }
    if entry.fixes >= 2 {
        let dates: Vec<&str> = sources
            .iter()
            .map(|s| s.date.as_str())
            .filter(|d| !d.is_empty())
            .take(2)
            .collect();
        return if dates.is_empty() {
            format!("You fixed this {} times", entry.fixes)
        } else {
            format!("You fixed this {} times: {}", entry.fixes, dates.join(", "))
        };
    }
    if entry.fixes == 1 {
        let extra = if entry.risk == HeardRisk::Ordinary {
            " · common word, needs nearby context"
        } else {
            ""
        };
        return if let Some(first) = sources.first() {
            format!("Fixed once on {}{extra}", first.date)
        } else {
            format!("Fixed once{extra}")
        };
    }
    if entry.keeps >= 2 {
        return format!("Kept {} times when doubted", entry.keeps);
    }
    let count = distinct_source_count(&entry.sources).max(1);
    format!("Used in {count} recordings")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmed_word(id: &str, heard: &str, written: &str, aliases: &[&str]) -> VocabularyEntry {
        VocabularyEntry {
            kind: VocabularyKind::Replace,
            id: id.into(),
            heard: heard.into(),
            written: written.into(),
            enabled: true,
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            context: Vec::new(),
            scope: None,
        }
    }

    #[test]
    fn guardrails_reject_protected_numbers_and_ordinary_grammar_swaps() {
        assert_eq!(classify_heard("four"), HeardRisk::Protected);
        assert_eq!(classify_heard("cannot"), HeardRisk::Protected);
        assert_eq!(classify_heard("left"), HeardRisk::Protected);
        assert_eq!(classify_heard("monday"), HeardRisk::Protected);
        assert_eq!(classify_heard("v2"), HeardRisk::Protected);
        assert_eq!(classify_heard("tax sheet"), HeardRisk::Ordinary);
        assert_eq!(classify_heard("jason"), HeardRisk::Ordinary);
        assert_eq!(classify_heard("velmore"), HeardRisk::Nonword);
        assert_eq!(classify_heard("sono jot"), HeardRisk::Nonword);
        assert!(is_ordinary_grammar_swap("affect", "effect"));
        assert!(!is_ordinary_grammar_swap("cash", "CacheFlow"));
        assert!(!is_ordinary_grammar_swap("sonojot", "SonoJot"));
    }

    #[test]
    fn nonword_alias_activates_after_two_fixes_with_probation_and_reverts_cleanly() {
        let mut lex = LexiconFile::default();
        let d1 = SourceRef::Dictation("d1".into());
        let d2 = SourceRef::Dictation("d2".into());

        // 1st fix -> Suggested
        let o1 = record_fix(&mut lex, &[], d1.clone(), SourceKind::OverlayChoice, "velmore", "Velmora", &[], 100);
        assert_eq!(o1, FixOutcome::Suggested);
        assert_eq!(lex.entries.len(), 1);
        assert_eq!(lex.entries[0].tier, Tier::Suggested);

        // Same dictation again -> idempotent, still Suggested
        let o1b = record_fix(&mut lex, &[], d1.clone(), SourceKind::HistoryEdit, "velmore", "Velmora", &[], 100);
        assert_eq!(o1b, FixOutcome::EvidenceAdded);
        assert_eq!(lex.entries[0].tier, Tier::Suggested);

        // 2nd distinct dictation -> Active with 3 probation uses!
        let o2 = record_fix(&mut lex, &[], d2.clone(), SourceKind::OverlayChoice, "velmore", "Velmora", &[], 101);
        assert_eq!(o2, FixOutcome::Activated);
        assert_eq!(lex.entries[0].tier, Tier::Active);
        assert_eq!(lex.entries[0].probation_left, PROBATION_USES);

        // Plan exposes it in Clean (with probation) and NOT in Format/Original
        let clean = plan(&lex, &[], "open velmore now", CleanupMode::Clean, true, None);
        assert!(clean.cleanup_vocabulary.iter().any(|v| v.written == "Velmora"));
        assert_eq!(clean.probation_pairs, vec![("velmore".into(), "Velmora".into())]);
        let orig = plan(&lex, &[], "open velmore now", CleanupMode::Original, true, None);
        assert!(orig.cleanup_vocabulary.iter().all(|v| v.written != "Velmora" || v.kind == VocabularyKind::Hint));
        assert!(orig.probation_pairs.is_empty());

        // 1st revert -> demotes back to Suggested
        assert!(record_revert(&mut lex, &[], "velmore", "Velmora", 102));
        assert_eq!(lex.entries[0].tier, Tier::Suggested);

        // 2nd revert -> deletes entry and writes device-keyed HMAC suppression
        assert!(record_revert(&mut lex, &[], "velmore", "Velmora", 103));
        assert!(lex.entries.is_empty());
        assert!(is_suppressed(&lex, Some("velmore"), "Velmora"));

        // Suppressed pair cannot be re-learned
        let d3 = SourceRef::Dictation("d3".into());
        assert_eq!(
            record_fix(&mut lex, &[], d3, SourceKind::OverlayChoice, "velmore", "Velmora", &[], 104),
            FixOutcome::Ignored
        );
    }

    #[test]
    fn ordinary_word_alias_never_auto_activates_and_harvests_near_cues() {
        let mut lex = LexiconFile::default();
        for i in 1..=3 {
            let src = SourceRef::Dictation(format!("d{i}"));
            let _ = record_fix(
                &mut lex,
                &[],
                src,
                SourceKind::HistoryEdit,
                "tax sheet",
                "Tacksheet",
                &["publish".into(), "phone".into()],
                100 + i,
            );
        }
        assert_eq!(lex.entries.len(), 1);
        assert_eq!(lex.entries[0].risk, HeardRisk::Ordinary);
        assert_eq!(lex.entries[0].tier, Tier::Suggested);
        assert!(!lex.entries[0].near_cues.is_empty());

        let clean = plan(&lex, &[], "send the tax sheet", CleanupMode::Clean, true, None);
        assert!(clean.cleanup_vocabulary.is_empty());
        assert_eq!(clean.trust_suggested_pairs, vec![("tax sheet".into(), "Tacksheet".into())]);
    }

    #[test]
    fn passive_observation_promotes_after_three_sources_two_days_and_clusters_variants() {
        let mut lex = LexiconFile::default();
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("d1".into()), "We shipped SonoJot yesterday.", 100);
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("d2".into()), "Open SonoJot settings.", 100);
        // Only 1 day so far -> still Observed
        let sj = lex.entries.iter().find(|e| e.written == "SonoJot").unwrap();
        assert_eq!(sj.tier, Tier::Observed);

        // 3rd source on day 2 -> Suggested Hint!
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("d3".into()), "Update SonoJot now.", 101);
        let sj = lex.entries.iter().find(|e| e.written == "SonoJot").unwrap();
        assert_eq!(sj.tier, Tier::Suggested);

        // Meeting repetition counts as only 1 source!
        observe_passive_text(
            &mut lex,
            &[],
            SourceRef::Meeting("m1".into()),
            "Kestrel Kestrel Kestrel Kestrel Kestrel",
            101,
        );
        let kes = lex.entries.iter().find(|e| e.written == "Kestrel").unwrap();
        assert_eq!(kes.sources.len(), 1);
        assert_eq!(kes.tier, Tier::Observed);

        // Near-sounding variants across 3 sources cluster into one review card
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("c1".into()), "Check Velmora today.", 101);
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("c2".into()), "Check Velmore today.", 102);
        observe_passive_text(&mut lex, &[], SourceRef::Dictation("c3".into()), "Check Velmora again.", 103);
        let clusters = build_clusters(&lex, &[]);
        assert!(clusters.iter().any(|c| c.variants.contains(&"Velmora".to_string()) && c.variants.contains(&"Velmore".to_string())));
    }

    #[test]
    fn online_proofreader_never_receives_unconfirmed_local_entries() {
        let mut lex = LexiconFile::default();
        record_speaker_name(&mut lex, &[], "m1", "Marguerite", 100);
        let confirmed = vec![confirmed_word("v1", "kuber", "Kubernetes", &["kuber"])];
        let p = plan(&lex, &confirmed, "Ask Marguerite about kuber", CleanupMode::Clean, true, Some("m1"));
        assert!(p.local_proofreader_hints.iter().any(|w| w == "Marguerite"));
        assert!(!p.online_proofreader_hints.iter().any(|w| w == "Marguerite"));
        assert!(p.online_proofreader_hints.iter().any(|w| w == "Kubernetes"));
    }

    #[test]
    fn retraction_removes_unsupported_entries_while_preserving_hmac_suppressions() {
        let mut lex = LexiconFile::default();
        let d1 = SourceRef::Dictation("d1".into());
        record_fix(&mut lex, &[], d1.clone(), SourceKind::HistoryEdit, "velmore", "Velmora", &[], 100);
        add_suppression(&mut lex, IgnoreTarget::Term, None, "SecretName", 100);
        assert_eq!(lex.entries.len(), 1);

        assert!(retract_source(&mut lex, &d1));
        assert!(lex.entries.is_empty());
        assert!(is_suppressed(&lex, None, "SecretName"));
    }

    #[test]
    fn scoped_confirmed_entry_does_not_leak_as_unscoped_alias() {
        let lex = LexiconFile::default();
        let scoped = VocabularyEntry {
            kind: VocabularyKind::Replace,
            id: "s1".into(),
            heard: "quinn".into(),
            written: "Qwen".into(),
            enabled: true,
            aliases: vec!["quinn".into()],
            context: Vec::new(),
            scope: Some("AI model discussion".into()),
        };
        let p = plan(&lex, &[scoped], "Ask quinn about dinner", CleanupMode::Clean, true, None);
        assert_eq!(p.confirmed_structured.len(), 1);
        // Should only be a Hint in cleanup_vocabulary, never an unscoped Replace alias!
        assert!(p.cleanup_vocabulary.iter().all(|v| v.kind == VocabularyKind::Hint));
    }
}
