use crate::{
    app::{lock, Runtime},
    asr, audio, cleanup,
    domain::{AppState, Settings, SetupProfile, Snapshot},
    setup::{SetupCapability, SetupState, SetupStatus},
    store,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};

type RuntimeState<'a> = State<'a, Arc<Runtime>>;

#[derive(Clone, Copy)]
pub(crate) enum CaptureKind {
    Dictation,
    Meeting,
    MicrophoneTest,
}

pub(crate) fn require_setup(settings: &Settings, kind: CaptureKind) -> Result<(), String> {
    crate::onboarding::require_terms(settings)?;
    if settings.setup_profile.is_some() || matches!(kind, CaptureKind::MicrophoneTest) {
        return Ok(());
    }
    Err(match kind {
        CaptureKind::Dictation => "SETUP_INCOMPLETE: Finish setup to start dictating.",
        CaptureKind::Meeting => "SETUP_INCOMPLETE: Finish setup to record meetings.",
        CaptureKind::MicrophoneTest => unreachable!("Microphone tests do not require setup"),
    }.into())
}

#[tauri::command]
pub async fn get_hardware_profile(state: RuntimeState<'_>) -> Result<crate::profiles::HardwareProfiles, String> {
    let root = state.models_root.clone();
    tauri::async_runtime::spawn_blocking(move || crate::profiles::inspect(&root))
        .await
        .map_err(|error| error.to_string())
}

/// Only this explicit Setup action chooses recommended models. Notes can finish
/// downloading after the commit; ordinary Custom selection still requires files.
#[tauri::command]
pub async fn apply_setup_profile(
    app: AppHandle,
    state: RuntimeState<'_>,
    profile: SetupProfile,
) -> Result<Snapshot, String> {
    let _settings = state.settings_gate.lock().await;
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selection = if profile == SetupProfile::Custom {
            None
        } else {
            Some(crate::profiles::recommend(&crate::profiles::inspect(&runtime.models_root).hardware, profile))
        };
        let control = lock(&runtime.control)?;
        let snapshot = runtime.snapshot()?;
        if !matches!(snapshot.state, AppState::Idle | AppState::Error) || snapshot.meeting_recording {
            return Err("Finish recording before changing models.".into());
        }
        let old = snapshot.settings;
        let next = profile_settings(&old, profile, selection.as_ref());
        next.validate()?;
        let models_changed = old.asr_model != next.asr_model
            || old.cleanup_model != next.cleanup_model
            || old.notes_model != next.notes_model;
        crate::resource_manager::with_model_selection(models_changed, || {
            // Final readiness check shares install/remove ownership with commit.
            // Changing an established profile to Custom changes visibility only.
            if (old.setup_profile.is_none() || profile != SetupProfile::Custom)
                && !asr::model_ready(&runtime.models_root, &next.asr_model)
            {
                return Err("Install the speech model before finishing setup.".into());
            }
            // Only the selected speech model was synchronously verified.
            // Catalog entries enqueue misses and retain their Checking state.
            let models = crate::app::model_catalog(&runtime.models_root);
            runtime.store.save_settings(&next)?;
            let mut snapshot = lock(&runtime.snapshot)?;
            snapshot.settings = next.clone();
            snapshot.models = models;
            snapshot.error = None;
            Ok(())
        })?;
        crate::resource_manager::configure(&next);
        drop(control);
        // The profile is authoritative. Preserve the legacy routing marker as
        // well; a marker write failure must not pretend the settings rolled back.
        if let Err(error) = mark_seen(&runtime) {
            runtime.report(&app, error);
        }
        runtime.emit(&app);
        runtime.snapshot()
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) fn profile_settings(
    old: &Settings,
    profile: SetupProfile,
    selection: Option<&crate::profiles::Selection>,
) -> Settings {
    let mut next = old.clone();
    next.setup_profile = Some(profile);
    if old.setup_profile.is_none() && profile != SetupProfile::Custom {
        // First-run automatic setup already includes the punctuation package.
        next.punctuation_recommendation_acknowledged = true;
        next.punctuation_v2_recommendation_acknowledged = true;
    }
    if let Some(selection) = selection {
        next.asr_model = selection.asr_model.clone();
        next.cleanup_model = selection.cleanup_model.clone();
        next.notes_model = selection.notes_model.clone();
    }
    next
}

#[cfg(test)]
mod setup_profile_tests {
    use super::*;

    #[test]
    fn every_capture_kind_requires_current_terms_even_with_a_saved_profile() {
        for kind in [CaptureKind::Dictation, CaptureKind::Meeting, CaptureKind::MicrophoneTest] {
            let mut settings = Settings {
                setup_profile: Some(SetupProfile::Custom),
                ..Settings::default()
            };
            assert!(require_setup(&settings, kind).unwrap_err().starts_with("TERMS_REQUIRED:"));
            settings.terms_accepted_version = crate::onboarding::TERMS_VERSION;
            assert!(require_setup(&settings, kind).is_err());
            settings.terms_accepted_at = Some("2026-10-12T12:00:00.000Z".into());
            assert!(require_setup(&settings, kind).is_ok());
            settings.terms_accepted_version = crate::onboarding::TERMS_VERSION - 1;
            assert!(require_setup(&settings, kind).is_err());
        }
    }

    #[test]
    fn accepted_terms_allow_the_microphone_check_but_do_not_complete_setup() {
        let settings = Settings {
            terms_accepted_version: crate::onboarding::TERMS_VERSION,
            terms_accepted_at: Some("2026-10-12T12:00:00.000Z".into()),
            ..Settings::default()
        };
        assert!(require_setup(&settings, CaptureKind::MicrophoneTest).is_ok());
        for kind in [CaptureKind::Dictation, CaptureKind::Meeting] {
            assert!(require_setup(&settings, kind).unwrap_err().starts_with("SETUP_INCOMPLETE:"));
        }
    }

    #[test]
    fn presets_disable_smart_cleanup_without_changing_cleanup_mode() {
        let old = Settings { setup_profile: Some(SetupProfile::Custom),
            cleanup_model: "speakoflow-mini-q4_k_m".into(), cleanup_mode: crate::domain::CleanupMode::Format,
            ..Settings::default() };
        let choice = crate::profiles::Selection { asr_model: old.asr_model.clone(),
            cleanup_model: "local-rules".into(), notes_model: old.notes_model.clone(), reasons: Vec::new() };
        for profile in [SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light] {
            let next = profile_settings(&old, profile, Some(&choice));
            assert_eq!(next.cleanup_model, "local-rules");
            assert_eq!(next.cleanup_mode, old.cleanup_mode);
            next.validate().unwrap();
        }
        assert_eq!(profile_settings(&old, SetupProfile::Custom, None), old);
    }

    #[test]
    fn custom_preserves_every_model_and_dictation_choice() {
        let old = Settings {
            asr_model: "whisper-large-v3-turbo-fp32-cuda".into(),
            cleanup_model: "lfm2.5-1.2b".into(),
            notes_model: "openrouter".into(),
            cleanup_mode: crate::domain::CleanupMode::Original,
            ..Settings::default()
        };
        let mut expected = old.clone();
        expected.setup_profile = Some(SetupProfile::Custom);
        assert_eq!(profile_settings(&old, SetupProfile::Custom, None), expected);
    }

    #[test]
    fn first_automatic_profile_acknowledges_its_included_punctuation_download() {
        let old = Settings {
            cleanup_mode: crate::domain::CleanupMode::Format,
            ..Settings::default()
        };
        let choice = crate::profiles::Selection {
            asr_model: "moonshine-small".into(),
            cleanup_model: "local-rules".into(),
            notes_model: "lfm2.5-1.2b".into(),
            reasons: Vec::new(),
        };
        for profile in [SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light] {
            let next = profile_settings(&old, profile, Some(&choice));
            let mut expected = old.clone();
            expected.setup_profile = Some(profile);
            expected.punctuation_recommendation_acknowledged = true;
            expected.punctuation_v2_recommendation_acknowledged = true;
            expected.asr_model = choice.asr_model.clone();
            expected.cleanup_model = choice.cleanup_model.clone();
            expected.notes_model = choice.notes_model.clone();
            assert_eq!(next, expected);
        }
        assert_eq!(old.setup_profile, None);
    }

    #[test]
    fn existing_profiles_preserve_punctuation_recommendation_acknowledgment() {
        for saved in [SetupProfile::Custom, SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light] {
            for acknowledged in [false, true] {
                for v2_acknowledged in [false, true] {
                    let old = Settings { setup_profile: Some(saved),
                        punctuation_recommendation_acknowledged: acknowledged,
                        punctuation_v2_recommendation_acknowledged: v2_acknowledged, ..Settings::default() };
                    for chosen in [SetupProfile::Custom, SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light] {
                        let next = profile_settings(&old, chosen, None);
                        assert_eq!(next.punctuation_recommendation_acknowledged, acknowledged);
                        assert_eq!(next.punctuation_v2_recommendation_acknowledged, v2_acknowledged);
                    }
                }
            }
        }
    }
}

fn capability(state: SetupState, detail: &str) -> SetupCapability {
    SetupCapability {
        state,
        detail: Some(detail.into()),
    }
}

pub fn converter_path(runtime: &Runtime) -> Option<PathBuf> {
    let configured = std::fs::read(runtime.store.root().join("converter.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PathBuf>(&bytes).ok());
    configured.filter(|path| path.is_file()).or_else(|| {
        let executable = std::env::current_exe().ok()?;
        let path = executable.parent()?.join(if cfg!(windows) {
            "tools/ffmpeg.exe"
        } else {
            "tools/ffmpeg"
        });
        path.is_file().then_some(path)
    })
}

/// Whether an import would find a converter: the located or installed one,
/// `SONOJOT_FFMPEG`, the app's own tools folder, or `ffmpeg` on PATH.
pub fn converter_available(runtime: &Runtime) -> bool {
    crate::meetings::resolve_ffmpeg(&runtime.store, converter_path(runtime).as_deref()).is_ok()
}

pub fn inspect(runtime: &Runtime) -> Result<SetupStatus, String> {
    use crate::model_verification::{Check, Status};
    let snapshot = runtime.snapshot()?;
    let devices = audio::list_devices();
    let microphone_error = devices.as_ref().err();
    let devices = devices.as_ref().map(Vec::as_slice).unwrap_or_default();
    let has_microphone = snapshot.settings.microphone.as_ref().map_or_else(
        || devices.iter().any(|device| device.is_default),
        |id| devices.iter().any(|device| &device.id == id),
    );
    let selected = &snapshot.settings.asr_model;
    let selected_status = asr::model_status(&runtime.models_root, selected, Check::Background);
    let speech_status = Status::any([
        selected_status,
        if selected == "moonshine-medium" {
            asr::model_status(&runtime.models_root, "moonshine-small", Check::Background)
        } else { Status::Missing },
    ]);
    let speech = if speech_status == Status::Checking {
        capability(SetupState::Checking, "Checking speech model files.")
    } else if !speech_status.ready() {
        if crate::optional_asr::is_optional(selected)
            || crate::whisper_catalog::is_whisper(selected)
            || crate::llama_asr::is_llama_asr(selected)
        {
            capability(
                SetupState::Missing,
                "Install this speech model to download its files. They load when dictation starts.",
            )
        } else {
            capability(
                SetupState::Missing,
                "Locate an existing speech model folder.",
            )
        }
    } else if matches!(
        snapshot.state,
        AppState::Loading | AppState::Listening | AppState::Finishing | AppState::Cleaning
    ) {
        if snapshot.interim_text.is_empty() {
            capability(SetupState::Loading, "Listening; preparing transcription.")
        } else {
            capability(
                SetupState::Ready,
                "The speech model is transcribing this session.",
            )
        }
    } else if snapshot.state == AppState::Error
        && snapshot.error.as_ref().is_some_and(|e| {
            e.contains("Moonshine") || e.contains("recogniz") || e.contains("speech model")
        })
    {
        capability(
            SetupState::Failed,
            "The speech model needs attention. Locate a verified model folder.",
        )
    } else {
        capability(
            SetupState::Installed,
            "Speech files are present. A phrase test checks that they can run.",
        )
    };
    Ok(SetupStatus {
        microphone: if let Some(error) = microphone_error {
            capability(SetupState::Failed, error)
        } else if has_microphone {
            capability(
                if snapshot.state == AppState::Listening
                    || (snapshot.meeting_recording && snapshot.settings.meeting_source != "system")
                {
                    SetupState::Ready
                } else {
                    SetupState::Installed
                },
                "Your selected microphone is available.",
            )
        } else {
            capability(SetupState::Missing, "Choose an available microphone.")
        },
        speech,
        shortcut: if snapshot.capabilities.global_shortcut {
            capability(SetupState::Ready, "Your shortcut is registered.")
        } else {
            capability(
                SetupState::Failed,
                "Choose a shortcut that another app is not using.",
            )
        },
        cleanup: match Status::any(crate::optional_llm::catalog().iter()
            .map(|model| cleanup::model_status(&runtime.models_root, model.id, Check::Background))) {
            Status::Ready => capability(
                SetupState::Installed,
                "Optional local text models are installed. Local formatting works without them.",
            ),
            Status::Checking => capability(SetupState::Checking, "Checking local text model files."),
            Status::Missing => capability(
                SetupState::Missing,
                "Optional. SonoJot Punctuation and your vocabulary remain available.",
            ),
        },
        media_import: if converter_available(runtime) {
            capability(
                SetupState::Installed,
                "The optional media converter is available.",
            )
        } else if crate::media_support::installing() {
            capability(SetupState::Loading, "Installing media support.")
        } else if crate::media_support::AUTOMATIC {
            capability(
                SetupState::Missing,
                "Optional. Install media support or locate FFmpeg to convert imported recordings.",
            )
        } else {
            capability(
                SetupState::Unsupported,
                "Automatic install is unavailable on this system. Locate an existing FFmpeg to convert imported recordings.",
            )
        },
        models_path: asr::model_display_directory(
            &runtime.models_root,
            if selected_status != Status::Missing {
                selected
            } else {
                "moonshine-small"
            },
        )
        .to_string_lossy()
        .into_owned(),
    })
}

pub fn needs_first_setup(runtime: &Runtime) -> bool {
    // First-run routing must not verify model weights on the UI thread.
    // Setup inspects capabilities asynchronously once its window is open.
    crate::onboarding::needs_setup(runtime)
}

fn mark_seen(runtime: &Runtime) -> Result<(), String> {
    crate::onboarding::mark_seen(runtime)
}

#[tauri::command]
pub fn complete_setup(state: RuntimeState<'_>) -> Result<(), String> {
    mark_seen(&state)
}

#[tauri::command]
pub async fn get_setup(state: RuntimeState<'_>) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inspect(&runtime))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn require_idle(snapshot: &Snapshot) -> Result<(), String> {
    if !matches!(snapshot.state, AppState::Idle | AppState::Error) || snapshot.meeting_recording {
        return Err("Finish recording before locating models or the converter.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn locate_models(
    app: AppHandle,
    state: RuntimeState<'_>,
    path: String,
) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        require_idle(&runtime.snapshot()?)?;
        let verified = crate::setup::verify_models_folder(Path::new(&path))?;
        // Verification does no mutation. Recheck capture ownership before committing one pointer file.
        let _capture = lock(&runtime.control)?;
        require_idle(&runtime.snapshot()?)?;
        std::fs::create_dir_all(&runtime.models_root).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(&verified.paths_json()).map_err(|e| e.to_string())?;
        store::atomic_write(&runtime.models_root.join("paths.json"), &bytes)?;
        runtime.engine.refresh_models();
        let models = crate::app::model_catalog(&runtime.models_root);
        {
            let mut snapshot = lock(&runtime.snapshot)?;
            snapshot.models = models;
            snapshot.error = None;
        }
        drop(_capture);
        runtime.emit(&app);
        inspect(&runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn locate_converter(
    state: RuntimeState<'_>,
    path: String,
) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        require_idle(&runtime.snapshot()?)?;
        let path =
            std::fs::canonicalize(&path).map_err(|e| format!("Cannot open the converter: {e}"))?;
        if !path.is_file()
            || !path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .eq_ignore_ascii_case(if cfg!(windows) {
                        "ffmpeg.exe"
                    } else {
                        "ffmpeg"
                    })
            })
        {
            return Err("Choose the FFmpeg application file.".into());
        }
        let _capture = lock(&runtime.control)?;
        require_idle(&runtime.snapshot()?)?;
        let bytes = serde_json::to_vec(&path).map_err(|e| e.to_string())?;
        store::atomic_write(&runtime.store.root().join("converter.json"), &bytes)?;
        drop(_capture);
        inspect(&runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}
