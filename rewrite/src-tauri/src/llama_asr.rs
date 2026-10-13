//! Audio-only Qwen3-ASR through the bundled llama.cpp server. The recognition
//! worker owns this child; the host owns sectioning and publication. Windows
//! uses sherpa VAD; Metal uses a separately pinned Silero speech gate.
use crate::optional_asr::{self, PinnedFile};
use base64::Engine as _;
use serde::Deserialize;
use std::{
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

pub const MODEL_ID: &str = "qwen3-asr-1.7b-q8-cuda";
pub const CPU_MODEL_ID: &str = "qwen3-asr-1.7b-q4-cpu";
pub const SMALL_CPU_MODEL_ID: &str = "qwen3-asr-0.6b-q8-cpu";
pub const METAL_Q8_MODEL_ID: &str = "qwen3-asr-1.7b-q8-metal";
pub const METAL_Q4_MODEL_ID: &str = "qwen3-asr-1.7b-q4-metal";

#[derive(Clone, Copy)]
pub enum Runtime {
    Cuda,
    // Runtime selection belongs to each catalog row.
    Cpu,
    Metal,
}

impl Runtime {
    fn executable(self, root: &Path) -> PathBuf {
        match self {
            Self::Cuda => crate::cleanup::cuda_server_path(root),
            Self::Cpu | Self::Metal => crate::cleanup::server_path(root),
        }
    }

    pub fn provider(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Cpu => "cpu",
            Self::Metal => "metal",
        }
    }

    fn gpu_layers(self) -> &'static str {
        match self {
            Self::Cuda | Self::Metal => "999",
            Self::Cpu => "0",
        }
    }

    fn supports(self, os: &str, arch: &str) -> bool {
        match self {
            Self::Cuda | Self::Cpu => os == "windows" && arch == "x86_64",
            Self::Metal => os == "macos" && arch == "aarch64",
        }
    }

    pub fn supported_platform(self) -> bool {
        self.supports(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn platform_requirement(self) -> &'static str {
        match self {
            Self::Cuda | Self::Cpu => "This speech model requires Windows x64.",
            Self::Metal => "This speech model requires an Apple Silicon Mac.",
        }
    }

    fn needs_vad(self) -> bool {
        !matches!(self, Self::Metal)
    }

    fn runtime_ready(self, root: &Path) -> bool {
        match self {
            Self::Metal => crate::cleanup::mac_runtime_ready(root),
            _ => self.executable(root).is_file(),
        }
    }

    fn vad_status(self, root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
        if self.needs_vad() {
            optional_asr::runtime_status(root, check)
        } else {
            optional_asr::mac_speech_runtime_status(root, check)
        }
    }

    fn runtime_download_bytes(self, root: &Path) -> u64 {
        if self.needs_vad() { optional_asr::runtime_download_bytes(root) }
        else { crate::model_manager::mac_runtime_download_bytes(root)
            + optional_asr::mac_speech_runtime_download_bytes(root) }
    }

    fn offload_confirmed(self, layers: u32) -> bool {
        matches!(self, Self::Cpu) || layers > 0
    }
}

pub struct LlamaModel {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Source of the decoder; shown in the library.
    pub repo: &'static str,
    pub revision: &'static str,
    pub decoder: PinnedFile,
    /// The audio projector may come from a different pinned repository.
    pub projector_repo: &'static str,
    pub projector_revision: &'static str,
    pub projector: PinnedFile,
    pub runtime: Runtime,
    pub license: &'static str,
    pub backend: &'static str,
    pub quantization: &'static str,
    pub compatibility: &'static str,
    pub guidance: &'static str,
    pub evidence: &'static str,
}

const GGML_ORG_REPO: &str = "ggml-org/Qwen3-ASR-1.7B-GGUF";
const GGML_ORG_REVISION: &str = "36a678687ba7d07a74ca70ccb0e36902e005fb80";
const GGML_ORG_DECODER_Q8: PinnedFile = PinnedFile {
    relative: "Qwen3-ASR-1.7B-Q8_0.gguf",
    bytes: 2_165_034_944,
    sha256: "58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57",
};
const Q4_REPO: &str = "mradermacher/Qwen3-ASR-1.7B-GGUF";
const Q4_REVISION: &str = "cc946c78d3804752f7ba1bc42720c0f7aaf3d1ad";
// Tensor data matches Q4_K_M quantized from ggml-org's BF16 by b10900;
// only GGUF metadata differs. CPU and Metal use this same artifact.
const DECODER_Q4: PinnedFile = PinnedFile {
    relative: "Qwen3-ASR-1.7B.Q4_K_M.gguf",
    bytes: 1_282_435_552,
    sha256: "3893b8926065bbff3da7586d21d8711a9b4fa4fa8f12cd0cefad58e31b2660b6",
};
const GGML_ORG_PROJECTOR_Q8: PinnedFile = PinnedFile {
    relative: "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
    bytes: 355_709_344,
    sha256: "46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d",
};

// Each artifact has one pin shared by every runtime that loads it.
const MODELS: &[LlamaModel] = &[LlamaModel {
    id: MODEL_ID,
    name: "Qwen3-ASR 1.7B (Q8 CUDA)",
    description: "Multilingual · NVIDIA CUDA · llama.cpp",
    repo: GGML_ORG_REPO,
    revision: GGML_ORG_REVISION,
    decoder: GGML_ORG_DECODER_Q8,
    projector_repo: GGML_ORG_REPO,
    projector_revision: GGML_ORG_REVISION,
    projector: GGML_ORG_PROJECTOR_Q8,
    runtime: Runtime::Cuda,
    license: "Apache-2.0 (Qwen3-ASR weights; GGUF conversion by ggml-org); llama.cpp MIT",
    backend: "llama.cpp b10900 CUDA 13.3 (Qwen3-ASR Q8_0 decoder + Q8_0 audio encoder; VAD CPU)",
    quantization: "Q8_0 decoder + Q8_0 audio encoder",
    compatibility: "Windows x64, NVIDIA GPU compatible with bundled CUDA 13.3; AMD has no CUDA path. Reuses the sherpa CPU runtime for VAD.",
    guidance: "Multilingual speech on NVIDIA CUDA. Requires the bundled llama.cpp CUDA runtime; installs the shared CPU voice detector when needed.",
    evidence: "2026-09-28, RTX 5070, pinned llama-server b10900 CUDA 13.3, 12 public TED clips (412 s): holdout 31/648 primary word errors (official BF16 on Mac: 30), warm 0.21 s per ~9 s clip and 1.08 s per ~60 s clip, 1.6 s server load, +3.4 GB device memory. Through the app worker: holdout 33/648, text 107-229 ms after audio ends, about 1.0 GB host memory; see rewrite/evaluation/BENCHMARK-WINDOWS-2026-09-28.md.",
}, LlamaModel {
    id: CPU_MODEL_ID,
    name: "Qwen3-ASR 1.7B (Q4 CPU)",
    description: "Multilingual · CPU · llama.cpp",
    repo: Q4_REPO,
    revision: Q4_REVISION,
    decoder: DECODER_Q4,
    projector_repo: GGML_ORG_REPO,
    projector_revision: GGML_ORG_REVISION,
    projector: GGML_ORG_PROJECTOR_Q8,
    runtime: Runtime::Cpu,
    license: "Apache-2.0 (Qwen3-ASR weights; Q4_K_M by mradermacher, audio encoder by ggml-org); llama.cpp MIT",
    backend: "llama.cpp b10900 CPU (Qwen3-ASR Q4_K_M decoder + Q8_0 audio encoder; VAD CPU)",
    quantization: "Q4_K_M decoder + Q8_0 audio encoder",
    compatibility: "Windows x64 CPU (AVX2 or newer recommended). No GPU required. Reuses the sherpa CPU runtime for VAD.",
    guidance: "Most accurate CPU option. Best on 8 or more cores; text for a pause-ended phrase arrives about 1 s after you stop on a fast desktop, longer on laptops. Uses about 2.1 GB of RAM while loaded.",
    evidence: "2026-09-28, Ryzen 9 9900X, llama-server b10900 CPU, 12 public TED clips: holdout 31/648 primary word errors with 8 threads (33/648 with 4), warm 0.94 s per ~9 s clip and 6.7 s per ~60 s clip at 8 threads (1.24 s and 9.0 s at 4), 2.34 GB server resident memory. Through the app worker (12 threads): holdout 31/648, text 0.8-1.1 s after audio ends (p95 2.1 s), 2.14 GB peak memory; see rewrite/evaluation/BENCHMARK-WINDOWS-2026-09-28.md.",
}, LlamaModel {
    id: SMALL_CPU_MODEL_ID,
    name: "Qwen3-ASR 0.6B (Q8 CPU)",
    description: "Multilingual · CPU · llama.cpp",
    repo: "ggml-org/Qwen3-ASR-0.6B-GGUF",
    revision: "928ab958557df9aa2ef1c93e0e83c7ad0933fae2",
    decoder: PinnedFile {
        relative: "Qwen3-ASR-0.6B-Q8_0.gguf",
        bytes: 804_749_248,
        sha256: "bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971",
    },
    projector_repo: "ggml-org/Qwen3-ASR-0.6B-GGUF",
    projector_revision: "928ab958557df9aa2ef1c93e0e83c7ad0933fae2",
    projector: PinnedFile {
        relative: "mmproj-Qwen3-ASR-0.6B-Q8_0.gguf",
        bytes: 214_392_480,
        sha256: "41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d",
    },
    runtime: Runtime::Cpu,
    license: "Apache-2.0 (Qwen3-ASR weights; GGUF conversion by ggml-org); llama.cpp MIT",
    backend: "llama.cpp b10900 CPU (Qwen3-ASR Q8_0 decoder + Q8_0 audio encoder; VAD CPU)",
    quantization: "Q8_0 decoder + Q8_0 audio encoder",
    compatibility: "Windows x64 CPU (AVX2 or newer recommended). No GPU required. Reuses the sherpa CPU runtime for VAD.",
    guidance: "Smaller multilingual speech model for computers with less memory or fewer CPU cores.",
    evidence: "Pinned Q8 decoder and audio encoder total 1,019,141,728 bytes. Runtime performance depends on the computer.",
}, LlamaModel {
    id: METAL_Q8_MODEL_ID,
    name: "Qwen3-ASR 1.7B (Q8 Metal)",
    description: "Multilingual · Apple Metal · llama.cpp",
    repo: GGML_ORG_REPO,
    revision: GGML_ORG_REVISION,
    decoder: GGML_ORG_DECODER_Q8,
    projector_repo: GGML_ORG_REPO,
    projector_revision: GGML_ORG_REVISION,
    projector: GGML_ORG_PROJECTOR_Q8,
    runtime: Runtime::Metal,
    license: "Apache-2.0 (Qwen3-ASR weights; GGUF conversion by ggml-org); llama.cpp MIT",
    backend: "llama.cpp b10964 Metal (Qwen3-ASR Q8_0 decoder + Q8_0 audio encoder; sectioning local)",
    quantization: "Q8_0 decoder + Q8_0 audio encoder",
    compatibility: "Apple Silicon Mac · Metal GPU. Includes a CPU Silero speech detector with pinned ONNX Runtime; no sherpa runtime.",
    guidance: "Multilingual speech on Apple Silicon. Uses the shared Metal runtime and local audio sectioning. Q8 made fewer word errors than Q4 in the measured app-worker clips; Q4 used less memory.",
    evidence: "2026-09-28, M1 Pro 16 GB, llama-server b10964: full clips with English prefill scored 21/576 development and 31/648 holdout errors. Through the app worker with automatic language and 20-second sections: 24/576 development, 40/648 holdout in file and real-time replay, warm short/long medians 0.67/4.40 s, text 0.75/1.50 s after audio ends (short/long medians, overall p95 1.73 s), peak process-tree RSS 2.91 GB. Unified memory is counted once. No physical 8 GB Mac measurement. See rewrite/evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.",
}, LlamaModel {
    id: METAL_Q4_MODEL_ID,
    name: "Qwen3-ASR 1.7B (Q4 Metal)",
    description: "Multilingual · Apple Metal · llama.cpp",
    repo: Q4_REPO,
    revision: Q4_REVISION,
    decoder: DECODER_Q4,
    projector_repo: GGML_ORG_REPO,
    projector_revision: GGML_ORG_REVISION,
    projector: GGML_ORG_PROJECTOR_Q8,
    runtime: Runtime::Metal,
    license: "Apache-2.0 (Qwen3-ASR weights; Q4_K_M by mradermacher, audio encoder by ggml-org); llama.cpp MIT",
    backend: "llama.cpp b10964 Metal (Qwen3-ASR Q4_K_M decoder + Q8_0 audio encoder; sectioning local)",
    quantization: "Q4_K_M decoder + Q8_0 audio encoder",
    compatibility: "Apple Silicon Mac · Metal GPU. Includes a CPU Silero speech detector with pinned ONNX Runtime; no sherpa runtime.",
    guidance: "Smaller Qwen speech option for Apple Silicon. Uses less memory and loads faster than Q8, with more word errors in the measured app-worker clips. Uses local audio sectioning.",
    evidence: "2026-09-28, M1 Pro 16 GB, llama-server b10964: full clips with English prefill scored 21/576 development and 31/648 holdout errors. Through the app worker with automatic language and 20-second sections: 25/576 development, 47/648 holdout in file and real-time replay, warm short/long medians 0.63/4.10 s, text 0.67/1.21 s after audio ends (short/long medians, overall p95 1.30 s), peak process-tree RSS 2.35 GB. Unified memory is counted once. No physical 8 GB Mac measurement. See rewrite/evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.",
}];

pub fn catalog() -> &'static [LlamaModel] {
    MODELS
}

pub fn find(id: &str) -> Option<&'static LlamaModel> {
    catalog().iter().find(|model| model.id == id)
}

pub fn is_llama_asr(id: &str) -> bool {
    find(id).is_some()
}

pub fn is_metal(id: &str) -> bool {
    find(id).is_some_and(|model| matches!(model.runtime, Runtime::Metal))
}

pub fn model_dir(root: &Path, id: &str) -> PathBuf {
    optional_asr::optional_root(root).join(id)
}

pub fn package_bytes(model: &LlamaModel) -> u64 {
    model.decoder.bytes + model.projector.bytes
}

pub fn download_total_bytes(root: &Path, id: &str) -> Option<u64> {
    download_bytes_with_check(root, id, crate::model_verification::Check::Background)
}

pub(crate) fn download_bytes_with_check(root: &Path, id: &str, check: crate::model_verification::Check) -> Option<u64> {
    find(id).map(|model| {
        // Blocking installation checks establish receipts before computing the
        // shared-runtime budget. Catalog reads remain asynchronous.
        let _ = model.runtime.vad_status(root, check);
        let weights = if check.files(root, &model_dir(root, id), &[model.decoder, model.projector]).ready() {
            0
        } else {
            package_bytes(model)
        };
        weights + model.runtime.runtime_download_bytes(root)
    })
}

pub fn model_ready(root: &Path, id: &str) -> bool {
    model_status(root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    let Some(model) = find(id) else { return Status::Missing };
    if !model.runtime.supported_platform() { return Status::Missing; }
    Status::all([
        Status::present(model.runtime.runtime_ready(root)),
        model.runtime.vad_status(root, check),
        check.files(root, &model_dir(root, model.id), &[model.decoder, model.projector]),
    ])
}

pub fn install_model(
    root: &Path,
    id: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let model = find(id).ok_or("Choose a downloadable llama.cpp speech model.")?;
    check_cancel(cancel)?;
    if !model.runtime.supported_platform() {
        return Err(model.runtime.platform_requirement().into());
    }
    {
        // Refuse before downloading anything if the app's required runtime is absent.
        if !model.runtime.runtime_ready(root) {
            let runtime = match model.runtime {
                Runtime::Cuda => "CUDA 13.3",
                Runtime::Cpu => "CPU",
                Runtime::Metal => "b10964 Metal",
            };
            return Err(format!("The bundled llama.cpp {runtime} runtime is missing. Restore the app's speech runtime before installing this model."));
        }
        let total = download_bytes_with_check(root, id, crate::model_verification::Check::Blocking)
            .ok_or("Unknown speech model.")?;
        let runtime_bytes = model.runtime.runtime_download_bytes(root);
        if model_ready(root, id) {
            progress(total, total);
            return Ok(());
        }
        // Include staged weights, shared runtime archives/unpacking and headroom.
        let disk_path = root.ancestors().find(|path| path.is_dir()).unwrap_or(root);
        if let Some(free) = crate::resource_manager::disk_free_bytes(disk_path) {
            let need = total.saturating_add(256 << 20);
            if free < need {
                return Err(format!(
                    "Not enough disk space: need about {} free, found {}. Free space and try again.",
                    crate::resource_manager::format_bytes(Some(need)),
                    crate::resource_manager::format_bytes(Some(free)),
                ));
            }
        }
        progress(0, total);
        if model.runtime.needs_vad() {
            optional_asr::ensure_runtime_with_cancel(root, cancel, &|done, _| {
                progress(done.min(runtime_bytes), total);
            })
            .map_err(|error| check_cancel(cancel).err().unwrap_or(error))?;
        } else {
            optional_asr::ensure_mac_speech_runtime(root, cancel, &|done, _| {
                progress(done.min(runtime_bytes), total);
            })
            .map_err(|error| check_cancel(cancel).err().unwrap_or(error))?;
        }
        check_cancel(cancel)?;
        // Repairing only the shared detector must not redownload gigabytes of
        // already verified decoder/projector weights.
        if model_ready(root, id) {
            progress(total, total);
            return Ok(());
        }
        let optional = optional_asr::optional_root(root);
        std::fs::create_dir_all(&optional)
            .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
        optional_asr::ensure_within(root, &optional)?;
        let dest = model_dir(root, model.id);
        optional_asr::ensure_within(root, &dest)?;
        let staging = optional.join(format!(".install-{}-{}", model.id, crate::store::new_id()));
        std::fs::create_dir(&staging)
            .map_err(|e| format!("Cannot create a staging folder: {e}"))?;
        let installed = (|| {
            let mut done = runtime_bytes;
            for (repo, revision, file) in [
                (model.repo, model.revision, model.decoder),
                (model.projector_repo, model.projector_revision, model.projector),
            ] {
                check_cancel(cancel)?;
                let url = format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    repo, revision, file.relative,
                );
                crate::whisper_catalog::download_with_progress(
                    &url,
                    &staging.join(file.relative),
                    file.bytes,
                    file.sha256,
                    cancel,
                    &|got, _| progress((done + got.min(file.bytes)).min(total - 1), total),
                )
                .map_err(|error| check_cancel(cancel).err().unwrap_or(error))?;
                done += file.bytes;
                check_cancel(cancel)?;
                if !optional_asr::file_matches(&staging.join(file.relative), &file)? {
                    return Err(format!("The downloaded {} did not match its pinned checksum. Nothing was installed.", file.relative));
                }
            }
            check_cancel(cancel)?;
            optional_asr::transactional_activate(root, &staging, &dest, "llama.cpp speech model")?;
            progress(total, total);
            Ok(())
        })();
        if installed.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        installed
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Install cancelled.".into())
    } else {
        Ok(())
    }
}

fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, String> {
    let mut output = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut output,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| format!("Cannot encode speech audio: {e}"))?;
    for &sample in samples {
        if !sample.is_finite() {
            return Err("Speech audio contains a non-finite sample.".into());
        }
        // Rust's float-to-int cast saturates at both i16 bounds, including +1.
        let pcm = (sample.clamp(-1.0, 1.0) * 32_768.0).round() as i16;
        writer
            .write_sample(pcm)
            .map_err(|e| format!("Cannot encode speech audio: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("Cannot finish speech audio: {e}"))?;
    Ok(output.into_inner())
}

fn request_body(samples: &[f32]) -> Result<serde_json::Value, String> {
    let audio = base64::engine::general_purpose::STANDARD.encode(encode_wav(samples)?);
    let body = serde_json::json!({
        "messages": [{"role":"user", "content":[{
            "type":"input_audio", "input_audio":{"data":audio, "format":"wav"}
        }]}],
        "temperature":0,
        "max_tokens":crate::sherpa::qwen_max_tokens(samples.len()),
        "cache_prompt":false,
    });
    #[cfg(feature = "fidelity-eval")]
    {
        let mut body = body;
        add_eval_language(
            &mut body,
            std::env::var("SONOJOT_EVAL_ASR_LANGUAGE").ok().as_deref(),
        );
        return Ok(body);
    }
    #[cfg(not(feature = "fidelity-eval"))]
    Ok(body)
}

#[cfg(feature = "fidelity-eval")]
fn add_eval_language(body: &mut serde_json::Value, language: Option<&str>) {
    if language == Some("en") {
        body["messages"]
            .as_array_mut()
            .expect("audio messages")
            .push(serde_json::json!({"role":"assistant", "content":"language English<asr_text>"}));
    }
}

struct LocalApi {
    client: reqwest::blocking::Client,
    base: String,
    key: String,
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
    finish_reason: String,
}
#[derive(Deserialize)]
struct Message {
    content: String,
}
#[derive(Deserialize)]
struct Usage {
    completion_tokens: Option<i64>,
}

impl LocalApi {
    fn new(port: u16, key: String) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("Cannot prepare local speech HTTP: {e}"))?;
        Ok(Self {
            client,
            base: format!("http://127.0.0.1:{port}"),
            key,
        })
    }

    fn transcribe(&self, samples: &[f32]) -> Result<(String, Option<i64>, String), String> {
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base))
            .bearer_auth(&self.key)
            .json(&request_body(samples)?)
            .send()
            .map_err(|e| format!("Local speech request failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Local speech server refused the request: {e}"))?;
        const RESPONSE_LIMIT: u64 = 4 * 1024 * 1024;
        let mut bytes = Vec::new();
        response
            .take(RESPONSE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Cannot read local speech response: {e}"))?;
        if bytes.len() as u64 > RESPONSE_LIMIT {
            return Err("Local speech response exceeded the size limit.".into());
        }
        let completion: Completion = serde_json::from_slice(&bytes)
            .map_err(|_| "Local speech server returned malformed completion JSON.".to_owned())?;
        let choice = completion
            .choices
            .into_iter()
            .next()
            .ok_or("Local speech server returned no completion choice.")?;
        if !matches!(choice.finish_reason.as_str(), "stop" | "length") {
            return Err("Local speech server returned an unsupported finish reason.".into());
        }
        let tokens = completion.usage.and_then(|usage| usage.completion_tokens);
        if tokens.is_some_and(|count| count < 0) {
            return Err("Local speech server returned an invalid completion token count.".into());
        }
        Ok((choice.message.content, tokens, choice.finish_reason))
    }
}

pub struct Server {
    child: Child,
    pid: u32,
    api: LocalApi,
    backend: &'static str,
    #[cfg(target_os = "macos")]
    _parent_watch: Option<std::os::unix::net::UnixStream>,
    #[cfg(windows)]
    _job: ProcessJob,
}

impl Server {
    pub fn launch(root: &Path, id: &str) -> Result<Self, String> {
        let model = find(id).ok_or("Unknown llama.cpp speech model.")?;
        // Installation/readiness requires the detector too. A worker already
        // selected before its files disappeared can still use the conservative
        // fallback gate in mac_speech; weights and the decoder stay verified.
        let ready = if is_metal(id) {
            model.runtime.supported_platform() && model.runtime.runtime_ready(root)
                && crate::model_verification::Check::Blocking.files(
                    root, &model_dir(root, id), &[model.decoder, model.projector],
                ).ready()
        } else {
            model_ready(root, id)
        };
        if !ready {
            return Err(format!(
                "{id} is not installed with its required speech runtime."
            ));
        }
        let executable = model
            .runtime
            .executable(root)
            .canonicalize()
            .map_err(|e| format!("Cannot resolve speech server: {e}"))?;
        let dir = model_dir(root, model.id);
        let decoder = dir
            .join(model.decoder.relative)
            .canonicalize()
            .map_err(|e| format!("Cannot resolve speech decoder: {e}"))?;
        let projector = dir
            .join(model.projector.relative)
            .canonicalize()
            .map_err(|e| format!("Cannot resolve speech audio encoder: {e}"))?;
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Cannot bind local speech port: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let api = LocalApi::new(port, uuid::Uuid::new_v4().to_string())?;
        #[cfg(target_os = "macos")]
        let parent_watch = if crate::recognition::is_worker() {
            let parent = unsafe { libc::getppid() };
            if parent <= 1 {
                return Err("The recognition worker's owner has exited.".into());
            }
            let watch = MacParentWatch::new(parent)?;
            if unsafe { libc::getppid() } != parent {
                return Err("The recognition worker's owner has exited.".into());
            }
            Some(watch)
        } else {
            None
        };
        let mut command = server_command(&executable);
        command
            .arg("-m")
            .arg(decoder)
            .arg("--mmproj")
            .arg(projector)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--api-key",
                &api.key,
                // A 28 s section is ~364 audio tokens plus at most 512 output
                // tokens; 2,048 leaves headroom and saves ~0.2 GB of KV cache.
                "--ctx-size",
                "2048",
                "--parallel",
                "1",
                "--n-gpu-layers",
                model.runtime.gpu_layers(),
                // Read weights instead of memory-mapping them: measured host
                // RAM fell 1.94 -> 1.06 GB on CUDA and 2.82 -> 2.34 GB on CPU
                // with identical output and speed.
                "--load-mode",
                "none",
                "--no-webui",
                "--cache-ram",
                "0",
                "--log-colors",
                "off",
                // As with cleanup, retain numeric offload evidence only.
                "--log-verbosity",
                "4",
            ])
            .current_dir(executable.parent().ok_or("Invalid speech server path.")?)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(not(target_os = "macos"))]
        command.stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        drop(listener);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start local speech server: {e}"))?;
        #[cfg(windows)]
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        // Own the child before any fallible health request so every failure
        // (including timeout) uses the same kill-and-wait Drop path.
        let pid = child.id();
        let mut server = Self {
            child,
            pid,
            api,
            backend: model.runtime.provider(),
            #[cfg(target_os = "macos")]
            _parent_watch: None,
            #[cfg(windows)]
            _job: job,
        };
        #[cfg(target_os = "macos")]
        {
            if let Some(watch) = parent_watch {
                let stdin = server.child.stdin.take().ok_or("Missing speech lifetime pipe.")?;
                server._parent_watch = Some(watch.start(stdin)?);
            }
            server.pid = supervisor_server_pid(&mut server.child)?;
        }
        let offloaded = Arc::new(AtomicU32::new(0));
        if let Some(stdout) = server.child.stdout.take() {
            drain_log(stdout, offloaded.clone());
        }
        if let Some(stderr) = server.child.stderr.take() {
            drain_log(stderr, offloaded.clone());
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut healthy = false;
        while Instant::now() < deadline {
            if let Some(status) = server.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "Local speech server exited during loading ({status})."
                ));
            }
            healthy = server
                .api
                .client
                .get(format!("{}/health", server.api.base))
                .bearer_auth(&server.api.key)
                .timeout(Duration::from_millis(250))
                .send()
                .is_ok_and(|response| response.status() == reqwest::StatusCode::OK);
            if healthy
                && model.runtime.offload_confirmed(offloaded.load(Ordering::Acquire))
            {
                return Ok(server);
            }
            thread::sleep(Duration::from_millis(50));
        }
        if healthy {
            Err(format!("Local speech server did not confirm {} layer offload.", model.runtime.provider()))
        } else {
            Err("Local speech model did not load within 120 seconds.".into())
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
    /// Successful CUDA/Metal loads require numeric GPU-layer offload from this child.
    pub fn backend(&self) -> &str {
        self.backend
    }

    pub fn transcribe(
        &self,
        samples_16k_mono: &[f32],
    ) -> Result<(String, Option<i64>, String), String> {
        self.api.transcribe(samples_16k_mono)
    }
}

// Keep no text logs: server logs can contain recognized speech or credentials.
// Bounded line fragments also prevent unbounded memory use on malformed logs.
fn drain_log(stream: impl Read + Send + 'static, offloaded: Arc<AtomicU32>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        let mut fragment = Vec::new();
        loop {
            fragment.clear();
            if !matches!(reader.by_ref().take(4096).read_until(b'\n', &mut fragment), Ok(n) if n > 0)
            {
                break;
            }
            let remaining = 4096usize.saturating_sub(line.len());
            line.extend_from_slice(&fragment[..fragment.len().min(remaining)]);
            if fragment.last() == Some(&b'\n') {
                if let Some(count) = offloaded_layers(&String::from_utf8_lossy(&line)) {
                    offloaded.fetch_max(count, Ordering::Release);
                }
                line.clear();
            }
        }
    });
}

fn offloaded_layers(line: &str) -> Option<u32> {
    let (_, rest) = line.split_once("offloaded ")?;
    let (counts, _) = rest.split_once(" layers to GPU")?;
    let (gpu, total) = counts.trim().split_once('/')?;
    let (gpu, total) = (gpu.parse::<u32>().ok()?, total.parse::<u32>().ok()?);
    (gpu > 0 && gpu <= total).then_some(gpu)
}

impl Drop for Server {
    fn drop(&mut self) {
        stop_server(&mut self.child);
    }
}

#[cfg(not(target_os = "macos"))]
fn server_command(executable: &Path) -> Command {
    Command::new(executable)
}

#[cfg(target_os = "macos")]
fn server_command(executable: &Path) -> Command {
    use std::os::unix::process::CommandExt;
    // The shell owns a fresh process group, waits for the server and gives a
    // background reader the lifetime pipe. Worker SIGKILL closes that pipe
    // even during a blocking HTTP request. A CHLD trap around a foreground
    // read is insufficient: macOS sh defers that trap until read returns.
    // Kill the group while its leader is still alive, so a recycled server PID
    // can never target an unrelated process. Arguments never enter shell text.
    // Server output goes to stderr; stdout is reserved for the PID handshake.
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "trap 'kill -KILL -- \"-$$\"' EXIT HUP INT TERM\nexec 3<&0\n\"$@\" </dev/null 3<&- 1>&2 &\nserver=$!\nprintf '%s\\n' \"$server\"\n( IFS= read -r lifetime <&3; kill -KILL -- \"-$$\" ) &\nexec 3<&-\nwait \"$server\"\n",
        "sonojot-speech-supervisor",
    ]).arg(executable).stdin(Stdio::piped()).process_group(0);
    command
}

#[cfg(target_os = "macos")]
fn supervisor_server_pid(child: &mut Child) -> Result<u32, String> {
    let stdout = child.stdout.take().ok_or("Missing speech server PID pipe.")?;
    let mut line = String::new();
    BufReader::new(stdout).take(32).read_line(&mut line)
        .map_err(|e| format!("Cannot read speech server PID: {e}"))?;
    line.trim().parse::<u32>().ok().filter(|pid| *pid > 1 && *pid != child.id())
        .ok_or_else(|| "Invalid speech server PID from supervisor.".into())
}

fn stop_server(child: &mut Child) {
    #[cfg(target_os = "macos")]
    {
        // An unreaped, live supervisor pins its process-group identity.
        // The EOF/exit trap handles teardown if the supervisor already exited.
        if matches!(child.try_wait(), Ok(None)) {
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL); }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = child.kill();
    let _ = child.wait();
}

// The worker's parent may die while the worker is inside native transcription
// and cannot read socket EOF. Observe the application with a kernel process
// event, independently of that request, and close the supervisor lifetime pipe.
// No PID polling, fork callbacks, third-party helper, or on-disk script is used.
#[cfg(target_os = "macos")]
struct MacParentWatch {
    queue: std::os::fd::OwnedFd,
    control: std::os::unix::net::UnixStream,
    stop: std::os::unix::net::UnixStream,
}

#[cfg(target_os = "macos")]
impl MacParentWatch {
    fn new(parent: libc::pid_t) -> Result<Self, String> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let (control, stop) = std::os::unix::net::UnixStream::pair()
            .map_err(|e| format!("Cannot create speech owner pipe: {e}"))?;
        let fd = unsafe { libc::kqueue() };
        if fd < 0 { return Err("Cannot watch speech worker owner.".into()); }
        let queue = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err("Cannot protect speech owner watcher descriptor.".into());
        }
        let events = [
            libc::kevent {
                ident: parent as libc::uintptr_t, filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT, fflags: libc::NOTE_EXIT,
                data: 0, udata: std::ptr::null_mut(),
            },
            libc::kevent {
                ident: control.as_raw_fd() as libc::uintptr_t, filter: libc::EVFILT_READ,
                flags: libc::EV_ADD | libc::EV_ONESHOT, fflags: 0,
                data: 0, udata: std::ptr::null_mut(),
            },
        ];
        if unsafe { libc::kevent(fd, events.as_ptr(), events.len() as i32,
            std::ptr::null_mut(), 0, std::ptr::null()) } < 0
        {
            return Err(format!("Cannot watch speech worker owner: {}", std::io::Error::last_os_error()));
        }
        Ok(Self { queue, control, stop })
    }

    fn start(self, lifetime: std::process::ChildStdin) -> Result<std::os::unix::net::UnixStream, String> {
        use std::os::fd::AsRawFd;
        let Self { queue, control, stop } = self;
        thread::Builder::new().name("speech-owner-watch".into()).spawn(move || {
            let mut event: libc::kevent = unsafe { std::mem::zeroed() };
            loop {
                let result = unsafe { libc::kevent(queue.as_raw_fd(), std::ptr::null(), 0,
                    &mut event, 1, std::ptr::null()) };
                if result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            drop(lifetime);
            drop(control);
        }).map_err(|e| format!("Cannot start speech owner watcher: {e}"))?;
        Ok(stop)
    }
}

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
// Exclusive owned handle, closed only by Drop; Win32 job operations are thread-safe.
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
impl ProcessJob {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "Cannot create speech server job: {}",
                std::io::Error::last_os_error()
            ));
        }
        let job = Self(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle, child.as_raw_handle()) } == 0
        {
            return Err(format!(
                "Cannot contain speech server lifetime: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(job)
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn wav_round_trip_preserves_format_count_and_clips_pcm() {
        let wav = encode_wav(&[-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0]).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let mut reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(
            reader.spec(),
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            }
        );
        assert_eq!(reader.duration(), 7);
        assert_eq!(
            reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![i16::MIN, i16::MIN, -16384, 0, 16384, i16::MAX, i16::MAX]
        );
        assert!(encode_wav(&[f32::NAN]).is_err());
    }

    #[test]
    fn request_is_audio_only_with_exact_sampling_fields() {
        let body = request_body(&[0.25, -0.25]).unwrap();
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(encode_wav(&[0.25, -0.25]).unwrap());
        let expected = serde_json::json!({
            "messages":[{"role":"user", "content":[{"type":"input_audio", "input_audio":{"data":encoded,"format":"wav"}}]}],
            "temperature":0,"max_tokens":33,"cache_prompt":false,
        });
        #[cfg(feature = "fidelity-eval")]
        let expected = {
            let mut expected = expected;
            add_eval_language(
                &mut expected,
                std::env::var("SONOJOT_EVAL_ASR_LANGUAGE").ok().as_deref(),
            );
            expected
        };
        assert_eq!(body, expected);
    }

    #[cfg(feature = "fidelity-eval")]
    #[test]
    fn eval_prefill_is_only_english_and_only_a_trailing_assistant() {
        let base = serde_json::json!({"messages":[{"role":"user","content":[]}]});
        for language in [None, Some("auto"), Some("fr"), Some("EN")] {
            let mut body = base.clone();
            add_eval_language(&mut body, language);
            assert_eq!(body, base);
        }
        let mut body = base.clone();
        add_eval_language(&mut body, Some("en"));
        assert_eq!(body["messages"][0], base["messages"][0]);
        assert_eq!(
            body["messages"][1],
            serde_json::json!({"role":"assistant","content":"language English<asr_text>"})
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn catalog_pins_and_paths_are_bounded_and_well_formed() {
        let root = std::env::temp_dir().join("sonojot-llama-catalog-only");
        for model in catalog() {
            assert_eq!(find(model.id).unwrap().id, model.id);
            assert!(is_llama_asr(model.id));
            for revision in [model.revision, model.projector_revision] {
                assert_eq!(revision.len(), 40);
                assert!(revision.bytes().all(|c| c.is_ascii_hexdigit()));
            }
            assert!(model.repo.contains('/') && model.projector_repo.contains('/'));
            assert_eq!(Path::new(model.id).components().count(), 1);
            for file in [model.decoder, model.projector] {
                assert!(file.bytes > 0);
                assert_eq!(file.sha256.len(), 64);
                assert!(file.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
                assert!(Path::new(file.relative)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))));
                let dir = model_dir(&root, model.id);
                assert!(dir.starts_with(root.join("optional")));
                assert!(dir.join(file.relative).starts_with(dir));
            }
        }
        assert!(!is_llama_asr("qwen3-asr-1.7b"));
        assert!(!is_llama_asr("qwen3-asr-1.7b-fp16-cuda"));
        assert!(matches!(find(CPU_MODEL_ID).unwrap().runtime, Runtime::Cpu));
        assert!(matches!(find(MODEL_ID).unwrap().runtime, Runtime::Cuda));
        assert!(!model_ready(&root, "../unknown"));
        let model = find(MODEL_ID).unwrap();
        assert_eq!(
            package_bytes(model),
            model.decoder.bytes + model.projector.bytes
        );
    }

    #[test]
    fn runtime_platform_gates_do_not_offer_cross_platform_packages() {
        for os in ["windows", "macos", "linux"] {
            for arch in ["x86_64", "aarch64", "x86"] {
                for runtime in [Runtime::Cuda, Runtime::Cpu] {
                    assert_eq!(runtime.supports(os, arch), os == "windows" && arch == "x86_64");
                }
                assert_eq!(Runtime::Metal.supports(os, arch), os == "macos" && arch == "aarch64");
            }
        }
        for model in catalog() {
            assert_eq!(model.runtime.supported_platform(),
                model.runtime.supports(std::env::consts::OS, std::env::consts::ARCH));
        }
    }

    #[test]
    fn metal_reuses_weights_and_requires_its_own_speech_detector() {
        let root = std::env::temp_dir().join(format!("sonojot-metal-pins-{}", uuid::Uuid::new_v4()));
        for (metal, windows) in [(METAL_Q8_MODEL_ID, MODEL_ID), (METAL_Q4_MODEL_ID, CPU_MODEL_ID)] {
            let (metal, windows) = (find(metal).unwrap(), find(windows).unwrap());
            assert!(is_metal(metal.id));
            assert!(!is_metal(windows.id));
            assert_eq!(metal.repo, windows.repo);
            assert_eq!(metal.revision, windows.revision);
            assert_eq!(metal.projector_repo, windows.projector_repo);
            assert_eq!(metal.projector_revision, windows.projector_revision);
            for (a, b) in [(metal.decoder, windows.decoder), (metal.projector, windows.projector)] {
                assert_eq!((a.relative, a.bytes, a.sha256), (b.relative, b.bytes, b.sha256));
                assert!(a.relative.ends_with(".gguf"));
            }
            assert!(!metal.runtime.needs_vad());
            assert!(!metal.runtime.vad_status(&root, crate::model_verification::Check::Blocking).ready());
            assert!(windows.runtime.needs_vad());
            assert!(!windows.runtime.vad_status(&root, crate::model_verification::Check::Blocking).ready());
            assert_eq!(download_total_bytes(&root, metal.id),
                Some(package_bytes(metal) + crate::model_manager::mac_runtime_download_bytes(&root)
                    + optional_asr::mac_speech_runtime_download_bytes(&root)));
            assert_eq!(metal.runtime.executable(&root), crate::cleanup::server_path(&root));
            assert!(!metal.runtime.runtime_ready(&root));
            assert!(metal.backend.starts_with("llama.cpp b10964 Metal (Qwen3-ASR "));
            assert!(metal.backend.ends_with("; sectioning local)"));
        }
        assert!(!is_metal("whisper-small-metal"));
        assert!(!is_metal("qwen3-asr-1.7b-q8-cuda"));
        assert!(!root.exists());
    }

    #[test]
    fn unsupported_llama_install_refuses_before_progress_or_staging() {
        let root = std::env::temp_dir().join(format!("sonojot-platform-{}", uuid::Uuid::new_v4()));
        for model in catalog().iter().filter(|model| !model.runtime.supported_platform()) {
            let error = install_model(&root, model.id, &AtomicBool::new(false), &|_, _| {
                panic!("unsupported package made progress")
            }).unwrap_err();
            assert_eq!(error, model.runtime.platform_requirement());
            assert!(!model_ready(&root, model.id));
        }
        assert!(!root.exists());
    }

    fn mock(status: &str, body: String) -> (LocalApi, thread::JoinHandle<serde_json::Value>) {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let api = LocalApi::new(
            listener.local_addr().unwrap().port(),
            "test-audio-key".into(),
        )
        .unwrap();
        let status = status.to_owned();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim(), "POST /v1/chat/completions HTTP/1.1");
            let mut length = None;
            let mut authorized = false;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
                if lower.trim() == "authorization: bearer test-audio-key" {
                    authorized = true;
                }
            }
            assert!(authorized);
            let mut bytes = vec![0; length.unwrap()];
            reader.read_exact(&mut bytes).unwrap();
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        });
        (api, handle)
    }

    fn completion(finish: &str) -> String {
        serde_json::json!({"choices":[{"message":{"content":"language English<asr_text>Hello there."},"finish_reason":finish}],"usage":{"completion_tokens":8}}).to_string()
    }

    #[test]
    fn mock_llama_asr_strips_header_and_retains_completion_evidence() {
        let (api, server) = mock("200 OK", completion("stop"));
        let response = api.transcribe(&[0.0; 160]).unwrap();
        assert_eq!(response.1, Some(8));
        assert_eq!(response.2, "stop");
        let recognition = crate::sherpa::llama_test_recognition(response);
        assert_eq!(recognition.text, "Hello there.");
        assert_eq!(
            recognition.completeness,
            crate::domain::RecognitionCompleteness::Complete
        );
        assert!(recognition.warnings.is_empty());
        assert_eq!(server.join().unwrap(), request_body(&[0.0; 160]).unwrap());
    }

    #[test]
    fn mock_llama_asr_length_is_incomplete_with_visible_warning() {
        let (api, server) = mock("200 OK", completion("length"));
        let response = api.transcribe(&[0.0; 160]).unwrap();
        let recognition = crate::sherpa::llama_test_recognition(response);
        assert_eq!(recognition.text, "Hello there.");
        assert_eq!(
            recognition.completeness,
            crate::domain::RecognitionCompleteness::Incomplete
        );
        assert!(!recognition.warnings.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn mock_llama_asr_http_and_malformed_responses_are_errors() {
        for (status, body) in [
            ("500 Internal Server Error", completion("stop")),
            ("200 OK", "not json".into()),
            ("200 OK", "{}".into()),
            (
                "200 OK",
                r#"{"choices":[{"message":{},"finish_reason":"stop"}]}"#.into(),
            ),
            ("200 OK", r#"{"choices":[],"usage":null}"#.into()),
        ] {
            let (api, server) = mock(status, body);
            assert!(api.transcribe(&[0.0; 160]).is_err());
            server.join().unwrap();
        }
    }

    #[test]
    fn llama_asr_install_cancelled_before_disk_or_network_work() {
        let root =
            std::env::temp_dir().join(format!("sonojot-llama-cancel-{}", uuid::Uuid::new_v4()));
        let result = install_model(&root, MODEL_ID, &AtomicBool::new(true), &|_, _| {
            panic!("cancelled progress")
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(!root.exists());
    }

    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn llama_asr_missing_cuda_runtime_refuses_before_download_or_staging() {
        let root =
            std::env::temp_dir().join(format!("sonojot-llama-missing-{}", uuid::Uuid::new_v4()));
        let error = install_model(&root, MODEL_ID, &AtomicBool::new(false), &|_, _| {
            panic!("missing runtime progress")
        })
        .unwrap_err();
        assert!(error.contains("CUDA 13.3 runtime is missing"));
        assert!(!root.exists());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn metal_missing_shared_runtime_refuses_before_download_or_staging() {
        let root = std::env::temp_dir().join(format!("sonojot-metal-missing-{}", uuid::Uuid::new_v4()));
        for id in [METAL_Q8_MODEL_ID, METAL_Q4_MODEL_ID] {
            let error = install_model(&root, id, &AtomicBool::new(false), &|_, _| {
                panic!("missing runtime progress")
            }).unwrap_err();
            assert!(error.contains("b10964 Metal runtime is missing"));
        }
        assert!(!root.exists());
    }

    #[test]
    fn llama_asr_cuda_and_metal_require_positive_valid_offload() {
        assert_eq!(
            offloaded_layers("load_tensors: offloaded 29/29 layers to GPU"),
            Some(29)
        );
        assert_eq!(offloaded_layers("offloaded 0/29 layers to GPU"), None);
        assert_eq!(offloaded_layers("offloaded 30/29 layers to GPU"), None);
        assert_eq!(offloaded_layers("found CUDA device"), None);
        assert_eq!(offloaded_layers("ggml_metal_init: found device: Apple M1 Pro"), None);
        assert_eq!(offloaded_layers("offloaded 0/0 layers to GPU"), None);
        assert_eq!(offloaded_layers("offloaded x/29 layers to GPU"), None);
        for runtime in [Runtime::Cuda, Runtime::Metal] {
            assert_eq!(runtime.gpu_layers(), "999");
            assert!(!runtime.offload_confirmed(0));
            assert!(runtime.offload_confirmed(29));
        }
        assert_eq!(Runtime::Cpu.gpu_layers(), "0");
        assert!(Runtime::Cpu.offload_confirmed(0));
    }

    #[cfg(target_os = "macos")]
    struct TestSupervisor(Child);

    #[cfg(target_os = "macos")]
    impl Drop for TestSupervisor {
        fn drop(&mut self) { stop_server(&mut self.0); }
    }

    #[cfg(target_os = "macos")]
    fn test_supervisor() -> (TestSupervisor, u32) {
        let child = server_command(Path::new("/bin/sleep")).arg("30")
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut owned = TestSupervisor(child);
        let pid = supervisor_server_pid(&mut owned.0).unwrap();
        assert_ne!(pid, owned.0.id());
        assert_eq!(unsafe { libc::getpgid(pid as libc::pid_t) }, owned.0.id() as libc::pid_t);
        (owned, pid)
    }

    #[cfg(target_os = "macos")]
    fn assert_supervisor_stopped(child: &mut Child, server_pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let supervisor_stopped = child.try_wait().unwrap().is_some();
            let server_stopped = unsafe { libc::kill(server_pid as libc::pid_t, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if supervisor_stopped && server_stopped { return; }
            assert!(Instant::now() < deadline, "owned speech processes survived teardown");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_supervisor_pipe_eof_stops_server_without_rust_drop() {
        let (mut owned, pid) = test_supervisor();
        // Same kernel EOF as abrupt worker death: no Server destructor or IPC.
        drop(owned.0.stdin.take());
        assert_supervisor_stopped(&mut owned.0, pid);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_supervisor_stops_server_when_application_dies_mid_request() {
        // Use a harmless process as the application; the worker keeps its
        // control handle alive, as it would while blocked in transcription.
        let mut application = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let watch = MacParentWatch::new(application.id() as libc::pid_t).unwrap();
        let (mut owned, pid) = test_supervisor();
        let stop = watch.start(owned.0.stdin.take().unwrap()).unwrap();
        application.kill().unwrap();
        application.wait().unwrap();
        assert_supervisor_stopped(&mut owned.0, pid);
        drop(stop);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_supervisor_observes_owner_exit_before_watcher_thread_starts() {
        let mut application = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let watch = MacParentWatch::new(application.id() as libc::pid_t).unwrap();
        application.kill().unwrap();
        application.wait().unwrap();
        let (mut owned, pid) = test_supervisor();
        let stop = watch.start(owned.0.stdin.take().unwrap()).unwrap();
        assert_supervisor_stopped(&mut owned.0, pid);
        drop(stop);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_supervisor_exits_when_server_exits() {
        let child = server_command(Path::new("/bin/sleep")).arg("0.1")
            .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut owned = TestSupervisor(child);
        let pid = supervisor_server_pid(&mut owned.0).unwrap();
        // Keep stdin open: the server's exit must wake the supervisor itself.
        assert_supervisor_stopped(&mut owned.0, pid);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_supervisor_drop_stops_owned_process_group() {
        let (mut owned, pid) = test_supervisor();
        stop_server(&mut owned.0);
        assert_supervisor_stopped(&mut owned.0, pid);
    }
}
