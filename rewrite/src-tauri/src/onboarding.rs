//! The existing setup-seen marker also owns pending first-run downloads.
//! No recognition, transcript publication or model verification policy lives here.
use crate::{
    app::{lock, Runtime},
    domain::{Settings, SetupProfile, Snapshot},
    store,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, State};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

static MARKER_GATE: Mutex<()> = Mutex::new(());
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

// Keep in sync with src/lib/terms.ts and the bundled legal/terms-of-use.md.
pub(crate) const TERMS_VERSION: u32 = 1;

pub(crate) fn terms_accepted(settings: &Settings) -> bool {
    settings.terms_accepted_version >= TERMS_VERSION
        && settings
            .terms_accepted_at
            .as_ref()
            .is_some_and(|at| !at.trim().is_empty())
}

pub(crate) fn require_terms(settings: &Settings) -> Result<(), String> {
    if terms_accepted(settings) {
        Ok(())
    } else {
        Err("TERMS_REQUIRED: Open SonoJot and review the Terms of Use to continue.".into())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct SetupProgress {
    pub seen: bool,
    #[serde(default)]
    profile: Option<SetupProfile>,
    #[serde(default)]
    downloads: Vec<String>,
}

fn read(root: &Path) -> Result<Option<SetupProgress>, String> {
    match std::fs::read(root.join("setup-seen.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn write(root: &Path, progress: &SetupProgress) -> Result<(), String> {
    store::atomic_write(
        &root.join("setup-seen.json"),
        &serde_json::to_vec(progress).map_err(|e| e.to_string())?,
    )
}
fn persist_choice(
    store: &store::Store,
    progress: &SetupProgress,
    settings: &Settings,
) -> Result<(), String> {
    write(store.root(), progress)?;
    store.save_settings(settings)
}
pub(crate) fn mark_seen(runtime: &Runtime) -> Result<(), String> {
    require_terms(&runtime.snapshot()?.settings)?;
    let _gate = lock(&MARKER_GATE)?;
    let mut progress = read(runtime.store.root())?.unwrap_or_default();
    progress.seen = true;
    write(runtime.store.root(), &progress)
}

pub(crate) fn show_queue(runtime: &Runtime, library: &mut crate::model_manager::ModelLibrary) {
    let Ok(_gate) = lock(&MARKER_GATE) else {
        return;
    };
    if let Ok(Some(progress)) = read(runtime.store.root()) {
        for model in &mut library.models {
            if model.state == "available" && progress.downloads.contains(&model.id) {
                model.state = "queued".into();
            }
        }
    }
}
fn required(settings: &Settings, progress: Option<&SetupProgress>) -> bool {
    settings.setup_profile.is_none() || progress.is_some_and(|p| !p.seen)
}
pub(crate) fn needs_setup(runtime: &Runtime) -> bool {
    runtime.snapshot().is_ok_and(|s| {
        !terms_accepted(&s.settings)
            || required(
                &s.settings,
                read(runtime.store.root()).ok().flatten().as_ref(),
            )
    })
}

#[tauri::command]
pub fn begin_onboarding(state: State<'_, Arc<Runtime>>) -> Result<bool, String> {
    let _gate = lock(&MARKER_GATE)?;
    let progress = read(state.store.root())?;
    let needs = required(&state.snapshot()?.settings, progress.as_ref());
    if needs && progress.is_none() {
        write(state.store.root(), &SetupProgress::default())?;
    }
    Ok(needs)
}

fn download_ids(settings: &Settings) -> Vec<String> {
    let mut ids = vec![
        settings.asr_model.clone(),
        crate::punctuation_model::Package::V2.id().into(),
    ];
    if settings.notes_model != "openrouter" && !ids.contains(&settings.notes_model) {
        ids.push(settings.notes_model.clone());
    }
    ids
}

fn chosen_settings(
    old: &Settings,
    profile: SetupProfile,
    selection: Option<&crate::profiles::Selection>,
    defaults: bool,
) -> Settings {
    let mut next = crate::setup_commands::profile_settings(old, profile, selection);
    if defaults {
        let base = Settings::default();
        next.microphone = base.microphone;
        next.hotkey = base.hotkey;
        next.hotkey_mode = base.hotkey_mode;
    }
    next.punctuation_recommendation_acknowledged = true;
    next.punctuation_v2_recommendation_acknowledged = true;
    next
}

/// First-run choices may name packages still downloading. Capture remains gated
/// by setup_profile=None until the tour finishes. Established tour edits keep
/// the ordinary model-selection exclusion and never apply on an unchanged tour.
async fn commit(
    app: &AppHandle,
    runtime: &Arc<Runtime>,
    profile: SetupProfile,
    defaults: bool,
    complete: bool,
) -> Result<Snapshot, String> {
    let _settings = runtime.settings_gate.lock().await;
    let r = runtime.clone();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selection = (profile != SetupProfile::Custom).then(|| {
            crate::profiles::recommend(&crate::profiles::inspect(&r.models_root).hardware, profile)
        });
        let mut control = lock(&r.control)?;
        control.microphone_preview.take();
        let snapshot = r.snapshot()?;
        require_terms(&snapshot.settings)?;
        crate::setup_commands::require_idle(&snapshot)?;
        let old = snapshot.settings;
        let _marker = lock(&MARKER_GATE)?;
        let saved_progress = read(r.store.root())?;
        let first = required(&old, saved_progress.as_ref());
        let mut progress = saved_progress.unwrap_or_default();
        if defaults && !first {
            return r.snapshot();
        }
        let mut next = chosen_settings(&old, profile, selection.as_ref(), defaults);
        if first && !complete {
            next.setup_profile = None;
        }
        next.validate()?;
        let ids = download_ids(&next);
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        crate::download_consent::require_components(&r, &ids)?;
        let changed = old.asr_model != next.asr_model
            || old.notes_model != next.notes_model
            || old.cleanup_model != next.cleanup_model;
        crate::resource_manager::with_model_selection(!first && changed, || {
            let shortcut_changed = old.hotkey != next.hotkey;
            let registered =
                !shortcut_changed || crate::app::register_shortcut(&a, &next.hotkey).is_ok();
            // Persist the queue before the settings. A failed second write keeps
            // the unfinished marker, so restarting reopens setup.
            progress.profile = Some(profile);
            progress.downloads = download_ids(&next);
            progress.seen = !first;
            if let Err(error) = persist_choice(&r.store, &progress, &next) {
                if shortcut_changed && registered {
                    let _ = a.global_shortcut().unregister(next.hotkey.as_str());
                }
                return Err(error);
            }
            if shortcut_changed {
                let _ = a.global_shortcut().unregister(old.hotkey.as_str());
            }
            let mut snapshot = lock(&r.snapshot)?;
            snapshot.settings = next.clone();
            if shortcut_changed {
                snapshot.capabilities.global_shortcut = registered;
            }
            Ok(())
        })?;
        if profile != SetupProfile::Custom {
            let mut ai = crate::ai::load_settings(&r.store)?;
            if ai.backend != crate::ai::AiBackend::Local {
                ai.backend = crate::ai::AiBackend::Local;
                crate::ai::save_settings(&r.store, &ai)?;
            }
        }
        progress.seen = complete || !first;
        write(r.store.root(), &progress)?;
        crate::resource_manager::configure(&next);
        drop(control);
        r.emit(&a);
        r.snapshot()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn confirm_onboarding_profile(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    profile: SetupProfile,
) -> Result<Snapshot, String> {
    let result = commit(&app, state.inner(), profile, false, false).await?;
    resume_downloads(&app, state.inner());
    Ok(result)
}

/// Optional meeting extras setup may queue behind the profile downloads.
const OPTIONAL_DOWNLOADS: &[&str] = &["nemotron-3-diarization"];

/// Queue an optional extra behind the confirmed profile downloads, so it
/// installs one at a time even if setup is finished first. Without a setup
/// marker (an existing install replaying the tour) it starts directly.
#[tauri::command]
pub async fn queue_onboarding_download(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    id: String,
) -> Result<crate::model_manager::ModelLibrary, String> {
    if !OPTIONAL_DOWNLOADS.contains(&id.as_str()) {
        return Err("This download cannot be queued from setup.".into());
    }
    let runtime = state.inner().clone();
    require_terms(&runtime.snapshot()?.settings)?;
    crate::download_consent::require_components(&runtime, &[&id])?;
    let queued = {
        let _gate = lock(&MARKER_GATE)?;
        queue_at(runtime.store.root(), &id)?
    };
    if queued {
        resume_downloads(&app, &runtime);
    }
    tauri::async_runtime::spawn_blocking(move || {
        if !queued {
            crate::model_manager::require_idle_snapshot(&runtime.snapshot()?)?;
            crate::model_manager::start_install(runtime.models_root.clone(), id)?;
        }
        let settings = runtime.snapshot()?.settings;
        let mut library = crate::model_manager::build_library(&runtime.models_root, &settings);
        show_queue(&runtime, &mut library);
        Ok(library)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn finish_onboarding(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    let _settings = state.settings_gate.lock().await;
    require_terms(&state.snapshot()?.settings)?;
    let _marker = lock(&MARKER_GATE)?;
    let mut progress = read(state.store.root())?.unwrap_or_default();
    if !required(&state.snapshot()?.settings, Some(&progress)) {
        return state.snapshot();
    }
    let profile = progress
        .profile
        .ok_or("Confirm a model profile before finishing setup.")?;
    let mut next = state.snapshot()?.settings;
    next.setup_profile = Some(profile);
    state.store.save_settings(&next)?;
    lock(&state.snapshot)?.settings = next;
    progress.seen = true;
    write(state.store.root(), &progress)?;
    state.emit(&app);
    state.snapshot()
}

pub(crate) async fn close_early(_app: &AppHandle, runtime: &Arc<Runtime>) -> Result<(), String> {
    // Closing is not agreement or a model choice. Leave the unfinished marker
    // intact; only an explicit profile confirmation may queue downloads.
    stop_preview(runtime)
}

/// Resume previously confirmed downloads after acceptance. An interrupted tour
/// remains unfinished; restarting must never silently choose a profile.
pub(crate) async fn recover(app: &AppHandle, runtime: &Arc<Runtime>) -> Result<(), String> {
    if adopt_default_punctuation(runtime).await? {
        runtime.emit(app);
    }
    resume_downloads(app, runtime);
    Ok(())
}

/// One-time migration for installs that predate the punctuation default.
/// Every new setup queues punctuation; an existing install adopts it only when
/// nothing shows a choice against it: Setup is finished, Cleanup is still the
/// default local rules, dictation is not As heard, neither punctuation notice
/// was answered (install, dismiss or first-run inclusion all acknowledge it)
/// and no punctuation package folder exists (a removal stays removed).
fn punctuation_default_due(
    settings: &Settings,
    progress: Option<&SetupProgress>,
    models_root: &Path,
    available: bool,
) -> bool {
    use crate::punctuation_model::Package;
    available
        && !required(settings, progress)
        && settings.cleanup_model == "local-rules"
        && settings.cleanup_mode != crate::domain::CleanupMode::Original
        && !settings.punctuation_recommendation_acknowledged
        && !settings.punctuation_v2_recommendation_acknowledged
        && [Package::V1, Package::V2].iter().all(|package| !package.path(models_root).exists())
}

async fn adopt_default_punctuation(runtime: &Arc<Runtime>) -> Result<bool, String> {
    let _settings = runtime.settings_gate.lock().await;
    let available = crate::diarization_model::AVAILABLE
        && crate::punctuation_model::Package::V2.released();
    let mut settings = runtime.snapshot()?.settings;
    let _marker = lock(&MARKER_GATE)?;
    let progress = read(runtime.store.root())?;
    if !punctuation_default_due(&settings, progress.as_ref(), &runtime.models_root, available) {
        return Ok(false);
    }
    // Queue before acknowledging, as `commit` does: a failed settings write
    // leaves the notice unanswered and the queue deduplicated next launch.
    let mut progress = progress.unwrap_or(SetupProgress { seen: true, ..Default::default() });
    let id = crate::punctuation_model::Package::V2.id().to_string();
    if !progress.downloads.contains(&id) {
        progress.downloads.push(id);
    }
    write(runtime.store.root(), &progress)?;
    settings.punctuation_v2_recommendation_acknowledged = true;
    runtime.store.save_settings(&settings)?;
    lock(&runtime.snapshot)?.settings = settings;
    Ok(true)
}

pub(crate) fn cancel_queued(runtime: &Runtime, id: &str) -> Result<(), String> {
    let _gate = lock(&MARKER_GATE)?;
    cancel_at(runtime.store.root(), id)
}
/// Append to an existing setup queue once. Never creates a setup marker, which
/// would reopen setup; returns whether a queue existed.
fn queue_at(root: &Path, id: &str) -> Result<bool, String> {
    let Some(mut progress) = read(root)? else {
        return Ok(false);
    };
    if !progress.downloads.iter().any(|queued| queued == id) {
        progress.downloads.push(id.to_string());
        write(root, &progress)?;
    }
    Ok(true)
}
fn cancel_at(root: &Path, id: &str) -> Result<(), String> {
    if let Some(mut progress) = read(root)? {
        progress.downloads.retain(|queued| queued != id);
        write(root, &progress)?;
    }
    Ok(())
}

pub(crate) fn resume_downloads(app: &AppHandle, runtime: &Arc<Runtime>) {
    if !runtime.snapshot().is_ok_and(|s| terms_accepted(&s.settings)) {
        return;
    }
    if DOWNLOADING.swap(true, Ordering::AcqRel) {
        return;
    }
    let a = app.clone();
    let r = runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut started = std::collections::HashSet::new();
        let mut run = || -> Result<(), String> {
            loop {
                // Recheck each attempt, including queues written by an older
                // release before this terms version was introduced.
                if !terms_accepted(&r.snapshot()?.settings) {
                    DOWNLOADING.store(false, Ordering::Release);
                    return Ok(());
                }
                let id = {
                    let _gate = lock(&MARKER_GATE)?;
                    let id = read(r.store.root())?.and_then(|p| p.downloads.first().cloned());
                    // Release ownership under the same gate used by queue writers.
                    if id.is_none() {
                        DOWNLOADING.store(false, Ordering::Release);
                    }
                    id
                };
                let Some(id) = id else {
                    return Ok(());
                };
                let settings = r.snapshot()?.settings;
                let catalog = crate::model_manager::build_library(&r.models_root, &settings);
                let model = catalog
                    .models
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or("Queued setup model is unavailable.")?;
                if matches!(
                    model.state.as_str(),
                    "installed" | "ready" | "unloaded" | "loading" | "busy"
                ) {
                    cancel_queued(&r, &id)?;
                    continue;
                }
                if model.state == "checking" || crate::model_manager::install_in_progress() {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
                // Completion, cancellation or failure gets one attempt. The
                // library retains Retry; later queue entries can still install.
                if started.contains(&id) || !model.installable {
                    cancel_queued(&r, &id)?;
                    continue;
                }
                let attempt = {
                    let _gate = lock(&MARKER_GATE)?;
                    if !read(r.store.root())?.is_some_and(|p| p.downloads.contains(&id)) {
                        continue;
                    }
                    started.insert(id.clone());
                    crate::download_consent::require_components(&r, &[&id])
                        .and_then(|()| crate::model_manager::start_install(r.models_root.clone(), id.clone()))
                };
                if let Err(error) = attempt {
                    r.report(&a, error);
                    cancel_queued(&r, &id)?;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        };
        if let Err(error) = run() {
            DOWNLOADING.store(false, Ordering::Release);
            r.report(&a, error);
        }
    });
}

// Level-only capture uses the existing native audio owner, protected by Control.
// Samples are drained and discarded. No recognizer, files or clipboard is used.
pub(crate) struct MicrophonePreview {
    capture: crate::audio::Capture,
    receiver: std::sync::mpsc::Receiver<crate::audio::AudioMessage>,
    _activity: crate::resource_manager::UpdateActivityLease,
}
pub(crate) fn stop_preview(runtime: &Runtime) -> Result<(), String> {
    lock(&runtime.control)?.microphone_preview.take();
    Ok(())
}
#[derive(Serialize)]
pub struct MicrophoneLevel {
    level: f32,
    active: bool,
}
#[tauri::command]
pub async fn onboarding_microphone(
    state: State<'_, Arc<Runtime>>,
    action: String,
) -> Result<MicrophoneLevel, String> {
    let r = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut control = lock(&r.control)?;
        if action == "stop" { control.microphone_preview.take(); }
        else if action == "start" {
            require_terms(&r.snapshot()?.settings)?;
            crate::setup_commands::require_idle(&r.snapshot()?)?;
            if control.is_publishing() { return Err("Wait for copying to finish.".into()); }
            control.microphone_preview.take();
            let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Dictation)?;
            let mut capture = crate::audio::Capture::start(r.snapshot()?.settings.microphone)?;
            let receiver = capture.take_receiver()?;
            // Bound capture even if a webview disappears without cleanup.
            let stop = capture.stop_handle();
            tauri::async_runtime::spawn(async move { tokio::time::sleep(Duration::from_secs(60)).await; stop.store(true, Ordering::Release); });
            control.microphone_preview = Some(MicrophonePreview { capture, receiver, _activity: activity });
        } else if action != "level" { return Err("Unknown microphone check action.".into()); }
        if let Some(preview) = &control.microphone_preview {
            for _ in preview.receiver.try_iter() {}
            if preview.capture.is_finished() || preview.capture.failure_cause() != 0 {
                control.microphone_preview.take();
                return Err("The microphone check stopped. Check the device and microphone privacy settings, then try again.".into());
            }
            return Ok(MicrophoneLevel { level: f32::from_bits(preview.capture.level_handle().load(Ordering::Relaxed)).clamp(0.0, 1.0), active: true });
        }
        Ok(MicrophoneLevel { level: 0.0, active: false })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_permission_settings(kind: String) -> Result<(), String> {
    let destination = match kind.as_str() {
        "microphone" if cfg!(windows) => "ms-settings:privacy-microphone",
        "microphone" if cfg!(target_os = "macos") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        "accessibility" if cfg!(target_os = "macos") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        _ => return Err("This permission pane is unavailable on this platform.".into()),
    };
    std::process::Command::new(if cfg!(windows) {
        "explorer.exe"
    } else {
        "open"
    })
    .arg(destination)
    .spawn()
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn onboarding_paste_permission() -> Option<String> {
    crate::platform::auto_paste_unavailable_reason()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_extras_join_an_existing_queue_once_and_never_create_one() {
        let root = std::env::temp_dir().join(format!(
            "sonojot-onboarding-extras-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        assert!(!queue_at(&root, "nemotron-3-diarization").unwrap());
        assert!(read(&root).unwrap().is_none());
        let progress = SetupProgress {
            downloads: vec!["speech".into()],
            seen: true,
            ..SetupProgress::default()
        };
        write(&root, &progress).unwrap();
        assert!(queue_at(&root, "nemotron-3-diarization").unwrap());
        assert!(queue_at(&root, "nemotron-3-diarization").unwrap());
        let saved = read(&root).unwrap().unwrap();
        assert_eq!(saved.downloads, vec!["speech", "nemotron-3-diarization"]);
        assert!(saved.seen);
        assert!(!OPTIONAL_DOWNLOADS.contains(&"speech"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn terms_acceptance_requires_the_version_and_local_timestamp() {
        let mut settings = Settings::default();
        assert!(!terms_accepted(&settings));
        settings.terms_accepted_version = TERMS_VERSION;
        assert!(!terms_accepted(&settings));
        settings.terms_accepted_at = Some("  ".into());
        assert!(!terms_accepted(&settings));
        settings.terms_accepted_at = Some("2026-10-12T12:00:00.000Z".into());
        assert!(terms_accepted(&settings));
        settings.terms_accepted_version = TERMS_VERSION - 1;
        assert!(!terms_accepted(&settings));
        settings.terms_accepted_version = TERMS_VERSION + 1;
        assert!(terms_accepted(&settings));
    }

    #[test]
    fn completed_and_legacy_users_are_not_forced_through_setup() {
        let mut settings = Settings::default();
        assert!(required(&settings, None));
        settings.setup_profile = Some(SetupProfile::Custom);
        assert!(!required(&settings, None));
        let legacy: SetupProgress = serde_json::from_str(r#"{"seen":true}"#).unwrap();
        assert!(!required(&settings, Some(&legacy)));
        assert!(required(&Settings::default(), Some(&legacy)));
        assert!(required(&settings, Some(&SetupProgress::default())));
    }
    #[test]
    fn every_profile_queues_v2_after_speech_even_custom() {
        for profile in [
            SetupProfile::Accurate,
            SetupProfile::Balanced,
            SetupProfile::Light,
            SetupProfile::Custom,
        ] {
            let settings = chosen_settings(&Settings::default(), profile, None, false);
            let ids = download_ids(&settings);
            assert_eq!(ids[0], settings.asr_model);
            assert_eq!(ids[1], "sonojot-punct-v2");
            assert!(settings.punctuation_v2_recommendation_acknowledged);
        }
    }
    #[test]
    fn fresh_settings_default_to_rules_and_punctuation_cleanup() {
        let settings = Settings::default();
        assert_eq!(settings.cleanup_model, "local-rules");
        assert_eq!(settings.cleanup_mode, crate::domain::CleanupMode::Clean);
        // A fresh install is not migrated; its Setup queues punctuation itself.
        let root = std::env::temp_dir().join(format!("sonojot-punct-default-{}", uuid::Uuid::new_v4()));
        assert!(!punctuation_default_due(&settings, None, &root, true));
    }
    #[test]
    fn existing_installs_adopt_punctuation_unless_they_chose_otherwise() {
        use crate::punctuation_model::Package;
        let root = std::env::temp_dir().join(format!("sonojot-punct-migrate-{}", uuid::Uuid::new_v4()));
        let seen = SetupProgress { seen: true, ..Default::default() };
        let existing = Settings {
            setup_profile: Some(SetupProfile::Balanced),
            ..Settings::default()
        };
        for profile in [SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light, SetupProfile::Custom] {
            let settings = Settings { setup_profile: Some(profile), ..existing.clone() };
            assert!(punctuation_default_due(&settings, Some(&seen), &root, true), "{profile:?}");
            // Legacy installs may have no setup marker at all.
            assert!(punctuation_default_due(&settings, None, &root, true), "{profile:?}");
        }
        assert!(punctuation_default_due(
            &Settings { cleanup_mode: crate::domain::CleanupMode::Format, ..existing.clone() },
            Some(&seen), &root, true,
        ));
        let declined = [
            Settings { cleanup_model: "lfm2.5-1.2b".into(), ..existing.clone() },
            Settings { cleanup_model: crate::optional_llm::SMART_CLEANUP_ID.into(), setup_profile: Some(SetupProfile::Custom), ..existing.clone() },
            Settings { cleanup_mode: crate::domain::CleanupMode::Original, ..existing.clone() },
            Settings { punctuation_recommendation_acknowledged: true, ..existing.clone() },
            Settings { punctuation_v2_recommendation_acknowledged: true, ..existing.clone() },
            Settings { setup_profile: None, ..existing.clone() },
        ];
        for settings in &declined {
            assert!(!punctuation_default_due(settings, Some(&seen), &root, true));
        }
        // Unfinished tours and unsupported platforms are left alone.
        assert!(!punctuation_default_due(&existing, Some(&SetupProgress::default()), &root, true));
        assert!(!punctuation_default_due(&existing, Some(&seen), &root, false));
        // Any package folder (installed, damaged or manual) is an earlier choice.
        for package in [Package::V1, Package::V2] {
            std::fs::create_dir_all(package.path(&root)).unwrap();
            assert!(!punctuation_default_due(&existing, Some(&seen), &root, true));
            std::fs::remove_dir_all(package.path(&root)).unwrap();
        }
        assert!(punctuation_default_due(&existing, Some(&seen), &root, true));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn explicit_default_choice_preserves_cleanup_behavior() {
        let mut old = Settings::default();
        old.microphone = Some("other".into());
        old.hotkey = "Alt+Shift+D".into();
        old.hotkey_mode = crate::domain::HotkeyMode::Hold;
        let choice = crate::profiles::Selection {
            asr_model: "speech".into(),
            cleanup_model: "local-rules".into(),
            notes_model: "notes".into(),
            reasons: vec![],
        };
        let next = chosen_settings(&old, SetupProfile::Balanced, Some(&choice), true);
        assert_eq!(next.microphone, None);
        assert_eq!(next.hotkey, Settings::default().hotkey);
        assert_eq!(next.hotkey_mode, Settings::default().hotkey_mode);
        assert_eq!(next.setup_profile, Some(SetupProfile::Balanced));
        assert_eq!(next.asr_model, "speech");
        assert_eq!(next.cleanup_mode, old.cleanup_mode);
    }
    #[test]
    fn interrupted_settings_save_preserves_unfinished_marker_and_queue_on_disk() {
        let root =
            std::env::temp_dir().join(format!("sonojot-onboarding-{}", uuid::Uuid::new_v4()));
        let store = store::Store::open(Some(root.clone())).unwrap();
        let mut edited = Settings::default();
        edited.microphone = Some("edited-microphone".into());
        edited.hotkey = "Alt+Shift+D".into();
        let choice = crate::profiles::Selection {
            asr_model: "whisper-small".into(),
            cleanup_model: "local-rules".into(),
            notes_model: "lfm2.5-1.2b".into(),
            reasons: vec![],
        };
        let next = chosen_settings(&edited, SetupProfile::Balanced, Some(&choice), true);
        let mut progress = SetupProgress {
            seen: false,
            profile: Some(SetupProfile::Balanced),
            downloads: download_ids(&next),
        };
        persist_choice(&store, &progress, &next).unwrap();
        // A crash between settings and completion writes keeps setup unfinished.
        let saved = store.load_settings().unwrap();
        assert!(required(&saved, read(&root).unwrap().as_ref()));
        assert_eq!(saved.microphone, None);
        assert_eq!(saved.hotkey, Settings::default().hotkey);
        progress.seen = true;
        write(&root, &progress).unwrap();
        assert!(!required(&saved, read(&root).unwrap().as_ref()));
        assert_eq!(
            read(&root).unwrap().unwrap().downloads[1],
            "sonojot-punct-v2"
        );
        // Later Custom selection cannot remove the independent punctuation job.
        let mut custom = saved.clone();
        custom.setup_profile = Some(SetupProfile::Custom);
        store.save_settings(&custom).unwrap();
        cancel_at(&root, &saved.asr_model).unwrap();
        assert_eq!(
            read(&root).unwrap().unwrap().downloads[0],
            "sonojot-punct-v2"
        );
        cancel_at(&root, "sonojot-punct-v2").unwrap();
        assert!(!read(&root)
            .unwrap()
            .unwrap()
            .downloads
            .contains(&"sonojot-punct-v2".into()));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_settings_write_never_marks_setup_done() {
        let root = std::env::temp_dir().join(format!(
            "sonojot-onboarding-failure-{}",
            uuid::Uuid::new_v4()
        ));
        let store = store::Store::open(Some(root.clone())).unwrap();
        std::fs::create_dir(root.join("settings.json")).unwrap();
        let progress = SetupProgress {
            seen: false,
            profile: Some(SetupProfile::Balanced),
            downloads: vec!["sonojot-punct-v2".into()],
        };
        assert!(persist_choice(&store, &progress, &Settings::default()).is_err());
        assert!(!read(&root).unwrap().unwrap().seen);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
