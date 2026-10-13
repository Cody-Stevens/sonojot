//! Native installable Whisper catalog through the existing sherpa CPU boundary.
//! No user Python. All entries run on the already-pinned sherpa-onnx v1.13.8
//! Windows x64 CPU runtime (same DLLs + silero VAD as Parakeet/Qwen).
//! NVIDIA CUDA is never used by this adapter; AMD has no CUDA path.
//!
//! Pins were read from the Hugging Face Hub API + GitHub release API on
//! 2026-09-16 and re-verified 2026-09-16 via paths-info at each pinned
//! revision (24/24 int8 sizes match; 24/24 LFS oids match after the
//! large-v2 decoder fix below). Int8 `bytes` are HF LFS sizes; `sha256`
//! for int8 files is the HF LFS oid (content SHA-256). SonoJot
//! download-verified 2026-09-16 for whisper-tiny encoder/decoder +
//! whisper-turbo triple (see asr-verification-result.md); remaining int8
//! oids are upstream-reported and install-verified on first download
//! (transactional, checksum-gated, never installed unverified).
//! Token hashes below were fetched and hashed individually for all 12
//! checkpoints (large-v3/turbo included): 8x multilingual identical
//! (816730 bytes) and 4x English-only identical (835554 bytes).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub struct WhisperFile {
    pub url_name: &'static str,
    pub local_name: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub struct WhisperCheckpoint {
    pub id: &'static str,
    pub checkpoint: &'static str,
    pub display_name: &'static str,
    pub family: &'static str,
    pub english_only: bool,
    pub older: bool,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [WhisperFile],
    pub license: &'static str,
}

const LICENSE: &str = "MIT (OpenAI Whisper weights; sherpa-onnx runtime Apache-2.0)";
const TOKENS_MULTI_SHA: &str = "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126";
const TOKENS_EN_SHA: &str = "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930";

const TINY_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "tiny-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 12_937_772,
        sha256: "d24fb083ae3b1041fc24e97971d60e280c9342201fbb67b0ab428a8b4a51a434",
    },
    WhisperFile {
        url_name: "tiny-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 89_855_401,
        sha256: "d2fece8dd42771f1df975c6c0445770d0c292bf7547c2cae04a6c0cc57540925",
    },
    WhisperFile {
        url_name: "tiny-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const TINY_EN_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "tiny.en-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 12_937_772,
        sha256: "0ce578b827c94a961aacb8fa14b02f096504b337e5c94be37c36238cbe3e8bc6",
    },
    WhisperFile {
        url_name: "tiny.en-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 89_853_865,
        sha256: "06c0e6ff6348d427e51839219d1c886c18cfdf411e629e33f5e1679bff9c1527",
    },
    WhisperFile {
        url_name: "tiny.en-tokens.txt",
        local_name: "tokens.txt",
        bytes: 835_554,
        sha256: TOKENS_EN_SHA,
    },
];
const BASE_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "base-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 29_120_534,
        sha256: "0b8fb1304b6109976038efff5ace81720e00386f3ff6b54ee8c75291ca0a1e11",
    },
    WhisperFile {
        url_name: "base-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 130_672_026,
        sha256: "9759d217388a01b3a4c7c15533201067b48ae819c4daafc8624e64b9409dc02d",
    },
    WhisperFile {
        url_name: "base-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const BASE_EN_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "base.en-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 29_120_534,
        sha256: "ef6b936f4c9b1d90a3b68634b60c4ed8576b26172b33c2535ec0e933c9edb823",
    },
    WhisperFile {
        url_name: "base.en-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 130_669_978,
        sha256: "f7162ad6db2dbef16cfaeaa7f945b9d7dd9c1b8d472f6aca82f2273d185e4d41",
    },
    WhisperFile {
        url_name: "base.en-tokens.txt",
        local_name: "tokens.txt",
        bytes: 835_554,
        sha256: TOKENS_EN_SHA,
    },
];
const SMALL_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "small-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 112_442_483,
        sha256: "4cbe7b22fa9026b843b60a68640c747de05bafb1a11b57edc0e66c232d9f33a9",
    },
    WhisperFile {
        url_name: "small-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 262_226_114,
        sha256: "acad50b5c782696e91b55914cc5ab4f756f1532f76e22aa6fc615f39fb69a8ee",
    },
    WhisperFile {
        url_name: "small-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const SMALL_EN_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "small.en-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 112_442_483,
        sha256: "8bdac288f369aa94ee2194059238c465ed82ea9d47ee8fa4a8c0a891873e462f",
    },
    WhisperFile {
        url_name: "small.en-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 262_223_042,
        sha256: "710ccf890e10f3faa15f51ec346081a2723c9f3adb6e4da81c6573a5a6f877fb",
    },
    WhisperFile {
        url_name: "small.en-tokens.txt",
        local_name: "tokens.txt",
        bytes: 835_554,
        sha256: TOKENS_EN_SHA,
    },
];
const MEDIUM_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "medium-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 374_196_283,
        sha256: "1c54582b4d829de0089f6cb63bbbdb3bf7555398bacaf855fbecf1a84dfd193e",
    },
    WhisperFile {
        url_name: "medium-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 571_059_257,
        sha256: "595d00a338a365a7bfa0ca7f296cabc639583bef770ab6130df90f49a6412747",
    },
    WhisperFile {
        url_name: "medium-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const MEDIUM_EN_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "medium.en-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 374_196_283,
        sha256: "5a8e3a36619e0b67db9320eef3152db59d4b440f5ce0212d2c162a61b750bf80",
    },
    WhisperFile {
        url_name: "medium.en-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 571_055_161,
        sha256: "7303be339ed4e51f4ffb7ae84f3803b10cf8e67e1dcf8a98cb4d843f0dea0141",
    },
    WhisperFile {
        url_name: "medium.en-tokens.txt",
        local_name: "tokens.txt",
        bytes: 835_554,
        sha256: TOKENS_EN_SHA,
    },
];
const LARGE_V3_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "large-v3-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 766_671_985,
        sha256: "d531cf17248acc43e8c09b472a0877055e770877857a5332fc1304b36534ec85",
    },
    WhisperFile {
        url_name: "large-v3-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 1_008_265_203,
        sha256: "ebc6bfd88e162a46cb3edee8a7e727e1dcbc65cabecb19e2573695e4d495e1af",
    },
    WhisperFile {
        url_name: "large-v3-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const TURBO_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "turbo-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 674_716_297,
        sha256: "b02dcdf54f348741e93fe732b67d933c8dcb6735655f710640143081db38878b",
    },
    WhisperFile {
        url_name: "turbo-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 361_080_764,
        sha256: "20accd02388482eb3a46bd615631adfdc85e1eb2c7db9ea3f02a40ffe6b81547",
    },
    WhisperFile {
        url_name: "turbo-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const LARGE_V1_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "large-v1-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 765_934_692,
        sha256: "1643928022a8fe0a40afff5935703751a6486aee65b99717daa14728fec273f6",
    },
    WhisperFile {
        url_name: "large-v1-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 1_008_260_083,
        sha256: "1b43c7ae36ef8c4eed9d2e07eca6c96e51edb83ceda275146c5282e93a8c68b1",
    },
    WhisperFile {
        url_name: "large-v1-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];
const LARGE_V2_FILES: &[WhisperFile] = &[
    WhisperFile {
        url_name: "large-v2-encoder.int8.onnx",
        local_name: "encoder.int8.onnx",
        bytes: 765_934_692,
        sha256: "f055f8397a69895818c1763fd001e47fb9ca51ea7efaccfafc1b635d121203a3",
    },
    WhisperFile {
        url_name: "large-v2-decoder.int8.onnx",
        local_name: "decoder.int8.onnx",
        bytes: 1_008_260_083,
        sha256: "ea513c5bfcbdc422b025da8fc93065ab8706994ba90053af442145b9aa7c2ee8",
    },
    WhisperFile {
        url_name: "large-v2-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: TOKENS_MULTI_SHA,
    },
];

pub const WHISPER_CATALOG: &[WhisperCheckpoint] = &[
    WhisperCheckpoint {
        id: "whisper-tiny",
        checkpoint: "tiny",
        display_name: "Whisper Tiny",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-tiny",
        revision: "65176e2deb88badc814a94058666cadccc29b61c",
        files: TINY_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-tiny.en",
        checkpoint: "tiny.en",
        display_name: "Whisper Tiny (English)",
        family: "whisper",
        english_only: true,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-tiny.en",
        revision: "d026532c022fa99fd789d6b32446a1df7b6bfc43",
        files: TINY_EN_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-base",
        checkpoint: "base",
        display_name: "Whisper Base",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-base",
        revision: "bb53ee204431c90d314c1cc08d28d23e5b7927cc",
        files: BASE_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-base.en",
        checkpoint: "base.en",
        display_name: "Whisper Base (English)",
        family: "whisper",
        english_only: true,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-base.en",
        revision: "59eea950fc76df2453efb57e6c0fd334548e8ffe",
        files: BASE_EN_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-small",
        checkpoint: "small",
        display_name: "Whisper Small",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-small",
        revision: "8f3c18b358db4d1f2fc1eae49d75cd20989e4309",
        files: SMALL_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-small.en",
        checkpoint: "small.en",
        display_name: "Whisper Small (English)",
        family: "whisper",
        english_only: true,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-small.en",
        revision: "d9533f69affd85061aee349af7fea5cb2996dbbe",
        files: SMALL_EN_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-medium",
        checkpoint: "medium",
        display_name: "Whisper Medium",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-medium",
        revision: "8c31d28503847560985df21f90e14f0c736e075e",
        files: MEDIUM_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-medium.en",
        checkpoint: "medium.en",
        display_name: "Whisper Medium (English)",
        family: "whisper",
        english_only: true,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-medium.en",
        revision: "251ab4521f354490e8f2c206fbd0b7f3f6b0a7ec",
        files: MEDIUM_EN_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-large-v3",
        checkpoint: "large-v3",
        display_name: "Whisper Large v3",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-large-v3",
        revision: "2a6507094dd6020d939d78e3f1834a1d06267fca",
        files: LARGE_V3_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-large-v3-turbo",
        checkpoint: "large-v3-turbo",
        display_name: "Whisper Large v3 Turbo",
        family: "whisper",
        english_only: false,
        older: false,
        repo: "csukuangfj/sherpa-onnx-whisper-turbo",
        revision: "2ca6ff69fc878651b770880507669577ac41c2ff",
        files: TURBO_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-large-v1",
        checkpoint: "large-v1",
        display_name: "Whisper Large v1 (older)",
        family: "whisper",
        english_only: false,
        older: true,
        repo: "csukuangfj/sherpa-onnx-whisper-large-v1",
        revision: "5ec7cc1eddcc63ec78aaae738f7023e8c6e048a1",
        files: LARGE_V1_FILES,
        license: LICENSE,
    },
    WhisperCheckpoint {
        id: "whisper-large-v2",
        checkpoint: "large-v2",
        display_name: "Whisper Large v2 (older)",
        family: "whisper",
        english_only: false,
        older: true,
        repo: "csukuangfj/sherpa-onnx-whisper-large-v2",
        revision: "1dce177ee978f227f39538443ae7c3bc611e9be4",
        files: LARGE_V2_FILES,
        license: LICENSE,
    },
];

pub const COMPATIBILITY: &str = "Windows x64 CPU via bundled sherpa-onnx v1.13.8; NVIDIA CUDA not used by this adapter; AMD has no CUDA path";
pub const BACKEND: &str = "sherpa-onnx-1.13.8-cpu (OfflineWhisper, transcribe, CPU provider)";

pub fn find(id: &str) -> Option<&'static WhisperCheckpoint> {
    WHISPER_CATALOG.iter().find(|m| m.id == id)
}

pub fn is_whisper(id: &str) -> bool {
    find(id).is_some()
}

pub fn model_dir(models_root: &Path, id: &str) -> PathBuf {
    crate::optional_asr::optional_root(models_root).join(id)
}

pub fn package_bytes(m: &WhisperCheckpoint) -> u64 {
    m.files.iter().map(|f| f.bytes).sum()
}

fn pinned_as_optional(file: &WhisperFile) -> crate::optional_asr::PinnedFile {
    crate::optional_asr::PinnedFile {
        relative: file.local_name,
        bytes: file.bytes,
        sha256: file.sha256,
    }
}

pub fn model_ready(models_root: &Path, id: &str) -> bool {
    model_status(models_root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(models_root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    let Some(m) = find(id) else { return Status::Missing };
    let dir = model_dir(models_root, m.id);
    Status::all([
        crate::optional_asr::runtime_status(models_root, check),
        Status::all(m.files.iter().map(|f| check.file(models_root, &dir.join(f.local_name), f.bytes, f.sha256))),
    ])
}

// ---- Resource-worker metadata (concrete, no fake precision) ----

pub struct AsrCatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    pub languages: &'static str,
    pub older: bool,
    pub revision: &'static str,
    pub quantization: &'static str,
    pub backend: &'static str,
    pub compatibility: &'static str,
    pub disk_bytes: u64,
    pub installable: bool,
    pub reason: &'static str,
    pub evidence: &'static str,
}

pub fn catalog_metadata() -> Vec<AsrCatalogEntry> {
    WHISPER_CATALOG
        .iter()
        .map(|m| {
            let (languages, reason) = if m.older {
                (
                    "Multilingual",
                    "Older checkpoint; shown for disclosure, not recommended over large-v3/turbo.",
                )
            } else if m.english_only {
                ("English only", "English-only checkpoint; multilingual audio needs the non-.en sibling.")
            } else if m.checkpoint == "large-v3-turbo" {
                (
                    "Multilingual",
                    "Fewer decoder layers than large-v3; faster per upstream design, quality Unrated in SonoJot.",
                )
            } else {
                ("Multilingual", "Curated int8 artifact for the bundled sherpa CPU runtime.")
            };
            AsrCatalogEntry {
                id: m.id,
                name: m.display_name,
                family: m.family,
                languages,
                older: m.older,
                revision: m.revision,
                quantization: "int8",
                backend: BACKEND,
                compatibility: COMPATIBILITY,
                disk_bytes: package_bytes(m),
                installable: true,
                reason,
                evidence: "HF paths-info at pinned revisions 2026-09-16 (12/12 revisions are HEAD; 24/24 int8 sizes + LFS oids match); all 12 tokenizers individually fetched+hashed (2 distinct SHA); tiny+turbo triples SonoJot download-verified, other int8 hashes install-gated",
            }
        })
        .collect()
}

// ---- Qwen CPU/CUDA truthful split ----

pub struct QwenGpuInfo {
    pub id: &'static str,
    pub installable: bool,
    pub reason: &'static str,
}

pub fn qwen_gpu_info() -> QwenGpuInfo {
    QwenGpuInfo {
        id: "qwen3-asr-1.7b-fp16-cuda",
        installable: false,
        reason: "Research-only: third-party FP16 provenance (luigi000/qwen3-asr-1.7b-onnx-fp16, rev 90c70bc1) unreviewed; ~4.7 GB weights + 478-595 MB CUDA runtime required; VRAM fit and speed unmeasured; int8 CPU fallback must never be reported as GPU accelerated. AMD unsupported (no CUDA path).",
    }
}

pub struct RuntimePin {
    pub id: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub fn cuda_runtime_pins() -> Vec<RuntimePin> {
    vec![
        RuntimePin {
            id: "sherpa-cuda13",
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-cuda-13.x-cudnn-9.x-onnxruntime1.28.2-win-x64-cuda.tar.bz2",
            bytes: 478_110_488,
            sha256: "1702e4a1ee68a07422469a7ec1d304a54b43a345ad888daf8e0b0257004a3273",
        },
        RuntimePin {
            id: "sherpa-cuda12",
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-cuda-12.x-cudnn-9.x-onnxruntime1.28.2-win-x64-cuda.tar.bz2",
            bytes: 595_017_373,
            sha256: "066c5b54dbafaa1388001a9c9837ac1374dbba6d6678f193ca06aa0d8e94d8c3",
        },
    ]
}

// ---- Install with cancel + progress ----
//
// Caller (optional_asr::install_with_progress) must already hold the install
// gate and the idle guard. This function never touches Tauri state.

const DOWNLOAD_LIMIT: Duration = Duration::from_secs(60 * 60);

pub fn install_model(
    models_root: &Path,
    id: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    if crate::asr::is_gpu_model(id) {
        return Err(format!(
            "GPU model {id} uses the CUDA installer; not a CPU Whisper install."
        ));
    }
    let m = find(id).ok_or("Choose a downloadable Whisper model.")?;
    if cancel.load(Ordering::Acquire) {
        return Err("Transcription cancelled.".into());
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (models_root, m, cancel, progress);
        return Err("Automatic speech-model install is available only on 64-bit Windows.".into());
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        if !crate::optional_asr::runtime_ready(models_root) {
            return Err("Install an existing optional speech model first so the verified CPU runtime is present; Whisper reuses that runtime.".into());
        }
        if model_ready(models_root, m.id) {
            return Ok(());
        }
        let total: u64 = package_bytes(m);
        let mut done: u64 = 0;
        progress(0, total);
        let root = crate::optional_asr::optional_root(models_root);
        fs::create_dir_all(&root)
            .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
        crate::optional_asr::ensure_within(models_root, &root)?;
        let dest = model_dir(models_root, m.id);
        // Preserve any existing install until the new staging verifies; delete
        // only just before atomic rename. A failed/cancelled download must not
        // corrupt the current model, and cancel never reports 100.
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
                download_with_progress(
                    &url,
                    &path,
                    f.bytes,
                    f.sha256,
                    cancel,
                    &|chunk_done, chunk_total| {
                        let base = done;
                        let scaled = if chunk_total == 0 {
                            0
                        } else {
                            chunk_done.min(chunk_total)
                        };
                        progress(base + scaled.min(f.bytes), total);
                    },
                )?;
                done = done.saturating_add(f.bytes);
                if done < total {
                    progress(done, total);
                }
            }
            if cancel.load(Ordering::Acquire) {
                return Err("Transcription cancelled.".into());
            }
            for f in m.files {
                if cancel.load(Ordering::Acquire) {
                    return Err("Transcription cancelled.".into());
                }
                let pinned = pinned_as_optional(f);
                let ok = crate::optional_asr::file_matches(&staging.join(f.local_name), &pinned)?;
                if !ok {
                    return Err(format!(
                        "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                        f.local_name
                    ));
                }
            }
            if cancel.load(Ordering::Acquire) {
                return Err("Transcription cancelled.".into());
            }
            if dest.exists() && model_ready(models_root, m.id) {
                return Err("This speech model is already installed.".into());
            }
            crate::optional_asr::transactional_activate(
                models_root,
                &staging,
                &dest,
                "speech model",
            )?;
            progress(total, total);
            Ok(())
        })();
        if installed.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        installed
    }
}

pub(crate) fn download_with_progress(
    url: &str,
    dest: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    use std::io::{Read, Write};
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(DOWNLOAD_LIMIT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    host == "huggingface.co"
                        || host.ends_with(".huggingface.co")
                        || host.ends_with(".hf.co")
                        || host == "cas-bridge.xethub.hf.co"
                        || host.ends_with(".xethub.hf.co")
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
        .map_err(|e| format!("The speech model download could not start: {e}"))?
        .error_for_status()
        .map_err(|e| format!("The speech model download was refused: {e}"))?;
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
    let file =
        std::fs::File::create(dest).map_err(|e| format!("Cannot stage the download: {e}"))?;
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
            return Err("The speech model download exceeded the time limit. Check the connection and try again.".into());
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The speech model download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err("The speech model download was larger than the pinned file.".into());
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
        return Err("The downloaded speech model file did not match its pinned checksum. Nothing was installed.".into());
    }
    Ok(())
}

fn sha256_of(path: &Path, expected_bytes: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("The download could not be read: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn catalog_covers_all_twelve_checkpoints_without_aliases() {
        assert_eq!(WHISPER_CATALOG.len(), 12);
        let ids: Vec<_> = WHISPER_CATALOG.iter().map(|m| m.id).collect();
        for expected in [
            "whisper-tiny",
            "whisper-tiny.en",
            "whisper-base",
            "whisper-base.en",
            "whisper-small",
            "whisper-small.en",
            "whisper-medium",
            "whisper-medium.en",
            "whisper-large-v3",
            "whisper-large-v3-turbo",
            "whisper-large-v1",
            "whisper-large-v2",
        ] {
            assert!(ids.contains(&expected), "{expected} missing");
        }
        // Aliases are never installable IDs.
        assert!(!is_whisper("large"));
        assert!(!is_whisper("turbo"));
        assert!(!is_whisper("large.en"));
        assert!(!is_whisper("turbo.en"));
        // Older placement is exactly v1/v2.
        let older: Vec<_> = WHISPER_CATALOG
            .iter()
            .filter(|m| m.older)
            .map(|m| m.id)
            .collect();
        assert_eq!(older, vec!["whisper-large-v1", "whisper-large-v2"]);
        // English-only scope is exactly the four .en checkpoints.
        let en_only: Vec<_> = WHISPER_CATALOG
            .iter()
            .filter(|m| m.english_only)
            .map(|m| m.id)
            .collect();
        assert_eq!(
            en_only,
            vec![
                "whisper-tiny.en",
                "whisper-base.en",
                "whisper-small.en",
                "whisper-medium.en"
            ]
        );
    }

    #[test]
    fn pinned_bytes_and_hashes_are_complete_and_compatible_cpu_only() {
        for m in WHISPER_CATALOG {
            assert_eq!(m.files.len(), 3);
            assert!(package_bytes(m) > 90_000_000, "{}", m.id);
            for f in m.files {
                assert!(f.bytes > 0);
                assert_eq!(f.sha256.len(), 64, "{} {}", m.id, f.local_name);
                assert!(!f.url_name.contains(".."));
                assert!(!f.local_name.contains(".."));
            }
            assert!(!m.repo.is_empty() && !m.revision.is_empty());
            assert_eq!(m.revision.len(), 40);
        }
        assert!(COMPATIBILITY.contains("NVIDIA CUDA not used"));
        assert!(COMPATIBILITY.contains("AMD has no CUDA path"));
        assert!(BACKEND.contains("CPU provider"));
        let meta = catalog_metadata();
        assert_eq!(meta.len(), 12);
        assert!(meta.iter().all(|e| e.installable));
    }

    #[test]
    fn qwen_gpu_variant_is_never_advertised_as_accelerated() {
        let gpu = qwen_gpu_info();
        assert_eq!(gpu.id, "qwen3-asr-1.7b-fp16-cuda");
        assert!(!gpu.installable);
        assert!(
            gpu.reason.contains("never be reported as GPU accelerated")
                || gpu.reason.contains("must never be reported")
                || gpu.reason.contains("AMD unsupported")
        );
        let pins = cuda_runtime_pins();
        assert_eq!(pins.len(), 2);
        for pin in &pins {
            assert!(pin.url.contains("sherpa-onnx-v1.13.8-cuda-"));
            assert!(pin.bytes > 400_000_000);
            assert_eq!(pin.sha256.len(), 64);
        }
    }

    #[test]
    fn missing_models_are_not_ready_and_cancel_refuses_install() {
        assert!(!model_ready(
            Path::new("missing-test-models"),
            "whisper-tiny"
        ));
        assert!(!model_ready(Path::new("."), "not-a-whisper"));
        let cancel = AtomicBool::new(true);
        let err = install_model(
            Path::new("missing-test-models"),
            "whisper-tiny",
            &cancel,
            &|_, _| {},
        )
        .unwrap_err();
        assert!(err.contains("cancelled") || err.contains("Cancelled"));
        let cancel = AtomicBool::new(false);
        assert!(install_model(
            Path::new("missing-test-models"),
            "not-a-whisper",
            &cancel,
            &|_, _| {}
        )
        .is_err());
    }

    #[test]
    fn whisper_paths_stay_under_the_models_root() {
        let root = Path::new(r"C:\Models");
        assert!(model_dir(root, "whisper-tiny").starts_with(root.join("optional")));
    }

    #[test]
    fn gpu_ids_are_disjoint_and_rejected_explicitly() {
        for gpu in ["qwen3-asr-1.7b-fp16-cuda", "whisper-tiny-fp32-cuda"] {
            assert!(!is_whisper(gpu), "{gpu}");
            assert!(find(gpu).is_none(), "{gpu}");
            assert!(!model_ready(Path::new("missing-test-models"), gpu));
            let cancel = AtomicBool::new(false);
            let err = install_model(Path::new("missing-test-models"), gpu, &cancel, &|_, _| {})
                .unwrap_err();
            assert!(err.contains("CUDA installer"), "{gpu}: {err}");
        }
        // 12 pins preserved (large-v2 decoder fix kept).
        assert_eq!(WHISPER_CATALOG.len(), 12);
        let v2 = find("whisper-large-v2").unwrap();
        let dec = v2
            .files
            .iter()
            .find(|f| f.local_name == "decoder.int8.onnx")
            .unwrap();
        assert_eq!(
            dec.sha256,
            "ea513c5bfcbdc422b025da8fc93065ab8706994ba90053af442145b9aa7c2ee8"
        );
        assert_eq!(dec.sha256.len(), 64);
    }
}
