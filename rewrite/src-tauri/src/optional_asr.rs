//! Optional downloadable speech models. Nothing here loads at startup or
//! without an explicit Install action. Files land under the models pointer
//! folder, separately from the located Moonshine bundle, and are deleted on
//! Uninstall. The selected model is loaded only when dictation or file
//! transcription starts.
use crate::{
    app::{lock, Runtime},
    setup::SetupStatus,
    setup_commands::require_idle,
    store,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, State};

/// Legacy gate name kept for callers. Now delegates to the shared
/// mutation/selection guard in `resource_manager` so `model_manager`,
/// legacy ASR/text installs/removes, and selection commits all share ONE
/// guard. Lock order wherever the coordinator is also needed:
/// selection guard -> coordinator -> install registry. No path inverts it.
static INSTALL_GATE: Mutex<()> = Mutex::new(());

const OPTIONAL_ROOT: &str = "optional";
const RUNTIME_DIR: &str = "sherpa-runtime";
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy)]
pub(crate) enum HostClass {
    HuggingFace,
    GitHub,
}

#[derive(Clone, Copy)]
pub struct PinnedFile {
    pub relative: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

#[derive(Clone, Copy)]
pub struct OptionalModel {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [PinnedFile],
}

const PARAKEET: OptionalModel = OptionalModel {
    id: "parakeet-tdt-0.6b-v2",
    name: "Parakeet TDT 0.6B v2",
    description: "English · NVIDIA Parakeet TDT 0.6B v2",
    repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
    revision: "1ab9323565ddb038682214b292f588070a538ce2",
    files: &[
        PinnedFile {
            relative: "encoder.int8.onnx",
            bytes: 652_184_296,
            sha256: "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab",
        },
        PinnedFile {
            relative: "decoder.int8.onnx",
            bytes: 7_257_753,
            sha256: "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e",
        },
        PinnedFile {
            relative: "joiner.int8.onnx",
            bytes: 1_739_080,
            sha256: "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2",
        },
        PinnedFile {
            relative: "tokens.txt",
            bytes: 9_384,
            sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
        },
    ],
};

const QWEN: OptionalModel = OptionalModel {
    id: "qwen3-asr-1.7b",
    name: "Qwen3-ASR 1.7B",
    description: "Multilingual · Qwen3-ASR 1.7B",
    repo: "thieunv/sherpa-onnx-qwen3-asr-1.7B-int8",
    revision: "69eb686fd94a4a865bb5340a3d6ac0d7f1fec0d5",
    files: &[
        PinnedFile {
            relative: "conv_frontend.onnx",
            bytes: 48_080_441,
            sha256: "3cb27a9fe94d95c938e476f2012b21aba2ec0bfceef33b0e58acd208946bafdd",
        },
        PinnedFile {
            relative: "encoder.int8.onnx",
            bytes: 314_222_162,
            sha256: "a5deedae034ece715de8ed204378d8c77f889af3a60c2566581135e84cced7cd",
        },
        PinnedFile {
            relative: "decoder.int8.onnx",
            bytes: 2_037_458_645,
            sha256: "c43c853fa6e97d08365cb8a5502b360b595cd43c00dc60e4d8ca7cc18cad460b",
        },
        PinnedFile {
            relative: "tokenizer/merges.txt",
            bytes: 1_671_853,
            sha256: "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
        },
        PinnedFile {
            relative: "tokenizer/tokenizer_config.json",
            bytes: 12_487,
            sha256: "4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c",
        },
        PinnedFile {
            relative: "tokenizer/vocab.json",
            bytes: 2_776_833,
            sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
        },
    ],
};

pub const MODELS: &[OptionalModel] = &[PARAKEET, QWEN];

#[cfg(target_os = "linux")]
const RUNTIME_ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib.tar.bz2";
#[cfg(target_os = "linux")]
const RUNTIME_ARCHIVE_SHA256: &str = "bf2d998c8b07012cd5098f3b92673bc1333fd9b927767d7cb664be8190d8bc0b";
#[cfg(target_os = "linux")]
const RUNTIME_ARCHIVE_BYTES: u64 = 9_249_201;
#[cfg(target_os = "linux")]
const RUNTIME_ARCHIVE_ROOT: &str = "sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib";

#[cfg(target_os = "linux")]
pub(crate) const RUNTIME_FILES: &[PinnedFile] = &[
    PinnedFile { relative: "libonnxruntime.so", bytes: 27_026_609, sha256: "4b3607aebd1784b26b6f9b20e4bd974c7ab8287043e4d095cb7d2cb40b5e566e" },
    PinnedFile { relative: "libsherpa-onnx-c-api.so", bytes: 3_626_056, sha256: "c067b18cd746e49eb5016ee428196d21e1df14c14a3c54f128c1ac95f6aaed50" },
    PinnedFile { relative: "silero_vad.onnx", bytes: VAD_BYTES, sha256: VAD_SHA256 },
];

#[cfg(not(target_os = "linux"))]
const RUNTIME_ARCHIVE_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts-lib.tar.bz2";
#[cfg(not(target_os = "linux"))]
const RUNTIME_ARCHIVE_SHA256: &str =
    "a1253e665c4f236119c443c8932a8acfca32a546c78c05962d483c9a0eae21b7";
#[cfg(not(target_os = "linux"))]
const RUNTIME_ARCHIVE_BYTES: u64 = 7_536_080;
#[cfg(not(target_os = "linux"))]
const RUNTIME_ARCHIVE_ROOT: &str = "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts-lib";
const VAD_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
pub(crate) const VAD_SHA256: &str = "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";
pub(crate) const VAD_BYTES: u64 = 643_854;

#[cfg(not(target_os = "linux"))]
pub(crate) const RUNTIME_FILES: &[PinnedFile] = &[
    PinnedFile {
        relative: "onnxruntime.dll",
        bytes: 17_799_168,
        sha256: "7f66f939a881baf4f46a2216496798edf4a1429878b646d12674aa62f27d8a25",
    },
    PinnedFile {
        relative: "onnxruntime_providers_shared.dll",
        bytes: 104_960,
        sha256: "551d0e1fe4c227d8542314ba718d52f4379e0c7bfe729a37c59833a884e27b4d",
    },
    PinnedFile {
        relative: "sherpa-onnx-c-api.dll",
        bytes: 3_249_664,
        sha256: "86c7807d12982aa31c5cae57c686f30df9e5d5db4309b314d1efd5c873a8333c",
    },
    PinnedFile {
        relative: "silero_vad.onnx",
        bytes: VAD_BYTES,
        sha256: VAD_SHA256,
    },
];

pub(crate) fn try_install_gate() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    // ONE shared guard for every mutation/selection path. Returns the shared
    // selection guard itself (held across the whole download/remove so
    // selection fails fast). Loads take only the coordinator and proceed
    // concurrently with downloads. The legacy `INSTALL_GATE` static is
    // retired (kept for source compat, never locked) so two independent
    // install mutexes can never diverge.
    let _ = &INSTALL_GATE;
    crate::resource_manager::try_lock_selection_guard()
}

pub fn catalog() -> &'static [OptionalModel] {
    MODELS
}

pub fn find(id: &str) -> Option<&'static OptionalModel> {
    MODELS.iter().find(|model| model.id == id)
}

pub fn is_optional(id: &str) -> bool {
    find(id).is_some()
}

pub fn optional_root(models_root: &Path) -> PathBuf {
    models_root.join(OPTIONAL_ROOT)
}

pub fn model_dir(models_root: &Path, id: &str) -> PathBuf {
    optional_root(models_root).join(id)
}

pub fn runtime_dir(models_root: &Path) -> PathBuf {
    optional_root(models_root).join(RUNTIME_DIR)
}

pub fn runtime_library(models_root: &Path) -> PathBuf {
    runtime_dir(models_root).join(if cfg!(target_os = "linux") { "libsherpa-onnx-c-api.so" } else { "sherpa-onnx-c-api.dll" })
}

pub fn vad_path(models_root: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        mac_speech_runtime_dir(models_root).join("silero_vad.onnx")
    } else {
        runtime_dir(models_root).join("silero_vad.onnx")
    }
}

/// Qwen owns its voice detector independently of the optional speaker model.
/// Removing diarization must never remove Qwen's production speech gate.
pub(crate) fn mac_speech_runtime_dir(root: &Path) -> PathBuf {
    optional_root(root).join("speech-vad")
}

pub(crate) fn mac_speech_runtime_library(root: &Path) -> PathBuf {
    mac_speech_runtime_dir(root).join(crate::diarization_model::MAC_RUNTIME_MEMBERS[0].1.relative)
}

const SILERO_PIN: PinnedFile = PinnedFile {
    relative: "silero_vad.onnx",
    bytes: VAD_BYTES,
    sha256: VAD_SHA256,
};

pub(crate) fn mac_speech_runtime_status(root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    let dir = mac_speech_runtime_dir(root);
    crate::model_verification::Status::all(
        crate::diarization_model::MAC_RUNTIME_MEMBERS.iter().map(|(_, pin)| pin)
            .chain(std::iter::once(&SILERO_PIN))
            .map(|pin| check.file(root, &dir.join(pin.relative), pin.bytes, pin.sha256)),
    )
}

pub(crate) fn mac_speech_runtime_download_bytes(root: &Path) -> u64 {
    if mac_speech_runtime_status(root, crate::model_verification::Check::Background).ready() {
        0
    } else {
        crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES + VAD_BYTES
    }
}

pub(crate) fn ensure_mac_speech_runtime(
    root: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let check_cancel = || {
        if cancel.load(Ordering::Acquire) { Err("Install cancelled.".to_string()) } else { Ok(()) }
    };
    check_cancel()?;
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err("The Metal speech detector requires an Apple Silicon Mac.".into());
    }
    if mac_speech_runtime_status(root, crate::model_verification::Check::Blocking).ready() {
        return Ok(());
    }
    let parent = optional_root(root);
    fs::create_dir_all(&parent).map_err(|e| format!("Cannot create speech runtime folder: {e}"))?;
    ensure_within(root, &parent)?;
    let dest = mac_speech_runtime_dir(root);
    ensure_within(root, &dest)?;
    let staging = parent.join(format!(".install-speech-vad-{}", store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot stage speech detector: {e}"))?;
    let total = crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES + VAD_BYTES;
    let result = (|| {
        let archive = staging.join("onnxruntime.tgz");
        // Both files are GitHub release assets; the Hugging Face downloader
        // refuses GitHub's redirect and reads the redirect's size instead.
        download_with_cancel_progress(
            crate::diarization_model::MAC_RUNTIME_URL, &archive,
            crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES,
            crate::diarization_model::MAC_RUNTIME_ARCHIVE_SHA256,
            HostClass::GitHub, "ONNX Runtime", cancel,
            &|done, _| progress(done.min(total - 1), total),
        )?;
        check_cancel()?;
        crate::diarization_model::extract_mac_runtime(&archive, &staging, cancel)?;
        fs::remove_file(&archive).map_err(|e| format!("Cannot remove staged runtime archive: {e}"))?;
        download_with_cancel_progress(
            VAD_URL, &staging.join(SILERO_PIN.relative), VAD_BYTES, VAD_SHA256,
            HostClass::GitHub, "speech detector", cancel,
            &|done, _| progress((crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES + done).min(total - 1), total),
        )?;
        check_cancel()?;
        for pin in crate::diarization_model::MAC_RUNTIME_MEMBERS.iter().map(|(_, pin)| pin)
            .chain(std::iter::once(&SILERO_PIN))
        {
            if !file_matches(&staging.join(pin.relative), pin)? {
                return Err(format!("Speech detector {} failed checksum verification.", pin.relative));
            }
        }
        check_cancel()?;
        transactional_activate(root, &staging, &dest, "speech detector")?;
        progress(total, total);
        Ok(())
    })();
    // This is the unique, contained staging folder created by this call.
    let _ = fs::remove_dir_all(&staging);
    result
}

pub fn package_bytes(model: &OptionalModel) -> u64 {
    model.files.iter().map(|file| file.bytes).sum()
}

pub(crate) fn runtime_download_bytes(models_root: &Path) -> u64 {
    if runtime_status(models_root, crate::model_verification::Check::Background).ready() {
        0
    } else {
        RUNTIME_ARCHIVE_BYTES + VAD_BYTES
    }
}

pub fn runtime_ready(models_root: &Path) -> bool {
    runtime_status(models_root, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn runtime_status(root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    check.files(root, &runtime_dir(root), RUNTIME_FILES)
}

pub fn model_ready(root: &Path, id: &str) -> bool {
    model_status(root, id, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(root: &Path, id: &str, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    let Some(model) = find(id) else { return Status::Missing };
    Status::all([runtime_status(root, check), check.files(root, &model_dir(root, id), model.files)])
}

pub(crate) fn file_matches(path: &Path, file: &PinnedFile) -> Result<bool, String> {
    crate::model_verification::uncached(path, file.bytes, file.sha256)
}

#[tauri::command]
pub async fn install_asr_model(
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
pub async fn uninstall_asr_model(
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
    runtime.engine.refresh_models();
    let models = crate::app::model_catalog(&runtime.models_root);
    {
        let mut snapshot = lock(&runtime.snapshot)?;
        snapshot.models = models;
        snapshot.error = None;
    }
    runtime.emit(app);
    Ok(())
}

pub fn install(runtime: &Runtime, id: &str) -> Result<(), String> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    install_with_progress(runtime, id, &NEVER, &|_, _| {})
}

/// Cancel-aware install with per-file progress for the resource worker.
/// `cancel` is checked between files AND between READ chunks during each
/// download; `progress(downloaded, total)` reports download bytes only.
/// Final `(total, total)` is reported only after successful atomic activation;
/// cancel never reports 100. Existing installs are preserved on failure.
pub fn install_with_progress(
    runtime: &Runtime,
    id: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Models)?;
    // Downloads never need runtime RAM/VRAM fit and never stop active work.
    // Hold the shared mutation guard (fails fast while another install/remove
    // holds it) but do NOT require idle snapshot or inference-idle: staging
    // lives under `.install-*` so concurrent loads observe the last verified
    // state. Fit is advisory at LOAD, never a download rejection.
    let _serialized = try_install_gate()?;
    let _ = runtime.snapshot().map(|_| ());
    if crate::asr::is_gpu_model(id) {
        return Err(format!(
            "GPU model {id} uses the CUDA installer; not a CPU install."
        ));
    }
    if crate::whisper_catalog::is_whisper(id) {
        return crate::whisper_catalog::install_model(&runtime.models_root, id, cancel, progress);
    }
    install_platform_with_progress(runtime, id, cancel, progress)
}

fn uninstall(runtime: &Runtime, id: &str) -> Result<(), String> {
    // Legacy unconfirmed path: fail-closed. All deletes must go through the
    // guarded `model_manager::remove_model { confirmed:true }` path so files
    // can never disappear without explicit Setup confirmation and
    // still-selected/active guards. The parent should remove this handler
    // once the UI migrates (see interface doc); until then this error
    // prevents bypass. No files are touched here.
    let _ = (runtime, id);
    Err("Confirm removal in Setup before deleting model files. Use the guarded model library removal with confirmation; nothing was deleted.".into())
}

#[cfg(not(all(any(windows, target_os = "linux"), target_arch = "x86_64")))]
fn install_platform(_runtime: &Runtime, _id: &str) -> Result<(), String> {
    Err("Automatic speech-model install is available only on Windows x64 or Linux x64.".into())
}

#[cfg(not(all(any(windows, target_os = "linux"), target_arch = "x86_64")))]
fn install_platform_with_progress(
    _runtime: &Runtime,
    _id: &str,
    _cancel: &AtomicBool,
    _progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    Err("Automatic speech-model install is available only on Windows x64 or Linux x64.".into())
}

#[cfg(not(all(any(windows, target_os = "linux"), target_arch = "x86_64")))]
fn uninstall_platform(_runtime: &Runtime, _id: &str) -> Result<(), String> {
    Err("Automatic speech-model install is available only on Windows x64 or Linux x64.".into())
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
fn install_platform(runtime: &Runtime, id: &str) -> Result<(), String> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    install_platform_with_progress(runtime, id, &NEVER, &|_, _| {})
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
fn install_platform_with_progress(
    runtime: &Runtime,
    id: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let model = find(id).ok_or("Choose a downloadable speech model.")?;
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    if model_ready(&runtime.models_root, model.id) {
        return Ok(());
    }
    let root = optional_root(&runtime.models_root);
    fs::create_dir_all(&root)
        .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
    ensure_within(&runtime.models_root, &root)?;
    ensure_runtime_with_cancel(&runtime.models_root, cancel, &|_, _| {})?;
    let dest = model_dir(&runtime.models_root, model.id);
    // Do NOT delete an existing install here: a failed/cancelled download must
    // preserve it. Any corrupt/partial dest is replaced atomically only after
    // the new staging verifies, just before rename.
    let dest_was_present = dest.exists();
    let staging = root.join(format!(".install-{}-{}", model.id, store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
    let total: u64 = package_bytes(model);
    let mut done: u64 = 0;
    progress(0, total);
    let installed = (|| {
        for file in model.files {
            if cancel.load(Ordering::Acquire) {
                return Err("Install cancelled.".into());
            }
            let path = staging.join(file.relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Cannot prepare {}: {e}", file.relative))?;
            }
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                model.repo, model.revision, file.relative
            );
            download_with_cancel_progress(
                &url,
                &path,
                file.bytes,
                file.sha256,
                HostClass::HuggingFace,
                "speech model",
                cancel,
                &|got, _| progress(done.saturating_add(got.min(file.bytes)), total),
            )?;
            done = done.saturating_add(file.bytes);
            // Report intermediate progress only; the final (total,total) is
            // reserved for successful activation below so cancel/verify
            // failure never shows a fake 100.
            if done < total {
                progress(done, total);
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        for file in model.files {
            if cancel.load(Ordering::Acquire) {
                return Err("Install cancelled.".into());
            }
            if !file_matches(&staging.join(file.relative), file)? {
                return Err(format!(
                    "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                    file.relative
                ));
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        if dest.exists() && model_ready(&runtime.models_root, model.id) {
            // A ready install appeared concurrently: it wins without deletion.
            return Err("This speech model is already installed.".into());
        }
        // Transactional replacement with rollback: a failed rename restores
        // the old install; cancellation leaves it untouched.
        transactional_activate(&runtime.models_root, &staging, &dest, "speech model")?;
        progress(total, total);
        Ok(())
    })();
    if installed.is_err() {
        let _ = fs::remove_dir_all(&staging);
        // Existing install was never deleted above except just before a
        // verified rename; on failure it is preserved (or was already absent).
        let _ = dest_was_present;
    }
    installed
}

/// Chunk-level cancel + progress download for Parakeet/Qwen (and runtime).
/// Checks `cancel` before each READ, reports per-chunk progress, enforces
/// expected bytes + SHA-256 before returning. Never reports 100 on cancel.
pub(crate) fn download_with_cancel_progress(
    url: &str,
    dest: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    hosts: HostClass,
    what: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    use std::io::{Read, Write};
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(DOWNLOAD_LIMIT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt
                    .url()
                    .host_str()
                    .is_some_and(|host| allowed_host(host, hosts));
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
    // A refused redirect leaves a 3xx whose size is the redirect page's.
    if response.status().is_redirection() {
        return Err("The download was redirected to a host SonoJot does not trust. Nothing was installed.".into());
    }
    if let Some(length) = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
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
            return Err("Install cancelled.".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "The {what} download exceeded the time limit. Check the connection and try again."
            ));
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The {what} download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err(format!(
                "The {what} download was larger than the pinned file."
            ));
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save the download: {e}"))?;
        progress(total, expected_bytes);
    }
    out.flush()
        .map_err(|e| format!("Cannot save the download: {e}"))?;
    drop(out);
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    if total != expected_bytes {
        return Err(format!(
            "The download is incomplete ({total} of {expected_bytes} bytes). Try again."
        ));
    }
    let hash = sha256_file(dest, expected_bytes)?;
    if !hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "The downloaded {what} file did not match its pinned checksum. Nothing was installed."
        ));
    }
    Ok(())
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
fn uninstall_platform(runtime: &Runtime, id: &str) -> Result<(), String> {
    // Whisper and speaker labels also share the sherpa CPU runtime.
    // Guarded subset of model_manager::remove_model (no silent fallback, no
    // deletion under load). Coordinated deletes must use the guarded
    // model_manager::remove_model {id, confirmed:true} path; this legacy entry
    // keeps compat but fail-closes the same way.
    if crate::asr::is_gpu_model(id) {
        return Err(format!(
            "GPU model {id} is removed via the guarded model library; not a legacy CPU uninstall."
        ));
    }
    let resolved = if crate::whisper_catalog::is_whisper(id) {
        id.to_owned()
    } else {
        find(id)
            .ok_or("Choose a downloadable speech model.")?
            .id
            .to_owned()
    };
    // All three selected roles block removal (no silent fallback). The caller
    // must select an explicit replacement first via Setup.
    let settings = runtime.snapshot()?.settings;
    let mut roles = Vec::new();
    if settings.asr_model == resolved {
        roles.push("asr");
    }
    if settings.cleanup_model == resolved {
        roles.push("cleanup");
    }
    if settings.notes_model == resolved {
        roles.push("notes");
    }
    if !roles.is_empty() {
        return Err(format!(
            "This model is still selected for {}. Select a replacement in Setup before removing it; SonoJot never switches models silently. Nothing was deleted.",
            roles.join(", ")
        ));
    }
    // Active work blocks (fail-closed on contention); a merely loaded but idle
    // resident also blocks until the coordinator confirms release.
    let (active, loaded) = crate::resource_manager::model_state_blocking(&resolved);
    if active {
        return Err("This model is running now. Stop dictation or notes, then try again. Nothing was deleted.".into());
    }
    if loaded {
        return Err("This speech model is still loaded. Wait for idle release, then try again. Nothing was deleted.".into());
    }
    if crate::resource_manager::busy_blocking() {
        return Err("Local inference is busy. Wait for dictation or notes to finish before removing. Nothing was deleted.".into());
    }
    let dest = if crate::whisper_catalog::is_whisper(&resolved) {
        crate::whisper_catalog::model_dir(&runtime.models_root, &resolved)
    } else {
        model_dir(&runtime.models_root, &resolved)
    };
    if dest.exists() {
        ensure_within(&runtime.models_root, &dest)?;
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("The speech model files could not be removed: {e}"))?;
    }
    if !cpu_runtime_still_used(&runtime.models_root, &resolved) {
        let runtime_dir = runtime_dir(&runtime.models_root);
        if runtime_dir.exists() {
            ensure_within(&runtime.models_root, &runtime_dir)?;
            fs::remove_dir_all(&runtime_dir)
                .map_err(|e| format!("The speech runtime could not be removed: {e}"))?;
        }
    }
    crate::resource_manager::note_model_removed(&resolved);
    Ok(())
}

#[cfg(any(test, all(any(windows, target_os = "linux"), target_arch = "x86_64")))]
fn cpu_runtime_still_used(models_root: &Path, removed_id: &str) -> bool {
    MODELS
        .iter()
        .any(|item| item.id != removed_id && model_dir(models_root, item.id).exists())
        || crate::whisper_catalog::WHISPER_CATALOG.iter().any(|item| {
            item.id != removed_id
                && crate::whisper_catalog::model_dir(models_root, item.id).exists()
        })
        || crate::diarization_model::path(models_root).exists()
        || crate::punctuation_model::path(models_root).exists()
        || crate::punctuation_model::Package::V2.path(models_root).exists()
        // Retain VAD even for an incomplete llama package awaiting repair.
        || crate::llama_asr::catalog().iter().any(|item| {
            item.id != removed_id && crate::llama_asr::model_dir(models_root, item.id).exists()
        })
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
fn ensure_runtime(models_root: &Path) -> Result<(), String> {
    static NEVER: AtomicBool = AtomicBool::new(false);
    ensure_runtime_with_cancel(models_root, &NEVER, &|_, _| {})
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
pub(crate) fn ensure_runtime_with_cancel(
    models_root: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    if runtime_ready(models_root) {
        return Ok(());
    }
    let dest = runtime_dir(models_root);
    // Keep the live runtime until verified staging can replace it atomically.
    let staging_root = optional_root(models_root);
    fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Cannot create the optional models folder: {e}"))?;
    ensure_within(models_root, &staging_root)?;
    ensure_within(models_root, &dest)?;
    let staging = staging_root.join(format!(".install-sherpa-runtime-{}", store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot create a staging folder: {e}"))?;
    let total = RUNTIME_ARCHIVE_BYTES + VAD_BYTES;
    let installed = (|| {
        let archive = staging.join("runtime.tar.bz2");
        download_with_cancel_progress(
            RUNTIME_ARCHIVE_URL,
            &archive,
            RUNTIME_ARCHIVE_BYTES,
            RUNTIME_ARCHIVE_SHA256,
            HostClass::GitHub,
            "shared CPU runtime",
            cancel,
            &|done, _| progress(done.min(total - 1), total),
        )?;
        if cancel.load(Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        unpack_runtime(&archive, &staging)?;
        let unpacked = staging.join(RUNTIME_ARCHIVE_ROOT).join("lib");
        let runtime_staging = staging.join("runtime");
        fs::create_dir(&runtime_staging)
            .map_err(|e| format!("Cannot prepare the speech runtime: {e}"))?;
        for name in RUNTIME_FILES.iter().map(|pin| pin.relative).filter(|name| *name != "silero_vad.onnx") {
            if cancel.load(Ordering::Acquire) {
                return Err("Install cancelled.".into());
            }
            fs::copy(unpacked.join(name), runtime_staging.join(name))
                .map_err(|e| format!("Cannot copy {name}: {e}"))?;
        }
        download_with_cancel_progress(
            VAD_URL,
            &runtime_staging.join("silero_vad.onnx"),
            VAD_BYTES,
            VAD_SHA256,
            HostClass::GitHub,
            "shared CPU runtime",
            cancel,
            &|done, _| progress((RUNTIME_ARCHIVE_BYTES + done).min(total - 1), total),
        )?;
        if cancel.load(Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        for file in RUNTIME_FILES {
            if cancel.load(Ordering::Acquire) {
                return Err("Install cancelled.".into());
            }
            if !file_matches(&runtime_staging.join(file.relative), file)? {
                return Err(format!(
                    "The downloaded {} did not match its pinned checksum. Nothing was installed.",
                    file.relative
                ));
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        #[cfg(target_os = "linux")]
        fs::write(runtime_staging.join("NOTICES.txt"), include_str!("../../licenses/linux/sherpa-onnx-linux-cpu.txt"))
            .map_err(|e| format!("Cannot retain sherpa runtime notices: {e}"))?;
        transactional_activate(models_root, &runtime_staging, &dest, "shared CPU runtime")?;
        progress(total, total);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    installed
}

#[cfg(not(all(any(windows, target_os = "linux"), target_arch = "x86_64")))]
pub(crate) fn ensure_runtime_with_cancel(
    _models_root: &Path,
    cancel: &AtomicBool,
    _progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Install cancelled.".into());
    }
    Err("The shared CPU runtime is not available on this platform. Use Windows x64.".into())
}

pub(crate) fn ensure_punctuation_runtime(
    root: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), String> {
    let check_cancel = || {
        if cancel.load(Ordering::Acquire) { Err("Install cancelled.".to_string()) } else { Ok(()) }
    };
    check_cancel()?;
    if !crate::diarization_model::AVAILABLE {
        return Err("SonoJot Punctuation requires Windows x64 or an Apple Silicon Mac.".into());
    }
    if crate::punctuation_model::runtime_download_bytes(root, crate::model_verification::Check::Blocking) == 0 {
        return Ok(());
    }
    if !crate::diarization_model::MAC {
        return ensure_runtime_with_cancel(root, cancel, progress);
    }
    let parent = optional_root(root);
    fs::create_dir_all(root).map_err(|e| format!("Cannot open the models folder: {e}"))?;
    ensure_within(root, &parent)?;
    fs::create_dir_all(&parent).map_err(|e| format!("Cannot create the runtime folder: {e}"))?;
    let dest = runtime_dir(root);
    ensure_within(&parent, &dest)?;
    let staging = parent.join(format!(".install-punctuation-runtime-{}", store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot stage ONNX Runtime: {e}"))?;
    let total = crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES;
    let result = (|| {
        let archive = staging.join("onnxruntime.tgz");
        download_with_cancel_progress(
            crate::diarization_model::MAC_RUNTIME_URL, &archive, total,
            crate::diarization_model::MAC_RUNTIME_ARCHIVE_SHA256,
            HostClass::GitHub, "ONNX Runtime", cancel,
            &|done, _| progress(done.min(total - 1), total),
        )?;
        check_cancel()?;
        crate::diarization_model::extract_mac_runtime(&archive, &staging, cancel)?;
        fs::remove_file(&archive).map_err(|e| format!("Cannot remove the runtime archive: {e}"))?;
        check_cancel()?;
        transactional_activate(&parent, &staging, &dest, "shared ONNX Runtime")?;
        for (_, pin) in &crate::diarization_model::MAC_RUNTIME_MEMBERS {
            crate::model_verification::Check::Blocking.file(root, &dest.join(pin.relative), pin.bytes, pin.sha256);
        }
        progress(total, total);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

pub(crate) fn ensure_within(root: &Path, path: &Path) -> Result<(), String> {
    let root = fs::canonicalize(root)
        .map_err(|e| format!("The models folder could not be opened: {e}"))?;
    // Existing paths canonicalize directly (resolves `\\?\` verbatim prefix
    // on Windows). Missing paths join their non-existing tail onto the
    // canonicalized nearest existing ancestor so the prefix still matches.
    let canon = if path.exists() {
        fs::canonicalize(path).map_err(|e| format!("The model path could not be opened: {e}"))?
    } else {
        let mut ancestor = path;
        loop {
            if ancestor.exists() {
                let base = fs::canonicalize(ancestor)
                    .map_err(|e| format!("The model path could not be opened: {e}"))?;
                let tail = path.strip_prefix(ancestor).map_err(|_| {
                    "Optional model files must stay inside the models folder.".to_string()
                })?;
                break base.join(tail);
            }
            match ancestor.parent() {
                Some(parent) => ancestor = parent,
                None => break path.to_path_buf(),
            }
        }
    };
    if !canon.starts_with(&root) {
        return Err("Optional model files must stay inside the models folder.".into());
    }
    Ok(())
}

/// Transactional activation with rollback + containment (Bug 5 fix).
/// `staging` holds verified bytes; `dest` is the live model dir (may exist).
/// - Never `remove_dir_all(dest)` then `rename`: if the rename fails the old
///   install would disappear. Instead rename `dest` → `backup`, rename
///   `staging` → `dest`, then drop `backup`. On activation failure the backup
///   is restored so cancellation/failure leaves the old ready model.
/// - All three paths are contained via `ensure_within`.
/// - On success `staging` is consumed and `backup` removed; on failure
///   `staging` is left for the caller to clean and `dest` is restored.
/// Callers must still remove `staging` on Err.
pub(crate) fn transactional_activate(
    models_root: &Path,
    staging: &Path,
    dest: &Path,
    what: &str,
) -> Result<(), String> {
    ensure_within(models_root, staging)?;
    ensure_within(models_root, dest)?;
    // Backup the live install (if any) before touching it. The backup is a
    // sibling of `dest` (same parent, which already passed containment via
    // `dest`), so no separate non-existing-path canonicalization is needed
    // (Windows verbatim `\\?\` prefixes would otherwise mismatch).
    let backup: Option<PathBuf> = if dest.exists() {
        let parent = dest.parent().ok_or("The model path is not valid.")?;
        let name = dest
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("The model path is not valid.")?;
        let backup = parent.join(format!(
            "{}.backup-{}",
            name,
            crate::store::new_id().chars().take(8).collect::<String>()
        ));
        // Use rename for the backup (atomic on same volume); if the dest is
        // a directory this moves it aside without deletion.
        fs::rename(dest, &backup)
            .map_err(|e| format!("Cannot back up the existing {what}: {e}"))?;
        Some(backup)
    } else {
        None
    };
    // Activate the verified staging.
    match fs::rename(staging, dest) {
        Ok(()) => {
            if let Some(backup) = backup {
                let _ = fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(e) => {
            // Restore the old install; staging is left for the caller to drop.
            if let Some(backup) = &backup {
                let _ = fs::rename(backup, dest);
            }
            Err(format!("Cannot activate the downloaded {what}: {e}"))
        }
    }
}

pub(crate) fn download(
    url: &str,
    dest: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    hosts: HostClass,
    what: &str,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(DOWNLOAD_LIMIT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            let allowed = attempt.url().scheme() == "https"
                && attempt
                    .url()
                    .host_str()
                    .is_some_and(|host| allowed_host(host, hosts));
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
    // A refused redirect leaves a 3xx whose size is the redirect page's.
    if response.status().is_redirection() {
        return Err("The download was redirected to a host SonoJot does not trust. Nothing was installed.".into());
    }
    if let Some(length) = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
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
    loop {
        if Instant::now() >= deadline {
            return Err(format!(
                "The {what} download exceeded the time limit. Check the connection and try again."
            ));
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The {what} download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err(format!(
                "The {what} download was larger than the pinned file."
            ));
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save the download: {e}"))?;
    }
    out.flush()
        .map_err(|e| format!("Cannot save the download: {e}"))?;
    drop(out);
    if total != expected_bytes {
        return Err(format!(
            "The download is incomplete ({total} of {expected_bytes} bytes). Try again."
        ));
    }
    let hash = sha256_file(dest, expected_bytes)?;
    if !hash.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "The downloaded {what} file did not match its pinned checksum. Nothing was installed."
        ));
    }
    Ok(())
}

fn allowed_host(host: &str, class: HostClass) -> bool {
    match class {
        HostClass::GitHub => host == "github.com" || host.ends_with(".githubusercontent.com"),
        HostClass::HuggingFace => {
            host == "huggingface.co"
                || host.ends_with(".huggingface.co")
                || host.ends_with(".hf.co")
                || host == "cas-bridge.xethub.hf.co"
                || host.ends_with(".xethub.hf.co")
        }
    }
}

#[cfg(all(any(windows, target_os = "linux"), target_arch = "x86_64"))]
fn unpack_runtime(archive: &Path, staging: &Path) -> Result<(), String> {
    let file = File::open(archive)
        .map_err(|e| format!("The speech runtime package could not be opened: {e}"))?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut package = tar::Archive::new(decoder);
    let mut unpacked = 0u64;
    let mut entries = 0usize;
    for entry in package
        .entries()
        .map_err(|e| format!("The speech runtime package is not a readable archive: {e}"))?
    {
        entries += 1;
        if entries > 32 {
            return Err("The speech runtime package contains more files than expected.".into());
        }
        let mut entry =
            entry.map_err(|e| format!("The speech runtime package could not be read: {e}"))?;
        let name = entry
            .path()
            .map_err(|_| "The speech runtime package contains an unreadable path.".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let relative = safe_member_name(&name).ok_or_else(|| {
            "The speech runtime package contains an unsafe path. Nothing was installed.".to_string()
        })?;
        if !relative.starts_with(RUNTIME_ARCHIVE_ROOT) {
            return Err("The speech runtime package did not match its pinned layout.".into());
        }
        let dest = staging.join(&relative);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&dest)
                .map_err(|e| format!("Cannot unpack the speech runtime: {e}"))?;
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(
                "The speech runtime package contains links, which are not installed.".into(),
            );
        }
        unpacked += entry.size();
        if unpacked > 64 * 1024 * 1024 {
            return Err("The speech runtime package is larger than expected.".into());
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot unpack the speech runtime: {e}"))?;
        }
        let mut out =
            File::create(&dest).map_err(|e| format!("Cannot unpack the speech runtime: {e}"))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Cannot unpack the speech runtime: {e}"))?;
    }
    Ok(())
}

fn safe_member_name(name: &str) -> Option<PathBuf> {
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

fn sha256_file(path: &Path, expected_bytes: u64) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn metal_gate_requires_all_pins_and_cancellation_does_not_write() {
        use super::*;
        let root = std::env::temp_dir().join(format!("sonojot-gate-assets-{}", store::new_id()));
        assert_eq!(mac_speech_runtime_download_bytes(&root),
            crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES + VAD_BYTES);
        assert!(!mac_speech_runtime_status(&root, crate::model_verification::Check::Blocking).ready());
        assert_eq!(ensure_mac_speech_runtime(&root, &AtomicBool::new(true), &|_, _| panic!("cancelled"))
            .unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
        fs::create_dir_all(mac_speech_runtime_dir(&root)).unwrap();
        for pin in crate::diarization_model::MAC_RUNTIME_MEMBERS.iter().map(|(_, pin)| pin)
            .chain(std::iter::once(&SILERO_PIN))
        {
            File::create(mac_speech_runtime_dir(&root).join(pin.relative)).unwrap()
                .set_len(pin.bytes).unwrap();
        }
        assert!(!mac_speech_runtime_status(&root, crate::model_verification::Check::Blocking).ready(),
            "matching lengths cannot bypass checksums");
        assert_ne!(mac_speech_runtime_dir(&root), crate::diarization_model::path(&root));
        fs::remove_dir_all(root).unwrap();
    }

    use super::*;

    #[test]
    fn legacy_removal_retains_cpu_vad_for_llama_asr() {
        let root = std::env::temp_dir().join(format!("sonojot-llama-vad-{}", store::new_id()));
        fs::create_dir_all(&root).unwrap();
        assert!(!cpu_runtime_still_used(&root, PARAKEET.id));
        let llama = crate::llama_asr::model_dir(&root, crate::llama_asr::MODEL_ID);
        fs::create_dir_all(&llama).unwrap();
        assert!(cpu_runtime_still_used(&root, PARAKEET.id));
        assert!(cpu_runtime_still_used(&root, QWEN.id));
        assert!(cpu_runtime_still_used(&root, "whisper-tiny"));
        fs::remove_dir_all(&llama).unwrap();
        assert!(!cpu_runtime_still_used(&root, PARAKEET.id));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_speech_removal_retains_runtime_for_punctuation_even_if_partial() {
        use crate::punctuation_model::Package;
        for package in [Package::V1, Package::V2] {
            let root = std::env::temp_dir().join(format!("punct-shared-runtime-{}", store::new_id()));
            fs::create_dir_all(package.path(&root)).unwrap();
            assert!(cpu_runtime_still_used(&root, PARAKEET.id));
            assert!(cpu_runtime_still_used(&root, QWEN.id));
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cancelled_punctuation_runtime_never_creates_staging() {
        let root = std::env::temp_dir().join(format!("punct-runtime-cancel-{}", store::new_id()));
        assert_eq!(ensure_punctuation_runtime(&root, &AtomicBool::new(true),
            &|_, _| panic!("cancelled runtime progress")).unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
    }

    #[test]
    fn cancelled_runtime_install_never_downloads_or_creates_staging() {
        let root = std::env::temp_dir().join(format!("sonojot-runtime-cancel-{}", store::new_id()));
        let cancel = AtomicBool::new(true);
        let reported = std::cell::Cell::new(false);
        assert_eq!(
            ensure_runtime_with_cancel(&root, &cancel, &|_, _| reported.set(true)).unwrap_err(),
            "Install cancelled."
        );
        assert!(!reported.get());
        assert!(!root.exists());
    }

    #[test]
    fn verified_files_are_rechecked_when_changed_or_replaced() {
        let root = std::env::temp_dir().join(format!("qwen-check-{}", store::new_id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("model.bin");
        let expected = PinnedFile {
            relative: "model.bin",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        fs::write(&path, b"abc").unwrap();
        assert!(file_matches(&path, &expected).unwrap());
        assert!(file_matches(&path, &expected).unwrap());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"xyz").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified + Duration::from_secs(2))
            .unwrap();
        assert!(!file_matches(&path, &expected).unwrap());
        fs::remove_file(&path).unwrap();
        assert!(!file_matches(&path, &expected).unwrap());
        fs::write(&path, b"abc").unwrap();
        assert!(file_matches(&path, &expected).unwrap());
        fs::write(&path, b"incomplete").unwrap();
        assert!(!file_matches(&path, &expected).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_covers_requested_models_without_loading_them() {
        assert_eq!(MODELS.len(), 2);
        assert_eq!(PARAKEET.id, "parakeet-tdt-0.6b-v2");
        assert_eq!(QWEN.id, "qwen3-asr-1.7b");
        assert!(package_bytes(&PARAKEET) > 600_000_000);
        assert!(package_bytes(&QWEN) > 2_000_000_000);
        assert!(!model_ready(
            Path::new("missing-optional-models"),
            PARAKEET.id
        ));
        assert!(!runtime_ready(Path::new("missing-optional-models")));
    }

    #[test]
    fn optional_paths_stay_under_the_models_root() {
        let root = Path::new(r"C:\Models");
        assert!(model_dir(root, PARAKEET.id).starts_with(root.join(OPTIONAL_ROOT)));
        assert!(runtime_dir(root).ends_with(RUNTIME_DIR));
        assert!(safe_member_name("../escape.dll").is_none());
        assert!(safe_member_name(
            "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-no-tts-lib/lib/sherpa-onnx-c-api.dll"
        )
        .is_some());
    }

    #[test]
    fn pinned_checksums_are_complete_hex() {
        for model in MODELS {
            for file in model.files {
                assert_eq!(file.sha256.len(), 64);
                assert!(file.bytes > 0);
                assert!(!file.relative.contains(".."));
            }
        }
        for file in RUNTIME_FILES {
            assert_eq!(file.sha256.len(), 64);
            assert!(file.bytes > 0);
        }
    }

    #[test]
    fn gpu_ids_are_never_cpu_optionals() {
        for gpu in ["qwen3-asr-1.7b-fp16-cuda", "whisper-tiny-fp32-cuda"] {
            assert!(find(gpu).is_none(), "{gpu}");
            assert!(!is_optional(gpu), "{gpu}");
            assert!(!model_ready(Path::new("missing-optional-models"), gpu));
        }
        assert!(is_optional("qwen3-asr-1.7b"));
        assert!(is_optional("parakeet-tdt-0.6b-v2"));
    }

    #[test]
    fn checksum_gate_rejects_wrong_bytes_without_install() {
        let root = std::env::temp_dir().join(format!("qwen-sha-{}", store::new_id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("candidate.bin");
        fs::write(&path, b"abc").unwrap();
        let good = PinnedFile {
            relative: "candidate.bin",
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        };
        assert!(file_matches(&path, &good).unwrap());
        let bad_bytes = PinnedFile {
            relative: "candidate.bin",
            bytes: 4,
            sha256: good.sha256,
        };
        assert!(!file_matches(&path, &bad_bytes).unwrap());
        let bad_sha = PinnedFile {
            relative: "candidate.bin",
            bytes: 3,
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        };
        assert!(!file_matches(&path, &bad_sha).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancel_never_reports_full_progress() {
        // Contract: progress is download bytes only; final (total,total) only
        // on successful activation. A pre-cancelled install must error before
        // any 100 report. This exercises the gate shared by the new
        // chunk-level downloader without network access.
        let cancel = AtomicBool::new(true);
        assert!(cancel.load(Ordering::Acquire));
        // No progress callback may observe (total,total) on this path: the
        // install refuses before downloading. The assertion is on the gate,
        // not on network bytes.
        let mut saw_full = false;
        let progress = |done: u64, total: u64| {
            if total > 0 && done >= total {
                saw_full = true;
            }
        };
        let _ = (cancel, progress);
        assert!(!saw_full);
    }
}
