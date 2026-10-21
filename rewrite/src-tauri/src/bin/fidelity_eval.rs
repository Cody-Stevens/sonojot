//! Opt-in, local file/text replay through the application's recognition and
//! cleanup owners. No Tauri application, microphone, clipboard or Store opens.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sonojot_core::{
    asr, audio::AudioMessage, cleanup, domain::RecognitionCompleteness, recognition, recording,
};
use sonojot_core::punctuation::WrittenFormMode;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const HELP: &str = r#"fidelity_eval --input cases.jsonl --output results.jsonl --models-root DIRECTORY
  --written-form-replay text|span  Standalone written-form stage; no audio
                          Text rows: {"id":"case","text":"spoken text","labels":["O",...]}
                          Model replay accepts saved whitespace BIO labels, or --models-root
                          with optional --punctuation-model-dir for live inference
                          Span rows also require class (or cls), optional before/after
  --written-form rules|off|model Written-form choice; default rules; model uses ROUTED_STRICT
                          Production host/model/daily dictation respects cleanup mode
  --prepare-capture cases.jsonl --prepared-out DIRECTORY
                          Separate offline capture-policy replay; no model loads
  --asr-model ID           Required for rows without input_text; WAV input only
  --language auto|en      Evaluation-only Metal language choice; default auto
  --max-chunk-seconds 20|28  Evaluation-only Metal buffer limit; default 20
  --replay-mode file|stream  File throughput (default), or production stream replay
  --pace realtime|unpaced   Stream arrival schedule; default realtime
  --callback-ms N           Stream PCM callback duration; default 20, range 1..1000
  --cleanup-model ID       Cleanup model; default off (alias: --model)
  --punctuation-model-dir DIR  Local punctuation package for production text replay
  --punctuation-labels true|false  Dump raw per-word labels instead of cleanup (parity)
  --punctuation-prewarm true|false  Load punctuation before timed rows; report DLL evidence
  --smart-layout true|false  Text-only local rules then layout; default false
                          Requires a generation --cleanup-model; holds a text lease
                          Defaults to --variant model --cleanup-policy conservative
                          Rows may supply app_context (single-line app description)
                          Installed model verification receipts remain read-only
  --meeting-rules-only true|false  Route scope:meeting rows to local rules
  --resource-lease true|false  Exercise production text admission and lease hooks
  --cancel-after-ms N      Cancel cleanup after N ms; row override also accepted
  --variant VARIANT        raw (default), host, model, baseline, structured, daily
  --cleanup-policy POLICY  daily (default), trial, legacy, targeted, conservative; model/host
  --trace-cleanup true|false  Daily/targeted model diagnostics; extra overhead, default false
  --cleanup-input-envelope plain|json  Evaluation-only daily input framing; default plain
  --daily-clean-prompt-file PATH  Clean-only override; requires daily model diagnostics
  --repeat N               Measured passes over the complete input; default 1
  --warmup N               Unmeasured passes recorded with measured:false; default 0
  --cold-asr true|false    Reload ASR for every audio row; default false
  --stop-after-ms N        Stop cleanup after N ms; row override also accepted
  --prompt-file PATH       Experimental structured route only
  --preserve-input-punctuation true|false      Experimental baseline/structured
  --preserve-terminal-punctuation true|false   Experimental baseline/structured
  --force-rejection true|false                Experimental baseline/structured

Each JSONL row: {"id":"clip","mode":"as_heard","audio_path":"clip.wav"}
Text replay: {"id":"clip","mode":"clean","input_text":"raw recognition"}
Modes: as_heard, clean, format_only. Relative audio paths resolve against the
input manifest directory. input_text takes precedence for frozen-ASR replay.
References are never passed to recognition or cleanup. Output must not exist.
Every row is flushed; errors produce JSON evidence and a nonzero final exit.
File ASR time includes WAV read, production resampling, worker IPC and decoding.
Stream replay uses independently scheduled PCM and the production audio backlog.
Only realtime replay reports final-audio-to-text delay. Neither route measures
microphone hardware, UI publication or cleanup prewarming. Resource admission is
excluded unless --resource-lease true or --smart-layout true is selected.
"#;

#[derive(Debug)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    root: PathBuf,
    asr_model: Option<String>,
    language: String,
    max_chunk_seconds: usize,
    replay_mode: String,
    pace: String,
    callback_ms: u64,
    model: String,
    punctuation_model_dir: Option<PathBuf>,
    punctuation_labels: bool,
    punctuation_prewarm: bool,
    written_form: WrittenFormMode,
    smart_layout: bool,
    meeting_rules_only: bool,
    resource_lease: bool,
    cancel_after_ms: Option<u64>,
    variant: String,
    policy: String,
    trace_cleanup: bool,
    cleanup_input_envelope: String,
    daily_clean_prompt: Option<String>,
    daily_clean_prompt_path: Option<PathBuf>,
    repeat: usize,
    warmup: usize,
    cold_asr: bool,
    stop_after_ms: Option<u64>,
    prompt: Option<String>,
    options: cleanup::EvaluationOptions,
}

fn take_required(values: &mut HashMap<String, String>, flag: &str) -> Result<String, String> {
    values
        .remove(flag)
        .ok_or_else(|| format!("Missing {flag}."))
}

fn take_number<T: std::str::FromStr>(
    values: &mut HashMap<String, String>,
    flag: &str,
    default: T,
) -> Result<T, String> {
    values.remove(flag).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("Invalid {flag}: {value}"))
    })
}

fn take_bool(values: &mut HashMap<String, String>, flag: &str) -> Result<bool, String> {
    values
        .remove(flag)
        .map_or(Ok(false), |value| match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("{flag} requires true or false.")),
        })
}

fn take_written_form(values: &mut HashMap<String, String>) -> Result<WrittenFormMode, String> {
    match values.remove("--written-form").as_deref().unwrap_or("rules") {
        "rules" => Ok(WrittenFormMode::Rules),
        "off" => Ok(WrittenFormMode::Off),
        "model" => Ok(WrittenFormMode::Model),
        _ => Err("--written-form requires rules, off or model.".into()),
    }
}

fn argument_values(arguments: impl IntoIterator<Item = String>) -> Result<HashMap<String, String>, String> {
    let mut arguments = arguments.into_iter();
    let mut values = HashMap::new();
    while let Some(flag) = arguments.next() {
        if !flag.starts_with("--") {
            return Err("Expected a --flag.".into());
        }
        let value = arguments.next().ok_or_else(|| format!("Missing value for {flag}."))?;
        if values.insert(flag.clone(), value).is_some() {
            return Err(format!("Duplicate argument: {flag}"));
        }
    }
    Ok(values)
}

impl Args {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut values = argument_values(arguments)?;
        let written_form_explicit = values.contains_key("--written-form");
        let written_form = take_written_form(&mut values)?;
        let input = PathBuf::from(take_required(&mut values, "--input")?);
        let output = PathBuf::from(take_required(&mut values, "--output")?);
        let root = PathBuf::from(take_required(&mut values, "--models-root")?);
        let asr_model = values.remove("--asr-model");
        let language = values.remove("--language").unwrap_or_else(|| "auto".into());
        if !matches!(language.as_str(), "auto" | "en") {
            return Err("--language requires auto or en.".into());
        }
        // Metal, sherpa Whisper and llama.cpp Qwen read these experiment
        // controls; other recognizers would silently ignore them.
        let experimental = |id: &String| id.starts_with("whisper-")
            || matches!(id.as_str(), "qwen3-asr-1.7b-q8-cuda" | "qwen3-asr-1.7b-q4-cpu"
                | "qwen3-asr-1.7b-q8-metal" | "qwen3-asr-1.7b-q4-metal");
        if language != "auto" && asr_model.as_ref().is_some_and(|id| !experimental(id)) {
            return Err("Explicit --language is supported only by Whisper and llama.cpp Qwen ASR models.".into());
        }
        let max_chunk_seconds = take_number(&mut values, "--max-chunk-seconds", 20usize)?;
        if !matches!(max_chunk_seconds, 20 | 28) {
            return Err("--max-chunk-seconds requires 20 or 28.".into());
        }
        if max_chunk_seconds != 20 && asr_model.as_ref().is_some_and(|id| !experimental(id)) {
            return Err("Experimental chunk limits are supported only by Whisper and llama.cpp Qwen ASR models.".into());
        }
        let replay_mode = values
            .remove("--replay-mode")
            .unwrap_or_else(|| "file".into());
        if !matches!(replay_mode.as_str(), "file" | "stream") {
            return Err("--replay-mode requires file or stream.".into());
        }
        let pace = values.remove("--pace").unwrap_or_else(|| "realtime".into());
        if !matches!(pace.as_str(), "realtime" | "unpaced") {
            return Err("--pace requires realtime or unpaced.".into());
        }
        let callback_ms = take_number(&mut values, "--callback-ms", 20u64)?;
        if !(1..=1000).contains(&callback_ms) {
            return Err("--callback-ms must be between 1 and 1000.".into());
        }
        let old_model = values.remove("--model");
        let cleanup_model = values.remove("--cleanup-model");
        if old_model.is_some() && cleanup_model.is_some() {
            return Err("Use either --model or --cleanup-model, not both.".into());
        }
        let model = cleanup_model.or(old_model).unwrap_or_else(|| "off".into());
        let punctuation_model_dir = values.remove("--punctuation-model-dir").map(PathBuf::from);
        let punctuation_labels = take_bool(&mut values, "--punctuation-labels")?;
        let punctuation_prewarm = take_bool(&mut values, "--punctuation-prewarm")?;
        let smart_layout = take_bool(&mut values, "--smart-layout")?;
        if smart_layout && cleanup::layout::select_model(&model, "off").is_none() {
            return Err("--smart-layout true requires a generation --cleanup-model (not off, local-rules or SpeakoFlow).".into());
        }
        let meeting_rules_only = take_bool(&mut values, "--meeting-rules-only")?;
        if smart_layout && values.get("--resource-lease").is_some_and(|value| value == "false") {
            return Err("Smart layout requires its production text resource lease.".into());
        }
        let resource_lease = take_bool(&mut values, "--resource-lease")? || smart_layout;
        let cancel_after_ms = values.remove("--cancel-after-ms")
            .map(|value| value.parse::<u64>().map_err(|_| "Invalid --cancel-after-ms.".to_owned()))
            .transpose()?;
        let variant = values.remove("--variant")
            .unwrap_or_else(|| if smart_layout { "model" } else { "raw" }.into());
        if !matches!(
            variant.as_str(),
            "raw" | "host" | "model" | "baseline" | "structured" | "daily"
        ) {
            return Err(format!("Unsupported variant: {variant}"));
        }
        let policy = values
            .remove("--cleanup-policy")
            .unwrap_or_else(|| if smart_layout { "conservative" } else { "daily" }.into());
        if !matches!(policy.as_str(), "daily" | "trial" | "legacy" | "targeted" | "conservative") {
            return Err(format!("Unsupported cleanup policy: {policy}"));
        }
        if matches!(policy.as_str(), "targeted" | "conservative")
            && !matches!(variant.as_str(), "host" | "model")
        {
            return Err("The targeted/conservative policies require variant host or model.".into());
        }
        if variant == "daily" && policy != "daily" {
            return Err("The daily variant requires the daily policy.".into());
        }
        let trace_cleanup = take_bool(&mut values, "--trace-cleanup")?;
        if written_form_explicit && (trace_cleanup || smart_layout || punctuation_labels || !matches!(variant.as_str(), "host" | "model" | "daily")) {
            return Err("--written-form requires untraced host/model/daily dictation, or --written-form-replay text|span.".into());
        }
        if (punctuation_model_dir.is_some() || punctuation_labels || punctuation_prewarm)
            && (trace_cleanup || !matches!(variant.as_str(), "host" | "model"))
        {
            return Err("--punctuation-model-dir requires the untraced production host/model route.".into());
        }
        if smart_layout && (variant != "model" || policy != "conservative" || trace_cleanup || meeting_rules_only) {
            return Err("Smart layout requires the untraced model variant with conservative local rules and dictation rows.".into());
        }
        if (meeting_rules_only || resource_lease)
            && (trace_cleanup || !matches!(variant.as_str(), "host" | "model"))
        {
            return Err("Meeting routing and resource leases require the untraced production host/model route.".into());
        }
        if trace_cleanup
            && (!matches!(variant.as_str(), "model" | "daily")
                || !matches!(policy.as_str(), "daily" | "targeted"))
        {
            return Err(
                "--trace-cleanup requires variant model/daily and the daily or targeted policy.".into(),
            );
        }
        let cleanup_input_envelope = values
            .remove("--cleanup-input-envelope")
            .unwrap_or_else(|| "plain".into());
        if !matches!(cleanup_input_envelope.as_str(), "plain" | "json") {
            return Err("--cleanup-input-envelope requires plain or json.".into());
        }
        if cleanup_input_envelope != "plain"
            && (!matches!(variant.as_str(), "model" | "daily") || policy != "daily")
        {
            return Err(
                "JSON input envelopes require variant model/daily and the daily policy.".into(),
            );
        }
        let daily_clean_prompt_path = values
            .remove("--daily-clean-prompt-file")
            .map(PathBuf::from);
        if daily_clean_prompt_path.is_some()
            && (!trace_cleanup
                || !matches!(variant.as_str(), "model" | "daily")
                || policy != "daily")
        {
            return Err("--daily-clean-prompt-file requires --trace-cleanup true, variant model/daily and the daily policy.".into());
        }
        let daily_clean_prompt = daily_clean_prompt_path
            .as_ref()
            .map(fs::read_to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        if daily_clean_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.trim().is_empty() || prompt.len() > 8000)
        {
            return Err(
                "Daily Clean prompt must contain 1..8000 UTF-8 bytes of nonempty text.".into(),
            );
        }
        let repeat = take_number(&mut values, "--repeat", 1usize)?;
        let warmup = take_number(&mut values, "--warmup", 0usize)?;
        if repeat == 0 {
            return Err("--repeat must be positive.".into());
        }
        let cold_asr = take_bool(&mut values, "--cold-asr")?;
        if smart_layout && (asr_model.is_some() || replay_mode != "file" || cold_asr
            || language != "auto" || max_chunk_seconds != 20 || pace != "realtime" || callback_ms != 20)
        {
            return Err("Smart layout supports frozen text replay only; ASR and stream controls are unsupported.".into());
        }
        let stop_after_ms = values
            .remove("--stop-after-ms")
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| "Invalid --stop-after-ms.".to_owned())
            })
            .transpose()?;
        let prompt_path = values.remove("--prompt-file");
        if prompt_path.is_some() && variant != "structured" {
            return Err(
                "--prompt-file is supported only by the experimental structured variant.".into(),
            );
        }
        let prompt = prompt_path
            .map(fs::read_to_string)
            .transpose()
            .map_err(|error| error.to_string())?;
        let options = cleanup::EvaluationOptions {
            preserve_input_punctuation: take_bool(&mut values, "--preserve-input-punctuation")?,
            preserve_terminal_punctuation: take_bool(
                &mut values,
                "--preserve-terminal-punctuation",
            )?,
            force_rejection: take_bool(&mut values, "--force-rejection")?,
        };
        if options != cleanup::EvaluationOptions::default()
            && !matches!(variant.as_str(), "baseline" | "structured")
        {
            return Err("Evaluation switches require baseline or structured; production policies are fixed.".into());
        }
        if let Some(flag) = values.keys().next() {
            return Err(format!("Unknown argument: {flag}"));
        }
        Ok(Self {
            input,
            output,
            root,
            asr_model,
            language,
            max_chunk_seconds,
            replay_mode,
            pace,
            callback_ms,
            model,
            punctuation_model_dir,
            punctuation_labels,
            punctuation_prewarm,
            written_form,
            smart_layout,
            meeting_rules_only,
            resource_lease,
            cancel_after_ms,
            variant,
            policy,
            trace_cleanup,
            cleanup_input_envelope,
            daily_clean_prompt,
            daily_clean_prompt_path,
            repeat,
            warmup,
            cold_asr,
            stop_after_ms,
            prompt,
            options,
        })
    }

    fn cleanup_policy(&self) -> cleanup::CleanupPolicy {
        match self.policy.as_str() {
            "legacy" => cleanup::CleanupPolicy::legacy(),
            "trial" => cleanup::CleanupPolicy::trial(),
            "targeted" => cleanup::CleanupPolicy::targeted(),
            "conservative" => cleanup::CleanupPolicy::conservative(),
            _ => cleanup::CleanupPolicy::daily(),
        }
    }
}

#[derive(Deserialize)]
struct Case {
    id: String,
    mode: String,
    input_text: Option<String>,
    audio_path: Option<PathBuf>,
    #[serde(default)]
    vocabulary: Vec<cleanup::Vocabulary>,
    #[serde(default)]
    confirmed_vocabulary: Vec<sonojot_core::structured_clean::VocabularyEntry>,
    active_scope: Option<String>,
    stop_after_ms: Option<u64>,
    cancel_after_ms: Option<u64>,
    scope: Option<String>,
    app_context: Option<String>,
}

fn mode(value: &str) -> Result<cleanup::Mode, String> {
    match value {
        "as_heard" => Ok(cleanup::Mode::Original),
        "clean" => Ok(cleanup::Mode::Clean),
        "format_only" => Ok(cleanup::Mode::Format),
        _ => Err(format!("Unknown mode: {value}")),
    }
}

fn read_cases(path: &Path) -> Result<Vec<(Case, Value)>, String> {
    let reader = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut cases = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(&line).map_err(|error| format!("Line {}: {error}", index + 1))?;
        let case: Case = serde_json::from_value(value.clone())
            .map_err(|error| format!("Line {}: {error}", index + 1))?;
        if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
            return Err(format!("Line {}: empty or duplicate id.", index + 1));
        }
        mode(&case.mode)?;
        if case.input_text.is_none() && case.audio_path.is_none() {
            return Err(format!("{} requires input_text or audio_path.", case.id));
        }
        cases.push((case, value));
    }
    if cases.is_empty() {
        return Err("Input has no cases.".into());
    }
    Ok(cases)
}

/// Wakes immediately on completion; a long requested Stop delay never delays
/// the next case or leaves a detached thread holding the next case's flag.
struct StopTimer {
    done: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StopTimer {
    fn start(milliseconds: Option<u64>, flag: Arc<AtomicBool>) -> Option<Self> {
        if milliseconds == Some(0) {
            flag.store(true, Ordering::Release);
            return None;
        }
        milliseconds.map(|milliseconds| {
            let (done, receiver) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                if receiver.recv_timeout(Duration::from_millis(milliseconds))
                    == Err(mpsc::RecvTimeoutError::Timeout)
                {
                    flag.store(true, Ordering::Release);
                }
            });
            Self {
                done,
                thread: Some(thread),
            }
        })
    }
}

impl Drop for StopTimer {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn milliseconds(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

// This mirrors the capture owner's admission boundary without opening a Store
// or capture device. Drop releases the reservation on every return path.
struct EvaluationTextLease;

impl EvaluationTextLease {
    fn acquire_layout(model: &str) -> Result<Self, String> {
        use sonojot_core::resource_manager;
        resource_manager::touch_activity(resource_manager::Stage::Cleanup);
        // Layout is optional. Match publication's immediate admission rather
        // than adding the older cleanup route's thirty-second wait.
        resource_manager::acquire_text_with_admission("cleanup", model, "dictation layout")?;
        Ok(Self)
    }

    fn acquire(model: &str, cancel: &AtomicBool, stop: &AtomicBool) -> Result<Self, String> {
        use sonojot_core::resource_manager;
        resource_manager::touch_activity(resource_manager::Stage::Cleanup);
        resource_manager::acquire_text_waiting_with_admission(
            "cleanup", model, "dictation cleanup", Duration::from_secs(30), cancel, stop,
        )?;
        Ok(Self)
    }
}

impl Drop for EvaluationTextLease {
    fn drop(&mut self) {
        sonojot_core::resource_manager::notify_text_finished();
    }
}

// These are the capture owner's existing limits/cadence, not a new ASR queue.
const CAPTURE_QUEUE_CAPACITY: usize = 4096;
const UPDATE_INTERVAL: Duration = Duration::from_millis(650);

#[derive(Serialize)]
struct EnqueueEvent {
    start_frame: u64,
    frames: u64,
    captured_at_ms: f64,
    scheduled_enqueue_ms: f64,
    enqueue_start_ms: f64,
    enqueued_ms: f64,
}

#[derive(Default)]
struct ProducerReport {
    sample_rate: u32,
    channels: u16,
    expected_frames: u64,
    enqueued_frames: u64,
    events: Vec<EnqueueEvent>,
    end_ms: Option<f64>,
    error: Option<String>,
}

#[derive(Serialize)]
struct DecodeEvent {
    start_frame: u64,
    frames: u64,
    start_ms: f64,
    end_ms: f64,
}

#[derive(Serialize)]
struct UpdateEvent {
    start_ms: f64,
    end_ms: f64,
    text: String,
}

struct ReplayCapture {
    epoch: Instant,
    backlog: Option<asr::EvaluationAudioBacklog>,
    producer: Option<std::thread::JoinHandle<ProducerReport>>,
    stop: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
}

impl ReplayCapture {
    fn start(
        path: &Path,
        pace: &str,
        callback_ms: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let backlog = asr::EvaluationAudioBacklog::start(rx, stop.clone(), cancel.clone())?;
        let epoch = Instant::now();
        let path = path.to_owned();
        let realtime = pace == "realtime";
        let thread_stop = stop.clone();
        let thread_ended = ended.clone();
        let producer = std::thread::Builder::new()
            .name("evaluation-pcm-capture".into())
            .spawn(move || {
                produce_audio(
                    &path,
                    realtime,
                    callback_ms,
                    epoch,
                    tx,
                    thread_stop,
                    thread_ended,
                    cancel,
                )
            })
            .map_err(|error| format!("Cannot start replay producer: {error}"))?;
        Ok(Self {
            epoch,
            backlog: Some(backlog),
            producer: Some(producer),
            stop,
            ended,
        })
    }

    fn join(&mut self) -> Result<ProducerReport, String> {
        // Drop the spool before joining: this closes its receiver and releases
        // a producer blocked on a terminal Error/End send after a failed decode.
        self.stop.store(true, Ordering::Release);
        self.backlog.take();
        self.producer
            .take()
            .ok_or("Replay producer already joined.")?
            .join()
            .map_err(|_| "Replay producer panicked.".into())
    }
}

impl Drop for ReplayCapture {
    fn drop(&mut self) {
        if self.producer.is_some() {
            let _ = self.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn produce_audio(
    path: &Path,
    realtime: bool,
    callback_ms: u64,
    epoch: Instant,
    tx: mpsc::SyncSender<AudioMessage>,
    stop: Arc<AtomicBool>,
    ended: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> ProducerReport {
    let mut report = ProducerReport::default();
    let result: Result<(), String> = (|| {
        // Open inside this thread: the production WAV iterator need not be Send.
        let (spec, expected_frames, mut values) = asr::evaluation_wav_samples(path)?;
        report.sample_rate = spec.sample_rate;
        report.channels = spec.channels;
        report.expected_frames = expected_frames;
        let block_frames = ((u64::from(spec.sample_rate) * callback_ms) / 1000).max(1);
        while report.enqueued_frames < expected_frames {
            let start_frame = report.enqueued_frames;
            let frames = block_frames.min(expected_frames - start_frame);
            let mut mono = Vec::with_capacity(frames as usize);
            for _ in 0..frames {
                let mut sum = 0.0f32;
                for _ in 0..spec.channels {
                    sum += values
                        .next()
                        .ok_or("WAV ended before all declared frames arrived.")??;
                }
                // Match microphone capture: downmix, then sanitize the mono value.
                let value = sum / f32::from(spec.channels);
                mono.push(if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                });
            }
            let captured_at =
                Duration::from_secs_f64(start_frame as f64 / f64::from(spec.sample_rate));
            let scheduled = Duration::from_secs_f64(
                (start_frame + frames) as f64 / f64::from(spec.sample_rate),
            );
            // Absolute deadlines keep arrival independent of decode/model load.
            // Unpaced mode is a bounded burst, not a virtual microphone clock.
            loop {
                if stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    return Err("Replay capture stopped before all frames were enqueued.".into());
                }
                let remaining = scheduled.saturating_sub(epoch.elapsed());
                if !realtime || remaining.is_zero() {
                    break;
                }
                std::thread::sleep(remaining.min(Duration::from_millis(20)));
            }
            let rms =
                (mono.iter().map(|value| value * value).sum::<f32>() / mono.len() as f32).sqrt();
            // Capture uses try_send. Overflow is an explicit failed observation;
            // waiting here would silently change the arrival schedule.
            let enqueue_start_ms = milliseconds(epoch);
            tx.try_send(AudioMessage::Chunk {
                samples: mono,
                sample_rate: spec.sample_rate,
                rms,
                captured_at,
                discontinuity: false,
                dropped_frames: 0,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => {
                    "Replay capture queue overflowed; audio is incomplete.".to_owned()
                }
                mpsc::TrySendError::Disconnected(_) => {
                    "Replay capture receiver disconnected.".to_owned()
                }
            })?;
            let enqueued_ms = milliseconds(epoch);
            report.events.push(EnqueueEvent {
                start_frame,
                frames,
                captured_at_ms: captured_at.as_secs_f64() * 1000.0,
                scheduled_enqueue_ms: scheduled.as_secs_f64() * 1000.0,
                enqueue_start_ms,
                enqueued_ms,
            });
            report.enqueued_frames += frames;
        }
        if values.next().is_some() {
            return Err("WAV frame count did not match its declared data length.".into());
        }
        Ok(())
    })();
    ended.store(true, Ordering::Release);
    report.end_ms = Some(milliseconds(epoch));
    if let Err(error) = result {
        report.error = Some(error.clone());
        let _ = tx.send(AudioMessage::Error(error));
    }
    let _ = tx.send(AudioMessage::End);
    report
}

struct ReplayEvidence {
    epoch: Instant,
    producer: ProducerReport,
    consumed_frames: u64,
    spool_peak_bytes: u64,
    decode: Vec<DecodeEvent>,
    updates: Vec<UpdateEvent>,
    first_text_ms: Option<f64>,
    first_text_kind: Option<&'static str>,
    raw_ready_ms: Option<f64>,
    finalize_start_ms: Option<f64>,
    stream_start_ms: f64,
    stream_ready_ms: f64,
    asr_active_ms: f64,
}

impl ReplayEvidence {
    fn json(&self, args: &Args, final_ready_ms: Option<f64>) -> Value {
        let mut completed = 0usize;
        let mut processed_frames = 0u64;
        let mut peak_pending_frames = 0u64;
        for enqueue in &self.producer.events {
            while self
                .decode
                .get(completed)
                .is_some_and(|decode| decode.end_ms <= enqueue.enqueued_ms)
            {
                processed_frames += self.decode[completed].frames;
                completed += 1;
            }
            peak_pending_frames = peak_pending_frames
                .max((enqueue.start_frame + enqueue.frames).saturating_sub(processed_frames));
        }
        let final_audio = self.producer.events.last().map(|event| event.enqueued_ms);
        let realtime = args.pace == "realtime";
        let raw_delay = realtime
            .then(|| {
                self.raw_ready_ms
                    .zip(final_audio)
                    .map(|(end, start)| end - start)
            })
            .flatten();
        let final_delay = realtime
            .then(|| {
                final_ready_ms
                    .zip(final_audio)
                    .map(|(end, start)| end - start)
            })
            .flatten();
        let max_lateness = realtime.then(|| {
            self.producer
                .events
                .iter()
                .map(|event| (event.enqueued_ms - event.scheduled_enqueue_ms).max(0.0))
                .fold(0.0, f64::max)
        });
        let sample_rate = self.producer.sample_rate;
        json!({
            "mode": "stream", "pace": args.pace, "callback_ms": args.callback_ms,
            "update_interval_ms": 650, "capture_queue_capacity": CAPTURE_QUEUE_CAPACITY,
            "spool_quota_bytes": 268_435_456u64,
            "timestamp_basis": "milliseconds since independent PCM producer start",
            "enqueue_timestamp_definition": "enqueued_ms is measured immediately after successful try_send; enqueue_start_ms bounds the call",
            "measurement_scope": "production stream, WAV reader and audio backlog; excludes microphone hardware, resource admission, cleanup prewarming and UI publication",
            "first_text_definition": "first nonempty text observed at the production update cadence, or finalization; provisional text is not a correctness guarantee",
            "pending_definition": "enqueued source frames not yet returned from stream.add; includes the active call, excludes native audio retained for future decoding",
            "sample_rate": sample_rate, "source_channels": self.producer.channels,
            "expected_frames": self.producer.expected_frames, "enqueued_frames": self.producer.enqueued_frames,
            "consumed_frames": self.consumed_frames,
            "frames_complete": self.producer.error.is_none() && self.producer.expected_frames == self.producer.enqueued_frames && self.producer.enqueued_frames == self.consumed_frames,
            "producer_error": self.producer.error,
            "spool_peak_bytes": self.spool_peak_bytes, "peak_pending_frames": peak_pending_frames,
            "peak_pending_audio_ms": if sample_rate > 0 { Some(peak_pending_frames as f64 * 1000.0 / f64::from(sample_rate)) } else { None },
            "max_enqueue_lateness_ms": max_lateness,
            "first_text_ms": self.first_text_ms, "first_text_kind": self.first_text_kind,
            "final_audio_enqueued_ms": final_audio, "producer_end_ms": self.producer.end_ms,
            "raw_ready_ms": self.raw_ready_ms, "final_ready_ms": final_ready_ms,
            "final_audio_to_raw_ms": raw_delay, "final_audio_to_final_ms": final_delay,
            "producer_end_to_raw_ms": self.raw_ready_ms.zip(self.producer.end_ms).map(|(end, start)| end - start),
            "asr_active_ms": self.asr_active_ms,
            "events": {"enqueue": self.producer.events, "decode": self.decode, "update": self.updates,
                "stream_start": {"start_ms": self.stream_start_ms, "end_ms": self.stream_ready_ms},
                "finalize": {"start_ms": self.finalize_start_ms, "end_ms": self.raw_ready_ms}}
        })
    }
}

fn load_recognizer(
    args: &Args,
    selected: &str,
    cancel: &AtomicBool,
    recognizer: &mut Option<asr::Recognizer>,
    row: &mut Value,
) -> Result<(), String> {
    if recognizer.is_none() {
        let start = Instant::now();
        let loaded = asr::Recognizer::load_interruptible(&args.root, selected, cancel);
        let load_ms = milliseconds(start);
        row["asr"]["load_ms"] = json!(load_ms);
        row["timings"]["asr_load_ms"] = json!(load_ms);
        *recognizer = Some(loaded?);
    }
    let loaded = recognizer.as_ref().ok_or("Recognizer unavailable.")?;
    row["asr"]["resolved_model"] = json!(loaded.model_id);
    row["asr"]["backend"] = json!(loaded.backend());
    row["asr"]["provider_initialized"] = json!(loaded.initialized_provider());
    row["asr"]["execution_proof"] = json!(loaded.execution_proof());
    Ok(())
}

fn transcribe_stream(
    args: &Args,
    path: &Path,
    selected: &str,
    cancel: Arc<AtomicBool>,
    recognizer: &mut Option<asr::Recognizer>,
    row: &mut Value,
    evidence: &mut Option<ReplayEvidence>,
) -> Result<asr::RecognitionResult, String> {
    // Capture starts before a cold model load, as it does in speech::run_job.
    let mut capture = ReplayCapture::start(path, &args.pace, args.callback_ms, cancel.clone())?;
    let mut data = ReplayEvidence {
        epoch: capture.epoch,
        producer: ProducerReport::default(),
        consumed_frames: 0,
        spool_peak_bytes: 0,
        decode: Vec::new(),
        updates: Vec::new(),
        first_text_ms: None,
        first_text_kind: None,
        raw_ready_ms: None,
        finalize_start_ms: None,
        stream_start_ms: 0.0,
        stream_ready_ms: 0.0,
        asr_active_ms: 0.0,
    };
    let mut decode_started = None;
    let result = (|| {
        load_recognizer(args, selected, &cancel, recognizer, row)?;
        let started = Instant::now();
        decode_started = Some(started);
        data.stream_start_ms = milliseconds(data.epoch);
        let mut stream = recognizer
            .as_mut()
            .ok_or("Recognizer unavailable.")?
            .stream_interruptible(&cancel)?;
        data.stream_ready_ms = milliseconds(data.epoch);
        data.asr_active_ms += data.stream_ready_ms - data.stream_start_ms;
        let mut last_update = Instant::now();
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Replay cancelled.".into());
            }
            let message = capture
                .backlog
                .as_mut()
                .ok_or("Replay backlog unavailable.")?
                .recv_timeout(Duration::from_millis(25));
            match message {
                Ok(AudioMessage::Chunk {
                    samples,
                    sample_rate,
                    ..
                }) => {
                    let start_ms = milliseconds(data.epoch);
                    let added = stream.add(&samples, sample_rate);
                    let end_ms = milliseconds(data.epoch);
                    data.asr_active_ms += end_ms - start_ms;
                    data.decode.push(DecodeEvent {
                        start_frame: data.consumed_frames,
                        frames: samples.len() as u64,
                        start_ms,
                        end_ms,
                    });
                    added?;
                    data.consumed_frames += samples.len() as u64;
                    if stream.evaluation_failed() {
                        break;
                    }
                    if last_update.elapsed() >= UPDATE_INTERVAL
                        && !capture.ended.load(Ordering::Acquire)
                    {
                        let start_ms = milliseconds(data.epoch);
                        let updated = stream.update();
                        let end_ms = milliseconds(data.epoch);
                        data.asr_active_ms += end_ms - start_ms;
                        let text = updated?;
                        if data.first_text_ms.is_none() && !text.trim().is_empty() {
                            data.first_text_ms = Some(end_ms);
                            data.first_text_kind = Some("interim");
                        }
                        data.updates.push(UpdateEvent {
                            start_ms,
                            end_ms,
                            text,
                        });
                        last_update = Instant::now();
                    }
                }
                Ok(AudioMessage::End) => break,
                Ok(AudioMessage::Error(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Audio ended without capture finalization.".into())
                }
            }
        }
        data.spool_peak_bytes = asr::evaluation_backlog_peak_bytes(
            capture
                .backlog
                .as_ref()
                .ok_or("Replay backlog unavailable.")?,
        );
        data.producer = capture.join()?;
        let finalize_start = milliseconds(data.epoch);
        data.finalize_start_ms = Some(finalize_start);
        let finalized = stream.finish_observed();
        let raw_ready = milliseconds(data.epoch);
        data.asr_active_ms += raw_ready - finalize_start;
        let (result, trace) = finalized?;
        data.raw_ready_ms = Some(raw_ready);
        row["asr"]["trace"] = json!({
            "requested_model": selected,
            "resolved_model": row["asr"]["resolved_model"],
            "sample_rate": data.producer.sample_rate,
            "channels": data.producer.channels,
            "input_frames_consumed": data.consumed_frames,
            "finalized": true,
            "recognition_completeness": result.completeness,
            "warnings": result.warnings,
            "recognition": trace,
        });
        row["raw_text"] = json!(result.text);
        if data.first_text_ms.is_none() && !result.text.trim().is_empty() {
            data.first_text_ms = Some(raw_ready);
            data.first_text_kind = Some("final");
        }
        // A failed worker can return a useful incomplete transcript. Preserve
        // that result; the caller skips cleanup and marks the observation failed.
        if result.completeness != RecognitionCompleteness::Incomplete {
            if let Some(error) = &data.producer.error {
                return Err(error.clone());
            }
            if data.producer.expected_frames != data.consumed_frames {
                return Err("Replay did not consume every declared source frame.".into());
            }
        }
        Ok(result)
    })();
    if let Some(start) = decode_started {
        let asr_ms = milliseconds(start);
        row["asr"]["transcribe_ms"] = json!(asr_ms);
        row["timings"]["asr_ms"] = json!(asr_ms);
        if args.pace == "unpaced" {
            if let Some(seconds) = row["asr"]["audio_seconds"]
                .as_f64()
                .filter(|seconds| *seconds > 0.0)
            {
                row["asr"]["real_time_factor"] = json!(asr_ms / (seconds * 1000.0));
            }
        }
    }
    if capture.producer.is_some() {
        if let Some(backlog) = &capture.backlog {
            data.spool_peak_bytes = asr::evaluation_backlog_peak_bytes(backlog);
        }
        match capture.join() {
            Ok(report) => data.producer = report,
            Err(error) => data.producer.error = Some(error),
        }
    }
    *evidence = Some(data);
    result
}

fn validate_layout_case(args: &Args, case: &Case) -> Result<(), String> {
    if let Some(context) = &case.app_context {
        if !args.smart_layout || context.trim().is_empty() || context.len() > 200
            || context.chars().any(char::is_control)
        {
            return Err(format!("{}: app_context requires smart layout and a single-line description of 1 to 200 bytes.", case.id));
        }
    }
    if args.smart_layout && (case.input_text.is_none()
        || case.scope.as_deref().is_some_and(|scope| scope != "dictation"))
    {
        return Err(format!("{}: smart layout requires input_text and dictation scope.", case.id));
    }
    Ok(())
}

/// Layout owns no new worker or lease. Keep the previous local-rules result
/// explicit so skipped/rejected proposals can be scored without rerunning it.
fn evaluate_layout(
    args: &Args,
    case: &Case,
    raw: &str,
    selected_mode: cleanup::Mode,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    row: &mut Value,
) {
    let policy = cleanup::CleanupPolicy::conservative();
    let rules_started = Instant::now();
    let mut result = cleanup::run_interruptible_with_policy(
        &args.root, raw, selected_mode, &case.vocabulary, &case.confirmed_vocabulary,
        "local-rules", policy, cancel, stop,
    );
    let rules_ms = milliseconds(rules_started);
    let prior = result.text.clone();
    let prior_status = result.status.clone();
    let admission_started = Instant::now();
    let mut lease = None;
    let mut admission_error = None;
    let preflight = cleanup::layout::preflight_outcome(&prior, selected_mode, cancel, stop);
    let early_outcome = if preflight.is_some() {
        preflight
    } else {
        match EvaluationTextLease::acquire_layout(&args.model) {
            Ok(acquired) => { lease = Some(acquired); None }
            Err(error) => {
                admission_error = Some(error);
                let reason = if cancel.load(Ordering::Acquire) { "cancelled" }
                    else if stop.load(Ordering::Acquire) { "cleanup_stopped" }
                    else { "resource_unavailable" };
                Some(cleanup::layout::skip(&prior, reason))
            }
        }
    };
    row["resource_lease"] = json!({"requested": true, "acquired": lease.is_some(),
        "admission_ms": milliseconds(admission_started), "error": admission_error});
    let worker_before = cleanup::resource::worker_status();
    let outcome = if let Some(outcome) = early_outcome {
        outcome
    } else {
        cleanup::layout::run(&args.root, &prior, selected_mode, &args.model, cancel, stop, case.app_context.as_deref())
    };
    row["smart_layout"] = json!(true);
    row["route"] = json!("local_rules_then_layout");
    row["fallback_text"] = json!(prior);
    row["layout"] = json!({
        "schema_version": 1, "requested_model": args.model,
        "policy_sha256": cleanup::layout::policy_hash_with_context(case.app_context.as_deref()),
        "app_context": case.app_context,
        "input_text": prior, "text": outcome.text, "class": outcome.class,
        "probabilities": outcome.probabilities, "applied": outcome.applied,
        "reason": outcome.reason, "elapsed_ms": outcome.elapsed_ms,
    });
    row["timings"]["rules_ms"] = json!(rules_ms);
    row["timings"]["layout_ms"] = json!(outcome.elapsed_ms);
    cleanup::layout::apply_to_result(&mut result, outcome);
    row["final_text"] = json!(result.text);
    row["status"] = json!(result.status);
    row["cleanup"] = json!({"trace_kind": "production_rules_then_layout", "result": result,
        "policy_sha256": policy.hash(), "requested_model": "local-rules",
        "rules_status": prior_status, "internal_stage_timings_available": false});
    for (output, field) in [("pre_model_text", "preModelText"),
        ("publication_stage", "publicationStage"), ("guard_reason", "guardReason")]
    {
        row[output] = row["cleanup"]["result"][field].clone();
    }
    let worker = cleanup::resource::worker_status();
    if lease.is_some() {
        // This replay process has one fixed model selection and no other
        // generation caller. worker_status deliberately exposes no model id.
        if worker.is_some() {
            sonojot_core::resource_manager::note_text_loaded(&args.model);
        } else {
            sonojot_core::resource_manager::note_text_load_failed();
        }
    }
    row["cleanup"]["worker_before"] = json!(worker_before);
    row["cleanup"]["worker_after"] = json!(worker);
    drop(lease);
    row["resource_lease"]["released"] = json!(true);
}

fn evaluate(
    args: &Args,
    case: &Case,
    input: &Value,
    recognizer: &mut Option<asr::Recognizer>,
    repetition: usize,
    measured: bool,
) -> Value {
    let started = Instant::now();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut replay_evidence = None;
    let stop = Arc::new(AtomicBool::new(false));
    let mut row = json!({
        "schema_version": 1, "id": case.id, "input": input, "mode": case.mode,
        "variant": args.variant, "cleanup_policy": match args.variant.as_str() {
            "raw" => "none", "baseline" => "legacy-evaluation",
            "structured" => "structured-evaluation", _ => args.cleanup_policy().version },
        "model": args.model, "repetition": repetition, "measured": measured,
        "phase": if measured { "measured" } else { "warmup" },
        "raw_text": null, "fallback_text": null, "final_text": null,
        "pre_model_text": null, "publication_stage": null, "punctuation_stage": null, "guard_reason": null,
        "scope": case.scope, "meeting_rules_only": args.meeting_rules_only,
        "asr": null, "cleanup": null, "error": null,
        "replay": {"mode": args.replay_mode, "pace": if args.replay_mode == "stream" { Some(args.pace.as_str()) } else { None }},
        "trace_cleanup": args.trace_cleanup,
        "cleanup_input_envelope": args.cleanup_input_envelope,
        "daily_clean_prompt_override": args.daily_clean_prompt.as_ref().map(|prompt| {
            use sha2::{Digest, Sha256};
            json!({"path": args.daily_clean_prompt_path, "sha256": format!("{:x}", Sha256::digest(prompt.as_bytes())), "applied": case.mode == "clean"})
        }),
        "timings": {"asr_load_ms": 0.0, "asr_ms": 0.0, "cleanup_ms": 0.0, "punctuation_ms": null, "total_ms": 0.0}
    });
    let result: Result<(), String> = (|| {
        let selected_mode = mode(&case.mode)?;
        validate_layout_case(args, case)?;
        let raw = if let Some(text) = &case.input_text {
            row["source"] = json!("frozen_text");
            row["replay"] = json!({"mode": "frozen_text", "pace": null});
            text.clone()
        } else {
            row["source"] = json!("audio");
            let selected = args
                .asr_model
                .as_deref()
                .ok_or("Audio rows require --asr-model.")?;
            let relative = case.audio_path.as_ref().ok_or("Missing audio_path.")?;
            let path = if relative.is_absolute() {
                relative.clone()
            } else {
                args.input.parent().unwrap_or(Path::new(".")).join(relative)
            };
            let audio_seconds = recording::wav_duration(&path)?;
            if args.cold_asr {
                *recognizer = None;
            }
            let cold_load = recognizer.is_none();
            row["asr"] = json!({"requested_model": selected, "resolved_model": null,
                "language": args.language,
                "max_chunk_seconds": args.max_chunk_seconds,
                "audio_path": path, "audio_seconds": audio_seconds, "cold_load": cold_load,
                "load_ms": 0.0, "transcribe_ms": null, "real_time_factor": null,
                "recognition_completeness": null, "trace": null});
            let result = if args.replay_mode == "stream" {
                transcribe_stream(
                    args,
                    &path,
                    selected,
                    cancel.clone(),
                    recognizer,
                    &mut row,
                    &mut replay_evidence,
                )
            } else {
                load_recognizer(args, selected, &cancel, recognizer, &mut row)?;
                let loaded = recognizer.as_mut().ok_or("Recognizer unavailable.")?;
                let decode_started = Instant::now();
                let mut trace = Value::Null;
                let result = asr::transcribe_file_observed(loaded, &path, &cancel, &mut trace);
                let asr_ms = milliseconds(decode_started);
                row["asr"]["transcribe_ms"] = json!(asr_ms);
                row["timings"]["asr_ms"] = json!(asr_ms);
                if audio_seconds > 0.0 {
                    row["asr"]["real_time_factor"] = json!(asr_ms / (audio_seconds * 1000.0));
                }
                row["asr"]["trace"] = trace;
                result
            };
            let result = result?;
            row["asr"]["recognition_completeness"] = json!(result.completeness);
            row["asr"]["warnings"] = json!(result.warnings);
            row["asr"]["segments"] = json!(result.segments);
            row["raw_text"] = json!(result.text);
            if result.completeness == RecognitionCompleteness::Incomplete {
                row["status"] = json!("incomplete_recognition");
                return Err(
                    "Recognition is incomplete; raw text retained and cleanup skipped.".into(),
                );
            }
            result.text
        };
        row["raw_text"] = json!(raw);
        // Historical replay scripts use this to freeze ASR without references.
        row["input"]["input_text"] = json!(raw);
        if args.punctuation_labels {
            let labels_started = Instant::now();
            row["punctuation_labels"] = json!(sonojot_core::punctuation::evaluation_labels(&args.root, &raw, &cancel)?);
            row["timings"]["punctuation_ms"] = json!(milliseconds(labels_started));
            row["status"] = json!("punctuation_labels");
            return Ok(());
        }
        if args.variant == "raw" {
            row["fallback_text"] = json!(raw);
            row["final_text"] = json!(raw);
            row["status"] = json!("original");
            return Ok(());
        }
        let cleanup_started = Instant::now();
        let timer = StopTimer::start(case.stop_after_ms.or(args.stop_after_ms), stop.clone());
        let cancel_timer = StopTimer::start(case.cancel_after_ms.or(args.cancel_after_ms), cancel.clone());
        if args.smart_layout {
            evaluate_layout(args, case, &raw, selected_mode, &cancel, &stop, &mut row);
        } else if args.trace_cleanup {
            let request = cleanup::EvaluationRequest {
                root: &args.root,
                original: &raw,
                mode: selected_mode,
                vocabulary: &case.vocabulary,
                model: &args.model,
                system_prompt: args.daily_clean_prompt.as_deref(),
                cancel: &cancel,
                stop_cleanup: &stop,
            };
            let evaluation = if args.policy == "targeted" {
                cleanup::evaluate_targeted_interruptible(request, &case.confirmed_vocabulary)
            } else {
                cleanup::evaluate_daily_interruptible(request, &case.confirmed_vocabulary)
            };
            row["final_text"] = json!(evaluation.final_text);
            row["fallback_text"] = json!(evaluation.fallback_text);
            row["status"] = json!(evaluation.status);
            row["cleanup"] =
                serde_json::to_value(&evaluation).map_err(|error| error.to_string())?;
            row["cleanup"]["trace_kind"] = json!(if args.policy == "targeted" {
                "targeted_diagnostic_extra_overhead"
            } else {
                "daily_diagnostic_extra_overhead"
            });
            row["cleanup"]["latency_ranking_eligible"] = json!(false);
            if let Some(error) = evaluation.error {
                row["cleanup_error"] = json!(error);
            }
        } else if matches!(args.variant.as_str(), "model" | "host" | "daily") {
            let policy = args.cleanup_policy();
            let meeting = args.meeting_rules_only && case.scope.as_deref() == Some("meeting");
            let model = if args.variant == "host" || meeting {
                "local-rules"
            } else {
                &args.model
            };
            row["route"] = json!(if meeting { "meeting_rules_only" } else if args.variant == "host" { "rules_only" } else { "production_model" });
            let lease_started = Instant::now();
            let lease = if args.resource_lease && !matches!(model, "off" | "local-rules" | "") {
                // Already interrupted requests still enter the production
                // route so its cancellation result is observable, with no load.
                if cancel.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
                    None
                } else {
                    Some(EvaluationTextLease::acquire(model, &cancel, &stop)?)
                }
            } else {
                None
            };
            row["resource_lease"] = json!({"requested": args.resource_lease,
                "acquired": lease.is_some(), "admission_ms": milliseconds(lease_started)});
            let worker_before = cleanup::resource::worker_status();
            // Execute production policy directly. Its internal chunk timings
            // are not exposed by this API; do not fabricate a model duration.
            let result = if case.scope.as_deref() == Some("meeting") {
                cleanup::run_interruptible_with_policy(
                    &args.root, &raw, selected_mode, &case.vocabulary, &case.confirmed_vocabulary,
                    model, policy, &cancel, &stop,
                )
            } else {
                row["written_form"] = json!(args.written_form.as_str());
                cleanup::run_dictation_interruptible_with_written_form_mode(
                    &args.root, &raw, selected_mode, &case.vocabulary, &case.confirmed_vocabulary,
                    model, policy, &cancel, &stop, args.written_form,
                )
            };
            row["final_text"] = json!(result.text);
            row["status"] = json!(result.status);
            row["timings"]["punctuation_ms"] = json!(result.punctuation_ms);
            row["punctuation_stage"] = json!(result.punctuation_stage);
            row["cleanup"] = json!({"trace_kind": "production_result", "result": result,
                "policy_sha256": if model == cleanup::smart::MODEL_ID { cleanup::smart::policy_hash() } else { policy.hash() }, "requested_model": model,
                "internal_stage_timings_available": false});
            // Preserve optional provenance without recomputing the host stage.
            // ResultText uses camelCase on the wire; top-level replay keys
            // remain snake_case for historical analysis scripts.
            for (output, field) in [("pre_model_text", "preModelText"),
                ("publication_stage", "publicationStage"), ("guard_reason", "guardReason")]
            {
                row[output] = row["cleanup"]["result"][field].clone();
            }
            row["fallback_text"] = if args.variant == "host" || meeting {
                row["final_text"].clone()
            } else {
                row["pre_model_text"].clone()
            };
            let worker = cleanup::resource::worker_status();
            if lease.is_some() {
                if worker.is_some() {
                    sonojot_core::resource_manager::note_text_loaded(model);
                } else {
                    sonojot_core::resource_manager::note_text_load_failed();
                }
            }
            row["cleanup"]["worker_after"] = json!(worker);
            row["cleanup"]["worker_before"] = json!(worker_before);
            let erase = cleanup::resource::last_erase_report();
            row["cleanup"]["job_erase"] = json!({
                "erased": erase.erased, "erased_tokens": erase.erased_tokens,
                "already_clear": erase.already_clear, "shutdown_fallback": erase.shutdown_fallback,
                "busy_deferred": erase.busy_deferred, "error": erase.error,
                "elapsed_ms": erase.elapsed_ms, "private": erase.private(),
            });
            drop(lease);
            row["resource_lease"]["released"] = json!(true);
        } else {
            let stage = if args.variant == "structured" && selected_mode == cleanup::Mode::Clean {
                Some(cleanup::structured_stage(
                    &raw,
                    &case.confirmed_vocabulary,
                    case.active_scope.as_deref(),
                    &cancel,
                ))
            } else {
                None
            };
            let source = stage
                .as_ref()
                .map_or(raw.as_str(), |stage| stage.text.as_str());
            let evaluation = cleanup::evaluate_interruptible_with_options(
                cleanup::EvaluationRequest {
                    root: &args.root,
                    original: source,
                    mode: selected_mode,
                    vocabulary: &case.vocabulary,
                    model: &args.model,
                    system_prompt: args.prompt.as_deref(),
                    cancel: &cancel,
                    stop_cleanup: &stop,
                },
                args.options,
            );
            if let Some(stage) = &stage {
                row["structured_stage"] = json!({"input_text": raw, "output_text": stage.text,
                    "edits": stage.edits, "vocabulary_edits": stage.vocabulary_edits,
                    "source_revision": stage.source_revision, "cancelled": stage.cancelled,
                    "error": stage.error, "elapsed_ms": stage.elapsed_ms});
            }
            row["fallback_text"] = json!(evaluation.fallback_text);
            row["final_text"] = json!(evaluation.final_text);
            row["status"] = json!(evaluation.status);
            row["cleanup"] =
                serde_json::to_value(&evaluation).map_err(|error| error.to_string())?;
            if let Some(error) = evaluation.error {
                // An explicit Stop and a deliberately injected validation
                // rejection are test outcomes, not infrastructure failures.
                if error != "Local cleanup stopped." && !args.options.force_rejection {
                    row["cleanup_error"] = json!(error);
                }
            }
        }
        drop(timer);
        drop(cancel_timer);
        row["interruption"] = json!({"stop_requested": stop.load(Ordering::Acquire),
            "cancel_requested": cancel.load(Ordering::Acquire)});
        if args.cleanup_input_envelope == "json" {
            row["cleanup"]["evaluation_override"] = json!({
                "input_envelope": "json", "scope": "single full-dictation packet only; context packets remain plain",
                "base_policy": args.cleanup_policy().version,
            });
        }
        row["timings"]["cleanup_ms"] = json!(milliseconds(cleanup_started));
        if (args.smart_layout && cancel.load(Ordering::Acquire)) || matches!(
            row["status"].as_str(),
            Some("safe_formatting_model_error" | "cancelled")
        ) {
            return Err(row["cleanup_error"]
                .as_str()
                .unwrap_or("Cleanup failed; safe fallback retained.")
                .to_owned());
        }
        Ok(())
    })();
    // Record text readiness before serializing the large callback trace.
    let final_ready_ms = replay_evidence.as_ref().and_then(|data: &ReplayEvidence| {
        (!row["final_text"].is_null()).then(|| milliseconds(data.epoch))
    });
    row["timings"]["total_ms"] = json!(milliseconds(started));
    if let Some(data) = replay_evidence {
        row["replay"] = data.json(args, final_ready_ms);
    }
    if let Err(error) = result {
        row["error"] = json!(error);
        if row.get("status").is_none() {
            row["status"] = json!("error");
        }
        // A failed worker must not contaminate the next warm observation.
        *recognizer = None;
    }
    row
}

fn run(args: Args) -> Result<(), String> {
    if args.smart_layout {
        std::env::set_var("SONOJOT_EVAL_READ_ONLY_MODELS", "1");
    }
    if let Some(directory) = &args.punctuation_model_dir {
        // Set once, before replay timers/workers. The production loader validates
        // all files and records model or rule fallback; no model is downloaded.
        std::env::set_var("SONOJOT_PUNCT_MODEL_DIR", directory);
    }
    // The isolated worker inherits this value. It is read only in builds with
    // fidelity-eval, and reset explicitly so a shell cannot change the default.
    std::env::set_var("SONOJOT_EVAL_ASR_LANGUAGE", &args.language);
    std::env::set_var(
        "SONOJOT_EVAL_CLEANUP_INPUT_ENVELOPE",
        &args.cleanup_input_envelope,
    );
    std::env::set_var(
        "SONOJOT_EVAL_MAX_CHUNK_SECONDS",
        args.max_chunk_seconds.to_string(),
    );
    let cases = read_cases(&args.input)?;
    let mut layout_needs_model = false;
    for (case, _) in &cases {
        validate_layout_case(&args, case)?;
        if args.smart_layout && cleanup::layout::preflight_outcome(
            case.input_text.as_deref().unwrap_or_default(), mode(&case.mode)?,
            &AtomicBool::new(case.cancel_after_ms.or(args.cancel_after_ms) == Some(0)),
            &AtomicBool::new(case.stop_after_ms.or(args.stop_after_ms) == Some(0)),
        ).is_none() {
            layout_needs_model = true;
        }
    }
    if layout_needs_model {
        // Verify installed bytes outside row timing. An unavailable model still
        // gets per-row fallback evidence from layout rather than losing text.
        let _ = cleanup::model_ready(&args.root, &args.model);
    }
    // Production inventory verifies installed bytes before selection. Mirror
    // that preparation outside timed rows; otherwise a fast all-text replay
    // could finish before the background hash and measure only cold fallbacks.
    if args.model == cleanup::smart::MODEL_ID && !cleanup::model_ready(&args.root, &args.model) {
        return Err("Smart cleanup model or runtime is not ready in the isolated models root.".into());
    }
    if args.replay_mode == "stream" && cases.iter().any(|(case, _)| case.input_text.is_some()) {
        return Err(
            "Streaming replay requires audio-only rows; use --replay-mode file for frozen text."
                .into(),
        );
    }
    if cases.iter().any(|(case, _)| case.input_text.is_none()) && args.asr_model.is_none() {
        return Err("Audio input requires --asr-model.".into());
    }
    if let Some(parent) = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)
        .map_err(|error| format!("Cannot create {}: {error}", args.output.display()))?;
    let mut output = BufWriter::new(output);
    let mut recognizer = None;
    let mut errors = 0usize;
    let prewarm_started = Instant::now();
    let prewarm = args.punctuation_prewarm.then(|| sonojot_core::punctuation::prewarm(&args.root, Arc::new(AtomicBool::new(false))));
    let prewarm_evidence = if prewarm.is_some() {
        Some(sonojot_core::punctuation::evaluation_wait_ready(&args.root, &AtomicBool::new(false))?)
    } else { None };
    let prewarm_ms = milliseconds(prewarm_started);
    for (passes, measured) in [(args.warmup, false), (args.repeat, true)] {
        for repetition in 0..passes {
            for (case, input) in &cases {
                let mut row = evaluate(&args, case, input, &mut recognizer, repetition, measured);
                if let Some(evidence) = &prewarm_evidence {
                    row["punctuation_prewarm"] = json!({"load_ms": prewarm_ms, "runtime": evidence});
                }
                if !row["error"].is_null() {
                    errors += 1;
                }
                serde_json::to_writer(&mut output, &row).map_err(|error| error.to_string())?;
                output
                    .write_all(b"\n")
                    .and_then(|_| output.flush())
                    .map_err(|error| error.to_string())?;
                eprintln!(
                    "{} repetition={} measured={} status={} total_ms={:.1}",
                    case.id,
                    repetition,
                    measured,
                    row["status"].as_str().unwrap_or("unknown"),
                    row["timings"]["total_ms"].as_f64().unwrap_or(0.0)
                );
            }
        }
    }
    drop(recognizer);
    if errors > 0 {
        return Err(format!(
            "{errors} cases failed; evidence retained in {}.",
            args.output.display()
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct WrittenFormReplayArgs {
    input: PathBuf,
    output: PathBuf,
    span: bool,
    mode: WrittenFormMode,
    root: Option<PathBuf>,
    package: Option<PathBuf>,
}

impl WrittenFormReplayArgs {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut values = argument_values(arguments)?;
        let mode = take_written_form(&mut values)?;
        let span = match take_required(&mut values, "--written-form-replay")?.as_str() {
            "text" => false,
            "span" => true,
            _ => return Err("--written-form-replay requires text or span.".into()),
        };
        let input = PathBuf::from(take_required(&mut values, "--input")?);
        let output = PathBuf::from(take_required(&mut values, "--output")?);
        let root = values.remove("--models-root").map(PathBuf::from);
        let package = values.remove("--punctuation-model-dir").map(PathBuf::from);
        if span && mode == WrittenFormMode::Model {
            return Err("Model routing requires text replay.".into());
        }
        if (root.is_some() || package.is_some()) && mode != WrittenFormMode::Model {
            return Err("Standalone model paths require --written-form model.".into());
        }
        if package.is_some() && root.is_none() {
            return Err("--punctuation-model-dir requires --models-root for the shared runtime.".into());
        }
        if let Some(flag) = values.keys().next() {
            return Err(format!("Unsupported standalone written-form argument: {flag}"));
        }
        Ok(Self { input, output, span, mode, root, package })
    }
}

#[derive(Deserialize)]
struct WrittenFormReplayCase {
    id: String,
    text: String,
    #[serde(alias = "cls")]
    class: Option<sonojot_core::written_form::Class>,
    #[serde(alias = "context_before")]
    before: Option<String>,
    #[serde(alias = "context_after")]
    after: Option<String>,
    #[serde(default, alias = "wf_labels")]
    labels: Option<Vec<sonojot_core::written_form::BioLabel>>,
}

fn read_written_form_cases(args: &WrittenFormReplayArgs) -> Result<Vec<WrittenFormReplayCase>, String> {
    let reader = BufReader::new(File::open(&args.input).map_err(|error| error.to_string())?);
    let mut cases = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() { continue; }
        // Do not include serde's rejected value: it can contain private text.
        let case: WrittenFormReplayCase = serde_json::from_str(&line)
            .map_err(|_| format!("Line {}: invalid written-form replay row.", index + 1))?;
        if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
            return Err(format!("Line {}: empty or duplicate id.", index + 1));
        }
        if args.span && case.class.is_none() {
            return Err(format!("Line {}: span replay requires class.", index + 1));
        }
        if !args.span && (case.class.is_some() || case.before.is_some() || case.after.is_some()) {
            return Err(format!("Line {}: class/context require span replay.", index + 1));
        }
        if let Some(labels) = &case.labels {
            if args.span || args.mode != WrittenFormMode::Model || args.root.is_some() {
                return Err(format!("Line {}: saved labels require model text replay without model paths.", index + 1));
            }
            if labels.len() != case.text.split_whitespace().count() {
                return Err(format!("Line {}: written-form label alignment failed.", index + 1));
            }
        }
        cases.push(case);
    }
    if cases.is_empty() { return Err("Input has no cases.".into()); }
    Ok(cases)
}

fn evaluate_written_form(args: &WrittenFormReplayArgs, case: &WrittenFormReplayCase) -> Value {
    let started = Instant::now();
    let enabled = args.mode != WrittenFormMode::Off;
    let inferred;
    let labels = if args.mode == WrittenFormMode::Model {
        if let Some(root) = &args.root {
            match sonojot_core::punctuation::evaluation_written_form_labels(root, &case.text, &AtomicBool::new(false)) {
                Ok(value) => { inferred = value; inferred.as_deref() }
                Err(error) => return json!({"id": case.id, "error": error}),
            }
        } else { case.labels.as_deref() }
    } else { None };
    let rendered = if enabled && args.span {
        case.class.and_then(|class| sonojot_core::written_form::render_span(
            &case.text, class, case.before.as_deref(), case.after.as_deref(),
        ))
    } else { None };
    let (output, spans) = if enabled && !args.span {
        sonojot_core::written_form::apply_with_edits(&case.text, labels)
    } else {
        (rendered.clone().unwrap_or_else(|| case.text.clone()), Vec::new())
    };
    json!({
        "schema_version": 1, "id": case.id,
        "route": if args.span { "written_form_span" } else { "written_form_text" },
        "written_form": args.mode.as_str(),
        "span_head": labels.is_some(),
        "span_labels": labels,
        "raw_text": case.text, "final_text": output, "rendered": rendered,
        "spans": spans,
        "changed": output != case.text, "elapsed_ms": milliseconds(started), "error": null,
    })
}

/// Report accepted edits with the same protection and rendering checks as apply.
/// This evaluation-only trace deliberately uses the public application path so
/// rejected proposals cannot become false predictions in held-out scoring.
#[cfg(test)]
fn written_form_replay_spans(text: &str, labels: Option<&[sonojot_core::written_form::BioLabel]>) -> Vec<Value> {
    sonojot_core::written_form::apply_with_edits(text, labels).1
        .into_iter().map(|edit| serde_json::to_value(edit).unwrap()).collect()
}

fn run_written_form_replay(args: WrittenFormReplayArgs) -> Result<(), String> {
    let cases = read_written_form_cases(&args)?;
    if let Some(package) = &args.package {
        std::env::set_var("SONOJOT_PUNCT_MODEL_DIR", package);
    }
    if let Some(parent) = args.output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = OpenOptions::new().write(true).create_new(true).open(&args.output)
        .map_err(|error| format!("Cannot create replay output: {error}"))?;
    let mut output = BufWriter::new(file);
    let mut changed = 0usize;
    let mut errors = 0usize;
    for case in &cases {
        let row = evaluate_written_form(&args, case);
        errors += usize::from(!row["error"].is_null());
        changed += usize::from(row["changed"] == true);
        serde_json::to_writer(&mut output, &row).map_err(|error| error.to_string())?;
        output.write_all(b"\n").and_then(|_| output.flush()).map_err(|error| error.to_string())?;
    }
    sonojot_core::punctuation::release_cached();
    if errors > 0 { return Err(format!("{errors} written-form replay rows failed.")); }
    eprintln!("written-form replay: rows={} changed={changed}", cases.len());
    Ok(())
}

fn main() {
    sonojot_core::punctuation::dispatch_worker();
    recognition::dispatch_worker();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().is_some_and(|arg| arg == "--prepare-capture") {
        let result = if arguments.len() == 4 && arguments[2] == "--prepared-out" {
            prepare_capture_replay(Path::new(&arguments[1]), Path::new(&arguments[3]))
        } else { Err("Use --prepare-capture cases.jsonl --prepared-out DIRECTORY.".into()) };
        if let Err(error) = result { eprintln!("fidelity_eval: {error}"); std::process::exit(1); }
        return;
    }
    if arguments
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        print!("{HELP}");
        return;
    }
    if arguments.iter().any(|arg| arg == "--written-form-replay") {
        if let Err(error) = WrittenFormReplayArgs::parse(arguments).and_then(run_written_form_replay) {
            eprintln!("fidelity_eval: {error}");
            std::process::exit(1);
        }
        return;
    }
    std::env::set_var("MOONSHINE_ORT_SINGLE_THREAD", "1");
    let result = Args::parse(arguments).and_then(run);
    sonojot_core::punctuation::release_cached();
    cleanup::shutdown();
    if let Err(error) = result {
        eprintln!("fidelity_eval: {error}");
        std::process::exit(1);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureReplayCase {
    id: String,
    audio_path: PathBuf,
    stop_frame: Option<usize>,
    cue_frame: Option<u64>,
    post_roll: String,
    suppress_cue: bool,
}

fn prepare_capture_replay(input: &Path, out: &Path) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let lines = fs::read_to_string(input).map_err(|e| e.to_string())?;
    let cases: Vec<CaptureReplayCase> = lines.lines().filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    if cases.is_empty() { return Err("Capture replay has no cases.".into()); }
    fs::create_dir(out).map_err(|e| e.to_string())?;
    let mut inference = OpenOptions::new().write(true).create_new(true).open(out.join("inference.jsonl")).map_err(|e| e.to_string())?;
    let mut receipts = OpenOptions::new().write(true).create_new(true).open(out.join("capture.jsonl")).map_err(|e| e.to_string())?;
    let mut ids = HashSet::new();
    for (index, case) in cases.into_iter().enumerate() {
        if case.id.is_empty() || !ids.insert(case.id.clone()) { return Err("Empty or duplicate capture replay ID.".into()); }
        let path = if case.audio_path.is_absolute() { case.audio_path } else {
            input.parent().unwrap_or(Path::new(".")).join(case.audio_path)
        };
        let (spec, frames, values) = asr::evaluation_wav_samples(&path)?;
        // These short development fixtures are deliberately bounded. The
        // streaming evaluator remains the path for long recordings.
        if frames > u64::from(spec.sample_rate) * 120 { return Err("Capture replay fixtures must be at most 120 seconds.".into()); }
        let samples: Vec<f32> = values.collect::<Result<_, _>>()?;
        let mono: Vec<f32> = samples.chunks(spec.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / f32::from(spec.channels)).collect();
        let stop = case.stop_frame.unwrap_or(mono.len());
        if stop > mono.len() { return Err("Stop position is outside source audio.".into()); }
        let cutoff = match case.post_roll.as_str() {
            "none" => mono.len(),
            "fixed60" => (stop + (u64::from(spec.sample_rate) * 60).div_ceil(1000) as usize).min(mono.len()),
            "adaptive" => sonojot_core::audio::replay_capture_post_roll(&mono, spec.sample_rate, stop)?,
            _ => return Err("Unknown capture replay post-roll policy.".into()),
        };
        let mut asr_audio = mono[..cutoff].to_vec();
        let suppressed = if case.suppress_cue {
            sonojot_core::audio::replay_suppress_start_cue(&mut asr_audio, spec.sample_rate, 0,
                case.cue_frame.ok_or("Cue suppression requires cue_frame.")?)?
        } else { 0 };
        let target = out.join(format!("{index:04}.wav"));
        let mut writer = hound::WavWriter::create(&target, hound::WavSpec {
            channels: 1, sample_rate: spec.sample_rate, bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        }).map_err(|e| e.to_string())?;
        for sample in asr_audio { writer.write_sample(sample).map_err(|e| e.to_string())?; }
        writer.finalize().map_err(|e| e.to_string())?;
        writeln!(inference, "{}", json!({"id":case.id,"mode":"as_heard","audio_path":target.canonicalize().map_err(|e| e.to_string())?}))
            .map_err(|e| e.to_string())?;
        writeln!(receipts, "{}", json!({"id":case.id,"source":path,
            "source_sha256":format!("{:x}", Sha256::digest(fs::read(&path).map_err(|e| e.to_string())?)),
            "asr_audio_sha256":format!("{:x}", Sha256::digest(fs::read(&target).map_err(|e| e.to_string())?)),
            "source_frames":mono.len(),"sample_rate":spec.sample_rate,"stop_frame":stop,
            "cutoff_frame":cutoff,"post_roll_frames":cutoff.saturating_sub(stop),
            "post_roll_ms":cutoff.saturating_sub(stop) as f64 * 1000.0 / f64::from(spec.sample_rate),
            "post_roll_policy":case.post_roll,"cue_frame":case.cue_frame,"suppressed_frames":suppressed,
            "source_unchanged":true,"scope":"Offline replay of production capture policies; no microphone hardware or playback timing measured"}))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(extra: &[&str]) -> Args {
        Args::parse(
            [
                "--input",
                "cases.jsonl",
                "--output",
                "results.jsonl",
                "--models-root",
                "models",
            ]
            .into_iter()
            .chain(extra.iter().copied())
            .map(str::to_owned),
        )
        .unwrap()
    }

    fn text_case(text: &str) -> (Case, Value) {
        let value = json!({"id": "unicode", "mode": "clean", "input_text": text,
            "reference": "This must never enter the cleanup source."});
        (serde_json::from_value(value.clone()).unwrap(), value)
    }

    #[test]
    fn written_form_flags_are_explicit_and_need_no_models_for_standalone_replay() {
        let required = ["--written-form-replay", "text", "--input", "in", "--output", "out"];
        let parsed = WrittenFormReplayArgs::parse(required.into_iter().map(str::to_owned)).unwrap();
        assert_eq!(parsed.mode, WrittenFormMode::Rules);
        assert!(!parsed.span);
        for extra in [vec!["--written-form", "yes"],
            vec!["--models-root", "models"], vec!["--asr-model", "unavailable"],
            vec!["--input", "duplicate"], vec!["--punctuation-prewarm", "true"]]
        {
            assert!(WrittenFormReplayArgs::parse(required.into_iter().chain(extra).map(str::to_owned)).is_err());
        }
        let routed = WrittenFormReplayArgs::parse(required.into_iter()
            .chain(["--written-form", "model"]).map(str::to_owned)).unwrap();
        assert_eq!(routed.mode, WrittenFormMode::Model);
        for variant in ["host", "model", "daily"] {
            assert_eq!(args(&["--variant", variant, "--written-form", "off"]).written_form, WrittenFormMode::Off);
        }
        for extra in [vec!["--variant", "raw"], vec!["--variant", "baseline"],
            vec!["--variant", "host", "--punctuation-labels", "true"],
            vec!["--variant", "model", "--trace-cleanup", "true"]]
        {
            assert!(Args::parse(["--input", "in", "--output", "out", "--models-root", "models",
                "--written-form", "rules"].into_iter().chain(extra).map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn standalone_written_form_replays_text_spans_and_byte_identical_off() {
        let mut args = WrittenFormReplayArgs {
            input: "unused".into(), output: "unused".into(), span: false, mode: WrittenFormMode::Rules, root: None, package: None,
        };
        let mut case: WrittenFormReplayCase = serde_json::from_value(json!({
            "id": "public-example", "text": "  ten dollars\n" })).unwrap();
        let row = evaluate_written_form(&args, &case);
        assert_eq!(row["final_text"], "  $10\n");
        assert_eq!(row["raw_text"], case.text);
        assert_eq!(row["changed"], true);
        assert_eq!(row["spans"], json!([{"start": 0, "end": 2, "class": "MONEY", "rendered": "$10"}]));
        args.mode = WrittenFormMode::Off;
        let row = evaluate_written_form(&args, &case);
        assert_eq!(row["final_text"], case.text);
        assert_eq!(row["changed"], false);
        assert_eq!(row["spans"], json!([]));
        args.mode = WrittenFormMode::Rules;
        args.span = true;
        case = serde_json::from_value(json!({"id": "public-span", "text": "ten dollars", "cls": "MONEY"})).unwrap();
        let row = evaluate_written_form(&args, &case);
        assert_eq!(row["rendered"], "$10");
        assert_eq!(row["final_text"], "$10");
        case.text = "many dollars".into();
        let row = evaluate_written_form(&args, &case);
        assert!(row["rendered"].is_null());
        assert_eq!(row["final_text"], case.text);
    }

    #[test]
    fn model_replay_routes_saved_labels_and_keeps_absent_head_fallback() {
        let args = WrittenFormReplayArgs {
            input: "unused".into(), output: "unused".into(), span: false,
            mode: WrittenFormMode::Model, root: None, package: None,
        };
        let mut case: WrittenFormReplayCase = serde_json::from_value(json!({
            "id": "routing", "text": "Use GPT six and pay ten dollars.",
            "labels": ["O", "O", "O", "O", "O", "O", "O"],
        })).unwrap();
        let row = evaluate_written_form(&args, &case);
        assert_eq!(row["final_text"], "Use GPT-6 and pay ten dollars.");
        assert_eq!(row["written_form"], "model");
        assert_eq!(row["span_head"], true);
        case.labels = None;
        let fallback = evaluate_written_form(&args, &case);
        assert_eq!(fallback["final_text"], "Use GPT-6 and pay $10.");
        assert_eq!(fallback["span_head"], false);
    }

    #[test]
    fn written_form_trace_reconstructs_output_and_omits_protected_spans() {
        for text in [
            "Pay ten dollars and twenty euros.",
            "Keep \"ten dollars\" and `twenty euros`; pay thirty dollars.",
            "naïve\t ten dollars,\nthen twenty percent.",
            "ten\ndollars and https://example.com/ten dollars",
            "two checks and 2025",
        ] {
            let spans = written_form_replay_spans(text, None);
            let words: Vec<&str> = text.split_whitespace().collect();
            let mut rebuilt = String::new();
            let mut cursor = 0;
            for span in spans {
                let first = words[span["start"].as_u64().unwrap() as usize];
                let last = words[span["end"].as_u64().unwrap() as usize - 1];
                let start = first.as_ptr() as usize - text.as_ptr() as usize;
                let end = last.as_ptr() as usize - text.as_ptr() as usize + last.len();
                rebuilt.push_str(&text[cursor..start]);
                rebuilt.push_str(span["rendered"].as_str().unwrap());
                cursor = end;
            }
            rebuilt.push_str(&text[cursor..]);
            assert_eq!(rebuilt, sonojot_core::written_form::apply(text, None));
        }
        assert!(written_form_replay_spans("\"ten dollars\" `twenty percent`", None).is_empty());
    }

    #[test]
    fn standalone_written_form_validates_before_creating_output_and_never_overwrites() {
        let directory = std::env::temp_dir().join(format!("sonojot-written-form-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let input = directory.join("input.jsonl");
        let output = directory.join("output.jsonl");
        let make_args = |span| WrittenFormReplayArgs { input: input.clone(), output: output.clone(), span, mode: WrittenFormMode::Off, root: None, package: None };
        let valid = json!({"id": "public-example", "text": "  naïve\n ten dollars  "}).to_string();
        for bad in [format!("{valid}\n{valid}\n"), "{\"id\":\"x\",\"text\":[\"DO_NOT_LOG_THIS\"]}".into(),
            json!({"id": "x", "text": "plain", "class": "UNKNOWN_PRIVATE_VALUE"}).to_string()]
        {
            fs::write(&input, bad).unwrap();
            let error = run_written_form_replay(make_args(false)).unwrap_err();
            assert!(!error.contains("DO_NOT_LOG_THIS"));
            assert!(!error.contains("UNKNOWN_PRIVATE_VALUE"));
            assert!(!output.exists());
        }
        fs::write(&input, &valid).unwrap();
        assert!(run_written_form_replay(make_args(true)).is_err());
        assert!(!output.exists());
        run_written_form_replay(make_args(false)).unwrap();
        let bytes = fs::read(&output).unwrap();
        let row: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(row["final_text"], "  naïve\n ten dollars  ");
        assert!(run_written_form_replay(make_args(false)).is_err());
        assert_eq!(fs::read(&output).unwrap(), bytes);
        fs::remove_file(input).unwrap();
        fs::remove_file(output).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn production_written_form_switch_preserves_raw_text_and_cleanup_modes() {
        let enabled = args(&["--variant", "host", "--written-form", "rules"]);
        let disabled = args(&["--variant", "host", "--written-form", "off"]);
        let (mut case, input) = text_case("Pay ten dollars.");
        for mode in ["clean", "format_only", "as_heard"] {
            case.mode = mode.into();
            let on = evaluate(&enabled, &case, &input, &mut None, 0, true);
            let off = evaluate(&disabled, &case, &input, &mut None, 0, true);
            assert!(on["error"].is_null());
            assert!(off["error"].is_null());
            assert_eq!(on["raw_text"], input["input_text"]);
            assert_eq!(off["raw_text"], input["input_text"]);
            assert!(off["final_text"].as_str().unwrap().contains("ten dollars"));
            if mode == "clean" {
                assert!(on["final_text"].as_str().unwrap().contains("$10"));
            } else {
                assert_eq!(on["final_text"], off["final_text"]);
            }
        }
        case.mode = "clean".into();
        case.scope = Some("meeting".into());
        let on = evaluate(&enabled, &case, &input, &mut None, 0, true);
        let off = evaluate(&disabled, &case, &input, &mut None, 0, true);
        assert_eq!(on["final_text"], off["final_text"]);
        assert!(on["written_form"].is_null());
    }

    #[test]
    fn layout_defaults_to_rules_then_model_and_keeps_disabled_replay_unchanged() {
        let parsed = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507"]);
        assert_eq!(parsed.variant, "model");
        assert_eq!(parsed.policy, "conservative");
        assert!(parsed.resource_lease);
        let disabled = args(&["--smart-layout", "false"]);
        assert_eq!(disabled.variant, "raw");
        assert_eq!(disabled.policy, "daily");
        assert!(!disabled.resource_lease);
        let (case, input) = text_case("  keep teh bytes  ");
        let row = evaluate(&disabled, &case, &input, &mut None, 0, true);
        assert_eq!(row["final_text"], input["input_text"]);
        assert!(row.get("layout").is_none());
    }

    #[test]
    fn layout_short_message_needs_no_lease_or_model() {
        let parsed = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507"]);
        let (case, input) = text_case("Please keep this message unchanged.");
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert!(row["error"].is_null());
        assert_eq!(row["layout"]["reason"], "short_text");
        assert_eq!(row["layout"]["class"], "A");
        assert!(row["layout"]["probabilities"].is_null());
        assert_eq!(row["resource_lease"]["acquired"], false);
        assert_eq!(row["final_text"], input["input_text"]);
    }

    #[test]
    fn layout_app_context_is_optional_and_eval_only() {
        let enabled = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507"]);
        let (mut case, input) = text_case("Please keep this message unchanged.");
        assert!(case.app_context.is_none());
        case.app_context = Some("an email app (Mail, Outlook or Gmail)".into());
        assert!(validate_layout_case(&enabled, &case).is_ok());
        assert!(validate_layout_case(&args(&[]), &case).is_err());
        let row = evaluate(&enabled, &case, &input, &mut None, 0, true);
        assert_eq!(row["layout"]["app_context"], case.app_context.as_deref().unwrap());
        assert_eq!(row["layout"]["policy_sha256"],
            cleanup::layout::policy_hash_with_context(case.app_context.as_deref()));
        for invalid in ["".to_owned(), "notes\nextra".to_owned(), "x".repeat(201)] {
            case.app_context = Some(invalid);
            assert!(validate_layout_case(&enabled, &case).is_err());
        }
    }

    #[test]
    fn frozen_layout_references_pass_the_production_guard() {
        let corpus = include_str!("../../../evaluation/fixtures/smart-layout-2026.jsonl");
        let mut count = 0;
        for line in corpus.lines().filter(|line| !line.trim().is_empty()) {
            let case: Value = serde_json::from_str(line).unwrap();
            assert_eq!(cleanup::layout::guard(case["input_text"].as_str().unwrap(),
                case["expected_text"].as_str().unwrap()), Ok(()), "{}", case["id"]);
            count += 1;
        }
        assert_eq!(count, 36);
    }

    #[test]
    fn layout_cancel_stop_and_as_heard_preserve_prior_text_without_a_lease() {
        for (flag, reason) in [("--cancel-after-ms", "cancelled"), ("--stop-after-ms", "cleanup_stopped")] {
            let parsed = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507", flag, "0"]);
            let (case, input) = text_case("  Keep every original byte, including these spaces.  ");
            let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
            assert_eq!(row["layout"]["reason"], reason);
            assert_eq!(row["layout"]["applied"], false);
            assert_eq!(row["layout"]["input_text"], input["input_text"]);
            assert_eq!(row["final_text"], row["fallback_text"]);
            assert_eq!(row["resource_lease"]["acquired"], false);
            assert_eq!(row["resource_lease"]["released"], true);
            assert_eq!(row["error"].is_null(), reason != "cancelled");
        }
        let parsed = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507"]);
        let (mut case, mut input) = text_case("  raw\nbytes and um no changes  ");
        case.mode = "as_heard".into();
        input["mode"] = json!("as_heard");
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert_eq!(row["layout"]["reason"], "as_heard");
        assert_eq!(row["final_text"], input["input_text"]);
        assert_eq!(row["resource_lease"]["acquired"], false);
        assert!(row["error"].is_null());
    }

    #[test]
    fn layout_rejects_unsupported_models_routes_and_audio_before_work() {
        let required = ["--input", "in", "--output", "out", "--models-root", "models", "--smart-layout", "true"];
        for model in ["off", "local-rules", "speakoflow-mini-q4_k_m", "unknown-model"] {
            assert!(Args::parse(required.into_iter().chain(["--cleanup-model", model]).map(str::to_owned)).is_err());
        }
        for extra in [
            vec!["--resource-lease", "false"], vec!["--variant", "raw"],
            vec!["--variant", "host"], vec!["--variant", "daily"],
            vec!["--variant", "structured"], vec!["--variant", "baseline"],
            vec!["--cleanup-policy", "daily"], vec!["--cleanup-policy", "targeted"],
            vec!["--meeting-rules-only", "true"], vec!["--trace-cleanup", "true"],
            vec!["--replay-mode", "stream"], vec!["--asr-model", "moonshine-medium"],
            vec!["--cold-asr", "true"], vec!["--cleanup-input-envelope", "json"],
            vec!["--prompt-file", "never-read.txt"], vec!["--daily-clean-prompt-file", "never-read.txt"],
            vec!["--force-rejection", "true"],
        ] {
            assert!(Args::parse(required.into_iter()
                .chain(["--cleanup-model", "qwen3-4b-instruct-2507"])
                .chain(extra).map(str::to_owned)).is_err());
        }
        assert!(Args::parse(["--input", "in", "--output", "out", "--models-root", "models",
            "--smart-layout", "yes"].into_iter().map(str::to_owned)).is_err());
        let parsed = args(&["--smart-layout", "true", "--cleanup-model", "qwen3-4b-instruct-2507"]);
        let (mut case, input) = text_case("text");
        case.scope = Some("meeting".into());
        assert!(validate_layout_case(&parsed, &case).is_err());
        case.scope = None;
        case.input_text = None;
        case.audio_path = Some(PathBuf::from("never-open.wav"));
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert!(row["error"].as_str().unwrap().contains("requires input_text"));
        assert!(row["asr"].is_null());
    }

    #[test]
    fn smart_model_alias_and_meeting_route_are_explicit() {
        let parsed = args(&["--variant", "model", "--cleanup-policy", "conservative",
            "--cleanup-model", "speakoflow-mini-q4_k_m", "--meeting-rules-only", "true"]);
        assert_eq!(parsed.model, "speakoflow-mini-q4_k_m");
        let (mut case, input) = text_case("Keep this meeting text.");
        case.scope = Some("meeting".into());
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert!(row["error"].is_null());
        assert_eq!(row["route"], "meeting_rules_only");
        assert_eq!(row["cleanup"]["requested_model"], "local-rules");
        assert_eq!(row["fallback_text"], row["final_text"]);
        assert!(Args::parse(["--input", "in", "--output", "out", "--models-root", "models",
            "--model", "off", "--cleanup-model", "off"].into_iter().map(str::to_owned)).is_err());
    }

    #[test]
    fn zero_delay_cancellation_is_set_before_production_cleanup() {
        let parsed = args(&["--variant", "host", "--cleanup-policy", "conservative",
            "--cancel-after-ms", "0"]);
        let (case, input) = text_case("Keep the complete original.");
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert_eq!(row["interruption"]["cancel_requested"], true);
        assert_eq!(row["status"], "cancelled");
        assert_eq!(row["raw_text"], "Keep the complete original.");
    }

    #[test]
    fn raw_replay_preserves_utf8_and_does_not_use_reference_or_audio() {
        let args = args(&[]);
        let (mut case, mut input) = text_case("  naïve café — A\u{301}\nAPI_v2  ");
        case.audio_path = Some(PathBuf::from("missing-audio.wav"));
        input["audio_path"] = json!("missing-audio.wav");
        let row = evaluate(&args, &case, &input, &mut None, 0, true);
        assert!(row["error"].is_null());
        assert!(row["asr"].is_null());
        assert_eq!(row["raw_text"], input["input_text"]);
        assert_eq!(row["final_text"], input["input_text"]);
        assert_eq!(row["repetition"], 0);
    }

    #[test]
    fn production_host_route_has_no_model_and_respects_original_mode() {
        let args = args(&["--variant", "host"]);
        let (mut case, input) = text_case("  keep teh bytes  ");
        case.mode = "as_heard".into();
        let row = evaluate(&args, &case, &input, &mut None, 0, true);
        assert!(row["error"].is_null());
        assert_eq!(row["final_text"], input["input_text"]);
        assert_eq!(row["cleanup"]["requested_model"], "local-rules");
        assert_eq!(row["cleanup"]["result"]["policyVersion"], "daily-v8");
        assert!(row["punctuation_stage"].is_null());
    }

    #[test]
    fn punctuation_directory_flag_and_dictation_scope_are_explicit() {
        let parsed = args(&["--variant", "host", "--punctuation-model-dir", "punct-model"]);
        assert_eq!(parsed.punctuation_model_dir, Some(PathBuf::from("punct-model")));
        let (mut case, input) = text_case("Keep the. Complete original.");
        for mode in ["clean", "format_only"] {
            case.mode = mode.into();
            let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
            assert!(row["error"].is_null());
            assert!(row["punctuation_stage"].as_str().is_some_and(|s| s.starts_with("punctuation_")));
            assert!(row["timings"]["punctuation_ms"].is_number());
            assert_eq!(row["raw_text"], input["input_text"]);
        }
        // Scope alone excludes meetings, even without --meeting-rules-only.
        case.scope = Some("meeting".into());
        let row = evaluate(&parsed, &case, &input, &mut None, 0, true);
        assert!(row["punctuation_stage"].is_null());
        assert_eq!(row["final_text"], "Keep the. Complete original.");
    }

    #[test]
    fn daily_clean_prompt_file_is_diagnostic_and_clean_only() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../evaluation/prompts/cleanup-copy-edit-2026.txt");
        let prompt = fs::read_to_string(&path).unwrap();
        let args = args(&[
            "--variant",
            "model",
            "--trace-cleanup",
            "true",
            "--daily-clean-prompt-file",
            path.to_str().unwrap(),
        ]);
        // Model off exercises prompt selection and host safeguards without
        // starting a runtime or running inference.
        for selected_mode in ["clean", "format_only", "as_heard"] {
            let input = json!({"id": "prompt-scope", "mode": selected_mode,
                "input_text": "Keep the literal phrase and do not send it."});
            let case: Case = serde_json::from_value(input.clone()).unwrap();
            let row = evaluate(&args, &case, &input, &mut None, 0, true);
            assert!(row["error"].is_null());
            assert_eq!(row["raw_text"], input["input_text"]);
            assert_eq!(
                row["daily_clean_prompt_override"]["applied"],
                selected_mode == "clean"
            );
            let expected = if selected_mode == "clean" {
                prompt.as_str()
            } else {
                cleanup::CleanupPolicy::daily().system_for(mode(selected_mode).unwrap())
            };
            assert_eq!(row["cleanup"]["system_prompt"], expected);
            assert_eq!(
                row["cleanup"]["generation"]["experimental_clean_prompt_override"],
                selected_mode == "clean"
            );
            if selected_mode == "clean" {
                assert_eq!(
                    row["cleanup"]["system_prompt_sha256"],
                    row["daily_clean_prompt_override"]["sha256"]
                );
            }
        }
    }

    #[test]
    fn targeted_policy_is_explicit_and_has_no_prompt_overrides() {
        assert_eq!(args(&[]).cleanup_policy().version, "daily-v8");
        for variant in ["host", "model"] {
            let parsed = args(&["--variant", variant, "--cleanup-policy", "targeted"]);
            assert_eq!(parsed.cleanup_policy().version, "targeted-v1");
        }
        let traced = args(&[
            "--variant", "model", "--cleanup-policy", "targeted", "--trace-cleanup", "true",
        ]);
        assert!(traced.trace_cleanup);
        for extra in [
            vec!["--variant", "raw"],
            vec!["--variant", "baseline"],
            vec!["--variant", "structured"],
            vec!["--variant", "daily"],
            vec!["--variant", "host", "--trace-cleanup", "true"],
            vec!["--variant", "model", "--cleanup-input-envelope", "json"],
            vec!["--variant", "model", "--trace-cleanup", "true", "--daily-clean-prompt-file", "not-read.txt"],
            vec!["--variant", "model", "--prompt-file", "not-read.txt"],
        ] {
            let result = Args::parse(
                ["--input", "in", "--output", "out", "--models-root", "models", "--cleanup-policy", "targeted"]
                    .into_iter().chain(extra).map(str::to_owned),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn conservative_policy_requires_plain_host_or_model_route() {
        for variant in ["host", "model"] {
            let parsed = args(&["--variant", variant, "--cleanup-policy", "conservative"]);
            assert_eq!(parsed.cleanup_policy().version, "conservative-v1");
            assert!(!parsed.trace_cleanup);
            assert_eq!(parsed.cleanup_input_envelope, "plain");
        }
        for extra in [
            vec!["--variant", "raw"],
            vec!["--variant", "baseline"],
            vec!["--variant", "structured"],
            vec!["--variant", "daily"],
            vec!["--variant", "host", "--trace-cleanup", "true"],
            vec!["--variant", "model", "--trace-cleanup", "true"],
            vec!["--variant", "model", "--cleanup-input-envelope", "json"],
            vec!["--variant", "model", "--daily-clean-prompt-file", "not-read.txt"],
            vec!["--variant", "model", "--prompt-file", "not-read.txt"],
        ] {
            let result = Args::parse(
                ["--input", "in", "--output", "out", "--models-root", "models", "--cleanup-policy", "conservative"]
                    .into_iter().chain(extra).map(str::to_owned),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn llama_asr_accepts_language_and_section_experiments_without_changing_sherpa_qwen() {
        let required = ["--input", "in", "--output", "out", "--models-root", "models"];
        for id in ["qwen3-asr-1.7b-q8-cuda", "qwen3-asr-1.7b-q4-cpu",
            "qwen3-asr-1.7b-q8-metal", "qwen3-asr-1.7b-q4-metal",
            "qwen3-asr-1.7b", "qwen3-asr-1.7b-fp16-cuda"] {
            for extra in [vec!["--language", "en"], vec!["--max-chunk-seconds", "28"]] {
                let parsed = Args::parse(required.into_iter()
                    .chain(["--asr-model", id]).chain(extra).map(str::to_owned));
                assert_eq!(parsed.is_ok(), id.contains("-q8-") || id.contains("-q4-"), "{id}");
            }
        }
    }

    #[test]
    fn invalid_flags_do_not_silently_change_production_policy() {
        let required = [
            "--input",
            "in",
            "--output",
            "out",
            "--models-root",
            "models",
        ];
        for extra in [
            vec!["--repeat", "0"],
            vec!["--unknown", "yes"],
            vec!["--cold-asr", "yes"],
            vec!["--replay-mode", "virtual"],
            vec!["--pace", "fake"],
            vec!["--callback-ms", "0"],
            vec!["--callback-ms", "1001"],
            vec!["--trace-cleanup", "true"],
            vec!["--daily-clean-prompt-file", "not-read.txt"],
            vec![
                "--variant",
                "model",
                "--daily-clean-prompt-file",
                "not-read.txt",
            ],
            vec![
                "--variant",
                "model",
                "--trace-cleanup",
                "true",
                "--cleanup-policy",
                "legacy",
                "--daily-clean-prompt-file",
                "not-read.txt",
            ],
            vec!["--cleanup-input-envelope", "xml"],
            vec!["--cleanup-input-envelope", "json"],
            vec![
                "--variant",
                "model",
                "--cleanup-policy",
                "legacy",
                "--cleanup-input-envelope",
                "json",
            ],
            vec![
                "--variant",
                "model",
                "--cleanup-policy",
                "legacy",
                "--trace-cleanup",
                "true",
            ],
            vec!["--max-chunk-seconds", "30"],
            vec![
                "--asr-model",
                "moonshine-medium",
                "--max-chunk-seconds",
                "28",
            ],
            vec!["--variant", "model", "--force-rejection", "true"],
            vec!["--variant", "daily", "--cleanup-policy", "legacy"],
        ] {
            assert!(Args::parse(required.into_iter().chain(extra).map(str::to_owned)).is_err());
        }
    }

    struct TestWav(PathBuf);
    impl TestWav {
        fn stereo(name: &str, frames: usize) -> Self {
            let path = std::env::temp_dir()
                .join(format!("sonojot-replay-{}-{name}.wav", std::process::id()));
            let mut wav = hound::WavWriter::create(
                &path,
                hound::WavSpec {
                    channels: 2,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for _ in 0..frames {
                wav.write_sample(8192i16).unwrap();
                wav.write_sample(16384i16).unwrap();
            }
            wav.finalize().unwrap();
            Self(path)
        }
    }
    impl Drop for TestWav {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn realtime_producer_finishes_without_decoder_and_backlog_preserves_tail() {
        // No consumer runs until the producer finishes. A decode-paced feeder
        // would deadlock here; the production spool retains all source frames.
        let wav = TestWav::stereo("independent", 805);
        let mut capture =
            ReplayCapture::start(&wav.0, "realtime", 20, Arc::new(AtomicBool::new(false))).unwrap();
        let report = capture.producer.take().unwrap().join().unwrap();
        assert!(report.error.is_none());
        assert_eq!(report.expected_frames, 805);
        assert_eq!(report.enqueued_frames, 805);
        assert_eq!(
            report
                .events
                .iter()
                .map(|event| event.frames)
                .collect::<Vec<_>>(),
            [320, 320, 165]
        );
        for event in &report.events {
            assert!(event.enqueued_ms >= event.scheduled_enqueue_ms);
        }
        let mut frames = 0;
        loop {
            match capture
                .backlog
                .as_mut()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
            {
                AudioMessage::Chunk {
                    samples,
                    captured_at,
                    ..
                } => {
                    assert_eq!(
                        captured_at,
                        Duration::from_secs_f64(frames as f64 / 16000.0)
                    );
                    assert!(samples.iter().all(|value| *value == 0.375));
                    frames += samples.len();
                }
                AudioMessage::End => break,
                AudioMessage::Error(error) => panic!("{error}"),
            }
        }
        assert_eq!(frames, 805);
    }

    #[test]
    fn capture_preparation_uses_shared_policies_and_preserves_source_bytes() {
        let root = std::env::temp_dir().join(format!("sonojot-capture-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let source = root.join("source.wav");
        let mut writer = hound::WavWriter::create(&source, hound::WavSpec {
            channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
        }).unwrap();
        for _ in 0..16_000 { writer.write_sample(0i16).unwrap(); }
        writer.finalize().unwrap();
        let original = fs::read(&source).unwrap();
        let manifest = root.join("cases.jsonl");
        fs::write(&manifest, json!({"id":"quiet","audio_path":source,"stop_frame":3_200,
            "cue_frame":160,"post_roll":"adaptive","suppress_cue":true}).to_string()).unwrap();
        let out = root.join("prepared");
        prepare_capture_replay(&manifest, &out).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        let receipt: Value = serde_json::from_str(&fs::read_to_string(out.join("capture.jsonl")).unwrap()).unwrap();
        assert_eq!(receipt["post_roll_frames"], 1_920);
        assert_eq!(receipt["cutoff_frame"], 5_120);
        assert_eq!(receipt["suppressed_frames"], 2_480);
        let inference: Value = serde_json::from_str(&fs::read_to_string(out.join("inference.jsonl")).unwrap()).unwrap();
        assert_eq!(inference.as_object().unwrap().len(), 3);
        assert!(prepare_capture_replay(&manifest, &out).is_err());
        for path in [out.join("0000.wav"), out.join("capture.jsonl"), out.join("inference.jsonl"), manifest, source] {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(out).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn burst_overflow_is_explicit_and_cancellation_sends_no_audio() {
        let wav = TestWav::stereo("overflow", 1000);
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(AtomicBool::new(false));
        let thread_ended = ended.clone();
        let path = wav.0.clone();
        let thread = std::thread::spawn(move || {
            produce_audio(
                &path,
                false,
                20,
                Instant::now(),
                tx,
                stop,
                thread_ended,
                cancel,
            )
        });
        // Wait for the first callback to be queued, then keep it undrained until
        // the producer detects overflow. Terminal sends may block, like capture.
        let wait = Instant::now();
        while !ended.load(Ordering::Acquire) && wait.elapsed() < Duration::from_secs(2) {
            std::thread::yield_now();
        }
        assert!(ended.load(Ordering::Acquire));
        assert!(matches!(rx.recv().unwrap(), AudioMessage::Chunk { .. }));
        let terminal = rx.recv().unwrap();
        assert!(matches!(terminal, AudioMessage::Error(ref error) if error.contains("overflowed")));
        assert!(matches!(rx.recv().unwrap(), AudioMessage::End));
        let report = thread.join().unwrap();
        assert_eq!(report.enqueued_frames, 320);
        assert!(report.error.is_some());

        let (tx, rx) = mpsc::sync_channel(3);
        let report = produce_audio(
            &wav.0,
            false,
            20,
            Instant::now(),
            tx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(true)),
        );
        assert_eq!(report.enqueued_frames, 0);
        assert!(report.error.is_some());
        assert!(matches!(rx.recv().unwrap(), AudioMessage::Error(_)));
    }

    #[test]
    fn unpaced_has_no_synthetic_stop_latency_and_pending_counts_active_add() {
        let data = ReplayEvidence {
            epoch: Instant::now(),
            producer: ProducerReport {
                sample_rate: 16000,
                channels: 1,
                expected_frames: 320,
                enqueued_frames: 320,
                events: vec![EnqueueEvent {
                    start_frame: 0,
                    frames: 320,
                    captured_at_ms: 0.0,
                    scheduled_enqueue_ms: 20.0,
                    enqueue_start_ms: 0.99,
                    enqueued_ms: 1.0,
                }],
                end_ms: Some(2.0),
                error: None,
            },
            consumed_frames: 320,
            spool_peak_bytes: 1304,
            decode: vec![DecodeEvent {
                start_frame: 0,
                frames: 320,
                start_ms: 1.1,
                end_ms: 5.0,
            }],
            updates: vec![],
            first_text_ms: Some(6.0),
            first_text_kind: Some("final"),
            raw_ready_ms: Some(6.0),
            finalize_start_ms: Some(5.0),
            stream_start_ms: 0.0,
            stream_ready_ms: 0.5,
            asr_active_ms: 5.4,
        };
        let burst = data.json(
            &args(&["--replay-mode", "stream", "--pace", "unpaced"]),
            Some(7.0),
        );
        assert!(burst["final_audio_to_raw_ms"].is_null());
        assert!(burst["final_audio_to_final_ms"].is_null());
        assert_eq!(burst["producer_end_to_raw_ms"], 4.0);
        assert_eq!(burst["peak_pending_frames"], 320);
        assert_eq!(burst["frames_complete"], true);
        let paced = data.json(&args(&["--replay-mode", "stream"]), Some(7.0));
        assert_eq!(paced["final_audio_to_raw_ms"], 5.0);
        assert_eq!(paced["final_audio_to_final_ms"], 6.0);
    }
}
