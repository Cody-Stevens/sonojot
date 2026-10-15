use crate::{
    audio, cleanup, domain::*, meetings::Meetings, platform, recording::MeetingRecorder, speech,
    store::Store, windows,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, MutexGuard,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const RECOGNITION_WARNING_NOTICE: &str =
    "Recognition incomplete. Review the text before copying it.";

pub struct Runtime {
    pub updater: crate::updater::UpdateService,
    pub store: Store,
    pub meetings: Meetings,
    pub models_root: PathBuf,
    pub snapshot: Mutex<Snapshot>,
    pub control: Mutex<Control>,
    pub engine: speech::Engine,
    pub recorder: Mutex<Option<(String, MeetingRecorder)>>,
    pub meeting_update_activity: Mutex<Option<crate::resource_manager::UpdateActivityLease>>,
    pub settings_gate: tokio::sync::Mutex<()>,
    pub last_delivery: Mutex<Option<String>>,
    pub last_published: Mutex<String>,
    /// Association for the current review/Copy-last text; private results have none.
    pub current_record_id: Mutex<Option<String>>,
    pub tab_target: Mutex<Option<platform::PasteTarget>>,
    pub command_generation: AtomicU64,
    pub tab_gate: Mutex<()>,
}

pub struct Control {
    pub(crate) microphone_preview: Option<crate::onboarding::MicrophonePreview>,
    update_activity: Option<crate::resource_manager::UpdateActivityLease>,
    session: Option<u64>,
    generation: u64,
    publishing: bool,
    target: Option<platform::PasteTarget>,
    last_meter: Instant,
    privacy_generation: u64,
}

/// A manual-copy publication claim owns the same start/cancel boundary as an
/// automatic delivery. It holds no mutex while the OS clipboard call awaits.
pub(crate) struct ManualCopyClaim<'a> {
    _update_activity: crate::resource_manager::UpdateActivityLease,
    control: &'a Mutex<Control>,
}

impl Drop for ManualCopyClaim<'_> {
    fn drop(&mut self) {
        if let Ok(mut control) = self.control.lock() {
            control.publishing = false;
        }
    }
}

enum SpeechUpdate {
    Ignored,
    Updated,
    CloseCursor,
    FinishCursor,
    Publish(Publication),
}

struct Publication {
    session_id: u64,
    original_text: String,
    text: String,
    pre_model_text: Option<String>,
    publication_stage: Option<String>,
    guard_reason: Option<String>,
    target: Option<platform::PasteTarget>,
    auto_paste: bool,
    /// Cleanup policy that produced `text`; recorded in the delivery log so
    /// the structured cleanup trial can verify it used the evaluated path.
    policy_version: String,
    policy_hash: String,
    recognition_completeness: RecognitionCompleteness,
    recognition_warnings: Vec<String>,
}

impl Publication {
    // Production injects platform::deliver_text; tests inject a local fake sink.
    // Keep the complete output, including whitespace, when delivery is needed.
    async fn deliver_with<'a, F, Fut>(
        &'a mut self,
        sink: F,
    ) -> Option<Result<platform::DeliveryOutcome, String>>
    where
        F: FnOnce(&'a str, Option<platform::PasteTarget>, bool) -> Fut,
        Fut: std::future::Future<Output = Result<platform::DeliveryOutcome, String>>,
    {
        if self.recognition_completeness == RecognitionCompleteness::Incomplete
            || self.text.trim().is_empty()
        {
            return None;
        }
        Some(sink(&self.text, self.target.take(), self.auto_paste).await)
    }
}

impl Control {
    pub(crate) fn is_publishing(&self) -> bool {
        self.publishing
    }

    fn claim_manual_copy(&mut self, state: &Snapshot, result: Option<u64>) -> Result<(), String> {
        if self.session.is_some()
            || self.publishing
            || !matches!(state.state, AppState::Idle | AppState::Error)
            || result.is_none()
            || state.result_session_id != result
        {
            return Err(
                "This dictation is no longer available to copy. Review it in Recent dictations."
                    .into(),
            );
        }
        self.publishing = true;
        Ok(())
    }

    fn can_start(&self, generation: u64, invalidation: &AtomicU64) -> Result<bool, String> {
        if invalidation.load(Ordering::Acquire) != generation {
            return Ok(false);
        }
        if self.session.is_some() || self.publishing || self.microphone_preview.is_some() {
            return Err("Dictation is already running.".into());
        }
        Ok(true)
    }

    fn can_stop(&self, ticket: (Option<u64>, u64)) -> bool {
        !self.publishing && stop_matches(ticket, self.session, self.generation)
    }

    fn can_stop_capture(&self, ticket: (Option<u64>, u64), state: AppState) -> bool {
        self.can_stop(ticket) && state != AppState::Cleaning
    }

    fn can_stop_cleanup(&self, ticket: (Option<u64>, u64), state: AppState) -> bool {
        self.can_stop(ticket) && state == AppState::Cleaning
    }

    fn cancel(&mut self, ticket: (Option<u64>, u64), state: &mut Snapshot) -> Result<bool, String> {
        if self.session.is_some() && !stop_matches(ticket, self.session, self.generation) {
            return Ok(false);
        }
        // Claiming publication is the cancellation boundary, before OS delivery.
        // Once claimed, the delivery task owns finalization; cancellation cannot undo it.
        if self.publishing {
            return Err("The final text is already being copied.".into());
        }
        if self.session.is_none() {
            return Ok(false);
        }
        self.session = None;
        self.target = None;
        state.state = AppState::Idle;
        state.interim_text.clear();
        state.original_text.clear();
        state.output_text.clear();
        state.pre_model_text = None;
        state.publication_stage = None;
        state.guard_reason = None;
        state.result_session_id = None;
        state.recognition_completeness = None;
        state.recognition_warnings.clear();
        state.can_copy_last = false;
        state.delivery_notice = None;
        state.preview_text.clear();
        state.audio_level = 0.0;
        state.session_id = None;
        state.capture_generation = None;
        self.update_activity = None;
        Ok(true)
    }

    // Called with Runtime::control and Runtime::snapshot locked, in that order.
    // All speech events go through this gate, including late terminal events.
    fn on_speech(&mut self, state: &mut Snapshot, event: speech::Event) -> SpeechUpdate {
        use speech::Event;
        let id = match &event {
            Event::Started { session_id }
            | Event::Progress { session_id, .. }
            | Event::Level { session_id, .. }
            | Event::Interim { session_id, .. }
            | Event::Preview { session_id, .. }
            | Event::Finishing { session_id }
            | Event::Cleaning { session_id, .. }
            | Event::Completed { session_id, .. }
            | Event::Cancelled { session_id }
            | Event::Error { session_id, .. } => *session_id,
        };
        if self.session != Some(id) || self.publishing {
            return SpeechUpdate::Ignored;
        }
        match event {
            Event::Progress { progress, .. } => {
                if !matches!(
                    state.state,
                    AppState::Loading | AppState::Listening | AppState::Finishing
                ) {
                    return SpeechUpdate::Ignored;
                }
                // Readiness follows the worker, not the microphone start cue.
                // A late load event must never reopen capture after Stop.
                if state.state != AppState::Finishing {
                    state.state = if progress.stage == "loading" {
                        AppState::Loading
                    } else {
                        AppState::Listening
                    };
                }
                state.recognition_progress = Some(progress);
            }
            Event::Started { .. } => {
                if state.state != AppState::Finishing {
                    state.state = AppState::Loading;
                }
            }
            Event::Level { rms, .. } => {
                if !matches!(state.state, AppState::Loading | AppState::Listening)
                    || self.last_meter.elapsed().as_millis() < 100
                {
                    return SpeechUpdate::Ignored;
                }
                self.last_meter = Instant::now();
                state.audio_level = rms.clamp(0.0, 1.0);
            }
            Event::Interim { text, .. } => {
                state.interim_text = text;
                if state.state != AppState::Finishing {
                    state.state = AppState::Listening;
                }
            }
            Event::Preview { text, .. } => {
                if !matches!(
                    state.state,
                    AppState::Loading | AppState::Listening | AppState::Finishing
                ) {
                    return SpeechUpdate::Ignored;
                }
                state.preview_text = text;
            }
            Event::Finishing { .. } => state.state = AppState::Finishing,
            Event::Cleaning { original_text, .. } => {
                state.recognition_progress = None;
                state.state = AppState::Cleaning;
                state.original_text = original_text.clone();
                state.output_text = original_text.clone();
                state.interim_text = original_text;
                state.preview_text.clear();
                state.audio_level = 0.0;
            }
            Event::Completed { result, .. } => {
                state.recognition_progress = None;
                state.original_text = result.original_text.clone();
                state.output_text = result.output_text.clone();
                state.pre_model_text = result.pre_model_text.clone();
                state.publication_stage = result.publication_stage.clone();
                state.guard_reason = result.guard_reason.clone();
                state.interim_text.clear();
                state.preview_text.clear();
                state.audio_level = 0.0;
                state.state = AppState::Finishing;
                state.result_session_id = Some(id);
                state.recognition_completeness = Some(result.recognition_completeness);
                state.recognition_warnings = result.recognition_warnings.clone();
                self.publishing = true;
                if result.recognition_completeness == RecognitionCompleteness::Incomplete {
                    state.delivery_notice = Some(RECOGNITION_WARNING_NOTICE.into());
                } else if !result.output_text.trim().is_empty() {
                    state.delivery_notice = Some("Text ready".into());
                }
                return SpeechUpdate::Publish(Publication {
                    session_id: id,
                    original_text: result.original_text,
                    text: result.output_text,
                    pre_model_text: result.pre_model_text,
                    publication_stage: result.publication_stage,
                    guard_reason: result.guard_reason,
                    target: self.target.take(),
                    auto_paste: state.settings.auto_paste,
                    policy_version: result.policy_version,
                    policy_hash: result.policy_hash,
                    recognition_completeness: result.recognition_completeness,
                    recognition_warnings: result.recognition_warnings,
                });
            }
            Event::Cancelled { .. } => {
                self.session = None;
                self.target = None;
                state.state = AppState::Idle;
                state.audio_level = 0.0;
                state.interim_text.clear();
                state.preview_text.clear();
                clear_review_text(state);
                state.session_id = None;
                state.capture_generation = None;
                self.update_activity = None;
                return SpeechUpdate::CloseCursor;
            }
            Event::Error { message, .. } => {
                state.recognition_progress = None;
                crate::recognition_diagnostics::event(
                    if message.starts_with("Memory unavailable:") {
                        "dictation-memory-denied"
                    } else {
                        "dictation-error"
                    },
                    &state.settings.asr_model,
                    std::process::id(),
                    id,
                    0,
                    0,
                );
                if state.tab_visible {
                    state.tab_expanded = true;
                }
                self.session = None;
                self.target = None;
                state.state = AppState::Error;
                state.error = Some(message.clone());
                state.audio_level = 0.0;
                // The draft is never promoted to text, even on failure.
                state.preview_text.clear();
                state.session_id = None;
                state.capture_generation = None;
                if !state.interim_text.trim().is_empty() {
                    state.original_text = state.interim_text.clone();
                    state.output_text = state.interim_text.clone();
                    state.result_session_id = Some(id);
                    state.recognition_completeness = Some(RecognitionCompleteness::Incomplete);
                    state.recognition_warnings = vec![message];
                    state.delivery_notice = Some(RECOGNITION_WARNING_NOTICE.into());
                }
                self.update_activity = None;
                return SpeechUpdate::FinishCursor;
            }
        }
        SpeechUpdate::Updated
    }

    fn finish_publication(&mut self, id: u64, state: &mut Snapshot) -> bool {
        if self.session != Some(id) || !self.publishing {
            return false;
        }
        self.session = None;
        self.publishing = false;
        state.state = AppState::Idle;
        state.session_id = None;
        state.capture_generation = None;
        self.update_activity = None;
        true
    }
}

pub fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "An operation was interrupted. Restart SonoJot before continuing.".into())
}

fn clear_review_text(state: &mut Snapshot) {
    state.recognition_progress = None;
    state.original_text.clear();
    state.output_text.clear();
    state.pre_model_text = None;
    state.publication_stage = None;
    state.guard_reason = None;
    state.interim_text.clear();
    state.preview_text.clear();
    state.result_session_id = None;
    state.recognition_completeness = None;
    state.recognition_warnings.clear();
    state.delivery_notice = None;
    state.error = None;
    state.can_copy_last = false;
}

fn stop_matches(ticket: (Option<u64>, u64), session: Option<u64>, generation: u64) -> bool {
    match ticket.0 {
        Some(expected) => session == Some(expected),
        None => session.is_some() && generation == ticket.1,
    }
}

fn tab_capture(snapshot: &Snapshot) -> bool {
    snapshot.meeting_recording
        || matches!(
            snapshot.state,
            AppState::Loading | AppState::Listening | AppState::Finishing | AppState::Cleaning
        )
}

fn capture_ticket(snapshot: &Snapshot, invalidation: &AtomicU64) -> (Option<u64>, u64) {
    let generation = invalidation.fetch_add(1, Ordering::AcqRel);
    (
        snapshot.session_id,
        snapshot.capture_generation.unwrap_or(generation),
    )
}

impl Runtime {
    fn clear_deleted_result(&self, id: Option<&str>) -> Result<(), String> {
        let mut current = lock(&self.current_record_id)?;
        if id.is_some() && current.as_deref() != id {
            return Ok(());
        }
        let clear_review = current.is_some();
        *current = None;
        lock(&self.last_published)?.clear();
        *lock(&self.last_delivery)? = None;
        let mut state = lock(&self.snapshot)?;
        state.can_copy_last = false;
        if clear_review {
            clear_review_text(&mut state);
        }
        Ok(())
    }

    pub(crate) fn delete_dictation_record(&self, id: &str, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Confirm deletion of this dictation first.".into());
        }
        let control = lock(&self.control)?;
        if control.publishing {
            return Err("Wait for the current copy or delivery to finish before deleting.".into());
        }
        // Validate existence before clearing the association. A failed derivative
        // or record deletion still clears Copy-last; it never touches the clipboard.
        self.store.get_dictation(id)?;
        self.clear_deleted_result(Some(id))?;
        self.store.delete_dictation(id, true)
    }

    pub(crate) fn delete_all_dictation_records(&self, confirmed: bool) -> Result<usize, String> {
        if !confirmed {
            return Err("Confirm deletion of all saved dictations first.".into());
        }
        let control = lock(&self.control)?;
        if control.publishing {
            return Err("Wait for the current copy or delivery to finish before deleting.".into());
        }
        self.clear_deleted_result(None)?;
        self.store.delete_all_dictations(true)
    }

    pub(crate) fn claim_last_copy(&self, expected: &str) -> Result<ManualCopyClaim<'_>, String> {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Dictation)?;
        let mut control = lock(&self.control)?;
        let state = lock(&self.snapshot)?;
        if expected.is_empty() || *lock(&self.last_published)? != expected || !state.can_copy_last {
            return Err("This dictation is no longer available to copy.".into());
        }
        control.claim_manual_copy(&state, state.result_session_id)?;
        Ok(ManualCopyClaim {
            _update_activity: activity,
            control: &self.control,
        })
    }

    pub(crate) fn claim_history_copy(&self, id: &str) -> Result<ManualCopyClaim<'_>, String> {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Dictation)?;
        let mut control = lock(&self.control)?;
        if control.publishing || control.session.is_some() {
            return Err("Finish dictation or copying before copying history.".into());
        }
        self.store.get_dictation(id)?;
        // Also prevents a source deletion between source validation and clipboard write.
        control.publishing = true;
        Ok(ManualCopyClaim {
            _update_activity: activity,
            control: &self.control,
        })
    }

    pub(crate) fn save_correction(
        &self,
        id: &str,
        text: &str,
        scope: Option<&str>,
    ) -> Result<SavedCorrection, String> {
        let control = lock(&self.control)?;
        if control.publishing {
            return Err("Wait for the current copy or delivery to finish before editing.".into());
        }
        let saved = self.store.save_dictation_correction(id, text, scope)?;
        if lock(&self.current_record_id)?.as_deref() == Some(id) {
            let mut state = lock(&self.snapshot)?;
            state.output_text = saved.record.published.clone();
            state.pre_model_text = saved.record.pre_model_text.clone();
            state.publication_stage = saved.record.publication_stage.clone();
            state.guard_reason = saved.record.guard_reason.clone();
            if state.can_copy_last {
                *lock(&self.last_published)? = saved.record.published.clone();
                state.can_copy_last = !saved.record.published.trim().is_empty();
            }
        }
        Ok(saved)
    }

    pub(crate) fn claim_manual_copy(
        &self,
        result: Option<u64>,
    ) -> Result<ManualCopyClaim<'_>, String> {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Dictation)?;
        let mut control = lock(&self.control)?;
        let state = lock(&self.snapshot)?;
        control.claim_manual_copy(&state, result)?;
        Ok(ManualCopyClaim {
            _update_activity: activity,
            control: &self.control,
        })
    }

    pub fn create(app: &AppHandle) -> Result<Arc<Self>, String> {
        let store = Store::open(None)?;
        let migration = store.import_legacy_default();
        let mut settings = store.load_settings()?;
        crate::acceleration::set_preference(settings.text_acceleration);
        if let Ok(executable) = std::env::current_exe() {
            if let Ok(enabled) = platform::reconcile_startup_registration(&executable) {
                settings.launch_at_login = enabled;
            }
        }
        let models_root = store.models_dir();
        let history = store.list_dictations(100);
        let error = migration.err().or_else(|| history.as_ref().err().cloned());
        // Copy-last is temporary. Saved history remains accessible by record ID.
        let last_published = String::new();
        let snapshot = Snapshot {
            // The menu bar icon is the idle control. The sheet stays closed until
            // a click or a new recording.
            tab_visible: !cfg!(target_os = "macos") && !settings.start_tab_hidden,
            can_copy_last: !last_published.is_empty(),
            settings,
            // CoreAudio can block while inspecting an unavailable device.
            // get_snapshot refreshes devices on a worker after the window opens.
            devices: Vec::new(),
            error,
            // Multi-gigabyte checksum verification belongs to get_snapshot's
            // blocking worker, after the desktop window is visible. Doing it
            // here stalls Tauri setup/message handling for tens of seconds.
            // An empty inventory is unverified, never permission to load/paste.
            models: Vec::new(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: Capabilities {
                auto_paste: platform::auto_paste_available(),
                system_audio: crate::system_audio::status() == crate::system_audio::SystemAudioStatus::Ready,
                system_audio_status: crate::system_audio::status(),
                global_shortcut: false,
                platform: std::env::consts::OS.into(),
            },
            ..Snapshot::default()
        };
        let (sender, events) = mpsc::channel();
        let engine = speech::Engine::new(
            models_root.clone(),
            Arc::new(move |event| {
                let _ = sender.send(event);
            }),
        );
        let runtime = Arc::new(Self {
            updater: crate::updater::UpdateService::default(),
            meetings: Meetings::new(store.clone()),
            store,
            models_root,
            snapshot: Mutex::new(snapshot),
            control: Mutex::new(Control {
                microphone_preview: None,
                update_activity: None,
                session: None,
                generation: 0,
                publishing: false,
                target: None,
                last_meter: Instant::now(),
                privacy_generation: 0,
            }),
            engine,
            recorder: Mutex::new(None),
            meeting_update_activity: Mutex::new(None),
            settings_gate: tokio::sync::Mutex::new(()),
            last_delivery: Mutex::new(None),
            last_published: Mutex::new(last_published),
            current_record_id: Mutex::new(None),
            tab_target: Mutex::new(None),
            command_generation: AtomicU64::new(0),
            tab_gate: Mutex::new(()),
        });
        let weak = Arc::downgrade(&runtime);
        let handle = app.clone();
        std::thread::Builder::new()
            .name("dictation-events".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    let Some(runtime) = weak.upgrade() else { break };
                    if let Err(error) = runtime.on_speech(&handle, event) {
                        runtime.report(&handle, error);
                    }
                }
            })
            .map_err(|e| format!("Cannot start the dictation event handler: {e}"))?;
        Ok(runtime)
    }

    pub fn prewarm_selected_models(&self) {
        let Ok(snapshot) = self.snapshot() else {
            return;
        };
        if snapshot.settings.setup_profile.is_none()
            || snapshot.settings.model_retention != ModelRetention::OnExit
        {
            return;
        }
        let _ = self.engine.prewarm(snapshot.settings.asr_model);
        if snapshot.settings.cleanup_model == "local-rules"
            || (snapshot.settings.cleanup_model == crate::optional_llm::SMART_CLEANUP_ID
                && snapshot.settings.cleanup_mode != CleanupMode::Clean)
        {
            return;
        }
        if crate::resource_manager::acquire_text_with_admission(
            "cleanup",
            &snapshot.settings.cleanup_model,
            "startup prewarm",
        )
        .is_ok()
        {
            let _ =
                crate::cleanup::warm_blocking(&self.models_root, &snapshot.settings.cleanup_model);
            crate::resource_manager::note_text_loaded(&snapshot.settings.cleanup_model);
            crate::resource_manager::notify_text_finished();
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snapshot = lock(&self.snapshot)?;
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.meetings_revision = self.meetings.revision();
        let mut value = snapshot.clone();
        drop(snapshot);
        // Selection commits take coordinator -> snapshot. Never invert that
        // order by holding the snapshot while reading update admission.
        value.update = self.updater.snapshot()?;
        Ok(value)
    }
    pub fn emit(&self, app: &AppHandle) {
        if let Ok(value) = self.snapshot() {
            #[cfg(target_os = "macos")]
            crate::menu_panel::sync_capture(app, tab_capture(&value));
            let _ = app.emit("app-state", value);
            self.update_tray(app);
        }
    }

    fn update_tray(&self, app: &AppHandle) {
        let tooltip = self.snapshot.lock().ok().map(|snapshot| {
            let status = if snapshot.meeting_recording {
                "Meeting recording"
            } else {
                match snapshot.state {
                    AppState::Idle => "Idle",
                    AppState::Loading => "Preparing transcription",
                    AppState::Listening => "Listening",
                    AppState::Finishing => "Finishing",
                    AppState::Cleaning => "Cleaning up text",
                    AppState::Error => "Needs attention",
                }
            };
            format!("SonoJot · {} · {status}", snapshot.settings.hotkey)
        });
        if let (Some(tray), Some(tooltip)) = (app.tray_by_id("sonojot"), tooltip) {
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }

    pub fn set_tab(
        &self,
        app: &AppHandle,
        visible: bool,
        expanded: Option<bool>,
        focus: bool,
    ) -> Result<Snapshot, String> {
        // A focus-taking activation (tray, keyboard expand) is an explicit show.
        self.apply_tab(app, visible, expanded, focus, focus)
    }

    /// `pin` is the explicit Keep/restore signal: it clears a dwell reveal's
    /// temporary state without requiring window focus.
    pub(crate) fn apply_tab(
        &self,
        app: &AppHandle,
        visible: bool,
        expanded: Option<bool>,
        focus: bool,
        pin: bool,
    ) -> Result<Snapshot, String> {
        let _gate = lock(&self.tab_gate)?;
        let returning_focus =
            windows::screen_tab_has_focus(app) && (!visible || expanded == Some(false));
        if focus && !windows::screen_tab_has_focus(app) {
            *lock(&self.tab_target)? = platform::capture_paste_target();
        }
        let expanded = if cfg!(target_os = "macos") {
            visible
        } else {
            visible && expanded.unwrap_or(self.snapshot()?.tab_expanded)
        };
        if returning_focus {
            if let Some(target) = lock(&self.tab_target)?.as_ref() {
                platform::restore_target_focus(target);
            }
        }
        let snapshot = self.snapshot()?;
        let capture = tab_capture(&snapshot);
        // Hover expand/collapse keeps a dwell reveal temporary; a pinned or
        // focus-taking request clears it. Hidden tabs stay edge-discoverable.
        let temporary = !cfg!(target_os = "macos") && snapshot.tab_temporary && visible && !pin;
        windows::set_screen_tab(app, visible, expanded, focus, capture, temporary)?;
        {
            let mut snapshot = lock(&self.snapshot)?;
            snapshot.tab_visible = visible;
            snapshot.tab_expanded = expanded;
            snapshot.tab_temporary = temporary;
        }
        self.emit(app);
        self.snapshot()
    }

    /// The top-edge poll found the hidden tab: show it expanded without
    /// focus. The edge watcher conceals it once it sits empty past its grace.
    #[cfg_attr(target_os = "macos", allow(unreachable_code, unused_variables))]
    pub fn reveal_temporary_tab(&self, app: &AppHandle, generation: u64) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            // The menu bar icon is always discoverable. There is no top-edge dwell.
            return Ok(());
        }
        let _gate = lock(&self.tab_gate)?;
        let snapshot = self.snapshot()?;
        if snapshot.tab_visible || !windows::edge_reveal_due(app, generation) {
            return Ok(());
        }
        let capture = tab_capture(&snapshot);
        windows::set_screen_tab(app, true, true, false, capture, true)?;
        {
            let mut snapshot = lock(&self.snapshot)?;
            snapshot.tab_visible = true;
            snapshot.tab_expanded = true;
            snapshot.tab_temporary = true;
        }
        self.emit(app);
        windows::tab_diagnostic("tab-edge-revealed", 0, generation);
        Ok(())
    }

    /// A temporary reveal outlived its grace: re-verify under the gate so a
    /// pointer that re-entered after the watcher's decision keeps it open.
    pub fn conceal_temporary_tab(&self, app: &AppHandle) -> Result<(), String> {
        let _gate = lock(&self.tab_gate)?;
        if !self.snapshot()?.tab_temporary || !windows::temporary_conceal_due(app) {
            return Ok(());
        }
        windows::set_screen_tab(app, false, false, false, false, false)?;
        {
            let mut snapshot = lock(&self.snapshot)?;
            snapshot.tab_visible = false;
            snapshot.tab_expanded = false;
            snapshot.tab_temporary = false;
        }
        self.emit(app);
        windows::tab_diagnostic("tab-edge-concealed", 0, 0);
        Ok(())
    }

    pub fn refresh_tab(&self, app: &AppHandle) -> Result<(), String> {
        let _gate = lock(&self.tab_gate)?;
        let snapshot = self.snapshot()?;
        let capture = tab_capture(&snapshot);
        let focus = snapshot.tab_expanded && windows::screen_tab_has_focus(app);
        windows::set_screen_tab(
            app,
            snapshot.tab_visible,
            snapshot.tab_expanded,
            focus,
            capture,
            snapshot.tab_temporary,
        )
    }

    pub(crate) fn refresh_tab_placement(
        &self,
        app: &AppHandle,
        monitors: &[windows::TabMonitor],
    ) -> Result<(), String> {
        // Display maintenance must not restore an old region after Hide or race
        // a native show before its visible state has been committed.
        let _gate = lock(&self.tab_gate)?;
        if let Some(window) = app.get_webview_window("overlay") {
            windows::place_screen_tab_on_monitors(&window, monitors)?;
        }
        Ok(())
    }
    pub fn report(&self, app: &AppHandle, error: String) {
        if let Ok(mut state) = self.snapshot.lock() {
            state.error = Some(error);
        }
        self.emit(app);
    }

    /// A completed dictation's log detail carries the active cleanup policy so
    /// the trial can verify which path produced the delivered text.
    fn delivery_log_detail(diagnostics: &str, publication: &Publication) -> String {
        let detail = format!(
            "{diagnostics} | cleanupPolicyVersion={} cleanupPolicyHash={} recognitionCompleteness={} recognitionWarnings={}",
            publication.policy_version,
            publication.policy_hash,
            publication.recognition_completeness,
            publication.recognition_warnings.len()
        );
        detail
    }

    pub(crate) fn log_delivery(&self, detail: String) {
        // Diagnostics may include external window/error text. Never persist
        // this string, in either retention mode.
        if let Ok(mut last) = self.last_delivery.lock() {
            *last = Some(detail);
        }
    }

    pub fn start_at_generation(
        &self,
        app: &AppHandle,
        target: Option<platform::PasteTarget>,
        generation: u64,
    ) -> Result<Snapshot, String> {
        self.start_capture_at_generation(app, target, generation, crate::setup_commands::CaptureKind::Dictation)
    }

    pub(crate) fn start_microphone_test_at_generation(
        &self,
        app: &AppHandle,
        generation: u64,
    ) -> Result<Snapshot, String> {
        self.start_capture_at_generation(app, None, generation, crate::setup_commands::CaptureKind::MicrophoneTest)
    }

    fn start_capture_at_generation(
        &self,
        app: &AppHandle,
        target: Option<platform::PasteTarget>,
        generation: u64,
        kind: crate::setup_commands::CaptureKind,
    ) -> Result<Snapshot, String> {
        let update_activity = crate::resource_manager::begin_update_activity(
            crate::resource_manager::UpdateActivity::Dictation,
        )?;
        let mut control = lock(&self.control)?;
        if !control.can_start(generation, &self.command_generation)? {
            return self.snapshot();
        }
        if lock(&self.recorder)?.is_some() {
            return Err("Stop the meeting recording before starting dictation.".into());
        }
        let settings = self.snapshot()?.settings;
        if let Err(error) = crate::setup_commands::require_setup(
            &settings,
            kind,
        ) {
            drop(control);
            self.report(app, error.clone());
            // A hotkey or tray attempt needs a visible recovery action even
            // when the screen tab was hidden. Reveal the existing sheet passively.
            let _ = self.apply_tab(app, true, Some(true), false, true);
            return Err(error);
        }
        let mut vocabulary = Vec::new();
        let mut confirmed = Vec::new();
        for entry in self
            .store
            .get_vocabulary()?
            .into_iter()
            .filter(|v| v.enabled)
        {
            // Scoped entries are explicit user confirmations and always reach
            // the structured stage with their scope (daily-v6 default/UI
            // promise). The `cleanup_trial` bool is preserved in Settings only
            // for migration/evidence and no longer gates runtime vocabulary.
            // Unscoped entries keep using only the existing host path. Nothing
            // here is derived from dictation history.
            let _ = settings.cleanup_trial;
            if let Some(scope) = entry
                .scope
                .as_deref()
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
            {
                confirmed.push(crate::structured_clean::VocabularyEntry {
                    id: Some(entry.id.clone()),
                    canonical: entry.written.clone(),
                    aliases: vec![entry.heard.clone()],
                    scope: scope.to_owned(),
                    status: crate::structured_clean::VocabularyStatus::Confirmed,
                    provenance:
                        crate::structured_clean::VocabularyProvenance::ExplicitUserConfirmation,
                });
            }
            vocabulary.push(cleanup::Vocabulary {
                heard: entry.heard,
                written: entry.written,
                enabled: true,
                context: vec![],
            });
        }
        // Learned aliases require actual ASR text and nearby scope evidence.
        // Speech retrieves them with learned_cleanup_vocabulary_for after
        // recognition (full raw + mapped mode + 1536 budget via the Store
        // handle below); never inject a global list at capture start.
        crate::ai_commands::cancel_optional_jobs();
        self.meetings.automatic_titles.cancel();
        let options = speech::StartOptions::from_settings_with_store(
            settings.clone(),
            vocabulary,
            confirmed,
            Some(self.store.clone()),
        );
        control.target = target;
        control.generation = generation;
        control.privacy_generation = self.store.privacy_generation();
        *lock(&self.current_record_id)? = None;
        lock(&self.last_published)?.clear();
        *lock(&self.last_delivery)? = None;
        self.log_delivery(format!(
            "capture {}",
            platform::paste_target_diagnostics(control.target.as_ref())
        ));
        {
            let mut state = lock(&self.snapshot)?;
            state.state = AppState::Loading;
            state.capture_generation = Some(generation);
            state.error = None;
            state.interim_text.clear();
            state.preview_text.clear();
            state.original_text.clear();
            state.output_text.clear();
            state.pre_model_text = None;
            state.publication_stage = None;
            state.guard_reason = None;
            state.can_copy_last = false;
            state.recognition_completeness = None;
            state.recognition_warnings.clear();
            state.audio_level = 0.0;
            state.delivery_notice = None;
            state.result_session_id = None;
            // A new recording reveals the strip; Hide remains effective until the next start.
            state.tab_visible = true;
            state.tab_expanded = true;
            state.tab_temporary = false;
        }
        match self.engine.start(options) {
            Ok(id) => {
                control.update_activity = Some(update_activity);
                control.session = Some(id);
                lock(&self.snapshot)?.session_id = Some(id);
                if settings.audio_cues {
                    self.engine.play_start_cue(id);
                }
            }
            Err(error) => {
                let mut state = lock(&self.snapshot)?;
                state.state = AppState::Error;
                state.capture_generation = None;
                state.error = Some(error.clone());
                return Err(error);
            }
        }
        // A settings-triggered start has no external target. The overlay never activates it.
        if let Err(error) = self.refresh_tab(app) {
            self.report(app, error);
        }
        if settings.cursor_indicator {
            if let Err(error) = windows::show_cursor(app) {
                self.report(app, error);
            }
        }
        // macOS: starting from another app must leave that app in front. If
        // anything made SonoJot active while capture started, hand activation
        // back; restore_target_focus acts only while SonoJot is frontmost.
        #[cfg(target_os = "macos")]
        if let Some(target) = control.target.clone() {
            std::thread::spawn(move || {
                for wait in [150, 350, 500] {
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                    platform::restore_target_focus(&target);
                }
            });
        }
        // Keep early completion/error events behind UI creation so they cannot leave a stale indicator.
        drop(control);
        self.emit(app);
        self.snapshot()
    }

    pub fn stop_ticket(&self) -> Result<(Option<u64>, u64), String> {
        // Fence queued starts immediately, before waiting for the capture owner.
        let snapshot = lock(&self.snapshot)?;
        Ok(capture_ticket(&snapshot, &self.command_generation))
    }

    pub fn stop_at(&self, app: &AppHandle, ticket: (Option<u64>, u64)) -> Result<Snapshot, String> {
        let control = lock(&self.control)?;
        if !control.can_stop_capture(ticket, self.snapshot()?.state) {
            return self.snapshot();
        }
        self.engine.stop()?;
        lock(&self.snapshot)?.state = AppState::Finishing;
        drop(control);
        if self.snapshot()?.settings.audio_cues {
            audio::play_cue("stop");
        }
        self.emit(app);
        self.snapshot()
    }

    pub fn cancel_at(
        &self,
        app: &AppHandle,
        ticket: (Option<u64>, u64),
    ) -> Result<Snapshot, String> {
        let mut control = lock(&self.control)?;
        {
            let mut state = lock(&self.snapshot)?;
            if !control.cancel(ticket, &mut state)? {
                drop(state);
                return self.snapshot();
            }
        }
        self.engine.cancel();
        drop(control);
        windows::close_cursor(app);
        self.emit(app);
        self.snapshot()
    }

    pub fn stop_cleanup_at(&self, ticket: (Option<u64>, u64)) -> Result<Snapshot, String> {
        let control = lock(&self.control)?;
        if control.can_stop_cleanup(ticket, self.snapshot()?.state) {
            self.engine.stop_cleanup()?;
        }
        self.snapshot()
    }

    fn on_speech(self: &Arc<Self>, app: &AppHandle, event: speech::Event) -> Result<(), String> {
        let mut control = lock(&self.control)?;
        let mut state = lock(&self.snapshot)?;
        match control.on_speech(&mut state, event) {
            SpeechUpdate::Ignored => return Ok(()),
            SpeechUpdate::Updated => {}
            SpeechUpdate::CloseCursor => windows::close_cursor(app),
            SpeechUpdate::FinishCursor => windows::finish_cursor(app),
            SpeechUpdate::Publish(mut publication) => {
                // Persist raw before attempting delivery. A storage error does not discard the clipboard result.
                let saved = if !publication.original_text.trim().is_empty()
                    || !publication.text.trim().is_empty()
                    || publication.recognition_completeness == RecognitionCompleteness::Incomplete
                {
                    self.store.commit_dictation_with_metadata(
                        control.privacy_generation,
                        &publication.original_text,
                        &publication.text,
                        publication.recognition_completeness,
                        &publication.recognition_warnings,
                        publication.pre_model_text.as_deref(),
                        publication.publication_stage.as_deref(),
                        publication.guard_reason.as_deref(),
                    )
                } else {
                    Ok(None)
                };
                *lock(&self.current_record_id)? = saved
                    .as_ref()
                    .ok()
                    .and_then(|entry| entry.as_ref().map(|entry| entry.id.clone()));
                drop(state);
                drop(control);
                if let Err(error) = saved {
                    self.report(app, error);
                }
                if let Ok(Some(id)) = self.current_record_id.lock().map(|id| id.clone()) {
                    let _ = app.emit("dictation-history-changed", id);
                }
                let runtime = self.clone();
                let delivery_app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(delivered) = publication.deliver_with(platform::deliver_text).await
                    {
                        match delivered {
                            Ok(outcome) => {
                                if let Ok(mut last) = runtime.last_published.lock() {
                                    *last = publication.text.clone();
                                }
                                if let Ok(mut snapshot) = runtime.snapshot.lock() {
                                    snapshot.can_copy_last = true;
                                }
                                runtime.log_delivery(Runtime::delivery_log_detail(
                                    &outcome.diagnostics,
                                    &publication,
                                ));
                                if publication.auto_paste
                                    && !outcome.paste_sent
                                    && publication.recognition_completeness
                                        == RecognitionCompleteness::Complete
                                {
                                    if let Ok(mut snapshot) = runtime.snapshot.lock() {
                                        snapshot.delivery_notice = Some(
                                            "Text ready. Copy it to paste in your editor.".into(),
                                        );
                                    }
                                }
                            }
                            Err(error) => {
                                runtime.log_delivery(Runtime::delivery_log_detail(
                                    &format!("delivery_error: {error}"),
                                    &publication,
                                ));
                                runtime.report(&delivery_app, error);
                            }
                        }
                    } else {
                        let status = if publication.recognition_completeness
                            == RecognitionCompleteness::Incomplete
                        {
                            "delivery_blocked_incomplete"
                        } else {
                            "delivery_skipped_empty"
                        };
                        runtime.log_delivery(Runtime::delivery_log_detail(status, &publication));
                    }
                    if let Ok(mut control) = runtime.control.lock() {
                        if let Ok(mut state) = runtime.snapshot.lock() {
                            if control.finish_publication(publication.session_id, &mut state) {
                                let audio_cues = state.settings.audio_cues;
                                drop(state);
                                // Capture the cursor generation before allowing another start.
                                windows::finish_cursor(&delivery_app);
                                if audio_cues {
                                    audio::play_cue("complete");
                                }
                            }
                        }
                    }
                    runtime.emit(&delivery_app);
                });
                self.emit(app);
                return Ok(());
            }
        }
        drop(state);
        drop(control);
        self.emit(app);
        Ok(())
    }
}

pub fn model_catalog(root: &std::path::Path) -> Vec<Model> {
    use crate::model_verification::Check;
    let moonshine_medium = crate::asr::model_status(root, "moonshine-medium", Check::Background);
    let moonshine_small = crate::asr::model_status(root, "moonshine-small", Check::Background);
    let mut models = vec![
        Model {
            id: "moonshine-medium".into(),
            name: "Moonshine Medium Streaming".into(),
            kind: ModelKind::Asr,
            ready: moonshine_medium.ready(),
            state: moonshine_medium.label().into(),
            description: "English · 245 million parameters".into(),
            size_bytes: folder_size(&crate::asr::model_display_directory(root, "moonshine-medium")),
            installable: false,
            removable: false,
        },
        Model {
            id: "moonshine-small".into(),
            name: "Moonshine Small Streaming".into(),
            kind: ModelKind::Asr,
            ready: moonshine_small.ready(),
            state: moonshine_small.label().into(),
            description: "English · lighter 123 million parameter option".into(),
            size_bytes: folder_size(&crate::asr::model_display_directory(root, "moonshine-small")),
            installable: false,
            removable: false,
        },
    ];
    for optional in crate::optional_asr::catalog() {
        let status = crate::optional_asr::model_status(root, optional.id, Check::Background);
        let ready = status.ready();
        models.push(Model {
            id: optional.id.into(),
            name: optional.name.into(),
            kind: ModelKind::Asr,
            ready,
            state: status.label().into(),
            description: optional.description.into(),
            size_bytes: if ready {
                folder_size(&crate::optional_asr::model_dir(root, optional.id))
            } else {
                crate::optional_asr::package_bytes(optional)
            },
            installable: true,
            removable: ready,
        });
    }
    // Round 6 Whisper catalog (ASR-owned whisper_catalog.rs, parent-wired here
    // per asr-interface.md §5). RAM/VRAM stay in the library mapping, never
    // synthesized here; this snapshot catalog keeps exact installed size when
    // ready, package bytes otherwise. CPU-only backend/compatibility verbatim.
    for entry in crate::whisper_catalog::catalog_metadata() {
        let status = crate::whisper_catalog::model_status(root, entry.id, Check::Background);
        let ready = status.ready();
        models.push(Model {
            id: entry.id.into(),
            name: entry.name.into(),
            kind: ModelKind::Asr,
            ready,
            state: status.label().into(),
            description: entry.reason.into(),
            size_bytes: if ready {
                folder_size(&crate::whisper_catalog::model_dir(root, entry.id))
            } else {
                entry.disk_bytes
            },
            installable: entry.installable,
            removable: ready,
        });
    }
    // Real GPU catalog (gpu_asr worker, CUDA 13.x only, NVIDIA-only, AMD has
    // no CUDA path). Ready uses the real folder size; not-ready uses model
    // bytes with the shared runtime disclosed in the description (first
    // install needs ~1.43 GiB extra, one-time, shared by both GPU models).
    // Requested provider "cuda" comes from the ID; observed execution is never
    // claimed here (see acceleration::speech_detail).
    for entry in crate::gpu_asr::catalog_metadata() {
        let status = crate::gpu_asr::model_status(root, entry.id, Check::Background);
        let ready = status.ready();
        let runtime_ready = crate::gpu_asr::runtime_status(root, Check::Background).ready();
        let description = if runtime_ready {
            entry.reason.to_owned()
        } else {
            format!(
                "{} First install adds the shared NVIDIA CUDA 13 runtime (~1.43 GiB, one-time).",
                entry.reason
            )
        };
        models.push(Model {
            id: entry.id.into(),
            name: entry.name.into(),
            kind: ModelKind::Asr,
            ready,
            state: status.label().into(),
            description,
            size_bytes: if ready {
                folder_size(&crate::gpu_asr::model_dir(root, entry.id))
            } else {
                entry.disk_bytes
            },
            installable: entry.installable,
            removable: ready,
        });
    }
    for entry in crate::llama_asr::catalog() {
        let status = crate::llama_asr::model_status(root, entry.id, Check::Background);
        let ready = status.ready();
        models.push(Model {
            id: entry.id.into(),
            name: entry.name.into(),
            kind: ModelKind::Asr,
            ready,
            state: status.label().into(),
            description: entry.description.into(),
            size_bytes: if ready {
                folder_size(&crate::llama_asr::model_dir(root, entry.id))
            } else {
                crate::llama_asr::package_bytes(entry)
            },
            installable: entry.runtime.supported_platform(),
            removable: ready,
        });
    }
    for optional in crate::optional_llm::catalog() {
        let removable = crate::optional_llm::optional_copy_status(root, optional.id, Check::Background).ready();
        let status = cleanup::model_status(root, optional.id, Check::Background);
        let ready = status.ready();
        models.push(Model {
            id: optional.id.into(),
            name: optional.name.into(),
            kind: ModelKind::Cleanup,
            ready,
            state: status.label().into(),
            description: optional.description.into(),
            size_bytes: if removable {
                crate::optional_llm::package_bytes(optional)
            } else if ready {
                std::fs::metadata(crate::asr::bundle_path(
                    root,
                    "cleanup_gguf",
                    "llama.cpp/models/LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
                ))
                .map(|m| m.len())
                .unwrap_or_else(|_| crate::optional_llm::package_bytes(optional))
            } else {
                crate::optional_llm::package_bytes(optional)
            },
            installable: true,
            removable,
        });
    }
    models.push(Model {
        id: "local-rules".into(),
        name: "SonoJot Punctuation with your vocabulary".into(),
        kind: ModelKind::Cleanup,
        ready: true,
        state: "installed".into(),
        description: "Rules, your vocabulary and the SonoJot punctuation model; CPU".into(),
        size_bytes: 0,
        installable: false,
        removable: false,
    });
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let native = crate::mac_speech::catalog().iter().map(|m| {
            let status = crate::mac_speech::model_status(root, m.id, Check::Background);
            Model {
            id: m.id.into(), name: m.name.into(), kind: ModelKind::Asr,
            ready: status.ready(), state: status.label().into(), description: "Multilingual · Apple Metal GPU".into(),
            size_bytes: m.bytes, installable: true, removable: status.ready(),
        }});
        models.splice(0..0, native);
        for model in &mut models {
            if crate::mac_speech::find(&model.id).is_none()
                && !crate::llama_asr::is_metal(&model.id)
                && !matches!(model.id.as_str(), "lfm2.5-1.2b" | "lfm2.5-2.6b" | "qwen3-4b-instruct-2507" | "local-rules" | crate::optional_llm::SMART_CLEANUP_ID) {
                model.installable = false;
            }
        }
    }
    models
}

fn folder_size(path: &std::path::Path) -> u64 {
    crate::model_manager::folder_size(path)
}

#[cfg(test)]
mod metal_catalog_tests {
    #[test]
    fn snapshot_catalog_respects_each_llama_runtime_platform() {
        let root = std::env::temp_dir().join(format!("sonojot-catalog-{}", uuid::Uuid::new_v4()));
        let models = super::model_catalog(&root);
        for row in crate::llama_asr::catalog() {
            let model = models.iter().find(|model| model.id == row.id).unwrap();
            assert_eq!(model.installable, row.runtime.supported_platform(), "{}", row.id);
            assert!(!model.ready);
        }
    }
}

fn shortcut_action(state: AppState, mode: HotkeyMode, key: ShortcutState) -> u8 {
    if state == AppState::Cleaning {
        return if key == ShortcutState::Pressed { 3 } else { 0 };
    }
    match (mode, key) {
        (HotkeyMode::Toggle, ShortcutState::Pressed) => match state {
            AppState::Loading | AppState::Listening => 2,
            AppState::Finishing => 0,
            _ => 1,
        },
        (HotkeyMode::Hold, ShortcutState::Pressed) => {
            if matches!(state, AppState::Idle | AppState::Error) {
                1
            } else {
                0
            }
        }
        (HotkeyMode::Hold, ShortcutState::Released) => 2,
        _ => 0,
    }
}

/// Shortcuts bound by the compositor instead of a key grab. Wayland gives
/// clients no global keys; Hyprland runs `SonoJot --shortcut ...` instead.
pub fn compositor_binds() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::hyprland::active()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

pub fn register_shortcut(app: &AppHandle, value: &str, mode: HotkeyMode) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::hyprland::active() {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        return crate::hyprland::install_shortcut(value, mode == HotkeyMode::Hold, &executable)
            .map_err(|e| format!("Cannot bind {value} in Hyprland: {e}"));
    }
    let _ = mode;
    let shortcut: Shortcut = value
        .replace(' ', "")
        .parse()
        .map_err(|e| format!("Invalid keyboard shortcut: {e}"))?;
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| shortcut_event(app, event.state()))
        .map_err(|e| format!("Cannot register {value}. Another app may be using it: {e}"))
}

/// One press or release of the dictation shortcut, from the key grab or from
/// a compositor bind.
pub fn shortcut_event(app: &AppHandle, key: ShortcutState) {
    let Some(runtime) = app.try_state::<Arc<Runtime>>() else {
        return;
    };
    let runtime = runtime.inner().clone();
    let app = app.clone();
    let Ok(snapshot) = runtime.snapshot() else {
        return;
    };
    let action = shortcut_action(snapshot.state, snapshot.settings.hotkey_mode, key);
    if action == 0 {
        return;
    }
    // Preserve the focused editor at the key event, before any app UI or worker delay.
    let target = if action == 1 {
        platform::capture_paste_target()
    } else {
        None
    };
    let generation = runtime.command_generation.load(Ordering::Acquire);
    let stop_ticket = if action == 2 || action == 3 {
        runtime.stop_ticket().ok()
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || {
        let result = if action == 1 {
            runtime.start_at_generation(&app, target, generation)
        } else if action == 3 {
            let Some(ticket) = stop_ticket else { return };
            runtime.stop_cleanup_at(ticket)
        } else {
            let Some(ticket) = stop_ticket else { return };
            runtime.stop_at(&app, ticket)
        };
        if let Err(error) = result {
            runtime.report(&app, error);
        }
    });
}

#[cfg(test)]
mod publication_boundary_tests {
    // These tests exercise the production Control event/command gate and the
    // production delivery dispatch with synthetic events and an injected sink.
    // They do not exercise capture, model generation, hotkeys, windows, history
    // persistence, clipboard access, focus restoration, or OS paste delivery.
    use super::*;
    use speech::{DictationResult, Event};

    #[derive(Default)]
    struct FakeDelivery {
        calls: Mutex<Vec<(String, bool)>>,
    }

    impl FakeDelivery {
        async fn deliver(
            &self,
            text: &str,
            target: Option<platform::PasteTarget>,
            auto_paste: bool,
        ) -> Result<platform::DeliveryOutcome, String> {
            // No native target is captured or manufactured by these tests.
            assert!(target.is_none());
            self.calls.lock().unwrap().push((text.into(), auto_paste));
            Ok(platform::DeliveryOutcome {
                paste_sent: false,
                reason: Some("Injected test sink; no OS delivery".into()),
                reason_code: "test_sink",
                diagnostics: "test_sink".into(),
            })
        }
    }

    fn executor() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    fn active(id: u64, generation: u64, state: AppState) -> (Control, Snapshot) {
        (
            Control {
                microphone_preview: None,
                update_activity: None,
                session: Some(id),
                generation,
                publishing: false,
                target: None,
                last_meter: Instant::now() - std::time::Duration::from_secs(1),
                privacy_generation: 0,
            },
            Snapshot {
                session_id: Some(id),
                capture_generation: Some(generation),
                state,
                original_text: "Earlier raw text".into(),
                output_text: "Earlier published text".into(),
                interim_text: "Pending capture".into(),
                audio_level: 0.5,
                ..Default::default()
            },
        )
    }

    fn completed(id: u64, raw: &str, output: &str, status: &str) -> Event {
        completed_with_policy(
            id,
            raw,
            output,
            status,
            &cleanup::CleanupPolicy::legacy(),
            RecognitionCompleteness::Complete,
            Vec::new(),
        )
    }

    #[test]
    fn smart_publication_retains_recovery_and_cancel_clears_it() {
        let (mut control, mut state) = active(7, 2, AppState::Cleaning);
        let mut event = completed(7, "raw recognition", "Send the summary.", "local_model_validated");
        if let Event::Completed { result, .. } = &mut event {
            result.pre_model_text = Some("Send the summary the summary.".into());
            result.publication_stage = Some("smart_cleanup".into());
        }
        let SpeechUpdate::Publish(publication) = control.on_speech(&mut state, event) else { panic!("publication missing"); };
        assert_eq!(publication.original_text, "raw recognition");
        assert_eq!(publication.pre_model_text, state.pre_model_text);
        assert_eq!(publication.publication_stage, state.publication_stage);
        assert!(control.is_publishing());
        assert!(control.finish_publication(7, &mut state));
        assert_eq!(crate::commands::smart_original_text(&state, 7).unwrap(), "Send the summary the summary.");
        clear_review_text(&mut state);
        assert!(state.pre_model_text.is_none());
        assert!(crate::commands::smart_original_text(&state, 7).is_err());
        let (mut control, mut state) = active(8, 3, AppState::Cleaning);
        state.pre_model_text = Some("stale original".into());
        state.publication_stage = Some("smart_cleanup".into());
        state.guard_reason = Some("timeout".into());
        assert!(control.cancel((Some(8), 3), &mut state).unwrap());
        assert!(state.pre_model_text.is_none() && state.publication_stage.is_none() && state.guard_reason.is_none());
    }

    #[test]
    fn progress_never_reopens_stopped_capture_or_overwrites_a_new_session() {
        let (mut control, mut state) = active(7, 2, AppState::Finishing);
        let progress = crate::domain::RecognitionProgress {
            stage: "loading".into(),
            elapsed_ms: 94_000,
        };
        assert!(matches!(
            control.on_speech(
                &mut state,
                Event::Progress {
                    session_id: 6,
                    progress: progress.clone()
                }
            ),
            SpeechUpdate::Ignored
        ));
        assert!(state.recognition_progress.is_none());
        control.on_speech(
            &mut state,
            Event::Progress {
                session_id: 7,
                progress,
            },
        );
        assert_eq!(state.state, AppState::Finishing);
        assert_eq!(
            state.recognition_progress.as_ref().unwrap().elapsed_ms,
            94_000
        );
        control.on_speech(&mut state, Event::Cancelled { session_id: 7 });
        assert!(state.recognition_progress.is_none());
    }

    #[test]
    fn live_preview_draft_is_display_only_and_never_becomes_text() {
        let preview = |text: &str| Event::Preview { session_id: 7, text: text.into() };
        // Shown while loading, before the primary has produced anything.
        let (mut control, mut state) = active(7, 2, AppState::Loading);
        control.on_speech(&mut state, preview("early words"));
        assert_eq!(state.preview_text, "early words");
        assert_eq!(state.state, AppState::Loading, "a draft is not model readiness");
        // Committed text does not absorb the draft.
        control.on_speech(&mut state, Event::Interim { session_id: 7, text: "Committed.".into() });
        assert_eq!(state.interim_text, "Committed.");
        assert_eq!(state.preview_text, "early words");
        // A failure promotes committed text only, never the draft.
        control.on_speech(&mut state, Event::Error { session_id: 7, message: "worker exited".into() });
        assert!(state.preview_text.is_empty());
        assert_eq!(state.original_text, "Committed.");
        assert!(!state.output_text.contains("early"));
        // Late drafts after the session ends are ignored.
        control.on_speech(&mut state, preview("late"));
        assert!(state.preview_text.is_empty());

        let (mut control, mut state) = active(7, 2, AppState::Listening);
        control.on_speech(&mut state, preview("draft"));
        let SpeechUpdate::Publish(publication) =
            control.on_speech(&mut state, completed(7, "Raw.", "Output.", "complete"))
        else {
            panic!("publication missing");
        };
        assert_eq!(publication.text, "Output.");
        assert_eq!(publication.original_text, "Raw.");
        assert!(state.preview_text.is_empty());
    }

    #[test]
    fn microphone_started_is_not_model_ready() {
        let (mut control, mut state) = active(7, 2, AppState::Loading);
        control.on_speech(&mut state, Event::Started { session_id: 7 });
        assert_eq!(state.state, AppState::Loading);
        control.on_speech(
            &mut state,
            Event::Progress {
                session_id: 7,
                progress: crate::domain::RecognitionProgress {
                    stage: "loading".into(),
                    elapsed_ms: 24_000,
                },
            },
        );
        assert_eq!(state.state, AppState::Loading);
        control.on_speech(
            &mut state,
            Event::Progress {
                session_id: 7,
                progress: crate::domain::RecognitionProgress {
                    stage: "transcribing".into(),
                    elapsed_ms: 0,
                },
            },
        );
        assert_eq!(state.state, AppState::Listening);
    }

    fn completed_with_policy(
        id: u64,
        raw: &str,
        output: &str,
        status: &str,
        policy: &cleanup::CleanupPolicy,
        recognition_completeness: RecognitionCompleteness,
        recognition_warnings: Vec<String>,
    ) -> Event {
        Event::Completed {
            session_id: id,
            result: DictationResult {
                original_text: raw.into(),
                output_text: output.into(),
                pre_model_text: None,
                publication_stage: None,
                guard_reason: None,
                recognition_completeness,
                recognition_warnings,
                asr_model: "synthetic-boundary-test".into(),
                cleanup_status: status.into(),
                policy_version: policy.version.into(),
                policy_hash: policy.hash(),
                asr_ms: 0,
                cleanup_ms: 0,
            },
        }
    }

    fn all_events(id: u64) -> Vec<Event> {
        vec![
            Event::Started { session_id: id },
            Event::Level {
                session_id: id,
                rms: 0.9,
            },
            Event::Interim {
                session_id: id,
                text: "Late interim".into(),
            },
            Event::Finishing { session_id: id },
            Event::Cleaning {
                session_id: id,
                original_text: "Late raw".into(),
            },
            completed(id, "Late raw", "Late output", "late"),
            Event::Cancelled { session_id: id },
            Event::Error {
                session_id: id,
                message: "Late error".into(),
            },
        ]
    }

    fn assert_snapshot_unchanged(before: &Snapshot, after: &Snapshot) {
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        // capture_generation is intentionally skipped by serialization.
        assert_eq!(before.capture_generation, after.capture_generation);
    }

    #[test]
    fn deleted_review_rejects_pending_copy_and_drops_raw_and_finished_text() {
        let (mut control, mut state) = active(7, 4, AppState::Finishing);
        let update = control.on_speech(
            &mut state,
            completed(7, "private raw", "private final", "test"),
        );
        assert!(matches!(update, SpeechUpdate::Publish(_)));
        assert!(control.finish_publication(7, &mut state));
        state.can_copy_last = true;
        let session = state.result_session_id;
        // A delayed Cancel after completion cannot erase a finished result.
        let completed = state.clone();
        assert!(!control.cancel((Some(7), 4), &mut state).unwrap());
        assert_snapshot_unchanged(&completed, &state);
        assert!(crate::commands::current_copy_text(&state, false, session).is_ok());
        clear_review_text(&mut state);
        assert!(state.original_text.is_empty());
        assert!(state.output_text.is_empty());
        assert!(!state.can_copy_last);
        assert!(crate::commands::current_copy_text(&state, false, session).is_err());
        assert!(control.claim_manual_copy(&state, session).is_err());
        assert!(control.can_start(4, &AtomicU64::new(4)).unwrap());
    }

    #[test]
    fn cancellation_before_claim_prevents_delivery_in_every_capture_phase() {
        executor().block_on(async {
            for phase in [
                AppState::Loading,
                AppState::Listening,
                AppState::Finishing,
                AppState::Cleaning,
            ] {
                let (mut control, mut state) = active(7, 4, phase);
                let invalidation = AtomicU64::new(4);
                let ticket = capture_ticket(&state, &invalidation);
                assert!(control.cancel(ticket, &mut state).unwrap());
                assert_eq!(state.state, AppState::Idle);
                assert_eq!(state.session_id, None);
                assert_eq!(state.capture_generation, None);
                assert!(state.interim_text.is_empty());
                assert!(state.original_text.is_empty());
                assert!(state.output_text.is_empty());
                assert_eq!(state.result_session_id, None);
                assert_eq!(state.audio_level, 0.0);
                let cancelled = state.clone();
                let sink = FakeDelivery::default();
                for event in all_events(7) {
                    let update = control.on_speech(&mut state, event);
                    // Dispatch any erroneous claim to the fake sink, so the
                    // assertion measures delivery as well as snapshot ownership.
                    if let SpeechUpdate::Publish(mut publication) = update {
                        publication
                            .deliver_with(|text, target, auto| sink.deliver(text, target, auto))
                            .await
                            .unwrap()
                            .unwrap();
                    } else {
                        assert!(matches!(update, SpeechUpdate::Ignored));
                    }
                }
                assert!(sink.calls.lock().unwrap().is_empty());
                assert_snapshot_unchanged(&cancelled, &state);
                assert!(!control.can_start(4, &invalidation).unwrap());
                assert!(control.can_start(5, &invalidation).unwrap());
            }
        });
    }

    #[test]
    fn new_session_rejects_old_events_commands_and_finalizers() {
        let (mut control, mut state) = active(7, 4, AppState::Cleaning);
        assert!(control.cancel((Some(7), 4), &mut state).unwrap());
        let invalidation = AtomicU64::new(5);
        assert!(control.can_start(5, &invalidation).unwrap());
        // Capture itself is outside this test; install its returned session ID.
        (control, state) = active(8, 5, AppState::Listening);
        let current = state.clone();
        for event in all_events(7) {
            assert!(matches!(
                control.on_speech(&mut state, event),
                SpeechUpdate::Ignored
            ));
        }
        for stale_ticket in [(Some(7), 4), (None, 4)] {
            assert!(!control.cancel(stale_ticket, &mut state).unwrap());
            assert!(!control.can_stop_capture(stale_ticket, state.state));
            assert!(!control.can_stop_cleanup(stale_ticket, AppState::Cleaning));
        }
        assert!(!control.finish_publication(7, &mut state));
        assert_snapshot_unchanged(&current, &state);
        assert_eq!(control.session, Some(8));
        assert!(!control.publishing);
        assert!(matches!(
            control.on_speech(
                &mut state,
                completed(8, "New raw", "New output", "accepted")
            ),
            SpeechUpdate::Publish(_)
        ));
        assert_eq!(state.output_text, "New output");
    }

    #[test]
    fn pending_delivery_owns_session_despite_duplicate_completion_and_late_events() {
        executor().block_on(async {
            let (mut control, mut state) = active(7, 4, AppState::Cleaning);
            let SpeechUpdate::Publish(mut publication) = control.on_speech(
                &mut state,
                completed(7, "Raw source", "Final output", "accepted"),
            ) else {
                panic!("Current completion must claim publication");
            };
            let claimed = state.clone();
            let sink = Arc::new(FakeDelivery::default());
            let worker_sink = sink.clone();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let delivery = tokio::spawn(async move {
                let delivered = publication
                    .deliver_with(|text, target, auto| async move {
                        entered_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        worker_sink.deliver(text, target, auto).await
                    })
                    .await;
                (publication, delivered)
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), entered_rx)
                .await
                .unwrap()
                .unwrap();
            // The claim wins even before the sink writes. Cancellation does not
            // promise to undo an OS operation once publication has been claimed.
            assert!(sink.calls.lock().unwrap().is_empty());
            let invalidation = AtomicU64::new(4);
            let ticket = capture_ticket(&state, &invalidation);
            assert_eq!(
                control.cancel(ticket, &mut state),
                Err("The final text is already being copied.".into())
            );
            assert!(control.can_start(5, &invalidation).is_err());
            assert!(!control.can_stop_capture(ticket, state.state));
            assert!(!control.can_stop_cleanup(ticket, AppState::Cleaning));
            for id in [7, 6] {
                for event in all_events(id) {
                    assert!(matches!(
                        control.on_speech(&mut state, event),
                        SpeechUpdate::Ignored
                    ));
                }
            }
            assert!(!control.finish_publication(6, &mut state));
            assert_snapshot_unchanged(&claimed, &state);
            assert_eq!(control.session, Some(7));
            assert!(control.publishing);
            release_tx.send(()).unwrap();
            let (publication, delivered) =
                tokio::time::timeout(std::time::Duration::from_secs(5), delivery)
                    .await
                    .unwrap()
                    .unwrap();
            assert!(delivered.unwrap().is_ok());
            assert_eq!(
                *sink.calls.lock().unwrap(),
                vec![("Final output".into(), true)]
            );
            // Delivery returning alone does not release the owner.
            assert!(control.can_start(5, &invalidation).is_err());
            assert!(control.finish_publication(publication.session_id, &mut state));
            assert!(!control.finish_publication(publication.session_id, &mut state));
            assert_eq!(state.state, AppState::Idle);
            assert_eq!(state.session_id, None);
            assert_eq!(state.capture_generation, None);
            assert_eq!(state.original_text, "Raw source");
            assert_eq!(state.output_text, "Final output");
            assert!(control.can_start(5, &invalidation).unwrap());
            for event in all_events(7) {
                assert!(matches!(
                    control.on_speech(&mut state, event),
                    SpeechUpdate::Ignored
                ));
            }
            assert_eq!(sink.calls.lock().unwrap().len(), 1);
        });
    }

    #[test]
    fn stop_keeps_completion_eligible_and_delivers_full_fallback_tail() {
        executor().block_on(async {
            let raw = format!(
                "  raw heading\r\n{}Final tail: café, 保留, version 1.10.\r\n  ",
                "Keep every source segment. ".repeat(600)
            );
            let output = raw.replacen("raw heading", "Raw heading.", 1);
            // Synthetic Completed results test boundary preservation, not whether
            // the cleanup engine produced a complete fallback after a real Stop.
            for phase in [AppState::Listening, AppState::Cleaning] {
                let (mut control, mut state) = active(7, 4, phase);
                state.settings.auto_paste = false;
                let invalidation = AtomicU64::new(4);
                let ticket = capture_ticket(&state, &invalidation);
                assert_eq!(
                    control.can_stop_capture(ticket, phase),
                    phase == AppState::Listening
                );
                assert_eq!(
                    control.can_stop_cleanup(ticket, phase),
                    phase == AppState::Cleaning
                );
                assert!(!control.can_stop((Some(6), 3)));
                assert!(control.can_stop(ticket)); // repeated Stop retains ownership
                assert!(control.can_start(5, &invalidation).is_err());
                let SpeechUpdate::Publish(mut publication) =
                    control.on_speech(&mut state, completed(7, &raw, &output, "stopped"))
                else {
                    panic!("Stop must allow the current full completion to publish");
                };
                assert_eq!(publication.original_text, raw);
                assert_eq!(state.original_text, raw);
                assert_eq!(state.output_text, output);
                let sink = FakeDelivery::default();
                publication
                    .deliver_with(|text, target, auto| sink.deliver(text, target, auto))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(*sink.calls.lock().unwrap(), vec![(output.clone(), false)]);
                assert!(control.finish_publication(7, &mut state));
            }
        });
    }

    #[test]
    fn delivery_failure_finalizes_without_republishing_or_losing_source_text() {
        executor().block_on(async {
            let (mut control, mut state) = active(7, 4, AppState::Cleaning);
            let SpeechUpdate::Publish(mut publication) = control.on_speech(
                &mut state,
                completed(7, "Complete raw tail", "Complete output tail", "fallback"),
            ) else {
                panic!("Current completion must claim publication");
            };
            let mut attempts = 0;
            let delivered = publication
                .deliver_with(|text, _, _| {
                    attempts += 1;
                    assert_eq!(text, "Complete output tail");
                    std::future::ready(Err("Injected delivery failure".into()))
                })
                .await;
            assert_eq!(delivered.unwrap().unwrap_err(), "Injected delivery failure");
            assert!(control.finish_publication(7, &mut state));
            assert_eq!(state.original_text, "Complete raw tail");
            assert_eq!(state.output_text, "Complete output tail");
            assert_eq!(publication.original_text, state.original_text);
            assert!(control.can_start(4, &AtomicU64::new(4)).unwrap());
            assert!(matches!(
                control.on_speech(
                    &mut state,
                    completed(7, "Retry raw", "Retry output", "late")
                ),
                SpeechUpdate::Ignored
            ));
            assert_eq!(attempts, 1);
        });
    }

    #[test]
    fn empty_output_skips_sink_and_still_releases_publication() {
        executor().block_on(async {
            for output in ["", " \r\n\t"] {
                let (mut control, mut state) = active(7, 4, AppState::Cleaning);
                let SpeechUpdate::Publish(mut publication) = control.on_speech(
                    &mut state,
                    completed(7, "Raw remains available", output, "fallback"),
                ) else {
                    panic!("Empty completion must still own finalization");
                };
                let sink = FakeDelivery::default();
                assert!(publication
                    .deliver_with(|text, target, auto| sink.deliver(text, target, auto))
                    .await
                    .is_none());
                assert!(sink.calls.lock().unwrap().is_empty());
                assert_eq!(publication.original_text, "Raw remains available");
                assert!(control.finish_publication(7, &mut state));
                assert_eq!(state.state, AppState::Idle);
                assert!(!control.publishing);
            }
        });
    }

    #[test]
    fn trial_completion_publishes_once_and_delivery_log_records_policy() {
        executor().block_on(async {
            let (mut control, mut state) = active(7, 4, AppState::Cleaning);
            let SpeechUpdate::Publish(mut publication) = control.on_speech(
                &mut state,
                completed_with_policy(
                    7,
                    "Email me at alex at example dot com.",
                    "Email me at alex@example.com.",
                    "accepted",
                    &cleanup::CleanupPolicy::trial(),
                    RecognitionCompleteness::Complete,
                    Vec::new(),
                ),
            ) else {
                panic!("Current completion must claim publication");
            };
            assert_eq!(publication.policy_version, "trial-v4");
            assert_eq!(
                publication.policy_hash,
                cleanup::CleanupPolicy::trial().hash()
            );
            let sink = FakeDelivery::default();
            let outcome = publication
                .deliver_with(|text, target, auto| sink.deliver(text, target, auto))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(sink.calls.lock().unwrap().len(), 1);
            let detail = Runtime::delivery_log_detail(&outcome.diagnostics, &publication);
            assert!(detail.contains("cleanupPolicyVersion=trial-v4"));
            assert!(detail.contains(&format!(
                "cleanupPolicyHash={}",
                cleanup::CleanupPolicy::trial().hash()
            )));
        });
    }

    #[test]
    fn incomplete_nonempty_result_blocks_every_automatic_delivery_route() {
        executor().block_on(async {
            let text = "Keep the first instruction. <thö Do not remove the backup. Final tail: café, 保留.";
            for cleanup_mode in [
                CleanupMode::Original,
                CleanupMode::Format,
                CleanupMode::Clean,
            ] {
                for cleanup_trial in [false, true] {
                    for auto_paste in [false, true] {
                        let (mut control, mut state) = active(7, 4, AppState::Finishing);
                        state.settings.cleanup_mode = cleanup_mode;
                        state.settings.cleanup_trial = cleanup_trial;
                        state.settings.auto_paste = auto_paste;
                        let policy = if cleanup_trial {
                            cleanup::CleanupPolicy::trial()
                        } else {
                            cleanup::CleanupPolicy::legacy()
                        };
                        let SpeechUpdate::Publish(mut publication) = control.on_speech(
                            &mut state,
                            completed_with_policy(
                                7,
                                text,
                                text,
                                "recognition_review_required",
                                &policy,
                                RecognitionCompleteness::Incomplete,
                                vec!["segment two remained ambiguous after one retry".into()],
                            ),
                        ) else {
                            panic!("Current completion must claim publication");
                        };
                        assert_eq!(state.result_session_id, Some(7));
                        assert_eq!(
                            state.recognition_completeness,
                            Some(RecognitionCompleteness::Incomplete)
                        );
                        assert_eq!(state.original_text, text);
                        assert_eq!(state.output_text, text);
                        assert_eq!(
                            state.delivery_notice.as_deref(),
                            Some(RECOGNITION_WARNING_NOTICE)
                        );
                        let sink = FakeDelivery::default();
                        assert!(publication
                            .deliver_with(|value, target, auto| {
                                sink.deliver(value, target, auto)
                            })
                            .await
                            .is_none());
                        assert!(sink.calls.lock().unwrap().is_empty());
                        let detail = Runtime::delivery_log_detail(
                            "delivery_blocked_incomplete",
                            &publication,
                        );
                        assert!(detail.contains("recognitionCompleteness=incomplete"));
                        assert!(detail.contains("recognitionWarnings=1"));
                        assert!(crate::commands::current_copy_text(&state, false, Some(7)).is_err());
                        assert!(control.finish_publication(7, &mut state));
                        assert_eq!(
                            crate::commands::current_copy_text(&state, false, Some(7)).unwrap(),
                            text
                        );
                        assert!(crate::commands::current_copy_text(&state, false, Some(6)).is_err());
                        state.result_session_id = None;
                        state.state = AppState::Listening;
                        assert!(crate::commands::current_copy_text(&state, false, Some(7)).is_err());
                    }
                }
            }
        });
    }

    #[test]
    fn incomplete_empty_result_never_calls_the_delivery_sink_or_manual_copy() {
        executor().block_on(async {
            let (mut control, mut state) = active(7, 4, AppState::Finishing);
            let SpeechUpdate::Publish(mut publication) = control.on_speech(
                &mut state,
                completed_with_policy(
                    7,
                    "",
                    "",
                    "recognition_review_required",
                    &cleanup::CleanupPolicy::legacy(),
                    RecognitionCompleteness::Incomplete,
                    vec!["recognition returned no reviewable text after one retry".into()],
                ),
            ) else {
                panic!("Current completion must claim publication");
            };
            assert_eq!(
                state.recognition_completeness,
                Some(RecognitionCompleteness::Incomplete)
            );
            assert_eq!(state.result_session_id, Some(7));
            let sink = FakeDelivery::default();
            assert!(publication
                .deliver_with(|text, target, auto| sink.deliver(text, target, auto))
                .await
                .is_none());
            assert!(sink.calls.lock().unwrap().is_empty());
            assert!(control.finish_publication(7, &mut state));
            assert!(crate::commands::current_copy_text(&state, false, Some(7)).is_err());
        });
    }

    #[test]
    fn reviewed_copy_claim_blocks_overtaking_capture_and_rejects_stale_session() {
        let _gate = crate::resource_manager::tests::test_gate();
        let (mut control, mut state) = active(7, 4, AppState::Idle);
        control.session = None;
        state.result_session_id = Some(7);
        assert!(control.claim_manual_copy(&state, Some(6)).is_err());
        assert!(!control.publishing);
        let control = Mutex::new(control);
        control
            .lock()
            .unwrap()
            .claim_manual_copy(&state, Some(7))
            .unwrap();
        let claim = ManualCopyClaim {
            control: &control,
            _update_activity: crate::resource_manager::begin_update_activity(
                crate::resource_manager::UpdateActivity::Dictation,
            ).unwrap(),
        };
        let generation = AtomicU64::new(4);
        assert!(control.lock().unwrap().can_start(4, &generation).is_err());
        assert!(control
            .lock()
            .unwrap()
            .cancel((None, 4), &mut state)
            .is_err());
        drop(claim);
        assert_eq!(
            control.lock().unwrap().can_start(4, &generation).unwrap(),
            true
        );
        state.result_session_id = Some(8);
        assert!(control
            .lock()
            .unwrap()
            .claim_manual_copy(&state, Some(7))
            .is_err());
        assert!(!control.lock().unwrap().publishing);
    }

    #[test]
    fn update_gate_covers_capture_publication_and_cancelled_worker_drain() {
        use crate::resource_manager::{begin_update_activity, begin_update_install, UpdateActivity};
        let _gate = crate::resource_manager::tests::test_gate();
        crate::resource_manager::reset_for_tests(MemoryMode::Overlap, ModelRetention::OnExit);
        for phase in [AppState::Loading, AppState::Listening, AppState::Finishing, AppState::Cleaning] {
            let (mut control, mut state) = active(7, 4, phase);
            control.update_activity = Some(begin_update_activity(UpdateActivity::Dictation).unwrap());
            assert!(begin_update_install().is_err());
            assert!(matches!(control.on_speech(&mut state, completed(7, "raw", "published", "rules")),
                SpeechUpdate::Publish(_)));
            assert!(begin_update_install().is_err());
            assert!(control.finish_publication(7, &mut state));
            assert!(begin_update_install().is_ok());

            let (mut control, mut state) = active(8, 5, phase);
            control.update_activity = Some(begin_update_activity(UpdateActivity::Dictation).unwrap());
            let worker = begin_update_activity(UpdateActivity::Dictation).unwrap();
            assert!(control.cancel((Some(8), 5), &mut state).unwrap());
            // Visible Idle after cancellation is insufficient while the
            // speech worker still drains capture/audio or releases inference.
            assert_eq!(state.state, AppState::Idle);
            assert!(begin_update_install().is_err());
            drop(worker);
            assert!(begin_update_install().is_ok());
        }
    }
}

#[cfg(test)]
mod command_order_tests {
    use super::*;
    #[test]
    fn cleanup_hotkey_stops_cleanup_without_starting_or_discarding_capture() {
        for mode in [HotkeyMode::Toggle, HotkeyMode::Hold] {
            assert_eq!(
                shortcut_action(AppState::Cleaning, mode, ShortcutState::Pressed),
                3
            );
            assert_eq!(
                shortcut_action(AppState::Cleaning, mode, ShortcutState::Released),
                0
            );
        }
        assert_eq!(
            shortcut_action(
                AppState::Finishing,
                HotkeyMode::Toggle,
                ShortcutState::Pressed
            ),
            0
        );
        assert_eq!(
            shortcut_action(
                AppState::Listening,
                HotkeyMode::Toggle,
                ShortcutState::Pressed
            ),
            2
        );
    }
    #[test]
    fn old_stop_cannot_finish_a_new_session() {
        assert!(!stop_matches((Some(1), 0), Some(2), 1));
        assert!(stop_matches((Some(1), 0), Some(1), 0));
    }
    #[test]
    fn stop_during_capture_initialization_only_matches_that_generation() {
        assert!(stop_matches((None, 4), Some(8), 4));
        assert!(!stop_matches((None, 4), Some(9), 5));
        assert!(!stop_matches((None, 4), None, 4));
    }
    #[test]
    fn repeated_stop_then_cancel_keep_the_pending_start_identity() {
        let invalidation = AtomicU64::new(4);
        let snapshot = Snapshot {
            capture_generation: Some(4),
            state: AppState::Loading,
            ..Default::default()
        };
        let stop = capture_ticket(&snapshot, &invalidation);
        let cancel = capture_ticket(&snapshot, &invalidation);
        assert_eq!(stop, (None, 4));
        assert_eq!(cancel, stop);
        assert_eq!(invalidation.load(Ordering::Acquire), 6);
        assert!(stop_matches(cancel, Some(8), 4));
        assert!(!stop_matches(cancel, Some(9), 6));
    }
}
