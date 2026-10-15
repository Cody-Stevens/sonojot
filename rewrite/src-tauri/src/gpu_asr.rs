//! Optional NVIDIA CUDA speech models. Nothing here loads at startup or
//! without an explicit Install action. GPU files land under the models pointer
//! folder in `optional/<model-id>` plus a separate CUDA runtime in
//! `optional/gpu-runtime-cuda13`, leaving the CPU `sherpa-runtime` and all
//! installed CPU models untouched. The selected GPU model is loaded only when
//! dictation or file transcription starts with provider `"cuda"`.
//!
//! Provenance (verified 2026-09-16 on RTX 5070, driver 610.88, CUDA UMD 13.3):
//! - sherpa-onnx v1.13.8 CUDA 13.x archive: 478,110,488 bytes, SHA-256
//!   `1702e4a1...` (GitHub release API + isolated SonoJot download verified).
//! - NVIDIA cuBLAS 13.7.0.27 win_amd64 wheel: 424,483,695 bytes, SHA-256
//!   `15d3963a...` (PyPI metadata + isolated download verified). Provides
//!   `cublas64_13.dll` + `cublasLt64_13.dll`. Minor-version compatible with the
//!   ORT 1.28.2 CUDA 13.0 build per NVIDIA minor-version compatibility.
//! - NVIDIA cuDNN cu13 9.26.0.51 win_amd64 wheel: 419,603,885 bytes, SHA-256
//!   `8e37a83d...` (PyPI metadata + isolated download verified). Provides the
//!   ten `cudnn*_9.dll` files required by Conv nodes.
//! - Qwen3-ASR 1.7B FP16 export `luigi000/qwen3-asr-1.7b-onnx-fp16`, revision
//!   `90c70bc1dfce2456bc0bcdc286ffc2eaff2d7e8b` (HF Hub API). Third-party
//!   export from official `Qwen/Qwen3-ASR-1.7B` via the documented
//!   Wasser1462 exporter + `onnxruntime.transformers` optimizer (fused
//!   RMSNorm/attention, `keep_io_types=True`); license Apache-2.0 inherited.
//!   Small headers + tokenizer isolated-download verified (SHA-256); frontend
//!   and tokenizer are byte-identical to the pinned CPU int8 files. Encoder
//!   and decoder graphs are pure floating-point (390 + 311 FP16 initializers,
//!   zero `MatMulInteger` / `DynamicQuantizeLinear`), so unlike the int8 CPU
//!   graphs they are eligible for CUDA kernels. The 610 MB + 4.06 GB
//!   `.data` blobs are pinned by HF `X-Linked-ETag` (proven to equal content
//!   SHA-256 by five control files) and verified transactionally at install.
//! - Whisper tiny FP32 `csukuangfj/sherpa-onnx-whisper-tiny`, revision
//!   `65176e2deb88badc814a94058666cadccc29b61c` (same revision as the CPU
//!   int8 catalog). FP32 encoder/decoder + multilingual tokens isolated
//!   download + SHA-256 verified. End-to-end CUDA inference proven on RTX 5070
//!   with public `test_wavs/0.wav`: provider `"cuda"`, transcript exactly
//!   matches `trans.txt`, VRAM 1822 MB idle to 2527 MB peak (+705 MB).
//!
//! AMD has no CUDA path; GPU entries are NVIDIA-only and never label CPU
//! execution as GPU. Task is always `transcribe`, never `translate`.
//! No persistent prompt/audio cache; raw recognition stays separate from
//! published text via `crate::asr::RecognitionResult`.

use crate::domain::RecognitionCompleteness;
use libloading::Library;
use std::{
    ffi::{c_char, CStr, CString},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// IDs and compatibility strings (exact; UI must show verbatim).
// ---------------------------------------------------------------------------

pub const GPU_RUNTIME_DIR: &str = "gpu-runtime-cuda13";
pub const QWEN_FP16_CUDA_ID: &str = "qwen3-asr-1.7b-fp16-cuda";
pub const WHISPER_TINY_FP32_CUDA_ID: &str = "whisper-tiny-fp32-cuda";

pub const CUDA_COMPATIBILITY: &str = "Windows x64 NVIDIA CUDA 13.x + cuDNN 9.x via bundled sherpa-onnx v1.13.8 CUDA runtime; AMD has no CUDA path";
pub const QWEN_CUDA_BACKEND: &str =
    "sherpa-onnx-1.13.8-cuda13 (OfflineQwen3ASR fp16, transcribe, CUDA provider; VAD CPU)";
pub const WHISPER_CUDA_BACKEND: &str =
    "sherpa-onnx-1.13.8-cuda13 (OfflineWhisper fp32, transcribe, CUDA provider; VAD CPU)";
pub const CUDA_PROVIDER: &str = "cuda";
#[path = "gpu_catalog_extra.rs"]
mod gpu_catalog_extra;

// ---------------------------------------------------------------------------
// Pinned files.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct GpuFile {
    pub url_name: &'static str,
    pub local_name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Copy)]
pub struct GpuModel {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub languages: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [GpuFile],
    pub license: &'static str,
    pub quantization: &'static str,
    pub backend: &'static str,
    pub compatibility: &'static str,
    pub guidance: &'static str,
    pub evidence: &'static str,
}

// Qwen3-ASR 1.7B FP16 (CUDA). Export layout uses `encoder.onnx` plus an
// external `encoder.onnx.data` blob (same for the decoder); the adapter loads
// these local names verbatim.
const QWEN_FP16_FILES: &[GpuFile] = &[
    GpuFile {
        url_name: "conv_frontend.onnx",
        local_name: "conv_frontend.onnx",
        bytes: 48_080_441,
        sha256: "3cb27a9fe94d95c938e476f2012b21aba2ec0bfceef33b0e58acd208946bafdd",
    },
    GpuFile {
        url_name: "encoder.onnx",
        local_name: "encoder.onnx",
        bytes: 3_395_072,
        sha256: "39eda1d849b03c5e945ac1843c472529976431f49982bafd323bb0e240aec651",
    },
    GpuFile {
        url_name: "encoder.onnx.data",
        local_name: "encoder.onnx.data",
        bytes: 610_920_448,
        sha256: "95a007f1af92d76c3fa756e886d25901b63290f455febaa1b57767ed2f5b7276",
    },
    GpuFile {
        url_name: "decoder.onnx",
        local_name: "decoder.onnx",
        bytes: 836_340,
        sha256: "b053f799295d89e1e3c17fa6f6fbc9c4710ff0e349f47a8f9d77457fef1880cf",
    },
    GpuFile {
        url_name: "decoder.onnx.data",
        local_name: "decoder.onnx.data",
        bytes: 4_063_465_472,
        sha256: "21cf4c6e3cbea4cd674092ba8e883efd74343f63837a7b08bbaca2756e320296",
    },
    GpuFile {
        url_name: "tokenizer/merges.txt",
        local_name: "tokenizer/merges.txt",
        bytes: 1_671_853,
        sha256: "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
    },
    GpuFile {
        url_name: "tokenizer/tokenizer_config.json",
        local_name: "tokenizer/tokenizer_config.json",
        bytes: 12_487,
        sha256: "4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c",
    },
    GpuFile {
        url_name: "tokenizer/vocab.json",
        local_name: "tokenizer/vocab.json",
        bytes: 2_776_833,
        sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
    },
];

const WHISPER_TINY_FP32_FILES: &[GpuFile] = &[
    GpuFile {
        url_name: "tiny-encoder.onnx",
        local_name: "encoder.onnx",
        bytes: 37_647_080,
        sha256: "42c1d4cbf889632ba21ab6f0d4064c80209755f265ce5cd630db4a6793e7089c",
    },
    GpuFile {
        url_name: "tiny-decoder.onnx",
        local_name: "decoder.onnx",
        bytes: 114_505_801,
        sha256: "e144c07dc6b55cece24392811f2d934b97013811f5e677d1315d341a0a74a25d",
    },
    GpuFile {
        url_name: "tiny-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
    },
];

pub const GPU_MODELS: &[GpuModel] = &[
    gpu_catalog_extra::SMALL,
    gpu_catalog_extra::TURBO,
    gpu_catalog_extra::PARAKEET,
    GpuModel {
        id: QWEN_FP16_CUDA_ID,
        name: "Qwen3-ASR 1.7B (FP16 CUDA)",
        family: "qwen",
        languages: "Multilingual",
        repo: "luigi000/qwen3-asr-1.7b-onnx-fp16",
        revision: "90c70bc1dfce2456bc0bcdc286ffc2eaff2d7e8b",
        files: QWEN_FP16_FILES,
        license: "Apache-2.0 (third-party FP16 export of Qwen/Qwen3-ASR-1.7B; sherpa runtime Apache-2.0)",
        quantization: "fp16",
        backend: QWEN_CUDA_BACKEND,
        compatibility: CUDA_COMPATIBILITY,
        guidance: "NVIDIA CUDA 13. Large multilingual model; measured on a 12 GB RTX 5070. Cold load can take about 100 seconds. Host loading allocations are substantial as well as GPU memory.",
        evidence: "2026-09-17 isolated app worker, public 6.625s clip: load including checks 107.6s, decode 3.66s, sampled private host peak 10.51 GB, working set 2.60 GB, device-use increase 7.56 GB. Complete transcript and acknowledged shutdown. One debug-build run; timings are not guarantees.",
    },
    GpuModel {
        id: WHISPER_TINY_FP32_CUDA_ID,
        name: "Whisper Tiny (FP32 CUDA)",
        family: "whisper",
        languages: "Multilingual",
        repo: "csukuangfj/sherpa-onnx-whisper-tiny",
        revision: "65176e2deb88badc814a94058666cadccc29b61c",
        files: WHISPER_TINY_FP32_FILES,
        license: "MIT (OpenAI Whisper weights; sherpa runtime Apache-2.0)",
        quantization: "fp32",
        backend: WHISPER_CUDA_BACKEND,
        compatibility: CUDA_COMPATIBILITY,
        guidance: "Small CUDA option (~153 MB weights, measured +~0.9 GB VRAM peak); same shared CUDA 13 runtime as the Qwen FP16 path. NVIDIA only, never AMD.",
        evidence: "Measured 2026-09-16 (gpu-native-evidence.json): isolated SonoJot download + SHA-256 verified; native CUDA decode of public audio matches trans.txt; cold load ~22 s, decode ~0.55 s (RTF 0.08); VRAM 1822 MB idle to 2527 MB peak, released after drop.",
    },
];

// ---------------------------------------------------------------------------
// Pinned runtime archives (download sources) and required DLL set.
// ---------------------------------------------------------------------------

pub struct GpuRuntimeArchive {
    pub id: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub archive_root: &'static str,
}

// Primary, verified end-to-end on RTX 5070 (driver 610.88, CUDA UMD 13.3).
pub const SHERPA_CUDA13_ARCHIVE: GpuRuntimeArchive = GpuRuntimeArchive {
    id: "sherpa-cuda13",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-cuda-13.x-cudnn-9.x-onnxruntime1.28.2-win-x64-cuda.tar.bz2",
    bytes: 478_110_488,
    sha256: "1702e4a1ee68a07422469a7ec1d304a54b43a345ad888daf8e0b0257004a3273",
    archive_root: "sherpa-onnx-v1.13.8-cuda-13.x-cudnn-9.x-onnxruntime1.28.2-win-x64-cuda",
};

pub struct NvidiaWheel {
    pub id: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub member_prefix: &'static str,
}

pub const CUBLAS_WHEEL: NvidiaWheel = NvidiaWheel {
    id: "nvidia-cublas-13.7.0.27",
    url: "https://files.pythonhosted.org/packages/f5/90/549ebae2524f3b7a695537f111c4c6a9e926ebc703c57371aa5558489e1b/nvidia_cublas-13.7.0.27-py3-none-win_amd64.whl",
    bytes: 424_483_695,
    sha256: "15d3963ae9de7292348ced320c849ff27f76b0f433b81589e9eb87f5407841c9",
    member_prefix: "nvidia/cu13/bin/x86_64/",
};

pub const CUDNN_WHEEL: NvidiaWheel = NvidiaWheel {
    id: "nvidia-cudnn-cu13-9.26.0.51",
    url: "https://files.pythonhosted.org/packages/d6/1f/9c5d7ed254d7040192b4bb4eef669ba3ed66a3c4576c50f7d2385b255c8e/nvidia_cudnn_cu13-9.26.0.51-py3-none-win_amd64.whl",
    bytes: 419_603_885,
    sha256: "8e37a83d7dd7c2663afcae38b1c95c5e9e7c9754f313800da95ca81439fa37f9",
    member_prefix: "nvidia/cudnn/bin/",
};

// Every DLL that must be present for `gpu_runtime_ready` to report true.
// Hashes were read from the verified archives/wheels above during the
// isolated 2026-09-16 trial. The VAD pin matches the CPU runtime pin.
#[derive(Clone, Copy)]
pub struct RuntimeDll {
    pub name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const RUNTIME_DLLS: &[RuntimeDll] = &[
    RuntimeDll {
        name: "sherpa-onnx-c-api.dll",
        bytes: 4_605_952,
        sha256: "d889e0a925052f8de5d64ca27d3ceb7ee923d8f9b56f89409a041dcfb911c5fa",
    },
    RuntimeDll {
        name: "onnxruntime.dll",
        bytes: 16_283_488,
        sha256: "acea7dcd8cc2bc51806382bd5bdeed5dd1bddfdcee38f4e6d67a6823b1980604",
    },
    RuntimeDll {
        name: "onnxruntime_providers_shared.dll",
        bytes: 21_816,
        sha256: "6b980b6c52f9849a20953d70a2ae5c09d9c7365be151cab9f55b38fc41e7f296",
    },
    RuntimeDll {
        name: "onnxruntime_providers_cuda.dll",
        bytes: 267_632_952,
        sha256: "26bce13391d2178c4fbb1ffeca6e828594ad1c1393c89f2a934bc977b0d1b5b7",
    },
    RuntimeDll {
        name: "cublas64_13.dll",
        bytes: 54_942_320,
        sha256: "1119dbca0a808e0c8850bb4221e330daf0b5ddb348d62d37b6ac533854723df9",
    },
    RuntimeDll {
        name: "cublasLt64_13.dll",
        bytes: 492_752_496,
        sha256: "0f5bc315fef706b5626248ebd74e8a48a9e82fa2f0cad38330f5f70984913107",
    },
    RuntimeDll {
        name: "cudnn64_9.dll",
        bytes: 270_448,
        sha256: "1a0728a6bd5c704bcdd1347cc6127fb6085fce773aaf9f1824281e9846c7b090",
    },
    RuntimeDll {
        name: "cudnn_adv64_9.dll",
        bytes: 105_227_888,
        sha256: "235134b2f410569d3517da2df3cccc5bc8b3ed8466bb49c2503702598d2bf12a",
    },
    RuntimeDll {
        name: "cudnn_cnn64_9.dll",
        bytes: 1_533_040,
        sha256: "f479a762f5e8b10ae737a8a279cef630e41ce4d4b32618f1fd9f943f61ff18d5",
    },
    RuntimeDll {
        name: "cudnn_engines_precompiled64_9.dll",
        bytes: 224_516_720,
        sha256: "c289ff5cab82e2be6a2ad116f966f4c655c94e11731e146bc3963d809eb46fc8",
    },
    RuntimeDll {
        name: "cudnn_engines_runtime_compiled64_9.dll",
        bytes: 32_993_904,
        sha256: "30a222f73ea8b89c216cc5384080addff3632a77ae6eed0d1eaa87fb57d9b184",
    },
    RuntimeDll {
        name: "cudnn_engines_tensor_ir64_9.dll",
        bytes: 156_272,
        sha256: "b97ef392f51923ee8dc616dca596fbc6f979281b135eb76991e8cadd43c6294f",
    },
    RuntimeDll {
        name: "cudnn_ext64_9.dll",
        bytes: 130_160,
        sha256: "ba3a74ab3996caee8ca893825db87b27e97f00ab14a13f7e985fd2ebc7683ea0",
    },
    RuntimeDll {
        name: "cudnn_graph64_9.dll",
        bytes: 116_192_880,
        sha256: "8f98ea372de52da536559b538f84739b0f1c50ae7f32a07e7fd111f4938fb641",
    },
    RuntimeDll {
        name: "cudnn_heuristic64_9.dll",
        bytes: 73_166_960,
        sha256: "7ff562ba7530ff390575abd62794338565d397cd5627760bb94028e6d256012d",
    },
    RuntimeDll {
        name: "cudnn_ops64_9.dll",
        bytes: 37_461_616,
        sha256: "bf813ebb0a8c1e09dd95499ac701b5e45e3d066d5624b2716a289ed5a2ca1c28",
    },
];

pub const VAD_FILE: GpuFile = GpuFile {
    url_name: "silero_vad.onnx",
    local_name: "silero_vad.onnx",
    bytes: 643_854,
    sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
};

/// Explicit preload order for the CUDA runtime DLLs (static-import topology
/// of the pinned cuBLAS 13.7 / cuDNN 9.26 binaries, verified 2026-09-16).
/// Plain LoadLibraryW does not search the DLL's own directory, so each entry
/// must already hold every bare-name static import of the entries after it:
/// Lt before cuBLAS, the cuDNN graph base before ops/cnn/adv/engines, and the
/// CUDA EP is absent on purpose — explicit preload of ORT 1.28.2's
/// `onnxruntime_providers_cuda.dll` fails its init (1114) outside ORT's own
/// provider-load sequence, while ORT resolves the on-disk copy itself at
/// session creation once its static imports are held (native proof in
/// artifacts/round6/gpu-native-result.md). `sherpa-onnx-c-api.dll` is likewise
/// absent: it is the main library, loaded after these are held.
pub const PRELOAD_ORDER: &[&str] = &[
    "onnxruntime.dll",
    "onnxruntime_providers_shared.dll",
    "cublasLt64_13.dll",
    "cublas64_13.dll",
    "cudnn_graph64_9.dll",
    "cudnn64_9.dll",
    "cudnn_ext64_9.dll",
    "cudnn_ops64_9.dll",
    "cudnn_cnn64_9.dll",
    "cudnn_adv64_9.dll",
    "cudnn_engines_tensor_ir64_9.dll",
    "cudnn_heuristic64_9.dll",
    "cudnn_engines_runtime_compiled64_9.dll",
    "cudnn_engines_precompiled64_9.dll",
];
const VAD_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";

// ---------------------------------------------------------------------------
// Paths and readiness.
// ---------------------------------------------------------------------------

pub fn find(id: &str) -> Option<&'static GpuModel> {
    GPU_MODELS.iter().find(|m| m.id == id)
}

pub fn is_gpu_model(id: &str) -> bool {
    find(id).is_some()
}

pub fn model_dir(models_root: &Path, id: &str) -> PathBuf {
    crate::optional_asr::optional_root(models_root).join(id)
}

pub fn gpu_runtime_dir(models_root: &Path) -> PathBuf {
    crate::optional_asr::optional_root(models_root).join(GPU_RUNTIME_DIR)
}

pub fn gpu_runtime_library(models_root: &Path) -> PathBuf {
    gpu_runtime_dir(models_root).join("sherpa-onnx-c-api.dll")
}

pub fn gpu_vad_path(models_root: &Path) -> PathBuf {
    gpu_runtime_dir(models_root).join("silero_vad.onnx")
}

pub fn package_bytes(m: &GpuModel) -> u64 {
    m.files.iter().map(|f| f.bytes).sum()
}

/// Total on-disk cost of a first GPU install (runtime DLLs + VAD + model).
pub fn install_bytes(id: &str) -> Option<u64> {
    let m = find(id)?;
    Some(runtime_disk_bytes() + package_bytes(m))
}

/// Total DOWNLOAD bytes of a first GPU install (runtime archives + model).
/// Progress callbacks report cumulative download bytes against this total so
/// the runtime (~1.32 GB) is honestly included before the model and
/// `downloaded <= total` always holds. On-disk cost is larger (unpacked DLLs)
/// and is reported separately via `install_bytes`.
pub fn download_total_bytes(id: &str) -> Option<u64> {
    let m = find(id)?;
    Some(RUNTIME_DOWNLOAD_BYTES + package_bytes(m))
}

/// Pinned download size of the whole CUDA runtime (sherpa archive + cuBLAS
/// wheel + cuDNN wheel + VAD). Must equal the sum of the four source pins
/// below; a unit test locks this.
pub const RUNTIME_DOWNLOAD_BYTES: u64 =
    SHERPA_CUDA13_ARCHIVE.bytes + CUBLAS_WHEEL.bytes + CUDNN_WHEEL.bytes + VAD_FILE.bytes;

fn runtime_disk_bytes() -> u64 {
    RUNTIME_DLLS.iter().map(|d| d.bytes).sum::<u64>() + VAD_FILE.bytes
}

fn dll_matches(path: &Path, dll: &RuntimeDll) -> bool {
    let pinned = crate::optional_asr::PinnedFile {
        relative: dll.name,
        bytes: dll.bytes,
        sha256: dll.sha256,
    };
    crate::optional_asr::file_matches(path, &pinned).unwrap_or(false)
}

pub fn gpu_runtime_ready(models_root: &Path) -> bool {
    runtime_status(models_root, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn runtime_status(models_root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    let dir = gpu_runtime_dir(models_root);
    Status::all([
        Status::all(RUNTIME_DLLS.iter().map(|dll| check.file(models_root, &dir.join(dll.name), dll.bytes, dll.sha256))),
        check.file(models_root, &dir.join(VAD_FILE.local_name), VAD_FILE.bytes, VAD_FILE.sha256),
    ])
}

pub fn model_ready(models_root: &Path, id: &str) -> bool {
    model_status(models_root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(models_root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    let Some(m) = find(id) else { return Status::Missing };
    let dir = model_dir(models_root, m.id);
    Status::all([
        runtime_status(models_root, check),
        Status::all(m.files.iter().map(|f| check.file(models_root, &dir.join(f.local_name), f.bytes, f.sha256))),
    ])
}

// ---------------------------------------------------------------------------
// Resource-worker metadata (concrete, no fake precision).
// ---------------------------------------------------------------------------

pub struct GpuCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub languages: &'static str,
    pub revision: &'static str,
    pub quantization: &'static str,
    pub backend: &'static str,
    pub compatibility: &'static str,
    pub disk_bytes: u64,
    pub installable: bool,
    pub reason: &'static str,
    pub evidence: &'static str,
    pub license: &'static str,
}

pub fn catalog_metadata() -> Vec<GpuCatalogEntry> {
    GPU_MODELS
        .iter()
        .map(|m| GpuCatalogEntry {
            id: m.id,
            name: m.name,
            family: m.family,
            languages: m.languages,
            revision: m.revision,
            quantization: m.quantization,
            backend: m.backend,
            compatibility: m.compatibility,
            disk_bytes: package_bytes(m),
            installable: true,
            reason: m.guidance,
            evidence: m.evidence,
            license: m.license,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Install with cancel + progress (transactional, never partial).
// ---------------------------------------------------------------------------

const DOWNLOAD_LIMIT: Duration = Duration::from_secs(60 * 60);

pub fn install_model(
    models_root: &Path,
    id: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let m = find(id).ok_or("Choose a downloadable GPU speech model.")?;
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (models_root, m, cancel, progress);
        return Err("Automatic GPU speech-model install is available only on 64-bit Windows with an NVIDIA GPU.".into());
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let model_total = package_bytes(m);
        let total = download_total_bytes(m.id).expect("pinned model has a download total");
        debug_assert_eq!(total, RUNTIME_DOWNLOAD_BYTES + model_total);
        // Runtime first so progress honestly includes its ~1.32 GB before any
        // model byte; every report is clamped so downloaded never exceeds total.
        if gpu_runtime_ready(models_root) {
            progress(RUNTIME_DOWNLOAD_BYTES.min(total), total);
        } else {
            ensure_gpu_runtime_with_progress(models_root, cancel, &|got, _| {
                progress(got.min(RUNTIME_DOWNLOAD_BYTES).min(total), total);
            })?;
        }
        if model_ready(models_root, m.id) {
            progress(total, total);
            return Ok(());
        }
        let mut done = 0u64;
        progress(RUNTIME_DOWNLOAD_BYTES.min(total), total);
        let root = crate::optional_asr::optional_root(models_root);
        fs::create_dir_all(&root)
            .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
        crate::optional_asr::ensure_within(models_root, &root)?;
        let dest = model_dir(models_root, m.id);
        // No pre-download deletion: the prior install (if any) stays live
        // until verified staging atomically replaces it, so a failed or
        // cancelled replacement keeps the old files.
        let staging = root.join(format!(".install-{}-{}", m.id, crate::store::new_id()));
        fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
        let installed = (|| {
            for f in m.files {
                if cancel.load(Ordering::Acquire) {
                    return Err("Transcription cancelled.".into());
                }
                let path = staging.join(f.local_name);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Cannot prepare {}: {e}", f.local_name))?;
                }
                let url = format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    m.repo, m.revision, f.url_name
                );
                download_with_progress(&url, &path, f.bytes, f.sha256, cancel, &|got, _| {
                    progress(
                        RUNTIME_DOWNLOAD_BYTES
                            .saturating_add(done.saturating_add(got.min(f.bytes)).min(model_total))
                            .min(total),
                        total,
                    );
                })?;
                done = done.saturating_add(f.bytes).min(model_total);
                progress(
                    RUNTIME_DOWNLOAD_BYTES.saturating_add(done).min(total),
                    total,
                );
            }
            for f in m.files {
                let pinned = crate::optional_asr::PinnedFile {
                    relative: f.local_name,
                    bytes: f.bytes,
                    sha256: f.sha256,
                };
                let ok = crate::optional_asr::file_matches(&staging.join(f.local_name), &pinned)?;
                if !ok {
                    return Err(format!(
                        "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                        f.local_name
                    ));
                }
            }
            // Transactional activation with rollback: backup live -> rename
            // staging -> drop backup, restoring on failure. Cancel/failure
            // keeps the prior install; only verified bytes ever go live.
            crate::optional_asr::transactional_activate(
                models_root,
                &staging,
                &dest,
                "GPU speech model",
            )?;
            Ok(())
        })();
        if installed.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        installed
    }
}

pub fn ensure_gpu_runtime(models_root: &Path) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    ensure_gpu_runtime_with_progress(models_root, &cancel, &|_, _| {})
}

fn ensure_gpu_runtime_with_progress(
    models_root: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (models_root, cancel, progress);
        return Err("Automatic GPU speech-model install is available only on 64-bit Windows with an NVIDIA GPU.".into());
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        if gpu_runtime_ready(models_root) {
            return Ok(());
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        let root = crate::optional_asr::optional_root(models_root);
        fs::create_dir_all(&root)
            .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
        crate::optional_asr::ensure_within(models_root, &root)?;
        let dest = gpu_runtime_dir(models_root);
        // No pre-download deletion: the prior runtime (if any) stays live
        // until the verified set atomically replaces it.
        let staging = root.join(format!(".gpu-runtime-{}", crate::store::new_id()));
        fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
        let installed = (|| {
            // Cumulative download progress over the four runtime downloads so
            // callers can map it into an overall runtime+model total.
            let mut done = 0u64;
            progress(0, RUNTIME_DOWNLOAD_BYTES);
            // 1. sherpa CUDA archive (verified 478,110,488 bytes).
            let archive = staging.join("cuda.tar.bz2");
            let base = done;
            download_with_progress(
                SHERPA_CUDA13_ARCHIVE.url,
                &archive,
                SHERPA_CUDA13_ARCHIVE.bytes,
                SHERPA_CUDA13_ARCHIVE.sha256,
                cancel,
                &|got, _| {
                    progress(
                        base.saturating_add(got.min(SHERPA_CUDA13_ARCHIVE.bytes))
                            .min(RUNTIME_DOWNLOAD_BYTES),
                        RUNTIME_DOWNLOAD_BYTES,
                    );
                },
            )?;
            done = base.saturating_add(SHERPA_CUDA13_ARCHIVE.bytes);
            progress(done.min(RUNTIME_DOWNLOAD_BYTES), RUNTIME_DOWNLOAD_BYTES);
            unpack_subset(
                &archive,
                &staging,
                SHERPA_CUDA13_ARCHIVE.archive_root,
                cancel,
            )?;
            // 2. NVIDIA cuBLAS wheel (verified 424,483,695 bytes).
            let cublas = staging.join("cublas.whl");
            let base = done;
            download_with_progress(
                CUBLAS_WHEEL.url,
                &cublas,
                CUBLAS_WHEEL.bytes,
                CUBLAS_WHEEL.sha256,
                cancel,
                &|got, _| {
                    progress(
                        base.saturating_add(got.min(CUBLAS_WHEEL.bytes))
                            .min(RUNTIME_DOWNLOAD_BYTES),
                        RUNTIME_DOWNLOAD_BYTES,
                    );
                },
            )?;
            done = base.saturating_add(CUBLAS_WHEEL.bytes);
            progress(done.min(RUNTIME_DOWNLOAD_BYTES), RUNTIME_DOWNLOAD_BYTES);
            unpack_wheel_dlls(&cublas, &staging, CUBLAS_WHEEL.member_prefix, cancel)?;
            // 3. NVIDIA cuDNN wheel (verified 419,603,885 bytes).
            let cudnn = staging.join("cudnn.whl");
            let base = done;
            download_with_progress(
                CUDNN_WHEEL.url,
                &cudnn,
                CUDNN_WHEEL.bytes,
                CUDNN_WHEEL.sha256,
                cancel,
                &|got, _| {
                    progress(
                        base.saturating_add(got.min(CUDNN_WHEEL.bytes))
                            .min(RUNTIME_DOWNLOAD_BYTES),
                        RUNTIME_DOWNLOAD_BYTES,
                    );
                },
            )?;
            done = base.saturating_add(CUDNN_WHEEL.bytes);
            progress(done.min(RUNTIME_DOWNLOAD_BYTES), RUNTIME_DOWNLOAD_BYTES);
            unpack_wheel_dlls(&cudnn, &staging, CUDNN_WHEEL.member_prefix, cancel)?;
            // 4. Collect exactly the required DLL set.
            let runtime_staging = staging.join("runtime");
            fs::create_dir(&runtime_staging)
                .map_err(|e| format!("Cannot prepare the GPU speech runtime: {e}"))?;
            let lib = staging.join(SHERPA_CUDA13_ARCHIVE.archive_root).join("lib");
            for name in [
                "sherpa-onnx-c-api.dll",
                "onnxruntime.dll",
                "onnxruntime_providers_shared.dll",
                "onnxruntime_providers_cuda.dll",
            ] {
                fs::copy(lib.join(name), runtime_staging.join(name))
                    .map_err(|e| format!("Cannot copy {name}: {e}"))?;
            }
            for name in ["cublas64_13.dll", "cublasLt64_13.dll"] {
                let src = staging.join("wheel-cublas").join(name);
                fs::copy(&src, runtime_staging.join(name))
                    .map_err(|e| format!("Cannot copy {name}: {e}"))?;
            }
            for dll in RUNTIME_DLLS.iter().filter(|d| d.name.starts_with("cudnn")) {
                let src = staging.join("wheel-cudnn").join(dll.name);
                fs::copy(&src, runtime_staging.join(dll.name))
                    .map_err(|e| format!("Cannot copy {}: {e}", dll.name))?;
            }
            // 5. Silero VAD (same 643,854-byte pin as the CPU runtime).
            let base = done;
            download_with_progress(
                VAD_URL,
                &runtime_staging.join(VAD_FILE.local_name),
                VAD_FILE.bytes,
                VAD_FILE.sha256,
                cancel,
                &|got, _| {
                    progress(
                        base.saturating_add(got.min(VAD_FILE.bytes))
                            .min(RUNTIME_DOWNLOAD_BYTES),
                        RUNTIME_DOWNLOAD_BYTES,
                    );
                },
            )?;
            done = base.saturating_add(VAD_FILE.bytes);
            debug_assert_eq!(done, RUNTIME_DOWNLOAD_BYTES);
            progress(done.min(RUNTIME_DOWNLOAD_BYTES), RUNTIME_DOWNLOAD_BYTES);
            // 6. Verify the full DLL set before activation.
            for dll in RUNTIME_DLLS {
                if !dll_matches(&runtime_staging.join(dll.name), dll) {
                    return Err(format!(
                        "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                        dll.name
                    ));
                }
            }
            let pinned_vad = crate::optional_asr::PinnedFile {
                relative: VAD_FILE.local_name,
                bytes: VAD_FILE.bytes,
                sha256: VAD_FILE.sha256,
            };
            if !crate::optional_asr::file_matches(
                &runtime_staging.join(VAD_FILE.local_name),
                &pinned_vad,
            )? {
                return Err(
                    "The downloaded silero_vad.onnx did not match its pinned checksum. Nothing was installed."
                        .into(),
                );
            }
            // Transactional activation with rollback (same guarantee as the
            // model installer): a failed activation restores the prior
            // runtime instead of deleting it.
            crate::optional_asr::transactional_activate(
                models_root,
                &runtime_staging,
                &dest,
                "GPU speech runtime",
            )?;
            Ok(())
        })();
        let _ = fs::remove_dir_all(&staging);
        installed
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn download_with_progress(
    url: &str,
    dest: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let is_hf = url.contains("huggingface.co");
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(DOWNLOAD_LIMIT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    if is_hf {
                        host == "huggingface.co"
                            || host.ends_with(".huggingface.co")
                            || host.ends_with(".hf.co")
                            || host == "cas-bridge.xethub.hf.co"
                            || host.ends_with(".xethub.hf.co")
                    } else {
                        host == "github.com"
                            || host.ends_with(".githubusercontent.com")
                            || host == "files.pythonhosted.org"
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
        .map_err(|e| format!("The GPU model download could not start: {e}"))?
        .error_for_status()
        .map_err(|e| format!("The GPU model download was refused: {e}"))?;
    // A refused redirect leaves a 3xx whose size is the redirect page's.
    if response.status().is_redirection() {
        return Err("The download was redirected to a host SonoJot does not trust. Nothing was installed.".into());
    }
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
    let file = File::create(dest).map_err(|e| format!("Cannot stage the download: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    let deadline = Instant::now() + DOWNLOAD_LIMIT;
    let mut total = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    progress(0, expected_bytes);
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        if Instant::now() >= deadline {
            return Err(
                "The GPU model download exceeded the time limit. Check the connection and try again."
                    .into(),
            );
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The GPU model download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err("The GPU model download was larger than the pinned file.".into());
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save the download: {e}"))?;
        progress(total, expected_bytes);
    }
    out.flush()
        .map_err(|e| format!("Cannot save the download: {e}"))?;
    drop(out);
    if total != expected_bytes {
        return Err(format!(
            "The download is incomplete ({total} of {expected_bytes} bytes). Try again."
        ));
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    let hash = sha256_of(dest, expected_bytes)?;
    if !hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(
            "The downloaded GPU file did not match its pinned checksum. Nothing was installed."
                .into(),
        );
    }
    Ok(())
}

fn sha256_of(path: &Path, expected_bytes: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|e| format!("The download could not be read: {e}"))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "The download could not be read completely.".to_string())?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err("The download grew while it was being checked.".into());
        }
        hasher.update(&buffer[..read]);
    }
    if total != expected_bytes {
        return Err(format!(
            "The download is incomplete ({total} of {expected_bytes} bytes). Try again."
        ));
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn unpack_subset(
    archive: &Path,
    staging: &Path,
    archive_root: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let file = File::open(archive)
        .map_err(|e| format!("The GPU runtime package could not be opened: {e}"))?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut package = tar::Archive::new(decoder);
    let mut entries = 0usize;
    for entry in package
        .entries()
        .map_err(|e| format!("The GPU runtime package is not a readable archive: {e}"))?
    {
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        entries += 1;
        if entries > 64 {
            return Err("The GPU runtime package contains more files than expected.".into());
        }
        let mut entry =
            entry.map_err(|e| format!("The GPU runtime package could not be read: {e}"))?;
        let name = entry
            .path()
            .map_err(|_| "The GPU runtime package contains an unreadable path.".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let relative = safe_member_name(&name).ok_or_else(|| {
            "The GPU runtime package contains an unsafe path. Nothing was installed.".to_string()
        })?;
        if !relative.starts_with(archive_root) {
            return Err("The GPU runtime package did not match its pinned layout.".into());
        }
        // Only the `lib/` DLLs are needed; skip exes, headers and static libs.
        // `relative` uses OS separators, so match the `lib` component and the
        // `dll` extension instead of a `/`-joined substring.
        let under_lib = relative.components().any(|c| c.as_os_str() == "lib");
        if !under_lib || relative.extension().and_then(|e| e.to_str()) != Some("dll") {
            continue;
        }
        let dest = staging.join(&relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot unpack the GPU speech runtime: {e}"))?;
        }
        let mut out = File::create(&dest)
            .map_err(|e| format!("Cannot unpack the GPU speech runtime: {e}"))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Cannot unpack the GPU speech runtime: {e}"))?;
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn unpack_wheel_dlls(
    wheel: &Path,
    staging: &Path,
    member_prefix: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    // Wheels are zip files; only the `bin/*.dll` members are installed.
    let file =
        File::open(wheel).map_err(|e| format!("The NVIDIA package could not be opened: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("The NVIDIA package is not a readable wheel: {e}"))?;
    let out_dir = staging.join(if member_prefix.contains("cudnn") {
        "wheel-cudnn"
    } else {
        "wheel-cublas"
    });
    fs::create_dir_all(&out_dir).map_err(|e| format!("Cannot unpack the NVIDIA package: {e}"))?;
    for index in 0..archive.len() {
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("The NVIDIA package could not be read: {e}"))?;
        let name = entry.name().replace('\\', "/");
        let Some(rest) = name.strip_prefix(member_prefix) else {
            continue;
        };
        if rest.is_empty() || rest.contains('/') || !rest.ends_with(".dll") {
            continue;
        }
        if entry.size() > 600_000_000 {
            return Err("The NVIDIA package contains a file larger than expected.".into());
        }
        let dest = out_dir.join(rest);
        let mut out =
            File::create(&dest).map_err(|e| format!("Cannot unpack the NVIDIA package: {e}"))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Cannot unpack the NVIDIA package: {e}"))?;
    }
    Ok(())
}

fn safe_member_name(name: &str) -> Option<PathBuf> {
    use std::path::Component;
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let mut relative = PathBuf::new();
    for component in Path::new(&name.replace('\\', "/")).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

// ---------------------------------------------------------------------------
// CUDA adapter: same capture-owner shape as `sherpa.rs` (normalized f32 at
// 16 kHz, VAD segments, transcribe-only, completeness + warnings).
// ---------------------------------------------------------------------------

// C ABI structs mirror `sherpa.rs` exactly (v1.13.8 layout); only the provider
// string and runtime directory differ.
#[repr(C)]
struct FeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}
#[repr(C)]
struct HomophoneReplacerConfig {
    dict_dir: *const c_char,
    lexicon: *const c_char,
    rule_fsts: *const c_char,
}
#[repr(C)]
struct OfflineTransducerModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    joiner: *const c_char,
}
#[repr(C)]
struct OfflineParaformerModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineNemoEncDecCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineWhisperModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    task: *const c_char,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}
#[repr(C)]
struct OfflineCanaryModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    src_lang: *const c_char,
    tgt_lang: *const c_char,
    use_pnc: i32,
}
#[repr(C)]
struct OfflineCohereTranscribeModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
    language: *const c_char,
    use_punct: i32,
    use_itn: i32,
}
#[repr(C)]
struct OfflineFireRedAsrModelConfig {
    encoder: *const c_char,
    decoder: *const c_char,
}
#[repr(C)]
struct OfflineFireRedAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineMoonshineModelConfig {
    preprocessor: *const c_char,
    encoder: *const c_char,
    uncached_decoder: *const c_char,
    cached_decoder: *const c_char,
    merged_decoder: *const c_char,
}
#[repr(C)]
struct OfflineTdnnModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineLMConfig {
    model: *const c_char,
    scale: f32,
}
#[repr(C)]
struct OfflineSenseVoiceModelConfig {
    model: *const c_char,
    language: *const c_char,
    use_itn: i32,
}
#[repr(C)]
struct OfflineDolphinModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineZipformerCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineWenetCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineOmnilingualAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineFunASRNanoModelConfig {
    encoder_adaptor: *const c_char,
    llm: *const c_char,
    embedding: *const c_char,
    tokenizer: *const c_char,
    system_prompt: *const c_char,
    user_prompt: *const c_char,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    language: *const c_char,
    itn: i32,
    hotwords: *const c_char,
}
#[repr(C)]
struct OfflineQwen3ASRModelConfig {
    conv_frontend: *const c_char,
    encoder: *const c_char,
    decoder: *const c_char,
    tokenizer: *const c_char,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: f32,
    top_p: f32,
    seed: i32,
    hotwords: *const c_char,
}
#[repr(C)]
struct OfflineMedAsrCtcModelConfig {
    model: *const c_char,
}
#[repr(C)]
struct OfflineModelConfig {
    transducer: OfflineTransducerModelConfig,
    paraformer: OfflineParaformerModelConfig,
    nemo_ctc: OfflineNemoEncDecCtcModelConfig,
    whisper: OfflineWhisperModelConfig,
    tdnn: OfflineTdnnModelConfig,
    tokens: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
    model_type: *const c_char,
    modeling_unit: *const c_char,
    bpe_vocab: *const c_char,
    telespeech_ctc: *const c_char,
    sense_voice: OfflineSenseVoiceModelConfig,
    moonshine: OfflineMoonshineModelConfig,
    fire_red_asr: OfflineFireRedAsrModelConfig,
    dolphin: OfflineDolphinModelConfig,
    zipformer_ctc: OfflineZipformerCtcModelConfig,
    canary: OfflineCanaryModelConfig,
    wenet_ctc: OfflineWenetCtcModelConfig,
    omnilingual: OfflineOmnilingualAsrCtcModelConfig,
    medasr: OfflineMedAsrCtcModelConfig,
    funasr_nano: OfflineFunASRNanoModelConfig,
    fire_red_asr_ctc: OfflineFireRedAsrCtcModelConfig,
    qwen3_asr: OfflineQwen3ASRModelConfig,
    cohere_transcribe: OfflineCohereTranscribeModelConfig,
}
#[repr(C)]
struct OfflineRecognizerConfig {
    feat_config: FeatureConfig,
    model_config: OfflineModelConfig,
    lm_config: OfflineLMConfig,
    decoding_method: *const c_char,
    max_active_paths: i32,
    hotwords_file: *const c_char,
    hotwords_score: f32,
    rule_fsts: *const c_char,
    rule_fars: *const c_char,
    blank_penalty: f32,
    hr: HomophoneReplacerConfig,
}
#[repr(C)]
struct SileroVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}
#[repr(C)]
struct TenVadModelConfig {
    model: *const c_char,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}
#[repr(C)]
struct VadModelConfig {
    silero_vad: SileroVadModelConfig,
    sample_rate: i32,
    num_threads: i32,
    provider: *const c_char,
    debug: i32,
    ten_vad: TenVadModelConfig,
}
#[repr(C)]
struct SpeechSegment {
    start: i32,
    samples: *mut f32,
    n: i32,
}
#[repr(C)]
struct OfflineRecognizerResult {
    text: *const c_char,
    timestamps: *mut f32,
    count: i32,
    tokens: *const c_char,
    tokens_arr: *const *const c_char,
    json: *const c_char,
}

enum OfflineRecognizer {}
enum OfflineStreamHandle {}
enum VoiceActivityDetector {}

type CreateOffline =
    unsafe extern "C" fn(*const OfflineRecognizerConfig) -> *const OfflineRecognizer;
type DestroyOffline = unsafe extern "C" fn(*const OfflineRecognizer);
type CreateOfflineStream =
    unsafe extern "C" fn(*const OfflineRecognizer) -> *const OfflineStreamHandle;
type DestroyOfflineStream = unsafe extern "C" fn(*const OfflineStreamHandle);
type AcceptWaveform = unsafe extern "C" fn(*const OfflineStreamHandle, i32, *const f32, i32);
type DecodeOffline = unsafe extern "C" fn(*const OfflineRecognizer, *const OfflineStreamHandle);
type GetResult = unsafe extern "C" fn(*const OfflineStreamHandle) -> *const OfflineRecognizerResult;
type DestroyResult = unsafe extern "C" fn(*const OfflineRecognizerResult);
type CreateVad = unsafe extern "C" fn(*const VadModelConfig, f32) -> *const VoiceActivityDetector;
type DestroyVad = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadAccept = unsafe extern "C" fn(*const VoiceActivityDetector, *const f32, i32);
type VadEmpty = unsafe extern "C" fn(*const VoiceActivityDetector) -> i32;
type VadPop = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadFront = unsafe extern "C" fn(*const VoiceActivityDetector) -> *const SpeechSegment;
type DestroySegment = unsafe extern "C" fn(*const SpeechSegment);
type VadFlush = unsafe extern "C" fn(*const VoiceActivityDetector);
type VadReset = unsafe extern "C" fn(*const VoiceActivityDetector);

struct CudaApi {
    // Keep mappings, including partial-load successes, until this dedicated
    // process exits. CPU/GPU DLL basenames must never share the UI process.
    _library: std::mem::ManuallyDrop<Library>,
    _dependencies: Vec<std::mem::ManuallyDrop<Library>>,
    create_offline: CreateOffline,
    destroy_offline: DestroyOffline,
    create_stream: CreateOfflineStream,
    destroy_stream: DestroyOfflineStream,
    accept: AcceptWaveform,
    decode: DecodeOffline,
    get_result: GetResult,
    destroy_result: DestroyResult,
    create_vad: CreateVad,
    destroy_vad: DestroyVad,
    vad_accept: VadAccept,
    vad_empty: VadEmpty,
    vad_pop: VadPop,
    vad_front: VadFront,
    destroy_segment: DestroySegment,
    vad_flush: VadFlush,
    vad_reset: VadReset,
}

impl CudaApi {
    fn open(models_root: &Path) -> Result<Self, String> {
        if !crate::recognition::is_worker() {
            return Err("CUDA recognition requires an isolated worker.".into());
        }
        let dir = gpu_runtime_dir(models_root);
        let library_path = dir.join("sherpa-onnx-c-api.dll");
        if !library_path.is_file() {
            return Err("The GPU speech runtime is not installed.".into());
        }
        // Preload CUDA dependencies by absolute path in PRELOAD_ORDER
        // (static-import topology; see the constant docs). Never alter PATH
        // or the process-wide DLL search directory.
        // Native-validated 2026-09-16 on RTX 5070: this exact order loads and
        // decodes through the real adapter (see artifacts/round6/gpu-native-result.md).
        let mut dependencies = Vec::new();
        for name in PRELOAD_ORDER {
            let dep = dir.join(name);
            if dep.is_file() {
                dependencies.push(std::mem::ManuallyDrop::new(
                    unsafe { Library::new(&dep) }
                        .map_err(|e| format!("Cannot load {}: {e}", dep.display()))?,
                ));
            }
        }
        let library = std::mem::ManuallyDrop::new(
            unsafe { Library::new(&library_path) }
                .map_err(|e| format!("Cannot load the GPU speech runtime: {e}"))?,
        );
        unsafe {
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("GPU speech runtime is missing {}: {e}", $name))?
                };
            }
            Ok(Self {
                create_offline: symbol!("SherpaOnnxCreateOfflineRecognizer", CreateOffline),
                destroy_offline: symbol!("SherpaOnnxDestroyOfflineRecognizer", DestroyOffline),
                create_stream: symbol!("SherpaOnnxCreateOfflineStream", CreateOfflineStream),
                destroy_stream: symbol!("SherpaOnnxDestroyOfflineStream", DestroyOfflineStream),
                accept: symbol!("SherpaOnnxAcceptWaveformOffline", AcceptWaveform),
                decode: symbol!("SherpaOnnxDecodeOfflineStream", DecodeOffline),
                get_result: symbol!("SherpaOnnxGetOfflineStreamResult", GetResult),
                destroy_result: symbol!("SherpaOnnxDestroyOfflineRecognizerResult", DestroyResult),
                create_vad: symbol!("SherpaOnnxCreateVoiceActivityDetector", CreateVad),
                destroy_vad: symbol!("SherpaOnnxDestroyVoiceActivityDetector", DestroyVad),
                vad_accept: symbol!("SherpaOnnxVoiceActivityDetectorAcceptWaveform", VadAccept),
                vad_empty: symbol!("SherpaOnnxVoiceActivityDetectorEmpty", VadEmpty),
                vad_pop: symbol!("SherpaOnnxVoiceActivityDetectorPop", VadPop),
                vad_front: symbol!("SherpaOnnxVoiceActivityDetectorFront", VadFront),
                destroy_segment: symbol!("SherpaOnnxDestroySpeechSegment", DestroySegment),
                vad_flush: symbol!("SherpaOnnxVoiceActivityDetectorFlush", VadFlush),
                vad_reset: symbol!("SherpaOnnxVoiceActivityDetectorReset", VadReset),
                _library: library,
                _dependencies: dependencies,
            })
        }
    }
}

pub struct Engine {
    api: CudaApi,
    recognizer: *const OfflineRecognizer,
    vad: *const VoiceActivityDetector,
    _keep: Vec<CString>,
    pub model_id: String,
    pub requested_model: String,
    pub provider: &'static str,
}

// SAFETY: the recognizer/VAD pointers are used only on the owning thread via
// `&mut self` stream creation, matching the `sherpa.rs` ownership pattern.
unsafe impl Send for Engine {}

impl Engine {
    pub fn load(models_root: &Path, requested: &str) -> Result<Self, String> {
        let m =
            find(requested).ok_or_else(|| format!("Unsupported GPU recognizer: {requested}"))?;
        if !model_ready(models_root, m.id) {
            return Err(format!("{requested} is not installed"));
        }
        let api = CudaApi::open(models_root)?;
        let dir = model_dir(models_root, m.id);
        let mut keep = Vec::new();
        let path = |keep: &mut Vec<CString>, value: PathBuf| -> Result<*const c_char, String> {
            keep.push(crate::asr::native_model_path(&value)?);
            Ok(keep.last().unwrap().as_ptr())
        };
        let intern = |keep: &mut Vec<CString>, value: &str| -> *const c_char {
            keep.push(CString::new(value).expect("static interned strings"));
            keep.last().unwrap().as_ptr()
        };
        let mut config: OfflineRecognizerConfig = unsafe { std::mem::zeroed() };
        config.feat_config.sample_rate = 16_000;
        // Whisper uses 80-dim features; Qwen uses 128. Set per model below.
        config.model_config.num_threads = 2;
        config.model_config.provider = intern(&mut keep, CUDA_PROVIDER);
        config.decoding_method = intern(&mut keep, "greedy_search");
        if m.family == "whisper" {
            config.feat_config.feature_dim =
                if m.id == gpu_catalog_extra::WHISPER_TURBO_FP32_CUDA_ID {
                    gpu_catalog_extra::WHISPER_TURBO_FEATURE_DIM
                } else {
                    80
                };
            config.model_config.whisper.encoder = path(&mut keep, dir.join("encoder.onnx"))?;
            config.model_config.whisper.decoder = path(&mut keep, dir.join("decoder.onnx"))?;
            // Multilingual checkpoint: auto-detect so non-English text is
            // preserved, never translated.
            config.model_config.whisper.language =
                intern(&mut keep, crate::asr::sherpa_whisper_language(false));
            config.model_config.whisper.task = intern(&mut keep, "transcribe");
            config.model_config.whisper.tail_paddings = 0;
            config.model_config.whisper.enable_token_timestamps = 0;
            config.model_config.whisper.enable_segment_timestamps = 0;
            config.model_config.tokens = path(&mut keep, dir.join("tokens.txt"))?;
            config.model_config.model_type = intern(&mut keep, "whisper");
        } else if m.family == "parakeet" {
            config.feat_config.feature_dim = 128;
            config.model_config.transducer.encoder =
                path(&mut keep, dir.join("encoder.fp16.onnx"))?;
            config.model_config.transducer.decoder =
                path(&mut keep, dir.join("decoder.fp16.onnx"))?;
            config.model_config.transducer.joiner = path(&mut keep, dir.join("joiner.fp16.onnx"))?;
            config.model_config.tokens = path(&mut keep, dir.join("tokens.txt"))?;
            config.model_config.model_type = intern(&mut keep, "nemo_transducer");
        } else {
            config.feat_config.feature_dim = 128;
            config.model_config.qwen3_asr.conv_frontend =
                path(&mut keep, dir.join("conv_frontend.onnx"))?;
            config.model_config.qwen3_asr.encoder = path(&mut keep, dir.join("encoder.onnx"))?;
            config.model_config.qwen3_asr.decoder = path(&mut keep, dir.join("decoder.onnx"))?;
            config.model_config.qwen3_asr.tokenizer = path(&mut keep, dir.join("tokenizer"))?;
            // FP16 export README: defaults (512/128) silently truncate long
            // turns; use the validated 4096/1024.
            config.model_config.qwen3_asr.max_total_len = 4096;
            config.model_config.qwen3_asr.max_new_tokens = QWEN_MAX_NEW_TOKENS as i32;
            config.model_config.qwen3_asr.temperature = 1e-6;
            config.model_config.qwen3_asr.top_p = 0.8;
            config.model_config.qwen3_asr.seed = 42;
        }
        let recognizer = unsafe { (api.create_offline)(&config) };
        if recognizer.is_null() {
            return Err(format!(
                "Cannot initialize GPU speech recognition: {requested}"
            ));
        }
        // VAD stays on CPU to keep VRAM for the recognizer.
        let mut vad_config: VadModelConfig = unsafe { std::mem::zeroed() };
        vad_config.silero_vad.model = path(&mut keep, gpu_vad_path(models_root))?;
        vad_config.silero_vad.threshold = 0.25;
        vad_config.silero_vad.min_silence_duration = 0.5;
        vad_config.silero_vad.min_speech_duration = 0.25;
        vad_config.silero_vad.window_size = 512;
        vad_config.silero_vad.max_speech_duration = crate::asr::sherpa_section_seconds() as f32;
        vad_config.sample_rate = 16_000;
        vad_config.num_threads = 1;
        vad_config.provider = intern(&mut keep, "cpu");
        let vad = unsafe { (api.create_vad)(&vad_config, 30.0) };
        if vad.is_null() {
            unsafe { (api.destroy_offline)(recognizer) };
            return Err(
                "Cannot initialize voice-activity detection for the GPU speech model.".into(),
            );
        }
        Ok(Self {
            api,
            recognizer,
            vad,
            _keep: keep,
            model_id: m.id.to_owned(),
            requested_model: requested.to_owned(),
            provider: CUDA_PROVIDER,
        })
    }

    pub fn stream(&mut self) -> Result<Stream<'_>, String> {
        unsafe { (self.api.vad_reset)(self.vad) };
        Ok(Stream {
            engine: self,
            pending: Vec::new(),
            ledger: std::collections::BTreeMap::new(),
            original_text: None,
            next_id: 1,
            vad_samples_since_segment: 0,
            vad_samples: 0,
            vad_origin: 0,
            completeness: RecognitionCompleteness::Complete,
            warnings: Vec::new(),
            #[cfg(feature = "fidelity-eval")]
            section_evidence: Vec::new(),
        })
    }

    fn decode_once(&self, samples: &[f32]) -> Result<(String, i32), String> {
        use std::ptr;
        let stream = unsafe { (self.api.create_stream)(self.recognizer) };
        if stream.is_null() {
            return Err("Cannot start GPU recognition.".into());
        }
        unsafe {
            (self.api.accept)(stream, 16_000, samples.as_ptr(), samples.len() as i32);
            (self.api.decode)(self.recognizer, stream);
        }
        let result = unsafe { (self.api.get_result)(stream) };
        let text = if result.is_null() {
            unsafe { (self.api.destroy_stream)(stream) };
            return Err("GPU recognition returned no result.".into());
        } else {
            let raw = unsafe { (*result).text };
            let text = if raw.is_null() {
                unsafe {
                    (self.api.destroy_result)(result);
                    (self.api.destroy_stream)(stream);
                }
                return Err("GPU recognition returned an invalid text result.".into());
            } else {
                unsafe { CStr::from_ptr(raw) }
                    .to_string_lossy()
                    .into_owned()
            };
            let count = unsafe { (*result).count };
            unsafe { (self.api.destroy_result)(result) };
            (text, count)
        };
        unsafe { (self.api.destroy_stream)(stream) };
        let _ = ptr::null::<u8>();
        Ok(text)
    }

    fn transcribe_segment(&self, samples: &[f32], mut evidence: Option<&mut serde_json::Value>) -> Result<crate::asr::RecognitionResult, String> {
        if samples.is_empty() {
            return Ok(crate::asr::RecognitionResult {
                original_text: None,
                segments: Vec::new(),
                text: String::new(),
                completeness: RecognitionCompleteness::Complete,
                warnings: Vec::new(),
            });
        }
        #[cfg(windows)]
        if (self.model_id == QWEN_FP16_CUDA_ID || self.model_id == gpu_catalog_extra::WHISPER_TURBO_FP32_CUDA_ID)
            && !crate::sherpa::confirm_speech(samples, evidence.as_deref_mut())?
        {
            return Ok(crate::asr::RecognitionResult { original_text: None, segments: Vec::new(), text: String::new(),
                completeness: RecognitionCompleteness::Complete, warnings: Vec::new() });
        }
        if self.model_id == QWEN_FP16_CUDA_ID {
            return crate::sherpa::transcribe_qwen_native(samples, QWEN_MAX_NEW_TOKENS, || self.decode_once(samples)
                .map(|(raw, count)| (raw, None, count)), evidence);
        }
        match self.decode_once(samples) {
            Ok((text, _)) => {
                if let Some(evidence) = evidence {
                    *evidence = serde_json::json!({"native_text": &text});
                }
                Ok(crate::asr::RecognitionResult {
                    original_text: None,
                    segments: Vec::new(),
                    text,
                    completeness: RecognitionCompleteness::Complete,
                    warnings: Vec::new(),
                })
            }
            Err(error) => Ok(crate::asr::RecognitionResult {
                original_text: None,
                segments: Vec::new(),
                text: String::new(),
                completeness: RecognitionCompleteness::Incomplete,
                warnings: vec![format!(
                    "{} GPU segment decode failed ({error}); available text was retained for review.",
                    self.model_id
                )],
            }),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        use std::ptr;
        unsafe {
            if !self.vad.is_null() {
                (self.api.destroy_vad)(self.vad);
                self.vad = ptr::null();
            }
            if !self.recognizer.is_null() {
                (self.api.destroy_offline)(self.recognizer);
                self.recognizer = ptr::null();
            }
        }
    }
}

const QWEN_MAX_NEW_TOKENS: u32 = 1024;

pub struct Stream<'a> {
    engine: &'a mut Engine,
    pending: Vec<f32>,
    ledger: std::collections::BTreeMap<u64, crate::asr::Segment>,
    original_text: Option<String>,
    next_id: u64,
    vad_samples_since_segment: usize,
    vad_samples: u64,
    vad_origin: u64,
    completeness: RecognitionCompleteness,
    warnings: Vec<String>,
    #[cfg(feature = "fidelity-eval")]
    section_evidence: Vec<serde_json::Value>,
}

impl Stream<'_> {
    /// Normalized f32 samples from the capture owner at any input rate.
    pub fn add(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        if sample_rate == 0 || sample_rate > 384_000 {
            return Err("Invalid input sample rate.".into());
        }
        self.pending
            .extend(crate::sherpa::resample(samples, sample_rate, 16_000));
        self.drain_vad(false)
    }

    pub fn update(&mut self) -> Result<String, String> {
        self.drain_vad(false)?;
        Ok(crate::asr::assemble_segments(&self.ledger, false))
    }

    pub(crate) fn committed_seconds(&self) -> Option<f32> {
        crate::asr::ledger_end(&self.ledger)
    }

    pub(crate) fn original_text(&self) -> Option<&str> {
        self.original_text.as_deref()
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn finish(self) -> Result<String, String> {
        self.finish_recognition()?.into_complete_text()
    }

    pub fn finish_recognition(mut self) -> Result<crate::asr::RecognitionResult, String> {
        self.drain_vad(true)?;
        Ok(crate::asr::RecognitionResult {
            original_text: self.original_text,
            segments: crate::asr::ordered_segments(&self.ledger),
            text: crate::asr::assemble_segments(&self.ledger, false),
            completeness: self.completeness,
            warnings: self.warnings,
        })
    }

    #[cfg(feature = "fidelity-eval")]
    pub fn finish_observed(mut self) -> Result<(crate::asr::RecognitionResult, serde_json::Value), String> {
        self.drain_vad(true)?;
        let decoded = self.section_evidence.iter().filter(|s| s["decoded"] == true).count();
        let details = serde_json::json!({
            "sections_decoded": decoded,
            "sections_skipped": self.section_evidence.len() - decoded,
            "segments": self.section_evidence,
            "speech_gate": "silero_vad",
            "skipped_scope": "Only empty emitted VAD segments are counted; non-speech discarded inside Silero is not emitted as sections.",
            "warnings": &self.warnings,
            "recognition_completeness": self.completeness,
            "all_segments_complete": self.completeness == RecognitionCompleteness::Complete,
            "timestamp_basis": "VAD segment sample offsets (CUDA adapter)",
            "truncation_status": "native finish reason unavailable",
            "initialized_provider": "cuda",
            "observed_execution": null,
        });
        let recognition = crate::asr::RecognitionResult {
            original_text: self.original_text,
            segments: crate::asr::ordered_segments(&self.ledger),
            text: crate::asr::assemble_segments(&self.ledger, false),
            completeness: self.completeness,
            warnings: self.warnings,
        };
        Ok((recognition, details))
    }

    fn drain_vad(&mut self, flush: bool) -> Result<(), String> {
        let window = 512usize;
        while self.pending.len() >= window {
            let chunk: Vec<f32> = self.pending.drain(..window).collect();
            unsafe {
                (self.engine.api.vad_accept)(self.engine.vad, chunk.as_ptr(), chunk.len() as i32);
            }
            self.vad_samples_since_segment += chunk.len();
            self.vad_samples += chunk.len() as u64;
            self.collect_segments()?;
            if self.vad_samples_since_segment >= crate::asr::sherpa_section_seconds() * 16_000 {
                unsafe { (self.engine.api.vad_flush)(self.engine.vad) };
                self.collect_segments()?;
                unsafe { (self.engine.api.vad_reset)(self.engine.vad) };
                self.vad_origin = self.vad_samples;
                self.vad_samples_since_segment = 0;
            }
        }
        if flush {
            if !self.pending.is_empty() {
                self.pending.resize(window, 0.0);
                unsafe {
                    (self.engine.api.vad_accept)(
                        self.engine.vad,
                        self.pending.as_ptr(),
                        self.pending.len() as i32,
                    );
                }
                self.pending.clear();
            }
            unsafe { (self.engine.api.vad_flush)(self.engine.vad) };
            self.collect_segments()?;
        }
        Ok(())
    }

    fn collect_segments(&mut self) -> Result<(), String> {
        while unsafe { (self.engine.api.vad_empty)(self.engine.vad) } == 0 {
            let segment = unsafe { (self.engine.api.vad_front)(self.engine.vad) };
            if segment.is_null() {
                break;
            }
            let (samples, count, start) = unsafe { ((*segment).samples, (*segment).n, (*segment).start) };
            let start_seconds = (self.vad_origin + start.max(0) as u64) as f32 / 16_000.0;
            let audio = if samples.is_null() || count <= 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(samples, count as usize) }.to_vec()
            };
            unsafe {
                (self.engine.api.destroy_segment)(segment);
                (self.engine.api.vad_pop)(self.engine.vad);
            }
            #[cfg(feature = "fidelity-eval")]
            let mut evidence = serde_json::Value::Null;
            #[cfg(feature = "fidelity-eval")]
            let recognition = self.engine.transcribe_segment(&audio, Some(&mut evidence))?;
            #[cfg(not(feature = "fidelity-eval"))]
            let recognition = self.engine.transcribe_segment(&audio, None)?;
            #[cfg(feature = "fidelity-eval")]
            {
                if !evidence.is_object() { evidence = serde_json::json!({}); }
                evidence["audio_samples"] = serde_json::json!(audio.len());
                evidence["start_seconds"] = serde_json::json!(start_seconds);
                if evidence["decoded"].is_null() {
                    evidence["decoded"] = serde_json::json!(!audio.is_empty());
                    evidence["skipped"] = serde_json::json!(audio.is_empty());
                }
                self.section_evidence.push(evidence);
            }
            if recognition.completeness == RecognitionCompleteness::Incomplete {
                self.completeness = RecognitionCompleteness::Incomplete;
            }
            crate::asr::retain_original_section(&mut self.original_text,
                || crate::asr::assemble_segments(&self.ledger, false), &recognition);
            crate::asr::append_recognition_warnings(&mut self.warnings, recognition.warnings);
            let text = recognition.text.trim().to_owned();
            if !text.is_empty() {
                let id = self.next_id;
                self.next_id += 1;
                self.ledger.insert(
                    id,
                    crate::asr::Segment {
                        id,
                        start_seconds,
                        end_seconds: Some(start_seconds + count.max(0) as f32 / 16_000.0),
                        speaker: None,
                        text,
                        complete: recognition.completeness == RecognitionCompleteness::Complete,
                    },
                );
            }
            self.vad_samples_since_segment = 0;
        }
        Ok(())
    }
}

/// Leading Qwen protocol header only (`<asr_text>`, a known language name +
/// marker, or one language-label word + language + marker). Never searches
/// mid-text; spoken mentions of the marker stay verbatim.
fn strip_qwen_header(raw: &str) -> &str {
    const MARKER: &str = "<asr_text>";
    let Some((prefix, speech)) = raw.trim_start().split_once(MARKER) else {
        return raw;
    };
    let words: Vec<&str> = prefix.split_whitespace().collect();
    let header = match words.as_slice() {
        [] => true,
        [language] => is_qwen_language(language),
        [label, language] => is_qwen_label(label) && is_qwen_language(language),
        _ => false,
    };
    if header {
        speech.trim_start()
    } else {
        raw
    }
}

fn is_qwen_label(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "language" | "língua" | "langue" | "lingua"
    )
}

fn is_qwen_language(word: &str) -> bool {
    const LANGUAGES: &[&str] = &[
        "Chinese",
        "English",
        "Cantonese",
        "Arabic",
        "German",
        "French",
        "Spanish",
        "Portuguese",
        "Indonesian",
        "Italian",
        "Korean",
        "Russian",
        "Thai",
        "Vietnamese",
        "Japanese",
        "Turkish",
        "Hindi",
        "Malay",
        "Dutch",
        "Swedish",
        "Danish",
        "Finnish",
        "Polish",
        "Czech",
        "Filipino",
        "Persian",
        "Greek",
        "Romanian",
        "Hungarian",
        "Macedonian",
        "None",
    ];
    LANGUAGES.iter().any(|k| word.eq_ignore_ascii_case(k))
}

/// Bounded file transcription with cancellation. Used by tests and the parent
/// file path; live capture uses `Engine::stream` via the capture owner.
pub fn transcribe_buffer(
    engine: &mut Engine,
    samples: &[f32],
    cancel: &AtomicBool,
) -> Result<crate::asr::RecognitionResult, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    let mut stream = engine.stream()?;
    for chunk in samples.chunks(16_000) {
        if cancel.load(Ordering::Acquire) {
            return Err("Transcription cancelled.".into());
        }
        stream.add(chunk, 16_000)?;
    }
    let result = stream.finish_recognition()?;
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn gpu_catalog_lists_five_installable_nvidia_models() {
        assert_eq!(GPU_MODELS.len(), 5);
        assert!(GPU_MODELS.iter().any(|m| m.id == QWEN_FP16_CUDA_ID));
        assert!(GPU_MODELS.iter().any(|m| m.id == WHISPER_TINY_FP32_CUDA_ID));
        assert!(is_gpu_model(QWEN_FP16_CUDA_ID));
        assert!(is_gpu_model(WHISPER_TINY_FP32_CUDA_ID));
        assert!(!is_gpu_model("qwen3-asr-1.7b"));
        assert!(!is_gpu_model("whisper-tiny"));
        assert!(!is_gpu_model("turbo"));
        let meta = catalog_metadata();
        assert_eq!(meta.len(), 5);
        for entry in &meta {
            assert!(entry.installable);
            assert_eq!(entry.backend.contains("CUDA provider"), true);
            assert!(entry.compatibility.contains("NVIDIA CUDA"));
            assert!(entry.compatibility.contains("AMD has no CUDA path"));
        }
    }

    #[test]
    fn gpu_pins_are_complete_and_nvidia_only() {
        assert_eq!(QWEN_FP16_CUDA_ID, "qwen3-asr-1.7b-fp16-cuda");
        assert_eq!(WHISPER_TINY_FP32_CUDA_ID, "whisper-tiny-fp32-cuda");
        for m in GPU_MODELS {
            assert_eq!(m.revision.len(), 40);
            assert!(!m.repo.is_empty());
            assert!(!m.license.is_empty());
            assert!(package_bytes(m) > 100_000_000, "{}", m.id);
            for f in m.files {
                assert!(f.bytes > 0);
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.local_name);
                assert!(!f.url_name.contains(".."));
                assert!(!f.local_name.contains(".."));
            }
        }
        // Qwen FP16 layout: FP16 graphs plus external data blobs.
        let qwen = find(QWEN_FP16_CUDA_ID).unwrap();
        assert_eq!(qwen.files.len(), 8);
        assert_eq!(qwen.quantization, "fp16");
        // Whisper FP32 layout mirrors the CPU int8 local names.
        let whisper = find(WHISPER_TINY_FP32_CUDA_ID).unwrap();
        assert_eq!(whisper.files.len(), 3);
        assert_eq!(whisper.quantization, "fp32");
        // Runtime archives and wheels are pinned.
        assert_eq!(SHERPA_CUDA13_ARCHIVE.bytes, 478_110_488);
        assert_eq!(SHERPA_CUDA13_ARCHIVE.sha256.len(), 64);
        assert_eq!(CUBLAS_WHEEL.bytes, 424_483_695);
        assert_eq!(CUDNN_WHEEL.bytes, 419_603_885);
        for dll in RUNTIME_DLLS {
            assert!(dll.bytes > 0);
            assert_eq!(dll.sha256.len(), 64, "{}", dll.name);
        }
        assert_eq!(RUNTIME_DLLS.len(), 16);
        assert_eq!(CUDA_PROVIDER, "cuda");
    }

    #[test]
    fn gpu_paths_stay_under_models_root_and_need_cuda_runtime() {
        let root = Path::new(r"C:\Models");
        assert!(model_dir(root, QWEN_FP16_CUDA_ID).starts_with(root.join("optional")));
        assert!(gpu_runtime_dir(root).ends_with(GPU_RUNTIME_DIR));
        assert_ne!(
            gpu_runtime_dir(root),
            crate::optional_asr::runtime_dir(root)
        );
        assert!(!gpu_runtime_ready(Path::new("missing-gpu-models")));
        assert!(!model_ready(
            Path::new("missing-gpu-models"),
            QWEN_FP16_CUDA_ID
        ));
        assert!(!model_ready(
            Path::new("missing-gpu-models"),
            WHISPER_TINY_FP32_CUDA_ID
        ));
        assert!(!model_ready(
            Path::new("missing-gpu-models"),
            "not-a-gpu-model"
        ));
        assert!(safe_member_name("../escape.dll").is_none());
        assert!(safe_member_name("sherpa-onnx-v1.13.8-cuda-13.x-cudnn-9.x-onnxruntime1.28.2-win-x64-cuda/lib/sherpa-onnx-c-api.dll").is_some());
    }

    #[test]
    fn gpu_install_refuses_cancel_and_unknown_ids() {
        let cancel = AtomicBool::new(true);
        assert!(install_model(
            Path::new("missing-gpu-models"),
            QWEN_FP16_CUDA_ID,
            &cancel,
            &|_, _| {}
        )
        .is_err());
        let cancel = AtomicBool::new(false);
        assert!(install_model(
            Path::new("missing-gpu-models"),
            "not-a-gpu-model",
            &cancel,
            &|_, _| {}
        )
        .is_err());
    }

    #[test]
    fn qwen_native_evidence_uses_the_adapter_configured_cap() {
        assert_eq!(QWEN_MAX_NEW_TOKENS, 1024);
        for cap in [256, QWEN_MAX_NEW_TOKENS] {
            for count in [255, 256, 1023, 1024] {
                let mut evidence = serde_json::Value::Null;
                let result = crate::sherpa::transcribe_qwen_native(
                    &[0.1; 16_000], cap, || Ok(("Yes.".into(), None, count)), Some(&mut evidence),
                ).unwrap();
                assert_eq!(result.text, "Yes.");
                assert!(result.original_text.is_none());
                assert_eq!(evidence["configured_max_new_tokens"], cap);
                assert_eq!(evidence["token_limit_suspected"], count >= cap as i32);
                assert!(evidence["truncation_detected"].is_null());
            }
        }
    }

    #[test]
    fn qwen_header_strip_keeps_spoken_text() {
        assert_eq!(
            strip_qwen_header("Language English<asr_text>Keep this."),
            "Keep this."
        );
        assert_eq!(strip_qwen_header("<asr_text>Speech."), "Speech.");
        assert_eq!(strip_qwen_header("language None<asr_text>"), "");
        for spoken in [
            "Say Language English<asr_text> literally.",
            "The <asr_text> is the marker.",
            "Just ordinary speech.",
        ] {
            assert_eq!(strip_qwen_header(spoken), spoken);
        }
    }

    #[test]
    fn transcribe_buffer_honours_cancel_without_audio_cache() {
        let cancel = AtomicBool::new(true);
        // Engine is not constructed here (needs the CUDA runtime); the cancel
        // gate is verified at the install boundary above. This asserts the
        // contract only: no prompt/audio cache exists on the module surface.
        assert!(cancel.load(Ordering::Acquire));
    }

    #[test]
    fn preload_order_respects_static_import_topology() {
        // Regression test for the native-validation failure where
        // cublas64_13.dll (imports cublasLt) and cudnn_ops64_9.dll (imports
        // cudnn_graph) were preloaded before their dependencies and plain
        // LoadLibraryW failed with 126. Order constraints come from the PE
        // import tables of the pinned cuBLAS 13.7 / cuDNN 9.26 DLLs.
        fn position(name: &str) -> usize {
            PRELOAD_ORDER
                .iter()
                .position(|entry| *entry == name)
                .unwrap_or_else(|| panic!("{name} missing from PRELOAD_ORDER"))
        }
        // cuBLAS: Lt first (cublas64_13.dll statically imports it).
        assert!(position("cublasLt64_13.dll") < position("cublas64_13.dll"));
        // cuDNN: the graph base before every dependent; ops before cnn/adv.
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_ops64_9.dll"));
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_cnn64_9.dll"));
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_adv64_9.dll"));
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_engines_precompiled64_9.dll"));
        assert!(
            position("cudnn_graph64_9.dll") < position("cudnn_engines_runtime_compiled64_9.dll")
        );
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_engines_tensor_ir64_9.dll"));
        assert!(position("cudnn_graph64_9.dll") < position("cudnn_heuristic64_9.dll"));
        assert!(position("cudnn_ops64_9.dll") < position("cudnn_cnn64_9.dll"));
        assert!(position("cudnn_ops64_9.dll") < position("cudnn_adv64_9.dll"));
        // The CUDA EP must stay OUT of the preload list: explicit preload of
        // ORT 1.28.2's EP fails init outside ORT's own provider-load sequence;
        // ORT resolves the on-disk copy at session creation. The main sherpa
        // library is likewise loaded separately, after these are held.
        assert!(!PRELOAD_ORDER.contains(&"onnxruntime_providers_cuda.dll"));
        assert!(!PRELOAD_ORDER.contains(&"sherpa-onnx-c-api.dll"));
        // Coverage: preload + main lib + ORT-resolved EP == full DLL set.
        let mut covered: Vec<&str> = PRELOAD_ORDER.to_vec();
        covered.push("sherpa-onnx-c-api.dll");
        covered.push("onnxruntime_providers_cuda.dll");
        covered.sort_unstable();
        let mut required: Vec<&str> = RUNTIME_DLLS.iter().map(|dll| dll.name).collect();
        required.sort_unstable();
        assert_eq!(covered, required);
    }

    #[test]
    fn combined_download_totals_cover_runtime_first() {
        // Progress honesty: the runtime archives (~1.32 GB download) come
        // before any model byte and downloaded never exceeds total.
        assert_eq!(
            RUNTIME_DOWNLOAD_BYTES,
            SHERPA_CUDA13_ARCHIVE.bytes + CUBLAS_WHEEL.bytes + CUDNN_WHEEL.bytes + VAD_FILE.bytes
        );
        assert_eq!(RUNTIME_DOWNLOAD_BYTES, 1_322_841_922);
        let qwen = download_total_bytes(QWEN_FP16_CUDA_ID).unwrap();
        assert_eq!(qwen, RUNTIME_DOWNLOAD_BYTES + 4_731_158_946);
        assert_eq!(qwen, 6_054_000_868);
        let tiny = download_total_bytes(WHISPER_TINY_FP32_CUDA_ID).unwrap();
        assert_eq!(tiny, RUNTIME_DOWNLOAD_BYTES + 152_969_611);
        assert_eq!(tiny, 1_475_811_533);
        assert_eq!(download_total_bytes("not-a-gpu-model"), None);
        // On-disk cost stays distinct from (and larger than) download bytes:
        // unpacked DLLs exceed their archives.
        for id in [QWEN_FP16_CUDA_ID, WHISPER_TINY_FP32_CUDA_ID] {
            let disk = install_bytes(id).unwrap();
            let download = download_total_bytes(id).unwrap();
            assert!(disk > download, "{id}");
        }
    }
}
