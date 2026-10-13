//! Versioned, local-only acknowledgments checked before download work starts.
use crate::{
    app::{lock, Runtime},
    domain::{Settings, Snapshot},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};
use tauri::{AppHandle, State};

pub(crate) const SHERPA_HOLD: &str = "Sherpa CUDA runtime downloads are paused until an ASR-only build without GPL text-to-speech libraries is available. Use a CPU speech model, Qwen3-ASR Q8 CUDA, or an already installed runtime.";

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub label: String,
    pub url: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Component {
    pub name: String,
    pub version: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub version: String,
    pub models: Vec<String>,
    pub platform: String,
    pub title: String,
    pub summary: String,
    pub checkbox: String,
    pub components: Vec<Component>,
    pub links: Vec<Link>,
}
#[derive(Deserialize)]
struct Catalog {
    groups: Vec<Group>,
}

fn catalog() -> Result<&'static Catalog, String> {
    static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../legal/download-consent.json"))
                .map_err(|e| format!("Cannot read download license information: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}
fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "other"
    }
}
fn applies(group: &Group, ids: &[&str], platform: &str) -> bool {
    (group.platform == "all" || group.platform == platform)
        && ids
            .iter()
            .any(|id| group.models.iter().any(|model| model == id))
}
fn pending<'a>(
    settings: &Settings,
    ids: &[&str],
    platform: &str,
    groups: &'a [Group],
) -> Vec<&'a Group> {
    groups
        .iter()
        .filter(|g| {
            applies(g, ids, platform)
                && settings.component_acknowledgments.get(&g.id) != Some(&g.version)
        })
        .collect()
}

pub(crate) fn require_sherpa_runtime(root: &Path, id: &str) -> Result<(), String> {
    if crate::gpu_asr::is_gpu_model(id) && !crate::gpu_asr::gpu_runtime_ready(root) {
        return Err(SHERPA_HOLD.into());
    }
    Ok(())
}

pub(crate) fn require_components(runtime: &Runtime, ids: &[&str]) -> Result<(), String> {
    for id in ids {
        require_sherpa_runtime(&runtime.models_root, id)?;
    }
    let settings = runtime.snapshot()?.settings;
    crate::onboarding::require_terms(&settings)?;
    if let Some(group) = pending(&settings, ids, platform(), &catalog()?.groups).first() {
        return Err(format!(
            "LICENSE_ACKNOWLEDGMENT_REQUIRED: Review {} in Models before downloading.",
            group.title
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn get_download_requirements(
    state: State<'_, Arc<Runtime>>,
    ids: Vec<String>,
) -> Result<Vec<Group>, String> {
    let settings = state.snapshot()?.settings;
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    Ok(pending(&settings, &ids, platform(), &catalog()?.groups)
        .into_iter()
        .cloned()
        .collect())
}

#[derive(Deserialize)]
pub struct Acknowledgment {
    id: String,
    version: String,
}

#[tauri::command]
pub async fn acknowledge_download_terms(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    acknowledgments: Vec<Acknowledgment>,
) -> Result<Snapshot, String> {
    if acknowledgments.is_empty() {
        return Err("Choose the license terms to acknowledge.".into());
    }
    let _settings = state.settings_gate.lock().await;
    let _capture = lock(&state.control)?;
    let mut settings = state.snapshot()?.settings;
    for acknowledgment in acknowledgments {
        let group = catalog()?
            .groups
            .iter()
            .find(|g| g.id == acknowledgment.id && g.version == acknowledgment.version)
            .ok_or("These license terms have changed. Review them again before downloading.")?;
        if group.platform != "all" && group.platform != platform() {
            return Err("These terms are for another platform.".into());
        }
        settings
            .component_acknowledgments
            .insert(group.id.clone(), group.version.clone());
    }
    state.store.save_settings(&settings)?;
    lock(&state.snapshot)?.settings = settings;
    drop(_capture);
    state.emit(&app);
    state.snapshot()
}

#[tauri::command]
pub async fn open_download_license(group: String, index: usize) -> Result<(), String> {
    let link = catalog()?
        .groups
        .iter()
        .find(|g| g.id == group)
        .and_then(|g| g.links.get(index))
        .ok_or("Unknown license link.")?;
    crate::legal_links::open_https(link.url.clone(), "license").await
}

// Keep old IPC names guarded without changing the protected installers.
#[tauri::command(rename = "install_asr_model")]
pub async fn consent_install_asr_model(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    id: String,
) -> Result<crate::setup::SetupStatus, String> {
    require_components(&state, &[&id])?;
    crate::optional_asr::install_asr_model(app, state, id).await
}
#[tauri::command(rename = "install_text_model")]
pub async fn consent_install_text_model(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    id: String,
) -> Result<crate::setup::SetupStatus, String> {
    require_components(&state, &[&id])?;
    crate::optional_llm::install_text_model(app, state, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_settings_require_terms_and_changed_component_pins_reprompt() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.terms_accepted_version, 0);
        assert_eq!(settings.terms_accepted_at, None);
        assert!(!settings.meeting_consent_acknowledged);
        let groups = &catalog().unwrap().groups;
        let ids = ["granite-speech-4.1-2b-nar-q8"];
        let group = pending(&settings, &ids, "windows", groups)[0];
        settings
            .component_acknowledgments
            .insert(group.id.clone(), group.version.clone());
        assert!(pending(&settings, &ids, "windows", groups).is_empty());
        settings
            .component_acknowledgments
            .insert(group.id.clone(), "old-pin".into());
        assert_eq!(pending(&settings, &ids, "windows", groups).len(), 1);
        assert!(pending(&settings, &ids, "macos", groups).is_empty());
    }
    #[test]
    fn every_restricted_group_has_versioned_https_links_and_distinct_acknowledgment() {
        let mut ids = std::collections::HashSet::new();
        for group in &catalog().unwrap().groups {
            assert!(ids.insert(&group.id));
            assert_eq!(group.version.len(), 64);
            assert!(!group.checkbox.is_empty());
            for link in &group.links {
                assert_eq!(reqwest::Url::parse(&link.url).unwrap().scheme(), "https");
            }
        }
    }
    #[test]
    fn missing_sherpa_cuda_is_blocked_but_cpu_and_llama_paths_remain_available() {
        let root = std::env::temp_dir().join("sonojot-compliance-empty-models");
        assert!(require_sherpa_runtime(&root, "whisper-tiny-fp32-cuda")
            .unwrap_err()
            .contains("ASR-only"));
        for id in [
            "qwen3-asr-1.7b-q8-cuda",
            "qwen3-asr-1.7b-q4-cpu",
            "moonshine-medium",
        ] {
            require_sherpa_runtime(&root, id).unwrap();
        }
    }
}
