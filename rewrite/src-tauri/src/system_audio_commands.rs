//! System audio readiness for meetings, reported truthfully. Asking for the
//! macOS permission goes through the shared helper (`permissions::SPECS`).
use crate::{
    app::{lock, Runtime},
    system_audio::{self, SystemAudioStatus},
};
use std::sync::Arc;
use tauri::{AppHandle, State};

/// Re-reads the OS state and publishes it. When system audio first works on a
/// Mac, an old forced "Microphone only" becomes the both-sides default once.
pub(crate) async fn refresh(runtime: &Runtime, app: &AppHandle) -> Result<SystemAudioStatus, String> {
    let status = tauri::async_runtime::spawn_blocking(system_audio::status)
        .await
        .map_err(|error| error.to_string())?;
    let changed = {
        let mut snapshot = lock(&runtime.snapshot)?;
        let changed = snapshot.capabilities.system_audio_status != status;
        snapshot.capabilities.system_audio_status = status;
        snapshot.capabilities.system_audio = status == SystemAudioStatus::Ready;
        changed
    };
    let defaulted = if status == SystemAudioStatus::Ready {
        let _gate = runtime.settings_gate.lock().await;
        apply_default(runtime)?
    } else {
        false
    };
    if changed || defaulted {
        runtime.emit(app);
    }
    Ok(status)
}

fn apply_default(runtime: &Runtime) -> Result<bool, String> {
    let snapshot = runtime.snapshot()?;
    if snapshot.settings.system_audio_default_applied || snapshot.meeting_recording {
        return Ok(false);
    }
    let mut settings = snapshot.settings;
    settings.system_audio_default_applied = true;
    settings.meeting_source = default_source(&settings.meeting_source, cfg!(target_os = "macos")).into();
    runtime.store.save_settings(&settings)?;
    lock(&runtime.snapshot)?.settings = settings;
    Ok(true)
}

/// Macs could not record system audio before, so their "microphone" choice
/// was forced, not chosen. Windows choices were always deliberate.
fn default_source(saved: &str, mac: bool) -> &str {
    if mac && saved == "microphone" {
        "both"
    } else {
        saved
    }
}

#[tauri::command]
pub async fn system_audio_status(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
) -> Result<SystemAudioStatus, String> {
    refresh(state.inner(), &app).await
}

#[cfg(test)]
mod tests {
    use super::default_source;

    #[test]
    fn only_a_mac_forced_microphone_becomes_both_sides() {
        assert_eq!(default_source("microphone", true), "both");
        assert_eq!(default_source("system", true), "system");
        assert_eq!(default_source("both", true), "both");
        for source in ["microphone", "system", "both"] {
            assert_eq!(default_source(source, false), source);
        }
    }
}
