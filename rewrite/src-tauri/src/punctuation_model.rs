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
//! `sonojot-punct-v1`, `sonojot-punct-v2` and `sonojot-punct-v3` use schema 1. The manifest ID
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
pub(crate) const V4_ID: &str = "sonojot-cleanup-v4";
/// Punctuation, written form, optional filler and repeat removal, and casing.
pub(crate) const P4_ID: &str = "sonojot-punct-v4";
/// The public Hugging Face upload commit (November 14, 2026).
pub(crate) const P4_RELEASE_COMMIT: Option<&str> = Some("2c242cbb72f54cf698d6aabfad5d84cacccef91f");
/// v4's graph uses 8-bit `MatMulNBits` (com.microsoft), which ONNX Runtime's CPU
/// provider supports from 1.22. Both shared runtimes are pinned to 1.28.2.
pub(crate) const P4_MIN_RUNTIME: (u32, u32) = (1, 22);
/// The released v4 package's decision thresholds. Downloads write them into the
/// local manifest, which the engine requires for v4.
pub(crate) const P4_DELETE_THRESHOLD: f32 = 0.955;
pub(crate) const P4_CASE_THRESHOLDS: crate::punctuation::CaseThresholds = crate::punctuation::CaseThresholds {
    raise: 0.8, upper: 0.7, lower_positional: 0.8, lower: 0.98,
};
pub(crate) const V3_ID: &str = "sonojot-punct-v3";
/// Set only to the real Hugging Face upload commit after publication is approved.
/// None keeps v3 local-only, including direct install requests.
pub(crate) const V3_RELEASE_COMMIT: Option<&str> = Some("e023ae245a00bac1785bb6a022b16d181e346e48");
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
// From the released v3 package's manifest (seed 2), October 27, 2026. Same
// architecture, tokenizer and input contract as v2.
pub(crate) const V3_PINS: &[PinnedFile] = &[
    PinnedFile {
        relative: "model.int8.onnx",
        bytes: 268_364_003,
        sha256: "11f46c02f68a2a05912ed0c65a6dc063f199280b816307fa3ec17ce23a5e2182",
    },
    PinnedFile {
        relative: "tokenizer.json",
        bytes: 34_363_188,
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    PinnedFile {
        relative: "punct-config.json",
        bytes: 1_672,
        sha256: "8bbd52775cedbbcf33c6be4402c9cb77d7c6f581fbc10909518384425039c9e4",
    },
];
// From the v4 upload package's manifest (8-bit weights in blocks of 32), November 14, 2026.
// Same tokenizer as v2 and v3; the config adds the casing input and thresholds.
pub(crate) const P4_PINS: &[PinnedFile] = &[
    PinnedFile {
        relative: "model.int8.onnx",
        bytes: 146_682_294,
        sha256: "5bd49ab7dfae3d7c0352852532db082a2beffc294adc3c70e17775a83208c3a2",
    },
    PinnedFile {
        relative: "tokenizer.json",
        bytes: 34_363_188,
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    },
    PinnedFile {
        relative: "punct-config.json",
        bytes: 2_500,
        sha256: "250ff56bfa7c7070a1b9a42401fa0dd82eff50cfa52c62036f0550fa5a360c43",
    },
];
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Package {
    V1,
    V2,
    V3,
    V4,
    P4,
}

impl Package {
    /// Selection order: the newest verified package wins.
    pub(crate) const NEWEST_FIRST: [Self; 5] = [Self::P4, Self::V4, Self::V3, Self::V2, Self::V1];
    /// Library order, oldest first.
    pub(crate) const ALL: [Self; 5] = [Self::V1, Self::V2, Self::V3, Self::V4, Self::P4];
    /// The package first-run Setup installs and existing users are offered.
    pub(crate) const LATEST: Self = Self::V3;

    /// The newest published package: what first-run Setup installs and existing
    /// users are offered. v1 is always published.
    pub(crate) fn offered() -> Self {
        Self::NEWEST_FIRST.into_iter().find(|package| package.released()).unwrap_or(Self::V1)
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id {
            ID => Some(Self::V1),
            V2_ID => Some(Self::V2),
            V3_ID => Some(Self::V3),
            V4_ID => Some(Self::V4),
            P4_ID => Some(Self::P4),
            _ => None,
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self { Self::V1 => ID, Self::V2 => V2_ID, Self::V3 => V3_ID, Self::V4 => V4_ID, Self::P4 => P4_ID }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::V1 => "SonoJot Punctuation",
            Self::V2 => "SonoJot Punctuation v2",
            Self::V3 => "SonoJot Punctuation v3",
            Self::V4 => "SonoJot Cleanup v4 (local test)",
            Self::P4 => "SonoJot Punctuation v4",
        }
    }

    pub(crate) fn page_url(self) -> &'static str {
        match self {
            Self::V1 => "https://huggingface.co/SonoJot/sonojot-punct-v1",
            Self::V2 => "https://huggingface.co/SonoJot/sonojot-punct-v2",
            Self::V3 => "https://huggingface.co/SonoJot/sonojot-punct-v3",
            Self::V4 => "https://huggingface.co/SonoJot/sonojot-cleanup-v4",
            Self::P4 => "https://huggingface.co/SonoJot/sonojot-punct-v4",
        }
    }

    pub(crate) fn path(self, root: &Path) -> PathBuf {
        root.join("punctuation").join(self.id())
    }

    pub(crate) fn revision(self) -> Option<&'static str> {
        let revision = match self {
            Self::V1 => Some(REVISION),
            Self::V2 => V2_RELEASE_COMMIT,
            Self::V3 => V3_RELEASE_COMMIT,
            Self::P4 => P4_RELEASE_COMMIT,
            Self::V4 => None,
        };
        revision.filter(|value| value.len() == 40 && value.bytes().all(|ch| ch.is_ascii_hexdigit()))
    }

    pub(crate) fn released(self) -> bool { self.revision().is_some() }

    pub(crate) fn pins(self) -> &'static [PinnedFile] {
        match self { Self::V1 => PINS, Self::V2 => V2_PINS, Self::V3 => V3_PINS, Self::P4 => P4_PINS, Self::V4 => &[] }
    }

    /// Thresholds a downloaded package's manifest carries (v4 needs both).
    fn release_thresholds(self) -> (Option<f32>, Option<crate::punctuation::CaseThresholds>) {
        match self {
            Self::P4 => (Some(P4_DELETE_THRESHOLD), Some(P4_CASE_THRESHOLDS)),
            _ => (None, None),
        }
    }

    pub(crate) fn pinned_bytes(self) -> u64 {
        self.pins().iter().map(|file| file.bytes).sum()
    }

    pub(crate) fn download_bytes(self, root: &Path, check: Check) -> u64 {
        self.pinned_bytes() + runtime_download_bytes(root, check)
    }

    pub(crate) fn download_url(self, file: &PinnedFile) -> Result<String, String> {
        let revision = self.revision().ok_or_else(|| self.unreleased_error())?;
        let repo = match self {
            Self::V1 => REPO,
            Self::V2 => "SonoJot/sonojot-punct-v2",
            Self::V3 => "SonoJot/sonojot-punct-v3",
            Self::V4 => "SonoJot/sonojot-cleanup-v4",
            Self::P4 => "SonoJot/sonojot-punct-v4",
        };
        Ok(format!("https://huggingface.co/{repo}/resolve/{revision}/{}", file.relative))
    }

    pub(crate) fn unreleased_error(self) -> String {
        format!("{} is not published yet. Its download is unavailable until the upload is complete.", self.name())
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
    Package::NEWEST_FIRST.into_iter().find(|package| {
        !unloadable(&package.path(root)) && Status::all([package.files_status(root, check), runtime]).ready()
    })
}

/// Verified packages whose graph still failed to open in this process, for example
/// on a runtime without an operator the graph needs (v4's 8-bit `MatMulNBits`
/// needs ONNX Runtime 1.22). Selection skips them until restart, so an older
/// verified package takes over instead of every dictation falling back to rules.
static UNLOADABLE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn identity(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub(crate) fn mark_unloadable(directory: &Path) {
    let directory = identity(directory);
    let mut list = UNLOADABLE.lock().unwrap_or_else(|e| e.into_inner());
    if !list.contains(&directory) {
        list.push(directory);
    }
}

fn unloadable(directory: &Path) -> bool {
    let list = UNLOADABLE.lock().unwrap_or_else(|e| e.into_inner());
    !list.is_empty() && list.contains(&identity(directory))
}

/// Verify the exact selected directory, rather than another installed version.
pub(crate) fn status_at(root: &Path, directory: &Path, check: Check) -> Status {
    Package::NEWEST_FIRST.into_iter()
        .find(|package| package.path(root) == directory)
        .map_or(Status::Missing, |package| package.model_status(root, check))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    id: String,
    files: Vec<FileRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delete_threshold: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    case_thresholds: Option<crate::punctuation::CaseThresholds>,
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
        let (delete_threshold, case_thresholds) = package.release_thresholds();
        let manifest = Manifest {
            schema: 1,
            id: package.id().into(),
            delete_threshold,
            case_thresholds,
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
    if matches!(manifest.id.as_str(), V4_ID | P4_ID) && !manifest.delete_threshold.is_some_and(|t| t.is_finite() && t > 0.0 && t <= 1.0) { return None; }
    if (manifest.id == P4_ID) != manifest.case_thresholds.is_some_and(|t| t.valid()) { return None; }
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
            delete_threshold: matches!(package, Package::V4 | Package::P4).then_some(0.975),
            case_thresholds: (package == Package::P4).then_some(crate::punctuation::CaseThresholds {
                raise: 0.7, upper: 0.8, lower_positional: 0.6, lower: 0.97,
            }),
            files: pins.iter().map(|pin| FileRecord {
                name: pin.relative.into(), bytes: pin.bytes, sha256: pin.sha256.into(),
            }).collect(),
        };
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn local_punct_v4_needs_both_thresholds_and_is_the_newest_choice() {
        let root = std::env::temp_dir().join(format!("punct-v4-{}", uuid::Uuid::new_v4()));
        write_fixture(&root, Package::V4);
        write_fixture(&root, Package::P4);
        assert!(Package::P4.released());
        assert_eq!(Package::offered(), Package::P4);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::P4));
        let path = Package::P4.path(&root).join("manifest.json");
        let good = fs::read(&path).unwrap();
        for key in ["case_thresholds", "delete_threshold"] {
            let mut manifest: serde_json::Value = serde_json::from_slice(&good).unwrap();
            manifest.as_object_mut().unwrap().remove(key);
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V4), "{key}");
        }
        let mut manifest: serde_json::Value = serde_json::from_slice(&good).unwrap();
        manifest["case_thresholds"]["lower"] = serde_json::json!(0.4);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V4));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_v4_requires_a_threshold_and_wins_only_when_verified() {
        let root = std::env::temp_dir().join(format!("cleanup-v4-{}", uuid::Uuid::new_v4()));
        write_fixture(&root, Package::V3);
        write_fixture(&root, Package::V4);
        assert!(!Package::V4.released());
        assert_eq!(Package::offered(), Package::P4);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V4));
        let path = Package::V4.path(&root).join("manifest.json");
        let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for bad in [serde_json::Value::Null, serde_json::json!(0), serde_json::json!(1.1)] {
            manifest["delete_threshold"] = bad;
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V3));
        }
        fs::remove_dir_all(root).unwrap();
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
    fn v3_release_pins_match_the_released_package_manifest() {
        assert_eq!(Package::V3.pinned_bytes(), 302_728_863);
        assert_eq!(V3_PINS.len(), FILES.len());
        for ((file, name), (bytes, sha256)) in V3_PINS.iter().zip(FILES).zip([
            (268_364_003, "11f46c02f68a2a05912ed0c65a6dc063f199280b816307fa3ec17ce23a5e2182"),
            (34_363_188, "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
            (1_672, "8bbd52775cedbbcf33c6be4402c9cb77d7c6f581fbc10909518384425039c9e4"),
        ]) {
            assert_eq!((file.relative, file.bytes, file.sha256), (name, bytes, sha256));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.bytes().all(|ch| ch.is_ascii_hexdigit()));
            let url = Package::V3.download_url(file).unwrap();
            assert_eq!(url, format!("https://huggingface.co/SonoJot/sonojot-punct-v3/resolve/e023ae245a00bac1785bb6a022b16d181e346e48/{name}"));
            assert!(!url.contains("/resolve/main/"));
        }
        assert_eq!(V3_RELEASE_COMMIT, Some("e023ae245a00bac1785bb6a022b16d181e346e48"));
        assert!(Package::V3.released());
        assert_eq!(Package::V3.revision(), V3_RELEASE_COMMIT);
        assert_eq!(Package::LATEST, Package::V3);
        assert_eq!(Package::offered(), Package::P4);
        assert_eq!(Package::V3.page_url(), "https://huggingface.co/SonoJot/sonojot-punct-v3");
    }

    #[test]
    fn v4_release_pins_match_the_published_package() {
        assert_eq!(Package::P4.pinned_bytes(), 181_047_982);
        assert_eq!(P4_PINS.len(), FILES.len());
        for ((file, name), (bytes, sha256)) in P4_PINS.iter().zip(FILES).zip([
            (146_682_294, "5bd49ab7dfae3d7c0352852532db082a2beffc294adc3c70e17775a83208c3a2"),
            (34_363_188, "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f"),
            (2_500, "250ff56bfa7c7070a1b9a42401fa0dd82eff50cfa52c62036f0550fa5a360c43"),
        ]) {
            assert_eq!((file.relative, file.bytes, file.sha256), (name, bytes, sha256));
        }
        assert_eq!(P4_RELEASE_COMMIT, Some("2c242cbb72f54cf698d6aabfad5d84cacccef91f"));
        assert!(Package::P4.released());
        for (file, name) in P4_PINS.iter().zip(FILES) {
            let url = Package::P4.download_url(file).unwrap();
            assert_eq!(url, format!("https://huggingface.co/SonoJot/sonojot-punct-v4/resolve/2c242cbb72f54cf698d6aabfad5d84cacccef91f/{name}"));
        }
        assert_eq!(Package::offered(), Package::P4);
        assert!(P4_CASE_THRESHOLDS.valid());
        assert_eq!(Package::P4.name(), "SonoJot Punctuation v4");
        assert_eq!(Package::P4.page_url(), "https://huggingface.co/SonoJot/sonojot-punct-v4");
    }

    #[test]
    fn downloaded_v4_manifest_carries_its_release_thresholds() {
        let root = std::env::temp_dir().join(format!("punct-v4-install-{}", uuid::Uuid::new_v4()));
        stage_and_activate(&root, Package::P4, &fixture_pins(), &AtomicBool::new(false),
            &|_, path, _| fs::write(path, b"abc").map_err(|e| e.to_string()), &|| {},
        ).unwrap();
        let manifest = manifest_for(&root, Package::P4).unwrap();
        assert_eq!(manifest.delete_threshold, Some(P4_DELETE_THRESHOLD));
        assert_eq!(manifest.case_thresholds, Some(P4_CASE_THRESHOLDS));
        assert!(Package::P4.files_status(&root, Check::Blocking).ready());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_package_that_fails_to_open_yields_to_the_next_verified_one() {
        let root = std::env::temp_dir().join(format!("punct-unloadable-{}", uuid::Uuid::new_v4()));
        write_fixture(&root, Package::V3);
        write_fixture(&root, Package::P4);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::P4));
        mark_unloadable(&Package::P4.path(&root));
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V3));
        mark_unloadable(&Package::V3.path(&root));
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), None);
        fs::remove_dir_all(root).unwrap();
    }

    /// Opt-in, network: a fresh model store downloads v4 from Hugging Face through the
    /// installer, cleans a sentence through the real helper, then falls back to a
    /// verified v3 when v4's verified model cannot be opened.
    ///   SONOJOT_PUNCT_TEST_WORKER   release fidelity_eval.exe built from this tree
    ///   SONOJOT_LIVE_STORE          a folder that does not exist yet (created and removed)
    ///   SONOJOT_LIVE_V3_DIR         an installed sonojot-punct-v3 folder to copy in
    ///   SONOJOT_LIVE_BROKEN_V4      an ONNX graph that verifies but fails to open
    #[test]
    #[ignore]
    fn live_v4_download_then_fallback_when_its_model_cannot_open() {
        use crate::punctuation::{run_with_written_form_mode, WrittenFormMode};
        let var = |name: &str| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
        let (store, v3, broken) = (var("SONOJOT_LIVE_STORE"), var("SONOJOT_LIVE_V3_DIR"), var("SONOJOT_LIVE_BROKEN_V4"));
        assert!(var("SONOJOT_PUNCT_TEST_WORKER").is_file());
        assert!(!store.exists(), "SONOJOT_LIVE_STORE must be a fresh folder");
        let root = store.join("models");
        let cancel = AtomicBool::new(false);
        install_package(&root, Package::P4, &cancel, &|_, _| {}, &|| {}).unwrap();
        let manifest = manifest_for(&root, Package::P4).unwrap();
        assert_eq!(manifest.delete_threshold, Some(P4_DELETE_THRESHOLD));
        assert_eq!(manifest.case_thresholds, Some(P4_CASE_THRESHOLDS));
        assert!(Package::P4.model_status(&root, Check::Blocking).ready());
        assert_eq!(selected_package(&root, Check::Background), Some(Package::P4));
        let sentence = "so we met john and maria in paris on friday";
        let downloaded = run_with_written_form_mode(&root, sentence, &[], &cancel, WrittenFormMode::Model);
        eprintln!("downloaded v4: {} | {}", downloaded.stage, downloaded.text);
        assert_eq!(downloaded.stage, "punctuation_model");
        assert!(downloaded.text.contains("John") && downloaded.text.contains("Paris"), "{}", downloaded.text);

        // An older verified package, and a v4 whose verified model cannot be opened.
        let dest = Package::V3.path(&root);
        fs::create_dir_all(&dest).unwrap();
        for entry in fs::read_dir(&v3).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
        }
        assert!(Package::V3.files_status(&root, Check::Blocking).ready());
        let p4 = Package::P4.path(&root);
        fs::copy(&broken, p4.join("model.int8.onnx")).unwrap();
        let mut manifest = manifest_for(&root, Package::P4).unwrap();
        let bytes = fs::read(p4.join("model.int8.onnx")).unwrap();
        let model = manifest.files.iter_mut().find(|file| file.name == "model.int8.onnx").unwrap();
        model.bytes = bytes.len() as u64;
        model.sha256 = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
        fs::write(p4.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        assert!(Package::P4.files_status(&root, Check::Blocking).ready(), "the broken v4 must still verify");
        assert_eq!(selected_package(&root, Check::Background), Some(Package::P4));
        let fallback = run_with_written_form_mode(&root, sentence, &[], &cancel, WrittenFormMode::Model);
        eprintln!("unloadable v4: {} | {}", fallback.stage, fallback.text);
        assert_eq!(fallback.stage, "punctuation_model", "the same dictation must use v3, not rules");
        assert!(!fallback.text.contains("John"), "v3 has no casing output: {}", fallback.text);
        assert_eq!(selected_package(&root, Check::Background), Some(Package::V3));
        let again = run_with_written_form_mode(&root, sentence, &[], &cancel, WrittenFormMode::Model);
        assert_eq!((again.stage, again.text.as_str()), (fallback.stage, fallback.text.as_str()));
        crate::punctuation::release_cached();
        fs::remove_dir_all(&store).unwrap();
    }

    #[test]
    fn shared_runtimes_meet_the_v4_operator_minimum() {
        let version = crate::diarization_model::MAC_RUNTIME_URL
            .split("/download/v").nth(1).and_then(|rest| rest.split('/').next()).unwrap();
        let mut parts = version.split('.').map(|part| part.parse::<u32>().unwrap());
        assert!((parts.next().unwrap(), parts.next().unwrap()) >= P4_MIN_RUNTIME, "{version}");
        // sherpa-onnx 1.13.8's onnxruntime.dll is ONNX Runtime 1.28.2 (file version read
        // 2026-11-14). A new pin must be checked against P4_MIN_RUNTIME before it lands.
        #[cfg(windows)]
        assert_eq!(optional_asr::RUNTIME_FILES[0].sha256, "7f66f939a881baf4f46a2216496798edf4a1429878b646d12674aa62f27d8a25");
    }

    #[test]
    fn cancelled_v3_install_cannot_create_files_or_report_progress() {
        let root = std::env::temp_dir().join(format!("punct-v3-cancel-{}", uuid::Uuid::new_v4()));
        let result = install_package(&root, Package::V3, &AtomicBool::new(true),
            &|_, _| panic!("cancelled progress"), &|| panic!("cancelled verification"));
        assert_eq!(result.unwrap_err(), "Install cancelled.");
        assert!(!root.exists());
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
    fn selection_prefers_the_newest_verified_package_and_preserves_legacy_v1_manifest() {
        // One root for every step, so the Background check below reads the
        // receipt the Blocking check just stored.
        let root = std::env::temp_dir().join(format!("punct-select-{}", uuid::Uuid::new_v4()));
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), None);
        write_fixture(&root, Package::V3);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V3));
        assert_eq!(status_at(&root, &root, Check::Blocking), Status::Missing);
        fs::remove_dir_all(Package::V3.path(&root)).unwrap();
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), None);
        write_fixture(&root, Package::V1);
        let legacy = fs::read(path(&root).join("manifest.json")).unwrap();
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V1));
        write_fixture(&root, Package::V2);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V2));
        assert_eq!(selected_package_with_runtime(&root, Check::Background, Status::Ready), Some(Package::V2));
        write_fixture(&root, Package::V3);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V3));
        assert_eq!(status_at(&root, &Package::V3.path(&root), Check::Blocking), Status::Missing, "no runtime in fixture");
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Missing), None);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Checking), None);
        assert_eq!(selected_path(&root, Check::Blocking), None, "fixture has no shared runtime");
        for package in [Package::V1, Package::V2, Package::V3] {
            assert_eq!(package.package_bytes(&root), 9, "local manifests need not match release pins");
            assert_eq!(package.model_status(&root, Check::Blocking), Status::Missing);
        }
        assert_eq!(fs::read(path(&root).join("manifest.json")).unwrap(), legacy);
        // An invalid v3 falls back to v2, then v1, and never borrows v2's manifest.
        let directory = Package::V3.path(&root);
        fs::copy(Package::V2.path(&root).join("manifest.json"), directory.join("manifest.json")).unwrap();
        assert_eq!(Package::V3.package_bytes(&root), 0);
        assert_eq!(Package::V3.files_status(&root, Check::Blocking), Status::Missing);
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V2));
        fs::remove_dir_all(Package::V2.path(&root)).unwrap();
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V1));
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
    fn package_ids_are_exact_and_ordered_newest_first() {
        for package in Package::ALL { assert_eq!(Package::from_id(package.id()), Some(package)); }
        for id in ["sonojot-punct-v5", "sonojot-punct-v3-s2", "../sonojot-punct-v3", ""] {
            assert_eq!(Package::from_id(id), None, "{id}");
        }
        assert_eq!(Package::NEWEST_FIRST, [Package::P4, Package::V4, Package::V3, Package::V2, Package::V1]);
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
        // A later verified v3 keeps v1, v2 and the shared runtime (same root).
        let older: Vec<_> = [Package::V1, Package::V2].iter()
            .map(|package| fs::read(package.path(&root).join("manifest.json")).unwrap()).collect();
        stage_and_activate(&root, Package::V3, &fixture_pins(), &AtomicBool::new(false),
            &|_, path, _| fs::write(path, b"abc").map_err(|e| e.to_string()), &|| {},
        ).unwrap();
        let manifest = manifest_for(&root, Package::V3).unwrap();
        assert_eq!(manifest.id, V3_ID);
        assert_eq!(manifest.files.len(), FILES.len());
        assert!(Package::V3.files_status(&root, Check::Blocking).ready());
        assert_eq!(fs::read_dir(root.join("punctuation")).unwrap().count(), 3);
        for (package, manifest) in [Package::V1, Package::V2].iter().zip(older) {
            assert_eq!(fs::read(package.path(&root).join("manifest.json")).unwrap(), manifest);
            assert!(package.files_status(&root, Check::Blocking).ready());
        }
        assert_eq!(selected_package_with_runtime(&root, Check::Blocking, Status::Ready), Some(Package::V3));
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
        let mut v3 = good.clone();
        v3["id"] = V3_ID.into();
        assert!(parse_manifest(&serde_json::to_vec(&v3).unwrap()).is_some());
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
