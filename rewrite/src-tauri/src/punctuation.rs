//! Word-preserving punctuation for dictation. The caller owns publication and
//! cancellation. Inference runs in an owned helper; rebuilding stays here.

use crate::{cleanup::Vocabulary, domain::ModelRetention};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};

pub struct Outcome {
    pub text: String,
    pub stage: &'static str,
    pub elapsed_ms: f64,
}

/// Production uses routed model spans when available. Evaluation can freeze
/// the rules baseline without disabling punctuation inference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrittenFormMode {
    Off,
    Rules,
    Model,
}

impl WrittenFormMode {
    pub fn as_str(self) -> &'static str {
        match self { Self::Off => "off", Self::Rules => "rules", Self::Model => "model" }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Prediction {
    pub labels: Vec<Mark>,
    pub spans: Option<Vec<crate::written_form::BioLabel>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mark {
    None,
    Period,
    Comma,
    Question,
    Semicolon,
}

impl Mark {
    fn text(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Period => ".",
            Self::Comma => ",",
            Self::Question => "?",
            Self::Semicolon => ";",
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Period | Self::Question)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) enum Failure {
    Cancelled,
    Missing,
    Config,
    Tokenizer,
    Runtime,
    Load,
    Inference,
    Output,
    Invariant,
    Busy,
    Unsupported,
}

impl Failure {
    fn event(self) -> &'static str {
        match self {
            Self::Cancelled => "punctuation-fallback-cancelled",
            Self::Missing => "punctuation-fallback-missing",
            Self::Config => "punctuation-fallback-config",
            Self::Tokenizer => "punctuation-fallback-tokenizer",
            Self::Runtime => "punctuation-fallback-runtime",
            Self::Load => "punctuation-fallback-load",
            Self::Inference => "punctuation-fallback-inference",
            Self::Output => "punctuation-fallback-output",
            Self::Invariant => "punctuation-fallback-invariant",
            Self::Busy => "punctuation-fallback-busy",
            Self::Unsupported => "punctuation-fallback-unsupported",
        }
    }
}

pub(crate) type Result<T> = std::result::Result<T, Failure>;

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Failure::Cancelled)
    } else {
        Ok(())
    }
}

fn event(kind: &'static str, ms: f64) {
    crate::recognition_diagnostics::event(
        kind,
        crate::punctuation_model::ID,
        std::process::id(),
        0,
        ms.round() as u64,
        0,
    );
}

/// Includes the deterministic house-style finish. `stage` is suitable for the
/// parent's publication evidence; elapsed time includes loading and fallback.
/// A cancelled call returns the input unchanged; the caller must still check its
/// cancellation flag before publication, just as for the other cleanup stages.
pub fn run(root: &Path, text: &str, vocabulary: &[Vocabulary], cancel: &AtomicBool) -> Outcome {
    run_with_written_form(root, text, vocabulary, cancel, false)
}

pub fn run_with_written_form(
    root: &Path, text: &str, vocabulary: &[Vocabulary], cancel: &AtomicBool, written_form: bool,
) -> Outcome {
    run_with_written_form_mode(root, text, vocabulary, cancel,
        if written_form { WrittenFormMode::Model } else { WrittenFormMode::Off })
}

pub fn run_with_written_form_mode(
    root: &Path, text: &str, vocabulary: &[Vocabulary], cancel: &AtomicBool, mode: WrittenFormMode,
) -> Outcome {
    let written_form = mode != WrittenFormMode::Off;
    let started = Instant::now();
    let selection = Selection::new(
        root,
        std::env::var_os("SONOJOT_PUNCT_MODEL_DIR").map(PathBuf::from),
    );
    event(
        if selection.overridden {
            "punctuation-source-override"
        } else {
            "punctuation-source-installed"
        },
        0.0,
    );
    // Capture guards pin a prewarmed helper until this job finishes.
    let retention = crate::resource_manager::current_retention();
    let document = Document::new(text, vocabulary, cancel);
    let prediction = match &document {
        Ok(document) if !document.words.is_empty() => {
            crate::punctuation_worker::predict(&selection, text, document.words.len(), cancel)
        }
        Ok(_) => Err(Failure::Missing),
        Err(error) => Err(*error),
    };
    if retention == ModelRetention::Immediately {
        release_idle(retention);
    }
    let (marks, spans) = match prediction {
        Ok(prediction) => (Ok(prediction.labels), prediction.spans),
        Err(error) => (Err(error), None),
    };
    let (mut output, stage, reason) = finish_inner(
        text, document.as_ref().ok(), marks, cancel, !written_form,
    );
    if written_form && check(cancel).is_ok() {
        let labels = if mode == WrittenFormMode::Model {
            spans.as_deref().map(|labels| project_span_labels(&output, labels)
                .unwrap_or_default())
        } else { None };
        output = written_form_finish(&output, vocabulary, labels.as_deref(), cancel);
    }
    if check(cancel).is_err() {
        output = text.into();
    }
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    if let Some(reason) = reason {
        event(reason.event(), elapsed_ms);
    }
    event(stage, elapsed_ms);
    Outcome {
        text: output,
        stage,
        elapsed_ms,
    }
}

/// Per-word predictions through exactly the production worker route. No rebuild
/// or fallback masks a failed model when checking parity with the Python scorer.
#[cfg(feature = "fidelity-eval")]
pub fn evaluation_labels(
    root: &Path,
    text: &str,
    cancel: &AtomicBool,
) -> std::result::Result<Vec<String>, String> {
    evaluation_prediction(root, text, cancel).map(|prediction| prediction.0)
}

/// Both heads from one worker request, in regex-word order.
#[cfg(feature = "fidelity-eval")]
pub fn evaluation_prediction(
    root: &Path,
    text: &str,
    cancel: &AtomicBool,
) -> std::result::Result<(Vec<String>, Option<Vec<crate::written_form::BioLabel>>), String> {
    let selection = evaluation_selection(root, cancel)?;
    let count = word_regex().find_iter(text).count();
    let result = crate::punctuation_worker::predict(&selection, text, count, cancel)
        .map(|prediction| {
            (prediction.labels
                .into_iter()
                .map(|mark| if mark == Mark::None { "0" } else { mark.text() }.to_owned())
                .collect(), prediction.spans)
        })
        .map_err(|failure| failure.event().to_owned());
    release_idle(crate::resource_manager::current_retention());
    result
}

/// Replay keeps punctuation fixed while using production BIO projection.
#[cfg(feature = "fidelity-eval")]
pub fn evaluation_written_form_labels(
    root: &Path, text: &str, cancel: &AtomicBool,
) -> std::result::Result<Option<Vec<crate::written_form::BioLabel>>, String> {
    let (_, spans) = evaluation_prediction(root, text, cancel)?;
    spans.map(|labels| project_span_labels(text, &labels)
        .ok_or_else(|| "Written-form label alignment failed.".to_owned())).transpose()
}

pub use crate::punctuation_worker::{dispatch_worker, prewarm, Prewarm, RuntimeEvidence};

/// Wait for an asynchronous capture prewarm, or cold-load a helper for evaluation.
/// Keep a `Prewarm` guard alive if Immediate retention must preserve the session.
#[cfg(feature = "fidelity-eval")]
pub fn evaluation_wait_ready(
    root: &Path,
    cancel: &AtomicBool,
) -> std::result::Result<RuntimeEvidence, String> {
    let selection = evaluation_selection(root, cancel)?;
    crate::punctuation_worker::wait_ready(&selection, cancel)
        .map_err(|failure| failure.event().to_owned())
}

#[cfg(feature = "fidelity-eval")]
fn evaluation_selection(root: &Path, cancel: &AtomicBool) -> std::result::Result<Selection, String> {
    check(cancel).map_err(|failure| failure.event().to_owned())?;
    let directory = std::env::var_os("SONOJOT_PUNCT_MODEL_DIR").map(PathBuf::from);
    // Standalone replay has no application inventory loop to finish background
    // checks. Verify cold manifests before selecting, without using an override.
    if directory.is_none()
        && crate::punctuation_model::selected_path(root, crate::model_verification::Check::Blocking).is_none()
    {
        return Err(Failure::Missing.event().to_owned());
    }
    check(cancel).map_err(|failure| failure.event().to_owned())?;
    Ok(Selection::new(root, directory))
}

#[cfg(test)]
fn finish(
    text: &str,
    document: Option<&Document<'_>>,
    prediction: Result<Vec<Mark>>,
    cancel: &AtomicBool,
) -> (String, &'static str, Option<Failure>) {
    finish_inner(text, document, prediction, cancel, true)
}

fn finish_inner(
    text: &str,
    document: Option<&Document<'_>>,
    prediction: Result<Vec<Mark>>,
    cancel: &AtomicBool,
    house_style: bool,
) -> (String, &'static str, Option<Failure>) {
    if check(cancel).is_err() {
        return (text.into(), "punctuation_rules", Some(Failure::Cancelled));
    }
    let Some(document) = document else {
        return (
            text.into(),
            "punctuation_rules",
            Some(prediction.err().unwrap_or(Failure::Output)),
        );
    };
    let proposed = prediction.and_then(|labels| document.rebuild_with_case(Some(&labels), cancel, house_style));
    let (mut output, mut stage, mut reason) = match proposed {
        Ok(output) if same_words(text, &output) => (output, "punctuation_model", None),
        result => {
            let reason = result.err().unwrap_or(Failure::Invariant);
            let rules = document
                .rebuild_with_case(None, cancel, house_style)
                .unwrap_or_else(|_| text.into());
            (rules, "punctuation_rules", Some(reason))
        }
    };
    if !same_words(text, &output) {
        output = text.into();
        stage = "punctuation_rules";
        reason = Some(Failure::Invariant);
    }
    if check(cancel).is_err() {
        output = text.into();
        stage = "punctuation_rules";
        reason = Some(Failure::Cancelled);
    }
    // Assert the published candidate, after the recoverable invariant guard.
    debug_assert!(same_words(text, &output));
    (output, stage, reason)
}

/// The punctuation tokenizer splits decimal components; the renderer keeps
/// whitespace-delimited tokens. Reject conflicting labels inside one token.
fn project_span_labels(text: &str, labels: &[crate::written_form::BioLabel]) -> Option<Vec<crate::written_form::BioLabel>> {
    use crate::written_form::BioLabel;
    let mut offset = 0;
    let mut result = Vec::new();
    for token in text.split_whitespace() {
        let count = word_regex().find_iter(token).count();
        if count == 0 {
            result.push(BioLabel::O);
            continue;
        }
        let parts = labels.get(offset..offset + count)?;
        let first = *parts.first()?;
        let compatible = parts.iter().skip(1).all(|part| match (&first, part) {
            (BioLabel::O, BioLabel::O) => true,
            (BioLabel::B(class) | BioLabel::I(class), BioLabel::I(other)) => class == other,
            _ => false,
        });
        // Match wf_model.render_text: an incompatible compound suppresses only
        // this token; it must not erase unrelated model evidence in the row.
        result.push(if compatible { first } else { BioLabel::O });
        offset += count;
    }
    (offset == labels.len()).then_some(result)
}

fn written_form_finish(
    text: &str,
    vocabulary: &[Vocabulary],
    labels: Option<&[crate::written_form::BioLabel]>,
    cancel: &AtomicBool,
) -> String {
    use crate::written_form;
    let Ok(matchers) = vocabulary_matchers(vocabulary, cancel) else { return text.into(); };
    let mut protected: Vec<_> = matchers.iter().flat_map(|matcher| matcher.find_iter(text))
        .map(|found| (found.start(), found.end())).collect();
    let Ok(incomplete) = crate::structured_clean::preserved_candidate_spans(text, &|| cancel.load(Ordering::Acquire)) else {
        return text.into();
    };
    protected.extend(incomplete);
    let words: Vec<_> = text.split_whitespace().collect();
    let mut search = 0;
    let bounds: Vec<_> = words.iter().map(|word| {
        let start = search + text[search..].find(*word).unwrap_or(0);
        search = start + word.len();
        (start, search)
    }).collect();
    let mut guarded = Vec::new();
    for span in written_form::spans(&words, labels) {
        if check(cancel).is_err() { return text.into(); }
        if span.start >= span.end || span.end > words.len() { continue; }
        let start = bounds[span.start].0;
        let end = bounds[span.end - 1].1;
        if protected.iter().any(|&(left, right)| start < right && end > left) { continue; }
        guarded.push(span);
    }
    let converted = written_form::apply_spans(text, &guarded);
    if check(cancel).is_err() { return text.into(); }
    // Reuse the existing sentence-case finish without proposing punctuation
    // again. Its protected-literal logic also covers the new written forms.
    Document::new(&converted, vocabulary, cancel).and_then(|document| {
        let gaps = (0..document.words.len()).map(|index| document.gap(index).to_owned()).collect::<Vec<_>>();
        document.rebuild_gaps(&gaps, None, cancel, true)
    }).unwrap_or_else(|_| text.into())
}

const LEGACY_ENCODING: &str = "CLS, then for each word: tokens of word_prefix+word (no special tokens) and, if the recognizer text has a sentence/comma mark right after the word, the tokens of that mark; then SEP. The label of a word is read at its last word token (before any mark tokens).";
const ENCODING: &str = "CLS, then for each word: tokens of word_prefix+word (no special tokens), followed by the recognizer cue from the gap after it; then SEP. Non-final gaps exactly equal to empty, dot, hyphen, slash or colon have no cue. Otherwise question wins over period/exclamation, then semicolon/comma/colon/em dash/en dash/double hyphen/spaced hyphen map to comma. A final word with no cue receives a period cue. Read each label at its last word token, before cue tokens.";
const I_FORMS: [&str; 5] = ["i", "i'm", "i'll", "i've", "i'd"];

// Lowercase English/American SCOWL 2020.12.07 words through size 35.
// https://wordlist.aspell.net/; permissive SCOWL and upstream notices are
// bundled in licenses/scowl-2020.12.07.txt (no CC BY-SA word data).
// Regenerate with evaluation/build_punctuation_wordlist.py. Never infer that
// an unfamiliar title-cased word is lowercase just because an end was removed.
fn common_lowercase(word: &str) -> bool {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS
        .get_or_init(|| include_str!("punctuation_lowercase.txt").lines().collect())
        .binary_search(&word.replace('’', "'").as_str())
        .is_ok()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LabelList {
    Array(Vec<String>),
    // Untagged Serde values retain JSON object keys as strings.
    Indexed(BTreeMap<String, String>),
}

impl LabelList {
    fn ordered(self) -> Result<Vec<String>> {
        match self {
            Self::Array(labels) => Ok(labels),
            Self::Indexed(map) => (0..map.len())
                .map(|index| map.get(&index.to_string()).cloned().ok_or(Failure::Config))
                .collect(),
        }
    }
}

// Missing fields mean a legacy package; explicit null is a malformed contract.
fn present_config_field<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

const SPAN_ENCODING: &str =
    "Read span_logits at the same last-subword position as logits; independent argmax.";

#[derive(Deserialize)]
struct ConfigFile {
    schema: u32,
    labels: LabelList,
    label_meaning: BTreeMap<String, Mark>,
    window_words: usize,
    hop_words: usize,
    cls_id: u32,
    sep_id: u32,
    pad_id: u32,
    word_prefix: String,
    lowercase_words_except: Vec<String>,
    encoding: String,
    #[serde(default)]
    recognizer_marks: BTreeMap<String, Vec<u32>>,
    #[serde(default, deserialize_with = "present_config_field")]
    span_labels: Option<LabelList>,
    #[serde(default, deserialize_with = "present_config_field")]
    span_label_scheme: Option<String>,
    #[serde(default, deserialize_with = "present_config_field")]
    span_output: Option<String>,
    #[serde(default, deserialize_with = "present_config_field")]
    punct_output: Option<String>,
    #[serde(default, deserialize_with = "present_config_field")]
    span_encoding: Option<String>,
    #[serde(default, deserialize_with = "present_config_field")]
    span_classes: Option<Vec<crate::written_form::Class>>,
}

struct Config {
    labels: Vec<Mark>,
    span_labels: Option<Vec<crate::written_form::BioLabel>>,
    window_words: usize,
    hop_words: usize,
    cls_id: u32,
    sep_id: u32,
    pad_id: u32,
    word_prefix: String,
    exceptions: HashSet<String>,
    recognizer_marks: BTreeMap<String, Vec<u32>>,
}

impl Config {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: ConfigFile = serde_json::from_slice(bytes).map_err(|_| Failure::Config)?;
        if raw.schema != 1
            // Existing exports describe the same tensor layout in less detail.
            // Cue extraction always follows build_train.tokens(), for both texts.
            || !matches!(raw.encoding.trim(), ENCODING | LEGACY_ENCODING)
            || raw.word_prefix != " "
            || raw.window_words == 0
            || raw.window_words > 1024
            || raw.hop_words == 0
            || raw.hop_words > raw.window_words
            || raw.cls_id == raw.sep_id
            || raw.cls_id == raw.pad_id
            || raw.sep_id == raw.pad_id
        {
            return Err(Failure::Config);
        }
        let span_labels = raw.parse_span_labels()?;
        if raw
            .punct_output
            .as_deref()
            .is_some_and(|name| name != "logits")
        {
            return Err(Failure::Config);
        }
        let labels = raw.labels.ordered()?;
        if labels.is_empty()
            || labels.len() > 32
            || labels.iter().collect::<HashSet<_>>().len() != labels.len()
        {
            return Err(Failure::Config);
        }
        let labels: Vec<Mark> = labels
            .iter()
            .map(|label| raw.label_meaning.get(label).copied().ok_or(Failure::Config))
            .collect::<Result<_>>()?;
        if !labels.contains(&Mark::None) {
            return Err(Failure::Config);
        }
        for word in &raw.lowercase_words_except {
            if word.is_empty()
                || word.to_lowercase() != *word
                || !word_regex().find(word).is_some_and(|m| m.as_str() == word)
            {
                return Err(Failure::Config);
            }
        }
        for (mark, ids) in &raw.recognizer_marks {
            if !matches!(mark.as_str(), "." | "," | "?") || ids.is_empty() {
                return Err(Failure::Config);
            }
        }
        Ok(Self {
            labels,
            span_labels,
            window_words: raw.window_words,
            hop_words: raw.hop_words,
            cls_id: raw.cls_id,
            sep_id: raw.sep_id,
            pad_id: raw.pad_id,
            word_prefix: raw.word_prefix,
            exceptions: raw.lowercase_words_except.into_iter().collect(),
            recognizer_marks: raw.recognizer_marks,
        })
    }
}

impl ConfigFile {
    fn parse_span_labels(&self) -> Result<Option<Vec<crate::written_form::BioLabel>>> {
        use crate::written_form::{BioLabel, Class};
        let Some(raw) = &self.span_labels else {
            if self.span_label_scheme.is_some()
                || self.span_output.is_some()
                || self.span_encoding.is_some()
                || self.span_classes.is_some()
            {
                return Err(Failure::Config);
            }
            return Ok(None);
        };
        if self
            .span_label_scheme
            .as_deref()
            .is_some_and(|scheme| scheme != "BIO")
            || self
                .span_output
                .as_deref()
                .is_some_and(|name| name != "span_logits")
            || self
                .span_encoding
                .as_deref()
                .is_some_and(|encoding| encoding != SPAN_ENCODING)
        {
            return Err(Failure::Config);
        }
        let raw: Vec<&str> = match raw {
            LabelList::Array(labels) => labels.iter().map(String::as_str).collect(),
            LabelList::Indexed(map) => (0..map.len())
                .map(|index| {
                    map.get(&index.to_string())
                        .map(String::as_str)
                        .ok_or(Failure::Config)
                })
                .collect::<Result<_>>()?,
        };
        // The renderer implements exactly these classes. Preserve config order
        // for argmax ties, but require one O and one B/I pair for every class.
        if raw.len() != 1 + 2 * Class::ALL.len() {
            return Err(Failure::Config);
        }
        let labels: Vec<BioLabel> = raw
            .into_iter()
            .map(|label| label.parse().map_err(|_| Failure::Config))
            .collect::<Result<_>>()?;
        if !labels.contains(&BioLabel::O)
            || Class::ALL.iter().any(|&class| {
                !labels.contains(&BioLabel::B(class)) || !labels.contains(&BioLabel::I(class))
            })
        {
            return Err(Failure::Config);
        }
        if let Some(classes) = &self.span_classes {
            if classes.len() != Class::ALL.len()
                || classes.iter().collect::<HashSet<_>>().len() != Class::ALL.len()
            {
                return Err(Failure::Config);
            }
        }
        Ok(Some(labels))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata.is_file().then(|| Stamp {
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
        created: metadata.created().ok(),
    })
}

fn identity_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.into()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Selection {
    pub(crate) root: PathBuf,
    directory: PathBuf,
    overridden: bool,
    stamps: Vec<Option<Stamp>>,
}

impl Selection {
    pub(crate) fn new(root: &Path, override_directory: Option<PathBuf>) -> Self {
        let overridden = override_directory.is_some();
        let directory = identity_path(
            &override_directory.unwrap_or_else(|| {
                crate::punctuation_model::selected_path(root, crate::model_verification::Check::Background)
                    .unwrap_or_else(|| crate::punctuation_model::path(root))
            }),
        );
        let mut stamps: Vec<_> = crate::punctuation_model::FILES
            .iter()
            .map(|file| stamp(&directory.join(file)))
            .collect();
        if !overridden {
            stamps.push(stamp(&directory.join("manifest.json")));
        }
        stamps.push(stamp(&crate::diarization_model::runtime_library(root)));
        Self {
            root: identity_path(root),
            directory,
            overridden,
            stamps,
        }
    }
    pub(crate) fn unchanged(&self) -> bool {
        *self == Self::new(&self.root, self.overridden.then(|| self.directory.clone()))
    }

    pub(crate) fn available(&self) -> bool {
        self.stamps.iter().all(Option::is_some)
    }
}

struct Cached {
    selection: Selection,
    engine: Engine,
    last_used: Instant,
}
static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
fn cache() -> &'static Mutex<Option<Cached>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// These two entry points are reachable only from the dedicated helper loop.
/// Engine::load independently enforces that boundary before any native call.
pub(crate) fn worker_load(selection: &Selection) -> Result<()> {
    let cancel = AtomicBool::new(false);
    let engine = Engine::load(selection, &cancel)?;
    if !selection.unchanged() {
        return Err(Failure::Load);
    }
    *cache().lock().map_err(|_| Failure::Busy)? = Some(Cached {
        selection: selection.clone(),
        engine,
        last_used: Instant::now(),
    });
    Ok(())
}

pub(crate) fn worker_predict(selection: &Selection, text: &str) -> Result<Prediction> {
    if !crate::punctuation_worker::is_worker() {
        return Err(Failure::Runtime);
    }
    let cancel = AtomicBool::new(false);
    let document = Document::new(text, &[], &cancel)?;
    predict_cached(selection, &document, &cancel, ModelRetention::OnExit)
}

fn predict_cached(
    selection: &Selection,
    document: &Document<'_>,
    cancel: &AtomicBool,
    retention: ModelRetention,
) -> Result<Prediction> {
    check(cancel)?;
    // Overlapping callers use rules instead of waiting behind another job.
    let mut guard = cache().try_lock().map_err(|_| Failure::Busy)?;
    if guard
        .as_ref()
        .is_some_and(|cached| cached.selection != *selection)
    {
        *guard = None;
    }
    if guard.is_none() {
        let engine = Engine::load(selection, cancel)?;
        if !selection.unchanged() {
            return Err(Failure::Load);
        }
        *guard = Some(Cached {
            selection: selection.clone(),
            engine,
            last_used: Instant::now(),
        });
    }
    let cached = guard.as_mut().ok_or(Failure::Load)?;
    let result = cached.engine.predict(document, cancel);
    cached.last_used = Instant::now();
    if retention == ModelRetention::Immediately
        || result
            .as_ref()
            .err()
            .is_some_and(|error| *error != Failure::Cancelled)
    {
        *guard = None;
    }
    result
}

pub(crate) fn expired(age: Duration, retention: ModelRetention) -> bool {
    match retention {
        ModelRetention::Immediately => true,
        ModelRetention::Minutes10 => age >= Duration::from_secs(600),
        ModelRetention::Minutes30 => age >= Duration::from_secs(1800),
        ModelRetention::OnExit => false,
    }
}

/// Parent idle/pressure workers own scheduling. Never wait for active inference.
pub fn release_idle(retention: ModelRetention) -> bool {
    crate::punctuation_worker::release_idle(retention)
}

pub fn release_cached() -> bool {
    crate::punctuation_worker::release_cached()
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
struct Engine {
    session: ort::session::Session,
    tokenizer: tokenizers::Tokenizer,
    config: Config,
}

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
struct Engine;

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
impl Engine {
    fn load(_: &Selection, _: &AtomicBool) -> Result<Self> {
        Err(Failure::Unsupported)
    }
    fn predict(&mut self, _: &Document<'_>, _: &AtomicBool) -> Result<Prediction> {
        Err(Failure::Unsupported)
    }
}

// Decode one window without allocating a softmax: its argmax is identical.
fn decode_logits<T: Copy>(
    shape: &[i64],
    logits: &[f32],
    tokens: usize,
    positions: &[usize],
    labels: &[T],
) -> Result<Vec<T>> {
    if shape != [1, tokens as i64, labels.len() as i64]
        || labels.is_empty()
        || tokens.checked_mul(labels.len()) != Some(logits.len())
        || logits.iter().any(|value| !value.is_finite())
        || positions.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Failure::Output);
    }
    positions
        .iter()
        .map(|&position| {
            if position == 0 || position >= tokens.saturating_sub(1) {
                return Err(Failure::Output);
            }
            let row = &logits[position * labels.len()..(position + 1) * labels.len()];
            // Strict > keeps the first class on ties, matching torch.argmax.
            let best = (1..row.len()).fold(
                0,
                |best, index| if row[index] > row[best] { index } else { best },
            );
            Ok(labels[best])
        })
        .collect()
}

fn windows(count: usize, width: usize, hop: usize) -> Vec<(usize, usize)> {
    if count == 0 {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + width).min(count);
        output.push((start, end));
        if end == count {
            break;
        }
        start += hop;
    }
    output
}

fn merge_window<T: Copy>(
    output: &mut [Option<T>],
    best: &mut [Option<usize>],
    start: usize,
    labels: &[T],
) -> Result<()> {
    if output.len() != best.len()
        || start
            .checked_add(labels.len())
            .is_none_or(|end| end > output.len())
    {
        return Err(Failure::Output);
    }
    for (index, &label) in labels.iter().enumerate() {
        let distance = index.min(labels.len() - 1 - index);
        if best[start + index].is_none_or(|previous| distance > previous) {
            best[start + index] = Some(distance);
            output[start + index] = Some(label);
        }
    }
    Ok(())
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
impl Engine {
    fn load(selection: &Selection, cancel: &AtomicBool) -> Result<Self> {
        use crate::model_verification::Check;
        check(cancel)?;
        if !crate::punctuation_worker::is_worker() {
            return Err(Failure::Runtime);
        }
        if !crate::diarization_model::AVAILABLE {
            return Err(Failure::Unsupported);
        }
        if !selection.overridden
            && !crate::punctuation_model::status_at(
                &selection.root,
                &selection.directory,
                Check::Blocking,
            )
            .ready()
        {
            return Err(Failure::Missing);
        }
        let config_path = selection.directory.join("punct-config.json");
        if stamp(&config_path).is_none_or(|s| s.bytes > 64 * 1024) {
            return Err(Failure::Config);
        }
        let config = Config::parse(&std::fs::read(config_path).map_err(|_| Failure::Config)?)?;
        let mut tokenizer =
            tokenizers::Tokenizer::from_file(selection.directory.join("tokenizer.json"))
                .map_err(|_| Failure::Tokenizer)?;
        // Per-word encoding must never inherit truncation or padding from JSON.
        tokenizer
            .with_truncation(None)
            .map_err(|_| Failure::Tokenizer)?;
        tokenizer.with_padding(None);
        for id in [config.cls_id, config.sep_id, config.pad_id] {
            if tokenizer.id_to_token(id).is_none() {
                return Err(Failure::Config);
            }
        }
        for (mark, ids) in &config.recognizer_marks {
            if tokenizer
                .encode(mark.as_str(), false)
                .map_err(|_| Failure::Tokenizer)?
                .get_ids()
                != ids.as_slice()
            {
                return Err(Failure::Config);
            }
        }
        check(cancel)?;
        if !crate::punctuation_model::runtime_status(&selection.root, Check::Blocking).ready() {
            return Err(Failure::Runtime);
        }
        crate::punctuation_worker::initialize_runtime(&selection.root)?;
        let session = ort::session::Session::builder()
            .map_err(|_| Failure::Load)?
            // Native messages can contain paths; diagnostics above record only
            // static failure categories, including on malformed graphs.
            .with_logger(std::sync::Arc::new(|_, _, _, _, _| {}))
            .map_err(|_| Failure::Load)?
            .with_execution_providers([ort::ep::CPU::default().build().error_on_failure()])
            .map_err(|_| Failure::Load)?
            .with_intra_threads(2)
            .map_err(|_| Failure::Load)?
            .with_inter_threads(1)
            .map_err(|_| Failure::Load)?
            .commit_from_file(selection.directory.join("model.int8.onnx"))
            .map_err(|_| Failure::Load)?;
        validate_graph(
            &session,
            config.labels.len(),
            config.span_labels.as_ref().map(Vec::len),
        )?;
        check(cancel)?;
        Ok(Self {
            session,
            tokenizer,
            config,
        })
    }

    fn predict(&mut self, document: &Document<'_>, cancel: &AtomicBool) -> Result<Prediction> {
        let mut pieces = Vec::with_capacity(document.words.len());
        for (index, word) in document.words.iter().enumerate() {
            check(cancel)?;
            let lower = word.text.to_lowercase();
            let input = format!(
                "{}{}",
                self.config.word_prefix,
                if self.config.exceptions.contains(&lower) {
                    word.text
                } else {
                    &lower
                }
            );
            let ids = self
                .tokenizer
                .encode(input.as_str(), false)
                .map_err(|_| Failure::Tokenizer)?
                .get_ids()
                .to_vec();
            if ids.is_empty() {
                return Err(Failure::Tokenizer);
            }
            let cue = recognizer_cue(document.gap(index), index + 1 == document.words.len());
            let marks = if cue.is_empty() {
                Vec::new()
            } else {
                self.tokenizer
                    .encode(cue, false)
                    .map_err(|_| Failure::Tokenizer)?
                    .get_ids()
                    .to_vec()
            };
            if !cue.is_empty() && marks.is_empty() {
                return Err(Failure::Tokenizer);
            }
            pieces.push((ids, marks));
        }
        let mut output = vec![None; pieces.len()];
        let mut best = vec![None; pieces.len()];
        for (start, end) in windows(
            pieces.len(),
            self.config.window_words,
            self.config.hop_words,
        ) {
            check(cancel)?;
            let (ids, positions) = encode_pieces(&pieces[start..end], &self.config)?;
            let count = ids.len();
            // Encoding is unpadded, just like onnx_check.py.
            let mask = vec![1_i64; count];
            let input = ort::value::Tensor::from_array(([1, count], ids))
                .map_err(|_| Failure::Inference)?;
            let attention = ort::value::Tensor::from_array(([1, count], mask))
                .map_err(|_| Failure::Inference)?;
            let values = self
                .session
                .run(ort::inputs!["input_ids" => input, "attention_mask" => attention])
                .map_err(|_| Failure::Inference)?;
            check(cancel)?;
            let (shape, logits) = values
                .get("logits")
                .ok_or(Failure::Output)?
                .try_extract_tensor::<f32>()
                .map_err(|_| Failure::Output)?;
            let labels = decode_logits(
                shape.as_ref(),
                logits,
                count,
                &positions,
                &self.config.labels,
            )?;
            let spans = if let Some(span_labels) = &self.config.span_labels {
                let (shape, logits) = values
                    .get("span_logits")
                    .ok_or(Failure::Output)?
                    .try_extract_tensor::<f32>()
                    .map_err(|_| Failure::Output)?;
                Some(decode_logits(
                    shape.as_ref(),
                    logits,
                    count,
                    &positions,
                    span_labels,
                )?)
            } else {
                None
            };
            // Merge both heads as one value so centrality and window ties can
            // never select punctuation and spans from different windows.
            let labels: Vec<_> = labels
                .into_iter()
                .enumerate()
                .map(|(index, label)| (label, spans.as_ref().map(|spans| spans[index])))
                .collect();
            merge_window(&mut output, &mut best, start, &labels)?;
        }
        check(cancel)?;
        let mut labels = Vec::with_capacity(output.len());
        let mut spans = self
            .config
            .span_labels
            .as_ref()
            .map(|_| Vec::with_capacity(output.len()));
        for prediction in output {
            let (label, span) = prediction.ok_or(Failure::Output)?;
            labels.push(label);
            if let Some(spans) = &mut spans {
                spans.push(span.ok_or(Failure::Output)?);
            }
        }
        Ok(Prediction { labels, spans })
    }
}

fn encode_pieces(
    pieces: &[(Vec<u32>, Vec<u32>)],
    config: &Config,
) -> Result<(Vec<i64>, Vec<usize>)> {
    let mut ids = vec![i64::from(config.cls_id)];
    let mut positions = Vec::with_capacity(pieces.len());
    for (word, marks) in pieces {
        if word.is_empty() {
            return Err(Failure::Tokenizer);
        }
        ids.extend(word.iter().map(|&id| i64::from(id)));
        positions.push(ids.len() - 1);
        ids.extend(marks.iter().map(|&id| i64::from(id)));
        // Bound pathological words. Never truncate a word or lose a label.
        if ids.len() >= 4096 {
            return Err(Failure::Tokenizer);
        }
    }
    ids.push(i64::from(config.sep_id));
    Ok((ids, positions))
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
fn validate_graph(
    session: &ort::session::Session,
    classes: usize,
    span_classes: Option<usize>,
) -> Result<()> {
    use ort::value::{TensorElementType, ValueType};
    if session.inputs().len() != 2
        || session.outputs().len() != 1 + usize::from(span_classes.is_some())
    {
        return Err(Failure::Load);
    }
    for name in ["input_ids", "attention_mask"] {
        let input = session
            .inputs()
            .iter()
            .find(|input| input.name() == name)
            .ok_or(Failure::Load)?;
        match input.dtype() {
            ValueType::Tensor {
                ty: TensorElementType::Int64,
                shape,
                ..
            } if shape.len() == 2
                && (shape[0] == 1 || shape[0] == -1)
                && (shape[1] == -1 || shape[1] > 0) => {}
            _ => return Err(Failure::Load),
        }
    }
    validate_head_outputs(
        session
            .outputs()
            .iter()
            .map(|output| (output.name(), output.dtype())),
        classes,
        span_classes,
    )
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
fn validate_head_outputs<'a>(
    outputs: impl IntoIterator<Item = (&'a str, &'a ort::value::ValueType)>,
    classes: usize,
    span_classes: Option<usize>,
) -> Result<()> {
    let mut punct = false;
    let mut spans = false;
    for (name, dtype) in outputs {
        match name {
            "logits" if !punct => {
                validate_head_type(dtype, classes)?;
                punct = true;
            }
            "span_logits" if !spans => {
                validate_head_type(dtype, span_classes.ok_or(Failure::Load)?)?;
                spans = true;
            }
            _ => return Err(Failure::Load),
        }
    }
    if punct && spans == span_classes.is_some() {
        Ok(())
    } else {
        Err(Failure::Load)
    }
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
fn validate_head_type(dtype: &ort::value::ValueType, classes: usize) -> Result<()> {
    use ort::value::{TensorElementType, ValueType};
    match dtype {
        ValueType::Tensor {
            ty: TensorElementType::Float32,
            shape,
            ..
        } if shape.len() == 3
            && (shape[0] == 1 || shape[0] == -1)
            && (shape[1] == -1 || shape[1] > 0)
            && (shape[2] == classes as i64 || shape[2] == -1) =>
        {
            Ok(())
        }
        _ => Err(Failure::Load),
    }
}

fn recognizer_cue(gap: &str, last: bool) -> &'static str {
    if !last && matches!(gap, "" | "." | "-" | "/" | ":") {
        return "";
    }
    if gap.contains('?') {
        "?"
    } else if gap.contains(['.', '!']) {
        "."
    } else if gap.contains([',', ';', ':', '—', '–']) || gap.contains("--") || gap.contains(" - ")
    {
        ","
    } else if last {
        "."
    } else {
        ""
    }
}

fn word_regex() -> &'static Regex {
    static WORD: OnceLock<Regex> = OnceLock::new();
    // Python re's [^\W_] is Unicode letters/numbers. Rust's \w also
    // includes combining marks, join controls and other connector punctuation,
    // so spelling this out is necessary to match metrics.py exactly.
    WORD.get_or_init(|| {
        Regex::new(r"[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*").expect("static word regex")
    })
}

fn same_words(left: &str, right: &str) -> bool {
    word_regex()
        .find_iter(left)
        .map(|word| word.as_str().to_lowercase())
        .eq(word_regex()
            .find_iter(right)
            .map(|word| word.as_str().to_lowercase()))
}

struct Word<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    protected: bool,
}
struct Document<'a> {
    text: &'a str,
    words: Vec<Word<'a>>,
    preserved: Vec<(usize, usize)>,
    proper: HashSet<String>,
}

type VocabularyMatchers = std::sync::Arc<Vec<Regex>>;

#[derive(Default)]
struct VocabularyCache {
    // One entry bounds retention even when vocabulary changes frequently.
    entry: Mutex<Option<(Vec<String>, VocabularyMatchers)>>,
}

impl VocabularyCache {
    fn matchers(
        &self,
        vocabulary: &[Vocabulary],
        cancel: &AtomicBool,
    ) -> Result<VocabularyMatchers> {
        let names: Vec<String> = vocabulary
            .iter()
            .filter(|entry| entry.enabled)
            .flat_map(|entry| [&entry.heard, &entry.written])
            .filter(|name| !name.is_empty())
            .cloned()
            .collect();
        let mut entry = self.entry.lock().unwrap_or_else(|error| error.into_inner());
        check(cancel)?;
        if let Some((_, matchers)) = entry.as_ref().filter(|(key, _)| *key == names) {
            return Ok(matchers.clone());
        }
        let mut matchers = Vec::with_capacity(names.len());
        for name in &names {
            check(cancel)?;
            matchers.push(
                regex::RegexBuilder::new(&regex::escape(name))
                    .case_insensitive(true)
                    .build()
                    .map_err(|_| Failure::Config)?,
            );
        }
        let matchers = std::sync::Arc::new(matchers);
        *entry = Some((names, matchers.clone()));
        Ok(matchers)
    }
}

fn vocabulary_matchers(
    vocabulary: &[Vocabulary],
    cancel: &AtomicBool,
) -> Result<VocabularyMatchers> {
    static MATCHERS: OnceLock<VocabularyCache> = OnceLock::new();
    MATCHERS
        .get_or_init(VocabularyCache::default)
        .matchers(vocabulary, cancel)
}

fn trim_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '.' | ','
                    | ';'
                    | ':'
                    | '?'
                    | '!'
                    | '…'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '\''
                    | '“'
                    | '”'
                    | '‘'
                    | '’'
                    | '—'
                    | '–'
                    | '-'
            )
    })
}

fn prose_dash(gap: &str) -> bool {
    gap.contains(['—', '–'])
        || gap.contains("--")
        || (gap.contains('-') && gap.chars().any(char::is_whitespace))
}

impl<'a> Document<'a> {
    fn new(text: &'a str, vocabulary: &[Vocabulary], cancel: &AtomicBool) -> Result<Self> {
        check(cancel)?;
        let mut words: Vec<_> = word_regex()
            .find_iter(text)
            .map(|word| Word {
                text: word.as_str(),
                start: word.start(),
                end: word.end(),
                protected: false,
            })
            .collect();
        let mut preserved = crate::structured_clean::preserved_candidate_spans(text, &|| {
            cancel.load(Ordering::Acquire)
        })
        .map_err(|_| {
            if cancel.load(Ordering::Acquire) {
                Failure::Cancelled
            } else {
                Failure::Output
            }
        })?;
        preserved.extend(literal_spans(text));
        // Preserve whole non-prose tokens, including numbers, addresses, paths,
        // identifiers and mixed-case names, as the existing cleanup code does.
        let mut offset = 0;
        for token in text.split_inclusive(char::is_whitespace) {
            let raw = token.trim_end_matches(char::is_whitespace);
            let core = trim_token(raw);
            // A URL, address, path or explicit identifier can contain dashes.
            // Other tokens may contain prose glued to a dash (LoopNet—the).
            let literal = core.contains(['/', '\\', '@', '_'])
                || core.starts_with("www.")
                || (!core.is_empty()
                    && word_regex()
                        .find_iter(core)
                        .all(|word| word.as_str().chars().all(char::is_numeric)));
            if literal {
                let begin = offset + raw.find(core).unwrap_or(0);
                if !core.is_empty() {
                    preserved.push((begin, begin + core.len()));
                }
            } else {
                // Do not let punctuation at a token edge or a glued prose dash
                // make an otherwise ordinary word a protected literal.
                static DASH: OnceLock<Regex> = OnceLock::new();
                let dash = DASH.get_or_init(|| Regex::new(r"[—–]|--").expect("static dash regex"));
                let core_offset = offset + raw.find(core).unwrap_or(0);
                let mut start = 0;
                for end in dash
                    .find_iter(core)
                    .map(|m| (m.start(), m.end()))
                    .chain(std::iter::once((core.len(), core.len())))
                {
                    let part = trim_token(&core[start..end.0]);
                    if word_regex().is_match(part)
                        && (!part
                            .chars()
                            .all(|ch| ch.is_alphabetic() || matches!(ch, '\'' | '’'))
                            || part.chars().skip(1).any(char::is_uppercase))
                    {
                        let begin =
                            core_offset + start + core[start..end.0].find(part).unwrap_or(0);
                        preserved.push((begin, begin + part.len()));
                    }
                    start = end.1;
                }
            }
            offset += token.len();
        }
        // A spaced numeric value/range is also an indivisible literal.
        for pair in words.windows(2) {
            if pair
                .iter()
                .all(|word| word.text.chars().all(char::is_numeric))
            {
                let gap = &text[pair[0].end..pair[1].start];
                if gap
                    .chars()
                    .all(|ch| ch.is_whitespace() || matches!(ch, ',' | '.' | ':' | '/' | '-' | '–'))
                {
                    preserved.push((pair[0].start, pair[1].end));
                }
            }
        }
        for matcher in vocabulary_matchers(vocabulary, cancel)?.iter() {
            check(cancel)?;
            for found in matcher.find_iter(text) {
                if words.iter().any(|word| word.start == found.start())
                    && words.iter().any(|word| {
                        word.end == found.end()
                            || (word.start < found.end()
                                && found.end() < word.end
                                && matches!(
                                    &text[found.end()..word.end],
                                    "'s" | "’s" | "'S" | "’S"
                                ))
                    })
                {
                    preserved.push((found.start(), found.end()));
                }
            }
        }
        preserved.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in preserved {
            if start >= end
                || !text.is_char_boundary(start)
                || !text.is_char_boundary(end)
                || end > text.len()
            {
                return Err(Failure::Output);
            }
            if let Some(last) = merged.last_mut().filter(|last| start < last.1) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        for word in &mut words {
            word.protected = merged
                .iter()
                .any(|&(start, end)| start < word.end && end > word.start);
        }
        // In glued forms even the casing of either component must survive.
        for index in 1..words.len() {
            let gap = &text[words[index - 1].end..words[index].start];
            if !gap.chars().any(char::is_whitespace) && !prose_dash(gap) {
                words[index - 1].protected = true;
                words[index].protected = true;
            }
        }
        let mut proper = HashSet::new();
        for (index, word) in words.iter().enumerate().skip(1) {
            if word.text.chars().next().is_some_and(char::is_uppercase)
                && !text[words[index - 1].end..word.start].contains(['.', '?', '!', ':', '"', '“'])
            {
                proper.insert(word.text.to_lowercase());
            }
        }
        check(cancel)?;
        Ok(Self {
            text,
            words,
            preserved: merged,
            proper,
        })
    }

    fn gap(&self, index: usize) -> &'a str {
        &self.text[self.words[index].end
            ..self
                .words
                .get(index + 1)
                .map_or(self.text.len(), |word| word.start)]
    }

    fn abbreviation(&self, index: usize) -> bool {
        let word = self.words[index].text;
        if !self.gap(index).starts_with('.') {
            return false;
        }
        let lower = word.to_lowercase();
        if matches!(lower.as_str(), "etc" | "vs") {
            return true;
        }
        if word.chars().count() == 1 && word != "I" && word.chars().all(char::is_uppercase) {
            return true;
        }
        index > 0
            && self.gap(index - 1) == "."
            && matches!(
                (
                    self.words[index - 1].text.to_lowercase().as_str(),
                    lower.as_str()
                ),
                ("e", "g") | ("i", "e")
            )
    }

    fn end_in_gap(&self, index: usize, gap: &str, prediction: Option<Mark>) -> bool {
        if gap.contains(['?', '!']) {
            return true;
        }
        if !gap.contains('.') {
            return false;
        }
        if !self.abbreviation(index) {
            return true;
        }
        let word = self.words[index].text;
        // A standalone letter can end a sentence ("R and D. But ...").
        // Keep glued acronyms and spelled-out abbreviations under their existing
        // rules. Only casing changes here; the abbreviation's period survives.
        if word.len() != 1
            || !word.chars().all(char::is_uppercase)
            || !gap.chars().any(char::is_whitespace)
            || (index > 0 && self.gap(index - 1) == ".")
        {
            return false;
        }
        prediction.is_some_and(Mark::terminal)
            || self.words.get(index + 1).is_some_and(|next| {
                let lower = next.text.to_lowercase();
                // Unknown title-cased words, known proper names and another
                // initial may belong to a name such as J. Smith or J. A. Smith.
                next.text.len() > 1
                    && next.text.chars().next().is_some_and(char::is_uppercase)
                    && !next.protected
                    && !self.proper.contains(&lower)
                    && common_lowercase(&lower)
            })
    }

    fn rebuilt_gap(&self, index: usize, prediction: Option<Mark>) -> String {
        let word = &self.words[index];
        let last = index + 1 == self.words.len();
        let end = self
            .words
            .get(index + 1)
            .map_or(self.text.len(), |word| word.start);
        let gap = self.gap(index);
        if !last
            && ((!gap.chars().any(char::is_whitespace) && !prose_dash(gap))
                || self.abbreviation(index))
        {
            return gap.into();
        }
        let mut begin_edit = word.end;
        let mut end_edit = end;
        for &(start, stop) in &self.preserved {
            if start <= word.end && stop > word.end {
                begin_edit = begin_edit.max(stop.min(end));
            }
            if start < end && stop >= end {
                end_edit = end_edit.min(start.max(word.end));
            }
        }
        if begin_edit > end_edit {
            return gap.into();
        }
        let editable = &self.text[begin_edit..end_edit];
        if !last && !editable.chars().any(char::is_whitespace) && !prose_dash(editable) {
            return gap.into();
        }
        // An empty quoted literal or other complete protected span in a gap
        // must survive too; abstain when no single editable interval remains.
        if self
            .preserved
            .iter()
            .any(|&(start, stop)| start >= begin_edit && stop <= end_edit && start < stop)
        {
            return gap.into();
        }
        let prefix = &self.text[word.end..begin_edit];
        let suffix = &self.text[end_edit..end];
        let mut mark = if last {
            prediction.filter(|mark| mark.terminal())
        } else {
            prediction
        };
        if prediction.is_none()
            && !last
            && !word.protected
            && fallback_word(word.text)
            && recognizer_period(editable)
        {
            mark = Some(Mark::None);
        }
        // Keep punctuation inside a protected closing quote. Do not append a
        // second sentence mark outside it or turn that ending into a comma.
        if prefix.contains(['.', '?', '!']) {
            mark = Some(Mark::None);
        }
        format!("{prefix}{}{suffix}", render_gap(editable, mark, last))
    }

    #[cfg(test)]
    fn rebuild(&self, predictions: Option<&[Mark]>, cancel: &AtomicBool) -> Result<String> {
        self.rebuild_with_case(predictions, cancel, true)
    }

    fn rebuild_with_case(&self, predictions: Option<&[Mark]>, cancel: &AtomicBool, house_style: bool) -> Result<String> {
        if predictions.is_some_and(|labels| labels.len() != self.words.len()) {
            return Err(Failure::Output);
        }
        if self.words.is_empty() {
            return Ok(self.text.into());
        }
        let gaps: Vec<_> = (0..self.words.len())
            .map(|index| self.rebuilt_gap(index, predictions.map(|labels| labels[index])))
            .collect();
        self.rebuild_gaps(&gaps, predictions, cancel, house_style)
    }

    fn rebuild_gaps(&self, gaps: &[String], predictions: Option<&[Mark]>, cancel: &AtomicBool, house_style: bool) -> Result<String> {
        if self.words.is_empty() {
            return Ok(self.text.into());
        }
        let mut output = self.text[..self.words[0].start].to_owned();
        for (index, word) in self.words.iter().enumerate() {
            check(cancel)?;
            let original = word.text;
            let lower = original.to_lowercase();
            let i_form = I_FORMS.contains(&lower.replace('’', "'").as_str());
            let capitalize = index == 0
                || self.end_in_gap(
                    index - 1,
                    &gaps[index - 1],
                    predictions.map(|labels| labels[index - 1]),
                );
            let removed_end =
                index > 0 && self.end_in_gap(index - 1, self.gap(index - 1), None) && !capitalize;
            let replacement = if word.protected {
                original.into()
            // Preserve explicit model sentence evidence (including a period
            // after a single-letter option). A later document cannot infer
            // that evidence from the period alone without confusing initials.
            } else if (house_style || predictions.is_some()) && (i_form || capitalize) {
                uppercase_first(original)
            } else if removed_end
                && common_lowercase(&lower)
                && !self.proper.contains(&lower)
                && !original.chars().skip(1).any(char::is_uppercase)
            {
                lower
            } else {
                original.into()
            };
            // Some Unicode case mappings expand into multiple letters/marks.
            // Only accept a case change that preserves the evaluation tokens.
            if same_words(original, &replacement) {
                output.push_str(&replacement);
            } else {
                output.push_str(original);
            }
            output.push_str(&gaps[index]);
        }
        check(cancel)?;
        Ok(output)
    }
}

fn uppercase_first(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect()
}

fn fallback_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the"
            | "a"
            | "an"
            | "your"
            | "my"
            | "our"
            | "their"
            | "its"
            | "every"
            | "and"
            | "or"
            | "but"
            | "because"
            | "if"
            | "than"
            | "whether"
            | "nor"
            | "of"
            | "onto"
    )
}

fn recognizer_period(gap: &str) -> bool {
    gap.chars().filter(|&ch| ch == '.').count() == 1
        && !gap.contains(['?', '!', ',', ';', ':'])
        && recognizer_cue(gap, false) == "."
}

fn is_dash(ch: char) -> bool {
    matches!(ch, '-' | '–' | '—')
}
fn editable_punctuation(ch: char) -> bool {
    matches!(ch, '.' | ',' | '?' | '!' | ';' | ':' | '…') || is_dash(ch)
}

fn render_gap(gap: &str, prediction: Option<Mark>, terminal: bool) -> String {
    // Unknown symbols are literal content, not punctuation evidence to erase.
    if gap.chars().any(|ch| {
        !ch.is_whitespace()
            && !editable_punctuation(ch)
            && !matches!(
                ch,
                '"' | '\'' | '“' | '”' | '‘' | '’' | '(' | ')' | '[' | ']' | '{' | '}'
            )
    }) {
        return gap.into();
    }
    let dash = gap.chars().any(is_dash);
    let ellipsis = gap.contains("..") || gap.contains('…');
    let prediction = if dash {
        Some(if terminal { Mark::Period } else { Mark::Comma })
    } else if ellipsis {
        Some(
            prediction
                .filter(|mark| *mark != Mark::None)
                .unwrap_or(if terminal { Mark::Period } else { Mark::Comma }),
        )
    } else {
        prediction
    };
    let Some(mark) = prediction else {
        return gap.into();
    };
    let split = gap.find(char::is_whitespace).unwrap_or(gap.len());
    let left = &gap[..split];
    let right = &gap[split..];
    let mut before = String::new();
    let mut after = String::new();
    let mut saw_mark = false;
    for ch in left.chars() {
        if editable_punctuation(ch) {
            saw_mark = true;
        } else if saw_mark {
            after.push(ch);
        } else {
            before.push(ch);
        }
    }
    let mut rest = String::new();
    for ch in right.chars() {
        if editable_punctuation(ch) {
            saw_mark = true;
        } else if matches!(ch, ')' | ']' | '}' | '”') {
            if saw_mark {
                after.push(ch);
            } else {
                before.push(ch);
            }
        } else if !ch.is_whitespace() {
            rest.push(ch);
        }
    }
    if terminal {
        format!("{before}{}{after}{rest}", mark.text())
    } else {
        format!("{before}{}{after} {rest}", mark.text())
    }
}

fn literal_spans(text: &str) -> Vec<(usize, usize)> {
    // Speech in quotes is prose, and a leading apostrophe is often part of a
    // word. Only paired code delimiters protect a whole quoted span.
    let mut spans = Vec::new();
    let mut open = None;
    for (index, ch) in text.char_indices() {
        if ch == '`' {
            if let Some(start) = open.take() {
                spans.push((start, index + 1));
            } else {
                open = Some(index);
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    #[test]
    fn written_form_finishes_casing_after_absorbing_internal_periods() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        for (source, expected) in [
            ("use GPT six point one. Soul today.", "Use GPT-6.1 Sol today."),
            ("pay four thousand. Dollars today.", "Pay $4,000 today."),
            ("pay 4.2 million dollars. send it.", "Pay $4.2 million. Send it."),
        ] {
            assert_eq!(super::written_form_finish(source, &[], None, &cancel), expected);
        }
    }

    #[test]
    fn written_form_preserves_vocabulary_and_protected_content() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let vocabulary = [crate::cleanup::Vocabulary {
            heard: "Ten Percent".into(), written: "Ten Percent".into(), enabled: true, context: vec![],
        }];
        assert_eq!(super::written_form_finish("ask Ten Percent for five dollars.", &vocabulary, None, &cancel),
            "Ask Ten Percent for $5.");
        let source = "keep `ten dollars`, \"ten dollars\" and https://example.com/ten. pay ten dollars.";
        assert_eq!(super::written_form_finish(source, &[], None, &cancel),
            "Keep `ten dollars`, \"ten dollars\" and https://example.com/ten. Pay $10.");
        assert_eq!(super::written_form_finish("Email ten at comet. Pay ten dollars.", &[], None, &cancel),
            "Email ten at comet. Pay $10.");
    }

    #[test]
    fn written_form_present_outside_labels_disable_rule_proposals() {
        use crate::written_form::BioLabel;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(super::written_form_finish("pay ten dollars.", &[], Some(&[BioLabel::O; 3]), &cancel),
            "Pay ten dollars.");
    }

    #[test]
    fn routed_special_rules_cannot_reenter_after_vocabulary_filtering() {
        use crate::written_form::BioLabel::{B, I, O};
        use crate::written_form::Class;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let vocabulary = vec![crate::cleanup::Vocabulary {
            heard: "GPT six Astra".into(), written: "GPT six Astra".into(),
            enabled: true, context: vec![],
        }];
        let text = "Ask GPT six Astra for ten dollars.";
        let labels = [O, O, O, O, O, B(Class::Money), I(Class::Money)];
        assert_eq!(super::written_form_finish(text, &vocabulary, Some(&labels), &cancel),
            "Ask GPT six Astra for $10.");
        assert_eq!(super::written_form_finish("GPT six Astra", &[], Some(&[]), &cancel),
            "GPT six Astra");
    }

    #[test]
    fn written_form_rules_do_not_rewrite_numeric_idioms() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let text = "I'm on cloud nine. It got done at the eleventh hour.";
        assert_eq!(super::written_form_finish(text, &[], None, &cancel), text);
    }

    #[test]
    fn second_head_labels_project_decimal_words_and_override_rules() {
        use crate::written_form::{BioLabel::{B, I, O}, Class};
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let labels = super::project_span_labels("pay 4.2 million dollars.",
            &[O, B(Class::Money), I(Class::Money), I(Class::Money), I(Class::Money)]).unwrap();
        assert_eq!(labels, [O, B(Class::Money), I(Class::Money), I(Class::Money)]);
        assert_eq!(super::written_form_finish("pay 4.2 million dollars.", &[], Some(&labels), &cancel),
            "Pay $4.2 million.");
        assert_eq!(super::project_span_labels("4.2", &[B(Class::Money), O]), Some(vec![O]));
        assert_eq!(super::project_span_labels("4.2 ten dollars", &[B(Class::Money), O, B(Class::Money), I(Class::Money)]),
            Some(vec![O, B(Class::Money), I(Class::Money)]));
        assert!(super::project_span_labels("ten dollars", &[B(Class::Money)]).is_none());
    }

    #[test]
    fn deferred_case_keeps_existing_removed_period_lowercase_guard() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let source = "use the. Feature for ten dollars";
        let document = super::Document::new(source, &[], &cancel).unwrap();
        let marks = vec![super::Mark::None; document.words.len()];
        let (text, _, _) = super::finish_inner(source, Some(&document), Ok(marks), &cancel, false);
        assert!(text.contains("the feature"), "{text}");
        assert!(super::written_form_finish(&text, &[], None, &cancel).starts_with("Use the feature"));
    }

    #[test]
    fn written_form_retains_model_sentence_evidence_after_single_letters() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let source = "choose option A and pay ten dollars";
        let document = super::Document::new(source, &[], &cancel).unwrap();
        let mut marks = vec![super::Mark::None; document.words.len()];
        marks[2] = super::Mark::Period;
        let (text, _, _) = super::finish_inner(source, Some(&document), Ok(marks), &cancel, false);
        assert!(super::written_form_finish(&text, &[], None, &cancel).contains("A. And pay $10"));
    }
    use super::*;

    fn rebuilt(text: &str, marks: &[Mark]) -> String {
        let cancel = AtomicBool::new(false);
        let document = Document::new(text, &[], &cancel).unwrap();
        let output = document.rebuild(Some(marks), &cancel).unwrap();
        assert!(same_words(text, &output));
        output
    }

    fn all_marks(text: &str, mark: Mark) -> String {
        rebuilt(text, &vec![mark; word_regex().find_iter(text).count()])
    }

    fn rules(text: &str) -> String {
        let cancel = AtomicBool::new(false);
        let document = Document::new(text, &[], &cancel).unwrap();
        let output = document.rebuild(None, &cancel).unwrap();
        assert!(same_words(text, &output));
        output
    }

    fn config_json() -> serde_json::Value {
        serde_json::json!({
            "schema":1,"labels":["0",".",",","?","-",":",";"],
            "label_meaning":{"0":"none",".":"period",",":"comma","?":"question","-":"comma",":":"comma",";":"semicolon"},
            "window_words":120,"hop_words":60,"cls_id":2,"sep_id":1,"pad_id":0,
            "word_prefix":" ","lowercase_words_except":I_FORMS,"encoding":ENCODING,
            "recognizer_marks":{".":[954],",":[1688],"?":[1654]}
        })
    }

    fn config() -> Config {
        let mut json = config_json();
        json["recognizer_marks"] = serde_json::json!({".":[954],",":[1688],"?":[1654]});
        Config::parse(&serde_json::to_vec(&json).unwrap()).unwrap()
    }

    fn span_config_json() -> serde_json::Value {
        use crate::written_form::{BioLabel, Class};
        let mut json = config_json();
        let labels: Vec<_> = std::iter::once(BioLabel::O)
            .chain(
                Class::ALL
                    .into_iter()
                    .flat_map(|class| [BioLabel::B(class), BioLabel::I(class)]),
            )
            .collect();
        json["span_labels"] = serde_json::json!(labels);
        json["span_label_scheme"] = serde_json::json!("BIO");
        json["span_output"] = serde_json::json!("span_logits");
        json["punct_output"] = serde_json::json!("logits");
        json["span_encoding"] = serde_json::json!(SPAN_ENCODING);
        json["span_classes"] = serde_json::json!(Class::ALL);
        json
    }

    #[test]
    fn unicode_words_match_python_evaluation_rule() {
        let input = "café don't don’t foo_bar 1.2 Straße 東京 ² e\u{301}x a\u{200d}b";
        let actual: Vec<_> = word_regex()
            .find_iter(input)
            .map(|word| word.as_str())
            .collect();
        assert_eq!(
            actual,
            [
                "café", "don't", "don’t", "foo", "bar", "1", "2", "Straße", "東京", "²", "e", "x",
                "a", "b"
            ]
        );
    }

    #[test]
    fn recognizer_features_match_reference_including_glued_punctuation() {
        for (gap, mark) in [
            (". ", "."),
            (".", ""),
            ("-", ""),
            ("/", ""),
            ("", ""),
            (".? ", "?"),
            (", ", ","),
            ("; ", ","),
            (":", ""),
            (": ", ","),
            ("— ", ","),
            ("–", ","),
            ("--", ","),
            (" - ", ","),
            ("! ", "."),
            (".\" ", "."),
            (".) ", "."),
            ("., ", "."),
            (" ", ""),
        ] {
            assert_eq!(recognizer_cue(gap, false), mark, "{gap:?}");
        }
        for (gap, mark) in [
            ("", "."),
            (" ", "."),
            ("'", "."),
            (".", "."),
            (":", ","),
            ("—", ","),
            ("?\"", "?"),
            (";", ","),
        ] {
            assert_eq!(recognizer_cue(gap, true), mark, "terminal {gap:?}");
        }
    }

    #[test]
    fn glued_gaps_and_their_casing_survive_any_prediction() {
        let text = "use Next.js at 10:30 on one-sheet with sam@example.com https://host.test/a and 10–20 at 1.2";
        for mark in [
            Mark::None,
            Mark::Comma,
            Mark::Period,
            Mark::Question,
            Mark::Semicolon,
        ] {
            let output = all_marks(text, mark);
            for literal in [
                "Next.js",
                "10:30",
                "one-sheet",
                "sam@example.com",
                "https://host.test/a",
                "10–20",
                "1.2",
            ] {
                assert!(output.contains(literal), "{literal}: {output}");
            }
        }
        assert_eq!(all_marks("alpha—beta", Mark::None), "Alpha, beta");
    }

    #[test]
    fn prose_dashes_become_commas_and_ranges_survive() {
        for dash in ["—", "–", "-", "--"] {
            let text = format!("alpha {dash} beta");
            assert_eq!(all_marks(&text, Mark::None), "Alpha, beta");
            assert_eq!(rules(&text), "Alpha, beta");
        }
        assert_eq!(
            all_marks("use 10 – 20 and 1 000 units", Mark::None),
            "Use 10 – 20 and 1 000 units"
        );
        for text in [
            "tried— it",
            "tried —it",
            "tried—it",
            "tried–it",
            "tried--it",
            "tried-- it",
            "tried --it",
            "tried - it",
        ] {
            assert_eq!(all_marks(text, Mark::None), "Tried, it", "{text}");
            assert_eq!(rules(text), "Tried, it", "{text}");
        }
        assert_eq!(rules("LoopNet—the listing"), "LoopNet, the listing");
        for literal in [
            "2019-2020",
            "10–20",
            "one-sheet",
            "https://host/a—b",
            "a--b@example.com",
            "C:\\path\\a—b",
            "some_id--other",
            "`alpha—beta`",
        ] {
            assert!(
                all_marks(&format!("use {literal} now"), Mark::Question).contains(literal),
                "{literal}"
            );
        }
    }

    #[test]
    fn ellipses_collapse_without_losing_words() {
        assert_eq!(
            all_marks("alpha... Beta… gamma", Mark::None),
            "Alpha, beta, gamma"
        );
        assert_eq!(rules("wait..."), "Wait.");
        assert_eq!(rules("wait…"), "Wait.");
    }

    #[test]
    fn sentence_starts_and_semicolons_questions() {
        assert_eq!(
            rebuilt(
                "hello world next",
                &[Mark::Period, Mark::None, Mark::Period]
            ),
            "Hello. World next."
        );
        assert_eq!(
            rebuilt(
                "hello world next",
                &[Mark::Semicolon, Mark::Question, Mark::None]
            ),
            "Hello; world? Next"
        );
        assert_eq!(rules("hello! world? next."), "Hello! World? Next.");
    }

    #[test]
    fn removed_ends_lowercase_only_with_case_safeguards() {
        assert_eq!(
            all_marks("we need the. Feature", Mark::None),
            "We need the feature"
        );
        let text = "we think. I'll help. NASA works. iPhone shines. Alice agrees and Alice returns";
        assert_eq!(
            all_marks(text, Mark::None),
            "We think I'll help NASA works iPhone shines Alice agrees and Alice returns"
        );
        assert_eq!(
            all_marks(
                "i know. i'm sure. i’ll ask. i’ve tried. i'd help",
                Mark::None
            ),
            "I know I'm sure I’ll ask I’ve tried I'd help"
        );
        // Mid-sentence title case is retained even without another occurrence.
        assert_eq!(
            all_marks("try Example today", Mark::None),
            "Try Example today"
        );
    }

    #[test]
    fn removed_ends_keep_unfamiliar_names_but_lowercase_common_words() {
        assert_eq!(
            all_marks("yes. Anna Groves sent the. Documents", Mark::None),
            "Yes Anna Groves sent the documents"
        );
        assert_eq!(
            all_marks("ask Robin first. Robin knows. Quenby agrees", Mark::None),
            "Ask Robin first Robin knows Quenby agrees"
        );
        assert_eq!(
            rules("send the. Anna Groves report"),
            "Send the Anna Groves report"
        );
        assert_eq!(
            all_marks("try again. Don’t stop", Mark::None),
            "Try again don’t stop"
        );
    }

    #[test]
    fn lowercase_allowlist_is_sorted_unique_and_excludes_title_case_names() {
        let words: Vec<_> = include_str!("punctuation_lowercase.txt").lines().collect();
        assert!(words.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(words
            .iter()
            .all(|word| !word.is_empty()
                && word.chars().all(|ch| ch.is_ascii_lowercase() || ch == '\'')));
        for word in ["but", "feature", "documents", "don't"] {
            assert!(common_lowercase(word), "{word}");
        }
        for name in ["anna", "alice", "quenby"] {
            assert!(!common_lowercase(name), "{name}");
        }
    }

    #[test]
    fn vocabulary_names_preserve_inner_text_and_case() {
        let text = "use the. Sono Jot and the. Name";
        let vocabulary = [
            Vocabulary {
                heard: "sono jot".into(),
                written: "Sono Jot".into(),
                enabled: true,
                context: vec![],
            },
            Vocabulary {
                heard: "name".into(),
                written: "Name".into(),
                enabled: true,
                context: vec![],
            },
        ];
        let cancel = AtomicBool::new(false);
        let document = Document::new(text, &vocabulary, &cancel).unwrap();
        let mut marks = vec![Mark::None; document.words.len()];
        marks[2] = Mark::Question; // Inside Sono Jot: must be ignored.
        let output = document.rebuild(Some(&marks), &cancel).unwrap();
        assert_eq!(output, "Use the Sono Jot and the Name");
        assert!(same_words(text, &output));
        let possessive = Document::new("use the. Name's label", &vocabulary, &cancel).unwrap();
        assert_eq!(
            possessive.rebuild(None, &cancel).unwrap(),
            "Use the Name's label"
        );
    }

    #[test]
    fn disabled_vocabulary_is_not_case_evidence() {
        let cancel = AtomicBool::new(false);
        let vocabulary = [Vocabulary {
            written: "Name".into(),
            enabled: false,
            ..Vocabulary::default()
        }];
        let document = Document::new("the. Name", &vocabulary, &cancel).unwrap();
        assert_eq!(document.rebuild(None, &cancel).unwrap(), "The name");
    }

    #[test]
    fn initials_and_abbreviations_keep_their_period_without_capitalizing() {
        let text = "ask D. A. about etc. this e.g. that i.e. another vs. something";
        assert_eq!(
            all_marks(text, Mark::None),
            "Ask D. A. about etc. this e.g. that i.e. another vs. something"
        );
        for text in [
            "etc. next",
            "U.S. next",
            "e.g. next",
            "i.e. next",
            "vs. next",
        ] {
            let output = all_marks(text, Mark::Period);
            assert!(output.contains(". next"), "{output}");
        }
        assert_eq!(rules("it was I. next"), "It was I. Next");
    }

    #[test]
    fn standalone_letter_can_end_a_sentence_without_changing_marks() {
        for mark in [Mark::Period, Mark::Question] {
            assert_eq!(
                rebuilt(
                    "R and D. but it works",
                    &[
                        Mark::None,
                        Mark::None,
                        mark,
                        Mark::None,
                        Mark::None,
                        Mark::None,
                    ]
                ),
                "R and D. But it works"
            );
        }
        for mark in [Mark::None, Mark::Comma, Mark::Semicolon] {
            assert_eq!(
                rebuilt(
                    "R and D. But it works",
                    &[
                        Mark::None,
                        Mark::None,
                        mark,
                        Mark::None,
                        Mark::None,
                        Mark::None,
                    ]
                ),
                "R and D. But it works"
            );
        }
        assert_eq!(rules("R and D. But it works"), "R and D. But it works");
        assert_eq!(rules("ask J. Smith about it"), "Ask J. Smith about it");
        assert_eq!(all_marks("ask J. A. Smith", Mark::None), "Ask J. A. Smith");
        assert_eq!(all_marks("ask J. Smith", Mark::None), "Ask J. Smith");
        assert_eq!(all_marks("D. next", Mark::None), "D. next");
        assert_eq!(all_marks("D. next", Mark::Period), "D. Next.");
    }

    #[test]
    fn quote_labels_stay_attached_to_their_words() {
        let text = "say, \"Yeah, we good, bro.\" You understand.";
        let marks = [
            Mark::Comma,
            Mark::Comma,
            Mark::None,
            Mark::Comma,
            Mark::Period,
            Mark::None,
            Mark::Question,
        ];
        assert_eq!(
            rebuilt(text, &marks),
            "Say, \"Yeah, we good, bro.\" You understand?"
        );
        let mut terminal = marks;
        terminal[2] = Mark::Period;
        assert_eq!(
            rebuilt(text, &terminal),
            "Say, \"Yeah, we. Good, bro.\" You understand?"
        );
    }

    #[test]
    fn code_and_numbers_remain_byte_exact_inside() {
        for literal in ["`x = foo_bar -- baz`", "123 456", "10 – 20"] {
            let text = format!("say {literal} now");
            let output = all_marks(&text, Mark::Question);
            assert!(output.contains(literal), "{output}");
        }
    }

    #[test]
    fn quoted_speech_is_repunctuated_and_sentence_starts_capitalize() {
        assert_eq!(
            all_marks("say \"hello there\" next", Mark::Period),
            "Say. \"Hello. There\". Next."
        );
        assert_eq!(
            all_marks("say \"hello there.\" next", Mark::Period),
            "Say. \"Hello. There.\" Next."
        );
        assert_eq!(
            all_marks("say \"the. Name — still... here\" now", Mark::None),
            "Say \"the name, still, here\" now"
        );
        assert_eq!(rules("he said. \"hello"), "He said. \"Hello");
        assert_eq!(rules("he said. “hello”"), "He said. “Hello”");
        for quote in ["'", "\"", "“", "‘"] {
            assert_eq!(
                rules(&format!("{quote}the. Feature")),
                format!("{quote}The feature")
            );
            assert!(literal_spans(&format!("{quote}the. Feature")).is_empty());
        }
    }

    #[test]
    fn apostrophes_stay_on_their_side_of_whitespace() {
        for apostrophe in ["'", "’"] {
            let text = format!(
                "give {apostrophe}em the boys{apostrophe} rock {apostrophe}n{apostrophe} roll"
            );
            assert_eq!(
                all_marks(&text, Mark::None),
                format!(
                    "Give {apostrophe}em the boys{apostrophe} rock {apostrophe}n{apostrophe} roll"
                )
            );
            assert_eq!(
                rebuilt(&format!("give {apostrophe}em"), &[Mark::Period, Mark::None]),
                format!("Give. {apostrophe}Em")
            );
            assert_eq!(
                rebuilt(
                    &format!("the boys{apostrophe} return"),
                    &[Mark::None, Mark::Comma, Mark::None]
                ),
                format!("The boys{apostrophe}, return")
            );
        }
    }

    #[test]
    fn vocabulary_regexes_are_reused_and_invalidated_by_the_active_names() {
        let cache = VocabularyCache::default();
        let cancel = AtomicBool::new(false);
        let mut vocabulary = vec![Vocabulary {
            heard: "sono jot".into(),
            written: "SonoJot".into(),
            enabled: true,
            ..Vocabulary::default()
        }];
        let first = cache.matchers(&vocabulary, &cancel).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &cache.matchers(&vocabulary, &cancel).unwrap()
        ));
        vocabulary[0].written = "NewName".into();
        let changed = cache.matchers(&vocabulary, &cancel).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first, &changed));
        assert!(changed.iter().any(|matcher| matcher.is_match("newname")));
        vocabulary[0].enabled = false;
        assert!(cache.matchers(&vocabulary, &cancel).unwrap().is_empty());
        cancel.store(true, Ordering::Release);
        assert!(matches!(
            cache.matchers(&vocabulary, &cancel),
            Err(Failure::Cancelled)
        ));
    }

    #[test]
    fn closing_brackets_survive_rewritten_gaps_and_terminal_marks() {
        assert_eq!(
            rebuilt("(hello.) world", &[Mark::Question, Mark::None]),
            "(Hello?) World"
        );
        assert_eq!(
            rebuilt("[hello] world", &[Mark::Semicolon, Mark::Question]),
            "[Hello]; world?"
        );
        assert_eq!(
            rebuilt("hello (world)", &[Mark::None, Mark::Period]),
            "Hello (world)."
        );
        assert_eq!(
            rebuilt("(hello ) world", &[Mark::None, Mark::Question]),
            "(Hello) world?"
        );
    }

    #[test]
    fn preserved_structured_candidates_are_not_rebuilt() {
        let text = "email sam at comet. Then wait";
        let cancel = AtomicBool::new(false);
        let spans = crate::structured_clean::preserved_candidate_spans(text, &|| false).unwrap();
        assert!(!spans.is_empty());
        let document = Document::new(text, &[], &cancel).unwrap();
        let output = document
            .rebuild(Some(&vec![Mark::Question; document.words.len()]), &cancel)
            .unwrap();
        for (start, end) in spans {
            assert!(output.contains(&text[start..end]), "{output}");
        }
        assert!(same_words(text, &output));
    }

    #[test]
    fn final_word_respects_predicted_terminal_or_existing_suffix() {
        for (mark, expected) in [
            (Mark::Period, "Hello."),
            (Mark::Question, "Hello?"),
            (Mark::None, "Hello!"),
            (Mark::Comma, "Hello!"),
            (Mark::Semicolon, "Hello!"),
        ] {
            assert_eq!(rebuilt("hello!", &[mark]), expected);
        }
        assert_eq!(rebuilt("hello", &[Mark::None]), "Hello");
        assert_eq!(rebuilt("hello", &[Mark::Question]), "Hello?");
        assert_eq!(rebuilt("etc.", &[Mark::Question]), "Etc?");
    }

    #[test]
    fn fallback_removes_only_periods_after_the_exact_function_words() {
        for word in [
            "the", "a", "an", "your", "my", "our", "their", "its", "every", "and", "or", "but",
            "because", "if", "than", "whether", "nor", "of", "onto",
        ] {
            let text = format!("use {word}. Something");
            assert_eq!(rules(&text), format!("Use {word} something"));
            for end in ['?', '!'] {
                assert_eq!(
                    rules(&format!("use {word}{end} Something")),
                    format!("Use {word}{end} Something")
                );
            }
        }
        assert_eq!(
            rules("the. NASA and. iPhone of. Alice knows Alice"),
            "The NASA and iPhone of Alice knows Alice"
        );
        assert_eq!(rules("that. Next"), "That. Next");
        assert_eq!(rules("the."), "The.");
        // The final URL/email component is not a standalone function word.
        assert_eq!(
            rules("visit https://example.com/a. Next send it to sam@my. Then wait."),
            "Visit https://example.com/a. Next send it to sam@my. Then wait."
        );
    }

    #[test]
    fn malformed_model_output_and_load_failure_return_rules() {
        let cancel = AtomicBool::new(false);
        let text = "the. Feature";
        let document = Document::new(text, &[], &cancel).unwrap();
        for prediction in [
            Err(Failure::Load),
            Err(Failure::Inference),
            Err(Failure::Output),
            Ok(vec![]),
            Ok(vec![Mark::None]),
        ] {
            let (output, stage, reason) = finish(text, Some(&document), prediction, &cancel);
            assert_eq!(output, "The feature");
            assert_eq!(stage, "punctuation_rules");
            assert!(reason.is_some());
        }
    }

    #[test]
    fn cancellation_rolls_back_to_the_complete_input() {
        let cancel = AtomicBool::new(false);
        let text = "the. Feature";
        let document = Document::new(text, &[], &cancel).unwrap();
        cancel.store(true, Ordering::Release);
        assert!(matches!(
            Document::new(text, &[], &cancel),
            Err(Failure::Cancelled)
        ));
        assert_eq!(document.rebuild(None, &cancel), Err(Failure::Cancelled));
        let (output, stage, reason) =
            finish(text, Some(&document), Ok(vec![Mark::None; 2]), &cancel);
        assert_eq!(output, text);
        assert_eq!(stage, "punctuation_rules");
        assert_eq!(reason, Some(Failure::Cancelled));
    }

    #[test]
    fn word_invariant_detects_additions_losses_changes_and_reordering() {
        assert!(same_words("Hello, i'm here", "hello. I'm here!"));
        for output in [
            "Hello here",
            "Hello I'm right here",
            "Here I'm hello",
            "Hello I'm there",
        ] {
            assert!(!same_words("Hello I'm here", output));
        }
        for text in [
            "",
            "  ",
            "...",
            "ß comes. Él waits",
            "don't don’t 1.2 café",
            "a e\u{301}x foo_bar a\u{200d}b",
        ] {
            for mark in [Mark::None, Mark::Period, Mark::Question, Mark::Semicolon] {
                all_marks(text, mark);
            }
            rules(text);
        }
    }

    #[test]
    fn config_validates_encoding_ids_window_and_label_map() {
        let valid = config();
        assert!(valid.span_labels.is_none());
        assert_eq!(
            valid.labels,
            [
                Mark::None,
                Mark::Period,
                Mark::Comma,
                Mark::Question,
                Mark::Comma,
                Mark::Comma,
                Mark::Semicolon
            ]
        );
        let mut good = config_json();
        good["encoding"] = serde_json::json!(LEGACY_ENCODING);
        assert!(Config::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
        good["encoding"] = serde_json::json!(ENCODING);
        good["recognizer_marks"] = serde_json::json!({});
        for (key, bad) in [
            ("schema", serde_json::json!(2)),
            ("encoding", serde_json::json!("first-subtoken")),
            ("window_words", serde_json::json!(0)),
            ("window_words", serde_json::json!(100_000)),
            ("hop_words", serde_json::json!(0)),
            ("hop_words", serde_json::json!(121)),
            ("cls_id", serde_json::json!(-1)),
            ("sep_id", serde_json::json!(2)),
            ("word_prefix", serde_json::json!("")),
            ("labels", serde_json::json!(["missing"])),
            ("labels", serde_json::json!(["0", "0"])),
            ("labels", serde_json::json!([])),
            ("labels", serde_json::json!({"1":"0"})),
            ("lowercase_words_except", serde_json::json!(["I"])),
            ("recognizer_marks", serde_json::json!({".":[]})),
            ("label_meaning", serde_json::json!({"0":"rewrite-words"})),
        ] {
            let mut json = good.clone();
            json[key] = bad;
            assert!(
                Config::parse(&serde_json::to_vec(&json).unwrap()).is_err(),
                "{key}"
            );
        }
        good["labels"] = serde_json::json!({"0":"0","1":".","2":";"});
        assert_eq!(
            Config::parse(&serde_json::to_vec(&good).unwrap())
                .unwrap()
                .labels,
            [Mark::None, Mark::Period, Mark::Semicolon]
        );
    }

    #[test]
    fn span_config_preserves_array_and_indexed_label_order() {
        use crate::written_form::{BioLabel, Class};
        let mut json = span_config_json();
        let parsed = Config::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        let labels = parsed.span_labels.unwrap();
        assert_eq!(labels.len(), 27);
        assert_eq!(
            &labels[..3],
            &[
                BioLabel::O,
                BioLabel::B(Class::Cardinal),
                BioLabel::I(Class::Cardinal)
            ]
        );
        assert_eq!(parsed.labels, config().labels);
        let raw = json["span_labels"].as_array_mut().unwrap();
        raw.rotate_left(1);
        let indexed: serde_json::Map<_, _> = raw
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, label)| (index.to_string(), label))
            .collect();
        json["span_labels"] = serde_json::Value::Object(indexed);
        let parsed = Config::parse(&serde_json::to_vec(&json).unwrap()).unwrap();
        let mut rotated = labels;
        rotated.rotate_left(1);
        assert_eq!(parsed.span_labels.unwrap(), rotated);
    }

    #[test]
    fn malformed_span_config_is_not_treated_as_a_legacy_package() {
        let good = span_config_json();
        for (key, value) in [
            ("span_labels", serde_json::json!(null)),
            ("span_labels", serde_json::json!([])),
            ("span_labels", serde_json::json!(["O"])),
            ("span_labels", serde_json::json!({"1":"O"})),
            ("span_label_scheme", serde_json::json!("BILOU")),
            ("span_output", serde_json::json!("logits")),
            ("punct_output", serde_json::json!("span_logits")),
            ("span_encoding", serde_json::json!("first subword")),
            ("span_classes", serde_json::json!([])),
            ("span_classes", serde_json::json!(["UNKNOWN"])),
        ] {
            let mut json = good.clone();
            json[key] = value;
            assert!(
                Config::parse(&serde_json::to_vec(&json).unwrap()).is_err(),
                "{key}"
            );
        }
        for replacement in ["O", "I-CARDINAL", "B-UNKNOWN", "b-CARDINAL"] {
            let mut json = good.clone();
            json["span_labels"][1] = serde_json::json!(replacement);
            assert!(Config::parse(&serde_json::to_vec(&json).unwrap()).is_err());
        }
        let mut duplicate_class = good.clone();
        duplicate_class["span_classes"][1] = duplicate_class["span_classes"][0].clone();
        assert!(Config::parse(&serde_json::to_vec(&duplicate_class).unwrap()).is_err());
        for key in [
            "span_label_scheme",
            "span_output",
            "span_encoding",
            "span_classes",
        ] {
            let mut json = config_json();
            json[key] = good[key].clone();
            assert!(
                Config::parse(&serde_json::to_vec(&json).unwrap()).is_err(),
                "{key}"
            );
            let mut json = good.clone();
            json[key] = serde_json::Value::Null;
            assert!(
                Config::parse(&serde_json::to_vec(&json).unwrap()).is_err(),
                "{key}"
            );
        }
        let mut no_outside = good;
        no_outside["span_labels"][0] = serde_json::json!("B-MONEY");
        assert!(Config::parse(&serde_json::to_vec(&no_outside).unwrap()).is_err());
    }

    #[test]
    fn labels_are_read_at_the_last_word_subtoken_before_cue_tokens() {
        let config = config();
        let pieces = [(vec![10, 11], vec![954]), (vec![12], vec![])];
        let (ids, positions) = encode_pieces(&pieces, &config).unwrap();
        assert_eq!(ids, [2, 10, 11, 954, 12, 1]);
        assert_eq!(positions, [2, 4]);
        let labels = [Mark::None, Mark::Question];
        let mut logits = vec![0.0; ids.len() * 2];
        logits[positions[0] * 2 + 1] = 2.0;
        logits[3 * 2] = 100.0; // Cue token does not own a label.
        assert_eq!(
            decode_logits(&[1, 6, 2], &logits, 6, &positions, &labels).unwrap(),
            [Mark::Question, Mark::None]
        );
        use crate::written_form::{BioLabel, Class};
        let spans = [BioLabel::O, BioLabel::B(Class::Money)];
        assert_eq!(
            decode_logits(&[1, 6, 2], &logits, 6, &positions, &spans).unwrap(),
            [BioLabel::B(Class::Money), BioLabel::O]
        );
        assert!(encode_pieces(&[(vec![], vec![])], &config).is_err());
    }

    #[test]
    fn malformed_logits_never_index_or_publish_a_partial_result() {
        let labels = [Mark::None, Mark::Period];
        for (shape, logits, positions) in [
            (vec![4, 2], vec![0.0; 8], vec![1]),
            (vec![1, 4, 3], vec![0.0; 12], vec![1]),
            (vec![1, 4, 2], vec![0.0; 7], vec![1]),
            (vec![1, 4, 2], vec![f32::NAN; 8], vec![1]),
            (vec![1, 4, 2], vec![f32::INFINITY; 8], vec![1]),
            (vec![1, 4, 2], vec![0.0; 8], vec![0]),
            (vec![1, 4, 2], vec![0.0; 8], vec![3]),
            (vec![1, 4, 2], vec![0.0; 8], vec![4]),
            (vec![1, 4, 2], vec![0.0; 8], vec![1, 1]),
            (vec![1, 4, 2], vec![0.0; 8], vec![2, 1]),
        ] {
            assert_eq!(
                decode_logits(&shape, &logits, 4, &positions, &labels),
                Err(Failure::Output)
            );
            use crate::written_form::{BioLabel, Class};
            assert_eq!(
                decode_logits(
                    &shape,
                    &logits,
                    4,
                    &positions,
                    &[BioLabel::O, BioLabel::B(Class::Money)]
                ),
                Err(Failure::Output)
            );
        }
    }

    #[test]
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    fn head_tensor_type_requires_float_batch_sequence_and_matching_classes() {
        use ort::value::{Shape, SymbolicDimensions, TensorElementType, ValueType};
        let tensor = |ty, shape: &[i64]| ValueType::Tensor {
            ty,
            shape: Shape::new(shape.iter().copied()),
            dimension_symbols: SymbolicDimensions::new(vec![String::new(); shape.len()]),
        };
        for classes in [5, 27] {
            assert!(validate_head_type(
                &tensor(TensorElementType::Float32, &[-1, -1, classes]),
                classes as usize
            )
            .is_ok());
            assert!(validate_head_type(
                &tensor(TensorElementType::Float32, &[1, 4, classes]),
                classes as usize
            )
            .is_ok());
            for shape in [
                vec![1, 4],
                vec![2, 4, classes],
                vec![1, 0, classes],
                vec![1, -2, classes],
                vec![1, 4, classes + 1],
            ] {
                assert_eq!(
                    validate_head_type(
                        &tensor(TensorElementType::Float32, &shape),
                        classes as usize
                    ),
                    Err(Failure::Load)
                );
            }
            assert_eq!(
                validate_head_type(
                    &tensor(TensorElementType::Int64, &[1, 4, classes]),
                    classes as usize
                ),
                Err(Failure::Load)
            );
        }
    }

    #[test]
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    fn graph_heads_must_match_the_optional_span_config() {
        use ort::value::{Shape, SymbolicDimensions, TensorElementType, ValueType};
        let tensor = |classes| ValueType::Tensor {
            ty: TensorElementType::Float32,
            shape: Shape::new([-1, -1, classes]),
            dimension_symbols: SymbolicDimensions::new(vec![String::new(); 3]),
        };
        let punct = tensor(5);
        let spans = tensor(27);
        assert!(validate_head_outputs([("logits", &punct)], 5, None).is_ok());
        assert!(
            validate_head_outputs([("span_logits", &spans), ("logits", &punct)], 5, Some(27))
                .is_ok()
        );
        for (outputs, span_classes) in [
            (vec![], None),
            (vec![("logits", &punct)], Some(27)),
            (vec![("span_logits", &spans)], Some(27)),
            (vec![("logits", &punct), ("span_logits", &spans)], None),
            (vec![("logits", &punct), ("span_logits", &punct)], Some(27)),
            (vec![("logits", &punct), ("logits", &punct)], None),
            (vec![("logits", &punct), ("unknown", &spans)], Some(27)),
        ] {
            assert_eq!(
                validate_head_outputs(outputs, 5, span_classes),
                Err(Failure::Load)
            );
        }
    }

    #[test]
    fn overlapping_windows_cover_long_text_once_and_keep_interior_predictions() {
        assert_eq!(windows(0, 120, 60), []);
        assert_eq!(windows(120, 120, 60), [(0, 120)]);
        assert_eq!(windows(121, 120, 60), [(0, 120), (60, 121)]);
        assert_eq!(
            windows(301, 120, 60),
            [(0, 120), (60, 180), (120, 240), (180, 300), (240, 301)]
        );
        let mut output = vec![None; 301];
        let mut best = vec![None; 301];
        for (index, (start, end)) in windows(301, 120, 60).into_iter().enumerate() {
            merge_window(
                &mut output,
                &mut best,
                start,
                &vec![
                    if index == 0 {
                        Mark::Period
                    } else {
                        Mark::Comma
                    };
                    end - start
                ],
            )
            .unwrap();
        }
        assert!(output.iter().all(Option::is_some));
        assert_eq!(output[60], Some(Mark::Period));
        assert_eq!(output[100], Some(Mark::Comma));
        let mut tie_output = vec![None; 3];
        let mut tie_best = vec![None; 3];
        merge_window(&mut tie_output, &mut tie_best, 0, &[Mark::Period; 3]).unwrap();
        merge_window(&mut tie_output, &mut tie_best, 0, &[Mark::Comma; 3]).unwrap();
        assert_eq!(tie_output, vec![Some(Mark::Period); 3]);
    }

    #[test]
    fn both_heads_share_window_seams_and_keep_earlier_windows_on_ties() {
        use crate::written_form::{BioLabel, Class};
        for count in [121, 180, 181, 300, 301] {
            let mut output = vec![None; count];
            let mut best = vec![None; count];
            let first = (Mark::Period, Some(BioLabel::B(Class::Money)));
            let later = (Mark::Comma, Some(BioLabel::I(Class::Money)));
            for (index, (start, end)) in windows(count, 120, 60).into_iter().enumerate() {
                merge_window(
                    &mut output,
                    &mut best,
                    start,
                    &vec![if index == 0 { first } else { later }; end - start],
                )
                .unwrap();
            }
            assert!(output.iter().all(Option::is_some));
            assert_eq!(output[89], Some(first));
            assert_eq!(output[90], Some(later));
            assert_eq!(output[100], Some(later));
        }
        let first = (Mark::Question, Some(BioLabel::B(Class::Money)));
        let later = (Mark::Comma, Some(BioLabel::B(Class::Percent)));
        let mut seam_output = vec![None; 7];
        let mut seam_best = vec![None; 7];
        merge_window(&mut seam_output, &mut seam_best, 0, &[first; 5]).unwrap();
        merge_window(&mut seam_output, &mut seam_best, 2, &[later; 5]).unwrap();
        assert_eq!(seam_output[3], Some(first)); // Equal distance from each window edge.
        assert_eq!(seam_output[4], Some(later));
        let mut output = vec![None; 3];
        let mut best = vec![None; 3];
        merge_window(&mut output, &mut best, 0, &[first; 3]).unwrap();
        merge_window(&mut output, &mut best, 0, &[later; 3]).unwrap();
        assert_eq!(output, vec![Some(first); 3]);
        assert_eq!(
            merge_window(&mut output, &mut best, 2, &[later; 2]),
            Err(Failure::Output)
        );
        assert_eq!(
            merge_window(&mut output, &mut best, usize::MAX, &[later]),
            Err(Failure::Output)
        );
        assert_eq!(
            merge_window(&mut output, &mut best[..2], 0, &[later]),
            Err(Failure::Output)
        );
        assert_eq!(output, vec![Some(first); 3]);
    }

    #[test]
    fn retention_deadlines() {
        assert!(expired(Duration::ZERO, ModelRetention::Immediately));
        assert!(!expired(
            Duration::from_secs(599),
            ModelRetention::Minutes10
        ));
        assert!(expired(Duration::from_secs(600), ModelRetention::Minutes10));
        assert!(!expired(
            Duration::from_secs(1799),
            ModelRetention::Minutes30
        ));
        assert!(expired(
            Duration::from_secs(1800),
            ModelRetention::Minutes30
        ));
        assert!(!expired(Duration::MAX, ModelRetention::OnExit));
    }

    #[test]
    fn selection_tracks_root_override_and_package_changes_without_environment_mutation() {
        let root =
            std::env::temp_dir().join(format!("sonojot-punct-selection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let first = Selection::new(&root, Some(root.clone()));
        assert!(first.unchanged());
        assert_ne!(
            first,
            Selection::new(&root.join("other"), Some(root.clone()))
        );
        assert_ne!(first, Selection::new(&root, None));
        std::fs::write(root.join("punct-config.json"), b"{}").unwrap();
        assert!(!first.unchanged());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    fn real_standin_when_environment_is_set() {
        let Some(directory) = std::env::var_os("SONOJOT_PUNCT_MODEL_DIR") else {
            return;
        };
        // Explicit isolated root avoids reading/writing installed user data.
        let root = std::env::var_os("SONOJOT_PUNCT_TEST_MODELS_ROOT")
            .expect("Set SONOJOT_PUNCT_TEST_MODELS_ROOT to an isolated models root containing the shared pinned runtime.");
        let _gate = crate::resource_manager::tests::test_gate();
        let selection = Selection::new(Path::new(&root), Some(PathBuf::from(directory)));
        let cancel = AtomicBool::new(false);
        assert!(matches!(
            Engine::load(&selection, &cancel),
            Err(Failure::Runtime)
        ));
        let text =
            "the problem is that. I need the. Feature and I want to use it today ".repeat(21);
        let document = Document::new(&text, &[], &cancel).unwrap();
        assert!(document.words.len() > 300);
        let labels =
            crate::punctuation_worker::predict(&selection, &text, document.words.len(), &cancel)
                .expect("all stand-in windows must run").labels;
        assert_eq!(labels.len(), document.words.len());
        let (output, stage, reason) = finish(&text, Some(&document), Ok(labels), &cancel);
        assert_eq!(stage, "punctuation_model");
        assert_eq!(reason, None);
        assert!(same_words(&text, &output));
        cancel.store(true, Ordering::Release);
        assert_eq!(
            crate::punctuation_worker::predict(&selection, &text, document.words.len(), &cancel),
            Err(Failure::Cancelled)
        );
        assert!(!release_idle(ModelRetention::OnExit));
        assert!(release_idle(ModelRetention::Immediately));
        assert!(!release_cached());
    }
}
