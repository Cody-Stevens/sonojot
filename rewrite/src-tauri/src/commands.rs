use crate::{
    app::{lock, model_catalog, register_shortcut, Runtime},
    audio, cleanup,
    domain::*,
    platform,
    recording::{MeetingRecorder, RecordingPaths},
    windows,
};
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

type AppStateHandle<'a> = State<'a, Arc<Runtime>>;

#[cfg(test)]
mod smart_recovery_tests {
    use super::*;

    #[test]
    fn original_recovery_requires_the_latest_changed_complete_idle_result() {
        let good = Snapshot { result_session_id: Some(7),
            original_text: "raw asr".into(), output_text: "Send the summary.".into(),
            pre_model_text: Some("Send the summary the summary.".into()),
            publication_stage: Some("smart_cleanup".into()),
            recognition_completeness: Some(RecognitionCompleteness::Complete),
            ..Snapshot::default() };
        assert_eq!(smart_original_text(&good, 7).unwrap(), "Send the summary the summary.");
        let shaped = Snapshot { publication_stage: Some("smart_layout".into()), ..good.clone() };
        assert_eq!(smart_original_text(&shaped, 7).unwrap(), "Send the summary the summary.");
        assert!(smart_original_text(&good, 6).is_err());
        for bad in [
            Snapshot { result_session_id: None, ..good.clone() },
            Snapshot { session_id: Some(8), ..good.clone() },
            Snapshot { meeting_recording: true, ..good.clone() },
            Snapshot { state: AppState::Listening, ..good.clone() },
            Snapshot { state: AppState::Finishing, ..good.clone() },
            Snapshot { state: AppState::Cleaning, ..good.clone() },
            Snapshot { publication_stage: Some("rules".into()), ..good.clone() },
            Snapshot { publication_stage: Some("edited".into()), ..good.clone() },
            Snapshot { pre_model_text: None, ..good.clone() },
            Snapshot { pre_model_text: Some(good.output_text.clone()), ..good.clone() },
            Snapshot { recognition_completeness: Some(RecognitionCompleteness::Incomplete), ..good },
        ] {
            assert!(smart_original_text(&bad, 7).is_err());
        }
    }
}

#[cfg(test)]
mod setup_gate_tests {
    use crate::domain::{Settings, SetupProfile};
    use crate::setup_commands::{require_setup, CaptureKind};

    fn accepted_settings() -> Settings {
        Settings { terms_accepted_version: crate::onboarding::TERMS_VERSION,
            terms_accepted_at: Some("2026-10-12T12:00:00Z".into()), ..Settings::default() }
    }

    #[test]
    fn unfinished_setup_refuses_both_capture_kinds_with_actionable_code() {
        let settings = accepted_settings();
        assert_eq!(require_setup(&settings, CaptureKind::Dictation).unwrap_err(),
            "SETUP_INCOMPLETE: Finish setup to start dictating.");
        assert_eq!(require_setup(&settings, CaptureKind::Meeting).unwrap_err(),
            "SETUP_INCOMPLETE: Finish setup to record meetings.");
    }

    #[test]
    fn each_completed_profile_passes_the_setup_gate() {
        for profile in [SetupProfile::Accurate, SetupProfile::Balanced,
            SetupProfile::Light, SetupProfile::Custom] {
            let settings = Settings { setup_profile: Some(profile), ..accepted_settings() };
            assert!(require_setup(&settings, CaptureKind::Dictation).is_ok());
            assert!(require_setup(&settings, CaptureKind::Meeting).is_ok());
        }
    }

    #[test]
    fn microphone_test_remains_available_before_setup() {
        assert!(require_setup(&accepted_settings(), CaptureKind::MicrophoneTest).is_ok());
    }
}

#[tauri::command]
pub async fn get_model_acceleration(
    state: AppStateHandle<'_>,
) -> Result<crate::acceleration::AccelerationStatus, String> {
    let root = state.models_root.clone();
    tauri::async_runtime::spawn_blocking(move || crate::acceleration::inspect(&root))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_snapshot(state: AppStateHandle<'_>) -> Result<Snapshot, String> {
    let root = state.models_root.clone();
    let (devices, models) =
        tauri::async_runtime::spawn_blocking(move || (audio::list_devices(), model_catalog(&root)))
            .await
            .map_err(|e| e.to_string())?;
    let mut snapshot = lock(&state.snapshot)?;
    snapshot.models = models;
    match devices {
        Ok(devices) => {
            snapshot.devices = devices
                .into_iter()
                .map(|d| Device {
                    id: d.id,
                    name: d.name,
                    is_default: d.is_default,
                })
                .collect()
        }
        Err(error) => {
            snapshot.devices.clear();
            snapshot.error = Some(error);
        }
    }
    drop(snapshot);
    state.snapshot()
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: AppStateHandle<'_>,
    settings: Settings,
) -> Result<Snapshot, String> {
    let _gate = state.settings_gate.lock().await;
    settings.validate()?;
    let root = state.models_root.clone();
    let models = tauri::async_runtime::spawn_blocking(move || model_catalog(&root))
        .await
        .map_err(|e| e.to_string())?;
    // Serialize the active-state check and settings commit with capture starts.
    let control = lock(&state.control)?;
    let old = state.snapshot()?.settings;
    if !matches!(state.snapshot()?.state, AppState::Idle | AppState::Error)
        || state.snapshot()?.meeting_recording
    {
        let mut visual_only = old.clone();
        visual_only.theme = settings.theme;
        visual_only.material = settings.material;
        visual_only.start_tab_hidden = settings.start_tab_hidden;
        visual_only.cursor_indicator = settings.cursor_indicator;
        if settings != visual_only {
            return Err("Finish recording before changing dictation settings.".into());
        }
    }
    if !models
        .iter()
        .any(|m| m.id == settings.asr_model && m.kind == ModelKind::Asr)
        || !models
            .iter()
            .any(|m| m.id == settings.cleanup_model && m.kind == ModelKind::Cleanup)
        || !((settings.notes_model == "openrouter"
            && crate::ai::load_settings(&state.store).is_ok_and(|ai| {
                ai.backend == crate::ai::AiBackend::OpenRouter && !ai.model.is_empty()
            }))
            || models.iter().any(|m| {
                m.id == settings.notes_model
                    && m.kind == ModelKind::Cleanup
                    && m.id != "local-rules"
            }))
    {
        return Err("Choose an installed local model or configure OpenRouter before selecting remote Notes.".into());
    }
    let shortcut_changed = old.hotkey != settings.hotkey;
    // Model identity plus the text-device preference decide which native
    // weights load: a textAcceleration flip mid-job would switch CPU/GPU
    // under later chunks (server_matches compares against this preference),
    // so it joins the guarded selection commit instead of slipping through
    // as a visual-only change.
    let ids_changed = settings.asr_model != old.asr_model
        || settings.meeting_asr_model != old.meeting_asr_model
        || settings.cleanup_model != old.cleanup_model
        || settings.notes_model != old.notes_model
        || settings.text_acceleration != old.text_acceleration;
    // Serialize selection with installs/removes/loads (runtime-last-fix
    // hook). The commit runs inside the shared selection + coordinator
    // critical section, so it must not call resource_manager/model_manager
    // locking APIs; `configure` runs after unlock below.
    let commit = || {
        // A missing package may remain in old settings after external removal,
        // but a new selection must be ready. Recheck inside the mutation guard
        // so a concurrent uninstall cannot invalidate the earlier catalog read.
        if settings.meeting_asr_model != old.meeting_asr_model
            && !settings.meeting_asr_model.is_empty()
        {
            if !crate::granite_model::supported() {
                return Err(crate::granite_model::unavailable_reason()
                    .unwrap_or("Granite is unavailable on this computer.").into());
            }
            if !crate::granite_model::installed(&state.models_root) {
                return Err("Install Granite Speech before selecting it for meetings.".into());
            }
        }
        if settings.asr_model != old.asr_model
            && !crate::asr::model_ready(&state.models_root, &settings.asr_model)
        {
            return Err("Install the speech model before selecting it.".into());
        }
        for (before, after) in [
            (&old.cleanup_model, &settings.cleanup_model),
            (&old.notes_model, &settings.notes_model),
        ] {
            if before != after
                && after != "local-rules"
                && after != "openrouter"
                && !crate::cleanup::model_ready(&state.models_root, after)
            {
                return Err("Install the text model before selecting it.".into());
            }
        }
        if shortcut_changed {
            register_shortcut(&app, &settings.hotkey)?;
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        if old.launch_at_login != settings.launch_at_login {
            if let Err(error) = platform::set_startup_enabled(settings.launch_at_login, &executable)
            {
                if shortcut_changed {
                    let _ = app
                        .global_shortcut()
                        .unregister(settings.hotkey.replace(' ', "").as_str());
                }
                return Err(error);
            }
        }
        if let Err(error) = state.store.save_settings(&settings) {
            if old.launch_at_login != settings.launch_at_login {
                let _ = platform::set_startup_enabled(old.launch_at_login, &executable);
            }
            if shortcut_changed {
                let _ = app
                    .global_shortcut()
                    .unregister(settings.hotkey.replace(' ', "").as_str());
            }
            return Err(error);
        }
        if shortcut_changed {
            let _ = app
                .global_shortcut()
                .unregister(old.hotkey.replace(' ', "").as_str());
        }
        {
            let mut snapshot = lock(&state.snapshot)?;
            crate::acceleration::set_preference(settings.text_acceleration);
            snapshot.settings = settings.clone();
            snapshot.error = None;
            if shortcut_changed {
                snapshot.capabilities.global_shortcut = true;
            }
        }
        Ok(())
    };
    crate::resource_manager::with_model_selection(ids_changed, commit)?;
    // Keep the shared resource coordinator on the current overlap/retention
    // choices without a restart. Idempotent; the next lease acquire observes
    // the new mode. Startup path already configures in lib.rs setup. Runs
    // after the selection guard is released (it takes the coordinator lock).
    crate::resource_manager::configure(&settings);
    if crate::onboarding::terms_accepted(&settings)
        && settings.memory_mode == crate::domain::MemoryMode::Overlap
        && settings.model_retention != crate::domain::ModelRetention::Immediately
    {
        let prewarm = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || prewarm.prewarm_selected_models());
    }
    drop(control);
    if !crate::onboarding::terms_accepted(&old) && crate::onboarding::terms_accepted(&settings) {
        crate::onboarding::resume_downloads(&app, state.inner());
    }
    if !state.snapshot()?.settings.cursor_indicator {
        windows::close_cursor(&app);
    }
    state.emit(&app);
    state.snapshot()
}

#[tauri::command]
pub async fn start_dictation(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let generation = runtime.command_generation.load(Ordering::Acquire);
    let target = platform::capture_paste_target();
    tauri::async_runtime::spawn_blocking(move || {
        runtime.start_at_generation(&app, target, generation)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn test_microphone(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let generation = runtime.command_generation.load(Ordering::Acquire);
    tauri::async_runtime::spawn_blocking(move || {
        runtime.start_microphone_test_at_generation(&app, generation)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn stop_dictation(app: AppHandle, state: AppStateHandle<'_>) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let ticket = runtime.stop_ticket()?;
    tauri::async_runtime::spawn_blocking(move || runtime.stop_at(&app, ticket))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn stop_cleanup(state: AppStateHandle<'_>) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let ticket = runtime.stop_ticket()?;
    tauri::async_runtime::spawn_blocking(move || runtime.stop_cleanup_at(ticket))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn cancel_dictation(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let ticket = runtime.stop_ticket()?;
    tauri::async_runtime::spawn_blocking(move || runtime.cancel_at(&app, ticket))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_screen_tab(
    app: AppHandle,
    state: AppStateHandle<'_>,
    visible: bool,
    expanded: Option<bool>,
    focus: Option<bool>,
    pinned: Option<bool>,
) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // `pinned` is the explicit Keep/restore intent; it pins without focus.
        // A focus-taking activation (keyboard expand) still pins on its own.
        let focus = focus.unwrap_or(false);
        runtime.apply_tab(
            &app,
            visible,
            expanded,
            focus,
            focus || pinned.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn screen_tab_record(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<Snapshot, String> {
    let runtime = state.inner().clone();
    let action = runtime.snapshot()?.state;
    let generation = runtime.command_generation.load(Ordering::Acquire);
    let ticket = if matches!(action, AppState::Listening | AppState::Loading) {
        Some(runtime.stop_ticket()?)
    } else {
        None
    };
    let target = if windows::screen_tab_has_focus(&app) {
        lock(&runtime.tab_target)?.clone()
    } else {
        platform::capture_paste_target()
    };
    tauri::async_runtime::spawn_blocking(move || match action {
        AppState::Listening | AppState::Loading => {
            runtime.stop_at(&app, ticket.ok_or("Recording state changed.")?)
        }
        AppState::Finishing | AppState::Cleaning => runtime.snapshot(),
        _ => runtime.start_at_generation(&app, target, generation),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn copy_last_dictation(state: AppStateHandle<'_>) -> Result<(), String> {
    let text = lock(&state.last_published)?.clone();
    if text.is_empty() {
        return Err("Finish a dictation before copying its text.".into());
    }
    platform::copy_text_after_claim(&text, || state.claim_last_copy(&text)).await
}

pub(crate) fn smart_original_text(snapshot: &Snapshot, session: u64) -> Result<String, String> {
    if snapshot.result_session_id != Some(session)
        || snapshot.session_id.is_some()
        || snapshot.meeting_recording
        || !matches!(snapshot.state, AppState::Idle | AppState::Error)
        || snapshot.recognition_completeness != Some(RecognitionCompleteness::Complete)
        || !matches!(snapshot.publication_stage.as_deref(), Some("smart_cleanup" | "smart_layout"))
    {
        return Err("This cleaned dictation is no longer current. Review it in Recent dictations.".into());
    }
    snapshot.pre_model_text.as_ref()
        .filter(|text| !text.trim().is_empty() && **text != snapshot.output_text)
        .cloned()
        .ok_or_else(|| "There is no original cleanup text to copy.".into())
}

/// Recovery only writes the clipboard; it never targets an editor or sends a key.
#[tauri::command]
pub async fn copy_smart_original(state: AppStateHandle<'_>, result_session_id: u64) -> Result<(), String> {
    let text = smart_original_text(&state.snapshot()?, result_session_id)?;
    platform::copy_text_after_claim(&text, || {
        let claim = state.claim_manual_copy(Some(result_session_id))?;
        // Recheck after clipboard availability awaits and while capture/edit is blocked.
        if smart_original_text(&state.snapshot()?, result_session_id)? != text {
            return Err("The original cleanup text changed. Review it before copying.".into());
        }
        Ok(claim)
    }).await
}

#[tauri::command]
pub async fn show_page(app: AppHandle, page: String) -> Result<(), String> {
    if !matches!(
        page.as_str(),
        "dictation" | "meetings" | "setup" | "appearance" | "microphone" | "shortcuts"
    ) {
        return Err("Choose a settings page.".into());
    }
    let handle = app.clone();
    let requested = page.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // The query selects the page when the main window is newly created.
        windows::show_settings_page(&handle, &requested)?;
        if let Some(window) = handle.get_webview_window("main") {
            window
                .emit("navigate-page", requested)
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn screen_tab_meeting(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<MeetingDetail, String> {
    crate::setup_commands::require_setup(&state.snapshot()?.settings, crate::setup_commands::CaptureKind::Meeting)?;
    let mut source = state.snapshot()?.settings.meeting_source;
    if source != "microphone" && !state.snapshot()?.capabilities.system_audio {
        return Err("This meeting source is unavailable. Choose microphone in Meetings.".into());
    }
    if !state.snapshot()?.settings.meeting_consent_acknowledged {
        // A native dialog fits both messages without changing the compact tab's
        // input region. The macOS panel and Windows tray use this same command.
        static CONSENT_DIALOG: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _dialog = CONSENT_DIALOG.try_lock().map_err(|_| MEETING_START_CANCELLED)?;
        let copy = meeting_consent_copy()?;
        let warning = match source.as_str() {
            "microphone" => Some(&copy.microphone),
            "system" => Some(&copy.system),
            _ => None,
        };
        let detail = match warning {
            Some(warning) => format!("{}\n\n{warning}", copy.reminder),
            None => copy.reminder.clone(),
        };
        let buttons = if warning.is_some() {
            MessageDialogButtons::YesNoCancelCustom(
                copy.got_it_record_both.clone(), copy.got_it_start_anyway.clone(), copy.cancel.clone(),
            )
        } else {
            MessageDialogButtons::OkCancelCustom(copy.got_it.clone(), copy.cancel.clone())
        };
        let generation = state.command_generation.load(Ordering::Acquire);
        let runtime = state.inner().clone();
        let handle = app.clone();
        let title = copy.title.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            if !tab_meeting_request_current(&runtime, generation)? {
                return Err(MEETING_START_CANCELLED.into());
            }
            // Close the passive panel before opening an independent dialog:
            // macOS's outside-click watcher must not dismiss its dialog parent.
            runtime.set_tab(&handle, false, Some(false), false)?;
            let result = handle.dialog().message(detail).title(title)
                .buttons(buttons).blocking_show_with_result();
            if tab_meeting_request_current(&runtime, generation)? {
                runtime.set_tab(&handle, true, Some(true), false)?;
            }
            Ok::<_, String>(result)
        }).await.map_err(|error| error.to_string())??;
        let chosen = consent_meeting_source(&copy, &source, &result)
            .ok_or(MEETING_START_CANCELLED)?.to_owned();
        if !tab_meeting_request_current(&state, generation)? {
            return Err(MEETING_START_CANCELLED.into());
        }
        let mut settings = state.snapshot()?.settings;
        // A source changed in another window has not been reviewed in this prompt.
        saved_meeting_source(&settings, &source)?;
        settings.meeting_consent_acknowledged = true;
        settings.meeting_source = chosen.clone();
        save_settings(app.clone(), state.clone(), settings).await?;
        // A shortcut can arrive while settings are being saved. Never start later.
        if !tab_meeting_request_current(&state, generation)? {
            return Err(MEETING_START_CANCELLED.into());
        }
        source = chosen;
    }
    start_meeting(app, state, String::new(), source).await
}

const MEETING_START_CANCELLED: &str = "MEETING_START_CANCELLED";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeetingConsentCopy {
    reminder: String,
    microphone: String,
    system: String,
    title: String,
    got_it: String,
    got_it_record_both: String,
    got_it_start_anyway: String,
    cancel: String,
}

fn meeting_consent_copy() -> Result<MeetingConsentCopy, String> {
    serde_json::from_str(include_str!("../../src/lib/meeting-source-copy.json"))
        .map_err(|error| format!("The recording reminder could not be read: {error}"))
}

fn consent_meeting_source<'a>(
    copy: &MeetingConsentCopy,
    source: &'a str,
    choice: &MessageDialogResult,
) -> Option<&'a str> {
    // Only the explicit custom button labels count. Cancel, close, and unexpected
    // native responses must never turn into an acknowledgement.
    match choice {
        MessageDialogResult::Custom(label) if source == "both" && label == &copy.got_it => Some("both"),
        MessageDialogResult::Custom(label) if source != "both" && label == &copy.got_it_record_both => Some("both"),
        MessageDialogResult::Custom(label) if source != "both" && label == &copy.got_it_start_anyway => Some(source),
        _ => None,
    }
}

fn tab_meeting_request_current(runtime: &Runtime, generation: u64) -> Result<bool, String> {
    let snapshot = runtime.snapshot()?;
    Ok(runtime.command_generation.load(Ordering::Acquire) == generation
        && !snapshot.meeting_recording
        && matches!(snapshot.state, AppState::Idle | AppState::Error))
}
pub(crate) fn current_copy_text(
    snapshot: &Snapshot,
    original: bool,
    result_session_id: Option<u64>,
) -> Result<String, String> {
    let requested = result_session_id
        .ok_or("This dictation is no longer current. Review it in Recent dictations.")?;
    if snapshot.result_session_id != Some(requested) {
        return Err("This dictation is no longer current. Review it in Recent dictations.".into());
    }
    if snapshot.recognition_completeness == Some(RecognitionCompleteness::Incomplete)
        && !matches!(snapshot.state, AppState::Idle | AppState::Error)
    {
        return Err("Review the current incomplete dictation before copying it.".into());
    }
    let text =
        if !original && snapshot.state == AppState::Error && !snapshot.interim_text.is_empty() {
            snapshot.interim_text.clone()
        } else if original {
            snapshot.original_text.clone()
        } else {
            snapshot.output_text.clone()
        };
    if text.is_empty() {
        return Err("There is no dictation to copy yet.".into());
    }
    Ok(text)
}

pub(crate) async fn copy_current_text(
    state: &Runtime,
    original: bool,
    result_session_id: Option<u64>,
) -> Result<(), String> {
    let snapshot = state.snapshot()?;
    let text = current_copy_text(&snapshot, original, result_session_id)?;
    platform::copy_text_after_claim(&text, || state.claim_manual_copy(result_session_id)).await?;
    if snapshot.recognition_completeness == Some(RecognitionCompleteness::Incomplete) {
        state.log_delivery(format!(
            "manual_review_copy session={} text={} recognitionCompleteness=incomplete clipboardOnly=true",
            result_session_id.unwrap_or_default(),
            if original { "original" } else { "finished" }
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_text(
    state: AppStateHandle<'_>,
    original: bool,
    result_session_id: Option<u64>,
) -> Result<(), String> {
    copy_current_text(state.inner().as_ref(), original, result_session_id).await
}

#[tauri::command]
pub fn list_meetings(state: AppStateHandle<'_>) -> Result<Vec<MeetingSummary>, String> {
    state.meetings.list()
}
#[tauri::command]
pub fn get_meeting(state: AppStateHandle<'_>, id: String) -> Result<MeetingDetail, String> {
    state.meetings.get(&id)
}

#[tauri::command]
pub async fn start_meeting(
    app: AppHandle,
    state: AppStateHandle<'_>,
    title: String,
    source: String,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Meeting)?;
        let _control = lock(&runtime.control)?;
        let settings = runtime.snapshot()?.settings;
        require_meeting_consent(&settings)?;
        let source = saved_meeting_source(&settings, &source)?.to_owned();
        if _control.microphone_preview.is_some() {
            return Err("Finish the microphone check before recording a meeting.".into());
        }
        if _control.is_publishing() {
            return Err("Wait for the current copy or delivery to finish before recording.".into());
        }
        if let Err(error) = crate::setup_commands::require_setup(
            &runtime.snapshot()?.settings,
            crate::setup_commands::CaptureKind::Meeting,
        ) {
            drop(_control);
            runtime.report(&app, error.clone());
            let _ = runtime.apply_tab(&app, true, Some(true), false, true);
            return Err(error);
        }
        if !matches!(runtime.snapshot()?.state, AppState::Idle | AppState::Error) {
            return Err("Finish dictation before recording a meeting.".into());
        }
        let mut recorder = lock(&runtime.recorder)?;
        if recorder.is_some() {
            return Err("A meeting is already recording.".into());
        }
        crate::ai_commands::cancel_optional_jobs();
        let meeting = runtime.meetings.create_recording(&title, &source)?;
        let paths = runtime.meetings.recording_paths(&meeting.id)?;
        match MeetingRecorder::start(
            RecordingPaths {
                audio: paths.mixed,
                microphone: paths.microphone,
                system: paths.system,
            },
            &source,
            runtime.snapshot()?.settings.microphone,
        ) {
            Ok(capture) => {
                *recorder = Some((meeting.id.clone(), capture));
                *lock(&runtime.meeting_update_activity)? = Some(activity);
            }
            Err(error) => {
                let _ = runtime.meetings.fail(&meeting.id, &error);
                return Err(error);
            }
        }
        {
            let mut snapshot = lock(&runtime.snapshot)?;
            snapshot.meeting_recording = true;
            snapshot.pre_model_text = None;
            snapshot.publication_stage = None;
            snapshot.guard_reason = None;
            snapshot.meeting_source_status.clear();
            snapshot.active_meeting_id = Some(meeting.id.clone());
            snapshot.error = None;
            snapshot.tab_visible = true;
            snapshot.tab_expanded = true;
            snapshot.tab_temporary = false;
        }
        drop(recorder);
        drop(_control);
        runtime.refresh_tab(&app)?;
        runtime.emit(&app);
        Ok(meeting)
    })
    .await
    .map_err(|e| format!("Meeting recorder failed: {e}"))?
}

fn saved_meeting_source<'a>(settings: &'a Settings, requested: &str) -> Result<&'a str, String> {
    settings.validate()?;
    if requested != settings.meeting_source {
        return Err("Meeting audio source changed. Review Record from before starting.".into());
    }
    Ok(&settings.meeting_source)
}

fn require_meeting_consent(settings: &Settings) -> Result<(), String> {
    if settings.meeting_consent_acknowledged {
        Ok(())
    } else {
        Err("MEETING_CONSENT_REQUIRED: Review the recording reminder before starting a meeting.".into())
    }
}

#[cfg(test)]
mod meeting_source_tests {
    use super::{saved_meeting_source, require_meeting_consent, meeting_consent_copy, consent_meeting_source};
    use crate::domain::Settings;
    use tauri_plugin_dialog::MessageDialogResult;

    #[test]
    fn meeting_command_requires_persisted_consent_for_every_source() {
        for source in ["both", "microphone", "system"] {
            let mut settings = Settings { meeting_source: source.into(), ..Settings::default() };
            assert_eq!(require_meeting_consent(&settings).unwrap_err(),
                "MEETING_CONSENT_REQUIRED: Review the recording reminder before starting a meeting.");
            settings.meeting_consent_acknowledged = true;
            assert!(require_meeting_consent(&settings).is_ok());
        }
    }

    #[test]
    fn native_reminder_only_accepts_explicit_got_it_buttons() {
        let copy = meeting_consent_copy().unwrap();
        for source in ["both", "microphone", "system"] {
            for choice in [MessageDialogResult::Cancel, MessageDialogResult::No,
                MessageDialogResult::Yes, MessageDialogResult::Ok,
                MessageDialogResult::Custom(copy.cancel.clone()),
                MessageDialogResult::Custom("unexpected".into())] {
                assert_eq!(consent_meeting_source(&copy, source, &choice), None);
            }
            if source == "both" {
                assert_eq!(consent_meeting_source(&copy, source,
                    &MessageDialogResult::Custom(copy.got_it.clone())), Some("both"));
            } else {
                assert_eq!(consent_meeting_source(&copy, source,
                    &MessageDialogResult::Custom(copy.got_it_record_both.clone())), Some("both"));
                assert_eq!(consent_meeting_source(&copy, source,
                    &MessageDialogResult::Custom(copy.got_it_start_anyway.clone())), Some(source));
                assert_eq!(consent_meeting_source(&copy, source,
                    &MessageDialogResult::Custom(copy.got_it.clone())), None);
            }
        }
    }

    #[test]
    fn meeting_start_uses_saved_source_and_rejects_stale_requests() {
        for source in ["both", "microphone", "system"] {
            let settings = Settings { meeting_source: source.into(), ..Settings::default() };
            assert_eq!(saved_meeting_source(&settings, source).unwrap(), source);
            for other in ["both", "microphone", "system", ""] {
                assert_eq!(saved_meeting_source(&settings, other).is_ok(), other == source);
            }
        }
    }
}

pub fn finish_recording(app: &AppHandle, runtime: &Arc<Runtime>) -> Result<MeetingDetail, String> {
    finish_meeting_capture(app, runtime, false)
}

pub(crate) fn take_finalizing_recording(
    meetings: &crate::meetings::Meetings,
    active: &mut Option<(String, MeetingRecorder)>,
) -> Result<(String, MeetingRecorder, MeetingDetail), String> {
    let (id, recorder) = active.as_ref().ok_or("No meeting is recording.")?;
    // Stop cannot depend on metadata I/O succeeding. If the write fails, keep
    // the stopped recorder and its completion receiver available for a retry;
    // the persisted recording state continues to guard unfinished audio.
    recorder.request_stop();
    let finalizing = meetings.begin_recording_finalization(id)?;
    let (id, recorder) = active.take().ok_or("No meeting is recording.")?;
    Ok((id, recorder, finalizing))
}

fn finish_meeting_capture(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    transcribe: bool,
) -> Result<MeetingDetail, String> {
    let _control = lock(&runtime.control)?;
    let mut active = lock(&runtime.recorder)?;
    // Persist the busy state before relinquishing capture ownership. A restart
    // can recover this checkpoint even if the writer never returns.
    let (id, recorder, finalizing) = take_finalizing_recording(&runtime.meetings, &mut active)?;
    let activity = lock(&runtime.meeting_update_activity)?.take();
    drop(active);
    let warnings = recorder.recording_warnings_handle();
    let issues = recorder.recording_issues_handle();
    {
        let mut snapshot = lock(&runtime.snapshot)?;
        snapshot.meeting_recording = false;
        snapshot.active_meeting_id = None;
        snapshot.audio_level = 0.0;
        snapshot.meeting_levels = None;
        snapshot.meeting_source_status.clear();
    }
    // Neither the UI snapshot nor the control mutex waits on a device driver.
    drop(_control);
    runtime.emit(app);
    let finish = {
        let app = app.clone();
        let runtime = runtime.clone();
        move |result| {
            // Keep ownership through metadata finalization and the handoff to
            // transcription, including a recorder completion timeout.
            let _activity = activity;
            let detail = runtime.meetings.complete_recording_with_issues(&id, result, lock(&warnings)?.clone(), lock(&issues)?.clone());
            runtime.emit(&app);
            let detail = detail?;
            if transcribe {
                transcribe_finished_recording(&app, &runtime, detail)
            } else {
                Ok(detail)
            }
        }
    };
    let app = app.clone();
    let runtime = runtime.clone();
    recorder.stop().finish_with(finish, move |error| runtime.report(&app, error))
        .map(|completed| completed.unwrap_or(finalizing))
}

fn transcribe_finished_recording(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    detail: MeetingDetail,
) -> Result<MeetingDetail, String> {
    let job = match runtime.meetings.begin_transcription(&detail.id, None) {
        Ok(job) => job,
        Err(error) => {
            let failed = runtime.meetings.fail(&detail.id, &error)?;
            runtime.emit(app);
            return Ok(failed);
        }
    };
    let pending = runtime.meetings.get(&detail.id)?;
    runtime.emit(app);
    let app = app.clone();
    let runtime = runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = transcribe_saved_meeting(&app, &runtime, job);
    });
    Ok(pending)
}

#[tauri::command]
pub async fn stop_meeting(
    app: AppHandle,
    state: AppStateHandle<'_>,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        finish_meeting_capture(&app, &runtime, true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn import_media(
    app: AppHandle,
    state: AppStateHandle<'_>,
    path: String,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Media)?;
        let meeting = runtime.meetings.begin_import(Path::new(&path))?;
        runtime.emit(&app);
        let result = runtime.meetings.finish_import(
            &meeting.id,
            crate::setup_commands::converter_path(&runtime).as_deref(),
        );
        runtime.emit(&app);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn transcribe_meeting(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    model: Option<String>,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let job = runtime
            .meetings
            .begin_transcription(&id, model.as_deref())?;
        transcribe_saved_meeting(&app, &runtime, job)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn transcribe_saved_meeting(
    app: &AppHandle,
    runtime: &Runtime,
    job: crate::meetings::MeetingTranscription,
) -> Result<MeetingDetail, String> {
    let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Meeting)?;
    // Single-owner ASR: MeetingTranscription holds the ASR lease; the closure
    // uses the snapshotted model without re-reading settings or acquiring.
    // Unknown IDs error; never fall back to another installed model or cloud.
    let mut recognizer = None;
    let mut granite = None;
    let result = job.run_recognition(move |audio, model, cancel| {
        runtime.emit(app);
        if crate::granite_speech::is_granite(model) {
            if granite.is_none() {
                granite = Some(crate::granite_worker::GraniteWorker::load(&runtime.models_root, cancel)?);
            }
            return crate::granite_speech::transcribe_file(granite.as_mut().unwrap(), audio, cancel);
        }
        if recognizer.is_none() {
            recognizer = Some(crate::asr::Recognizer::load_interruptible(&runtime.models_root, model, cancel)?);
        }
        crate::asr::transcribe_file_recognition_with(recognizer.as_mut().unwrap(), audio, cancel)
    });
    runtime.emit(app);
    // Publishing recognition wakes the idle title worker for every source.
    // Naming must not hold this command open or depend on Notes/cloud settings.
    result
}

#[tauri::command]
pub fn cancel_meeting(app: AppHandle, state: AppStateHandle<'_>, id: String) -> Result<(), String> {
    if state.snapshot()?.active_meeting_id.as_deref() == Some(&id) {
        return Err("Stop the recording before cancelling its processing.".into());
    }
    state.meetings.cancel(&id)?;
    state.emit(&app);
    Ok(())
}
#[tauri::command]
pub fn save_meeting(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    title: String,
    transcript: String,
    notes: String,
    cleaned_transcript: Option<String>,
) -> Result<MeetingDetail, String> {
    let result = state.meetings.save_with_cleaned(&id, &title, &transcript, &notes, cleaned_transcript.as_deref())?;
    state.emit(&app);
    Ok(result)
}
#[tauri::command]
pub fn set_meeting_speaker_labels(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    enabled: bool,
    confirmed: bool,
) -> Result<MeetingDetail, String> {
    let result = state.meetings.set_speaker_labels(&id, enabled, confirmed)?;
    state.emit(&app);
    Ok(result)
}
#[tauri::command]
pub fn delete_meeting(app: AppHandle, state: AppStateHandle<'_>, id: String) -> Result<(), String> {
    if state.snapshot()?.active_meeting_id.as_deref() == Some(&id) {
        return Err("Stop this recording before deleting it.".into());
    }
    state.meetings.delete(&id)?;
    state.emit(&app);
    Ok(())
}
#[tauri::command]
pub async fn generate_notes(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    preset: crate::meeting_ai::NotesPreset,
    instructions: String,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Single snapshot per notes job; single text-lease owner is the
        // snapshot worker. Never call ai::generate (per-request acquire)
        // inside a job that already holds the whole-job lease.
        let snapshot = crate::ai::snapshot_notes_config(&runtime.store)?;
        let result = runtime.meetings.generate_notes(
            &id,
            &instructions,
            |transcript, instructions, cancel| {
                runtime.emit(&app);
                crate::meeting_ai::generate_notes_with_snapshot(
                    &runtime.models_root,
                    &runtime.store,
                    &snapshot,
                    transcript,
                    preset,
                    instructions,
                    cancel,
                )
            },
        );
        runtime.emit(&app);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn clean_meeting(
    app: AppHandle, state: AppStateHandle<'_>, id: String,
) -> Result<MeetingDetail, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = runtime.meetings.clean_local(&id, || runtime.emit(&app));
        runtime.emit(&app);
        result
    }).await.map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn export_meeting(
    state: AppStateHandle<'_>,
    id: String,
    path: String,
    format: String,
    transcript_version: Option<String>,
) -> Result<(), String> {
    let meetings = state.meetings.clone();
    tauri::async_runtime::spawn_blocking(move || meetings.export_version(&id, Path::new(&path), &format, transcript_version.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}
/// Save a verbatim copy of the saved original recording (`{title}.wav`).
///
/// Separate from `export_meeting` so the Save-copy audio button does not
/// need a transcript/draft and stays available while transcription/notes
/// process. Refuses an active recording; streams atomically; never deletes
/// the original on failure.
#[tauri::command]
pub async fn export_meeting_audio(
    state: AppStateHandle<'_>,
    id: String,
    path: String,
) -> Result<(), String> {
    let meetings = state.meetings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Media)?;
        meetings.export_audio(&id, Path::new(&path)).map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn get_vocabulary(state: AppStateHandle<'_>) -> Result<Vec<VocabularyEntry>, String> {
    state.store.get_vocabulary()
}
#[tauri::command]
pub fn save_vocabulary(
    state: AppStateHandle<'_>,
    entries: Vec<VocabularyEntry>,
) -> Result<Vec<VocabularyEntry>, String> {
    state.store.save_vocabulary(&entries)
}

#[tauri::command]
pub fn list_dictations(
    state: AppStateHandle<'_>,
    limit: usize,
) -> Result<Vec<DictationRecord>, String> {
    state.store.list_dictations(limit)
}

#[tauri::command]
pub fn count_dictations(state: AppStateHandle<'_>) -> Result<usize, String> {
    state.store.count_dictations()
}

#[tauri::command]
pub fn delete_dictation(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    confirmed: bool,
) -> Result<Snapshot, String> {
    let result = state.delete_dictation_record(&id, confirmed);
    state.emit(&app);
    let _ = app.emit("dictation-history-changed", ());
    result?;
    state.snapshot()
}

#[tauri::command]
pub fn delete_all_dictations(
    app: AppHandle,
    state: AppStateHandle<'_>,
    confirmed: bool,
) -> Result<DeletedDictations, String> {
    let result = state.delete_all_dictation_records(confirmed);
    state.emit(&app);
    let _ = app.emit("dictation-history-changed", ());
    Ok(DeletedDictations {
        deleted_count: result?,
        snapshot: state.snapshot()?,
    })
}

#[tauri::command]
pub fn save_dictation_correction(
    app: AppHandle,
    state: AppStateHandle<'_>,
    id: String,
    text: String,
    scope: Option<String>,
) -> Result<SavedCorrection, String> {
    let saved = state.save_correction(&id, &text, scope.as_deref())?;
    state.emit(&app);
    let _ = app.emit("dictation-history-changed", &id);
    Ok(saved)
}

#[tauri::command]
pub fn get_learning(state: AppStateHandle<'_>) -> Result<Vec<LearnedEntry>, String> {
    state.store.get_learning()
}

#[tauri::command]
pub fn save_learning(
    state: AppStateHandle<'_>,
    entries: Vec<LearnedEntry>,
) -> Result<Vec<LearnedEntry>, String> {
    state.store.save_learning(&entries)
}

#[tauri::command]
pub fn reset_learning(state: AppStateHandle<'_>, confirmed: bool) -> Result<(), String> {
    state.store.reset_learning(confirmed)
}
#[tauri::command]
pub fn learn_correction(
    state: AppStateHandle<'_>,
    id: String,
    heard: String,
    written: String,
) -> Result<Vec<VocabularyEntry>, String> {
    state.store.learn_correction(&id, &heard, &written)
}

#[tauri::command]
pub async fn copy_history(
    state: AppStateHandle<'_>,
    id: String,
    original: bool,
) -> Result<(), String> {
    let entry = state.store.get_dictation(&id)?;
    let text = if original {
        &entry.raw
    } else {
        &entry.published
    };
    if text.trim().is_empty() {
        return Err("There is no dictation text to copy.".into());
    }
    platform::copy_text_after_claim(text, || state.claim_history_copy(&id)).await?;
    if entry.recognition_completeness == RecognitionCompleteness::Incomplete {
        state.log_delivery(format!(
            "manual_history_review_copy record={} text={} recognitionCompleteness=incomplete clipboardOnly=true",
            entry.id,
            if original { "original" } else { "finished" }
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_history_pre_model(state: AppStateHandle<'_>, id: String) -> Result<(), String> {
    let entry = state.store.get_dictation(&id)?;
    let text = entry.pre_model_text.as_deref().filter(|text| !text.trim().is_empty())
        .ok_or("There is no original cleanup text to copy.")?;
    platform::copy_text_after_claim(text, || state.claim_history_copy(&id)).await
}

#[tauri::command]
pub async fn open_punctuation_model_page(
    state: AppStateHandle<'_>,
    id: Option<String>,
) -> Result<(), String> {
    let root = state.models_root.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Cold verification can hash package files; keep it off the UI thread.
        let url = punctuation_model_page_url(id.as_deref(), || {
            crate::punctuation_model::selected_package(&root, crate::model_verification::Check::Blocking)
        })?;
        // Only package-owned HTTPS destinations reach the OS browser.
        crate::legal_links::open_https_blocking(url, "model page")
    })
    .await
    .map_err(|e| e.to_string())?
}

fn punctuation_model_page_url(
    id: Option<&str>,
    selected: impl FnOnce() -> Option<crate::punctuation_model::Package>,
) -> Result<&'static str, String> {
    use crate::punctuation_model::Package;
    let package = match id {
        Some(id) => Package::from_id(id).ok_or("Unknown punctuation package.")?,
        None => selected().unwrap_or(Package::V2),
    };
    Ok(package.page_url())
}

#[cfg(test)]
mod punctuation_model_page_tests {
    use super::punctuation_model_page_url;
    use crate::punctuation_model::Package;

    #[test]
    fn default_page_follows_the_verified_package_or_released_v2() {
        for (selected, expected) in [
            (Some(Package::V1), "https://huggingface.co/SonoJot/sonojot-punct-v1"),
            (Some(Package::V2), "https://huggingface.co/SonoJot/sonojot-punct-v2"),
            (None, "https://huggingface.co/SonoJot/sonojot-punct-v2"),
        ] {
            assert_eq!(punctuation_model_page_url(None, || selected).unwrap(), expected);
        }
        assert!(Package::V2.released(), "The default model page must be published.");
    }

    #[test]
    fn explicit_library_pages_skip_selection_and_reject_arbitrary_destinations() {
        for (id, expected) in [
            ("sonojot-punct-v1", "https://huggingface.co/SonoJot/sonojot-punct-v1"),
            ("sonojot-punct-v2", "https://huggingface.co/SonoJot/sonojot-punct-v2"),
        ] {
            assert_eq!(punctuation_model_page_url(Some(id), || panic!("explicit link must not verify models")).unwrap(), expected);
        }
        for id in ["", "sonojot-punct-v3", "../sonojot-punct-v1", "https://huggingface.co/SonoJot/sonojot-punct-v2",
            "https://example.com", "file:///C:/Windows", "--help"] {
            assert_eq!(punctuation_model_page_url(Some(id), || panic!("invalid id must not verify models")).unwrap_err(),
                "Unknown punctuation package.");
        }
    }
}

#[tauri::command]
pub fn open_path(state: AppStateHandle<'_>, kind: String) -> Result<(), String> {
    let path = match kind.as_str() {
        "data" => state.store.root().to_path_buf(),
        "models" => state.models_root.clone(),
        "logs" => state.store.logs_dir(),
        _ => return Err("Unknown app folder.".into()),
    };
    open_folder(&path)
}
fn open_folder(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("The app folder does not exist yet.".into());
    }
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Cannot open the folder: {e}"))
}
#[tauri::command]
pub fn get_diagnostics(state: AppStateHandle<'_>) -> Result<String, String> {
    let snapshot = state.snapshot()?;
    // Match speech session defaults. The saved trial flag remains migration
    // evidence; result delivery logs retain each producing policy's identity.
    let policy = cleanup::CleanupPolicy::conservative();
    Ok(serde_json::json!({ "version": snapshot.version, "platform": snapshot.capabilities, "state": snapshot.state,
        "asrModel": snapshot.settings.asr_model, "cleanupModel": snapshot.settings.cleanup_model,
        "cleanupTrial": snapshot.settings.cleanup_trial,
        "recognitionCompleteness": snapshot.recognition_completeness,
        "recognitionWarnings": snapshot.recognition_warnings,
        "resultSessionId": snapshot.result_session_id,
        "cleanupPolicyVersion": policy.version, "cleanupPolicyHash": policy.hash(),
        "models": model_catalog(&state.models_root), "microphoneCount": snapshot.devices.len(), "meetingCount": state.meetings.list()?.len(),
        "runtime": "Rust + Tauri", "dataSchema": 1,
        "lastDelivery": state.last_delivery.lock().ok().and_then(|value| value.clone()) }).to_string())
}
