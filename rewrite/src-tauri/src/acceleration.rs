//! Hardware capability and observed model offload, kept separate from inference
//! locks so the UI remains readable during long generation requests.
use crate::domain::TextAcceleration;
use serde::Serialize;
use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{ChildStderr, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

static CPU_ONLY: AtomicBool = AtomicBool::new(false);
pub fn set_preference(preference: TextAcceleration) {
    CPU_ONLY.store(preference == TextAcceleration::Cpu, Ordering::Release);
}
pub fn cpu_only() -> bool {
    CPU_ONLY.load(Ordering::Acquire)
        || crate::environment::var_os("CLEANUP_CPU").is_some_and(|v| v == "1")
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRuntime {
    pub pid: u32,
    pub model_id: String,
    pub backend: String,
    pub loading: bool,
    pub gpu_layers: Option<u32>,
    pub total_layers: Option<u32>,
    pub fallback_reason: Option<String>,
}
static TEXT_RUNTIME: Mutex<Option<TextRuntime>> = Mutex::new(None);

pub fn started(pid: u32, model: &str, backend: &str, stdout: ChildStdout, stderr: ChildStderr) {
    if let Ok(mut status) = TEXT_RUNTIME.lock() {
        *status = Some(TextRuntime {
            pid,
            model_id: model.into(),
            backend: backend.into(),
            loading: true,
            gpu_layers: None,
            total_layers: None,
            fallback_reason: None,
        });
    }
    // Drain logs even after startup. Only numeric offload evidence is retained;
    // prompts, model responses and server credentials are never saved here.
    let streams = Arc::new(AtomicUsize::new(2));
    drain_log(pid, stdout, streams.clone());
    drain_log(pid, stderr, streams);
}
fn drain_log(pid: u32, stream: impl Read + Send + 'static, streams: Arc<AtomicUsize>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut fragment = Vec::new();
        let mut line = Vec::new();
        loop {
            fragment.clear();
            let count = match reader.by_ref().take(4096).read_until(b'\n', &mut fragment) {
                Ok(n) => n,
                Err(_) => break,
            };
            if count == 0 {
                break;
            }
            if line.len() < 4096 {
                let remaining = 4096 - line.len();
                line.extend_from_slice(&fragment[..fragment.len().min(remaining)]);
            }
            if fragment.last() == Some(&b'\n') {
                if let Some((gpu, total)) = parse_offload(&String::from_utf8_lossy(&line)) {
                    if let Ok(mut status) = TEXT_RUNTIME.lock() {
                        if let Some(current) = status.as_mut().filter(|s| s.pid == pid) {
                            current.gpu_layers = Some(gpu);
                            current.total_layers = Some(total);
                        }
                    }
                }
                line.clear();
            }
        }
        if streams.fetch_sub(1, Ordering::AcqRel) == 1 {
            stopped(pid);
        }
    });
}
pub fn ready(pid: u32) {
    if let Ok(mut status) = TEXT_RUNTIME.lock() {
        if let Some(current) = status.as_mut().filter(|s| s.pid == pid) {
            current.loading = false;
        }
    }
}
pub fn fallback(pid: u32, reason: &str) {
    if let Ok(mut status) = TEXT_RUNTIME.lock() {
        if let Some(current) = status.as_mut().filter(|s| s.pid == pid) {
            current.fallback_reason = Some(reason.chars().take(240).collect());
        }
    }
}
pub fn stopped(pid: u32) {
    if let Ok(mut status) = TEXT_RUNTIME.lock() {
        if status.as_ref().is_some_and(|s| s.pid == pid) {
            *status = None;
        }
    }
}
fn parse_offload(line: &str) -> Option<(u32, u32)> {
    let (_, rest) = line.split_once("offloaded ")?;
    let (numbers, _) = rest.split_once(" layers to GPU")?;
    let (gpu, total) = numbers.trim().split_once('/')?;
    let (gpu, total) = (gpu.parse().ok()?, total.parse().ok()?);
    (total > 0 && gpu <= total).then_some((gpu, total))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuSupport {
    pub available: bool,
    pub backend: String,
    pub devices: Vec<String>,
    pub detail: String,
}
/// Structured ASR runtime identity (selection vs execution, never conflated).
/// `providerRequested` comes from the model ID (`asr::provider_requested`);
/// `providerInitialized` names what the native backend initialized (`cuda`
/// for the GPU adapter, `cpu` otherwise); `executionProof` is measured GPU
/// execution evidence or `None` when the worker supplies none (this build:
/// always `None` for speech — see `asr::Recognizer::execution_proof`).
/// `deviceIndex` is the selected CUDA device (normally 0) when the resident
/// model is a CUDA id, else `None`. Text numeric offload stays in
/// `textRuntime` (`parse_offload`, 0.18.4 proof) and is never copied here.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrRuntime {
    pub model_id: Option<String>,
    pub backend: Option<String>,
    pub compatibility: Option<String>,
    pub provider_requested: String,
    pub provider_initialized: String,
    pub device_index: Option<u32>,
    pub execution_proof: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccelerationStatus {
    pub text_gpu: GpuSupport,
    pub text_runtime: Option<TextRuntime>,
    pub asr_runtime: Option<AsrRuntime>,
    pub cpu_only: bool,
    pub speech_detail: String,
}
struct CachedProbe {
    executable: PathBuf,
    modified: Option<SystemTime>,
    checked: Instant,
    support: GpuSupport,
}
static GPU_PROBE: OnceLock<Mutex<Option<CachedProbe>>> = OnceLock::new();

pub fn inspect(root: &Path) -> AccelerationStatus {
    AccelerationStatus {
        text_gpu: gpu_support(root),
        text_runtime: TEXT_RUNTIME.lock().ok().and_then(|s| s.clone()),
        asr_runtime: asr_runtime(),
        cpu_only: cpu_only(),
        speech_detail: speech_detail(root),
    }
}

/// Best-effort structured ASR identity from coordinator residency (no locks
/// held across inference). When an ASR model is resident, reports its ID,
/// verbatim backend/compat, requested vs initialized providers, selected
/// device for CUDA ids, and `None` execution proof (honest: no numeric GPU
/// evidence exists for speech in this build). When nothing is resident,
/// reports an empty selection (`modelId: None`, cpu/cpu, no device) so UI can
/// distinguish "no model" from "CPU execution". Never claims observed GPU.
fn asr_runtime() -> Option<AsrRuntime> {
    let resident = crate::resource_manager::loaded_models();
    // Prefer the ASR-family resident; text rows are not ASR execution.
    let asr_id = resident.iter().find(|id| {
        *id == "moonshine-medium"
            || *id == "moonshine-small"
            || crate::optional_asr::is_optional(id)
            || crate::whisper_catalog::is_whisper(id)
            || crate::mac_speech::find(id).is_some()
            || crate::asr::is_gpu_model(id)
            || crate::llama_asr::is_llama_asr(id)
    });
    match asr_id {
        Some(id) => {
            let requested = crate::asr::provider_requested(id).to_owned();
            let initialized = if crate::asr::provider_requested(id) == "metal" {
                "metal".to_owned()
            } else if crate::asr::provider_requested(id) == "cuda" {
                "cuda".to_owned()
            } else {
                "cpu".to_owned()
            };
            let device = if crate::asr::provider_requested(id) == "cuda" {
                Some(crate::resource_manager::SELECTED_CUDA_DEVICE_INDEX)
            } else {
                None
            };
            Some(AsrRuntime {
                model_id: Some(id.clone()),
                backend: Some(crate::asr::backend_for(id).to_owned()),
                compatibility: Some(crate::asr::compatibility_for(id).to_owned()),
                provider_requested: requested,
                provider_initialized: initialized,
                device_index: device,
                execution_proof: None,
            })
        }
        None => Some(AsrRuntime {
            model_id: None,
            backend: None,
            compatibility: None,
            provider_requested: "cpu".into(),
            provider_initialized: "cpu".into(),
            device_index: None,
            execution_proof: None,
        }),
    }
}

/// Artifact-aware speech detail. Every installed CPU package stays CPU;
/// installed CUDA packages report their actual model ID/provider/backend with
/// verbatim NVIDIA-only compatibility, and no observed GPU execution is
/// claimed without numeric evidence. Requested provider "cuda" (from the ID
/// via `asr::provider_requested`) is selection, not execution: the native
/// factory provider alone is never observed-GPU proof. Hardware availability
/// (CUDA runtime + VRAM) is distinct from execution. Text numeric offload
/// (`parse_offload`, 0.18.4 layer proof) is preserved untouched.
fn speech_detail(root: &Path) -> String {
    use crate::model_verification::{Check, Status};
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return "Linux speech uses CPU: Moonshine streaming, sherpa-onnx, or Qwen3-ASR through llama.cpp. Text cleanup can use Vulkan; speech GPU offload is not enabled.".into();
    }
    let mut checking = false;
    let mut is_ready = |id: &str| {
        let status = crate::asr::model_status(root, id, Check::Background);
        checking |= status == Status::Checking;
        status.ready()
    };
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let count = crate::mac_speech::catalog().iter().filter(|m| is_ready(m.id)).count()
            + crate::llama_asr::catalog().iter()
                .filter(|m| crate::llama_asr::is_metal(m.id) && is_ready(m.id)).count();
        drop(is_ready);
        if checking { return format!("{count} Apple Metal speech models installed. Checking speech model files."); }
        return format!("{count} Apple Metal speech models installed. Qwen requires confirmed GPU layer offload; Whisper checks Metal initialization. Both run in isolated workers and share unified memory. Per-decode GPU activity is not measured.");
    }
    // Verbatim compatibility/backend from the pinned catalogs. These strings
    // already state CPU-only / CUDA-13-only + AMD-no-CUDA-path; reuse them so
    // labels cannot drift into a false GPU claim.
    let whisper_backend = crate::whisper_catalog::BACKEND;
    let whisper_compat = crate::whisper_catalog::COMPATIBILITY;
    // Count installed CPU artifacts without measuring performance.
    let mut installed: Vec<&str> = Vec::new();
    if is_ready("moonshine-medium")
        | is_ready("moonshine-small")
    {
        installed.push("Moonshine (CPU)");
    }
    if is_ready("parakeet-tdt-0.6b-v2") {
        installed.push("Parakeet (CPU)");
    }
    if is_ready("qwen3-asr-1.7b") {
        installed.push("Qwen3-ASR int8 (CPU)");
    }
    let mut whisper_ready = 0;
    for entry in crate::whisper_catalog::WHISPER_CATALOG {
        if is_ready(entry.id) {
            whisper_ready += 1;
        }
    }
    if whisper_ready > 0 {
        installed.push("Whisper (CPU)");
    }
    // Actual installed CUDA artifacts (model + runtime both verified).
    let mut gpu_installed: Vec<String> = Vec::new();
    for entry in crate::gpu_asr::catalog_metadata() {
        if is_ready(entry.id) {
            gpu_installed.push(format!(
                "{} (requested provider {}, backend {})",
                entry.id,
                crate::asr::provider_requested(entry.id),
                entry.backend
            ));
        }
    }
    for entry in crate::llama_asr::catalog() {
        if is_ready(entry.id) {
            gpu_installed.push(format!(
                "{} (requested provider {}, backend {})",
                entry.id, crate::asr::provider_requested(entry.id), entry.backend
            ));
        }
    }
    drop(is_ready);
    let runtime_status = crate::gpu_asr::runtime_status(root, Check::Background);
    checking |= runtime_status == Status::Checking;
    let runtime_ready = runtime_status.ready();
    let installed_text = if checking {
        "Checking speech model files.".to_owned()
    } else if installed.is_empty() && gpu_installed.is_empty() {
        "No optional speech package is installed yet.".to_owned()
    } else {
        let mut parts = Vec::new();
        if !installed.is_empty() {
            parts.push(format!("Installed CPU packages: {}", installed.join(", ")));
        }
        if !gpu_installed.is_empty() {
            parts.push(format!(
                "Installed CUDA packages: {}",
                gpu_installed.join("; ")
            ));
        }
        format!("{}.", parts.join(". "))
    };
    // Resident (actually loaded) models with requested provider + verbatim
    // backend/compatibility. Loaded means the coordinator marked readiness on
    // successful native load; it is still selection identity, not observed
    // execution proof. Text rows use the honest CPU/CUDA text labels.
    let resident = crate::resource_manager::loaded_models();
    let resident_text = if resident.is_empty() {
        "No speech/text model is resident now.".to_owned()
    } else {
        let mut rows = Vec::new();
        for id in &resident {
            if crate::optional_llm::find(id).is_some() {
                rows.push(format!(
                    "{id} (text; backend {}; compat: {})",
                    crate::optional_llm::backend_desc(),
                    crate::optional_llm::compatibility_desc()
                ));
            } else {
                rows.push(format!(
                    "{id} (requested provider {}, backend {}; compat: {})",
                    crate::asr::provider_requested(id),
                    crate::asr::backend_for(id),
                    crate::asr::compatibility_for(id)
                ));
            }
        }
        format!("Resident now: {}.", rows.join("; "))
    };
    let cuda_text = if runtime_status == Status::Checking {
        "Checking shared NVIDIA CUDA runtime files.".to_owned()
    } else if runtime_ready {
        format!(
            "Sherpa NVIDIA CUDA 13 runtime is installed ({}). Sherpa CUDA models require NVIDIA; AMD has no CUDA path. {}",
            crate::gpu_asr::GPU_RUNTIME_DIR,
            crate::gpu_asr::CUDA_COMPATIBILITY
        )
    } else {
        "Sherpa NVIDIA CUDA 13 runtime is not installed. Sherpa CUDA models disclose ~1.43 GiB shared runtime on first install; llama.cpp speech uses the separate bundled CUDA runtime. AMD has no CUDA path.".to_owned()
    };
    format!(
        "{} {} Speech CPU runs via {} / sherpa-onnx CPU + Moonshine CPU. {}. {} Compatibility: {}.",
        installed_text, resident_text, whisper_backend, cuda_text, "Requested provider cuda (ID selection) is never reported as observed GPU execution; observed GPU execution requires numeric evidence, which speech does not report in this build.", whisper_compat
    )
}
pub fn gpu_support(root: &Path) -> GpuSupport {
    let executable = if cfg!(target_os = "windows") {
        crate::cleanup::cuda_server_path(root)
    } else if cfg!(target_os = "linux") && crate::cleanup::vulkan_server_path(root).is_file() {
        crate::cleanup::vulkan_server_path(root)
    } else {
        crate::cleanup::server_path(root)
    };
    let modified = executable.metadata().ok().and_then(|m| m.modified().ok());
    let Ok(mut cache) = GPU_PROBE.get_or_init(|| Mutex::new(None)).lock() else {
        return unavailable("Hardware check unavailable.");
    };
    if let Some(cached) = cache.as_ref().filter(|c| {
        c.executable == executable
            && c.modified == modified
            // A cold runtime can exceed the bounded probe deadline. Retry a
            // failed check soon instead of showing unavailable for five minutes.
            && c.checked.elapsed()
                < Duration::from_secs(if c.support.available { 300 } else { 10 })
    }) {
        return cached.support.clone();
    }
    let support = if !executable.is_file() {
        unavailable(if cfg!(target_os = "linux") { "The Vulkan runtime is not installed. Install a local GGUF model in Setup." }
            else { "GPU runtime is not installed. On Windows this app supports NVIDIA CUDA, not AMD graphics." })
    } else if cfg!(target_os = "windows") || cfg!(target_os = "macos") || cfg!(target_os = "linux") {
        match device_output(&executable) {
            Ok(output) => support_from_output(&output),
            Err(_) => unavailable(if cfg!(target_os = "linux") { "Vulkan could not detect a compatible device. Check the Vulkan loader and graphics driver; CPU remains available." }
                else { "The GPU runtime could not detect a compatible device. Check its installation and graphics driver. Windows acceleration requires NVIDIA CUDA; AMD acceleration is not supported in this build." }),
        }
    } else {
        unavailable("GPU acceleration is not configured for this platform in this app.")
    };
    *cache = Some(CachedProbe {
        executable,
        modified,
        checked: Instant::now(),
        support: support.clone(),
    });
    support
}
fn unavailable(detail: &str) -> GpuSupport {
    GpuSupport {
        available: false,
        backend: "none".into(),
        devices: Vec::new(),
        detail: detail.into(),
    }
}
fn support_from_output(output: &str) -> GpuSupport {
    let devices: Vec<String> = output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, description) = line.split_once(": ")?;
            ((id.starts_with("CUDA") || id.starts_with("Metal") || id.starts_with("MTL") || id.starts_with("Vulkan")) && id.len() < 24
                && !description.to_ascii_lowercase().contains("llvmpipe")
                && !description.to_ascii_lowercase().contains("lavapipe")).then(|| {
                description
                    .split(" (")
                    .next()
                    .unwrap_or(description)
                    .chars()
                    .take(160)
                    .collect()
            })
        })
        .take(8)
        .collect();
    if devices.is_empty() {
        unavailable(if cfg!(target_os = "linux") { "No hardware Vulkan device was reported. Models will run on CPU." }
            else { "No compatible GPU was reported by the text runtime. Windows acceleration requires NVIDIA CUDA; AMD acceleration is not supported in this build." })
    } else {
        GpuSupport { available: true,
            backend: if output.lines().any(|l| l.trim().starts_with("CUDA")) { "cuda" }
                else if output.lines().any(|l| l.trim().starts_with("Vulkan")) { "vulkan" } else { "metal" }.into(),
            devices, detail: "Compatible GPU detected. The same downloaded text model can run on CPU or GPU; no second model download is needed.".into() }
    }
}
fn device_output(executable: &Path) -> Result<String, String> {
    let mut command = Command::new(executable);
    command
        .arg("--list-devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
        thread::spawn(move || {
            let mut out = Vec::new();
            let _ = pipe.by_ref().take(65536).read_to_end(&mut out);
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            out
        })
    }
    let stdout = drain(child.stdout.take().ok_or("Missing probe output")?);
    let stderr = drain(child.stderr.take().ok_or("Missing probe output")?);
    let start = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if start.elapsed() < Duration::from_secs(4) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let mut output = stdout.join().unwrap_or_default();
    output.extend(stderr.join().unwrap_or_default());
    if success {
        Ok(String::from_utf8_lossy(&output).into_owned())
    } else {
        Err("GPU probe failed".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn vulkan_requires_a_hardware_device() {
        let support = super::support_from_output("Available devices:\n  Vulkan0: NVIDIA GeForce RTX 5070 (12000 MiB, 11000 MiB free)");
        assert!(support.available);
        assert_eq!(support.backend, "vulkan");
        assert!(!super::support_from_output("Available devices:\n  Vulkan0: llvmpipe (LLVM 21.1.0, 256 bits)").available);
        assert!(!super::support_from_output("Available devices:\n  (none)").available);
    }
    #[test]
    fn mac_mtl_device_name_is_recognized() {
        let support = super::support_from_output("Available devices:\n  MTL0: Apple M4 (12124 MiB, 12123 MiB free)\n  BLAS: Accelerate (0 MiB, 0 MiB free)");
        assert!(support.available);
        assert_eq!(support.backend, "metal");
        assert_eq!(support.devices, vec!["Apple M4"]);
    }

    use super::*;
    #[test]
    fn existing_settings_keep_their_models_and_processing_policy() {
        let previous = serde_json::json!({"asrModel":"qwen3-asr-1.7b", "cleanupModel":"qwen3-4b-instruct-2507", "cleanupTrial":true});
        let mut settings: crate::domain::Settings =
            serde_json::from_value(previous.clone()).unwrap();
        assert_eq!(settings.text_acceleration, TextAcceleration::Auto);
        settings.text_acceleration = TextAcceleration::Cpu;
        let saved = serde_json::to_value(settings).unwrap();
        for key in ["asrModel", "cleanupModel", "cleanupTrial"] {
            assert_eq!(saved[key], previous[key]);
        }
        assert_eq!(saved["textAcceleration"], "cpu");
    }
    #[test]
    fn only_observed_offload_counts_as_gpu_execution() {
        assert_eq!(
            parse_offload("load_tensors: offloaded 37/37 layers to GPU"),
            Some((37, 37))
        );
        assert_eq!(
            parse_offload("load_tensors: offloaded 0/37 layers to GPU"),
            Some((0, 37))
        );
        assert_eq!(parse_offload("offloading 36 repeating layers to GPU"), None);
        assert_eq!(parse_offload("offloaded 99/37 layers to GPU"), None);
    }
    #[test]
    fn compatibility_requires_a_device_report_not_a_runtime_label() {
        let gpu = support_from_output(
            "Available devices:\n  CUDA0: NVIDIA GeForce RTX 5070 (12226 MiB, 9000 MiB free)\n",
        );
        assert!(gpu.available);
        assert_eq!(gpu.devices, ["NVIDIA GeForce RTX 5070"]);
        assert!(!support_from_output("ggml CUDA backend loaded\nAvailable devices:\n").available);
        assert!(!support_from_output("CPU: AMD Ryzen").available);
    }

    #[test]
    fn asr_runtime_never_claims_observed_gpu() {
        // No resident model: empty selection, cpu/cpu, no proof.
        let rt = asr_runtime().expect("asr_runtime always Some");
        assert!(rt.execution_proof.is_none());
        assert_eq!(rt.provider_requested, "cpu");
        assert_eq!(rt.provider_initialized, "cpu");
        // Serialized camelCase keys for the UI contract.
        let value = serde_json::to_value(&rt).unwrap();
        for key in [
            "modelId",
            "backend",
            "compatibility",
            "providerRequested",
            "providerInitialized",
            "deviceIndex",
            "executionProof",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        let status = serde_json::to_value(&AccelerationStatus {
            text_gpu: unavailable("test"),
            text_runtime: None,
            asr_runtime: Some(rt),
            cpu_only: false,
            speech_detail: "test".into(),
        })
        .unwrap();
        assert!(status.get("asrRuntime").is_some());
        assert!(status.get("speechDetail").is_some());
    }
}
