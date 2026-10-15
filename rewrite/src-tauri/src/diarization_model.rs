//! Pinned, optional speaker-label model. Installation stays in the live model
//! library; checking readiness never loads ONNX Runtime in the desktop process.

use crate::optional_asr::{self, PinnedFile};
use std::path::{Path, PathBuf};

pub(crate) const ID: &str = "nemotron-3-diarization";
pub(crate) const NAME: &str = "Nemotron 3 Diarization";
pub(crate) const AVAILABLE: bool = cfg!(any(
    all(any(windows, target_os = "linux"), target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64")
));
pub(crate) const REPO: &str = "joosthel/Nemotron-3-Diarization-ONNX";
pub(crate) const REVISION: &str = "4a911fc3ca821b76a99fffd5ce5135bd1efca540";
pub(crate) const FILES: &[PinnedFile] = &[
    PinnedFile {
        relative: "preprocessor_core.onnx",
        bytes: 135_897,
        sha256: "28e815a770a8cbe47c351cd36908dd129a0b36ab31461aa8f370a369419703e1",
    },
    PinnedFile {
        relative: "model.int8.onnx",
        bytes: 103_726_888,
        sha256: "17fd04215d655d2feee413a44f96e57d4ebd96cdd9cdbdf8c26da266e08bc835",
    },
    PinnedFile {
        relative: "constants.npz",
        bytes: 6_244,
        sha256: "fd49737a61e90e1990f4e90a3c0088f2f3ef649860ecdaa4abc76ad2428876df",
    },
    PinnedFile {
        relative: "LICENSE",
        bytes: 2_619,
        sha256: "2ab44b68365473c112f5092211a38f231cb23e50de68b75a13369adbd76a74df",
    },
];

pub(crate) fn path(root: &Path) -> PathBuf {
    optional_asr::optional_root(root).join(ID)
}

pub(crate) fn package_bytes() -> u64 {
    FILES.iter().map(|file| file.bytes).sum()
}

/// Existing Mac speaker packages carry their own ONNX Runtime. Punctuation can
/// also prepare the same pinned library in the shared runtime directory.
pub(crate) const MAC: bool = cfg!(target_os = "macos");

// Official Microsoft release, pinned by the GitHub asset digest:
// https://github.com/microsoft/onnxruntime/releases/tag/v1.28.2
pub(crate) const MAC_RUNTIME_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.28.2/onnxruntime-osx-arm64-1.28.2.tgz";
pub(crate) const MAC_RUNTIME_ARCHIVE_BYTES: u64 = 32_093_967;
pub(crate) const MAC_RUNTIME_ARCHIVE_SHA256: &str =
    "c4fceacfc53765d0869dc9180c31ec91054d149017a99d1e80ffe28dc79596de";
/// Archive members unpacked into the model folder, with their own pins.
pub(crate) const MAC_RUNTIME_MEMBERS: [(&str, PinnedFile); 2] = [
    (
        "onnxruntime-osx-arm64-1.28.2/lib/libonnxruntime.1.28.2.dylib",
        PinnedFile {
            relative: "libonnxruntime.1.28.2.dylib",
            bytes: 39_332_040,
            sha256: "c986ef16fd63406bc2cee8cc7cc056189b07c587beb9b79d29e31c3cca129e98",
        },
    ),
    (
        "onnxruntime-osx-arm64-1.28.2/LICENSE",
        PinnedFile {
            relative: "ONNXRUNTIME-LICENSE",
            bytes: 1_073,
            sha256: "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c",
        },
    ),
];

/// The ONNX Runtime library the diarization worker loads on this platform.
pub(crate) fn runtime_library(root: &Path) -> PathBuf {
    runtime_library_for_check(root, crate::model_verification::Check::Blocking)
}

pub(crate) fn runtime_library_for_check(root: &Path, check: crate::model_verification::Check) -> PathBuf {
    if MAC {
        mac_runtime_library(root, check, &MAC_RUNTIME_MEMBERS.map(|(_, pin)| pin))
    } else {
        optional_asr::runtime_dir(root).join(if cfg!(target_os = "linux") { "libonnxruntime.so" } else { "onnxruntime.dll" })
    }
}

fn mac_runtime_library(root: &Path, check: crate::model_verification::Check, pins: &[PinnedFile]) -> PathBuf {
    let shared = optional_asr::runtime_dir(root);
    if check.files(root, &shared, pins) != crate::model_verification::Status::Missing {
        shared.join(pins[0].relative)
    } else {
        // A damaged shared copy must not hide a working package-owned runtime
        // or prevent the existing speaker installer from repairing its copy.
        path(root).join(pins[0].relative)
    }
}

/// Library-only verification for optional inference without the diarization model.
pub(crate) fn runtime_library_pin() -> &'static PinnedFile {
    if MAC {
        &MAC_RUNTIME_MEMBERS[0].1
    } else {
        &optional_asr::RUNTIME_FILES[0]
    }
}

fn runtime_ready(root: &Path) -> bool {
    runtime_status(root, crate::model_verification::Check::Blocking).ready()
}

fn runtime_status(root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    if MAC {
        let library = runtime_library_for_check(root, check);
        let directory = library.parent().expect("runtime library has a parent");
        crate::model_verification::Status::all(MAC_RUNTIME_MEMBERS.iter().map(|(_, file)| {
            check.file(root, &directory.join(file.relative), file.bytes, file.sha256)
        }))
    } else {
        optional_asr::runtime_status(root, check)
    }
}

/// Windows adds the shared CPU runtime only when it is missing; a Mac install
/// always fetches the runtime archive into the package it replaces.
pub(crate) fn download_bytes(root: &Path) -> u64 {
    package_bytes()
        + if MAC {
            MAC_RUNTIME_ARCHIVE_BYTES
        } else {
            optional_asr::runtime_download_bytes(root)
        }
}

pub(crate) fn installed(root: &Path) -> bool {
    model_status(root, crate::model_verification::Check::Blocking).ready()
}

pub(crate) fn model_status(root: &Path, check: crate::model_verification::Check) -> crate::model_verification::Status {
    use crate::model_verification::Status;
    if !AVAILABLE { return Status::Missing; }
    Status::all([runtime_status(root, check), check.files(root, &path(root), FILES)])
}

/// Unpacks only the pinned library and license from the Microsoft archive,
/// verifying each against its own size and SHA-256.
pub(crate) fn extract_mac_runtime(
    archive: &Path,
    dest: &Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::io::Read;
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("Cannot open the ONNX Runtime archive: {e}"))?;
    let mut entries = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut found = 0;
    for entry in entries
        .entries()
        .map_err(|e| format!("Cannot read the ONNX Runtime archive: {e}"))?
    {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Install cancelled.".into());
        }
        let mut entry = entry.map_err(|e| format!("Cannot read the ONNX Runtime archive: {e}"))?;
        let name = entry
            .path()
            .map_err(|e| format!("Cannot read the ONNX Runtime archive: {e}"))?
            .to_string_lossy()
            .replace('\\', "/");
        let Some((_, pin)) = MAC_RUNTIME_MEMBERS.iter().find(|(member, _)| *member == name)
        else {
            continue;
        };
        if entry.header().entry_type() != tar::EntryType::Regular || entry.size() != pin.bytes {
            return Err(format!("The ONNX Runtime archive has an unexpected {}.", pin.relative));
        }
        let target = dest.join(pin.relative);
        let mut out = std::fs::File::create(&target)
            .map_err(|e| format!("Cannot unpack {}: {e}", pin.relative))?;
        std::io::copy(&mut (&mut entry).take(pin.bytes), &mut out)
            .map_err(|e| format!("Cannot unpack {}: {e}", pin.relative))?;
        drop(out);
        if !optional_asr::file_matches(&target, pin)? {
            return Err(format!(
                "The unpacked {} did not match its pinned checksum. Nothing was installed.",
                pin.relative
            ));
        }
        found += 1;
    }
    if found != MAC_RUNTIME_MEMBERS.len() {
        return Err("The ONNX Runtime archive is missing its library.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let library = dest.join(MAC_RUNTIME_MEMBERS[0].1.relative);
        std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Cannot prepare the ONNX Runtime library: {e}"))?;
    }
    Ok(())
}

pub(crate) fn files_ready(root: &Path) -> bool {
    crate::model_verification::Check::Blocking.files(root, &path(root), FILES).ready()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mac_library_prefers_verified_shared_runtime_and_recovers_from_damage() {
        use crate::model_verification::{Check, Status};
        let root = std::env::temp_dir().join(format!("punct-mac-runtime-{}", crate::store::new_id()));
        let name = MAC_RUNTIME_MEMBERS[0].1.relative;
        let pins = [PinnedFile { relative: name, bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad" }];
        let legacy = path(&root).join(name);
        assert_eq!(mac_runtime_library(&root, Check::Blocking, &pins), legacy);
        fs::create_dir_all(path(&root)).unwrap();
        fs::write(&legacy, b"abc").unwrap();
        let shared = optional_asr::runtime_dir(&root);
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join(name), b"abc").unwrap();
        assert_eq!(mac_runtime_library(&root, Check::Blocking, &pins), shared.join(name));
        fs::write(shared.join(name), b"bad").unwrap();
        let resolved = mac_runtime_library(&root, Check::Blocking, &pins);
        assert_eq!(resolved, legacy);
        assert_eq!(Check::Blocking.file(&root, &resolved, pins[0].bytes, pins[0].sha256), Status::Ready);
        assert_eq!(mac_runtime_library(&root, Check::Background, &pins), legacy);
        // Repairing the shared copy restores preference for it.
        fs::write(shared.join(name), b"abc").unwrap();
        assert_eq!(mac_runtime_library(&root, Check::Blocking, &pins), shared.join(name));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pins_match_the_diarization_brief() {
        assert_eq!(ID, "nemotron-3-diarization");
        assert_eq!(REPO, "joosthel/Nemotron-3-Diarization-ONNX");
        assert_eq!(REVISION, "4a911fc3ca821b76a99fffd5ce5135bd1efca540");
        assert_eq!(package_bytes(), 103_871_648);
        let expected = [
            (
                "preprocessor_core.onnx",
                135_897,
                "28e815a770a8cbe47c351cd36908dd129a0b36ab31461aa8f370a369419703e1",
            ),
            (
                "model.int8.onnx",
                103_726_888,
                "17fd04215d655d2feee413a44f96e57d4ebd96cdd9cdbdf8c26da266e08bc835",
            ),
            (
                "constants.npz",
                6_244,
                "fd49737a61e90e1990f4e90a3c0088f2f3ef649860ecdaa4abc76ad2428876df",
            ),
            (
                "LICENSE",
                2_619,
                "2ab44b68365473c112f5092211a38f231cb23e50de68b75a13369adbd76a74df",
            ),
        ];
        assert_eq!(FILES.len(), expected.len());
        for (file, (name, bytes, sha256)) in FILES.iter().zip(expected) {
            assert_eq!(
                (file.relative, file.bytes, file.sha256),
                (name, bytes, sha256)
            );
        }
    }

    #[test]
    fn missing_or_incomplete_files_and_runtime_are_not_installed() {
        let root =
            std::env::temp_dir().join(format!("sonojot-diar-model-{}", crate::store::new_id()));
        assert_eq!(path(&root), root.join("optional").join(ID));
        assert!(!installed(&root));
        assert!(!files_ready(&root));
        fs::create_dir_all(path(&root)).unwrap();
        for file in FILES {
            fs::write(path(&root).join(file.relative), b"incomplete").unwrap();
        }
        assert!(!files_ready(&root));
        assert!(!installed(&root));
        let runtime = if MAC {
            MAC_RUNTIME_ARCHIVE_BYTES
        } else {
            optional_asr::runtime_download_bytes(&root)
        };
        assert_eq!(download_bytes(&root), package_bytes() + runtime);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn mac_runtime_extraction_rejects_archives_without_the_pinned_library() {
        use std::sync::atomic::AtomicBool;
        let root =
            std::env::temp_dir().join(format!("sonojot-ort-mac-{}", crate::store::new_id()));
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("runtime.tgz");
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        ));
        let body = b"not the library";
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_cksum();
        builder
            .append_data(&mut header, MAC_RUNTIME_MEMBERS[0].0, &body[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let error = extract_mac_runtime(&archive, &root, &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("unexpected"), "{error}");
        let cancelled = extract_mac_runtime(&archive, &root, &AtomicBool::new(true)).unwrap_err();
        assert_eq!(cancelled, "Install cancelled.");
        fs::remove_dir_all(root).unwrap();
    }

    /// The real Microsoft archive, which cannot be exercised on this platform
    /// otherwise. Set `SONOJOT_ORT_OSX_TGZ` to a downloaded copy.
    #[test]
    #[ignore]
    fn real_mac_runtime_archive_unpacks_the_pinned_library() {
        use std::sync::atomic::AtomicBool;
        let archive = std::path::PathBuf::from(std::env::var_os("SONOJOT_ORT_OSX_TGZ").unwrap());
        assert!(optional_asr::file_matches(
            &archive,
            &PinnedFile {
                relative: "onnxruntime.tgz",
                bytes: MAC_RUNTIME_ARCHIVE_BYTES,
                sha256: MAC_RUNTIME_ARCHIVE_SHA256,
            },
        )
        .unwrap());
        let root =
            std::env::temp_dir().join(format!("sonojot-ort-mac-{}", crate::store::new_id()));
        fs::create_dir_all(&root).unwrap();
        extract_mac_runtime(&archive, &root, &AtomicBool::new(false)).unwrap();
        let mut names: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["ONNXRUNTIME-LICENSE", "libonnxruntime.1.28.2.dylib"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(any(
        all(any(windows, target_os = "linux"), target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    #[test]
    fn unsupported_platform_never_reports_installed() {
        assert!(!AVAILABLE);
        assert!(!installed(Path::new("missing-models")));
    }
}
