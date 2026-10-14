//! Round 6 model library: one honest catalog, async cancellable installs,
//! and safe removal with no silent fallback.
//!
//! This module builds on the existing verified catalogs
//! (`optional_asr`, `optional_llm`, bundled Moonshine) and leaves room for
//! the ASR worker's Whisper catalog. Until that worker pins native artifacts,
//! Whisper checkpoints appear as explicitly unverified placeholders, never as
//! installable models with invented sizes or hashes.

use crate::{diarization_model, domain::Settings, granite_model, resource_manager};
use crate::model_verification::{Check, Status};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub unified_memory: bool,
    pub total_ram_bytes: Option<u64>,
    pub available_ram_bytes: Option<u64>,
    pub dedicated_vram_bytes: Option<u64>,
    pub available_vram_bytes: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub roles: Vec<String>,
    pub family: String,
    pub languages: Vec<String>,
    pub older: bool,
    pub deprecated: bool,
    pub disk_bytes: u64,
    pub ram_min_bytes: Option<u64>,
    pub ram_max_bytes: Option<u64>,
    pub vram_min_bytes: Option<u64>,
    pub vram_max_bytes: Option<u64>,
    pub memory_basis: String,
    pub guidance: String,
    pub evidence: String,
    pub backend: String,
    pub compatibility: String,
    pub state: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
    pub installable: bool,
    pub removable: bool,
    pub used_by: Vec<String>,
    pub revision: String,
    pub quantization: String,
    pub context_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLibrary {
    pub hardware: HardwareInfo,
    pub models: Vec<ModelEntry>,
    pub guidance: String,
}

// ---------------------------------------------------------------------------
// Install registry: progress + cancellation for polling UI.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Downloading,
    Verifying,
    Failed,
    Done,
    /// A removal holds this marker so a concurrent install for the same id
    /// (or any install, via the any-active check) fails fast instead of
    /// racing file deletion.
    Removing,
}

struct JobState {
    phase: Phase,
    downloaded_bytes: u64,
    total_bytes: u64,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

fn install_jobs() -> &'static Mutex<HashMap<String, JobState>> {
    static JOBS: OnceLock<Mutex<HashMap<String, JobState>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn job_snapshot(id: &str) -> Option<(Phase, u64, u64, Option<String>)> {
    install_jobs().lock().ok().and_then(|jobs| {
        jobs.get(id)
            .map(|j| (j.phase, j.downloaded_bytes, j.total_bytes, j.error.clone()))
    })
}

fn set_job_progress(id: &str, downloaded: u64, total: u64) {
    if let Ok(mut jobs) = install_jobs().lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.downloaded_bytes = downloaded;
            job.total_bytes = total;
            if job.phase == Phase::Verifying && downloaded < total {
                job.phase = Phase::Downloading;
            }
        }
    }
}

fn set_job_phase(id: &str, phase: Phase, error: Option<String>) {
    invalidate_sizes();
    if let Ok(mut jobs) = install_jobs().lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.phase = phase;
            job.error = error;
        }
    }
}

/// Clamp a GPU progress report to the combined download total. `gpu_asr`
/// reports cumulative `(downloaded, download_total_bytes)` directly (runtime
/// archives first, then model bytes); this adds nothing on top, so a report
/// can never exceed 100%. The final `(total, total)` is set by the caller on
/// verified activation only — never from a progress report.
fn clamp_gpu_progress(done: u64, total: u64) -> (u64, u64) {
    (done.min(total), total)
}

// ---------------------------------------------------------------------------
// Catalog metadata: honest, no fake rankings.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
const UNRATED: &str = "Unrated";

/// RAM/VRAM estimate pair for a catalog id, sourced from
/// `resource_manager::{estimated_ram_interval, estimated_vram_interval}`.
/// Returns `(ram_min, ram_max, vram_min, vram_max)` with None for unknown.
fn estimate_pair(id: &str) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
    let (ram_min, ram_max) = match resource_manager::estimated_ram_interval(id) {
        Some((lo, hi)) => (Some(lo), Some(hi)),
        None => (None, None),
    };
    let (vram_min, vram_max) = match resource_manager::estimated_vram_interval(id) {
        Some((lo, hi)) => (Some(lo), Some(hi)),
        None => (None, None),
    };
    (ram_min, ram_max, vram_min, vram_max)
}

fn used_by(settings: &Settings, id: &str) -> Vec<String> {
    let mut roles = Vec::new();
    if settings.asr_model == id {
        roles.push("asr".to_owned());
    }
    if settings.cleanup_model == id {
        roles.push("cleanup".to_owned());
    }
    if settings.notes_model == id {
        roles.push("notes".to_owned());
    }
    if settings.meeting_asr_model == id {
        roles.push("meetingSpeech".to_owned());
    }
    roles
}

type SizeCache = HashMap<PathBuf, (std::time::Instant, u64)>;

fn size_cache() -> &'static Mutex<SizeCache> {
    static CACHE: OnceLock<Mutex<SizeCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn invalidate_sizes() {
    if let Ok(mut cache) = size_cache().lock() {
        cache.clear();
    }
}

/// File bytes, not RAM. Install/remove invalidates immediately; short expiry
/// picks up external folder edits without a watcher or a scan per role.
pub(crate) fn folder_size(path: &Path) -> u64 {
    if let Ok(cache) = size_cache().lock() {
        if let Some((at, bytes)) = cache.get(path) {
            if at.elapsed() < Duration::from_secs(10) {
                return *bytes;
            }
        }
    }
    let bytes = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum();
    if let Ok(mut cache) = size_cache().lock() {
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(path.to_owned(), (std::time::Instant::now(), bytes));
    }
    bytes
}

fn entry_state(id: &str, ready: bool, installable: bool) -> (String, u64, u64, Option<String>) {
    if let Some((phase, downloaded, total, error)) = job_snapshot(id) {
        match phase {
            Phase::Downloading => return ("downloading".into(), downloaded, total, None),
            Phase::Verifying => return ("verifying".into(), downloaded, total, None),
            Phase::Removing => return ("busy".into(), downloaded, total, None),
            Phase::Failed => {
                if error.as_deref() == Some("Install cancelled.") {
                    return ("cancelled".into(), downloaded, total, None);
                }
                return ("failed".into(), downloaded, total, error);
            }
            Phase::Done => {}
        }
    }
    if ready {
        if resource_manager::is_model_active(id) {
            return ("busy".into(), 0, 0, None);
        }
        if resource_manager::is_model_loaded(id) {
            return ("ready".into(), 0, 0, None);
        }
        return ("installed".into(), 0, 0, None);
    }
    if installable {
        return ("not-installed".into(), 0, 0, None);
    }
    ("not-installed".into(), 0, 0, None)
}

fn status_entry_state(id: &str, status: Status, installable: bool) -> (String, u64, u64, Option<String>) {
    let current = entry_state(id, status.ready(), installable);
    if status == Status::Checking && !matches!(current.0.as_str(), "downloading" | "verifying" | "busy") {
        ("checking".into(), current.1, current.2, None)
    } else {
        current
    }
}

// Frozen from models/rewrite/manifest.json. The custom Moonshine DLL is
// app-bundled; these URLs provide weights only, never a replacement runtime.
const MOONSHINE_REVISION: &str = "quantized_26_08_21";
const MOONSHINE_AVAILABLE: bool = cfg!(all(target_os = "windows", target_arch = "x86_64"));

fn moonshine_cdn_base(id: &str) -> Option<&'static str> {
    match id {
        "moonshine-medium" => Some("https://download.moonshine.ai/model/medium-streaming-en/quantized_26_08_21"),
        "moonshine-small" => Some("https://download.moonshine.ai/model/small-streaming-en/quantized_26_08_21"),
        _ => None,
    }
}

#[derive(serde::Deserialize)]
struct BundledIdentityFile<'a> {
    name: &'a str,
    bytes: u64,
    sha256: &'a str,
}

#[derive(serde::Deserialize)]
struct BundledIdentityGroup<'a> {
    id: &'a str,
    #[serde(borrow)]
    files: Vec<BundledIdentityFile<'a>>,
}

#[derive(serde::Deserialize)]
struct BundledIdentities<'a> {
    #[serde(borrow)]
    groups: Vec<BundledIdentityGroup<'a>>,
}

fn moonshine_files(id: &str) -> Option<&'static [BundledIdentityFile<'static>]> {
    if moonshine_cdn_base(id).is_none() && id != "moonshine-runtime" {
        return None;
    }
    static IDENTITIES: OnceLock<Option<BundledIdentities<'static>>> = OnceLock::new();
    IDENTITIES
        .get_or_init(|| serde_json::from_str(include_str!("../assets/model-identities.json")).ok())
        .as_ref()?
        .groups
        .iter()
        .find(|group| group.id == id && !group.files.is_empty())
        .map(|group| group.files.as_slice())
}

fn moonshine_file_matches(path: &Path, file: &BundledIdentityFile<'static>) -> bool {
    crate::optional_asr::file_matches(
        path,
        &crate::optional_asr::PinnedFile {
            relative: file.name,
            bytes: file.bytes,
            sha256: file.sha256,
        },
    )
    .unwrap_or(false)
}

/// Only verified app-managed weights can take the place of missing bundle
/// files. Located paths and bundled resources are never install destinations.
pub(crate) fn moonshine_managed_directory(root: &Path, id: &str) -> Option<PathBuf> {
    moonshine_managed_status(root, id, Check::Blocking).ready().then(|| crate::optional_asr::model_dir(root, id))
}

pub(crate) fn moonshine_managed_status(root: &Path, id: &str, check: Check) -> Status {
    if !MOONSHINE_AVAILABLE || moonshine_cdn_base(id).is_none() {
        return Status::Missing;
    }
    let optional = crate::optional_asr::optional_root(root);
    let directory = optional.join(id);
    if !directory.is_dir()
        || crate::optional_asr::ensure_within(root, &optional).is_err()
        || crate::optional_asr::ensure_within(&optional, &directory).is_err()
    {
        return Status::Missing;
    }
    let Some(files) = moonshine_files(id) else { return Status::Missing };
    Status::all(files.iter().map(|file| check.file(root, &directory.join(file.name), file.bytes, file.sha256)))
}

fn require_moonshine_runtime(root: &Path) -> Result<(), String> {
    let library = crate::asr::runtime_path(root);
    let ready = library.parent().is_some_and(|directory| {
        moonshine_files("moonshine-runtime").is_some_and(|files| {
            files.iter().all(|file| {
                let path = if file.name == "moonshine.dll" {
                    library.clone()
                } else {
                    directory.join(file.name)
                };
                moonshine_file_matches(&path, file)
            })
        })
    });
    if !ready {
        return Err("Moonshine's speech runtime is missing or damaged. Repair SonoJot or choose a complete models folder in Setup, then retry.".into());
    }
    Ok(())
}

fn moonshine_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    for (id, name, params) in [
        ("moonshine-medium", "Moonshine Medium Streaming", "245M"),
        ("moonshine-small", "Moonshine Small Streaming", "123M"),
    ] {
        let status = if MOONSHINE_AVAILABLE { crate::asr::model_status(models_root, id, Check::Background) } else { Status::Missing };
        let ready = status.ready();
        let package_bytes = moonshine_files(id)
            .map(|files| files.iter().map(|file| file.bytes).sum())
            .unwrap_or(0);
        let installable = MOONSHINE_AVAILABLE && !ready && package_bytes > 0;
        let (disk, bundled) = if ready {
            let directory = crate::asr::model_display_directory(models_root, id);
            let bundled = directory != crate::optional_asr::optional_root(models_root).join(id);
            (folder_size(&directory), bundled)
        } else {
            (package_bytes, false)
        };
        let (mut state, dl, total, error) = status_entry_state(id, status, installable);
        if !MOONSHINE_AVAILABLE {
            state = "unavailable".into();
        }
        let total = if ready { disk } else { total.max(package_bytes) };
        let downloaded = if ready { disk } else { dl };
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(id);
        let _ = params;
        out.push(ModelEntry {
            id: id.into(),
            name: name.into(),
            kind: "asr".into(),
            roles: vec!["asr".into()],
            family: "Moonshine".into(),
            languages: vec!["en".into()],
            older: false,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: resource_manager::estimate_basis(id),
            guidance: if bundled {
                "Provisional estimate in SonoJot. Bundled English streaming default; runs on CPU. No GPU offload in this build."
            } else {
                "English streaming speech recognition on CPU. Downloads verified model weights; uses SonoJot's bundled speech runtime."
            }.into(),
            evidence: resource_manager::estimate_evidence(id).into(),
            backend: "moonshine-ort-cpu".into(),
            compatibility: "Windows x64 CPU via bundled ONNX runtime. No GPU offload in this build; never labeled as GPU.".into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable,
            removable: false,
            used_by: used_by(settings, id),
            revision: if bundled { "bundled" } else { MOONSHINE_REVISION }.into(),
            quantization: "bundled-ort".into(),
            context_tokens: None,
        });
    }
    out
}

fn optional_asr_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    for model in crate::optional_asr::catalog() {
        let status = crate::optional_asr::model_status(models_root, model.id, Check::Background);
        let ready = status.ready();
        let package = crate::optional_asr::package_bytes(model);
        let disk = if ready {
            folder_size(&crate::optional_asr::model_dir(models_root, model.id))
        } else {
            package
        };
        let (state, dl, total, error) = status_entry_state(model.id, status, true);
        let (downloaded, total) = if ready && dl == 0 && total == 0 {
            (disk, disk)
        } else if total == 0 {
            (dl, package)
        } else {
            (dl, total)
        };
        let (family, languages) = if model.id.contains("parakeet") {
            ("Parakeet", vec!["en".into()])
        } else {
            ("Qwen3-ASR", vec!["multilingual".into()])
        };
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(model.id);
        out.push(ModelEntry {
            id: model.id.into(),
            name: model.name.into(),
            kind: "asr".into(),
            roles: vec!["asr".into()],
            family: family.into(),
            languages,
            older: false,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: format!(
                "{} Pinned download is {package} bytes across {} files; runtime is sherpa-onnx CPU.",
                resource_manager::estimate_basis(model.id),
                model.files.len()
            ),
            guidance: "Provisional estimate in SonoJot. Install only for its language need; it loads on dictation start and releases per retention.".into(),
            evidence: resource_manager::estimate_evidence(model.id).into(),
            backend: "sherpa-onnx-cpu".into(),
            compatibility: "Windows x64 CPU via sherpa-onnx runtime. No GPU offload in this build; AMD acceleration is not supported.".into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable: true,
            removable: ready,
            used_by: used_by(settings, model.id),
            revision: model.revision.into(),
            quantization: "int8".into(),
            context_tokens: None,
        });
    }
    out
}

fn llama_asr_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    crate::llama_asr::catalog().iter().map(|model| {
        let status = crate::llama_asr::model_status(models_root, model.id, Check::Background);
        let ready = status.ready();
        let installable = model.runtime.supported_platform();
        let package = crate::llama_asr::package_bytes(model);
        let disk = if ready {
            folder_size(&crate::llama_asr::model_dir(models_root, model.id))
        } else {
            package
        };
        let (mut state, downloaded, total, error) = status_entry_state(model.id, status, installable);
        if !installable {
            state = "unavailable".into();
        }
        let (downloaded, total) = if ready && downloaded == 0 && total == 0 {
            (disk, disk)
        } else if total == 0 {
            (
                downloaded,
                crate::llama_asr::download_total_bytes(models_root, model.id).unwrap_or(package),
            )
        } else {
            (downloaded, total)
        };
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(model.id);
        ModelEntry {
            id: model.id.into(),
            name: model.name.into(),
            kind: "asr".into(),
            roles: vec!["asr".into()],
            family: "Qwen3-ASR".into(),
            languages: vec!["multilingual".into()],
            older: false,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: format!(
                "{} Pinned decoder and audio encoder: {package} bytes. {}",
                resource_manager::estimate_basis(model.id),
                model.license
            ),
            guidance: model.guidance.into(),
            evidence: model.evidence.into(),
            backend: model.backend.into(),
            compatibility: model.compatibility.into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable,
            removable: ready,
            used_by: used_by(settings, model.id),
            revision: model.revision.into(),
            quantization: model.quantization.into(),
            context_tokens: None,
        }
    }).collect()
}

fn granite_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    granite_entry_with_availability(models_root, settings, granite_model::unavailable_reason())
        .into_iter().collect()
}

fn granite_entry_with_availability(models_root: &Path, settings: &Settings, reason: Option<&str>) -> [ModelEntry; 1] {
    let id = granite_model::GRANITE_ID;
    let status = granite_model::model_status(models_root, Check::Background);
    let available = reason.is_none();
    let (mut state, downloaded, total, error) = status_entry_state(id, status, available);
    if !available { state = "unavailable".into(); }
    let disk = if status.ready() { folder_size(&granite_model::path(models_root)) } else { granite_model::package_bytes() };
    let (downloaded, total) = if status.ready() && downloaded == 0 && total == 0 {
        (disk, disk)
    } else if total == 0 {
        (downloaded, granite_model::download_bytes())
    } else { (downloaded, total) };
    let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(id);
    [ModelEntry {
        id: id.into(), name: granite_model::NAME.into(), kind: "asr".into(),
        roles: vec!["meetingSpeech".into()],
        family: "Granite Speech".into(),
        languages: ["en", "fr", "de", "es", "pt"].into_iter().map(str::to_owned).collect(),
        // The approved 2.5 GB library label describes the model weights, as
        // other speech rows do. Download progress includes the private runtime.
        older: false, deprecated: false, disk_bytes: granite_model::MODEL.bytes,
        ram_min_bytes: ram_min, ram_max_bytes: ram_max,
        vram_min_bytes: vram_min, vram_max_bytes: vram_max,
        memory_basis: format!("{} Model weights are 2.5 GB; this platform's complete installed package is {} bytes including its private runtime and license notices. Download progress includes every file.", resource_manager::estimate_basis(id), granite_model::package_bytes()),
        guidance: "More accurate on room microphones and calls; meetings only.".into(),
        evidence: "2026-10-10 public-corpus benchmark: Q8_0 WER 7.2% on far-field meetings, 10.2% on calls and 3.8% on TED. Observed about 3.9 GiB VRAM on RTX 5070; Apple Metal remains unmeasured.".into(),
        backend: "transcribe.cpp".into(),
        compatibility: reason.unwrap_or("Meetings only. Windows x64 with NVIDIA CUDA (compute capability 7.5+, driver 576.02+) or Apple Silicon with Metal. No CPU fallback.").into(),
        state, downloaded_bytes: downloaded, total_bytes: total, error,
        installable: available, removable: granite_model::path(models_root).exists(),
        used_by: used_by(settings, id), revision: granite_model::REVISION.into(),
        quantization: "Q8_0".into(), context_tokens: Some(4096),
    }]
}

fn diarization_entries(models_root: &Path) -> Vec<ModelEntry> {
    let id = diarization_model::ID;
    let status = diarization_model::model_status(models_root, Check::Background);
    let ready = status.ready();
    let available = diarization_model::AVAILABLE;
    let package = diarization_model::package_bytes();
    let dir = diarization_model::path(models_root);
    let (state, downloaded, total, error) = status_entry_state(id, status, available);
    let (downloaded, total) = if ready && downloaded == 0 && total == 0 {
        (package, package)
    } else if total == 0 {
        (downloaded, diarization_model::download_bytes(models_root))
    } else {
        (downloaded, total)
    };
    vec![ModelEntry {
        id: id.into(),
        name: diarization_model::NAME.into(),
        kind: "diarization".into(),
        roles: vec!["speakers".into()],
        family: "Nemotron 3 · CPU / ONNX Runtime".into(),
        languages: vec!["language-independent".into()],
        older: false,
        deprecated: false,
        disk_bytes: if ready { folder_size(&dir) } else { package },
        ram_min_bytes: None,
        ram_max_bytes: None,
        vram_min_bytes: Some(0),
        vram_max_bytes: Some(0),
        memory_basis: "RAM use is unmeasured and includes the recording audio. Runs on CPU; uses no VRAM.".into(),
        guidance: if diarization_model::MAC {
            "Labels up to 8 speakers after local meeting transcription. Includes Microsoft's ONNX Runtime 1.28.2 for Apple Silicon, removed with the package."
        } else {
            "Labels up to 8 speakers after local meeting transcription. Installs the shared CPU runtime when needed; removal keeps that runtime."
        }.into(),
        evidence: "Pinned int8 ONNX export of NVIDIA Nemotron 3 Diarization (100M parameters), OpenMDW-1.1. File sizes and SHA-256 checksums are verified on installation.".into(),
        backend: "onnxruntime-cpu".into(),
        compatibility: if !available {
            "Not available on this platform. Requires Windows x64 or an Apple Silicon Mac."
        } else if diarization_model::MAC {
            "Apple Silicon Mac CPU via ONNX Runtime 1.28.2. Runs in a separate process after transcription."
        } else {
            "Windows x64 CPU via shared ONNX Runtime 1.28.2. Runs in a separate process after transcription."
        }.into(),
        state,
        downloaded_bytes: downloaded,
        total_bytes: total,
        error,
        installable: available,
        // A partial install can still be removed when its runtime is missing.
        removable: dir.exists(),
        used_by: Vec::new(),
        revision: diarization_model::REVISION.into(),
        quantization: "int8".into(),
        context_tokens: None,
    }]
}

fn punctuation_entries(models_root: &Path) -> Vec<ModelEntry> {
    use crate::punctuation_model::Package;
    [Package::V1, Package::V2].into_iter().map(|package| punctuation_entry(models_root, package)).collect()
}

fn punctuation_entry(models_root: &Path, model: crate::punctuation_model::Package) -> ModelEntry {
    punctuation_entry_with_status(models_root, model, model.model_status(models_root, Check::Background))
}

fn punctuation_entry_with_status(models_root: &Path, model: crate::punctuation_model::Package, status: Status) -> ModelEntry {
    use crate::punctuation_model::Package;
    let id = model.id();
    let ready = status.ready();
    let available = diarization_model::AVAILABLE && model.released();
    let package = if ready {
        model.package_bytes(models_root)
    } else {
        model.pinned_bytes()
    };
    let (mut state, downloaded, total, error) = status_entry_state(id, status, available);
    if !diarization_model::AVAILABLE { state = "unavailable".into(); }
    let (downloaded, total) = if ready && downloaded == 0 && total == 0 {
        (package, package)
    } else if total == 0 {
        (downloaded, model.download_bytes(models_root, Check::Background))
    } else {
        (downloaded, total)
    };
    ModelEntry {
        id: id.into(),
        name: match model { Package::V1 => "SonoJot Punctuation", Package::V2 => "SonoJot Punctuation v2" }.into(),
        kind: "punctuation".into(),
        roles: vec!["punctuation".into()],
        family: "Token classifier".into(),
        languages: vec!["English".into()],
        older: false,
        deprecated: false,
        disk_bytes: package,
        ram_min_bytes: Some(crate::punctuation_model::RAM_ESTIMATE_BYTES),
        ram_max_bytes: Some(crate::punctuation_model::RAM_ESTIMATE_BYTES),
        vram_min_bytes: Some(0),
        vram_max_bytes: Some(0),
        memory_basis: "CPU only. Estimated resident memory: about 400 MB; model and tokenizer download about 303 MB, plus the shared runtime when missing.".into(),
        guidance: if model == Package::V2 && !model.released() {
            "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU. Download is not available yet."
        } else if model == Package::V2 {
            "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU."
        } else if model.released() {
            "Fixes sentence breaks and commas in dictation; runs on CPU."
        } else {
            "Fixes sentence breaks and commas in dictation; runs on CPU. Download is unavailable until the model upload is complete."
        }.into(),
        evidence: match model {
            Package::V1 => "CC BY-SA 4.0. Downloads verify pinned sizes and SHA-256 checksums; existing local manifests remain supported. Model, source and license: https://huggingface.co/SonoJot/sonojot-punct-v1",
            Package::V2 => "CC BY-SA 4.0. Downloads verify pinned sizes and SHA-256 checksums; existing local manifests remain supported. Adds written numbers, money and dates in Clean dictation. Preferred over v1 when ready. Model, source and license: https://huggingface.co/SonoJot/sonojot-punct-v2",
        }.into(),
        backend: "onnxruntime-cpu".into(),
        compatibility: if diarization_model::AVAILABLE && !model.released() {
            "Windows x64 and Apple Silicon CPU. Requires the shared ONNX Runtime; removal keeps it."
        } else if diarization_model::AVAILABLE {
            "Windows x64 and Apple Silicon CPU. Installs the shared ONNX Runtime when needed; removal keeps it."
        } else {
            "Not available on this platform. Requires Windows x64 or an Apple Silicon Mac."
        }.into(),
        state,
        downloaded_bytes: downloaded,
        total_bytes: total,
        error,
        installable: available,
        removable: model.path(models_root).exists(),
        used_by: Vec::new(),
        revision: model.revision().unwrap_or_default().into(),
        quantization: "embedding-only int8".into(),
        context_tokens: None,
    }
}

fn optional_llm_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    for model in crate::optional_llm::catalog() {
        let copy_ready = crate::optional_llm::optional_copy_status(models_root, model.id, Check::Background).ready();
        let status = crate::cleanup::model_status(models_root, model.id, Check::Background);
        let server_ready = status.ready();
        let package = crate::optional_llm::package_bytes(model);
        let disk = if copy_ready {
            package
        } else if server_ready {
            // Bundled LFM fallback: real file size when present.
            fs::metadata(crate::asr::bundle_path(
                models_root,
                "cleanup_gguf",
                "llama.cpp/models/LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
            ))
            .map(|m| m.len())
            .unwrap_or(package)
        } else {
            package
        };
        let installable = crate::optional_llm::installable(models_root, model.id);
        let (state, dl, total, error) = status_entry_state(model.id, status, installable);
        let (downloaded, total) = if server_ready && dl == 0 && total == 0 {
            (disk, disk)
        } else if total == 0 {
            (dl, package)
        } else {
            (dl, total)
        };
        let family = crate::optional_llm::family_of(model.id);
        let context = crate::optional_llm::context_tokens_of(model.id);
        let smart = model.id == crate::optional_llm::SMART_CLEANUP_ID;
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(model.id);
        out.push(ModelEntry {
            id: model.id.into(),
            name: model.name.into(),
            kind: "cleanup".into(),
            roles: crate::optional_llm::roles_of(model.id).iter().map(|role| (*role).into()).collect(),
            family: family.into(),
            languages: vec![if smart { "en" } else { "multilingual" }.into()],
            older: false,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: format!(
                "{} Pinned GGUF is {package} bytes, {}, {}-token server context. {}",
                resource_manager::estimate_basis(model.id)
                , crate::optional_llm::quantization_of(model.id)
                , crate::optional_llm::context_tokens_of(model.id).unwrap_or(4096)
                , if smart { "SpeakoFlow Mini by Abhishek Barali · Apache-2.0. Pinned card declares no additional attribution condition; no standalone LICENSE or NOTICE is supplied. https://huggingface.co/SpeakoFlow/speakoflow-mini/blob/835431771f72820251fe6c6b4b07f12b000e2647/README.md · https://www.apache.org/licenses/LICENSE-2.0" } else { "One artifact can serve cleanup and notes with separate contexts." }
            ),
            guidance: if smart {
                "Custom profile only; Clean dictation only. SpeakoFlow Mini 0.8B Q4_K_M. Guarded proposals fall back to local rules; Use original copies the pre-model text. Mac is unmeasured.".into()
            } else if crate::optional_llm::is_bonsai(model.id) && !crate::optional_llm::bonsai_runtime_ready(models_root) {
                "Installs the pinned PrismML CUDA server plus CUDA DLL bundle (~535 MB) and PQ2_0 weights (~7.2 GB) together. The bundled stock llama-server cannot load this model.".into()
            } else if model.id == "lfm2.5-1.2b" {
                "Provisional estimate in SonoJot. Smallest local text option; try it first on low-RAM machines.".into()
            } else if model.id == "qwen3-4b-instruct-2507" {
                "Provisional estimate in SonoJot. Larger notes-capable option; needs more RAM than the 1.2B choice. Same download runs on CPU or NVIDIA CUDA.".into()
            } else {
                "Provisional estimate in SonoJot. Mid-size option; pick only if the 1.2B choice misses useful edits.".into()
            },
            evidence: resource_manager::estimate_evidence(model.id).into(),
            backend: if crate::optional_llm::is_bonsai(model.id) { "PrismML llama-server fork (PQ2_0; CUDA)" } else { "llama-server (cpu; cuda when NVIDIA runtime present)" }.into(),
            compatibility: if crate::optional_llm::is_bonsai(model.id) { "Windows x64 NVIDIA CUDA via PrismML llama.cpp fork; bundled stock llama-server cannot load this format; runtime currently unavailable." } else { "Windows x64 CPU; NVIDIA CUDA when the CUDA runtime is present and text acceleration is auto. AMD graphics not supported." }.into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable,
            removable: copy_ready,
            used_by: used_by(settings, model.id),
            revision: model.revision.into(),
            quantization: crate::optional_llm::quantization_of(model.id).into(),
            context_tokens: context,
        });
    }
    // Rules-only path: always present, no download.
    let (state, _, _, _) = entry_state("local-rules", true, false);
    out.push(ModelEntry {
        id: "local-rules".into(),
        name: "Formatting and vocabulary".into(),
        kind: "cleanup".into(),
        roles: vec!["cleanup".into()],
        family: "Rules".into(),
        languages: vec!["en".into()],
        older: false,
        deprecated: false,
        disk_bytes: 0,
        ram_min_bytes: None,
        ram_max_bytes: None,
        vram_min_bytes: None,
        vram_max_bytes: None,
        memory_basis: "Host-only rules; no model weights, no measurable inference RAM.".into(),
        guidance: "Immediate local rules; no language model. Use when no text model is installed."
            .into(),
        evidence: "Host rules; no model rating applies.".into(),
        backend: "local-rules".into(),
        compatibility: "All platforms, CPU only, no download.".into(),
        state,
        downloaded_bytes: 0,
        total_bytes: 0,
        error: None,
        installable: false,
        removable: false,
        used_by: used_by(settings, "local-rules"),
        revision: "host-rules-v4".into(),
        quantization: "none".into(),
        context_tokens: None,
    });
    out
}

/// Real Whisper catalog from the landed ASR worker (`whisper_catalog.rs`,
/// pinned 2026-09-16 with HF revisions/sizes/hashes + `catalog_metadata()`).
/// Placeholder seed entries are removed; every row here is installable with
/// real disk bytes and provisional RAM estimates. Real GPU installs arrive via
/// `gpu_asr::catalog_metadata` (Qwen FP16 CUDA + Whisper tiny FP32 CUDA) with
/// install/progress/cancel/remove paths below; the old display-only
/// `qwen_gpu_info` placeholder is replaced, never resurrected.
fn whisper_real_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    let mut out = Vec::new();
    for entry in crate::whisper_catalog::catalog_metadata() {
        let status = crate::whisper_catalog::model_status(models_root, entry.id, Check::Background);
        let ready = status.ready();
        let disk = if ready {
            folder_size(&crate::whisper_catalog::model_dir(models_root, entry.id))
        } else {
            entry.disk_bytes
        };
        let (state, dl, total, error) = status_entry_state(entry.id, status, entry.installable);
        let (downloaded, total) = if ready && dl == 0 && total == 0 {
            (disk, disk)
        } else if total == 0 {
            (dl, entry.disk_bytes)
        } else {
            (dl, total)
        };
        let languages = if entry.languages == "English only" {
            vec!["en".into()]
        } else {
            vec!["multilingual".into()]
        };
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(entry.id);
        out.push(ModelEntry {
            id: entry.id.into(),
            name: entry.name.into(),
            kind: "asr".into(),
            roles: vec!["asr".into()],
            family: "Whisper".into(),
            languages,
            older: entry.older,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: format!(
                "{} HF package {} bytes (pinned revision {}).",
                resource_manager::estimate_basis(entry.id),
                entry.disk_bytes,
                entry.revision
            ),
            guidance: entry.reason.into(),
            evidence: entry.evidence.into(),
            backend: entry.backend.into(),
            compatibility: entry.compatibility.into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable: entry.installable,
            removable: ready,
            used_by: used_by(settings, entry.id),
            revision: entry.revision.into(),
            quantization: entry.quantization.into(),
            context_tokens: None,
        });
    }
    // Real GPU catalog (gpu_asr worker, verified 2026-09-16 on RTX 5070).
    // Qwen FP16 CUDA (~4.73 GiB) + Whisper tiny FP32 CUDA (~153 MiB), both
    // installable with transactional staging + cancel/progress. Compatibility
    // is Windows x64 NVIDIA CUDA 13.x only; AMD has no CUDA path. Requested
    // provider "cuda" comes from the ID alone; observed GPU execution is never
    // claimed here (see acceleration::speech_detail, which separates hardware
    // availability from execution).
    for entry in crate::gpu_asr::catalog_metadata() {
        let status = crate::gpu_asr::model_status(models_root, entry.id, Check::Background);
        let ready = status.ready();
        let runtime_ready = crate::gpu_asr::runtime_status(models_root, Check::Background).ready();
        // First-install cost discloses the shared runtime: model bytes plus
        // ~1.43 GiB CUDA runtime when it is not yet present. Ready rows report
        // the real folder size; not-ready rows report model bytes with the
        // runtime disclosed in guidance (progress totals use
        // download_total_bytes; disk cost uses install_bytes).
        let disk = if ready {
            folder_size(&crate::gpu_asr::model_dir(models_root, entry.id))
        } else {
            entry.disk_bytes
        };
        let total_for_progress =
            crate::gpu_asr::download_total_bytes(entry.id).unwrap_or(entry.disk_bytes);
        let (state, dl, total, error) = status_entry_state(entry.id, status, entry.installable);
        let (downloaded, total) = if ready && dl == 0 && total == 0 {
            (disk, disk)
        } else if total == 0 {
            (dl, total_for_progress)
        } else {
            (dl, total)
        };
        let languages = if entry.languages.eq_ignore_ascii_case("Multilingual") {
            vec!["multilingual".into()]
        } else {
            vec!["en".into()]
        };
        let (ram_min, ram_max, vram_min, vram_max) = estimate_pair(entry.id);
        let family = if entry.family.eq_ignore_ascii_case("qwen") {
            "Qwen3-ASR"
        } else if entry.family.eq_ignore_ascii_case("parakeet") {
            "Parakeet"
        } else {
            "Whisper"
        };
        let guidance = if runtime_ready {
            entry.reason.to_owned()
        } else {
            format!(
                "{} First install also downloads the shared NVIDIA CUDA 13 runtime (1.32 GB, one-time).",
                entry.reason
            )
        };
        out.push(ModelEntry {
            id: entry.id.into(),
            name: entry.name.into(),
            kind: "asr".into(),
            roles: vec!["asr".into()],
            family: family.into(),
            languages,
            older: false,
            deprecated: false,
            disk_bytes: disk,
            ram_min_bytes: ram_min,
            ram_max_bytes: ram_max,
            vram_min_bytes: vram_min,
            vram_max_bytes: vram_max,
            memory_basis: format!(
                "{} HF package {} bytes (pinned revision {}). {}",
                resource_manager::estimate_basis(entry.id),
                entry.disk_bytes,
                entry.revision,
                entry.license
            ),
            guidance,
            evidence: entry.evidence.into(),
            backend: entry.backend.into(),
            compatibility: entry.compatibility.into(),
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
            error,
            installable: entry.installable,
            removable: ready,
            used_by: used_by(settings, entry.id),
            revision: entry.revision.into(),
            quantization: entry.quantization.into(),
            context_tokens: None,
        });
    }
    out
}

/// Back-compat alias for the integration worker's expected hook name.
/// Returns the real Whisper entries (no placeholders).
pub fn asr_worker_extra_entries(models_root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    whisper_real_entries(models_root, settings)
}

/// Only native, supported engines are offered for installation on this host.
fn mac_supported(id: &str) -> bool {
    crate::mac_speech::find(id).is_some()
        || crate::llama_asr::is_metal(id)
        || matches!(
            id,
            "lfm2.5-1.2b" | "lfm2.5-2.6b" | "qwen3-4b-instruct-2507" | "local-rules"
                | crate::optional_llm::SMART_CLEANUP_ID
                | diarization_model::ID
                | granite_model::GRANITE_ID
                | crate::punctuation_model::ID
                | crate::punctuation_model::V2_ID
        )
}

fn mac_entries(root: &Path, settings: &Settings) -> Vec<ModelEntry> {
    crate::mac_speech::catalog().iter().map(|model| {
        let status = crate::mac_speech::model_status(root, model.id, Check::Background);
        let ready = status.ready();
        let (state, downloaded, total, error) = status_entry_state(model.id, status, true);
        ModelEntry {
            id: model.id.into(), name: model.name.into(), kind: "asr".into(),
            roles: vec!["asr".into()], family: "Whisper".into(), languages: vec!["multilingual".into()],
            older: false, deprecated: false, disk_bytes: model.bytes,
            ram_min_bytes: Some(model.ram_min_bytes), ram_max_bytes: Some(model.ram_max_bytes),
            vram_min_bytes: Some(0), vram_max_bytes: Some(0),
            memory_basis: resource_manager::estimate_basis(model.id),
            guidance: "Native Apple GPU speech recognition. Installs verified weights; the Metal engine is included in SonoJot. Text arrives at pauses, with a 20-second segment limit.".into(),
            evidence: "Provisional unified-memory estimate; not a measured peak.".into(),
            backend: "whisper.cpp · Apple Metal".into(), compatibility: "Apple Silicon Mac · Metal GPU".into(),
            state, downloaded_bytes: downloaded, total_bytes: if total == 0 { model.bytes } else { total },
            error, installable: true, removable: ready, used_by: used_by(settings, model.id),
            revision: model.revision.into(), quantization: "F16".into(), context_tokens: None,
        }
    }).collect()
}

/// Pure library builder for tests and Tauri commands. Never touches the
/// user's daily store; callers pass an explicit root and settings.
/// Curated discovery only. Existing packages, selections and removal stay valid.
fn deprecated_model(id: &str) -> bool {
    crate::gpu_asr::is_gpu_model(id)
        || matches!(id, "qwen3-asr-1.7b" | "whisper-large-v3" | "whisper-medium" | "whisper-medium.en")
        || id.starts_with("granite-4.0-")
        || id.starts_with("granite-speech-4.0-")
}

#[cfg(test)]
#[test]
fn curated_model_flags_preserve_supported_choices() {
    for model in crate::gpu_asr::GPU_MODELS { assert!(deprecated_model(model.id)); }
    for id in ["qwen3-asr-1.7b", "whisper-large-v3", "whisper-medium", "whisper-medium.en", "granite-4.0-1b-speech"] {
        assert!(deprecated_model(id), "{id}");
    }
    for id in [crate::llama_asr::SMALL_CPU_MODEL_ID, crate::llama_asr::CPU_MODEL_ID, crate::llama_asr::MODEL_ID, "moonshine-medium", "whisper-small", "whisper-large-v3-turbo", crate::granite_model::GRANITE_ID] {
        assert!(!deprecated_model(id), "{id}");
    }
    assert_eq!(crate::llama_asr::package_bytes(crate::llama_asr::find(crate::llama_asr::SMALL_CPU_MODEL_ID).unwrap()), 1_019_141_728);
}

pub fn build_library(models_root: &Path, settings: &Settings) -> ModelLibrary {
    let hardware_probe = resource_manager::probe_hardware();
    let gpu = resource_manager::displayed_gpu_pair();
    let hardware = HardwareInfo {
        unified_memory: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        total_ram_bytes: hardware_probe.total_ram_bytes,
        available_ram_bytes: hardware_probe.available_ram_bytes,
        dedicated_vram_bytes: gpu.map(|(total, _)| total),
        available_vram_bytes: gpu.map(|(_, free)| free),
    };
    let mut models = Vec::new();
    models.extend(moonshine_entries(models_root, settings));
    models.extend(optional_asr_entries(models_root, settings));
    models.extend(llama_asr_entries(models_root, settings));
    models.extend(granite_entries(models_root, settings));
    models.extend(diarization_entries(models_root));
    models.extend(punctuation_entries(models_root));
    models.extend(optional_llm_entries(models_root, settings));
    models.extend(asr_worker_extra_entries(models_root, settings));
    for model in &mut models {
        model.deprecated = deprecated_model(&model.id);
        if crate::gpu_asr::is_gpu_model(&model.id)
            && !crate::gpu_asr::runtime_status(models_root, Check::Background).ready()
        {
            model.installable = false;
            model.guidance = crate::download_consent::SHERPA_HOLD.into();
        }
    }
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        models.splice(0..0, mac_entries(models_root, settings));
        for model in &mut models {
            if !mac_supported(&model.id) {
                model.installable = false;
                model.compatibility =
                    "This package requires Windows; choose a Metal model on this Mac.".into();
                if !model.removable {
                    model.state = "unavailable".into();
                }
            } else if model.kind == "cleanup" && model.id != "local-rules" {
                model.backend = "llama.cpp · Apple Metal or CPU".into();
                model.compatibility =
                    "Apple Silicon Mac · Metal with Auto acceleration, or CPU".into();
                model.guidance = if model.id == crate::optional_llm::SMART_CLEANUP_ID {
                    "Custom profile only; Clean dictation only. Installs verified GGUF weights and the shared Metal runtime. Auto uses the Apple GPU; CPU remains available. Mac latency, memory and guarded output are unmeasured.".into()
                } else {
                    "Installs verified GGUF weights and the shared Metal runtime. Auto uses the Apple GPU; CPU remains available in Setup.".into()
                };
                model.memory_basis = model.memory_basis.replace("GPU range assumes full CUDA offload; CPU-only uses no VRAM.", "On Apple Silicon the host estimate covers the shared CPU/GPU pool, counted once.");
            }
        }
        models.sort_by_key(|m| {
            if crate::mac_speech::find(&m.id).is_some() {
                0
            } else if mac_supported(&m.id) {
                1
            } else {
                2
            }
        });
    }
    let plan = resource_manager::plan_range(
        &settings.asr_model,
        &settings.cleanup_model,
        resource_manager::current_memory_mode(),
    );
    let mut guidance = resource_manager::guidance_text(
        &resource_manager::HardwareSnapshot {
            total_ram_bytes: hardware.total_ram_bytes,
            available_ram_bytes: hardware.available_ram_bytes,
            dedicated_vram_bytes: hardware.dedicated_vram_bytes,
        },
        plan,
    ) + " Estimates are provisional (weights + engine/context assumptions documented per model); disk size is real and separate. Text models run on CPU, or NVIDIA CUDA when that runtime is present. AMD graphics are not supported.";
    if hardware.unified_memory {
        guidance = "Apple Silicon shares memory between CPU and GPU. Model estimates count that pool once; available memory estimates free and reclaimable inactive pages. Estimates are provisional. Metal speech and text models are listed first.".into();
    }
    ModelLibrary {
        hardware,
        models,
        guidance,
    }
}

// ---------------------------------------------------------------------------
// Generic cancellable verified installs, reusing the existing download stack
// semantics: pinned URL/revision, expected bytes, SHA-256, atomic readiness,
// partial-file cleanup, insufficient-disk check.
// ---------------------------------------------------------------------------

struct PinnedDownload {
    relative: &'static str,
    bytes: u64,
    sha256: &'static str,
    url: String,
}

struct InstallTarget {
    id: String,
    dest_dir: PathBuf,
    optional_root: PathBuf,
    files: Vec<PinnedDownload>,
    total_bytes: u64,
    needs_runtime: bool,
}

fn describe_target(models_root: &Path, id: &str) -> Result<InstallTarget, String> {
    if id == granite_model::GRANITE_ID {
        if let Some(reason) = granite_model::unavailable_reason() { return Err(reason.into()); }
        return Ok(InstallTarget {
            id: id.into(), dest_dir: granite_model::path(models_root),
            optional_root: crate::optional_asr::optional_root(models_root),
            files: Vec::new(), total_bytes: granite_model::download_bytes(), needs_runtime: false,
        });
    }
    if let Some(package) = crate::punctuation_model::Package::from_id(id) {
        if !package.released() {
            return Err(package.unreleased_error());
        }
        if !diarization_model::AVAILABLE {
            return Err("SonoJot Punctuation requires Windows x64 or an Apple Silicon Mac.".into());
        }
        return Ok(InstallTarget {
            id: id.into(),
            dest_dir: package.path(models_root),
            optional_root: models_root.join("punctuation"),
            files: package.pins().iter().map(|file| Ok(PinnedDownload {
                relative: file.relative, bytes: file.bytes, sha256: file.sha256,
                url: package.download_url(file)?,
            })).collect::<Result<Vec<_>, String>>()?,
            total_bytes: package.download_bytes(models_root, Check::Background),
            needs_runtime: true,
        });
    }
    if let Some(base) = moonshine_cdn_base(id) {
        if !MOONSHINE_AVAILABLE {
            return Err("Moonshine requires Windows x64. On Mac, choose Whisper Small.".into());
        }
        let files: Vec<_> = moonshine_files(id)
            .ok_or("The app's Moonshine identity list could not be read. Repair SonoJot.")?
            .iter()
            .map(|file| PinnedDownload {
                relative: file.name,
                bytes: file.bytes,
                sha256: file.sha256,
                url: format!("{base}/{}", file.name),
            })
            .collect();
        let optional_root = crate::optional_asr::optional_root(models_root);
        return Ok(InstallTarget {
            id: id.into(),
            dest_dir: optional_root.join(id),
            optional_root,
            total_bytes: files.iter().map(|file| file.bytes).sum(),
            files,
            // The custom bundled runtime has no downloadable substitute.
            needs_runtime: false,
        });
    }
    if let Some(model) = crate::llama_asr::find(id) {
        if !model.runtime.supported_platform() {
            return Err(model.runtime.platform_requirement().into());
        }
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: crate::llama_asr::model_dir(models_root, id),
            optional_root: crate::optional_asr::optional_root(models_root),
            // Includes both GGUFs and the platform's missing shared runtime.
            files: Vec::new(),
            total_bytes: crate::llama_asr::download_total_bytes(models_root, id)
                .unwrap_or_else(|| crate::llama_asr::package_bytes(model)),
            needs_runtime: false,
        });
    }
    if let Some(model) = crate::mac_speech::find(id) {
        if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            return Err("This model requires an Apple Silicon Mac.".into());
        }
        return Ok(InstallTarget {
            id: id.into(),
            dest_dir: crate::mac_speech::model_dir(models_root, id),
            optional_root: crate::optional_asr::optional_root(models_root),
            files: vec![PinnedDownload {
                relative: model.file,
                bytes: model.bytes,
                sha256: model.sha256,
                url: format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    crate::mac_speech::REPO, model.revision, model.file
                ),
            }],
            total_bytes: model.bytes,
            needs_runtime: false,
        });
    }
    if id == diarization_model::ID {
        if !diarization_model::AVAILABLE {
            return Err(
                "Speaker labels are not available on this platform. Use Windows x64 or an Apple Silicon Mac.".into(),
            );
        }
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: diarization_model::path(models_root),
            optional_root: crate::optional_asr::optional_root(models_root),
            files: diarization_model::FILES
                .iter()
                .map(|file| PinnedDownload {
                    relative: file.relative,
                    bytes: file.bytes,
                    sha256: file.sha256,
                    url: format!(
                        "https://huggingface.co/{}/resolve/{}/{}",
                        diarization_model::REPO, diarization_model::REVISION, file.relative
                    ),
                })
                .collect(),
            total_bytes: diarization_model::download_bytes(models_root),
            needs_runtime: true,
        });
    }
    if let Some(model) = crate::optional_llm::find(id) {
        if !crate::optional_llm::installable(models_root, id) {
            return Err("Ternary Bonsai 2 requires the compatible PrismML llama.cpp runtime. The current Windows runtime package does not include its server binary yet.".into());
        }
        let files = vec![PinnedDownload {
            relative: model.file.relative,
            bytes: model.file.bytes,
            sha256: model.file.sha256,
            url: format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                model.repo, model.revision, model.file.relative
            ),
        }];
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: crate::optional_llm::model_dir(models_root, model.id),
            optional_root: crate::optional_asr::optional_root(models_root),
            files,
            total_bytes: model.file.bytes + mac_runtime_download_bytes(models_root),
            needs_runtime: false,
        });
    }
    if let Some(model) = crate::optional_asr::find(id) {
        let files = model
            .files
            .iter()
            .map(|f| PinnedDownload {
                relative: f.relative,
                bytes: f.bytes,
                sha256: f.sha256,
                url: format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    model.repo, model.revision, f.relative
                ),
            })
            .collect::<Vec<_>>();
        let total = files.iter().map(|f| f.bytes).sum();
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: crate::optional_asr::model_dir(models_root, model.id),
            optional_root: crate::optional_asr::optional_root(models_root),
            files,
            total_bytes: total,
            needs_runtime: true,
        });
    }
    // Real Whisper installer from the landed ASR worker. The generic file
    // list stays empty because whisper uses url_name->local_name mapping;
    // install_inner delegates to whisper_catalog::install_model with progress
    // mapped into the job registry. Total is the pinned package size.
    if let Some(checkpoint) = crate::whisper_catalog::find(id) {
        let total = crate::whisper_catalog::package_bytes(checkpoint);
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: crate::whisper_catalog::model_dir(models_root, checkpoint.id),
            optional_root: crate::optional_asr::optional_root(models_root),
            files: Vec::new(),
            total_bytes: total,
            needs_runtime: true,
        });
    }
    // Real GPU installer (gpu_asr worker). File list stays empty; install_inner
    // delegates to gpu_asr::install_model. Progress totals use
    // download_total_bytes (runtime archives + model download bytes, the exact
    // denominator gpu_asr reports against); disk cost uses install_bytes
    // (unpacked on-disk size), which is larger.
    if let Some(gpu) = crate::gpu_asr::find(id) {
        let total = crate::gpu_asr::download_total_bytes(gpu.id).unwrap_or_else(|| {
            crate::gpu_asr::catalog_metadata()
                .iter()
                .find(|e| e.id == gpu.id)
                .map(|e| e.disk_bytes)
                .unwrap_or(0)
        });
        return Ok(InstallTarget {
            id: id.to_owned(),
            dest_dir: crate::gpu_asr::model_dir(models_root, gpu.id),
            optional_root: crate::optional_asr::optional_root(models_root),
            files: Vec::new(),
            total_bytes: total,
            needs_runtime: false,
        });
    }
    Err(format!("Unknown or not installable model: {id}"))
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    Ok(())
}

// Reserve the final byte for verified activation, including while hashing the
// last file. A failed or cancelled install must not report 100% completion.
fn set_download_progress(id: &str, downloaded: u64, total: u64) {
    set_job_progress(id, downloaded.min(total.saturating_sub(1)), total);
}

#[allow(clippy::too_many_arguments)]
fn download_cancellable(
    url: &str,
    dest: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    host_is_github: bool,
    what: &str,
    base_done: u64,
    total: u64,
    id: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    check_cancel(cancel)?;
    let host_is_moonshine = url.starts_with("https://download.moonshine.ai/");
    let host_is_pythonhosted = url.starts_with("https://files.pythonhosted.org/");
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60 * 60))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    if host_is_github {
                        host == "github.com" || host.ends_with(".githubusercontent.com")
                    } else if host_is_moonshine {
                        host == "download.moonshine.ai"
                    } else if host_is_pythonhosted {
                        host == "files.pythonhosted.org"
                    } else {
                        host == "huggingface.co"
                            || host.ends_with(".huggingface.co")
                            || host.ends_with(".hf.co")
                            || host == "cas-bridge.xethub.hf.co"
                            || host.ends_with(".xethub.hf.co")
                    }
                });
            if allowed && attempt.previous().len() < 8 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| format!("Cannot prepare the download: {e}"))?;
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .map_err(|e| format!("The {what} download could not start: {e}"))?
        .error_for_status()
        .map_err(|e| format!("The {what} download was refused: {e}"))?;
    if let Some(length) = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        if length != expected_bytes {
            return Err(
                "The server offered an unexpected download size. Nothing was installed.".into(),
            );
        }
    }
    let file = fs::File::create(dest).map_err(|e| format!("Cannot stage the download: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    let deadline = std::time::Instant::now() + Duration::from_secs(60 * 60);
    let mut total_written = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        check_cancel(cancel)?;
        if std::time::Instant::now() >= deadline {
            return Err(
                "The download exceeded the time limit. Check the connection and try again.".into(),
            );
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The {what} download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total_written += read as u64;
        if total_written > expected_bytes {
            return Err(format!(
                "The {what} download was larger than the pinned file."
            ));
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save the download: {e}"))?;
        set_download_progress(id, base_done.saturating_add(total_written), total);
    }
    out.flush()
        .map_err(|e| format!("Cannot save the download: {e}"))?;
    drop(out);
    check_cancel(cancel)?;
    if total_written != expected_bytes {
        return Err(format!(
            "The download is incomplete ({total_written} of {expected_bytes} bytes). Try again."
        ));
    }
    set_job_phase(id, Phase::Verifying, None);
    // Verify SHA-256 before activation, same as the existing stack.
    let mut file = fs::File::open(dest)
        .map_err(|_| "The download could not be read completely.".to_string())?;
    let mut hasher = Sha256::new();
    let mut checked = 0u64;
    loop {
        check_cancel(cancel)?;
        let read = file
            .read(&mut buffer)
            .map_err(|_| "The download could not be read completely.".to_string())?;
        if read == 0 {
            break;
        }
        checked += read as u64;
        hasher.update(&buffer[..read]);
    }
    if checked != expected_bytes {
        return Err(format!(
            "The download is incomplete ({checked} of {expected_bytes} bytes). Try again."
        ));
    }
    let hash = format!("{:x}", hasher.finalize());
    if !hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "The downloaded {what} file did not match its pinned checksum. Nothing was installed."
        ));
    }
    Ok(())
}

fn install_blocking(models_root: PathBuf, id: String, cancel: Arc<AtomicBool>) {
    let result = install_inner(&models_root, &id, &cancel);
    match result {
        Ok(()) => {
            set_job_phase(&id, Phase::Done, None);
            // Drop the finished job so the library shows installed, not stale
            // progress. Pollers see installed on the next read.
            if let Ok(mut jobs) = install_jobs().lock() {
                jobs.remove(&id);
            }
        }
        Err(error) => {
            let cancelled = error == "Install cancelled.";
            set_job_phase(&id, Phase::Failed, Some(error));
            let _ = cancelled;
        }
    }
}

fn install_inner(models_root: &Path, id: &str, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    if cfg!(target_os = "macos") && (!cfg!(target_arch = "aarch64") || !mac_supported(id)) {
        return Err(
            "This model package is not available on this Mac. Choose an Apple Metal model.".into(),
        );
    }

    if id == granite_model::GRANITE_ID {
        return install_granite(models_root, cancel);
    }
    if let Some(package) = crate::punctuation_model::Package::from_id(id) {
        set_job_phase(id, Phase::Downloading, None);
        return crate::punctuation_model::install_package(models_root, package, cancel,
            &|done, total| set_download_progress(id, done, total),
            &|| set_job_phase(id, Phase::Verifying, None),
        ).and_then(|()| {
            crate::model_licenses::write_punctuation_notices(models_root, id)?;
            if let Some((_, _, total, _)) = job_snapshot(id) {
                set_job_progress(id, total, total);
            }
            Ok(())
        });
    }
    if crate::llama_asr::is_llama_asr(id) {
        let _target = describe_target(models_root, id)?;
        check_cancel(cancel)?;
        set_job_phase(id, Phase::Downloading, None);
        let total = crate::llama_asr::download_bytes_with_check(models_root, id, Check::Blocking)
            .ok_or("Unknown speech model.")?;
        set_job_progress(id, 0, total);
        let runtime_done = if crate::llama_asr::is_metal(id) { mac_runtime_download_bytes(models_root) } else { 0 };
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if crate::llama_asr::is_metal(id) {
            // Setup installs speech before Notes. Prepare the existing shared
            // Metal runtime here too, before the GGUF installer requires it.
            let disk_path = models_root.ancestors().find(|path| path.is_dir()).unwrap_or(models_root);
            if resource_manager::disk_free_bytes(disk_path)
                .is_some_and(|free| free < total.saturating_add(256 << 20))
            {
                return Err("Not enough disk space for the speech model and Metal runtime. Free space and try again.".into());
            }
            install_mac_runtime(models_root, id, total, cancel)?;
            set_job_phase(id, Phase::Downloading, None);
        }
        // Windows includes CPU VAD in the installer's progress. Metal's shared
        // runtime was just prepared; add its bytes once to the GGUF progress.
        let result = crate::llama_asr::install_model(models_root, id, cancel, &|done, _| {
            let (done, total) = clamp_gpu_progress(runtime_done + done, total);
            set_job_progress(id, done, total);
        });
        if result.is_ok() {
            set_job_progress(id, total, total);
        }
        return result;
    }

    // Real Whisper path: delegate to the ASR worker's verified transactional
    // installer (staging + checksum + atomic rename + cancel + progress).
    if crate::whisper_catalog::is_whisper(id) {
        check_cancel(cancel)?;
        set_job_phase(id, Phase::Downloading, None);
        let total = crate::whisper_catalog::find(id)
            .map(crate::whisper_catalog::package_bytes)
            .unwrap_or(0);
        set_job_progress(id, 0, total);
        let result =
            crate::whisper_catalog::install_model(models_root, id, cancel, &|done, total| {
                set_job_progress(id, done, total);
            });
        if result.is_ok() {
            set_job_progress(id, total, total);
        }
        return result;
    }
    // Real GPU path (gpu_asr worker, CUDA 13 only). Transactional staging +
    // SHA-256 + cancel live in gpu_asr::install_model, which reports COMBINED
    // download progress against download_total_bytes (runtime archives first,
    // then model bytes, every report clamped). Forward those reports directly:
    // adding the runtime again here would double-count past 100%. Disk is
    // gated on install_bytes (unpacked on-disk cost), not the download total.
    if crate::gpu_asr::is_gpu_model(id) {
        check_cancel(cancel)?;
        // Downloads do NOT load weights and need no runtime RAM/VRAM fit.
        // Runtime memory admission runs only at LOAD (atomic acquire); a
        // download must never be denied because other apps occupy RAM/VRAM,
        // and active inference is never stopped to install. Disk, integrity
        // (SHA-256 + sizes in gpu_asr), and hardware-compat guidance stay.
        // Fit is advisory only: surface a warning via the catalog, never a
        // rejection here.
        let _fit_warning = resource_manager::vram_fit_warning(id, "GPU speech model install");
        // Disk gate for model + shared runtime when missing.
        let disk_need = crate::gpu_asr::install_bytes(id).unwrap_or(0);
        if let Some(free) =
            resource_manager::disk_free_bytes(&crate::optional_asr::optional_root(models_root))
        {
            let need = disk_need.saturating_add(256 << 20);
            if free < need {
                return Err(format!(
                    "Not enough disk space: need about {} free, found {}. Free space and try again.",
                    resource_manager::format_bytes(Some(need)),
                    resource_manager::format_bytes(Some(free)),
                ));
            }
        }
        set_job_phase(id, Phase::Downloading, None);
        // Progress denominator is the combined download total; gpu_asr already
        // reports cumulative (downloaded, download_total_bytes). Clamp on
        // receipt so a backend slip can never show >100% or 100% before the
        // verified activation completes (final 100 is set below, on Ok only).
        let total = crate::gpu_asr::download_total_bytes(id).unwrap_or(0);
        set_job_progress(id, 0, total);
        let result = crate::gpu_asr::install_model(models_root, id, cancel, &|done, _| {
            let (clamped, total) = clamp_gpu_progress(done, total);
            set_job_progress(id, clamped, total);
        });
        if result.is_ok() {
            set_job_progress(id, total, total);
        }
        return result;
    }
    if crate::optional_llm::is_bonsai(id) {
        return install_bonsai(models_root, id, cancel);
    }
    let target = describe_target(models_root, id)?;
    check_cancel(cancel)?;
    let is_moonshine = moonshine_cdn_base(id).is_some();
    if is_moonshine {
        if crate::asr::model_ready(models_root, id) {
            return Ok(());
        }
        require_moonshine_runtime(models_root)?;
    }
    let is_diarization = id == diarization_model::ID;
    if target.needs_runtime && !is_diarization && !crate::optional_asr::runtime_ready(models_root) {
        return Err(
            "The speech runtime is not installed. Install one speech model via Setup once to fetch the runtime, then retry this install."
                .into(),
        );
    }
    // Insufficient-disk check before any bytes move.
    if let Some(free) = resource_manager::disk_free_bytes(&target.optional_root) {
        let need = target.total_bytes.saturating_add(256 << 20);
        if free < need {
            return Err(format!(
                "Not enough disk space: need about {} free, found {}. Free space and try again.",
                resource_manager::format_bytes(Some(need)),
                resource_manager::format_bytes(Some(free)),
            ));
        }
    }
    fs::create_dir_all(&target.optional_root)
        .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
    crate::optional_asr::ensure_within(models_root, &target.optional_root)?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if crate::optional_llm::find(id).is_some() {
        install_mac_runtime(models_root, id, target.total_bytes, cancel)?;
        if crate::cleanup::model_ready(models_root, id) {
            return Ok(());
        }
    }
    crate::optional_asr::ensure_within(models_root, &target.dest_dir)?;
    if is_moonshine {
        crate::optional_asr::ensure_within(&target.optional_root, &target.dest_dir)?;
    }
    if is_diarization && !diarization_model::MAC {
        let runtime_bytes = target.total_bytes - diarization_model::package_bytes();
        set_job_phase(id, Phase::Downloading, None);
        set_download_progress(id, 0, target.total_bytes);
        crate::optional_asr::ensure_runtime_with_cancel(models_root, cancel, &|done, _| {
            set_download_progress(id, done.min(runtime_bytes), target.total_bytes);
        })?;
        check_cancel(cancel)?;
        // If only the runtime was missing, the existing verified model wins.
        if diarization_model::installed(models_root) {
            set_job_progress(id, target.total_bytes, target.total_bytes);
            return Ok(());
        }
    }
    // Keep an incomplete download until the replacement is fully verified.
    // Installation stays atomic; Retry can repair a damaged managed package.
    // A diarization folder that is present but unverified (installed returned
    // above) is repaired the same way.
    let repair_existing = target.dest_dir.exists()
        && (cfg!(target_os = "macos") || is_diarization || is_moonshine);
    if target.dest_dir.exists() && !repair_existing { return Err("This model is already installed.".into()); }
    // Staging under the optional root keeps activation atomic.
    let staging =
        target
            .optional_root
            .join(format!(".install-{}-{}", target.id, crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
    let installed = (|| {
        if is_diarization && diarization_model::MAC {
            // The Mac package carries its own ONNX Runtime; it is fetched first
            // so the byte count below starts after it.
            set_job_phase(id, Phase::Downloading, None);
            let archive = staging.join("onnxruntime.tgz");
            download_cancellable(
                diarization_model::MAC_RUNTIME_URL,
                &archive,
                diarization_model::MAC_RUNTIME_ARCHIVE_BYTES,
                diarization_model::MAC_RUNTIME_ARCHIVE_SHA256,
                true,
                "ONNX Runtime",
                0,
                target.total_bytes,
                id,
                cancel,
            )?;
            diarization_model::extract_mac_runtime(&archive, &staging, cancel)?;
            fs::remove_file(&archive)
                .map_err(|e| format!("Cannot remove the ONNX Runtime archive: {e}"))?;
        }
        let mut done = target
            .total_bytes
            .saturating_sub(target.files.iter().map(|f| f.bytes).sum());
        for file in &target.files {
            check_cancel(cancel)?;
            set_job_phase(id, Phase::Downloading, None);
            let path = staging.join(file.relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot prepare {}: {e}", file.relative))?;
            }
            download_cancellable(
                &file.url,
                &path,
                file.bytes,
                file.sha256,
                false,
                "model",
                done,
                target.total_bytes,
                id,
                cancel,
            )?;
            done = done.saturating_add(file.bytes);
            set_download_progress(id, done, target.total_bytes);
        }
        check_cancel(cancel)?;
        set_job_phase(id, Phase::Verifying, None);
        for file in &target.files {
            check_cancel(cancel)?;
            if !crate::optional_asr::file_matches(
                &staging.join(file.relative),
                &crate::optional_asr::PinnedFile {
                    relative: file.relative,
                    bytes: file.bytes,
                    sha256: file.sha256,
                },
            )? {
                return Err(format!(
                    "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                    file.relative
                ));
            }
        }
        check_cancel(cancel)?;
        if is_moonshine {
            require_moonshine_runtime(models_root)?;
            crate::optional_asr::ensure_within(models_root, &target.optional_root)?;
            crate::optional_asr::ensure_within(&target.optional_root, &target.dest_dir)?;
            check_cancel(cancel)?;
        }
        let previous = target.optional_root.join(format!(".replaced-{}-{}", id, crate::store::new_id()));
        if target.dest_dir.exists() {
            if !repair_existing { return Err("This model is already installed.".into()); }
            crate::optional_asr::ensure_within(models_root, &target.dest_dir)?;
            fs::rename(&target.dest_dir, &previous).map_err(|e| format!("Cannot prepare model repair: {e}"))?;
        }
        if let Err(error) = fs::rename(&staging, &target.dest_dir) {
            if previous.exists() { let _ = fs::rename(&previous, &target.dest_dir); }
            return Err(format!("Cannot activate the downloaded model: {error}"));
        }
        // Keep the old damaged package for manual recovery. It is never loaded.

        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    if installed.is_ok() {
        set_job_progress(id, target.total_bytes, target.total_bytes);
    }
    installed
}

fn install_granite(models_root: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let id = granite_model::GRANITE_ID;
    check_cancel(cancel)?;
    let target = describe_target(models_root, id)?;
    // A repair keeps the previous package intact until its replacement passes
    // every hash. Archives are removed one at a time, so only the largest
    // archive must fit alongside the complete extracted package.
    let archive_bytes = granite_model::downloads().iter()
        .filter(|download| download.format != granite_model::Format::File)
        .map(|download| download.pin.bytes).max().unwrap_or(0);
    let need = granite_model::package_bytes()
        .saturating_add(archive_bytes)
        .saturating_add(256 << 20);
    let disk_path = models_root.ancestors().find(|path| path.is_dir()).unwrap_or(models_root);
    if resource_manager::disk_free_bytes(disk_path).is_some_and(|free| free < need) {
        return Err(format!("Not enough disk space for Granite. Free about {} and try again.", resource_manager::format_bytes(Some(need))));
    }
    fs::create_dir_all(models_root).map_err(|e| format!("Cannot prepare the models folder: {e}"))?;
    crate::optional_asr::ensure_within(models_root, &target.optional_root)?;
    fs::create_dir_all(&target.optional_root).map_err(|e| format!("Cannot prepare optional models: {e}"))?;
    crate::optional_asr::ensure_within(&target.optional_root, &target.dest_dir)?;
    let staging = target.optional_root.join(format!(".install-{id}-{}", crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot stage Granite: {e}"))?;
    let result = (|| {
        let mut done = 0;
        set_job_progress(id, 0, target.total_bytes);
        for download in granite_model::downloads() {
            check_cancel(cancel)?;
            set_job_phase(id, Phase::Downloading, None);
            let file = staging.join(download.pin.relative);
            download_cancellable(
                download.url, &file, download.pin.bytes, download.pin.sha256,
                download.url.starts_with("https://github.com/") || download.url.starts_with("https://raw.githubusercontent.com/"),
                "Granite", done, target.total_bytes, id, cancel,
            )?;
            if download.format != granite_model::Format::File {
                granite_model::extract(download, &file, &staging, cancel)?;
                fs::remove_file(&file).map_err(|e| format!("Cannot remove Granite's staging archive: {e}"))?;
            }
            done += download.pin.bytes;
            set_download_progress(id, done, target.total_bytes);
        }
        set_job_phase(id, Phase::Verifying, None);
        granite_model::verify_package(&staging, cancel)?;
        check_cancel(cancel)?;
        let (active, loaded) = resource_manager::model_state_blocking(id);
        if active || loaded {
            return Err("Granite is still running. Wait for meeting transcription to finish, then retry the install.".into());
        }
        crate::optional_asr::transactional_activate(models_root, &staging, &target.dest_dir, "Granite package")?;
        set_job_progress(id, target.total_bytes, target.total_bytes);
        Ok(())
    })();
    // The staging path is generated beneath this package's checked optional
    // root, and is consumed by transactional_activate on success.
    if staging.exists() {
        crate::optional_asr::ensure_within(&target.optional_root, &staging)?;
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

pub(crate) fn mac_runtime_download_bytes(root: &Path) -> u64 {
    if cfg!(all(target_os = "macos", target_arch = "aarch64"))
        && !crate::cleanup::mac_runtime_ready(root)
    {
        11_149_739
    } else {
        0
    }
}

// Pinned official llama.cpp Apple Silicon release, shared by speech and text.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn install_mac_runtime(
    root: &Path,
    id: &str,
    total: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if crate::cleanup::mac_runtime_ready(root) {
        return Ok(());
    }
    const BYTES: u64 = 11_149_739;
    const SHA: &str = "033c845c1df9bf945ff37bb193238b40910b2244be3e1e637b2ceb5878f1a6f5";
    let parent = root.join("llama.cpp/runtime");
    fs::create_dir_all(&parent).map_err(|e| format!("Cannot create Metal runtime folder: {e}"))?;
    crate::optional_asr::ensure_within(root, &parent)?;
    let staging = parent.join(format!(".install-metal-{}", crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let archive_path = staging.join("runtime.tar.gz");
        set_job_phase(id, Phase::Downloading, None);
        download_cancellable(
            "https://github.com/ggml-org/llama.cpp/releases/download/b10964/llama-b10964-bin-macos-arm64.tar.gz",
            &archive_path, BYTES, SHA, true, "Metal runtime", 0, total, id, cancel)?;
        set_job_phase(id, Phase::Verifying, None);
        let file = fs::File::open(&archive_path).map_err(|e| e.to_string())?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in archive.entries().map_err(|e| e.to_string())? {
            check_cancel(cancel)?;
            let mut entry = entry.map_err(|e| e.to_string())?;
            if !entry
                .unpack_in(&staging)
                .map_err(|e| format!("Cannot unpack Metal runtime: {e}"))?
            {
                return Err("The Metal runtime archive contains an invalid path.".into());
            }
        }
        let unpacked = staging.join("llama-b10964");
        if !unpacked.join("llama-server").is_file()
            || !unpacked.join("libggml-metal.dylib").is_file()
        {
            return Err("The downloaded runtime is missing its server or Metal library.".into());
        }
        check_cancel(cancel)?;
        let dest = parent.join("mac-arm64");
        fs::write(unpacked.join("sonojot-runtime-release"), "b10964\n").map_err(|e| e.to_string())?;
        let previous = parent.join(format!(".replaced-metal-{}", crate::store::new_id()));
        if dest.exists() {
            crate::optional_asr::ensure_within(root, &dest)?;
            fs::rename(&dest, &previous).map_err(|e| format!("Cannot prepare runtime repair: {e}"))?;
        }
        if let Err(error) = fs::rename(unpacked, &dest) {
            if previous.exists() { let _ = fs::rename(&previous, &dest); }
            return Err(format!("Cannot activate Metal runtime: {error}"));
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(staging);
    result
}

/// Install the Bonsai weights and its PrismML server runtime as one guarded
/// model operation. The stock bundled server is never used for this format.
#[cfg(all(windows, target_arch = "x86_64"))]
fn install_bonsai(models_root: &Path, id: &str, cancel: &AtomicBool) -> Result<(), String> {
    let model = crate::optional_llm::find(id).ok_or("Unknown Bonsai model.")?;
    let optional_root = crate::optional_asr::optional_root(models_root);
    fs::create_dir_all(&optional_root)
        .map_err(|e| format!("Cannot create optional models folder: {e}"))?;
    let total = crate::optional_llm::BONSAI_RUNTIME_BYTES
        .saturating_add(crate::optional_llm::BONSAI_CUDA_BYTES)
        .saturating_add(model.file.bytes);
    if let Some(free) = resource_manager::disk_free_bytes(&optional_root) {
        let need = total.saturating_add(512 << 20);
        if free < need {
            return Err(format!(
                "Not enough disk space: need about {} free, found {}.",
                resource_manager::format_bytes(Some(need)),
                resource_manager::format_bytes(Some(free))
            ));
        }
    }
    let runtime_staging = optional_root.join(format!(".bonsai-runtime-{}", crate::store::new_id()));
    fs::create_dir(&runtime_staging)
        .map_err(|e| format!("Cannot create runtime staging folder: {e}"))?;
    let runtime_zip = runtime_staging.join("runtime.zip");
    let result = (|| {
        set_job_phase(id, Phase::Downloading, None);
        download_cancellable(
            crate::optional_llm::BONSAI_RUNTIME_URL,
            &runtime_zip,
            crate::optional_llm::BONSAI_RUNTIME_BYTES,
            crate::optional_llm::BONSAI_RUNTIME_SHA256,
            true,
            "PrismML Bonsai runtime",
            0,
            total,
            id,
            cancel,
        )?;
        let cuda_zip = runtime_staging.join("cuda-runtime.zip");
        download_cancellable(
            crate::optional_llm::BONSAI_CUDA_URL,
            &cuda_zip,
            crate::optional_llm::BONSAI_CUDA_BYTES,
            crate::optional_llm::BONSAI_CUDA_SHA256,
            true,
            "PrismML CUDA runtime",
            crate::optional_llm::BONSAI_RUNTIME_BYTES,
            total,
            id,
            cancel,
        )?;
        let runtime_files = runtime_staging.join("files");
        fs::create_dir(&runtime_files).map_err(|e| format!("Cannot prepare Prism runtime: {e}"))?;
        let archive_file =
            fs::File::open(&runtime_zip).map_err(|e| format!("Cannot open Prism runtime: {e}"))?;
        let mut archive = zip::ZipArchive::new(archive_file)
            .map_err(|e| format!("Cannot read Prism runtime: {e}"))?;
        for index in 0..archive.len() {
            check_cancel(cancel)?;
            let mut entry = archive
                .by_index(index)
                .map_err(|e| format!("Cannot read Prism runtime entry: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().replace('\\', "/");
            let relative = std::path::Path::new(&name);
            if relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err("Prism runtime contains an unsafe path.".into());
            }
            let destination = runtime_files.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut output = fs::File::create(&destination).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
        }
        let cuda_file = fs::File::open(&cuda_zip)
            .map_err(|e| format!("Cannot open Prism CUDA runtime: {e}"))?;
        let mut cuda_archive = zip::ZipArchive::new(cuda_file)
            .map_err(|e| format!("Cannot read Prism CUDA runtime: {e}"))?;
        for index in 0..cuda_archive.len() {
            check_cancel(cancel)?;
            let mut entry = cuda_archive
                .by_index(index)
                .map_err(|e| format!("Cannot read Prism CUDA entry: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().replace('\\', "/");
            let relative = std::path::Path::new(&name);
            if relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err("Prism CUDA runtime contains an unsafe path.".into());
            }
            let destination = runtime_files.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut output = fs::File::create(&destination).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
        }
        let server = runtime_files.join(if cfg!(windows) {
            "llama-server.exe"
        } else {
            "llama-server"
        });
        if !server.is_file() {
            return Err("The PrismML runtime archive did not contain llama-server.".into());
        }
        let runtime_dest = optional_root.join("bonsai-prism-runtime");
        crate::optional_asr::transactional_activate(
            models_root,
            &runtime_files,
            &runtime_dest,
            "Bonsai Prism runtime",
        )?;
        let model_staging =
            optional_root.join(format!(".install-{}-{}", id, crate::store::new_id()));
        fs::create_dir(&model_staging).map_err(|e| e.to_string())?;
        let model_path = model_staging.join(model.file.relative);
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            model.repo, model.revision, model.file.relative
        );
        download_cancellable(
            &url,
            &model_path,
            model.file.bytes,
            model.file.sha256,
            false,
            "Bonsai model",
            crate::optional_llm::BONSAI_RUNTIME_BYTES
                .saturating_add(crate::optional_llm::BONSAI_CUDA_BYTES),
            total,
            id,
            cancel,
        )?;
        if !crate::optional_asr::file_matches(&model_path, &model.file)? {
            return Err("The Bonsai model checksum did not match.".into());
        }
        crate::optional_asr::transactional_activate(
            models_root,
            &model_staging,
            &crate::optional_llm::model_dir(models_root, id),
            "Bonsai model",
        )?;
        set_job_progress(id, total, total);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&runtime_staging);
    result
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn install_bonsai(_models_root: &Path, _id: &str, _cancel: &AtomicBool) -> Result<(), String> {
    Err("Bonsai 2 requires the pinned PrismML runtime on 64-bit Windows.".into())
}

/// Start returns promptly; poll `get_model_library` for progress.
pub fn start_install(models_root: PathBuf, id: String) -> Result<(), String> {
    crate::download_consent::require_sherpa_runtime(&models_root, &id)?;
    let activity = resource_manager::begin_update_activity(resource_manager::UpdateActivity::Models)?;
    if cfg!(target_os = "macos") && (!cfg!(target_arch = "aarch64") || !mac_supported(&id)) {
        return Err(
            "This model package is not available on this Mac. Choose an Apple Metal model.".into(),
        );
    }

    // Real shared guard: hold the shared selection/mutation guard across the
    // check + marker-insert critical section (short). The job-registry marker
    // carries exclusion for the download duration; selection
    // (`with_model_selection`) checks both the guard and the marker, so no
    // check-then-act race remains. Loads use only the coordinator and proceed
    // concurrently with downloads.
    let _shared = crate::optional_asr::try_install_gate()
        .map_err(|_| "A model is already being installed.".to_string())?;
    {
        let mut jobs = install_jobs()
            .lock()
            .map_err(|_| "Install state lock failed.".to_string())?;
        if jobs.values().any(|j| {
            matches!(
                j.phase,
                Phase::Downloading | Phase::Verifying | Phase::Removing
            )
        }) {
            return Err("A model is already being installed.".into());
        }
        if jobs.get(&id).is_some_and(|j| {
            matches!(
                j.phase,
                Phase::Downloading | Phase::Verifying | Phase::Removing
            )
        }) {
            return Err("This model is already being installed.".into());
        }
        drop(jobs);
        let target = describe_target(&models_root, &id)?;
        if moonshine_cdn_base(&id).is_some() {
            // A working bundled/located copy wins even when the managed
            // destination does not exist. Install never replaces that copy.
            if crate::asr::model_ready(&models_root, &id) {
                return Err("This model is already installed.".into());
            }
            require_moonshine_runtime(&models_root)?;
        }
        if target.dest_dir.exists() {
            // Distinguish already-installed from a stale failed job.
            let ready = if moonshine_cdn_base(&id).is_some() {
                crate::asr::model_ready(&models_root, &id)
            } else if id == diarization_model::ID {
                diarization_model::installed(&models_root)
            } else if id == granite_model::GRANITE_ID {
                granite_model::installed(&models_root)
            } else if let Some(package) = crate::punctuation_model::Package::from_id(&id) {
                package.model_status(&models_root, Check::Blocking).ready()
            } else if crate::mac_speech::find(&id).is_some() {
                crate::mac_speech::model_ready(&models_root, &id)
            } else if crate::llama_asr::is_llama_asr(&id) {
                crate::llama_asr::model_ready(&models_root, &id)
            } else if crate::gpu_asr::is_gpu_model(&id) {
                crate::gpu_asr::model_ready(&models_root, &id)
            } else if crate::whisper_catalog::is_whisper(&id) {
                crate::whisper_catalog::model_ready(&models_root, &id)
            } else if target.needs_runtime {
                crate::optional_asr::model_ready(&models_root, &id)
            } else {
                crate::cleanup::model_ready(&models_root, &id)
                    || (!cfg!(target_os = "macos")
                        && crate::optional_llm::optional_copy_ready(&models_root, &id))
            };
            if ready {
                return Err("This model is already installed.".into());
            }
        }
        // Downloads never need runtime RAM/VRAM fit and never stop active
        // work. Disk is gated in the worker; fit is advisory (catalog
        // warning via `vram_fit_warning`), enforced only at LOAD. Active
        // inference proceeds concurrently with downloads; partial staging
        // lives under `.install-*` so loads observe the last verified state.
        let _fit_warning = if crate::gpu_asr::is_gpu_model(&id)
            || crate::llama_asr::is_llama_asr(&id)
            || id == granite_model::GRANITE_ID
        {
            resource_manager::vram_fit_warning(&id, "GPU speech model install")
        } else {
            None
        };
        let _ = _fit_warning;
        let mut jobs = install_jobs().lock().map_err(|_| "Install state lock failed.".to_string())?;
        jobs.insert(
            id.clone(),
            JobState {
                phase: Phase::Downloading,
                downloaded_bytes: 0,
                total_bytes: target.total_bytes,
                error: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
    }
    let cancel = install_jobs()
        .lock()
        .ok()
        .and_then(|jobs| jobs.get(&id).map(|j| j.cancel.clone()))
        .ok_or("Install state was lost.".to_string())?;
    let worker_id = id.clone();
    if let Err(error) = std::thread::Builder::new()
        .name(format!("model-install-{id}"))
        .spawn(move || {
            let _activity = activity;
            install_blocking(models_root, worker_id, cancel)
        })
    {
        let message = format!("Cannot start the install worker: {error}");
        set_job_phase(&id, Phase::Failed, Some(message.clone()));
        return Err(message);
    }
    Ok(())
}

pub fn cancel_install(id: &str) -> bool {
    if let Ok(jobs) = install_jobs().lock() {
        if let Some(job) = jobs.get(id) {
            job.cancel.store(true, Ordering::Release);
            return true;
        }
    }
    false
}

/// Public gate for parent-owned selection paths (`commands.rs::save_settings`,
/// applied by the parent after the export worker lands). True while any
/// install/remove holds the registry marker (`Downloading`/`Verifying`/
/// `Removing`). Selection (model-id changes) must fail fast while true —
/// same message as installs — instead of racing file activation. Load paths
/// (`acquire_*`) are already serialized via `busy_blocking` on the install
/// side; partial staging lives under `.install-*` so loads observe the last
/// verified state, never partial bytes.
pub fn install_in_progress() -> bool {
    install_jobs().lock().ok().is_some_and(|jobs| {
        jobs.values().any(|j| {
            matches!(
                j.phase,
                Phase::Downloading | Phase::Verifying | Phase::Removing
            )
        })
    })
}

/// Safe removal: no silent fallback. `confirmed=false` is rejected first for
/// EVERY deletion (even unselected) so files can never disappear without an
/// explicit Setup confirmation. If the model is still selected in any role,
/// removal stays blocked even when confirmed until the user picks an explicit
/// replacement. Blocking ownership checks (fail-closed) serialize with
/// install/selection/loading; try_lock hints never authorize destruction.
///
/// Ordering: the still-selected check MUST read CURRENT selections inside the
/// shared guard. The Tauri command route uses `remove_model_blocking_with_store`
/// (guard first, then fresh settings read) so a selection commit that lands
/// between the UI's library read and this removal cannot leave us deleting a
/// just-selected model on stale data. This direct variant is for callers that
/// already hold current settings (tests, single-threaded flows).
pub fn remove_model_blocking(
    models_root: &Path,
    settings: &Settings,
    id: &str,
    confirmed: bool,
) -> Result<(), String> {
    // 1. Explicit confirmation first, for every deletion.
    if !confirmed {
        return Err(
            "Confirm removal in Setup before deleting model files. Nothing was deleted.".into(),
        );
    }
    // 2. Serialize with installs/selection. Hold the shared guard for the
    // whole removal (same thread, synchronous) and place a Removing marker
    // so a concurrent model_manager install for any id fails fast instead of
    // racing deletion.
    let _legacy_gate = crate::optional_asr::try_install_gate()
        .map_err(|_| "A model is being installed. Cancel the install before removing it.")?;
    remove_model_guarded(models_root, settings, id)
}

/// Guard-first removal for the live command route: acquires the shared
/// selection/mutation guard BEFORE reading selections, then re-reads CURRENT
/// settings from the store inside the guard. A queued removal therefore uses
/// fresh selections even when a `save_settings` commit lands after the UI's
/// library snapshot but before this delete runs.
pub fn remove_model_blocking_with_store(
    models_root: &Path,
    store: &crate::store::Store,
    id: &str,
    confirmed: bool,
) -> Result<(), String> {
    if !confirmed {
        return Err(
            "Confirm removal in Setup before deleting model files. Nothing was deleted.".into(),
        );
    }
    let _legacy_gate = crate::optional_asr::try_install_gate()
        .map_err(|_| "A model is being installed. Cancel the install before removing it.")?;
    // Fresh read INSIDE the guard: holds the selection commit out while the
    // current selections are observed, so stale UI data cannot authorize a
    // delete of a just-selected model.
    let settings = store.load_settings()?;
    remove_model_guarded(models_root, &settings, id)
}

/// Still-selected check + Removing marker + physical delete. Caller holds the
/// shared guard across this whole call.
fn remove_model_guarded(models_root: &Path, settings: &Settings, id: &str) -> Result<(), String> {
    let _activity = resource_manager::begin_update_activity(resource_manager::UpdateActivity::Models)?;
    // 2. No silent fallback: still-selected/shared stays blocked.
    let roles = used_by(settings, id);
    if !roles.is_empty() {
        return Err(format!(
            "This model is still selected for {}. Select a replacement in Setup before removing it; SonoJot never switches models silently.",
            roles.join(", ")
        ));
    }
    // 3. Removing marker so a concurrent install for any id fails fast.
    {
        let mut jobs = install_jobs()
            .lock()
            .map_err(|_| "Install state lock failed.".to_string())?;
        if jobs.values().any(|j| {
            matches!(
                j.phase,
                Phase::Downloading | Phase::Verifying | Phase::Removing
            )
        }) {
            return Err(
                "A model is being installed. Cancel the install before removing it.".into(),
            );
        }
        jobs.insert(
            id.to_owned(),
            JobState {
                phase: Phase::Removing,
                downloaded_bytes: 0,
                total_bytes: 0,
                error: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
    }
    let result = remove_inner(models_root, id);
    if let Ok(mut jobs) = install_jobs().lock() {
        jobs.remove(id);
    }
    result
}

fn remove_inner(models_root: &Path, id: &str) -> Result<(), String> {
    // 4. Blocking ownership: fail-closed on contention. Active inference for
    // this id blocks; a merely loaded (resident but idle) text worker is
    // confirmed-unloaded via stop_loaded_model before deletion, while a
    // resident ASR recognizer (owned by speech.rs, CPU or CUDA) blocks until
    // speech confirms release — deleting under a live recognizer is never
    // allowed.
    let (active, loaded) = resource_manager::model_state_blocking(id);
    if active {
        return Err("This model is running now. Stop dictation or notes, then try again.".into());
    }
    if let Some(package) = crate::punctuation_model::Package::from_id(id) {
        if loaded || resource_manager::busy_blocking() {
            return Err("Punctuation may still be running. Wait for dictation or notes to finish, then try again.".into());
        }
        // The command has already checked capture idle. Release the idle warm
        // helper before deleting its graph; missing files then use local rules.
        crate::punctuation::release_cached();
        let dest = package.path(models_root);
        if dest.exists() {
            let parent = models_root.join("punctuation");
            crate::optional_asr::ensure_within(models_root, &parent)?;
            crate::optional_asr::ensure_within(&parent, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The punctuation files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        invalidate_sizes();
        return Ok(());
    }
    if id == diarization_model::ID {
        if loaded {
            return Err("Speaker labels are still running. Wait for transcription to finish, then try again.".into());
        }
        let dest = diarization_model::path(models_root);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        // sherpa-runtime is shared with the optional speech models.
        resource_manager::note_model_removed(id);
        invalidate_sizes();
        return Ok(());
    }
    if id == granite_model::GRANITE_ID {
        if loaded {
            return Err("Granite is still running. Wait for meeting transcription to finish, then try again.".into());
        }
        let dest = granite_model::path(models_root);
        if dest.exists() {
            let optional = crate::optional_asr::optional_root(models_root);
            crate::optional_asr::ensure_within(models_root, &optional)?;
            crate::optional_asr::ensure_within(&optional, &dest)?;
            fs::remove_dir_all(&dest).map_err(|e| format!("The Granite package could not be removed: {e}"))?;
        }
        // All Granite libraries and notices are private to this package.
        resource_manager::note_model_removed(id);
        invalidate_sizes();
        return Ok(());
    }
    // Only optional downloads + real Whisper checkpoints + real GPU models are
    // removable. Moonshine (bundled or managed) and local-rules stay protected.
    // The shared CUDA runtime is never deleted by a single-model
    // remove (both GPU models share it).
    if crate::optional_llm::find(id).is_some() {
        crate::cleanup::stop_loaded_model(id);
        let dest = crate::optional_llm::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        resource_manager::touch_activity(resource_manager::Stage::Cleanup);
        return Ok(());
    }
    if crate::mac_speech::find(id).is_some() {
        if loaded {
            return Err(
                "This speech model is still loaded. Wait for idle release, then try again.".into(),
            );
        }
        let dest = crate::mac_speech::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        return Ok(());
    }
    if crate::optional_asr::find(id).is_some() {
        if loaded {
            return Err(
                "This speech model is still loaded. Wait for idle release, then try again.".into(),
            );
        }
        let dest = crate::optional_asr::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        resource_manager::touch_activity(resource_manager::Stage::Asr);
        return Ok(());
    }
    if crate::whisper_catalog::is_whisper(id) {
        if loaded {
            return Err(
                "This speech model is still loaded. Wait for idle release, then try again.".into(),
            );
        }
        let dest = crate::whisper_catalog::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        resource_manager::touch_activity(resource_manager::Stage::Asr);
        return Ok(());
    }
    if crate::llama_asr::is_llama_asr(id) {
        if loaded {
            return Err(
                "This GPU speech model is still loaded. Wait for idle release, then try again."
                    .into(),
            );
        }
        let dest = crate::llama_asr::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        // Keep shared runtimes (Windows CPU VAD and separately managed llama).
        resource_manager::note_model_removed(id);
        resource_manager::touch_activity(resource_manager::Stage::Asr);
        invalidate_sizes();
        return Ok(());
    }
    if crate::gpu_asr::is_gpu_model(id) {
        if loaded {
            return Err(
                "This GPU speech model is still loaded. Wait for idle release, then try again."
                    .into(),
            );
        }
        let dest = crate::gpu_asr::model_dir(models_root, id);
        if dest.exists() {
            crate::optional_asr::ensure_within(models_root, &dest)?;
            fs::remove_dir_all(&dest)
                .map_err(|e| format!("The model files could not be removed: {e}"))?;
        }
        resource_manager::note_model_removed(id);
        resource_manager::touch_activity(resource_manager::Stage::Asr);
        return Ok(());
    }
    Err("This model cannot be removed: it is bundled, rules-only, or not installable.".into())
}

// ---------------------------------------------------------------------------
// Tauri commands: exact schema from contracts.md. Parent registers these in
// lib.rs invoke_handler. Each returns the full library so Setup can poll.
// ---------------------------------------------------------------------------

fn runtime_snapshot(runtime: &crate::app::Runtime) -> Result<(PathBuf, Settings), String> {
    let snapshot = runtime.snapshot()?;
    Ok((runtime.models_root.clone(), snapshot.settings))
}

pub(crate) fn require_idle_snapshot(snapshot: &crate::domain::Snapshot) -> Result<(), String> {
    if !matches!(
        snapshot.state,
        crate::domain::AppState::Idle | crate::domain::AppState::Error
    ) || snapshot.meeting_recording
    {
        return Err("Finish recording before changing models.".into());
    }
    // Fail-closed blocking check: try_lock hints must not authorize mutation.
    if resource_manager::busy_blocking() {
        return Err(
            "Local inference is busy. Wait for it to finish before changing models.".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn get_model_library(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
) -> Result<ModelLibrary, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (root, settings) = runtime_snapshot(&runtime)?;
        let mut library = build_library(&root, &settings);
        crate::onboarding::show_queue(&runtime, &mut library);
        Ok(library)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn start_model_install(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
    id: String,
) -> Result<ModelLibrary, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        require_idle_snapshot(&runtime.snapshot()?)?;
        let (root, settings) = runtime_snapshot(&runtime)?;
        crate::download_consent::require_components(&runtime, &[&id])?;
        start_install(root.clone(), id)?;
        let mut library = build_library(&root, &settings);
        crate::onboarding::show_queue(&runtime, &mut library);
        Ok(library)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn cancel_model_install(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
    id: String,
) -> Result<ModelLibrary, String> {
    let runtime = state.inner().clone();
    crate::onboarding::cancel_queued(&runtime, &id)?;
    tauri::async_runtime::spawn_blocking(move || {
        cancel_install(&id);
        // Give the worker a beat to observe cancellation; polling continues.
        std::thread::sleep(Duration::from_millis(30));
        let (root, settings) = runtime_snapshot(&runtime)?;
        let mut library = build_library(&root, &settings);
        crate::onboarding::show_queue(&runtime, &mut library);
        Ok(library)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remove_model(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
    id: String,
    confirmed: bool,
) -> Result<ModelLibrary, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        require_idle_snapshot(&runtime.snapshot()?)?;
        // Guard-first route: selections are re-read from the store INSIDE the
        // shared guard, so a selection commit racing this delete cannot leave
        // a just-selected model deleted on stale UI data.
        let root = runtime.models_root.clone();
        remove_model_blocking_with_store(&root, &runtime.store, &id, confirmed)?;
        let (root, settings) = runtime_snapshot(&runtime)?;
        Ok(build_library(&root, &settings))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Requires an explicitly supplied model root and installed Mac model"]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn mac_text_native_smoke() {
        let root = std::path::PathBuf::from(std::env::var_os("SONOJOT_NATIVE_MODELS_ROOT").expect("explicit test model root"));
        let result = crate::cleanup::generate_local(&root, "lfm2.5-1.2b",
            "Repeat the user's sentence exactly. Do not add commentary.",
            "The small garden is quiet today.", 40, &std::sync::atomic::AtomicBool::new(false));
        let status = crate::acceleration::inspect(&root);
        let erase = crate::cleanup::last_erase_report();
        crate::cleanup::shutdown();
        let text = result.expect("local text generation");
        assert!(text.contains("garden"), "{text}");
        let runtime = status.text_runtime.expect("resident Metal text runtime");
        assert_eq!(runtime.backend, "metal");
        assert!(runtime.gpu_layers.is_some_and(|n| n > 0));
        assert!(erase.erased || erase.shutdown_fallback, "cache must be erased or worker stopped");
        println!("Text: {text}\nRuntime: {}\nErase: {erase:?}", serde_json::to_string(&runtime).unwrap());
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn mac_catalog_prioritizes_supported_installs() {
        let root = std::env::temp_dir().join(format!("sonojot-catalog-{}", crate::store::new_id()));
        let library = super::build_library(&root, &crate::domain::Settings::default());
        assert!(library.hardware.unified_memory);
        assert_eq!(library.models[0].id, "whisper-small-metal");
        assert_eq!(library.models[1].id, "whisper-turbo-metal");
        let llama = library.models.iter().find(|m| m.id == crate::llama_asr::MODEL_ID).unwrap();
        assert!(!llama.installable);
        assert_eq!(llama.state, "unavailable");
        assert!(llama.compatibility.contains("requires Windows"));
        for model in &library.models {
            if model.installable {
                assert!(super::mac_supported(&model.id), "{}", model.id);
            }
        }
        let target = super::describe_target(&root, "whisper-small-metal").unwrap();
        assert!(!target.needs_runtime);
        assert_eq!(target.files.len(), 1);
        assert_eq!(target.total_bytes, 487_601_967);
    }

    use super::*;
    use crate::domain::{MemoryMode, ModelRetention};

    fn isolated_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "sonojot-modelmgr-{name}-{}",
            crate::store::new_id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(root.join("optional")).unwrap();
        root
    }

    /// Global install/remove registry and ASR lease state are process-wide;
    /// share the resource manager's gate so tests in other modules cannot
    /// leave a model marked loaded while a removal test runs.
    fn test_gate() -> std::sync::MutexGuard<'static, ()> {
        crate::resource_manager::tests::test_gate()
    }

    #[test]
    fn punctuation_catalog_offers_the_pinned_download() {
        let _gate = test_gate();
        let root = isolated_root("punctuation-catalog");
        let entries = punctuation_entries(&root);
        assert_eq!(entries.len(), 2);
        let entry = &entries[0];
        assert_eq!(entry.id, "sonojot-punct-v1");
        assert_eq!(entry.kind, "punctuation");
        assert!(crate::punctuation_model::released(), "The upload commit must be pinned before shipping.");
        assert_eq!(entry.installable, diarization_model::AVAILABLE);
        assert!(!entry.removable);
        assert!(entry.used_by.is_empty());
        assert_eq!(entry.state, if diarization_model::AVAILABLE { "not-installed" } else { "unavailable" });
        assert_eq!(entry.disk_bytes, 302_708_257);
        let runtime = if diarization_model::MAC { diarization_model::MAC_RUNTIME_ARCHIVE_BYTES } else { 8_179_934 };
        assert_eq!(entry.total_bytes, entry.disk_bytes + runtime);
        assert_eq!(entry.ram_min_bytes, Some(400_000_000));
        assert_eq!(entry.ram_max_bytes, Some(400_000_000));
        assert_eq!(entry.quantization, "embedding-only int8");
        assert!(entry.evidence.contains("CC BY-SA 4.0"));
        assert!(entry.evidence.contains("https://huggingface.co/SonoJot/sonojot-punct-v1"));
        assert_eq!(entry.revision, crate::punctuation_model::REVISION);
        if diarization_model::AVAILABLE {
            let target = describe_target(&root, &entry.id).unwrap();
            assert_eq!(target.dest_dir, root.join("punctuation").join(&entry.id));
            assert_eq!(target.files.len(), 3);
            for (file, pin) in target.files.iter().zip(crate::punctuation_model::PINS) {
                assert_eq!(file.relative, pin.relative);
                assert_eq!(file.bytes, pin.bytes);
                assert_eq!(file.sha256, pin.sha256);
                assert_eq!(file.url, crate::punctuation_model::download_url(pin));
            }
        } else {
            let error = describe_target(&root, &entry.id).err().unwrap();
            assert!(error.contains("requires Windows x64"), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn punctuation_cancelled_worker_preserves_package_and_retry_state() {
        let _gate = test_gate();
        let root = isolated_root("punctuation-cancel");
        let id = crate::punctuation_model::ID;
        let dest = crate::punctuation_model::path(&root);
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("old-model"), b"preserve").unwrap();
        let total = crate::punctuation_model::download_bytes(&root, Check::Background);
        let cancel = Arc::new(AtomicBool::new(false));
        install_jobs().lock().unwrap().insert(id.into(), JobState {
            phase: Phase::Downloading, downloaded_bytes: 0, total_bytes: total,
            error: None, cancel: cancel.clone(),
        });
        set_download_progress(id, u64::MAX, total);
        assert_eq!(punctuation_entries(&root)[0].downloaded_bytes, total - 1);
        assert!(cancel_install(id));
        install_blocking(root.clone(), id.into(), cancel);
        let row = punctuation_entries(&root).remove(0);
        assert_eq!(job_snapshot(id).unwrap().0, Phase::Failed);
        if diarization_model::AVAILABLE { assert_eq!(row.state, "cancelled"); }
        assert!(row.downloaded_bytes < row.total_bytes);
        assert_eq!(fs::read(dest.join("old-model")).unwrap(), b"preserve");
        assert_eq!(fs::read_dir(root.join("punctuation")).unwrap().count(), 1);
        assert!(!crate::optional_asr::runtime_dir(&root).exists());
        install_jobs().lock().unwrap().remove(id);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn punctuation_v2_catalog_offers_the_pinned_download() {
        use crate::punctuation_model::Package;
        let _gate = test_gate();
        let root = isolated_root("punctuation-v2-catalog");
        let entry = punctuation_entry(&root, Package::V2);
        assert_eq!(entry.id, crate::punctuation_model::V2_ID);
        assert_eq!(entry.kind, "punctuation");
        assert_eq!(entry.disk_bytes, 302_728_954);
        assert_eq!(entry.installable, diarization_model::AVAILABLE);
        assert!(!entry.removable);
        assert!(entry.used_by.is_empty());
        assert_eq!(entry.state, if diarization_model::AVAILABLE { "not-installed" } else { "unavailable" });
        let runtime = if diarization_model::MAC { diarization_model::MAC_RUNTIME_ARCHIVE_BYTES } else { 8_179_934 };
        assert_eq!(entry.total_bytes, entry.disk_bytes + runtime);
        assert_eq!(entry.revision, "94d84b4b17d4d2fb3cee46dbd7a4425b8a6ed625");
        assert!(!entry.guidance.contains("not available yet"));
        assert!(entry.evidence.contains("CC BY-SA 4.0"));
        assert!(entry.evidence.contains(Package::V2.page_url()));
        if diarization_model::AVAILABLE {
            let target = describe_target(&root, &entry.id).unwrap();
            assert_eq!(target.dest_dir, Package::V2.path(&root));
            assert_eq!(target.total_bytes, entry.total_bytes);
            assert_eq!(target.files.len(), 3);
            for (file, pin) in target.files.iter().zip(Package::V2.pins()) {
                assert_eq!(file.relative, pin.relative);
                assert_eq!(file.bytes, pin.bytes);
                assert_eq!(file.sha256, pin.sha256);
                assert_eq!(file.url, Package::V2.download_url(pin).unwrap());
                assert!(!file.url.contains("/resolve/main/"));
            }
        } else {
            let error = describe_target(&root, &entry.id).err().unwrap();
            assert!(error.contains("requires Windows x64"), "{error}");
        }
        assert!(job_snapshot(&entry.id).is_none());
        assert!(!root.join("punctuation").exists());
        assert!(!crate::optional_asr::runtime_dir(&root).exists());
        assert!(mac_supported(&entry.id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn punctuation_manual_v2_ready_state_preserves_manifest_sizes() {
        use crate::punctuation_model::{Package, FILES};
        let _gate = test_gate();
        let root = isolated_root("punctuation-v2-manual");
        let directory = Package::V2.path(&root);
        fs::create_dir_all(&directory).unwrap();
        let manifest = serde_json::json!({"schema": 1, "id": Package::V2.id(), "files": FILES.map(|name|
            serde_json::json!({"name": name, "bytes": 3, "sha256": "a".repeat(64)}))});
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        // Package and runtime verification are tested in punctuation_model;
        // inject readiness here to check catalog state without a large runtime.
        let row = punctuation_entry_with_status(&root, Package::V2, Status::Ready);
        assert_eq!(row.installable, diarization_model::AVAILABLE);
        assert!(row.removable);
        assert_eq!(row.disk_bytes, 9);
        assert_eq!((row.downloaded_bytes, row.total_bytes), (9, 9));
        if diarization_model::AVAILABLE {
            assert_eq!(row.state, "installed");
            let checking = punctuation_entry_with_status(&root, Package::V2, Status::Checking);
            assert_eq!(checking.state, "checking");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn punctuation_v2_removal_keeps_v1_and_shared_runtime() {
        use crate::punctuation_model::Package;
        let _gate = test_gate();
        let root = isolated_root("punctuation-v2-remove");
        let runtime = crate::optional_asr::runtime_dir(&root);
        for directory in [Package::V1.path(&root), Package::V2.path(&root), runtime.clone()] {
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("preserve"), b"abc").unwrap();
        }
        assert!(punctuation_entry(&root, Package::V2).removable);
        let settings = Settings::default();
        assert!(remove_model_blocking(&root, &settings, Package::V2.id(), false).is_err());
        assert!(Package::V2.path(&root).exists());
        remove_model_blocking(&root, &settings, Package::V2.id(), true).unwrap();
        assert!(!Package::V2.path(&root).exists());
        assert_eq!(fs::read(Package::V1.path(&root).join("preserve")).unwrap(), b"abc");
        assert_eq!(fs::read(runtime.join("preserve")).unwrap(), b"abc");
        assert!(!punctuation_entry(&root, Package::V2).removable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn punctuation_removal_requires_confirmation_and_keeps_shared_runtime() {
        let _gate = test_gate();
        let root = isolated_root("punctuation-remove");
        let id = crate::punctuation_model::ID;
        let dest = crate::punctuation_model::path(&root);
        let runtime = crate::optional_asr::runtime_dir(&root);
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("model.int8.onnx"), b"partial").unwrap();
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("shared-runtime"), b"preserve").unwrap();
        assert!(punctuation_entries(&root)[0].removable);
        let settings = Settings::default();
        assert!(remove_model_blocking(&root, &settings, id, false).is_err());
        assert!(dest.exists());
        remove_model_blocking(&root, &settings, id, true).unwrap();
        assert!(!dest.exists());
        assert!(!punctuation_entries(&root)[0].removable);
        assert_eq!(crate::punctuation_model::model_status(&root, Check::Blocking), Status::Missing);
        assert_eq!(fs::read(runtime.join("shared-runtime")).unwrap(), b"preserve");
        fs::remove_dir_all(root).unwrap();
    }

    /// Parent-owned opt-in smoke test. Uses the same public entry as Setup and
    /// leaves the verified isolated package for a subsequent fidelity_eval replay.
    #[cfg(feature = "fidelity-eval")]
    #[test]
    #[ignore = "Downloads the release into an explicitly supplied existing artifacts models root"]
    fn punctuation_real_download_into_artifacts() {
        let _gate = test_gate();
        let supplied = PathBuf::from(std::env::var_os("SONOJOT_PUNCT_DOWNLOAD_ROOT")
            .expect("set SONOJOT_PUNCT_DOWNLOAD_ROOT to an isolated artifacts directory"));
        assert!(supplied.is_absolute(), "models root must be absolute");
        let root = fs::canonicalize(supplied).expect("create the isolated models root first");
        let artifacts = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts")).unwrap();
        assert!(root.starts_with(&artifacts) && root != artifacts, "root must be inside this worktree's artifacts");
        let id = crate::punctuation_model::ID;
        assert!(!crate::punctuation_model::path(&root).exists(), "use a fresh punctuation destination");
        start_install(root.clone(), id.into()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(60 * 60);
        loop {
            match job_snapshot(id) {
                Some((Phase::Failed, _, _, error)) => panic!("download failed: {error:?}"),
                None => break,
                _ if std::time::Instant::now() >= deadline => {
                    cancel_install(id);
                    panic!("download exceeded the smoke-test deadline");
                }
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        assert!(crate::punctuation_model::model_status(&root, Check::Blocking).ready());
        assert_eq!(crate::punctuation_model::package_bytes(&root), 302_708_257);
        assert_eq!(fs::read_dir(crate::punctuation_model::path(&root)).unwrap().count(), 4);
        println!("Verified punctuation package: {}", crate::punctuation_model::path(&root).display());
    }

    #[test]
    fn pending_verification_is_distinct_from_missing_and_download_verification() {
        let _gate = test_gate();
        let id = "verification-status-fixture";
        assert_eq!(status_entry_state(id, Status::Checking, true).0, "checking");
        assert_eq!(status_entry_state(id, Status::Missing, true).0, "not-installed");
        for phase in [Phase::Downloading, Phase::Verifying, Phase::Failed] {
            install_jobs().lock().unwrap().insert(id.into(), JobState {
                phase, downloaded_bytes: 1, total_bytes: 3,
                error: Some("previous failure".into()), cancel: Arc::new(AtomicBool::new(false)),
            });
            let state = status_entry_state(id, Status::Checking, true);
            assert_eq!(state.0, match phase {
                Phase::Downloading => "downloading",
                Phase::Verifying => "verifying",
                _ => "checking",
            });
            assert!(state.3.is_none());
        }
        install_jobs().lock().unwrap().remove(id);
    }

    #[test]
    fn moonshine_targets_use_shipped_identities_and_recorded_weight_urls() {
        let root = isolated_root("moonshine-pins");
        for (id, size, bytes) in [
            ("moonshine-medium", "medium", 269_141_623),
            ("moonshine-small", "small", 142_300_974),
        ] {
            let files = moonshine_files(id).unwrap();
            assert_eq!(files.len(), 8);
            assert_eq!(files.iter().map(|file| file.bytes).sum::<u64>(), bytes);
            let base = format!(
                "https://download.moonshine.ai/model/{size}-streaming-en/{MOONSHINE_REVISION}"
            );
            assert_eq!(moonshine_cdn_base(id), Some(base.as_str()));
            if !MOONSHINE_AVAILABLE {
                assert!(describe_target(&root, id).err().unwrap().contains("Windows x64"));
                continue;
            }
            let target = describe_target(&root, id).unwrap();
            assert_eq!(target.dest_dir, root.join("optional").join(id));
            assert_eq!(target.optional_root, root.join("optional"));
            assert_eq!(target.total_bytes, bytes);
            assert!(!target.needs_runtime, "Moonshine never downloads a runtime");
            assert_eq!(target.files.len(), files.len());
            for (download, identity) in target.files.iter().zip(files) {
                assert_eq!(download.relative, identity.name);
                assert_eq!(download.bytes, identity.bytes);
                assert_eq!(download.sha256, identity.sha256);
                assert_eq!(download.sha256.len(), 64);
                assert_eq!(download.url, format!("{base}/{}", identity.name));
                assert!(!download.relative.ends_with(".dll"));
            }
        }
        for id in ["moonshine-runtime", "moonshine-tiny", "moonshine-small/../medium"] {
            assert!(moonshine_cdn_base(id).is_none());
            assert!(describe_target(&root, id).is_err());
        }
        assert!(!mac_supported("moonshine-medium"));
        assert!(!mac_supported("moonshine-small"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_moonshine_rows_offer_pinned_installs_only_on_windows_x64() {
        let _gate = test_gate();
        let root = isolated_root("moonshine-missing");
        let mut settings = Settings::default();
        settings.asr_model = "moonshine-small".into();
        let entries = moonshine_entries(&root, &settings);
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert_eq!(entry.installable, MOONSHINE_AVAILABLE);
            assert_eq!(entry.state, if MOONSHINE_AVAILABLE { "not-installed" } else { "unavailable" });
            assert!(!entry.removable);
            assert_eq!(entry.downloaded_bytes, 0);
            assert_eq!(entry.total_bytes, entry.disk_bytes);
            assert!(entry.disk_bytes > 0);
            assert_eq!(entry.revision, MOONSHINE_REVISION);
            assert_eq!(entry.used_by, if entry.id == settings.asr_model { vec!["asr"] } else { vec![] });
        }
        assert_eq!(settings.asr_model, "moonshine-small");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn bundled_moonshine_readiness_and_located_paths_keep_precedence() {
        let _gate = test_gate();
        let root = isolated_root("moonshine-bundled");
        let located = isolated_root("moonshine-located");
        // Tiny files exercise the existing bundled readiness contract only;
        // no recognizer is loaded and no fixture claims valid model weights.
        for id in ["moonshine-medium", "moonshine-small"] {
            let directory = located.join(id);
            fs::create_dir(&directory).unwrap();
            for file in moonshine_files(id).unwrap() {
                fs::write(directory.join(file.name), b"bundled fixture").unwrap();
            }
            let managed = root.join("optional").join(id);
            fs::create_dir(&managed).unwrap();
            fs::write(managed.join("encoder.ort"), b"partial download").unwrap();
        }
        fs::write(root.join("paths.json"), serde_json::to_vec(&serde_json::json!({
            "moonshine_medium_dir": located.join("moonshine-medium"),
            "moonshine_small_dir": located.join("moonshine-small"),
        })).unwrap()).unwrap();
        let runtime = crate::asr::runtime_path(&root);
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(&runtime, b"runtime presence fixture").unwrap();
        for entry in moonshine_entries(&root, &Settings::default()) {
            assert_eq!(entry.state, "installed");
            assert!(!entry.installable);
            assert!(!entry.removable);
            assert_eq!(entry.revision, "bundled");
            assert_eq!(crate::asr::model_directory(&root, &entry.id), located.join(&entry.id));
            assert!(crate::asr::model_ready(&root, &entry.id));
            assert!(start_install(root.clone(), entry.id.clone()).unwrap_err().contains("already installed"));
            let target = describe_target(&root, &entry.id).unwrap();
            assert_eq!(target.dest_dir, root.join("optional").join(&entry.id));
        }
        // A damaged located copy must not make the incomplete managed copy ready.
        fs::remove_file(located.join("moonshine-small/encoder.ort")).unwrap();
        assert!(moonshine_managed_directory(&root, "moonshine-small").is_none());
        assert!(!crate::asr::model_ready(&root, "moonshine-small"));
        assert!(!install_in_progress());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(located).unwrap();
    }

    #[test]
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn moonshine_missing_runtime_fails_before_staging_or_download() {
        let _gate = test_gate();
        let root = isolated_root("moonshine-runtime");
        let shared = crate::optional_asr::runtime_dir(&root);
        fs::create_dir(&shared).unwrap();
        fs::write(shared.join("onnxruntime.dll"), b"shared CPU VAD runtime").unwrap();
        let cancel = AtomicBool::new(false);
        for id in ["moonshine-medium", "moonshine-small"] {
            let error = start_install(root.clone(), id.into()).unwrap_err();
            assert!(error.contains("Repair SonoJot"), "{error}");
            let error = install_inner(&root, id, &cancel).unwrap_err();
            assert!(error.contains("Repair SonoJot"), "{error}");
            assert!(!root.join("optional").join(id).exists());
            assert!(job_snapshot(id).is_none());
        }
        let runtime = crate::asr::runtime_path(&root);
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(&runtime, b"damaged runtime").unwrap();
        assert!(require_moonshine_runtime(&root).unwrap_err().contains("Repair SonoJot"));
        assert_eq!(fs::read_dir(root.join("optional")).unwrap().count(), 1);
        assert_eq!(fs::read(shared.join("onnxruntime.dll")).unwrap(), b"shared CPU VAD runtime");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn moonshine_install_markers_keep_selection_and_cancel_guards() {
        let _gate = test_gate();
        let root = isolated_root("moonshine-selection");
        let id = "moonshine-small";
        let mut settings = Settings::default();
        settings.asr_model = "moonshine-medium".into();
        for phase in [Phase::Downloading, Phase::Verifying] {
            let cancel = Arc::new(AtomicBool::new(false));
            install_jobs().lock().unwrap().insert(id.into(), JobState {
                phase, downloaded_bytes: 10, total_bytes: 142_300_974,
                error: None, cancel: cancel.clone(),
            });
            let result = resource_manager::with_model_selection(true, || {
                settings.asr_model = id.into();
                Ok(())
            });
            assert!(result.unwrap_err().contains("being installed"));
            assert_eq!(settings.asr_model, "moonshine-medium");
            let entry = moonshine_entries(&root, &settings).into_iter().find(|entry| entry.id == id).unwrap();
            if MOONSHINE_AVAILABLE {
                assert_eq!(entry.state, if phase == Phase::Downloading { "downloading" } else { "verifying" });
                assert_eq!(entry.downloaded_bytes, 10);
                assert_eq!(entry.total_bytes, 142_300_974);
            }
            assert!(cancel_install(id));
            assert!(cancel.load(Ordering::Acquire));
            set_job_phase(id, Phase::Failed, Some("Install cancelled.".into()));
            assert!(!install_in_progress());
            assert_eq!(entry_state(id, false, MOONSHINE_AVAILABLE).0, "cancelled");
            install_jobs().lock().unwrap().remove(id);
        }
        assert!(remove_model_blocking(&root, &settings, id, true).is_err());
        assert_eq!(settings.asr_model, "moonshine-medium");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn granite_library_is_meeting_only_and_unavailable_cuda_is_explicit() {
        let _gate = test_gate();
        let root = isolated_root("granite-catalog");
        let settings = Settings::default();
        let [entry] = granite_entry_with_availability(&root, &settings, None);
        assert_eq!(entry.id, granite_model::GRANITE_ID);
        assert_eq!(entry.name, "Granite Speech 4.1 NAR");
        assert_eq!(entry.roles, ["meetingSpeech"]);
        assert!(!entry.roles.iter().any(|role| ["asr", "cleanup", "notes"].contains(&role.as_str())));
        assert_eq!(entry.guidance, "More accurate on room microphones and calls; meetings only.");
        assert_eq!(entry.backend, "transcribe.cpp");
        assert_eq!(entry.quantization, "Q8_0");
        assert_eq!(entry.total_bytes, granite_model::download_bytes());
        assert_eq!(entry.disk_bytes, granite_model::MODEL.bytes);
        assert!(entry.installable);
        assert_eq!(entry.state, "not-installed");
        let reason = "Granite meetings need an NVIDIA graphics card and its CUDA driver.";
        let [entry] = granite_entry_with_availability(&root, &settings, Some(reason));
        assert!(!entry.installable);
        assert_eq!(entry.state, "unavailable");
        assert_eq!(entry.compatibility, reason);
        assert!(mac_supported(granite_model::GRANITE_ID));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn granite_cancel_preserves_old_files_and_removal_includes_private_runtime() {
        let _gate = test_gate();
        let root = isolated_root("granite-cancel-remove");
        let id = granite_model::GRANITE_ID;
        let runtime = granite_model::runtime_dir(&root);
        fs::create_dir_all(&runtime).unwrap();
        fs::write(granite_model::model_path(&root), b"old model").unwrap();
        fs::write(runtime.join("transcribe.dll"), b"old runtime").unwrap();
        let cancel = Arc::new(AtomicBool::new(true));
        install_jobs().lock().unwrap().insert(id.into(), JobState {
            phase: Phase::Downloading, downloaded_bytes: 0,
            total_bytes: granite_model::download_bytes(), error: None, cancel: cancel.clone(),
        });
        install_blocking(root.clone(), id.into(), cancel);
        let [entry] = granite_entry_with_availability(&root, &Settings::default(), None);
        assert_eq!(entry.state, "cancelled");
        assert!(entry.removable);
        assert_eq!(fs::read(granite_model::model_path(&root)).unwrap(), b"old model");
        assert_eq!(fs::read(runtime.join("transcribe.dll")).unwrap(), b"old runtime");
        install_jobs().lock().unwrap().remove(id);
        let mut settings = Settings::default();
        settings.meeting_asr_model = id.into();
        assert_eq!(used_by(&settings, id), ["meetingSpeech"]);
        assert!(remove_model_blocking(&root, &settings, id, true).unwrap_err().contains("still selected"));
        settings.meeting_asr_model.clear();
        assert!(remove_model_blocking(&root, &settings, id, false).is_err());
        let shared = crate::optional_asr::runtime_dir(&root);
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("keep.dll"), b"shared").unwrap();
        remove_model_blocking(&root, &settings, id, true).unwrap();
        assert!(!granite_model::path(&root).exists());
        assert_eq!(fs::read(shared.join("keep.dll")).unwrap(), b"shared");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diarization_library_metadata_and_pins_are_separate_from_speech() {
        let _gate = test_gate();
        let root = isolated_root("diarization-catalog");
        let entries = diarization_entries(&root);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.id, diarization_model::ID);
        assert_eq!(entry.name, "Nemotron 3 Diarization");
        assert_eq!(entry.kind, "diarization");
        assert_eq!(entry.roles, ["speakers"]);
        assert!(entry.used_by.is_empty());
        assert!(entry.family.contains("CPU"));
        assert!(entry.family.contains("ONNX Runtime"));
        assert_eq!(entry.backend, "onnxruntime-cpu");
        assert_eq!(entry.state, "not-installed");
        assert_eq!(entry.quantization, "int8");
        assert_eq!(entry.revision, diarization_model::REVISION);
        assert_eq!(entry.disk_bytes, 103_871_648);
        let runtime = if diarization_model::MAC {
            diarization_model::MAC_RUNTIME_ARCHIVE_BYTES
        } else {
            8_179_934
        };
        assert_eq!(entry.total_bytes, 103_871_648 + runtime);
        assert_eq!((entry.ram_min_bytes, entry.ram_max_bytes), (None, None));
        assert_eq!(
            (entry.vram_min_bytes, entry.vram_max_bytes),
            (Some(0), Some(0))
        );
        assert_eq!(entry.installable, diarization_model::AVAILABLE);
        assert!(!entry.removable);

        if diarization_model::AVAILABLE {
            let target = describe_target(&root, diarization_model::ID).unwrap();
            assert!(target.needs_runtime);
            assert_eq!(
                target.dest_dir,
                root.join("optional").join(diarization_model::ID)
            );
            assert_eq!(target.total_bytes, entry.total_bytes);
            assert_eq!(target.files.len(), 4);
            for (download, pin) in target.files.iter().zip(diarization_model::FILES) {
                assert_eq!(download.relative, pin.relative);
                assert_eq!(download.bytes, pin.bytes);
                assert_eq!(download.sha256, pin.sha256);
                assert_eq!(download.url, format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    diarization_model::REPO, diarization_model::REVISION, pin.relative
                ));
            }
        } else {
            let error = describe_target(&root, diarization_model::ID).err().unwrap();
            assert!(error.contains("not available"), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diarization_cancelled_live_install_preserves_files_and_reports_failure() {
        let _gate = test_gate();
        let root = isolated_root("diarization-cancel");
        let id = diarization_model::ID;
        let dest = diarization_model::path(&root);
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("model.int8.onnx"), b"old install").unwrap();
        let total = diarization_model::download_bytes(&root);
        let cancel = Arc::new(AtomicBool::new(false));
        install_jobs().lock().unwrap().insert(
            id.into(),
            JobState {
                phase: Phase::Downloading,
                downloaded_bytes: 0,
                total_bytes: total,
                error: None,
                cancel: cancel.clone(),
            },
        );
        set_download_progress(id, u64::MAX, total);
        let entry = diarization_entries(&root).remove(0);
        assert_eq!(entry.state, "downloading");
        assert_eq!(entry.downloaded_bytes, total - 1);
        assert_eq!(entry.total_bytes, total);
        assert!(cancel_install(id));
        // Calls the actual worker entry, but its cancellation gate prevents
        // both runtime and model downloads before any staging is created.
        install_blocking(root.clone(), id.into(), cancel);
        let entry = diarization_entries(&root).remove(0);
        assert_eq!(entry.state, "cancelled");
        assert!(entry.downloaded_bytes < entry.total_bytes);
        assert_eq!(
            fs::read(dest.join("model.int8.onnx")).unwrap(),
            b"old install"
        );
        assert_eq!(fs::read_dir(root.join("optional")).unwrap().count(), 1);
        assert!(!crate::optional_asr::runtime_dir(&root).exists());
        install_jobs().lock().unwrap().remove(id);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diarization_removal_keeps_shared_runtime_even_for_partial_install() {
        let _gate = test_gate();
        let root = isolated_root("diarization-remove");
        let dest = diarization_model::path(&root);
        let runtime = crate::optional_asr::runtime_dir(&root);
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("model.int8.onnx"), b"partial").unwrap();
        fs::create_dir(&runtime).unwrap();
        fs::write(runtime.join("onnxruntime.dll"), b"shared runtime").unwrap();
        assert!(!diarization_model::installed(&root));
        assert!(diarization_entries(&root)[0].removable);
        let settings = Settings::default();
        assert!(remove_model_blocking(&root, &settings, diarization_model::ID, false).is_err());
        assert!(dest.exists());
        remove_model_blocking(&root, &settings, diarization_model::ID, true).unwrap();
        assert!(!dest.exists());
        assert_eq!(
            fs::read(runtime.join("onnxruntime.dll")).unwrap(),
            b"shared runtime"
        );
        assert!(!diarization_entries(&root)[0].removable);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn smart_cleanup_catalog_is_pinned_cleanup_only_and_mac_supported() {
        let _gate = test_gate();
        let root = isolated_root("smart-catalog");
        let id = crate::optional_llm::SMART_CLEANUP_ID;
        let entries = optional_llm_entries(&root, &Settings::default());
        let entry = entries.iter().find(|entry| entry.id == id).unwrap();
        assert_eq!(entry.name, "Smart cleanup (experimental)");
        assert_eq!(entry.roles, ["cleanup"]);
        assert_eq!(entry.disk_bytes, 541_903_616);
        assert!(entry.used_by.is_empty());
        assert!(entry.memory_basis.contains("Apache-2.0"));
        assert!(entry.memory_basis.contains("Abhishek Barali"));
        assert!(entry.guidance.contains("Mac is unmeasured"));
        assert!(mac_supported(id));
        let target = describe_target(&root, id).unwrap();
        assert_eq!(target.files.len(), 1);
        assert_eq!(target.files[0].bytes, 541_903_616);
        assert_eq!(target.files[0].sha256, "530e0a5d4e9314e3bae2c92aecf3ab39ca48e8cfecac41372058f9fcb742329d");
        assert!(target.files[0].url.contains("835431771f72820251fe6c6b4b07f12b000e2647"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_lists_existing_catalog_honestly() {
        let _gate = test_gate();
        let root = isolated_root("catalog");
        let settings = Settings::default();
        let library = build_library(&root, &settings);
        let mac = cfg!(all(target_os = "macos", target_arch = "aarch64"));
        // Existing CPU/text packages plus the sherpa and llama GPU catalogs.
        assert_eq!(
            library.models.len(),
            2 + 2 + 2 + 1 + crate::optional_llm::catalog().len() + 2 + 12 + crate::gpu_asr::GPU_MODELS.len()
                + crate::llama_asr::catalog().len()
                + if mac { crate::mac_speech::catalog().len() } else { 0 }
        );
        assert!(
            library.hardware.total_ram_bytes.is_none()
                || library.hardware.total_ram_bytes.unwrap() > 0
        );
        // Dedicated VRAM is Some only when bounded nvidia-smi succeeded.
        assert!(
            library.hardware.dedicated_vram_bytes.is_none()
                || library.hardware.dedicated_vram_bytes.unwrap() > 0
        );
        // Guidance carries a real plan range for the default setup when known.
        assert!(
            library.guidance.contains("estimated to use")
                || library.guidance.contains("unmeasured amount")
                || (mac && library.guidance.contains("Apple Silicon shares memory"))
        );
        for entry in &library.models {
            if mac && !mac_supported(&entry.id) {
                assert!(!entry.installable, "{}", entry.id);
                assert_eq!(entry.state, "unavailable", "{}", entry.id);
                assert!(entry.compatibility.contains("requires Windows"), "{}", entry.id);
            }
            if crate::mac_speech::find(&entry.id).is_some() {
                assert!(mac && entry.installable, "{}", entry.id);
                assert_eq!(entry.vram_min_bytes, Some(0));
                assert!(entry.compatibility.contains("Apple Silicon"));
            }
            if !mac && entry.family == "Whisper" && !crate::gpu_asr::is_gpu_model(&entry.id) {
                // Real pinned entries: installable, real disk bytes, provisional estimates.
                assert!(entry.installable, "{}", entry.id);
                assert!(!entry.removable, "{}", entry.id);
                assert!(entry.disk_bytes > 0, "{}", entry.id);
                assert!(entry.ram_min_bytes.is_some(), "{}", entry.id);
                assert!(entry.compatibility.contains("AMD"), "{}", entry.id);
                assert!(entry.evidence.contains("2026-09-16"), "{}", entry.id);
            }
            if !mac && crate::gpu_asr::is_gpu_model(&entry.id) {
                // New sherpa CUDA downloads wait for an ASR-only runtime.
                // CUDA-13-only compatibility, provisional host+VRAM estimates.
                assert!(!entry.installable, "{}", entry.id);
                assert!(entry.guidance.contains("ASR-only"));
                assert!(!entry.removable, "{}", entry.id);
                assert!(entry.disk_bytes > 0, "{}", entry.id);
                assert!(entry.ram_min_bytes.is_some(), "{}", entry.id);
                assert!(entry.vram_min_bytes.is_some(), "{}", entry.id);
                assert!(entry.compatibility.contains("AMD"), "{}", entry.id);
                assert!(entry.compatibility.contains("NVIDIA"), "{}", entry.id);
                assert!(entry.compatibility.contains("CUDA 13"), "{}", entry.id);
                assert!(entry.backend.contains("cuda"), "{}", entry.id);
                assert!(entry.evidence.contains("2026-09-"), "{}", entry.id);
                // First-install discloses the shared runtime cost.
                assert!(
                    entry.guidance.contains("shared") || entry.guidance.contains("runtime"),
                    "{}",
                    entry.id
                );
            }
            // Language scope stays visible.
            assert!(!entry.languages.is_empty());
            // Backend never relabels CPU as GPU.
            assert!(!entry.backend.contains("cuda") || entry.compatibility.contains("NVIDIA")
                || (mac && !entry.installable && entry.compatibility.contains("requires Windows")));
            // usedBy reflects default settings.
            if entry.id == "moonshine-medium" {
                assert!(entry.used_by.contains(&"asr".to_owned()));
            }
            // Default Cleanup is rules plus punctuation; no generative model.
            if entry.id == "local-rules" {
                assert!(entry.used_by.contains(&"cleanup".to_owned()));
            }
            if entry.id == "lfm2.5-1.2b" {
                assert!(!entry.used_by.contains(&"cleanup".to_owned()));
            }
        }
        // Contract field presence (serde camelCase).
        let value = serde_json::to_value(&library).unwrap();
        assert!(value
            .get("hardware")
            .unwrap()
            .get("totalRamBytes")
            .is_some());
        assert!(value.get("guidance").is_some());
        let first = value
            .get("models")
            .unwrap()
            .as_array()
            .unwrap()
            .first()
            .unwrap();
        for key in [
            "id",
            "name",
            "kind",
            "roles",
            "family",
            "languages",
            "older",
            "diskBytes",
            "ramMinBytes",
            "ramMaxBytes",
            "vramMinBytes",
            "vramMaxBytes",
            "memoryBasis",
            "guidance",
            "evidence",
            "backend",
            "compatibility",
            "state",
            "downloadedBytes",
            "totalBytes",
            "error",
            "installable",
            "removable",
            "usedBy",
            "revision",
            "quantization",
            "contextTokens",
        ] {
            assert!(first.get(key).is_some(), "missing {key}");
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn llama_library_reuses_catalog_metadata_and_preserves_selection() {
        let _gate = test_gate();
        let root = isolated_root("llama-catalog");
        let mut settings = Settings::default();
        let saved_asr = settings.asr_model.clone();
        let model = crate::llama_asr::find(crate::llama_asr::MODEL_ID).unwrap();
        let entries = llama_asr_entries(&root, &settings);
        assert_eq!(entries.len(), crate::llama_asr::catalog().len());
        let cpu = entries.iter().find(|e| e.id == crate::llama_asr::CPU_MODEL_ID).unwrap();
        assert_eq!(cpu.name, "Qwen3-ASR 1.7B (Q4 CPU)");
        assert_eq!(cpu.vram_max_bytes, Some(0));
        let entry = &entries[0];
        assert_eq!(entry.id, model.id);
        assert_eq!(entry.name, "Qwen3-ASR 1.7B (Q8 CUDA)");
        assert_eq!(entry.kind, "asr");
        assert_eq!(entry.roles, ["asr"]);
        assert_eq!(entry.languages, ["multilingual"]);
        assert_eq!(entry.disk_bytes, crate::llama_asr::package_bytes(model));
        assert_eq!(entry.quantization, model.quantization);
        assert_eq!(entry.total_bytes, crate::llama_asr::download_total_bytes(&root, model.id).unwrap());
        assert_eq!(entry.total_bytes, entry.disk_bytes + crate::optional_asr::runtime_download_bytes(&root));
        assert!(entry.total_bytes > entry.disk_bytes, "first install includes CPU VAD runtime");
        assert_eq!(entry.downloaded_bytes, 0);
        assert_eq!(entry.state, if model.runtime.supported_platform() { "not-installed" } else { "unavailable" });
        assert_eq!(entry.installable, model.runtime.supported_platform());
        assert!(!entry.removable);
        assert!(entry.used_by.is_empty());
        assert_eq!(entry.guidance, model.guidance);
        assert_eq!(entry.evidence, model.evidence);
        assert_eq!(entry.backend, model.backend);
        assert_eq!(entry.compatibility, model.compatibility);
        assert_eq!(entry.revision, model.revision);
        assert!(entry.memory_basis.contains(model.license));
        assert!(entry.memory_basis.contains("Observed 2026-09-28"));
        assert_eq!(entry.ram_max_bytes, Some(1600 << 20));
        assert_eq!(entry.vram_max_bytes, Some(4200 << 20));
        assert_eq!(settings.asr_model, saved_asr);
        settings.asr_model = model.id.into();
        assert_eq!(llama_asr_entries(&root, &settings)[0].used_by, ["asr"]);
        assert_eq!(settings.asr_model, model.id);
        assert!(!mac_supported(model.id));
        if cfg!(all(windows, target_arch = "x86_64")) {
            let target = describe_target(&root, model.id).unwrap();
            assert_eq!(target.dest_dir, crate::llama_asr::model_dir(&root, model.id));
            assert_eq!(target.total_bytes, entry.total_bytes);
            assert!(target.files.is_empty());
            assert!(!target.needs_runtime, "llama installer owns CPU VAD installation");
        } else {
            assert!(describe_target(&root, model.id).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metal_llama_library_gates_install_and_preserves_catalog_metadata() {
        let _gate = test_gate();
        let root = isolated_root("metal-llama-catalog");
        let mut settings = Settings::default();
        let entries = llama_asr_entries(&root, &settings);
        for id in [crate::llama_asr::METAL_Q8_MODEL_ID, crate::llama_asr::METAL_Q4_MODEL_ID] {
            let model = crate::llama_asr::find(id).unwrap();
            let entry = entries.iter().find(|entry| entry.id == id).unwrap();
            let supported = cfg!(all(target_os = "macos", target_arch = "aarch64"));
            assert!(mac_supported(id));
            assert_eq!(entry.installable, supported);
            assert_eq!(entry.state, if supported { "not-installed" } else { "unavailable" });
            assert_eq!(entry.name, model.name);
            assert_eq!(entry.backend, model.backend);
            assert_eq!(entry.compatibility, model.compatibility);
            assert_eq!(entry.guidance, model.guidance);
            assert_eq!(entry.evidence, model.evidence);
            assert_eq!(entry.quantization, model.quantization);
            assert_eq!(entry.disk_bytes, crate::llama_asr::package_bytes(model));
            assert_eq!(entry.total_bytes, entry.disk_bytes + mac_runtime_download_bytes(&root)
                + crate::optional_asr::mac_speech_runtime_download_bytes(&root),
                "Metal includes its speech gate and never the Windows sherpa runtime");
            assert_eq!(entry.roles, ["asr"]);
            assert!(!entry.removable);
            assert!(entry.used_by.is_empty());
            settings.asr_model = id.into();
            assert_eq!(llama_asr_entries(&root, &settings).iter().find(|entry| entry.id == id).unwrap().used_by, ["asr"]);
            assert_eq!(settings.asr_model, id);
            if supported {
                let target = describe_target(&root, id).unwrap();
                assert!(!target.needs_runtime, "Metal installs its own speech detector, not sherpa");
                assert!(target.files.is_empty());
                assert_eq!(target.total_bytes, entry.total_bytes);
                assert_eq!(target.dest_dir, crate::llama_asr::model_dir(&root, id));
            } else {
                assert_eq!(describe_target(&root, id).err().unwrap(), model.runtime.platform_requirement());
                assert!(install_inner(&root, id, &AtomicBool::new(false)).is_err());
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    fn metal_speech_prepares_shared_runtime_before_weights_without_sherpa() {
        let _gate = test_gate();
        let root = isolated_root("metal-first-speech");
        let shared = root.join("llama.cpp");
        // A filesystem obstacle stops the runtime installer before networking.
        // This catches the former early "missing runtime" rejection in ASR.
        fs::write(&shared, b"blocked runtime directory").unwrap();
        for id in [crate::llama_asr::METAL_Q8_MODEL_ID, crate::llama_asr::METAL_Q4_MODEL_ID] {
            let model = crate::llama_asr::find(id).unwrap();
            assert_eq!(describe_target(&root, id).unwrap().total_bytes,
                crate::llama_asr::package_bytes(model) + 11_149_739
                    + crate::optional_asr::mac_speech_runtime_download_bytes(&root));
            assert_eq!(install_inner(&root, id, &AtomicBool::new(true)).unwrap_err(), "Install cancelled.");
            let error = install_inner(&root, id, &AtomicBool::new(false)).unwrap_err();
            assert!(error.contains("Cannot create Metal runtime folder"), "{error}");
            assert!(!crate::llama_asr::model_dir(&root, id).exists());
            assert!(!crate::optional_asr::runtime_dir(&root).exists());
        }
        fs::remove_file(shared).unwrap();
        let runtime = root.join("llama.cpp/runtime/mac-arm64");
        fs::create_dir_all(&runtime).unwrap();
        // Presence fixtures only; no native code is loaded or model claimed ready.
        for file in ["llama-server", "libggml-metal.dylib", "libggml-base.dylib",
            "libggml-cpu.dylib", "libllama.dylib", "libllama-server-impl.dylib"]
        {
            fs::write(runtime.join(file), b"runtime presence fixture").unwrap();
        }
        fs::write(runtime.join("sonojot-runtime-release"), b"b10964\n").unwrap();
        assert_eq!(mac_runtime_download_bytes(&root), 0);
        for id in [crate::llama_asr::METAL_Q8_MODEL_ID, crate::llama_asr::METAL_Q4_MODEL_ID] {
            let bytes = crate::llama_asr::package_bytes(crate::llama_asr::find(id).unwrap());
            assert_eq!(describe_target(&root, id).unwrap().total_bytes,
                bytes + crate::optional_asr::mac_speech_runtime_download_bytes(&root));
            install_mac_runtime(&root, id, bytes, &AtomicBool::new(false)).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn cancelled_llama_install_does_not_create_files_or_progress() {
        let _gate = test_gate();
        let root = std::env::temp_dir().join(format!("sonojot-llama-cancel-{}", crate::store::new_id()));
        let id = crate::llama_asr::MODEL_ID;
        assert_eq!(
            install_inner(&root, id, &AtomicBool::new(true)).unwrap_err(),
            "Install cancelled."
        );
        assert!(!root.exists());
        assert!(job_snapshot(id).is_none());
    }

    #[test]
    fn llama_library_tracks_combined_install_progress_and_cancellation() {
        let _gate = test_gate();
        let root = isolated_root("llama-progress");
        let id = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            crate::llama_asr::METAL_Q8_MODEL_ID
        } else {
            crate::llama_asr::MODEL_ID
        };
        if !crate::llama_asr::find(id).unwrap().runtime.supported_platform() { return; }
        let total = crate::llama_asr::download_total_bytes(&root, id).unwrap();
        install_jobs().lock().unwrap().insert(id.into(), JobState {
            phase: Phase::Downloading,
            downloaded_bytes: 0,
            total_bytes: total,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let done = total / 2;
        let (done, total) = clamp_gpu_progress(done, total);
        set_job_progress(id, done, total);
        let entries = llama_asr_entries(&root, &Settings::default());
        let entry = entries.iter().find(|entry| entry.id == id).unwrap();
        assert_eq!(entry.state, "downloading");
        assert_eq!(entry.downloaded_bytes, done);
        assert_eq!(entry.total_bytes, total);
        assert_eq!(clamp_gpu_progress(u64::MAX, total), (total, total));
        assert!(cancel_install(id));
        assert!(install_jobs().lock().unwrap().get(id).unwrap().cancel.load(Ordering::Acquire));
        set_job_phase(id, Phase::Failed, Some("Install cancelled.".into()));
        assert_eq!(llama_asr_entries(&root, &Settings::default()).iter().find(|entry| entry.id == id).unwrap().state, "cancelled");
        install_jobs().lock().unwrap().remove(id);
        assert_eq!(entry_state(id, true, true).0, "installed");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn llama_removal_requires_confirmation_selection_and_lease_release() {
        let _gate = test_gate();
        let root = isolated_root("llama-remove");
        let id = crate::llama_asr::MODEL_ID;
        let dest = crate::llama_asr::model_dir(&root, id);
        let runtime = crate::optional_asr::runtime_dir(&root);
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("decoder.gguf"), b"partial model").unwrap();
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("onnxruntime.dll"), b"shared CPU VAD runtime").unwrap();
        let mut settings = Settings::default();
        assert!(remove_model_blocking(&root, &settings, id, false).unwrap_err().contains("Confirm removal"));
        settings.asr_model = id.into();
        assert!(remove_model_blocking(&root, &settings, id, true).unwrap_err().contains("still selected"));
        assert_eq!(settings.asr_model, id);
        settings.asr_model = Settings::default().asr_model;
        resource_manager::configure(&settings);
        resource_manager::acquire_asr(id).unwrap();
        assert_eq!(entry_state(id, true, true).0, "busy");
        assert!(remove_model_blocking(&root, &settings, id, true).unwrap_err().contains("running now"));
        resource_manager::note_asr_loaded(id);
        resource_manager::notify_asr_finished();
        assert_eq!(entry_state(id, true, true).0, "ready");
        assert!(remove_model_blocking(&root, &settings, id, true).unwrap_err().contains("still loaded"));
        assert!(dest.join("decoder.gguf").exists());
        resource_manager::confirm_asr_released();
        remove_model_blocking(&root, &settings, id, true).unwrap();
        assert!(!dest.exists());
        assert_eq!(fs::read(runtime.join("onnxruntime.dll")).unwrap(), b"shared CPU VAD runtime");
        assert!(!install_in_progress());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removal_reads_current_selections_inside_the_shared_guard() {
        // A queued removal must use CURRENT selections, not a snapshot taken
        // before the guard: a selection commit racing the delete (new model
        // just selected) must block the delete instead of deleting a live
        // selection on stale data.
        let _gate = test_gate();
        let root = isolated_root("fresh-select");
        let store_dir =
            std::env::temp_dir().join(format!("sonojot-modelmgr-store-{}", crate::store::new_id()));
        let store = crate::store::Store::open(Some(store_dir.clone())).unwrap();
        // Stage dummy files for a removable model.
        let dest = crate::optional_asr::model_dir(&root, "parakeet-tdt-0.6b-v2");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("encoder.int8.onnx"), b"dummy").unwrap();
        // Case 1: store still selects the model in the ASR role -> the
        // guard-first route blocks even though the caller never passed the
        // stale snapshot.
        let mut settings = store.load_settings().unwrap();
        settings.asr_model = "parakeet-tdt-0.6b-v2".into();
        store.save_settings(&settings).unwrap();
        let err = remove_model_blocking_with_store(&root, &store, "parakeet-tdt-0.6b-v2", true)
            .unwrap_err();
        assert!(err.contains("still selected"), "{err}");
        assert!(dest.join("encoder.int8.onnx").exists());
        // Case 2: the selection commit lands (replacement picked), then the
        // same queued removal re-reads and proceeds to delete.
        settings.asr_model = "moonshine-medium".into();
        store.save_settings(&settings).unwrap();
        assert!(
            remove_model_blocking_with_store(&root, &store, "parakeet-tdt-0.6b-v2", true).is_ok()
        );
        assert!(!dest.exists());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&store_dir).unwrap();
    }

    #[test]
    fn selected_shared_removal_is_blocked_without_silent_fallback() {
        let _gate = test_gate();
        let root = isolated_root("remove");
        let mut settings = Settings::default();
        settings.cleanup_model = "qwen3-4b-instruct-2507".into();
        settings.notes_model = "qwen3-4b-instruct-2507".into();
        // Shared text artifact: both roles block, even when confirmed.
        let err =
            remove_model_blocking(&root, &settings, "qwen3-4b-instruct-2507", false).unwrap_err();
        assert!(err.contains("Confirm removal"), "{err}");
        let err =
            remove_model_blocking(&root, &settings, "qwen3-4b-instruct-2507", true).unwrap_err();
        assert!(err.contains("still selected"), "{err}");
        // Settings untouched: no silent fallback happened.
        assert_eq!(settings.cleanup_model, "qwen3-4b-instruct-2507");
        // Unselected removable model with no files still succeeds (nothing to delete).
        settings.cleanup_model = "lfm2.5-1.2b".into();
        settings.notes_model = "lfm2.5-1.2b".into();
        assert!(remove_model_blocking(&root, &settings, "qwen3-4b-instruct-2507", true).is_ok());
        // Bundled/rules models are never removable.
        assert!(remove_model_blocking(&root, &settings, "moonshine-medium", true).is_err());
        assert!(remove_model_blocking(&root, &settings, "local-rules", true).is_err());
        assert!(remove_model_blocking(&root, &settings, "unknown-id", true).is_err());
        // Real Whisper checkpoints are removable when unselected (no-op when
        // nothing is installed).
        assert!(remove_model_blocking(&root, &settings, "whisper-tiny", true).is_ok());
        // Real GPU models are removable when unselected (no-op when nothing
        // installed; the shared runtime is never deleted here).
        assert!(remove_model_blocking(&root, &settings, "qwen3-asr-1.7b-fp16-cuda", true).is_ok());
        assert!(remove_model_blocking(&root, &settings, "whisper-tiny-fp32-cuda", true).is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unconfirmed_removal_never_deletes_unselected_files() {
        let _gate = test_gate();
        let root = isolated_root("unconfirmed");
        let settings = Settings::default();
        // Stage dummy files for an unselected model.
        let dest = crate::optional_asr::model_dir(&root, "parakeet-tdt-0.6b-v2");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("encoder.int8.onnx"), b"dummy").unwrap();
        assert!(dest.join("encoder.int8.onnx").exists());
        // confirmed=false must reject first and leave files untouched.
        let err =
            remove_model_blocking(&root, &settings, "parakeet-tdt-0.6b-v2", false).unwrap_err();
        assert!(err.contains("Confirm removal"), "{err}");
        assert!(dest.join("encoder.int8.onnx").exists());
        // Whisper unselected files also survive an unconfirmed request.
        let wdest = crate::whisper_catalog::model_dir(&root, "whisper-tiny");
        fs::create_dir_all(&wdest).unwrap();
        fs::write(wdest.join("encoder.int8.onnx"), b"dummy").unwrap();
        let err = remove_model_blocking(&root, &settings, "whisper-tiny", false).unwrap_err();
        assert!(err.contains("Confirm removal"), "{err}");
        assert!(wdest.join("encoder.int8.onnx").exists());
        // Confirmed removal deletes the staged files.
        assert!(remove_model_blocking(&root, &settings, "parakeet-tdt-0.6b-v2", true).is_ok());
        assert!(!dest.exists());
        assert!(remove_model_blocking(&root, &settings, "whisper-tiny", true).is_ok());
        assert!(!wdest.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn gpu_progress_total_is_combined_download_and_disk_is_install_bytes() {
        // Progress denominator must be download_total_bytes (exactly what
        // gpu_asr reports against); disk cost is install_bytes (unpacked).
        // The two differ and must never be mixed: the mapping adds no runtime
        // on top of the combined reports.
        let qwen_dl = crate::gpu_asr::download_total_bytes("qwen3-asr-1.7b-fp16-cuda").unwrap();
        assert_eq!(qwen_dl, 6_054_000_868);
        let qwen_disk = crate::gpu_asr::install_bytes("qwen3-asr-1.7b-fp16-cuda").unwrap();
        assert!(
            qwen_disk > qwen_dl,
            "unpacked disk exceeds download archives"
        );
        let root = isolated_root("gpu-totals");
        let target = describe_target(&root, "qwen3-asr-1.7b-fp16-cuda").expect("gpu target");
        assert_eq!(target.total_bytes, qwen_dl);
        let tiny_dl = crate::gpu_asr::download_total_bytes("whisper-tiny-fp32-cuda").unwrap();
        assert_eq!(tiny_dl, 1_475_811_533);
        assert_eq!(
            describe_target(&root, "whisper-tiny-fp32-cuda")
                .expect("tiny target")
                .total_bytes,
            tiny_dl
        );
        // Clamp: reports can never exceed the total, and partial reports never
        // read as 100 (final 100 is set on verified activation only).
        assert_eq!(clamp_gpu_progress(0, qwen_dl), (0, qwen_dl));
        assert_eq!(
            clamp_gpu_progress(qwen_dl - 1, qwen_dl),
            (qwen_dl - 1, qwen_dl)
        );
        assert_eq!(clamp_gpu_progress(qwen_dl, qwen_dl), (qwen_dl, qwen_dl));
        assert_eq!(clamp_gpu_progress(u64::MAX, qwen_dl), (qwen_dl, qwen_dl));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_install_targets_are_rejected_before_disk_work() {
        let root = isolated_root("unknown");
        assert_eq!(describe_target(&root, "moonshine-medium").is_ok(), MOONSHINE_AVAILABLE);
        assert!(describe_target(&root, "local-rules").is_err());
        assert!(describe_target(&root, "nope").is_err());
        assert!(describe_target(&root, "lfm2.5-1.2b").is_ok());
        assert!(describe_target(&root, "parakeet-tdt-0.6b-v2").is_ok());
        // Real Whisper checkpoints resolve to the ASR worker installer.
        assert!(describe_target(&root, "whisper-tiny").is_ok());
        assert!(describe_target(&root, "whisper-large-v3-turbo").is_ok());
        // Real GPU targets resolve with shared-runtime-inclusive totals.
        let gpu = describe_target(&root, "qwen3-asr-1.7b-fp16-cuda").expect("gpu target");
        assert!(gpu.total_bytes > 4_000_000_000);
        assert!(describe_target(&root, "whisper-tiny-fp32-cuda").is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn second_install_is_serialized_and_cancel_is_safe() {
        let _gate = test_gate();
        let root = isolated_root("serial");
        // Pretend an install is in flight.
        if let Ok(mut jobs) = install_jobs().lock() {
            jobs.insert(
                "lfm2.5-1.2b".into(),
                JobState {
                    phase: Phase::Downloading,
                    downloaded_bytes: 10,
                    total_bytes: 100,
                    error: None,
                    cancel: Arc::new(AtomicBool::new(false)),
                },
            );
        }
        assert!(start_install(root.clone(), "qwen3-4b-instruct-2507".into()).is_err());
        assert!(cancel_install("lfm2.5-1.2b"));
        // Cancel a missing job is safe and returns false.
        assert!(!cancel_install("missing-id"));
        if let Ok(mut jobs) = install_jobs().lock() {
            jobs.remove("lfm2.5-1.2b");
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn retention_modes_parse_from_wire() {
        // Wire contract: camelCase MemoryMode/ModelRetention survive round-trip.
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "memoryMode": "sequential", "modelRetention": "exit"
        }))
        .unwrap();
        assert_eq!(settings.memory_mode, MemoryMode::Sequential);
        assert_eq!(settings.model_retention, ModelRetention::OnExit);
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["memoryMode"], "sequential");
        assert_eq!(value["modelRetention"], "exit");
    }

    #[test]
    fn transactional_activation_preserves_old_on_failure() {
        let _gate = test_gate();
        let root = isolated_root("txn");
        // Old live install with real content.
        let dest = root.join("optional").join("whisper-tiny");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("tokens.txt"), b"old").unwrap();
        // Verified staging with new content activates and drops the backup.
        let staging = root.join("optional").join(".install-whisper-tiny-new");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("tokens.txt"), b"new").unwrap();
        crate::optional_asr::transactional_activate(&root, &staging, &dest, "speech model")
            .unwrap();
        assert_eq!(fs::read(dest.join("tokens.txt")).unwrap(), b"new");
        assert!(!staging.exists());
        // Failed activation (missing staging) restores the old install.
        let bad_staging = root.join("optional").join(".install-missing");
        // Recreate old content for the failure case.
        fs::write(dest.join("tokens.txt"), b"old2").unwrap();
        let err =
            crate::optional_asr::transactional_activate(&root, &bad_staging, &dest, "speech model")
                .unwrap_err();
        assert!(err.contains("Cannot activate"), "{err}");
        assert_eq!(
            fs::read(dest.join("tokens.txt")).unwrap(),
            b"old2",
            "old install must survive a failed rename"
        );
        // Cancellation before activation (no transactional call) leaves old.
        // Simulate by creating a staging dir then dropping it without calling
        // activate — dest untouched.
        let cancel_staging = root.join("optional").join(".install-cancel");
        fs::create_dir_all(&cancel_staging).unwrap();
        fs::write(cancel_staging.join("tokens.txt"), b"partial").unwrap();
        let _ = fs::remove_dir_all(&cancel_staging);
        assert_eq!(fs::read(dest.join("tokens.txt")).unwrap(), b"old2");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selection_commit_serializes_with_remove_behind_barrier() {
        use std::sync::{Arc, Barrier};
        let _gate = test_gate();
        let root = isolated_root("sel-del");
        let settings = Settings::default();
        // Fake installed files for an unselected model.
        let dest = crate::optional_asr::model_dir(&root, "parakeet-tdt-0.6b-v2");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("tokens.txt"), b"dummy").unwrap();
        assert!(dest.join("tokens.txt").exists());
        // Barrier: selection commit holds the shared guard across a 150 ms
        // file-commit simulation while a concurrent remove tries the same
        // guard. Exactly one holds it; the other fails fast (no interleaved
        // delete + commit).
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let root1 = root.clone();
        let settings1 = settings.clone();
        let selection = std::thread::spawn(move || {
            b1.wait();
            crate::resource_manager::with_model_selection(true, || {
                // Simulate a settings file commit holding both guards.
                std::thread::sleep(Duration::from_millis(150));
                // Inside the critical section no remove marker may appear.
                assert!(!install_in_progress());
                Ok::<(), String>(())
            })
        });
        let b2 = barrier.clone();
        let remover = std::thread::spawn(move || {
            b2.wait();
            // Small delay so selection wins the guard first deterministically.
            std::thread::sleep(Duration::from_millis(20));
            remove_model_blocking(&root1, &settings1, "parakeet-tdt-0.6b-v2", true)
        });
        let sel_res = selection.join().unwrap();
        let rem_res = remover.join().unwrap();
        // Selection held the guard; remove either waited (then deleted after
        // commit) or failed fast with install-busy. Both are safe; what must
        // never happen is a partial commit + partial delete interleaving.
        // Here selection wins, so remove must fail fast while selection holds
        // the shared guard (legacy remove holds the same guard).
        assert!(sel_res.is_ok(), "{sel_res:?}");
        // Remove tried while selection held the guard → shared-guard contention.
        // Our remove holds the shared guard for its duration; with selection
        // holding it, try_lock fails with the install-busy message.
        match rem_res {
            Ok(()) => {
                // If it waited past the commit (blocking impls would), files
                // are gone cleanly and no marker leaks.
                assert!(!dest.exists() || dest.join("tokens.txt").exists());
                assert!(!install_in_progress());
            }
            Err(e) => {
                assert!(
                    e.contains("being installed") || e.contains("busy") || e.contains("lock"),
                    "{e}"
                );
                // Failed fast: files untouched.
                assert!(dest.join("tokens.txt").exists());
                // Retry after commit succeeds and deletes cleanly.
                assert!(
                    remove_model_blocking(&root, &settings, "parakeet-tdt-0.6b-v2", true).is_ok()
                );
                assert!(!dest.exists());
            }
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
