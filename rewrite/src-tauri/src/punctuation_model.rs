//! Versioned punctuation packages, with legacy local-manifest support.
//!
//! `manifest.json` is UTF-8 JSON, with exactly these three file records:
//! ```text
//! {"schema":1,"id":"sonojot-punct-v1","files":[
//!   {"name":"model.int8.onnx","bytes":123,"sha256":"<64 hex digits>"},
//!   {"name":"tokenizer.json","bytes":456,"sha256":"<64 hex digits>"},
//!   {"name":"punct-config.json","bytes":789,"sha256":"<64 hex digits>"}
//! ]}
//! ```
//! Both `sonojot-punct-v1` and `sonojot-punct-v2` use schema 1. The manifest ID
//! must match the package folder. Existing manifests retain their own
//! sizes/hashes; downloads use release pins.
//! A local manifest verifies package integrity, not a signed publisher
//! identity. The development override bypasses this manifest, but the engine
//! still parses its tokenizer/config/graph and verifies the shared runtime pin.

use crate::model_verification::{Check, Status};
use crate::optional_asr::{self, PinnedFile};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) const ID: &str = "sonojot-punct-v1";
pub(crate) const REPO: &str = "SonoJot/sonojot-punct-v1";
// Public release commit containing all three runtime files and their manifest.
pub(crate) const REVISION: &str = "6634a9884cf7bd68663c0eefb4840f4de882e280";
pub(crate) const V2_ID: &str = "sonojot-punct-v2";
/// Set only to the real Hugging Face upload commit after publication is approved.
/// None keeps v2 local-only, including direct install requests.
pub(crate) const V2_RELEASE_COMMIT: Option<&str> = Some("94d84b4b17d4d2fb3cee46dbd7a4425b8a6ed625");
pub(crate) const FILES: [&str; 3] = ["model.int8.onnx", "tokenizer.json", "punct-config.json"];
/// Planning estimate for the warm CPU helper, which is prewarmed while speech
/// records. The library shows the same figure; it is not a measured peak.
pub(crate) const RAM_ESTIMATE_BYTES: u64 = 400_000_000;
pub(crate) const PINS: &[PinnedFile] = &[
    PinnedFile {
        relative: "model.int8.onnx",
        bytes: 268_344_270,
        sha256: "6dccbb7d461407bfa9a315e0551b8a2db6ea31521f6e493180fec0a30e891168",
    },
    PinnedFile {
        relative: "tokenizer.json",
        bytes: 34_363_188,
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    PinnedFile {
        relative: "punct-config.json",
        bytes: 799,
        sha256: "489026321fa6f54db2ae6c1d0fc91da981d352ec7d5f078e3d1c56edaa1856e3",
    },
];
// From the selected small-runtime package's manifest, October 8, 2026.
pub(crate) const V2_PINS: &[PinnedFile] = &[
    PinnedFile {
        relative: "model.int8.onnx",
        bytes: 268_364_003,
        sha256: "02b5cbbe66e9456fb282b0663a42937d6cdbbb4780667be44caab803118446a6",
    },
    PinnedFile {
        relative: "tokenizer.json",
        bytes: 34_363_188,
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    PinnedFile {
        relative: "punct-config.json",
        bytes: 1_763,
        sha256: "3dd789e72775cd7d8d739f687da7f1ee4ad382758aba044a64c8afd61083ea6b",
    },
];
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Package {
    V1,
    V2,
}

impl Package {
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id {
            ID => Some(Self::V1),
            V2_ID => Some(Self::V2),
            _ => None,
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self { Self::V1 => ID, Self::V2 => V2_ID }
    }

    pub(crate) fn page_url(self) -> &'static str {
        match self {
            Self::V1 => "https://huggingface.co/SonoJot/sonojot-punct-v1",
            Self::V2 => "https://huggingface.co/SonoJot/sonojot-punct-v2",
        }
    }

    pub(crate) fn path(self, root: &Path) -> PathBuf {
        root.join("punctuation").join(self.id())
    }

    pub(crate) fn revision(self) -> Option<&'static str> {
        let revision = match self {
            Self::V1 => Some(REVISION),
            Self::V2 => V2_RELEASE_COMMIT,
        };
        revision.filter(|value| value.len() == 40 && value.bytes().all(|ch| ch.is_ascii_hexdigit()))
    }

    pub(crate) fn released(self) -> bool { self.revision().is_some() }

    pub(crate) fn pins(self) -> &'static [PinnedFile] {
        match self { Self::V1 => PINS, Self::V2 => V2_PINS }
    }

    pub(crate) fn pinned_bytes(self) -> u64 {
        self.pins().iter().map(|file| file.bytes).sum()
    }

    pub(crate) fn download_bytes(self, root: &Path, check: Check) -> u64 {
        self.pinned_bytes() + runtime_download_bytes(root, check)
    }

    pub(crate) fn download_url(self, file: &PinnedFile) -> Result<String, String> {
        let revision = self.revision().ok_or_else(|| self.unreleased_error())?;
        let repo = match self { Self::V1 => REPO, Self::V2 => "SonoJot/sonojot-punct-v2" };
        Ok(format!("https://huggingface.co/{repo}/resolve/{revision}/{}", file.relative))
    }

    pub(crate) fn unreleased_error(self) -> String {
        let name = match self { Self::V1 => "SonoJot Punctuation", Self::V2 => "SonoJot Punctuation v2" };
        format!("{name} is not published yet. Its download is unavailable until the upload is complete.")
    }

    pub(crate) fn package_bytes(self, root: &Path) -> u64 {
        manifest_for(root, self).map_or(0, |manifest| {
            manifest.files.iter().map(|file| file.bytes).sum()
        })
    }

    fn files_status(self, root: &Path, check: Check) -> Status {
        let Some(manifest) = manifest_for(root, self) else {
            return Status::Missing;
        };
        let directory = self.path(root);
        Status::all(manifest.files.iter().map(|file| {
            check.file(root, &directory.join(&file.name), file.bytes, &file.sha256.to_ascii_lowercase())
        }))
    }

    pub(crate) fn model_status(self, root: &Path, check: Check) -> Status {
        if !crate::diarization_model::AVAILABLE { return Status::Missing; }
        Status::all([self.files_status(root, check), runtime_status(root, check)])
    }
}

/// Background selection never hashes on the caller's thread. A worker may use
/// Blocking to select on first load. None means neither package is ready.
pub(crate) fn selected_path(root: &Path, check: Check) -> Option<PathBuf> {
    selected_package(root, check).map(|package| package.path(root))
}

/// Use the same verified package for inference and its model-page link.
pub(crate) fn selected_package(root: &Path, check: Check) -> Option<Package> {
    if !crate::diarization_model::AVAILABLE { return None; }
    selected_package_with_runtime(root, check, runtime_status(root, check))
}

fn selected_package_with_runtime(root: &Path, check: Check, runtime: Status) -> Option<Package> {
    [Package::V2, Package::V1].into_iter().find(|package| {
        Status::all([package.files_status(root, check), runtime]).ready()
    })
}

/// Verify the exact selected directory, rather than another installed version.
pub(crate) fn status_at(root: &Path, directory: &Path, check: Check) -> Status {
    [Package::V2, Package::V1].into_iter()
        .find(|package| package.path(root) == directory)
        .map_or(Status::Missing, |package| package.model_status(root, check))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    id: String,
    files: Vec<FileRecord>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    name: String,
    bytes: u64,
    sha256: String,
}

pub(crate) fn path(root: &Path) -> PathBuf {
    Package::V1.path(root)
}

#[cfg(test)]
pub(crate) fn released() -> bool {
    Package::V1.released()
}

#[cfg(test)]
pub(crate) fn download_url(file: &PinnedFile) -> String {
    format!("https://huggingface.co/{REPO}/resolve/{REVISION}/{}", file.relative)
}

#[cfg(test)]
pub(crate) fn pinned_bytes() -> u64 {
    PINS.iter().map(|file| file.bytes).sum()
}

pub(crate) fn runtime_download_bytes(root: &Path, check: Check) -> u64 {
    let status = if crate::diarization_model::MAC {
        check.files(root, &optional_asr::runtime_dir(root),
            &crate::diarization_model::MAC_RUNTIME_MEMBERS.map(|(_, pin)| pin))
    } else {
        runtime_status(root, check)
    };
    if status.ready() {
        0
    } else if crate::diarization_model::MAC {
        crate::diarization_model::MAC_RUNTIME_ARCHIVE_BYTES
    } else {
        optional_asr::runtime_download_bytes(root)
    }
}

#[cfg(test)]
pub(crate) fn download_bytes(root: &Path, check: Check) -> u64 {
    pinned_bytes() + runtime_download_bytes(root, check)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Install cancelled.".into())
    } else {
        Ok(())
    }
}

/// Runtime preparation is shared with speech on Windows. On Apple Silicon the
/// same pinned Microsoft archive is staged into optional/sherpa-runtime, without
/// requiring a speaker model or removing an older package-owned runtime.
#[cfg(test)]
pub(crate) fn install_model(
    root: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    verifying: &dyn Fn(),
) -> Result<(), String> {
    install_package(root, Package::V1, cancel, progress, verifying)
}

pub(crate) fn install_package(
    root: &Path,
    package: Package,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    verifying: &dyn Fn(),
) -> Result<(), String> {
    check_cancel(cancel)?;
    if !package.released() {
        return Err(package.unreleased_error());
    }
    if !crate::diarization_model::AVAILABLE {
        return Err("SonoJot Punctuation requires Windows x64 or an Apple Silicon Mac.".into());
    }
    let total = package.download_bytes(root, Check::Blocking);
    let runtime_bytes = total - package.pinned_bytes();
    let disk_path = root.ancestors().find(|path| path.is_dir()).unwrap_or(root);
    if crate::resource_manager::disk_free_bytes(disk_path)
        .is_some_and(|free| free < total.saturating_add(256 << 20))
    {
        return Err("Not enough disk space for punctuation and its CPU runtime. Free space and try again.".into());
    }
    progress(0, total);
    if runtime_bytes > 0 {
        optional_asr::ensure_punctuation_runtime(root, cancel, &|done, _| {
            progress(done.min(runtime_bytes), total);
        })?;
    }
    check_cancel(cancel)?;
    if !runtime_status(root, Check::Blocking).ready() {
        return Err("The shared ONNX Runtime is missing or damaged. Retry the punctuation installation.".into());
    }
    // Repairing only a missing runtime preserves older, valid local packages.
    if package.model_status(root, Check::Blocking).ready() {
        progress(total, total);
        return Ok(());
    }
    stage_and_activate(root, package, package.pins(), cancel, &|file, dest, done| {
        optional_asr::download_with_cancel_progress(
            &package.download_url(file)?, dest, file.bytes, file.sha256,
            optional_asr::HostClass::HuggingFace, "punctuation model", cancel,
            &|bytes, _| progress((runtime_bytes + done + bytes).min(total - 1), total),
        )
    }, verifying)?;
    progress(total, total);
    Ok(())
}

/// The injected transport lets tests exercise the publication boundary with
/// small files. Production supplies the package's pins and verified HTTPS helper.
fn stage_and_activate(
    root: &Path,
    package: Package,
    pins: &[PinnedFile],
    cancel: &AtomicBool,
    download: &dyn Fn(&PinnedFile, &Path, u64) -> Result<(), String>,
    verifying: &dyn Fn(),
) -> Result<(), String> {
    check_cancel(cancel)?;
    fs::create_dir_all(root).map_err(|e| format!("Cannot open the models folder: {e}"))?;
    let dest = package.path(root);
    let parent = dest.parent().ok_or("The punctuation model path is invalid.")?;
    optional_asr::ensure_within(root, parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create the punctuation folder: {e}"))?;
    optional_asr::ensure_within(parent, &dest)?;
    let staging = parent.join(format!(".install-{}-{}", package.id(), crate::store::new_id()));
    fs::create_dir(&staging).map_err(|e| format!("Cannot stage punctuation: {e}"))?;
    let result = (|| {
        let mut done = 0;
        for file in pins {
            check_cancel(cancel)?;
            download(file, &staging.join(file.relative), done)?;
            done += file.bytes;
        }
        check_cancel(cancel)?;
        verifying();
        for file in pins {
            check_cancel(cancel)?;
            if !optional_asr::file_matches(&staging.join(file.relative), file)? {
                return Err(format!("The downloaded {} did not match its pinned checksum. Nothing was installed.", file.relative));
            }
        }
        check_cancel(cancel)?;
        let manifest = Manifest {
            schema: 1,
            id: package.id().into(),
            files: pins.iter().map(|file| FileRecord {
                name: file.relative.into(), bytes: file.bytes, sha256: file.sha256.into(),
            }).collect(),
        };
        let bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| format!("Cannot prepare the punctuation manifest: {e}"))?;
        if parse_manifest(&bytes).is_none() {
            return Err("The punctuation package has an invalid manifest.".into());
        }
        fs::write(staging.join("manifest.json"), bytes)
            .map_err(|e| format!("Cannot save the punctuation manifest: {e}"))?;
        check_cancel(cancel)?;
        optional_asr::ensure_within(root, parent)?;
        optional_asr::transactional_activate(parent, &staging, &dest, "punctuation model")?;
        // Receipts belong to final paths; staging receipts do not survive a move.
        Check::Blocking.files(root, &dest, pins);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn parse_manifest(bytes: &[u8]) -> Option<Manifest> {
    let manifest: Manifest = serde_json::from_slice(bytes).ok()?;
    if manifest.schema != 1 || Package::from_id(&manifest.id).is_none() || manifest.files.len() != FILES.len() {
        return None;
    }
    for name in FILES {
        if manifest
            .files
            .iter()
            .filter(|file| file.name == name)
            .count()
            != 1
        {
            return None;
        }
    }
    for file in &manifest.files {
        if file.bytes == 0
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|ch| ch.is_ascii_hexdigit())
        {
            return None;
        }
    }
    manifest
        .files
        .iter()
        .try_fold(0u64, |sum, file| sum.checked_add(file.bytes))?;
    Some(manifest)
}

#[cfg(test)]
fn manifest(root: &Path) -> Option<Manifest> {
    manifest_for(root, Package::V1)
}

fn manifest_for(root: &Path, package: Package) -> Option<Manifest> {
    let file = File::open(package.path(root).join("manifest.json")).ok()?;
    if file.metadata().ok()?.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    parse_manifest(&bytes).filter(|manifest| manifest.id == package.id())
}

#[cfg(test)]
pub(crate) fn package_bytes(root: &Path) -> u64 {
    Package::V1.package_bytes(root)
}

pub(crate) fn runtime_status(root: &Path, check: Check) -> Status {
    let pin = crate::diarization_model::runtime_library_pin();
    check.file(
        root,
        &crate::diarization_model::runtime_library_for_check(root, check),
        pin.bytes,
        pin.sha256,
    )
}

#[cfg(test)]
pub(crate) fn model_status(root: &Path, check: Check) -> Status {
    Package::V1.model_status(root, check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(root: &Path, package: Package) {
        let directory = package.path(root);
        fs::create_dir_all(&directory).unwrap();
        let pins = fixture_pins();
        for pin in &pins {
            fs::write(directory.join(pin.relative), b"abc").unwrap();
        }
        let manifest = Manifest {
            schema: 1,
            id: package.id().into(),
            files: pins.iter().map(|pin| FileRecord {
                name: pin.relative.into(), bytes: pin.bytes, sha256: pin.sha256.into(),
            }).collect(),
        };
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn v2_release_pins_match_the_selected_package_manifest() {
        assert_eq!(Package::V2.pinned_bytes(), 302_728_954);
        assert_eq!(V2_PINS.len(), FILES.len());
        for ((file, name), (bytes, sha256)) in V2_PINS.iter().zip(FILES).zip([
            (268_364_003, "02b5cbbe66e9456fb282b0663a42937d6cdbbb4780667be44caab803118446a6"),
            (34_363_188, "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
            (1_763, "3dd789e72775cd7d8d739f687da7f1ee4ad382758aba044a64c8afd61083ea6b"),
        ]) {
            assert_eq!((file.relative, file.bytes, file.sha256), (name, bytes, sha256));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.bytes().all(|ch| ch.is_ascii_hexdigit()));
            let url = Package::V2.download_url(file).unwrap();
            assert_eq!(url, format!("https://huggingface.co/SonoJot/sonojot-punct-v2/resolve/94d84b4b17d4d2fb3cee46dbd7a4425b8a6ed625/{name}"));
            assert!(!url.contains("/resolve/main/"));
        }
        assert_eq!(V2_RELEASE_COMMIT, Some("94d84b4b17d4d2fb3cee46dbd7a4425b8a6ed625"));
        assert!(Package::V2.released());
        assert_eq!(Package::V2.revision(), V2_RELEASE_COMMIT);
    }

    #[test]
    fn cancelled_v2_install_cannot_create_files_or_report_progress() {
        let root = std::env::temp_dir().join(format!("punct-v2-cancel-{}", uuid::Uuid::new_v4()));
        let result = install_package(&root, Package::V2, &AtomicBool::new(true),
            &|_, _| panic!("cancelled progress"), &|| panic!("cancelled verification"));
        assert_eq!(result.unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
    }

    #[test]
    fn selection_prefers_verified_v2_and_preserves_legacy_v1_manifest() {
        let root = std::env::temp_dir().join(format!("punct-select-{}", uuid::Uuid::new_v4()));
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), None);
        write_fixture(&root, Package::V1);
        let legacy = fs::read(path(&root).join("manifest.json")).unwrap();
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V1));
        write_fixture(&root, Package::V2);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V2));
        assert_eq!(selected_package_with_runtime(&root, Check::Background, Status::Ready), Some(Package::V2));
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Missing), None);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Checking), None);
        assert_eq!(selected_path(&root, Check::Blocking), None, "fixture has no shared runtime");
        for package in [Package::V1, Package::V2] {
            assert_eq!(package.package_bytes(&root), 9, "local manifests need not match release pins");
            assert_eq!(package.model_status(&root, Check::Blocking), Status::Missing);
        }
        assert_eq!(fs::read(path(&root).join("manifest.json")).unwrap(), legacy);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_v2_falls_back_to_v1_and_never_borrows_its_manifest() {
        for fault in ["hash", "missing-file", "manifest", "wrong-id", "oversize"] {
            let root = std::env::temp_dir().join(format!("punct-fallback-{}", uuid::Uuid::new_v4()));
            write_fixture(&root, Package::V1);
            write_fixture(&root, Package::V2);
            let directory = Package::V2.path(&root);
            match fault {
                "hash" => fs::write(directory.join(FILES[0]), b"bad").unwrap(),
                "missing-file" => fs::remove_file(directory.join(FILES[1])).unwrap(),
                "manifest" => fs::write(directory.join("manifest.json"), b"{}").unwrap(),
                "wrong-id" => {
                    fs::copy(path(&root).join("manifest.json"), directory.join("manifest.json")).unwrap();
                    assert_eq!(Package::V2.package_bytes(&root), 0);
                }
                "oversize" => fs::write(directory.join("manifest.json"), vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(Package::V2.files_status(&root, Check::Blocking), Status::Missing, "{fault}");
            assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V1), "{fault}");
            fs::remove_dir_all(path(&root)).unwrap();
            assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), None, "{fault}");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn valid_v2_alone_is_selected_and_unknown_directories_are_rejected() {
        let root = std::env::temp_dir().join(format!("punct-v2-only-{}", uuid::Uuid::new_v4()));
        write_fixture(&root, Package::V2);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V2));
        assert_eq!(status_at(&root, &root, Check::Blocking), Status::Missing);
        assert_eq!(Package::from_id("sonojot-punct-v3"), None);
        fs::remove_dir_all(root).unwrap();
    }

    fn fixture_pins() -> Vec<PinnedFile> {
        FILES.map(|relative| PinnedFile {
            relative, bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        }).to_vec()
    }

    #[test]
    fn release_pins_and_urls_match_the_download_brief() {
        assert_eq!(pinned_bytes(), 302_708_257);
        assert_eq!(PINS.len(), FILES.len());
        for ((file, name), (bytes, sha256)) in PINS.iter().zip(FILES).zip([
            (268_344_270, "6dccbb7d461407bfa9a315e0551b8a2db6ea31521f6e493180fec0a30e891168"),
            (34_363_188, "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
            (799, "489026321fa6f54db2ae6c1d0fc91da981d352ec7d5f078e3d1c56edaa1856e3"),
        ]) {
            assert_eq!((file.relative, file.bytes, file.sha256), (name, bytes, sha256));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.bytes().all(|ch| ch.is_ascii_hexdigit()));
            assert_eq!(download_url(file), format!("https://huggingface.co/{REPO}/resolve/{REVISION}/{name}"));
            assert!(!download_url(file).contains("/resolve/main/"));
        }
        assert!(released(), "Replace the unpublished sentinel with the uploaded model's commit before shipping.");
        assert_ne!(REVISION, "98f7b7a96942180d36869bec981675f17ae25219");
    }

    #[test]
    fn cancelled_install_never_creates_a_package_or_staging() {
        let root = std::env::temp_dir().join(format!("punct-cancel-{}", uuid::Uuid::new_v4()));
        let cancel = AtomicBool::new(true);
        assert_eq!(install_model(&root, &cancel, &|_, _| panic!("cancelled progress"),
            &|| panic!("cancelled verification")).unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
    }

    #[test]
    fn bad_sha_and_late_cancel_never_publish_or_replace_a_package() {
        for previous in [false, true] {
            for cancelled in [false, true] {
                let root = std::env::temp_dir().join(format!("punct-transaction-{}", uuid::Uuid::new_v4()));
                let dest = path(&root);
                if previous {
                    fs::create_dir_all(&dest).unwrap();
                    fs::write(dest.join("old-model"), b"preserve").unwrap();
                }
                let cancel = AtomicBool::new(false);
                let pins = fixture_pins();
                let result = stage_and_activate(&root, Package::V1, &pins, &cancel,
                    &|_, path, _| fs::write(path, if cancelled { b"abc" } else { b"bad" }).map_err(|e| e.to_string()),
                    &|| { if cancelled { cancel.store(true, Ordering::Release); } },
                );
                let error = result.unwrap_err();
                if cancelled { assert_eq!(error, "Install cancelled."); }
                else { assert!(error.contains("checksum"), "{error}"); }
                assert!(!dest.join("manifest.json").exists());
                if previous {
                    assert_eq!(fs::read(dest.join("old-model")).unwrap(), b"preserve");
                    assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
                } else {
                    assert!(!dest.exists());
                }
                assert_eq!(fs::read_dir(root.join("punctuation")).unwrap().count(), usize::from(previous));
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn verified_retry_writes_only_runtime_files_and_compatible_manifest() {
        let root = std::env::temp_dir().join(format!("punct-retry-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(path(&root)).unwrap();
        fs::write(path(&root).join("incomplete"), b"old").unwrap();
        let pins = fixture_pins();
        stage_and_activate(&root, Package::V1, &pins, &AtomicBool::new(false),
            &|_, path, _| fs::write(path, b"abc").map_err(|e| e.to_string()), &|| {},
        ).unwrap();
        let manifest = manifest(&root).unwrap();
        assert_eq!(manifest.schema, 1);
        assert_eq!(manifest.id, ID);
        assert_eq!(manifest.files.len(), 3);
        assert_eq!(package_bytes(&root), 9);
        assert_eq!(fs::read_dir(path(&root)).unwrap().count(), 4);
        assert_eq!(fs::read_dir(root.join("punctuation")).unwrap().count(), 1);
        assert!(Check::Background.files(&root, &path(&root), &pins).ready());
        assert_eq!(model_status(&root, Check::Blocking), Status::Missing, "no runtime in fixture");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_v2_download_keeps_v1_and_writes_its_own_manifest() {
        let root = std::env::temp_dir().join(format!("punct-v2-coexist-{}", uuid::Uuid::new_v4()));
        write_fixture(&root, Package::V1);
        let legacy_manifest = fs::read(Package::V1.path(&root).join("manifest.json")).unwrap();
        let runtime = optional_asr::runtime_dir(&root);
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("preserve"), b"shared runtime").unwrap();
        let pins = fixture_pins();
        stage_and_activate(&root, Package::V2, &pins, &AtomicBool::new(false),
            &|_, path, _| fs::write(path, b"abc").map_err(|e| e.to_string()), &|| {},
        ).unwrap();
        let manifest = manifest_for(&root, Package::V2).unwrap();
        assert_eq!(manifest.id, V2_ID);
        assert_eq!(manifest.files.len(), FILES.len());
        assert_eq!(Package::V2.package_bytes(&root), 9);
        assert!(Package::V2.files_status(&root, Check::Background).ready());
        assert_eq!(fs::read_dir(Package::V2.path(&root)).unwrap().count(), 4);
        assert_eq!(fs::read_dir(root.join("punctuation")).unwrap().count(), 2);
        assert_eq!(fs::read(Package::V1.path(&root).join("manifest.json")).unwrap(), legacy_manifest);
        assert!(Package::V1.files_status(&root, Check::Blocking).ready());
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V2));
        assert_eq!(fs::read(runtime.join("preserve")).unwrap(), b"shared runtime");
        fs::remove_dir_all(root).unwrap();
    }

    fn valid() -> serde_json::Value {
        serde_json::json!({"schema":1,"id":ID,"files":FILES.map(|name|
            serde_json::json!({"name":name,"bytes":3,"sha256":"a".repeat(64)}))})
    }

    #[test]
    fn manifest_requires_exact_files_and_real_size_hash_fields() {
        let good = valid();
        assert!(parse_manifest(&serde_json::to_vec(&good).unwrap()).is_some());
        let mut v2 = good.clone();
        v2["id"] = V2_ID.into();
        assert!(parse_manifest(&serde_json::to_vec(&v2).unwrap()).is_some());
        for invalid in [
            {
                let mut v = good.clone();
                v["schema"] = 2.into();
                v
            },
            {
                let mut v = good.clone();
                v["id"] = "different".into();
                v
            },
            {
                let mut v = good.clone();
                v["files"][0]["name"] = "../model.int8.onnx".into();
                v
            },
            {
                let mut v = good.clone();
                v["files"][1] = v["files"][0].clone();
                v
            },
            {
                let mut v = good.clone();
                v["files"][0]["bytes"] = 0.into();
                v
            },
            {
                let mut v = good.clone();
                v["files"][0]["sha256"] = "g".repeat(64).into();
                v
            },
            {
                let mut v = good.clone();
                v["files"].as_array_mut().unwrap().pop();
                v
            },
            {
                let mut v = good.clone();
                v["files"]
                    .as_array_mut()
                    .unwrap()
                    .push(good["files"][0].clone());
                v
            },
        ] {
            assert!(parse_manifest(&serde_json::to_vec(&invalid).unwrap()).is_none());
        }
    }

    #[test]
    fn installed_files_use_existing_verifier() {
        use sha2::{Digest, Sha256};
        let root =
            std::env::temp_dir().join(format!("sonojot-punct-manifest-{}", uuid::Uuid::new_v4()));
        let directory = path(&root);
        std::fs::create_dir_all(&directory).unwrap();
        let mut data = valid();
        for (index, name) in FILES.iter().enumerate() {
            std::fs::write(directory.join(name), b"abc").unwrap();
            data["files"][index]["sha256"] = format!("{:x}", Sha256::digest(b"abc")).into();
        }
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&data).unwrap(),
        )
        .unwrap();
        assert_eq!(package_bytes(&root), 9);
        let pins = manifest(&root).unwrap();
        for file in pins.files {
            assert!(Check::Blocking
                .file(&root, &directory.join(file.name), file.bytes, &file.sha256)
                .ready());
        }
        // No runtime is installed in this isolated fixture.
        assert_eq!(model_status(&root, Check::Blocking), Status::Missing);
        std::fs::write(directory.join(FILES[0]), b"bad").unwrap();
        let pin = &manifest(&root).unwrap().files[0];
        assert_eq!(
            Check::Blocking.file(&root, &directory.join(&pin.name), pin.bytes, &pin.sha256),
            Status::Missing
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
