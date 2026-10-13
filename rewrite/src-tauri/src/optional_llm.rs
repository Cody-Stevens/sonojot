//! Optional downloadable GGUF text models. Nothing here loads at startup or
//! without an explicit Install action. Files land under the models pointer
//! folder, separately from the located Moonshine bundle, and are deleted on
//! Uninstall. llama-server still comes from that bundle and is started only
//! when cleanup, notes, or vocabulary review need a local model.
use crate::{
    app::Runtime,
    optional_asr::{self, HostClass, PinnedFile},
    setup::SetupStatus,
    setup_commands::require_idle,
    store,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};

#[derive(Clone, Copy)]
pub struct OptionalTextModel {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub file: PinnedFile,
}

pub const SMART_CLEANUP_ID: &str = "speakoflow-mini-q4_k_m";
const SPEAKOFLOW: OptionalTextModel = OptionalTextModel {
    id: SMART_CLEANUP_ID,
    name: "Smart cleanup (experimental)",
    description: "Small cleanup model; may miss or occasionally change wording. Numbers, names and links are checked.",
    repo: "SpeakoFlow/speakoflow-mini",
    revision: "835431771f72820251fe6c6b4b07f12b000e2647",
    file: PinnedFile {
        relative: "SpeakoFlow-Mini-0.8B-Q4_K_M.gguf",
        bytes: 541_903_616,
        sha256: "530e0a5d4e9314e3bae2c92aecf3ab39ca48e8cfecac41372058f9fcb742329d",
    },
};

const LFM_12: OptionalTextModel = OptionalTextModel {
    id: "lfm2.5-1.2b",
    name: "LFM2.5-1.2B Instruct",
    description: "Live cleanup · Q4_K_M",
    repo: "LiquidAI/LFM2.5-1.2B-Instruct-GGUF",
    revision: "6767265158422fb8a19c62ceb45f16f05363615b",
    file: PinnedFile {
        relative: "LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
        bytes: 730_895_168,
        sha256: "b1b3de114215d9507409a662a501a631095a479a419584e8a2ded6304b19b4f5",
    },
};

const LFM_26: OptionalTextModel = OptionalTextModel {
    id: "lfm2.5-2.6b",
    name: "LFM2.5-2.6B",
    description: "Live cleanup · Q4_K_M",
    repo: "LiquidAI/LFM2.5-2.6B-GGUF",
    revision: "84022ce711b28455e8c4fc364ce68c00cf995875",
    file: PinnedFile {
        relative: "LFM2.5-2.6B-Q4_K_M.gguf",
        bytes: 1_674_455_040,
        sha256: "02a8b7e17487d326e46d68ce0ba24211e1b80a14c4cd0597fa73c1cd697f52ed",
    },
};

const QWEN: OptionalTextModel = OptionalTextModel {
    id: "qwen3-4b-instruct-2507",
    name: "Qwen3-4B-Instruct-2507",
    description: "Live cleanup, vocabulary review, and meeting notes · Q4_K_M",
    repo: "bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF",
    revision: "ae44f08e1392f39c0e474af10c3ff8355c8b6688",
    file: PinnedFile {
        relative: "Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        bytes: 2_497_280_736,
        sha256: "2fde00ce69dd4899c70d020845e2638353015bba0fdf161b3eb965f2bca4464e",
    },
};

pub const BONSAI_ID: &str = "ternary-bonsai-2-27b";
pub const BONSAI_RUNTIME_URL: &str = "https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10683-d8f26ee/llama-prism-b10683-d8f26ee-bin-win-cuda-13.3-x64.zip";
pub const BONSAI_RUNTIME_BYTES: u64 = 144_929_011;
pub const BONSAI_RUNTIME_SHA256: &str =
    "31bbd608376473d42ac985069975134cc6638b8f51bdf065dac07e448399daa9";
pub const BONSAI_CUDA_URL: &str = "https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10683-d8f26ee/cudart-llama-bin-win-cuda-13.3-x64.zip";
pub const BONSAI_CUDA_BYTES: u64 = 390_970_417;
pub const BONSAI_CUDA_SHA256: &str =
    "1462a050eb4c684921ba51dcc4cc488a036674c3e73e9945ee705b854808d03e";
const BONSAI: OptionalTextModel = OptionalTextModel {
    id: BONSAI_ID,
    name: "Ternary Bonsai 2 27B",
    description: "PrismML fork required · PQ2_0 · preview",
    repo: "prism-ml/Ternary-Bonsai-2-27B-gguf",
    revision: "6ed5e12bf84b7a63069882c91dd9e9218647d17b",
    file: PinnedFile {
        relative: "Ternary-Bonsai-2-27B-PQ2_0.gguf",
        bytes: 7_206_168_928,
        sha256: "3907dc1658db1f78a9826bf8d5bcb8dc65db0d466388937af57f2294fae62ec1",
    },
};

pub const MODELS: &[OptionalTextModel] = &[LFM_12, LFM_26, QWEN, BONSAI, SPEAKOFLOW];
pub const NOTES_MODEL_ID: &str = QWEN.id;

/// Honest static metadata for the Round 6 model library. No measured
/// rankings live here; memory stays unmeasured until native observation.
/// These helpers keep family/quantization/context in one place so
/// model_manager.rs does not re-declare pinned facts.
pub fn family_of(id: &str) -> &'static str {
    if id == SMART_CLEANUP_ID {
        "SpeakoFlow"
    } else if id.starts_with("lfm") {
        "LFM"
    } else if id.starts_with("qwen") {
        "Qwen3"
    } else if is_bonsai(id) {
        "Bonsai"
    } else {
        "Text"
    }
}

pub fn quantization_of(_id: &str) -> &'static str {
    if is_bonsai(_id) {
        "PQ2_0"
    } else {
        "Q4_K_M"
    }
}

pub fn context_tokens_of(_id: &str) -> Option<u32> {
    Some(if _id == BONSAI_ID { 8192 } else { 4096 })
}

pub fn revision_of(id: &str) -> Option<&'static str> {
    find(id).map(|model| model.revision)
}

pub fn backend_desc() -> &'static str {
    "llama-server (cpu; cuda when NVIDIA runtime present)"
}

pub fn compatibility_desc() -> &'static str {
    "Windows x64 CPU; NVIDIA CUDA when the CUDA runtime is present and text acceleration is auto. AMD graphics not supported."
}

pub fn roles_of(id: &str) -> &'static [&'static str] {
    if id == SMART_CLEANUP_ID { &["cleanup"] } else { &["cleanup", "notes"] }
}

pub fn catalog() -> &'static [OptionalTextModel] {
    MODELS
}

pub fn find(id: &str) -> Option<&'static OptionalTextModel> {
    MODELS.iter().find(|model| model.id == id)
}

pub fn model_dir(models_root: &Path, id: &str) -> PathBuf {
    optional_asr::optional_root(models_root).join(id)
}

pub fn gguf_path(models_root: &Path, model: &OptionalTextModel) -> PathBuf {
    model_dir(models_root, model.id).join(model.file.relative)
}

pub fn package_bytes(model: &OptionalTextModel) -> u64 {
    model.file.bytes
}

pub fn optional_copy_ready(models_root: &Path, id: &str) -> bool {
    optional_copy_status(models_root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn optional_copy_status(models_root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    let Some(model) = find(id) else {
        return crate::model_verification::Status::Missing;
    };
    check.file(models_root, &gguf_path(models_root, model), model.file.bytes, model.file.sha256)
}

#[tauri::command]
pub async fn install_text_model(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    id: String,
) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        install(&runtime, &id)?;
        refresh_snapshot(&app, &runtime)?;
        crate::setup_commands::inspect(&runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[deprecated(
    note = "Retired: Setup deletes only via model_manager::remove_model { confirmed:true }. Kept for compat; fails closed without touching files."
)]
pub async fn uninstall_text_model(
    app: AppHandle,
    state: State<'_, Arc<Runtime>>,
    id: String,
) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        uninstall(&runtime, &id)?;
        refresh_snapshot(&app, &runtime)?;
        crate::setup_commands::inspect(&runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn refresh_snapshot(app: &AppHandle, runtime: &Runtime) -> Result<(), String> {
    let models = crate::app::model_catalog(&runtime.models_root);
    {
        let mut snapshot = crate::app::lock(&runtime.snapshot)?;
        snapshot.models = models;
        snapshot.error = None;
    }
    runtime.emit(app);
    Ok(())
}

pub fn is_bonsai(id: &str) -> bool {
    id == BONSAI_ID
}
pub fn bonsai_runtime_path(models_root: &Path) -> PathBuf {
    optional_asr::optional_root(models_root)
        .join("bonsai-prism-runtime")
        .join(if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        })
}
pub fn bonsai_runtime_ready(models_root: &Path) -> bool {
    bonsai_runtime_path(models_root).is_file()
}
pub fn installable(models_root: &Path, id: &str) -> bool {
    let _ = models_root;
    !id.is_empty()
}

fn install(runtime: &Runtime, id: &str) -> Result<(), String> {
    let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Models)?;
    // Downloads never need runtime fit and never stop active work (same rule
    // as ASR installs). Shared mutation guard only; no idle/busy gate.
    let _serialized = optional_asr::try_install_gate()?;
    let _ = runtime.snapshot().map(|_| ());
    install_platform(runtime, id)
}

fn uninstall(runtime: &Runtime, id: &str) -> Result<(), String> {
    // Legacy unconfirmed path: fail-closed like the ASR legacy path. Deletes
    // must use `model_manager::remove_model { confirmed:true }`. Parent
    // removes this handler after UI migration; until then no bypass.
    let _ = (runtime, id);
    Err("Confirm removal in Setup before deleting model files. Use the guarded model library removal with confirmation; nothing was deleted.".into())
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_platform(_runtime: &Runtime, _id: &str) -> Result<(), String> {
    Err("Automatic text-model install is available only on 64-bit Windows.".into())
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn uninstall_platform(_runtime: &Runtime, _id: &str) -> Result<(), String> {
    Err("Automatic text-model install is available only on 64-bit Windows.".into())
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn install_platform(runtime: &Runtime, id: &str) -> Result<(), String> {
    let model = find(id).ok_or("Choose a downloadable text model.")?;
    if optional_copy_ready(&runtime.models_root, model.id) {
        return Ok(());
    }
    let root = optional_asr::optional_root(&runtime.models_root);
    fs::create_dir_all(&root)
        .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
    optional_asr::ensure_within(&runtime.models_root, &root)?;
    let dest = model_dir(&runtime.models_root, model.id);
    // Preserve any existing install until the new staging verifies; delete
    // only via transactional activation with rollback just before rename.
    // A failed/cancelled download must leave the old ready model untouched.
    let staging = root.join(format!(".install-{}-{}", model.id, store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
    let installed = (|| {
        let path = staging.join(model.file.relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot prepare {}: {e}", model.file.relative))?;
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            model.repo, model.revision, model.file.relative
        );
        optional_asr::download(
            &url,
            &path,
            model.file.bytes,
            model.file.sha256,
            HostClass::HuggingFace,
            "text model",
        )?;
        if !optional_asr::file_matches(&path, &model.file)? {
            return Err(format!(
                "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                model.file.relative
            ));
        }
        if dest.exists() && optional_copy_ready(&runtime.models_root, model.id) {
            return Err("This text model is already installed.".into());
        }
        optional_asr::transactional_activate(&runtime.models_root, &staging, &dest, "text model")?;
        Ok(())
    })();
    if installed.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    installed
}

/// Legacy uninstall: no silent fallback. A still-selected or shared model
/// stays blocked until the user picks an explicit replacement in Setup
/// (same rule as `model_manager::remove_model`). New Setup flows should use
/// `model_manager::remove_model`; this legacy path rejects unsafe deletes
/// instead of silently reassigning roles.
#[cfg(all(windows, target_arch = "x86_64"))]
fn uninstall_platform(runtime: &Runtime, id: &str) -> Result<(), String> {
    let model = find(id).ok_or("Choose a downloadable text model.")?;
    // No silent role reassignment: explicit replacement first.
    let snapshot = runtime.snapshot()?;
    let mut roles = Vec::new();
    if snapshot.settings.cleanup_model == model.id {
        roles.push("cleanup");
    }
    if snapshot.settings.notes_model == model.id {
        roles.push("notes");
    }
    if !roles.is_empty() {
        return Err(format!(
            "This model is still selected for {}. Select a replacement in Setup before removing it; SonoJot never switches models silently.",
            roles.join(", ")
        ));
    }
    // Blocking ownership: never delete under active inference.
    let (active, _loaded) = crate::resource_manager::model_state_blocking(model.id);
    if active {
        return Err("This model is running now. Stop dictation or notes, then try again.".into());
    }
    crate::cleanup::stop_loaded_model(model.id);
    let dest = model_dir(&runtime.models_root, model.id);
    if dest.exists() {
        optional_asr::ensure_within(&runtime.models_root, &dest)?;
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("The text model files could not be removed: {e}"))?;
    }
    crate::resource_manager::note_model_removed(model.id);
    crate::resource_manager::touch_activity(crate::resource_manager::Stage::Cleanup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_requested_text_models_without_loading_them() {
        assert_eq!(MODELS.len(), 5);
        assert_eq!(LFM_12.id, "lfm2.5-1.2b");
        assert_eq!(LFM_26.id, "lfm2.5-2.6b");
        assert_eq!(QWEN.id, "qwen3-4b-instruct-2507");
        assert_eq!(BONSAI.id, BONSAI_ID);
        assert_eq!(NOTES_MODEL_ID, QWEN.id);
        assert!(BONSAI.file.bytes > 7_000_000_000);
        assert!(installable(Path::new("missing-optional-models"), BONSAI_ID));
        for model in MODELS {
            assert_eq!(model.file.sha256.len(), 64);
            assert!(model.file.bytes > 500_000_000);
            assert!(!model.file.relative.contains(".."));
            assert!(!optional_copy_ready(
                Path::new("missing-optional-models"),
                model.id
            ));
        }
    }

    #[test]
    fn library_metadata_stays_honest_and_cpu_cuda_explicit() {
        for model in MODELS {
            assert_eq!(
                quantization_of(model.id),
                if is_bonsai(model.id) {
                    "PQ2_0"
                } else {
                    "Q4_K_M"
                }
            );
            assert_eq!(
                context_tokens_of(model.id),
                Some(if is_bonsai(model.id) { 8192 } else { 4096 })
            );
            assert_eq!(revision_of(model.id), Some(model.revision));
            assert!(roles_of(model.id).contains(&"cleanup"));
            assert_eq!(roles_of(model.id).contains(&"notes"), model.id != SMART_CLEANUP_ID);
        }
        assert_eq!(family_of("lfm2.5-1.2b"), "LFM");
        assert_eq!(family_of("qwen3-4b-instruct-2507"), "Qwen3");
        assert_eq!(family_of(SMART_CLEANUP_ID), "SpeakoFlow");
        assert_eq!(SPEAKOFLOW.file.bytes, 541_903_616);
        assert_eq!(roles_of(SMART_CLEANUP_ID), &["cleanup"]);
        // CPU vs NVIDIA CUDA is explicit; AMD is never implied.
        assert!(compatibility_desc().contains("NVIDIA CUDA"));
        assert!(compatibility_desc().contains("AMD"));
        assert!(!backend_desc().contains("amd"));
    }
}
