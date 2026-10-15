//! Pinned Linux x64 runtimes. Activation uses the model install transaction;
//! paths.json is a load override, never an installation destination.
use crate::{model_verification::{Check, Status}, optional_asr::{self, HostClass, PinnedFile}};
use std::{fs, path::Path, sync::atomic::{AtomicBool, Ordering}};

pub(crate) const AVAILABLE: bool = cfg!(all(target_os = "linux", target_arch = "x86_64"));
pub(crate) const MOONSHINE_FILES: &[PinnedFile] = &[
    PinnedFile { relative: "libmoonshine.so", bytes: 12_157_928, sha256: "9d516b03b178f01b6f986ae29a6d1b9adc8f49bb6e4d202672b22c0181cc9e00" },
    PinnedFile { relative: "libonnxruntime.so.1", bytes: 22_326_072, sha256: "13ab8084954fa4a47c777880180b90810d6020f021441395712b48a75b74c68b" },
];
const MOONSHINE_BYTES: u64 = 14_576_999;

pub(crate) fn moonshine_status(root: &Path, check: Check) -> Status {
    let library = crate::asr::runtime_path(root);
    let Some(dir) = library.parent() else { return Status::Missing };
    Status::all(MOONSHINE_FILES.iter().map(|pin| {
        let path = if pin.relative == "libmoonshine.so" { library.clone() } else { dir.join(pin.relative) };
        check.file(root, &path, pin.bytes, pin.sha256)
    }))
}

pub(crate) fn moonshine_download_bytes(root: &Path) -> u64 {
    if AVAILABLE && !moonshine_status(root, Check::Background).ready() { MOONSHINE_BYTES } else { 0 }
}

pub(crate) fn ensure_moonshine(root: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    check_cancel(cancel)?;
    if !AVAILABLE { return Err("This speech runtime requires Linux x64.".into()); }
    if moonshine_status(root, Check::Blocking).ready() { return Ok(()); }
    check_disk(root, MOONSHINE_BYTES + 64 * 1024 * 1024)?;
    let parent = root.join("moonshine/runtime");
    fs::create_dir_all(&parent).map_err(|e| format!("Cannot create speech runtime folder: {e}"))?;
    optional_asr::ensure_within(root, &parent)?;
    let dest = parent.join("linux-x64");
    optional_asr::ensure_within(root, &dest)?;
    let staging = parent.join(format!(".install-linux-{}", crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let archive = staging.join("runtime.tar.gz");
        optional_asr::download_with_cancel_progress(
            "https://github.com/moonshine-ai/moonshine/releases/download/v0.1.5/moonshine-voice-linux-x86_64.tar.gz",
            &archive, MOONSHINE_BYTES, "9c3a87fea93ff2ad957938868f95a0a366dce9ff8ad86bde6cdcf5a4cadb51df",
            HostClass::GitHub, "Moonshine Linux runtime", cancel,
            &|done, total| progress(done.min(total - 1), total),
        )?;
        unpack_gzip(&archive, &staging, cancel)?;
        let unpacked = staging.join("moonshine-voice-linux-x86_64");
        for pin in MOONSHINE_FILES {
            check_cancel(cancel)?;
            if !optional_asr::file_matches(&unpacked.join("lib").join(pin.relative), pin)? {
                return Err(format!("The downloaded {} failed verification.", pin.relative));
            }
        }
        fs::write(unpacked.join("NOTICES.txt"), include_str!("../../licenses/linux/moonshine-linux.txt"))
            .map_err(|e| format!("Cannot retain Moonshine notices: {e}"))?;
        check_cancel(cancel)?;
        optional_asr::transactional_activate(root, &unpacked, &dest, "Moonshine Linux runtime")?;
        progress(MOONSHINE_BYTES, MOONSHINE_BYTES);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) { Err("Install cancelled.".into()) } else { Ok(()) }
}

fn check_disk(root: &Path, bytes: u64) -> Result<(), String> {
    let existing = root.ancestors().find(|p| p.is_dir()).unwrap_or(root);
    if crate::resource_manager::disk_free_bytes(existing).is_some_and(|free| free < bytes.saturating_add(256 << 20)) {
        return Err("Not enough disk space for the Linux runtime. Free space and retry.".into());
    }
    Ok(())
}

fn unpack_gzip(path: &Path, staging: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        check_cancel(cancel)?;
        if !entry.map_err(|e| e.to_string())?.unpack_in(staging).map_err(|e| format!("Cannot unpack Linux runtime: {e}"))? {
            return Err("The Linux runtime archive contains an invalid path.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_runtime_cancellation_does_not_touch_disk() {
        let root = std::env::temp_dir().join(format!("sonojot-linux-cancel-{}", crate::store::new_id()));
        let cancel = AtomicBool::new(true);
        let progress = |_, _| panic!("cancelled runtime must not report progress");
        assert_eq!(ensure_moonshine(&root, &cancel, &progress).unwrap_err(), "Install cancelled.");
        assert_eq!(ensure_llama(&root, &cancel, &progress).unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
    }

    #[test]
    fn linux_llama_receipts_cover_server_and_device_libraries() {
        for vulkan in [false, true] {
            let pins = llama_files(vulkan);
            assert!(pins.iter().any(|p| p.relative == "llama-server"));
            assert!(pins.iter().any(|p| p.relative == "libllama-server-impl.so"));
            assert_eq!(pins.iter().any(|p| p.relative == "libggml-vulkan.so"), vulkan);
            assert!(pins.iter().all(|p| p.bytes > 0 && p.sha256.len() == 64 && !p.relative.contains('/')));
        }
    }
}

#[derive(serde::Deserialize)]
struct Identity<'a> { relative: &'a str, bytes: u64, sha256: &'a str }
#[derive(serde::Deserialize)]
struct LlamaIdentities<'a> {
    #[serde(borrow)]
    cpu: Vec<Identity<'a>>,
    #[serde(borrow)]
    vulkan: Vec<Identity<'a>>,
}

fn llama_files(vulkan: bool) -> &'static [Identity<'static>] {
    static PINS: std::sync::OnceLock<LlamaIdentities<'static>> = std::sync::OnceLock::new();
    let pins = PINS.get_or_init(|| serde_json::from_str(include_str!("../assets/linux-llama-identities.json"))
        .expect("embedded Linux llama.cpp identities"));
    if vulkan { &pins.vulkan } else { &pins.cpu }
}

pub(crate) fn llama_status(root: &Path, vulkan: bool, check: Check) -> Status {
    if !AVAILABLE { return Status::Missing; }
    let executable = if vulkan { crate::cleanup::vulkan_server_path(root) } else { crate::cleanup::server_path(root) };
    let Some(dir) = executable.parent() else { return Status::Missing };
    if !fs::read_to_string(dir.join("sonojot-runtime-release")).is_ok_and(|s| s.trim() == "b10964") {
        return Status::Missing;
    }
    Status::all(llama_files(vulkan).iter().map(|pin| {
        let path = if pin.relative == "llama-server" { executable.clone() } else { dir.join(pin.relative) };
        check.file(root, &path, pin.bytes, pin.sha256)
    }))
}

fn llama_bytes(vulkan: bool) -> u64 { if vulkan { 30_166_472 } else { 16_825_086 } }

pub(crate) fn llama_download_bytes(root: &Path) -> u64 {
    if !AVAILABLE { return 0; }
    [false, true].into_iter().filter(|&v| !llama_status(root, v, Check::Background).ready()).map(llama_bytes).sum()
}

pub(crate) fn ensure_llama(root: &Path, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    check_cancel(cancel)?;
    if !AVAILABLE { return Err("This model runtime requires Linux x64.".into()); }
    let missing: Vec<_> = [false, true].into_iter().filter(|&v| !llama_status(root, v, Check::Blocking).ready()).collect();
    let total = missing.iter().map(|&v| llama_bytes(v)).sum();
    if !missing.is_empty() { check_disk(root, total + 256 * 1024 * 1024)?; }
    let mut done = 0;
    for vulkan in missing {
        install_llama_variant(root, vulkan, cancel, &|n, _| progress((done + n).min(total - 1), total))?;
        done += llama_bytes(vulkan);
    }
    progress(total, total);
    Ok(())
}

fn install_llama_variant(root: &Path, vulkan: bool, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), String> {
    let (variant, sha) = if vulkan {
        ("ubuntu-vulkan-x64", "55d1e58e14c11eedea090bf088fdeefbfe7b4b09ee03bf6dba9834651769afcf")
    } else {
        ("ubuntu-x64", "9abf88aea48a55d0f80edb1ee20220b186848cca0b4e919d71518cfd7ca67443")
    };
    let parent = root.join("llama.cpp/runtime");
    fs::create_dir_all(&parent).map_err(|e| format!("Cannot create Linux runtime folder: {e}"))?;
    optional_asr::ensure_within(root, &parent)?;
    let dest = parent.join(if vulkan { "linux-x64-vulkan" } else { "linux-x64" });
    optional_asr::ensure_within(root, &dest)?;
    let staging = parent.join(format!(".install-linux-{}", crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let archive = staging.join("runtime.tar.gz");
        optional_asr::download_with_cancel_progress(
            &format!("https://github.com/ggml-org/llama.cpp/releases/download/b10964/llama-b10964-bin-{variant}.tar.gz"),
            &archive, llama_bytes(vulkan), sha, HostClass::GitHub, "llama.cpp Linux runtime", cancel, progress,
        )?;
        unpack_gzip(&archive, &staging, cancel)?;
        let unpacked = staging.join("llama-b10964");
        for pin in llama_files(vulkan) {
            check_cancel(cancel)?;
            optional_asr::ensure_within(&unpacked, &unpacked.join(pin.relative))?;
            if !crate::model_verification::uncached(&unpacked.join(pin.relative), pin.bytes, pin.sha256)? {
                return Err(format!("The downloaded {} failed verification.", pin.relative));
            }
        }
        fs::write(unpacked.join("NOTICES.txt"), include_str!("../../licenses/linux/llama-cpp-linux.txt"))
            .map_err(|e| format!("Cannot retain llama.cpp notices: {e}"))?;
        fs::write(unpacked.join("sonojot-runtime-release"), "b10964\n").map_err(|e| e.to_string())?;
        check_cancel(cancel)?;
        optional_asr::transactional_activate(root, &unpacked, &dest, "llama.cpp Linux runtime")
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}
