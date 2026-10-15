//! Tauri-independent dictation engine. Callbacks contain session IDs; the
//! desktop layer must publish only its current session's Completed event.
use crate::{
    asr,
    audio::{AudioMessage, Capture},
    audio_backlog::AudioBacklog,
    cleanup,
    domain::RecognitionCompleteness,
    resource_manager,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Sender},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct StartOptions {
    pub microphone: Option<String>,
    pub asr_model: String,
    pub cleanup_model: String,
    pub cleanup_mode: cleanup::Mode,
    /// Optional final formatting stage; model selection is snapshotted at start.
    pub smart_layout: bool,
    pub written_form: bool,
    pub notes_model: String,
    pub vocabulary: Vec<cleanup::Vocabulary>,
    /// The cleanup policy for this session: `conservative-v1` for normal runtime.
    /// The saved `cleanup_trial` bool is preserved in Settings for backwards
    /// migration/evidence only and no longer selects the runtime policy.
    pub policy: cleanup::CleanupPolicy,
    /// Enabled vocabulary entries with a non-empty scope. They reach the
    /// structured stage as confirmed entries with explicit user provenance;
    /// they are never derived from history.
    pub confirmed_vocabulary: Vec<crate::structured_clean::VocabularyEntry>,
    pub auto_stop_seconds: u64,
    /// Run the live preview beside the primary when its model is installed.
    pub live_preview: bool,
    /// No-history snapshot for this job (from `do_not_save_dictations`).
    /// When true, the job calls the owned per-job erase
    /// (`cleanup_resource::end_job_report`) under its still-held text lease
    /// before release; it never shuts down after release (that could kill a
    /// newly acquired notes job). New field with a Default so existing
    /// `..Default::default()` call sites keep compiling.
    pub no_history: bool,
    /// Store handle for post-ASR learned-vocabulary lookup (full raw
    /// utterance, mapped mode, ≤1536 budget). `None` in legacy/test paths
    /// (no learned injection). Never a global list at capture start: raw and
    /// scope evidence are unavailable until recognition finishes.
    pub store: Option<crate::store::Store>,
}
impl Default for StartOptions {
    fn default() -> Self {
        Self {
            microphone: None,
            asr_model: "moonshine-medium".into(),
            cleanup_model: "lfm2.5-1.2b".into(),
            cleanup_mode: cleanup::Mode::Clean,
            smart_layout: false,
            written_form: true,
            notes_model: "qwen3-4b-instruct-2507".into(),
            vocabulary: Vec::new(),
            policy: cleanup::CleanupPolicy::conservative(),
            confirmed_vocabulary: Vec::new(),
            auto_stop_seconds: 0,
            live_preview: false,
            no_history: false,
            store: None,
        }
    }
}
impl StartOptions {
    pub fn from_settings(
        settings: crate::domain::Settings,
        vocabulary: Vec<cleanup::Vocabulary>,
        confirmed_vocabulary: Vec<crate::structured_clean::VocabularyEntry>,
    ) -> Self {
        Self::from_settings_with_store(settings, vocabulary, confirmed_vocabulary, None)
    }

    /// Production path: `app::start_at_generation` passes `Some(store)` so
    /// `run_job` can inject `Store::learned_cleanup_vocabulary_for` after
    /// final ASR. Legacy/test callers pass `None` (no learned injection).
    /// Never injects a global learned list here: raw/scope are unavailable at
    /// capture start by contract.
    pub fn from_settings_with_store(
        settings: crate::domain::Settings,
        vocabulary: Vec<cleanup::Vocabulary>,
        confirmed_vocabulary: Vec<crate::structured_clean::VocabularyEntry>,
        store: Option<crate::store::Store>,
    ) -> Self {
        // Keep the resource coordinator on the current lifecycle choices.
        // Idempotent; the next lease acquire sees the new mode/retention.
        // Also ensures the singleton idle worker runs.
        resource_manager::configure(&settings);
        let _ = settings.cleanup_trial;
        Self {
            microphone: settings.microphone.clone(),
            asr_model: settings.asr_model.clone(),
            cleanup_model: settings.cleanup_model.clone(),
            smart_layout: settings.smart_layout,
            written_form: settings.written_form,
            notes_model: settings.notes_model.clone(),
            cleanup_mode: match settings.cleanup_mode {
                crate::domain::CleanupMode::Clean => cleanup::Mode::Clean,
                crate::domain::CleanupMode::Original => cleanup::Mode::Original,
                crate::domain::CleanupMode::Format => cleanup::Mode::Format,
            },
            vocabulary,
            policy: cleanup::CleanupPolicy::conservative(),
            confirmed_vocabulary,
            auto_stop_seconds: settings.auto_stop_seconds as u64,
            live_preview: settings.live_preview,
            no_history: settings.do_not_save_dictations,
            store,
        }
    }
}

/// Post-ASR learned vocabulary: full raw utterance + mapped mode, ≤1536
/// budget, explicit + confirmed-scoped only. Never at capture start, never
/// per-chunk, never to cloud. Failures fall back to explicit vocabulary only
/// (never fail dictation for a learning lookup).
fn learned_cleanup_for_job(
    store: &Option<crate::store::Store>,
    raw: &str,
    mode: cleanup::Mode,
) -> Vec<cleanup::Vocabulary> {
    let Some(store) = store.as_ref() else {
        return Vec::new();
    };
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let domain_mode = match mode {
        cleanup::Mode::Clean => crate::domain::CleanupMode::Clean,
        cleanup::Mode::Original => crate::domain::CleanupMode::Original,
        cleanup::Mode::Format => crate::domain::CleanupMode::Format,
    };
    // Store enforces opt-in, Clean-only, raw+scope evidence, ≤12 entries.
    store
        .learned_cleanup_vocabulary_for(raw, domain_mode, 1536)
        .unwrap_or_default()
}
#[derive(Clone, Debug)]
pub struct DictationResult {
    pub original_text: String,
    pub output_text: String,
    pub pre_model_text: Option<String>,
    pub publication_stage: Option<String>,
    pub guard_reason: Option<String>,
    pub recognition_completeness: RecognitionCompleteness,
    pub recognition_warnings: Vec<String>,
    pub asr_model: String,
    pub cleanup_status: String,
    /// Cleanup policy version and hash that produced `output_text`.
    pub policy_version: String,
    pub policy_hash: String,
    /// Time spent in the final ASR finish call, after draining captured audio.
    /// Load, live decoding, and Stop-to-result latency are separate diagnostics.
    pub asr_ms: u64,
    pub cleanup_ms: u64,
}
#[derive(Clone, Debug)]
pub enum Event {
    Progress {
        session_id: u64,
        progress: crate::domain::RecognitionProgress,
    },
    Started {
        session_id: u64,
    },
    Level {
        session_id: u64,
        rms: f32,
    },
    Interim {
        session_id: u64,
        text: String,
    },
    /// Live-preview draft after the primary's committed text. Display only.
    Preview {
        session_id: u64,
        text: String,
    },
    Finishing {
        session_id: u64,
    },
    Cleaning {
        session_id: u64,
        original_text: String,
    },
    Completed {
        session_id: u64,
        result: DictationResult,
    },
    Cancelled {
        session_id: u64,
    },
    Error {
        session_id: u64,
        message: String,
    },
}
type Callback = Arc<dyn Fn(Event) + Send + Sync>;
#[derive(Clone)]
struct Control {
    id: u64,
    started: Instant,
    stop_requested: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    stop_cleanup: Arc<AtomicBool>,
    capture_stop: Arc<AtomicBool>,
    start_cue: Option<crate::audio::StartCue>,
}
impl Control {
    fn request_capture_stop(&self) {
        if let Some(cue) = &self.start_cue { cue.request_stop(); }
        else { self.capture_stop.store(true, Ordering::Release); }
    }
    fn mark_stop(&self) {
        if let Ok(mut at) = self.stop_requested.lock() {
            if at.is_none() {
                *at = Some(Instant::now());
                crate::recognition_diagnostics::event(
                    "stop-requested",
                    "",
                    std::process::id(),
                    self.id,
                    self.started.elapsed().as_millis() as u64,
                    0,
                );
            }
        }
    }
}
struct Job {
    control: Control,
    options: StartOptions,
    capture: Capture,
    audio: Option<AudioBacklog>,
    /// Display-only helper; dropped with the job (see `live_preview`).
    preview: Option<crate::live_preview::Preview>,
    _punctuation_prewarm: Option<crate::punctuation::Prewarm>,
    // Drop last, after the capture and audio drain owners.
    _update_activity: resource_manager::UpdateActivityLease,
}
enum WorkerRequest {
    Job(Job),
    Prewarm { model: String },
    Shutdown,
}
struct Inner {
    models_root: PathBuf,
    reload_models: Arc<AtomicBool>,
    tx: Sender<WorkerRequest>,
    shutting_down: AtomicBool,
    completion: Arc<(Mutex<bool>, Condvar)>,
    active: Arc<Mutex<Option<Control>>>,
    next: AtomicU64,
    callback: Callback,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::Release);
        if let Ok(active) = self.active.lock() {
            if let Some(c) = active.as_ref() {
                c.cancel.store(true, Ordering::Release);
                c.request_capture_stop();
            }
        }
        let _ = self.tx.send(WorkerRequest::Shutdown);
    }
}
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}
impl Engine {
    pub fn new(models_root: PathBuf, callback: Callback) -> Self {
        let (tx, rx) = mpsc::channel::<WorkerRequest>();
        let completion = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_completion = completion.clone();
        let active = Arc::new(Mutex::new(None));
        let worker_active = active.clone();
        let worker_callback = callback.clone();
        let reload_models = Arc::new(AtomicBool::new(false));
        let worker_reload = reload_models.clone();
        let prewarm_root = models_root.clone();
        thread::Builder::new()
            .name("dictation-recognition".into())
            .spawn(move || {
                let mut recognizer: Option<asr::Recognizer> = None;
                loop {
                    let mut job = match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(WorkerRequest::Prewarm { model }) => {
                            if recognizer
                                .as_ref()
                                .is_none_or(|r| r.requested_model != model)
                            {
                                recognizer = None;
                                resource_manager::confirm_asr_released();
                                if resource_manager::acquire_asr_with_admission(
                                    &model,
                                    "startup prewarm",
                                )
                                .is_ok()
                                {
                                    match asr::Recognizer::load_interruptible(
                                        &models_root,
                                        &model,
                                        &AtomicBool::new(false),
                                    ) {
                                        Ok(loaded) => {
                                            recognizer = Some(loaded);
                                            resource_manager::note_asr_loaded(&model);
                                        }
                                        Err(_) => {
                                            resource_manager::note_asr_load_failed();
                                            resource_manager::notify_asr_finished();
                                        }
                                    }
                                }
                            } else {
                                resource_manager::note_asr_loaded(&model);
                            }
                            resource_manager::notify_asr_finished();
                            continue;
                        }
                        Ok(WorkerRequest::Job(job)) => job,
                        Ok(WorkerRequest::Shutdown) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            // Idle confirmed-release: the coordinator requested
                            // eviction while no job holds the worker. Drop the
                            // native recognizer here (the only owner) and
                            // confirm so admission sees reclaimed memory.
                            // Pending is never counted as freed before this.
                            // A models-root refresh drops for the same reason
                            // and must confirm too: without it the coordinator
                            // keeps a stale resident flag (free reload
                            // admission, blocked removal, hung file-job
                            // waits). No lease is active while idle, so the
                            // confirm only clears.
                            if worker_reload.swap(false, Ordering::AcqRel) {
                                recognizer = None;
                                resource_manager::confirm_asr_released();
                            }
                            if resource_manager::asr_release_pending() {
                                recognizer = None;
                                resource_manager::confirm_asr_released();
                            }
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let id = job.control.id;
                    if job.control.cancel.load(Ordering::Acquire) {
                        continue;
                    }
                    // A models-root refresh between jobs drops the stale
                    // resident the same way the switch path below does: the
                    // confirm keeps residency truthful (no lease is active
                    // yet — run_job acquires below — so it only clears).
                    if worker_reload.swap(false, Ordering::AcqRel) {
                        recognizer = None;
                        resource_manager::confirm_asr_released();
                    }
                    // A pending idle request that arrived just before this job
                    // was already cancelled by acquire_asr's touch; but if the
                    // job reuses a different model, drop the stale resident
                    // first so the new load never runs under a stale worker.
                    if resource_manager::asr_release_pending()
                        && recognizer
                            .as_ref()
                            .is_some_and(|r| r.requested_model != job.options.asr_model)
                    {
                        recognizer = None;
                        resource_manager::confirm_asr_released();
                    }
                    let mut failure = FailureKind::Other;
                    let outcome = run_job(
                        &models_root,
                        &mut recognizer,
                        &mut job,
                        &worker_callback,
                        &mut failure,
                    );
                    // A cancelled/crashed child no longer holds model memory.
                    // Clear residency now; the next load needs fresh admission.
                    if recognizer.as_mut().is_some_and(|r| !r.is_alive()) {
                        recognizer = None;
                        resource_manager::confirm_asr_released();
                    }
                    job.capture.stop();
                    let cancelled = job.control.cancel.load(Ordering::Acquire);
                    if cancelled
                        || outcome.as_ref().map_or(true, |result| {
                            result.recognition_completeness == RecognitionCompleteness::Incomplete
                        })
                    {
                        // A cancelled/failed RemoteStream may have killed its
                        // process. Never retain a model wrapper as proof that
                        // native weights are still resident.
                        recognizer = None;
                        resource_manager::confirm_asr_released();
                    }
                    let mut current = false;
                    if let Ok(mut active) = worker_active.lock() {
                        if active.as_ref().is_some_and(|c: &Control| c.id == id) {
                            *active = None;
                            current = true;
                        }
                    }
                    if cancelled || !current {
                        continue;
                    }
                    match outcome {
                        Ok(result) => (worker_callback)(Event::Completed {
                            session_id: id,
                            result,
                        }),
                        Err(message) => {
                            crate::recognition_diagnostics::event(
                                "dictation-failure",
                                &job.options.asr_model,
                                std::process::id(),
                                id,
                                job.control.started.elapsed().as_millis() as u64,
                                failure as u64,
                            );
                            (worker_callback)(Event::Error {
                                session_id: id,
                                message,
                            })
                        }
                    }
                }
                // Acknowledge only after the recognition process and queued
                // capture/backlog owners have actually been released.
                drop(recognizer);
                drop(rx);
                resource_manager::confirm_asr_released();
                if let Ok(mut done) = worker_completion.0.lock() {
                    *done = true;
                    worker_completion.1.notify_all();
                }
            })
            .expect("Cannot start dictation worker thread");
        Self {
            inner: Arc::new(Inner {
                models_root: prewarm_root,
                reload_models,
                tx,
                shutting_down: AtomicBool::new(false),
                completion,
                active,
                next: AtomicU64::new(1),
                callback,
            }),
        }
    }
    pub fn refresh_models(&self) {
        self.inner.reload_models.store(true, Ordering::Release);
    }

    /// Load the selected speech model on the owner thread without opening the
    /// microphone. Used only for explicit Keep-ready retention.
    pub fn prewarm(&self, model: String) -> Result<(), String> {
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("Dictation is shutting down.".into());
        }
        self.inner
            .tx
            .send(WorkerRequest::Prewarm { model })
            .map_err(|_| "Dictation worker is unavailable.".into())
    }

    /// Cancel capture and wait at most `timeout` for the owner to release its
    /// child. Run off the UI thread; a timeout is not a release acknowledgment.
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        let started = Instant::now();
        self.inner.shutting_down.store(true, Ordering::Release);
        self.cancel();
        let _ = self.inner.tx.send(WorkerRequest::Shutdown);
        let (lock, changed) = &*self.inner.completion;
        let done = lock
            .lock()
            .map_err(|_| "Dictation shutdown state failed.")?;
        let (done, _) = changed
            .wait_timeout_while(done, timeout.saturating_sub(started.elapsed()), |done| {
                !*done
            })
            .map_err(|_| "Dictation shutdown state failed.")?;
        crate::recognition_diagnostics::event(
            if *done {
                "shutdown-complete"
            } else {
                "shutdown-timeout"
            },
            "",
            std::process::id(),
            0,
            started.elapsed().as_millis() as u64,
            0,
        );
        if *done {
            Ok(())
        } else {
            Err("Dictation shutdown acknowledgment timed out.".into())
        }
    }

    pub fn start(&self, options: StartOptions) -> Result<u64, String> {
        let update_activity = resource_manager::begin_update_activity(resource_manager::UpdateActivity::Dictation)?;
        let mut active = self
            .inner
            .active
            .lock()
            .map_err(|_| "Dictation state lock failed.")?;
        if self.inner.shutting_down.load(Ordering::Acquire) {
            return Err("Dictation engine is shutting down.".into());
        }
        if active.is_some() {
            return Err("Dictation is already active.".into());
        }
        // Start capture before the worker loads either model. CPAL buffers all
        // initial audio while native inference initializes. A disk spool drains
        // that bounded queue independently of recognition speed.
        let mut capture = Capture::start(options.microphone.clone())?;
        let audio = capture.take_receiver()?;
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let control = Control {
            id,
            started: Instant::now(),
            stop_requested: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            cancel: capture.cancel_handle(),
            stop_cleanup: Arc::new(AtomicBool::new(false)),
            capture_stop: capture.stop_handle(),
            start_cue: Some(capture.start_cue()),
        };
        // The live preview receives a copy of the same captured audio, before
        // the disk spool, so a slow or loading primary never delays it.
        let (preview, tap) = crate::live_preview::wanted(
            &self.inner.models_root,
            options.live_preview,
            &options.asr_model,
        )
        .then(|| {
            crate::live_preview::Preview::start(
                self.inner.models_root.clone(),
                id,
                self.inner.callback.clone(),
                capture.cue_mask(),
                control.cancel.clone(),
            )
        })
        .flatten()
        .unzip();
        let audio = AudioBacklog::start_with_tap(
            audio,
            control.capture_stop.clone(),
            control.cancel.clone(),
            tap,
        )?;
        let punctuation_prewarm = (options.cleanup_mode != cleanup::Mode::Original)
            .then(|| crate::punctuation::prewarm(&self.inner.models_root, control.cancel.clone()));
        *active = Some(control.clone());
        crate::recognition_diagnostics::event(
            "capture-start",
            &options.asr_model,
            std::process::id(),
            id,
            0,
            0,
        );
        (self.inner.callback)(Event::Started { session_id: id });
        let meter = capture.level_handle();
        let meter_stop = control.capture_stop.clone();
        let meter_callback = self.inner.callback.clone();
        thread::spawn(move || {
            while !meter_stop.load(Ordering::Acquire) {
                meter_callback(Event::Level {
                    session_id: id,
                    rms: f32::from_bits(meter.load(Ordering::Relaxed)),
                });
                thread::sleep(Duration::from_millis(100));
            }
            meter_callback(Event::Level {
                session_id: id,
                rms: 0.0,
            });
        });
        if self
            .inner
            .tx
            .send(WorkerRequest::Job(Job {
                _update_activity: update_activity,
                control,
                options,
                capture,
                audio: Some(audio),
                preview,
                _punctuation_prewarm: punctuation_prewarm,
            }))
            .is_err()
        {
            *active = None;
            return Err("Dictation worker is unavailable.".into());
        }
        Ok(id)
    }
    pub(crate) fn play_start_cue(&self, session: u64) {
        if let Ok(active) = self.inner.active.lock() {
            if let Some(control) = active.as_ref().filter(|control| control.id == session) {
                if let Some(cue) = &control.start_cue { cue.clone().play(); }
            }
        }
    }
    pub fn stop(&self) -> Result<(), String> {
        let active = self
            .inner
            .active
            .lock()
            .map_err(|_| "Dictation state lock failed.")?;
        let Some(c) = active.as_ref() else {
            return Err("No dictation is active.".into());
        };
        c.request_capture_stop();
        if !c.stop.swap(true, Ordering::AcqRel) {
            c.mark_stop();
            (self.inner.callback)(Event::Finishing { session_id: c.id });
        }
        Ok(())
    }
    pub fn cancel(&self) {
        if let Ok(mut active) = self.inner.active.lock() {
            if let Some(c) = active.take() {
                c.cancel.store(true, Ordering::Release);
                c.request_capture_stop();
                (self.inner.callback)(Event::Cancelled { session_id: c.id });
            }
        }
    }

    pub fn stop_cleanup(&self) -> Result<(), String> {
        let active = self
            .inner
            .active
            .lock()
            .map_err(|_| "Dictation state lock failed.")?;
        if let Some(control) = active.as_ref() {
            control.stop_cleanup.store(true, Ordering::Release);
        }
        Ok(())
    }
}

fn recognition_cleanup_route(
    options: &StartOptions,
    completeness: RecognitionCompleteness,
) -> (cleanup::Mode, &str) {
    if completeness == RecognitionCompleteness::Incomplete {
        (cleanup::Mode::Original, "local-rules")
    } else if options.cleanup_model == crate::optional_llm::SMART_CLEANUP_ID
        && options.cleanup_mode != cleanup::Mode::Clean
    {
        (options.cleanup_mode, "local-rules")
    } else {
        (options.cleanup_mode, &options.cleanup_model)
    }
}

/// ASR lease for one dictation job. Drop always clears the active flag so
/// cancellation and errors cannot strand the coordinator in a busy state.
/// Sequential release uses `finish_for_cleanup`, which also confirms the
/// native recognizer was actually dropped before cleanup loads.
struct AsrLease {
    finished: bool,
}

impl AsrLease {
    fn acquire(model_id: &str) -> Result<Self, String> {
        resource_manager::touch_activity(resource_manager::Stage::Asr);
        // Atomic reservation + fit tied to lease ownership (Bug 1). A second
        // contender observes this pending reservation and fails explicitly
        // instead of both passing a pre-acquire admission check.
        resource_manager::acquire_asr_with_admission(model_id, "dictation")?;
        Ok(Self { finished: false })
    }

    fn finish_for_cleanup(mut self, recognizer: &mut Option<asr::Recognizer>) {
        // Sequential mode: actual ASR release before cleanup load. Dropping
        // the recognizer frees native inference memory; the confirm tells
        // admission the reclaim happened. Overlap keeps it resident.
        if resource_manager::asr_release_due_for_cleanup() {
            *recognizer = None;
            resource_manager::notify_asr_finished();
            resource_manager::confirm_asr_released();
        } else {
            resource_manager::notify_asr_finished();
        }
        self.finished = true;
    }
}

impl Drop for AsrLease {
    fn drop(&mut self) {
        if !self.finished {
            resource_manager::notify_asr_finished();
        }
    }
}

struct TextLease {
    finished: bool,
}

impl TextLease {
    /// Blocking wait/queue at the dictation text boundary. Holds no capture;
    /// capture already joined before this call. Observes cancel/stop so
    /// cancellation and Stop keep their guards. Timeout is explicit, never a
    /// silent downgrade (see run_job busy handling). Atomic reservation + fit
    /// (fail-fast on proven OOM, retry only on busy).
    fn acquire_waiting(
        role: &str,
        model_id: &str,
        cancel: &AtomicBool,
        stop: &AtomicBool,
    ) -> Result<Self, String> {
        resource_manager::touch_activity(resource_manager::Stage::Cleanup);
        // Smart abstains immediately on contention; its optional cleanup must
        // not wait behind Notes. Other models retain the existing 30 s queue.
        // 30 s queue: long enough for a notes chunk to finish, short enough
        // to keep delivery visible. Notes jobs use fail-fast waiting errors
        // with preservation; dictation waits here instead of silently
        // downgrading, then falls back explicitly only on timeout/stop.
        resource_manager::acquire_text_waiting_with_admission(
            role,
            model_id,
            "dictation cleanup",
            if model_id == crate::optional_llm::SMART_CLEANUP_ID {
                Duration::ZERO
            } else {
                Duration::from_secs(30)
            },
            cancel,
            stop,
        )
        .map(|()| Self { finished: false })
    }

    /// Prewarm reservation using the SAME lease the later text phase reuses.
    /// Single non-blocking try (fail-open: `None` skips prewarm, the later
    /// text phase still enforces atomic admission explicitly). Sequential
    /// contention fails here (ASR holds exclusivity) so no unowned prewarm
    /// ever bypasses the gate. The returned lease must be held until the text
    /// phase reuses or drops it; dropping releases active + reservation.
    fn try_acquire_for_prewarm(role: &str, model_id: &str, what: &str) -> Option<Self> {
        resource_manager::touch_activity(resource_manager::Stage::Cleanup);
        resource_manager::acquire_text_with_admission(role, model_id, what)
            .ok()
            .map(|()| Self { finished: false })
    }

    fn finish(mut self) {
        resource_manager::notify_text_finished();
        self.finished = true;
    }
}

impl Drop for TextLease {
    fn drop(&mut self) {
        if !self.finished {
            resource_manager::notify_text_finished();
        }
        self.finished = true;
    }
}

/// Layout owns a fail-open, non-queued use of the existing text worker after
/// every cleanup stage has finished. Keep this call last when adding stages.
fn shape_dictation(
    root: &std::path::Path,
    options: &StartOptions,
    completeness: RecognitionCompleteness,
    cleaned: &mut cleanup::ResultText,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    on_cleaning: impl FnOnce(),
) {
    use cleanup::layout;
    if !options.smart_layout {
        return;
    }
    let skip_reason = if options.cleanup_mode == cleanup::Mode::Original {
        Some("as_heard")
    } else if completeness == RecognitionCompleteness::Incomplete {
        Some("incomplete_recognition")
    } else if cancel.load(Ordering::Acquire) {
        Some("cancelled")
    } else if stop.load(Ordering::Acquire) {
        Some("cleanup_stopped")
    } else {
        None
    };
    if let Some(reason) = skip_reason {
        let outcome = layout::skip(&cleaned.text, reason);
        layout::apply_to_result(cleaned, outcome);
        return;
    }
    if let Some(outcome) = layout::preflight_outcome(&cleaned.text, options.cleanup_mode, cancel, stop) {
        layout::apply_to_result(cleaned, outcome);
        return;
    }
    // Availability is a cached/background check, never a weight hash on the
    // publication path. Prefer the selected cleanup model, then Notes.
    let model = [&options.cleanup_model, &options.notes_model]
        .into_iter()
        .find(|id| {
            layout::select_model(id, "off").is_some()
                && cleanup::model_status(root, id, crate::model_verification::Check::Background)
                    .ready()
        });
    let Some(model) = model else {
        let outcome = layout::skip(&cleaned.text, "model_unavailable");
        layout::apply_to_result(cleaned, outcome);
        return;
    };
    on_cleaning();
    let Some(lease) = TextLease::try_acquire_for_prewarm("cleanup", model, "dictation layout") else {
        let outcome = layout::skip(&cleaned.text, "model_busy");
        layout::apply_to_result(cleaned, outcome);
        return;
    };
    let outcome = layout::run(root, &cleaned.text, options.cleanup_mode, model, cancel, stop);
    // Reflect actual residency; an expired request may have killed its worker
    // under the process guard while erasing KV.
    if let Some(resident) = cleanup::resident_model_id() {
        resource_manager::note_text_loaded(&resident);
    } else {
        resource_manager::note_text_load_failed();
    }
    layout::apply_to_result(cleaned, outcome);
    // The layout owner erases both prompts under its guard on every exit,
    // including no-history, Stop and deadline fallbacks.
    lease.finish();
}

/// Stable, content-free categories for the `dictation-failure` diagnostic
/// (`value`). Codes are compared across field reports: never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// The resident speech worker had exited and its replacement failed.
    WorkerExited = 1,
    /// Another job held the speech stage, or admission stayed contended.
    AdmissionBusy = 2,
    /// Admission refused the load for lack of memory.
    MemoryDenied = 3,
    LoadFailed = 4,
    StreamFailed = 5,
    Microphone = 6,
    Other = 7,
}

/// Drop a cached recognizer that cannot serve `model`: another model, or a
/// worker that exited while idle (macOS kills idle helpers under memory
/// pressure). Logs the exit value content-free and confirms the release so
/// admission sees the truth. Returns true when the worker was dead.
pub(crate) fn drop_unusable_recognizer(
    recognizer: &mut Option<asr::Recognizer>,
    model: &str,
    session: u64,
) -> bool {
    let Some(current) = recognizer.as_mut() else {
        return false;
    };
    let alive = current.is_alive();
    if alive && current.requested_model == model {
        return false;
    }
    if let Some((pid, value)) = current.worker_exit() {
        crate::recognition_diagnostics::event(
            "asr-worker-exited",
            &current.model_id,
            pid,
            session,
            0,
            value,
        );
    }
    *recognizer = None;
    resource_manager::confirm_asr_released();
    !alive
}

/// Under the held ASR lease, leave `recognizer` holding a live worker for
/// `model`: reuse it, or load it once. A worker found dead here (it exited
/// after the pre-admission check) is dropped and confirmed first, which arms
/// the lease's reservation for the replacement. Audio keeps buffering in the
/// backlog meanwhile, so a reload only delays the first words.
pub(crate) fn ensure_recognizer(
    root: &std::path::Path,
    recognizer: &mut Option<asr::Recognizer>,
    model: &str,
    cancel: &AtomicBool,
    session: u64,
    worker_exited: bool,
    failure: &mut FailureKind,
) -> Result<(), String> {
    let exited = drop_unusable_recognizer(recognizer, model, session) || worker_exited;
    if recognizer.is_some() {
        // Reusing the resident worker: confirm readiness under the new lease.
        resource_manager::note_asr_loaded(model);
        return Ok(());
    }
    // Clear any stale resident flag so a failed load cannot strand it.
    resource_manager::confirm_asr_released();
    *failure = if exited {
        FailureKind::WorkerExited
    } else {
        FailureKind::LoadFailed
    };
    match asr::Recognizer::load_interruptible(root, model, cancel) {
        Ok(loaded) => {
            *recognizer = Some(loaded);
            // Readiness is marked on actual successful load, not on lease.
            resource_manager::note_asr_loaded(model);
            Ok(())
        }
        Err(error) => {
            // Nothing resident; the lease's Drop clears active.
            resource_manager::note_asr_load_failed();
            Err(error)
        }
    }
}

fn run_job(
    root: &std::path::Path,
    recognizer: &mut Option<asr::Recognizer>,
    job: &mut Job,
    callback: &Callback,
    failure: &mut FailureKind,
) -> Result<DictationResult, String> {
    let control = &job.control;
    let id = control.id;
    let progress_callback = callback.clone();
    let _progress = crate::recognition::observe_progress(Arc::new(move |progress| {
        progress_callback(Event::Progress {
            session_id: id,
            progress,
        });
    }));
    // Release a different or dead cached model before sampling free memory.
    // This owner cannot service its own idle-release request while blocked in
    // admission. Waiting there would wrongly reject an otherwise fitting switch.
    let worker_exited = drop_unusable_recognizer(recognizer, &job.options.asr_model, id);
    // Atomic acquire: fit + active + pending reservation in one critical
    // section (see resource_manager). No separate pre-acquire admission.
    *failure = FailureKind::AdmissionBusy;
    let asr_lease = AsrLease::acquire(&job.options.asr_model).inspect_err(|error| {
        if error.starts_with(resource_manager::MEMORY_UNAVAILABLE) {
            *failure = FailureKind::MemoryDenied;
        }
    })?;
    // Prewarm uses the SAME text lease/reservation the later cleanup phase
    // reuses — never a separate should_prewarm check + unowned
    // cleanup::prewarm. Single try, fail-open (None skips prewarm); the text
    // phase still enforces atomic admission. Sequential contention fails the
    // try (ASR exclusivity) so no bypass. The try-lock polling hint never
    // drives prewarm/destruction; only this owned lease does.
    let mut prewarm_lease: Option<TextLease> = if job.options.cleanup_mode
        != cleanup::Mode::Original
        && !matches!(job.options.cleanup_model.as_str(), "off" | "local-rules")
        && (job.options.cleanup_model != crate::optional_llm::SMART_CLEANUP_ID
            || job.options.cleanup_mode == cleanup::Mode::Clean)
    {
        let lease = TextLease::try_acquire_for_prewarm(
            "cleanup",
            &job.options.cleanup_model,
            "cleanup prewarm",
        );
        if lease.is_some() {
            cleanup::prewarm(
                root.to_owned(),
                &job.options.cleanup_model,
                job.options.cleanup_mode,
            );
        }
        lease
    } else {
        None
    };
    // A pending idle eviction is cancelled by this new activity. The acquire
    // above already cleared it via touch_activity. Admission can take a
    // while, so liveness is checked again here (defence in depth): a worker
    // that died meanwhile is reloaded once instead of failing the job. If it
    // dies after this check, the remote stream's single Begin reconnect
    // covers it; a second failure surfaces as before.
    ensure_recognizer(
        root,
        recognizer,
        &job.options.asr_model,
        &control.cancel,
        id,
        worker_exited,
        failure,
    )?;
    if control.cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled.".into());
    }
    *failure = if worker_exited {
        FailureKind::WorkerExited
    } else {
        FailureKind::StreamFailed
    };
    let active = recognizer.as_mut().unwrap();
    let model_id = active.model_id.clone();
    let mut stream = active.stream_interruptible(&control.cancel)?;
    *failure = FailureKind::StreamFailed;
    let mut last_update = Instant::now();
    let mut silence_start = Duration::ZERO;
    let mut last_interim = String::new();
    let mut first_result = false;
    let mut audio_ms = 0.0f64;
    let mut audible_ms = 0.0f64;
    let mut peak_rms = 0.0f32;
    let mut audio = job.audio.take().ok_or("Audio backlog is unavailable.")?;
    loop {
        if control.cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled.".into());
        }
        match audio.recv_timeout(Duration::from_millis(25)) {
            Ok(AudioMessage::Chunk {
                mut samples,
                sample_rate,
                rms,
                captured_at,
                ..
            }) => {
                let duration = samples.len() as f64 * 1000.0 / f64::from(sample_rate.max(1));
                audio_ms += duration;
                if rms >= 0.008 {
                    audible_ms += duration;
                }
                peak_rms = peak_rms.max(rms);
                // The backlog and any saved source retain captured audio.
                // Only the owned ASR input is changed, on the capture timeline.
                job.capture.filter_asr(&mut samples);
                stream.add(&samples, sample_rate)?;
                if stream.failed() {
                    // The retry budget is exhausted. Stop capture and publish
                    // only the explicitly incomplete, acknowledged raw text.
                    job.capture.stop();
                    control.mark_stop();
                    callback(Event::Finishing { session_id: id });
                    break;
                }
                if rms >= 0.008 {
                    silence_start = captured_at;
                }
                if last_update.elapsed() >= Duration::from_millis(650)
                    && !control.stop.load(Ordering::Acquire)
                {
                    let text = stream.update()?;
                    if control.cancel.load(Ordering::Acquire) {
                        return Err("Dictation cancelled.".into());
                    }
                    if text != last_interim {
                        if !first_result && !text.is_empty() {
                            first_result = true;
                            crate::recognition_diagnostics::event(
                                "capture-first-result",
                                &model_id,
                                std::process::id(),
                                id,
                                control.started.elapsed().as_millis() as u64,
                                0,
                            );
                        }
                        let publish = || {
                            callback(Event::Interim {
                                session_id: id,
                                text: text.clone(),
                            })
                        };
                        // The preview restarts after the newly committed audio.
                        match &job.preview {
                            Some(preview) => preview.commit(stream.committed_seconds(), publish),
                            None => publish(),
                        }
                        last_interim = text;
                    }
                    last_update = Instant::now();
                }
                // Only the user's silence setting can finish capture automatically.
                // Recognition drains continuously; session length is not a stop signal.
                if silence_auto_stop(
                    job.options.auto_stop_seconds,
                    captured_at.saturating_sub(silence_start),
                ) && !control.stop.swap(true, Ordering::AcqRel)
                {
                    control.mark_stop();
                    job.capture.stop();
                    callback(Event::Finishing { session_id: id });
                }
            }
            Ok(AudioMessage::End) => break,
            Ok(AudioMessage::Error(error)) => {
                *failure = FailureKind::Microphone;
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                *failure = FailureKind::Microphone;
                return Err("Audio ended without capture finalization.".into());
            }
        }
    }
    // Final recognition replaces the draft; stop decoding it now.
    if let Some(preview) = &job.preview {
        preview.stop();
    }
    for (event, value) in [
        ("audio-consumed-ms", audio_ms as u64),
        ("audio-above-meter-threshold-ms", audible_ms as u64),
        ("audio-peak-rms-ppm", (peak_rms * 1_000_000.0) as u64),
    ] {
        crate::recognition_diagnostics::event(event, &model_id, std::process::id(), id, 0, value);
    }
    crate::recognition_diagnostics::event(
        "backlog-released",
        &model_id,
        std::process::id(),
        id,
        control.started.elapsed().as_millis() as u64,
        audio.peak_bytes(),
    );
    drop(audio);
    job.capture.join();
    crate::recognition_diagnostics::capture_completion(&model_id, id, &job.capture.diagnostics());
    callback(Event::Level {
        session_id: id,
        rms: 0.0,
    });
    let finalized = Instant::now();
    let recognition = stream.finish_recognition()?;
    *failure = FailureKind::Other;
    let recognition_completeness = recognition.completeness;
    let guarded_text = &recognition.text;
    let asr_ms = finalized.elapsed().as_millis() as u64;
    crate::recognition_diagnostics::event(
        "finish-complete",
        &model_id,
        std::process::id(),
        id,
        asr_ms,
        0,
    );
    if !first_result && !guarded_text.is_empty() {
        crate::recognition_diagnostics::event(
            "capture-first-result",
            &model_id,
            std::process::id(),
            id,
            control.started.elapsed().as_millis() as u64,
            0,
        );
    }
    if let Some(stopped) = control.stop_requested.lock().ok().and_then(|at| *at) {
        crate::recognition_diagnostics::event(
            "stop-to-result",
            &model_id,
            std::process::id(),
            id,
            stopped.elapsed().as_millis() as u64,
            0,
        );
    }
    if control.cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled.".into());
    }
    let cleanup_started = Instant::now();
    let (cleanup_mode, cleanup_model) =
        recognition_cleanup_route(&job.options, recognition_completeness);
    let needs_text_model =
        cleanup_mode != cleanup::Mode::Original && !matches!(cleanup_model, "off" | "local-rules");
    // Sequential handoff: the actual ASR recognizer is dropped before cleanup
    // loads or prewarms. Overlap only clears the active flag and keeps the
    // resident recognizer for the next job.
    asr_lease.finish_for_cleanup(recognizer);
    // Queue for the shared text worker instead of silently downgrading.
    // Cancellation keeps its guard; Stop requests a fast safe path; a busy
    // timeout falls back to local rules with an explicit busy status (never
    // silent). The complete tail is preserved by the safe-formatting fallback
    // inside run_interruptible_with_policy; single capture ownership and
    // notes processing are untouched (notes hold their own whole-job lease).
    // Atomic waiter ties reservation + fit to the lease: proven OOM fails
    // fast with an explicit note (never retried as busy), busy waits 30 s.
    // The prewarm lease above (same reservation) is reused when it matches;
    // no second acquire double-counts the bytes.
    let (text_lease, busy_fallback, admission_note) = if needs_text_model {
        // Reuse the prewarm lease when it already holds this exact model.
        // `prewarm_lease` is Some only when the try succeeded for the same
        // cleanup_model; route changes (Incomplete → local-rules) drop it.
        let reused = match prewarm_lease.take() {
            Some(lease) if cleanup_model == job.options.cleanup_model => Some(lease),
            other => {
                // Model mismatch or no prewarm: release (Drop) before waiting.
                drop(other);
                None
            }
        };
        if let Some(lease) = reused {
            (Some(lease), false, None)
        } else {
            match TextLease::acquire_waiting(
                "cleanup",
                cleanup_model,
                &control.cancel,
                &control.stop_cleanup,
            ) {
                Ok(lease) => (Some(lease), false, None),
                Err(e) if e.contains("cancelled") || e.contains("Cancelled") => {
                    return Err("Dictation cancelled.".into())
                }
                Err(e) if e.contains("stopped") || e.contains("Stopped") => {
                    // User pressed Stop during the wait: fast safe path, complete
                    // tail preserved, no second worker started.
                    (None, false, None)
                }
                Err(e) if e.contains("Not enough") => {
                    // Proven-insufficient RAM/VRAM: explicit fallback, tail preserved.
                    (None, true, Some(e))
                }
                Err(_) => (None, true, None),
            }
        }
    } else {
        // No text model needed: release any unused prewarm reservation.
        drop(prewarm_lease);
        (None, false, None)
    };
    let smart_unavailable = cleanup_model == crate::optional_llm::SMART_CLEANUP_ID
        && cleanup_mode == cleanup::Mode::Clean && text_lease.is_none();
    let (effective_mode, effective_model) = if smart_unavailable {
        (cleanup::Mode::Clean, "local-rules")
    } else if needs_text_model && text_lease.is_none() {
        // Busy or unadmitted optional model: keep the default cleanup (rules,
        // vocabulary and punctuation) in the chosen mode instead of raw text.
        (cleanup_mode, "local-rules")
    } else {
        (cleanup_mode, cleanup_model)
    };
    if effective_mode != cleanup::Mode::Original
        && !matches!(effective_model, "off" | "local-rules")
    {
        callback(Event::Cleaning {
            session_id: id,
            original_text: guarded_text.clone(),
        });
    }
    // Learned cleanup vocabulary: full raw utterance + mapped mode, ≤1536
    // budget, explicit + confirmed-scoped only. Never at capture start, never
    // per-chunk, never to cloud. Merged with explicit vocabulary for this
    // cleanup request only.
    let learned = learned_cleanup_for_job(&job.options.store, guarded_text, effective_mode);
    let mut request_vocabulary = job.options.vocabulary.clone();
    request_vocabulary.extend(learned);
    // Stopping the optional model still keeps the deterministic first stage.
    // Cancellation remains live and wins before any publication.
    let fallback_stop = AtomicBool::new(false);
    let (original_text, mut cleaned) = clean_dictation_recognition(&recognition, |text| cleanup::run_dictation_interruptible_with_options(
        root,
        text,
        effective_mode,
        &request_vocabulary,
        &job.options.confirmed_vocabulary,
        effective_model,
        job.options.policy,
        &control.cancel,
        if smart_unavailable { &fallback_stop } else { &control.stop_cleanup },
        job.options.written_form,
    ));
    if smart_unavailable {
        cleaned.pre_model_text = Some(cleaned.text.clone());
        if cleaned.publication_stage.is_none() {
            cleaned.publication_stage = Some("rules".into());
        }
        cleaned.guard_reason = Some(if control.stop_cleanup.load(Ordering::Acquire) {
            "cleanup_stopped"
        } else { "model_busy" }.into());
    }
    // Mark text readiness on actual successful inference under the held lease.
    // Shared weights are not duplicated: one artifact keeps one resident flag.
    if text_lease.is_some() && !matches!(effective_model, "off" | "local-rules") {
        resource_manager::note_text_loaded(effective_model);
    }
    // Per-job KV hygiene belongs UNDER the still-owned lease/process guard,
    // never as a shutdown after release (that could kill a newly acquired
    // notes job). The cleanup owner erases slot KV per request under-guard;
    // this final report confirms no prompt KV crosses into the next job.
    // No-history jobs gate privacy on it; history jobs already erased per
    // request and need no extra shutdown.
    if let Some(lease) = text_lease {
        if needs_text_model && job.options.no_history {
            let report = crate::cleanup_resource::end_job_report(true);
            // `private()` means erased / already-clear / shutdown-fallback.
            // `busy_deferred` cannot happen here: this job still holds the
            // resource lease, so no other job owns the erase. If the erase
            // failed, the report already forced a shutdown before reuse.
            let _ = report.private();
        }
        lease.finish();
    }
    shape_dictation(root, &job.options, recognition_completeness, &mut cleaned,
        &control.cancel, &control.stop_cleanup, || callback(Event::Cleaning {
            session_id: id,
            original_text: guarded_text.clone(),
        }));
    let cleanup_ms = cleanup_started.elapsed().as_millis() as u64;
    if control.cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled.".into());
    }
    // Only this result reaches the publication boundary. There is no delayed
    // cleanup callback, so stopping cleanup cannot later mutate a clipboard.
    let cleanup_status = if busy_fallback {
        if let Some(note) = admission_note {
            format!(
                "model_busy_fallback; local rules used ({note}). {}",
                cleaned.status
            )
        } else {
            format!(
                "model_busy_fallback; local rules used while text inference was busy. {}",
                cleaned.status
            )
        }
    } else {
        cleaned.status
    };
    Ok(DictationResult {
        original_text,
        output_text: cleaned.text,
        pre_model_text: cleaned.pre_model_text,
        publication_stage: cleaned.publication_stage,
        guard_reason: cleaned.guard_reason,
        recognition_completeness,
        recognition_warnings: recognition.warnings,
        asr_model: model_id,
        cleanup_status,
        policy_version: cleaned.policy_version,
        policy_hash: if effective_model == crate::optional_llm::SMART_CLEANUP_ID {
            cleanup::smart::policy_hash()
        } else { job.options.policy.hash() },
        asr_ms,
        cleanup_ms,
    })
}

/// Cleanup consumes only guarded speech; the untouched recognition is returned
/// separately for DictationResult/history and never used as a cleanup fallback.
fn clean_dictation_recognition(
    recognition: &asr::RecognitionResult,
    clean: impl FnOnce(&str) -> cleanup::ResultText,
) -> (String, cleanup::ResultText) {
    let cleaned = clean(&recognition.text);
    (recognition.original_text().to_owned(), cleaned)
}

fn silence_auto_stop(seconds: u64, silence: Duration) -> bool {
    seconds > 0 && silence >= Duration::from_secs(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Engine>();
    }
    #[test]
    fn smart_model_is_bypassed_outside_complete_clean_dictation() {
        for mode in [cleanup::Mode::Original, cleanup::Mode::Format, cleanup::Mode::Clean] {
            let options = StartOptions { cleanup_model: crate::optional_llm::SMART_CLEANUP_ID.into(), cleanup_mode: mode, ..StartOptions::default() };
            assert_eq!(recognition_cleanup_route(&options, RecognitionCompleteness::Incomplete), (cleanup::Mode::Original, "local-rules"));
            let (actual_mode, model) = recognition_cleanup_route(&options, RecognitionCompleteness::Complete);
            assert_eq!(actual_mode, mode);
            assert_eq!(model, if mode == cleanup::Mode::Clean { crate::optional_llm::SMART_CLEANUP_ID } else { "local-rules" });
        }
    }

    #[test]
    fn dictation_cleanup_consumes_guarded_text_and_retains_original_separately() {
        let directory = std::env::temp_dir().join(format!("sonojot-guarded-dictation-{}", uuid::Uuid::new_v4()));
        let store = crate::store::Store::open(Some(directory.clone())).unwrap();
        for source in [
            "go go go go",
            "Keep these four words. go go go go go go go go",
            "Keep these four words.",
        ] {
            let recognition = crate::sherpa::llama_test_recognition((
                format!("language English<asr_text>{source}"), Some(12), "stop".into(),
            ));
            for mode in [cleanup::Mode::Original, cleanup::Mode::Format, cleanup::Mode::Clean] {
                let options = StartOptions { cleanup_mode: mode, cleanup_model: "local-rules".into(), ..StartOptions::default() };
                let (mode, model) = recognition_cleanup_route(&options, recognition.completeness);
                let (original, cleaned) = clean_dictation_recognition(&recognition, |input| {
                    assert_eq!(input, recognition.text);
                    assert!(!input.contains("go go"));
                    cleanup::run_dictation_interruptible_with_policy(
                        std::path::Path::new("unused"), input, mode, &[], &[], model,
                        options.policy, &AtomicBool::new(false), &AtomicBool::new(false),
                    )
                });
                assert_eq!(original, source);
                let saved = store.commit_dictation_with_metadata(
                    store.privacy_generation(), &original, &cleaned.text,
                    recognition.completeness, &recognition.warnings,
                    cleaned.pre_model_text.as_deref(), cleaned.publication_stage.as_deref(), cleaned.guard_reason.as_deref(),
                ).unwrap().unwrap();
                let saved = store.get_dictation(&saved.id).unwrap();
                assert_eq!(saved.raw, source);
                assert_eq!(saved.published, cleaned.text);
                assert_eq!(saved.recognition_warnings, recognition.warnings);
                assert!(!cleaned.text.contains("go go"));
                if recognition.completeness == RecognitionCompleteness::Incomplete {
                    assert_eq!(cleaned.text, recognition.text);
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn default_cleanup_is_clean_and_conservative() {
        assert_eq!(StartOptions::default().cleanup_mode, cleanup::Mode::Clean);
        assert!(StartOptions::default().policy.is_conservative());
        assert!(!StartOptions::default().no_history);
        assert!(!StartOptions::default().smart_layout);
    }

    #[test]
    fn layout_bypasses_disabled_raw_incomplete_and_interrupted_dictation() {
        let text = "um keep my words exactly. including this punctuation";
        for (enabled, mode, completeness, cancel, stop) in [
            (false, cleanup::Mode::Clean, RecognitionCompleteness::Complete, false, false),
            (true, cleanup::Mode::Original, RecognitionCompleteness::Complete, false, false),
            (true, cleanup::Mode::Clean, RecognitionCompleteness::Incomplete, false, false),
            (true, cleanup::Mode::Format, RecognitionCompleteness::Complete, true, false),
            (true, cleanup::Mode::Clean, RecognitionCompleteness::Complete, false, true),
            (true, cleanup::Mode::Format, RecognitionCompleteness::Complete, false, false),
        ] {
            let options = StartOptions { smart_layout: enabled, cleanup_mode: mode, ..Default::default() };
            let mut result = cleanup::run_interruptible_with_policy(
                std::path::Path::new(""), text, cleanup::Mode::Original, &[], &[], "local-rules",
                cleanup::CleanupPolicy::conservative(), &AtomicBool::new(false), &AtomicBool::new(false),
            );
            let before = result.text.clone();
            let prior_status = result.status.clone();
            shape_dictation(std::path::Path::new(""), &options, completeness, &mut result,
                &AtomicBool::new(cancel), &AtomicBool::new(stop), || panic!("Layout must not start"));
            assert_eq!(result.text, before);
            assert!(result.pre_model_text.is_none());
            if !enabled {
                assert_eq!(result.status, prior_status);
                assert!(result.publication_stage.is_none());
            }
        }
    }
    #[test]
    fn from_settings_selects_conservative_and_snapshots_no_history() {
        let mut settings = crate::domain::Settings::default();
        settings.cleanup_trial = true;
        settings.do_not_save_dictations = true;
        settings.smart_layout = true;
        settings.written_form = false;
        settings.notes_model = "qwen3-4b-instruct-2507".into();
        let options = StartOptions::from_settings(settings, Vec::new(), Vec::new());
        // Saved trial bool is preserved in Settings for migration/evidence;
        // normal runtime uses the tested conservative host policy.
        assert!(options.policy.is_conservative());
        assert!(options.no_history);
        assert!(options.smart_layout);
        assert!(!options.written_form);
        assert_eq!(options.notes_model, "qwen3-4b-instruct-2507");
        let mut settings = crate::domain::Settings::default();
        settings.cleanup_trial = false;
        settings.do_not_save_dictations = false;
        let options = StartOptions::from_settings(settings, Vec::new(), Vec::new());
        assert!(options.policy.is_conservative());
        assert!(!options.no_history);
        assert!(!options.smart_layout);
        assert!(options.written_form);
    }
    #[test]
    fn incomplete_recognition_bypasses_cleanup_in_every_mode_and_trial_state() {
        for cleanup_mode in [
            cleanup::Mode::Original,
            cleanup::Mode::Format,
            cleanup::Mode::Clean,
        ] {
            for policy in [
                cleanup::CleanupPolicy::legacy(),
                cleanup::CleanupPolicy::trial(),
                cleanup::CleanupPolicy::daily(),
                cleanup::CleanupPolicy::targeted(),
                cleanup::CleanupPolicy::conservative(),
            ] {
                let options = StartOptions {
                    cleanup_mode,
                    cleanup_model: "qwen3-4b-instruct-2507".into(),
                    policy,
                    ..Default::default()
                };
                assert_eq!(
                    recognition_cleanup_route(&options, RecognitionCompleteness::Incomplete),
                    (cleanup::Mode::Original, "local-rules")
                );
                assert_eq!(
                    recognition_cleanup_route(&options, RecognitionCompleteness::Complete),
                    (cleanup_mode, "qwen3-4b-instruct-2507")
                );
            }
        }
    }

    #[test]
    fn dictation_has_no_elapsed_time_stop() {
        let options = StartOptions::default();
        assert_eq!(options.auto_stop_seconds, 0);
        for seconds in [599, 600, 601, 3_600, 86_400, u32::MAX as u64] {
            assert!(!silence_auto_stop(
                options.auto_stop_seconds,
                Duration::from_secs(seconds)
            ));
        }
        assert!(!silence_auto_stop(5, Duration::from_millis(4_999)));
        assert!(silence_auto_stop(5, Duration::from_secs(5)));
        assert!(silence_auto_stop(5, Duration::from_secs(601)));
    }
    #[test]
    fn explicit_stop_and_cancel_keep_their_capture_and_publication_guards() {
        // Exercise real Engine controls without opening a microphone or model.
        let control = Control {
            id: 42,
            started: Instant::now(),
            stop_requested: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            stop_cleanup: Arc::new(AtomicBool::new(false)),
            capture_stop: Arc::new(AtomicBool::new(false)),
            start_cue: None,
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        let (tx, _rx) = mpsc::channel();
        let engine = Engine {
            inner: Arc::new(Inner {
                reload_models: Arc::new(AtomicBool::new(false)),
                models_root: PathBuf::new(),
                tx,
                shutting_down: AtomicBool::new(false),
                completion: Arc::new((Mutex::new(false), Condvar::new())),
                active: Arc::new(Mutex::new(Some(control.clone()))),
                next: AtomicU64::new(43),
                callback: Arc::new(move |event| observed.lock().unwrap().push(event)),
            }),
        };
        engine.stop().unwrap();
        engine.stop().unwrap();
        assert!(control.capture_stop.load(Ordering::Acquire));
        assert!(control.stop.load(Ordering::Acquire));
        assert!(!control.cancel.load(Ordering::Acquire));
        engine.stop_cleanup().unwrap();
        assert!(control.stop_cleanup.load(Ordering::Acquire));
        assert!(!control.cancel.load(Ordering::Acquire));
        assert!(engine.inner.active.lock().unwrap().is_some());
        engine.cancel();
        assert!(control.cancel.load(Ordering::Acquire));
        assert!(engine.inner.active.lock().unwrap().is_none());
        assert!(engine.stop().is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Finishing { session_id: 42 }));
        assert!(matches!(events[1], Event::Cancelled { session_id: 42 }));
    }

    #[test]
    fn shutdown_acknowledges_owner_exit_and_rejects_new_capture() {
        let engine = Engine::new(PathBuf::from("unused"), Arc::new(|_| {}));
        engine.shutdown(Duration::from_secs(2)).unwrap();
        engine.shutdown(Duration::ZERO).unwrap();
        assert!(engine
            .start(StartOptions::default())
            .unwrap_err()
            .contains("shutting down"));
    }

    #[test]
    fn shutdown_timeout_never_claims_owner_release() {
        let (tx, rx) = mpsc::channel();
        let engine = Engine {
            inner: Arc::new(Inner {
                models_root: PathBuf::new(),
                reload_models: Arc::new(AtomicBool::new(false)),
                tx,
                shutting_down: AtomicBool::new(false),
                completion: Arc::new((Mutex::new(false), Condvar::new())),
                active: Arc::new(Mutex::new(None)),
                next: AtomicU64::new(1),
                callback: Arc::new(|_| {}),
            }),
        };
        assert!(engine
            .shutdown(Duration::from_millis(10))
            .unwrap_err()
            .contains("timed out"));
        assert!(matches!(rx.try_recv(), Ok(WorkerRequest::Shutdown)));
        assert!(!*engine.inner.completion.0.lock().unwrap());
    }
}
