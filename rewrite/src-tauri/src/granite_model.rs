//! Optional meeting-only Granite NAR package. All downloaded and extracted
//! files have source-controlled pins; native loading never trusts disk receipts.
//! Source URLs, licenses and compatibility evidence: docs/GraniteRuntime.md.

use crate::{
    model_verification::{self, Check, Status},
    optional_asr::{self, PinnedFile},
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) fn path(root: &Path) -> PathBuf {
    optional_asr::optional_root(root).join(GRANITE_ID)
}

pub(crate) fn model_path(root: &Path) -> PathBuf {
    path(root).join(MODEL.relative)
}

pub(crate) fn runtime_dir(root: &Path) -> PathBuf {
    path(root).join("runtime")
}

/// Cheap catalog qualification. The isolated worker still verifies that the
/// requested GPU backend actually initialized; a probe is not an inference test.
pub(crate) fn supported() -> bool {
    unavailable_reason().is_none()
}

pub(crate) fn unavailable_reason() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        let gpu = crate::resource_manager::setup_hardware().gpu;
        windows_unavailable_reason(gpu.as_ref())
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        None
    } else {
        Some("Granite meetings require Windows x64 with NVIDIA CUDA or an Apple Silicon Mac.")
    }
}

fn version_pair(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn windows_unavailable_reason(
    gpu: Option<&crate::resource_manager::GpuHardwareRow>,
) -> Option<&'static str> {
    let Some(gpu) = gpu else {
        return Some("Granite meetings need an NVIDIA graphics card and its CUDA driver. No usable NVIDIA device was detected.");
    };
    match gpu.compute_capability.as_deref().and_then(version_pair) {
        Some(capability) if capability >= (7, 5) => {},
        Some(_) => return Some("This NVIDIA graphics card is too old for Granite meetings. CUDA compute capability 7.5 or newer is required."),
        None => return Some("The NVIDIA graphics card's CUDA capability could not be checked. Update the NVIDIA driver and try again."),
    }
    // This release is built by CUDA 12.9 and can JIT PTX on newer GPUs. Use
    // its toolkit driver floor rather than promise minor-version/PTX fallback.
    match gpu.driver_version.as_deref().and_then(version_pair) {
        Some(driver) if driver >= (576, 2) => None,
        _ => Some("Granite meetings need NVIDIA driver 576.02 or newer. Update the NVIDIA driver and try again."),
    }
}

pub(crate) fn downloads() -> Vec<&'static Download> {
    downloads_for(cfg!(all(windows, target_arch = "x86_64")))
}

fn downloads_for(windows: bool) -> Vec<&'static Download> {
    let mut files: Vec<_> = MODEL_DOWNLOADS.iter().collect();
    if windows {
        files.extend([&WINDOWS_RUNTIME, &CUDA_RUNTIME, &CUBLAS]);
    } else {
        files.push(&MAC_RUNTIME);
    }
    files
}

fn installed_pins(downloads: &[&Download]) -> Vec<PinnedFile> {
    downloads
        .iter()
        .flat_map(|download| {
            if download.format == Format::File {
                vec![download.pin]
            } else {
                download.members.iter().map(|member| member.pin).collect()
            }
        })
        .collect()
}

pub(crate) fn download_bytes() -> u64 {
    downloads().iter().map(|download| download.pin.bytes).sum()
}

pub(crate) fn package_bytes() -> u64 {
    installed_pins(&downloads())
        .iter()
        .map(|pin| pin.bytes)
        .sum()
}

pub(crate) fn model_status(root: &Path, check: Check) -> Status {
    let pins = installed_pins(&downloads());
    let directory = path(root);
    if !directory.is_dir() || validate_runtime_files(&directory, &pins).is_err() {
        return Status::Missing;
    }
    check.files(root, &directory, &pins)
}

pub(crate) fn installed(root: &Path) -> bool {
    model_status(root, Check::Blocking).ready()
}

/// Called immediately before native loading in the worker. Disk receipts are
/// useful for catalog polling, but are not an executable trust boundary.
pub(crate) fn verify_for_load(root: &Path) -> Result<(), String> {
    if let Some(reason) = unavailable_reason() {
        return Err(reason.into());
    }
    let directory = path(root);
    optional_asr::ensure_within(root, &directory)?;
    verify_package(&directory, &AtomicBool::new(false))
}

pub(crate) fn verify_package(directory: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let pins = installed_pins(&downloads());
    validate_runtime_files(directory, &pins)?;
    verify_pins(directory, &pins, cancel)
}

fn verify_pins(directory: &Path, pins: &[PinnedFile], cancel: &AtomicBool) -> Result<(), String> {
    for pin in pins {
        check_cancel(cancel)?;
        let file = directory.join(pin.relative);
        optional_asr::ensure_within(directory, &file)?;
        if !model_verification::uncached(&file, pin.bytes, pin.sha256)? {
            return Err(format!(
                "Granite's {} is missing or damaged. Reinstall Granite Speech 4.1 NAR from Models.",
                pin.relative
            ));
        }
    }
    Ok(())
}

// ggml discovers backend libraries beside transcribe. Reject extra files so an
// editable receipt or an unpinned DLL cannot authorize another native backend.
fn validate_runtime_files(directory: &Path, pins: &[PinnedFile]) -> Result<(), String> {
    let mut pending = vec![directory.join("runtime")];
    while let Some(current) = pending.pop() {
        optional_asr::ensure_within(directory, &current)?;
        for entry in
            fs::read_dir(&current).map_err(|e| format!("Cannot read Granite's runtime: {e}"))?
        {
            let entry = entry.map_err(|e| format!("Cannot read Granite's runtime: {e}"))?;
            let file = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("Cannot inspect Granite's runtime: {e}"))?;
            if kind.is_dir() && !kind.is_symlink() {
                let relative = file.strip_prefix(directory).map_err(|e| e.to_string())?;
                if !pins
                    .iter()
                    .any(|pin| Path::new(pin.relative).starts_with(relative))
                {
                    return Err("Granite's runtime contains an unexpected folder. Reinstall it from Models.".into());
                }
                pending.push(file);
            } else if !kind.is_file()
                || !pins.iter().any(|pin| directory.join(pin.relative) == file)
            {
                return Err(
                    "Granite's runtime contains an unexpected file. Reinstall it from Models."
                        .into(),
                );
            }
        }
    }
    Ok(())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Install cancelled.".into())
    } else {
        Ok(())
    }
}

pub(crate) fn extract(
    download: &Download,
    archive: &Path,
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if !model_verification::uncached(archive, download.pin.bytes, download.pin.sha256)? {
        return Err(
            "The Granite runtime archive did not match its pinned checksum. Nothing was installed."
                .into(),
        );
    }
    match download.format {
        Format::TarGz => extract_tar(archive, directory, download.members, cancel),
        Format::Zip => extract_zip(archive, directory, download.members, cancel),
        Format::File => Err("A Granite model file cannot be extracted as an archive.".into()),
    }
}

fn extract_tar(
    archive: &Path,
    directory: &Path,
    members: &[Member],
    cancel: &AtomicBool,
) -> Result<(), String> {
    let file = fs::File::open(archive)
        .map_err(|e| format!("Cannot open Granite's runtime archive: {e}"))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut found = std::collections::HashSet::new();
    for entry in tar
        .entries()
        .map_err(|e| format!("Cannot read Granite's runtime archive: {e}"))?
    {
        check_cancel(cancel)?;
        let mut entry = entry.map_err(|e| format!("Cannot read Granite's runtime archive: {e}"))?;
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let Some(member) = members.iter().find(|member| member.archive_path == name) else {
            continue;
        };
        if !found.insert(member.archive_path)
            || entry.header().entry_type() != tar::EntryType::Regular
            || entry.size() != member.pin.bytes
        {
            return Err(format!(
                "Granite's archive has an unexpected {}.",
                member.pin.relative
            ));
        }
        extract_member(&mut entry, directory, &member.pin, cancel)?;
    }
    require_members(found.len(), members.len())
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
fn extract_zip(
    archive: &Path,
    directory: &Path,
    members: &[Member],
    cancel: &AtomicBool,
) -> Result<(), String> {
    let file =
        fs::File::open(archive).map_err(|e| format!("Cannot open Granite's CUDA archive: {e}"))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("Cannot read Granite's CUDA archive: {e}"))?;
    let mut found = std::collections::HashSet::new();
    for index in 0..zip.len() {
        check_cancel(cancel)?;
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("Cannot read Granite's CUDA archive: {e}"))?;
        let Some(member) = members
            .iter()
            .find(|member| member.archive_path == entry.name())
        else {
            continue;
        };
        let special = entry
            .unix_mode()
            .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000));
        if !found.insert(member.archive_path)
            || entry.is_dir()
            || special
            || entry.size() != member.pin.bytes
        {
            return Err(format!(
                "Granite's CUDA archive has an unexpected {}.",
                member.pin.relative
            ));
        }
        extract_member(&mut entry, directory, &member.pin, cancel)?;
    }
    require_members(found.len(), members.len())
}

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
fn extract_zip(_: &Path, _: &Path, _: &[Member], _: &AtomicBool) -> Result<(), String> {
    Err("Granite's CUDA package requires Windows x64.".into())
}

fn require_members(found: usize, expected: usize) -> Result<(), String> {
    if found == expected {
        Ok(())
    } else {
        Err("Granite's runtime archive is incomplete. Nothing was installed.".into())
    }
}

fn extract_member(
    reader: &mut impl Read,
    directory: &Path,
    pin: &PinnedFile,
    cancel: &AtomicBool,
) -> Result<(), String> {
    // Destinations come only from the compiled manifest, never archive paths.
    let dest = directory.join(pin.relative);
    optional_asr::ensure_within(directory, &dest)?;
    fs::create_dir_all(dest.parent().ok_or("Invalid Granite runtime path.")?)
        .map_err(|e| format!("Cannot prepare Granite's runtime: {e}"))?;
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest)
        .map_err(|e| format!("Cannot unpack {}: {e}", pin.relative))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut written = 0u64;
    loop {
        check_cancel(cancel)?;
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("Cannot unpack {}: {e}", pin.relative))?;
        if read == 0 {
            break;
        }
        written += read as u64;
        if written > pin.bytes {
            return Err(format!(
                "Granite's {} is larger than expected.",
                pin.relative
            ));
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save {}: {e}", pin.relative))?;
    }
    out.flush()
        .map_err(|e| format!("Cannot save {}: {e}", pin.relative))?;
    drop(out);
    check_cancel(cancel)?;
    if written != pin.bytes || !model_verification::uncached(&dest, pin.bytes, pin.sha256)? {
        return Err(format!(
            "Granite's unpacked {} did not match its pinned checksum.",
            pin.relative
        ));
    }
    #[cfg(unix)]
    if pin.relative.ends_with(".dylib") {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) const CUDA_RUNTIME: Download = Download {
    pin: PinnedFile { relative: "nvidia_cuda_runtime_cu12-12.8.90-py3-none-win_amd64.whl", bytes: 944318, sha256: "c0c6027f01505bfed6c3b21ec546f69c687689aad5f1a377554bc6ca4aa993a8" },
    url: "https://files.pythonhosted.org/packages/30/a5/a515b7600ad361ea14bfa13fb4d6687abf500adc270f19e89849c0590492/nvidia_cuda_runtime_cu12-12.8.90-py3-none-win_amd64.whl",
    format: Format::Zip,
    members: &[
        Member {
            archive_path: "nvidia/cuda_runtime/bin/cudart64_12.dll",
            pin: PinnedFile { relative: "runtime/cudart64_12.dll", bytes: 573952, sha256: "c2c9a9c22a9bcba90e261825968836787b331038047a26770cffb7a583c28344" },
        },
        Member {
            archive_path: "nvidia_cuda_runtime_cu12-12.8.90.dist-info/License.txt",
            pin: PinnedFile { relative: "runtime/licenses/NVIDIA-CUDA-RUNTIME-LICENSE.txt", bytes: 59262, sha256: "ad6f5853fba0ca0d159d0f58d49ae49830c2f8c93f7a92648b9ce90adb4c6ccd" },
        },
    ],
};

pub(crate) const CUBLAS: Download = Download {
    pin: PinnedFile { relative: "nvidia_cublas_cu12-12.8.4.1-py3-none-win_amd64.whl", bytes: 567544208, sha256: "47e9b82132fa8d2b4944e708049229601448aaad7e6f296f630f2d1a32de35af" },
    url: "https://files.pythonhosted.org/packages/70/61/7d7b3c70186fb651d0fbd35b01dbfc8e755f69fd58f817f3d0f642df20c3/nvidia_cublas_cu12-12.8.4.1-py3-none-win_amd64.whl",
    format: Format::Zip,
    members: &[
        Member {
            archive_path: "nvidia/cublas/bin/cublas64_12.dll",
            pin: PinnedFile { relative: "runtime/cublas64_12.dll", bytes: 113716224, sha256: "9513540e4ec4c51ee9e7304138c2cc255c29a8c181f9e80c38efa25738becd99" },
        },
        Member {
            archive_path: "nvidia/cublas/bin/cublasLt64_12.dll",
            pin: PinnedFile { relative: "runtime/cublasLt64_12.dll", bytes: 674667520, sha256: "b199d1ff892a81b7fd3d57ba1781549609b41500b36008fef326038393ad46c7" },
        },
        Member {
            archive_path: "nvidia_cublas_cu12-12.8.4.1.dist-info/License.txt",
            pin: PinnedFile { relative: "runtime/licenses/NVIDIA-CUBLAS-LICENSE.txt", bytes: 59262, sha256: "ad6f5853fba0ca0d159d0f58d49ae49830c2f8c93f7a92648b9ce90adb4c6ccd" },
        },
    ],
};

pub(crate) const GRANITE_ID: &str = "granite-speech-4.1-2b-nar-q8";
pub(crate) const NAME: &str = "Granite Speech 4.1 NAR";
pub(crate) const REVISION: &str = "c186f8d7ed6c67b947b90fd8ed9587be971766fa";
pub(crate) const RUNTIME_REVISION: &str = "3f32fbcc7bb3246851a0234263438bc3c0fa1cac";
pub(crate) const MODEL: PinnedFile = PinnedFile {
    relative: "granite-speech-4.1-2b-nar-Q8_0.gguf",
    bytes: 2_498_105_472,
    sha256: "4762df797b9efa1ce0d73d2f5128517c86fdfdba2e9d00cd5cca97e624760b21",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    File,
    TarGz,
    Zip,
}

pub(crate) struct Member {
    pub archive_path: &'static str,
    /// Destination relative to the model package directory.
    pub pin: PinnedFile,
}

pub(crate) struct Download {
    pub pin: PinnedFile,
    pub url: &'static str,
    pub format: Format,
    pub members: &'static [Member],
}

pub(crate) const MODEL_DOWNLOADS: &[Download] = &[
    Download {
        pin: MODEL,
        url: "https://huggingface.co/handy-computer/granite-speech-4.1-2b-nar-gguf/resolve/c186f8d7ed6c67b947b90fd8ed9587be971766fa/granite-speech-4.1-2b-nar-Q8_0.gguf",
        format: Format::File, members: &[],
    },
    Download {
        pin: PinnedFile { relative: "MODEL-CARD.md", bytes: 14199, sha256: "02b9c6231b065b70caa78a77b583ecce59f07ef3c83c661111b74ecb23ff6b4e" },
        url: "https://huggingface.co/handy-computer/granite-speech-4.1-2b-nar-gguf/resolve/c186f8d7ed6c67b947b90fd8ed9587be971766fa/README.md",
        format: Format::File, members: &[],
    },
    Download {
        pin: PinnedFile { relative: "IBM-MODEL-CARD.md", bytes: 9287, sha256: "e4b15fab1cd6619ad0d5fab77fa4f129d0f50fa090c329b05622343e62704541" },
        url: "https://huggingface.co/ibm-granite/granite-speech-4.1-2b-nar/resolve/a1e3416e25ce29ab3852778e54fa8b3bd59c4bf2/README.md",
        format: Format::File, members: &[],
    },
    Download {
        pin: PinnedFile { relative: "MODEL-LICENSE.txt", bytes: 11338, sha256: "be592609e2a9f551c04ec455e082cd41a8ef9b3663feefb4936ec3157509c31b" },
        url: "https://raw.githubusercontent.com/ibm-granite/granite-speech/2fa56f2ce5d2ef83b0f2280650320aa8d97c2c6d/LICENSE",
        format: Format::File, members: &[],
    },
];

pub(crate) const WINDOWS_RUNTIME: Download = Download {
    pin: PinnedFile { relative: "transcribe-native-0.3.1-windows-x86_64-cuda.tar.gz", bytes: 200583487, sha256: "8212f8ee472cd87e027317b50c8705d947f7e8ef209061b5a6ccc2748556422a" },
    url: "https://github.com/handy-computer/transcribe.cpp/releases/download/v0.3.1/transcribe-native-0.3.1-windows-x86_64-cuda.tar.gz",
    format: Format::TarGz,
    members: &[
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/contract.json",
            pin: PinnedFile { relative: "runtime/contract.json", bytes: 125, sha256: "b4f84cd1bb78c4368994c07ba829293c912a1056eb91a899f91fe9019bf7bdc7" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/ggml-base.dll",
            pin: PinnedFile { relative: "runtime/ggml-base.dll", bytes: 670208, sha256: "c2dcbf222f9e0bc186b2548fb92e0ce5a78049c382463f3f214b0a41cf94430a" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/ggml-cpu.dll",
            pin: PinnedFile { relative: "runtime/ggml-cpu.dll", bytes: 845824, sha256: "77cf62b9330328a37026bc098c09191ccb7a246c3df7e783bb010c1b116937d3" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/ggml-cuda.dll",
            pin: PinnedFile { relative: "runtime/ggml-cuda.dll", bytes: 199469056, sha256: "d58ba63c633345e40f83a0d739f14dc0726bc1196c3e338a850751c0ad4dbc89" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/ggml.dll",
            pin: PinnedFile { relative: "runtime/ggml.dll", bytes: 61952, sha256: "450b405b8232c4e596663461d03cc3e7d6f1dd2b32b7f4a715cf98e1d8ee373d" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/licenses/LICENSE",
            pin: PinnedFile { relative: "runtime/licenses/LICENSE", bytes: 1104, sha256: "acef7d1711847656cdeb0007784dbe19bd7fd069767b0b38bd62714c6a8f3159" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/licenses/LICENSE-ggml",
            pin: PinnedFile { relative: "runtime/licenses/LICENSE-ggml", bytes: 1099, sha256: "bcd8ec749126d45cb06737d0690295d73df4b6e7e194205bcf91190368f27285" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/licenses/LICENSE-miniz",
            pin: PinnedFile { relative: "runtime/licenses/LICENSE-miniz", bytes: 1184, sha256: "0115478d567121238cf6cc1c0c361926cf07a49d9e4c9e66da97fac6a01646b3" },
        },
        Member {
            archive_path: "transcribe-native-windows-x86_64-cuda/transcribe.dll",
            pin: PinnedFile { relative: "runtime/transcribe.dll", bytes: 1881600, sha256: "1b7f4b8dfb5ddbd19b19c7145140825f3571adcae92a547fd3676275f3a06eb1" },
        },
    ],
};

pub(crate) const MAC_RUNTIME: Download = Download {
    pin: PinnedFile { relative: "transcribe-native-0.3.1-macos-arm64-metal.tar.gz", bytes: 1811963, sha256: "904adfa090946c1f4902e7117c03cf245e45c041a370c2ea5547eccf8d00126f" },
    url: "https://github.com/handy-computer/transcribe.cpp/releases/download/v0.3.1/transcribe-native-0.3.1-macos-arm64-metal.tar.gz",
    format: Format::TarGz,
    members: &[
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/contract.json",
            pin: PinnedFile { relative: "runtime/contract.json", bytes: 113, sha256: "fd2b9c6068d94507f7689af2272b2e77d63a385b30b381bb400f3217e0f0e620" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/libggml-base.dylib",
            pin: PinnedFile { relative: "runtime/libggml-base.dylib", bytes: 657808, sha256: "b06cce265869e5252c77bdc5680ae488b1afaca660981af3099981e7321e0572" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/libggml-cpu.dylib",
            pin: PinnedFile { relative: "runtime/libggml-cpu.dylib", bytes: 968800, sha256: "43fcd6b6299528043d4b2b71c06ea92ebe0d8ded8741ccb005adef8027886671" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/libggml-metal.dylib",
            pin: PinnedFile { relative: "runtime/libggml-metal.dylib", bytes: 2149120, sha256: "65315a42bf62837d4b4b66e9c161fcc71e6e82e7d9200e815f2d06fb1a251df1" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/libggml.dylib",
            pin: PinnedFile { relative: "runtime/libggml.dylib", bytes: 90976, sha256: "1b9af2350ae6207b09b1e3c2e5b66a13049bdf16d833aa966207a363b5cefbc2" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/libtranscribe.dylib",
            pin: PinnedFile { relative: "runtime/libtranscribe.dylib", bytes: 1515472, sha256: "fa8233a42f37fc14b21ab1cae0f81bfc22eec2f198f5b11e1f4f7b5f4c281fdf" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/licenses/LICENSE",
            pin: PinnedFile { relative: "runtime/licenses/LICENSE", bytes: 1083, sha256: "86a53633b56f6b029d3cb42158bcc7aac0cdff898aceb13e83b93e368bbc4ac6" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/licenses/ggml/LICENSE",
            pin: PinnedFile { relative: "runtime/licenses/ggml/LICENSE", bytes: 1078, sha256: "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d" },
        },
        Member {
            archive_path: "transcribe-native-macos-arm64-metal/licenses/src/third_party/miniz/LICENSE",
            pin: PinnedFile { relative: "runtime/licenses/src/third_party/miniz/LICENSE", bytes: 1162, sha256: "6f20fa7672b00e2e975c291df737cf227addf3ad32e36fef3fe0f416e4664d3d" },
        },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const FIXTURE: &[Member] = &[Member {
        archive_path: "native/transcribe.dll",
        pin: PinnedFile {
            relative: "runtime/transcribe.dll",
            bytes: 3,
            sha256: ABC,
        },
    }];

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("sonojot-granite-pins-{}", crate::store::new_id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn dest(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn catalog_pins_are_complete_and_runtime_is_private() {
        assert_eq!(GRANITE_ID, "granite-speech-4.1-2b-nar-q8");
        assert_eq!(MODEL.bytes, 2_498_105_472);
        assert_eq!(
            MODEL.sha256,
            "4762df797b9efa1ce0d73d2f5128517c86fdfdba2e9d00cd5cca97e624760b21"
        );
        assert_eq!(REVISION.len(), 40);
        assert_eq!(RUNTIME_REVISION, "3f32fbcc7bb3246851a0234263438bc3c0fa1cac");
        assert_eq!(
            WINDOWS_RUNTIME.pin.sha256,
            "8212f8ee472cd87e027317b50c8705d947f7e8ef209061b5a6ccc2748556422a"
        );
        assert_eq!(
            MAC_RUNTIME.pin.sha256,
            "904adfa090946c1f4902e7117c03cf245e45c041a370c2ea5547eccf8d00126f"
        );
        for windows in [false, true] {
            let downloads = downloads_for(windows);
            let pins = installed_pins(&downloads);
            let mut names = std::collections::HashSet::new();
            for pin in &pins {
                assert!(names.insert(pin.relative));
                assert!(pin.bytes > 0);
                assert_eq!(pin.sha256.len(), 64);
                assert!(pin.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(Path::new(pin.relative)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))));
            }
            for download in downloads {
                assert!(download.url.starts_with("https://"));
                assert!(!download.url.contains("/main/") && !download.url.contains("latest"));
                assert_eq!(download.pin.sha256.len(), 64);
            }
            assert!(pins.iter().any(|pin| pin.relative == "MODEL-LICENSE.txt"));
            assert!(pins.iter().any(|pin| pin.relative == "MODEL-CARD.md"));
            assert!(pins
                .iter()
                .any(|pin| pin.relative.ends_with("licenses/LICENSE")));
            assert!(!pins
                .iter()
                .any(|pin| pin.relative.contains("vulkan") || pin.relative.contains("nvblas")));
            assert_eq!(
                pins.iter()
                    .filter(|pin| pin.relative.ends_with("64_12.dll"))
                    .count(),
                if windows { 3 } else { 0 }
            );
        }
        let root = Path::new("model-root");
        assert_eq!(
            model_path(root),
            root.join("optional").join(GRANITE_ID).join(MODEL.relative)
        );
        assert_eq!(
            runtime_dir(root),
            root.join("optional").join(GRANITE_ID).join("runtime")
        );
    }

    #[test]
    fn cuda_wheels_match_the_measured_cuda12_libraries() {
        assert_eq!(
            CUDA_RUNTIME.pin.sha256,
            "c0c6027f01505bfed6c3b21ec546f69c687689aad5f1a377554bc6ca4aa993a8"
        );
        assert_eq!(
            CUBLAS.pin.sha256,
            "47e9b82132fa8d2b4944e708049229601448aaad7e6f296f630f2d1a32de35af"
        );
        let pins = installed_pins(&[&CUDA_RUNTIME, &CUBLAS]);
        for (name, hash) in [
            (
                "runtime/cudart64_12.dll",
                "c2c9a9c22a9bcba90e261825968836787b331038047a26770cffb7a583c28344",
            ),
            (
                "runtime/cublas64_12.dll",
                "9513540e4ec4c51ee9e7304138c2cc255c29a8c181f9e80c38efa25738becd99",
            ),
            (
                "runtime/cublasLt64_12.dll",
                "b199d1ff892a81b7fd3d57ba1781549609b41500b36008fef326038393ad46c7",
            ),
        ] {
            assert_eq!(
                pins.iter().find(|pin| pin.relative == name).unwrap().sha256,
                hash
            );
        }
        assert_eq!(
            pins.iter()
                .filter(|pin| pin.relative.contains("LICENSE"))
                .count(),
            2
        );
    }

    #[test]
    fn windows_without_supported_cuda_has_a_plain_reason() {
        use crate::resource_manager::GpuHardwareRow;
        assert!(windows_unavailable_reason(None).unwrap().contains("NVIDIA"));
        let mut gpu = GpuHardwareRow {
            index: 0,
            total_bytes: None,
            name: None,
            driver_version: Some("610.88".into()),
            compute_capability: Some("12.0".into()),
        };
        assert_eq!(windows_unavailable_reason(Some(&gpu)), None);
        gpu.driver_version = Some("576.02".into());
        gpu.compute_capability = Some("7.5".into());
        assert_eq!(windows_unavailable_reason(Some(&gpu)), None);
        gpu.driver_version = Some("575.99".into());
        assert!(windows_unavailable_reason(Some(&gpu))
            .unwrap()
            .contains("576.02"));
        gpu.compute_capability = Some("6.1".into());
        assert!(windows_unavailable_reason(Some(&gpu))
            .unwrap()
            .contains("too old"));
        gpu.compute_capability = None;
        assert!(windows_unavailable_reason(Some(&gpu))
            .unwrap()
            .contains("could not be checked"));
    }

    fn tar_fixture(path: &Path, content: &[u8], duplicate: bool, symlink: bool) {
        let file = fs::File::create(path).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        for _ in 0..if duplicate { 2 } else { 1 } {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            if symlink {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("outside.dll").unwrap();
                header.set_size(0);
                header.set_cksum();
                builder
                    .append_data(&mut header, FIXTURE[0].archive_path, std::io::empty())
                    .unwrap();
            } else {
                header.set_size(content.len() as u64);
                header.set_cksum();
                builder
                    .append_data(&mut header, FIXTURE[0].archive_path, content)
                    .unwrap();
            }
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    #[test]
    fn extracted_runtime_requires_size_hash_and_regular_unique_members() {
        let fixture = Fixture::new();
        let archive = fixture.0.join("native.tgz");
        let cancel = AtomicBool::new(false);
        tar_fixture(&archive, b"abc", false, false);
        let good = fixture.dest("good");
        extract_tar(&archive, &good, FIXTURE, &cancel).unwrap();
        assert_eq!(
            fs::read(good.join(FIXTURE[0].pin.relative)).unwrap(),
            b"abc"
        );
        for (name, bytes, duplicate, symlink) in [
            ("bad-hash", b"xyz".as_slice(), false, false),
            ("bad-size", b"short".as_slice(), false, false),
            ("duplicate", b"abc".as_slice(), true, false),
            ("symlink", b"".as_slice(), false, true),
        ] {
            tar_fixture(&archive, bytes, duplicate, symlink);
            assert!(
                extract_tar(&archive, &fixture.dest(name), FIXTURE, &cancel).is_err(),
                "{name}"
            );
        }
        assert!(extract(
            &WINDOWS_RUNTIME,
            &archive,
            &fixture.dest("bad-archive"),
            &cancel
        )
        .unwrap_err()
        .contains("checksum"));
    }

    #[test]
    fn load_verification_rejects_modified_content_and_extra_backends() {
        let fixture = Fixture::new();
        let pins = [FIXTURE[0].pin];
        fs::create_dir(fixture.0.join("runtime")).unwrap();
        let file = fixture.0.join(pins[0].relative);
        fs::write(&file, b"abc").unwrap();
        assert!(Check::Blocking.files(&fixture.0, &fixture.0, &pins).ready());
        verify_pins(&fixture.0, &pins, &AtomicBool::new(false)).unwrap();
        validate_runtime_files(&fixture.0, &pins).unwrap();
        // Same length and a prior successful catalog receipt do not authorize
        // changed executable bytes at the uncached activation boundary.
        fs::write(&file, b"xyz").unwrap();
        assert!(verify_pins(&fixture.0, &pins, &AtomicBool::new(false)).is_err());
        fs::write(&file, b"abc").unwrap();
        fs::write(fixture.0.join("runtime/ggml-unknown.dll"), b"abc").unwrap();
        assert!(validate_runtime_files(&fixture.0, &pins)
            .unwrap_err()
            .contains("unexpected file"));
    }

    #[test]
    fn missing_members_and_cancelled_extraction_are_not_installed() {
        let fixture = Fixture::new();
        assert!(!installed(&fixture.0));
        assert!(require_members(0, FIXTURE.len()).is_err());
        let archive = fixture.0.join("native.tgz");
        tar_fixture(&archive, b"abc", false, false);
        let dest = fixture.dest("cancelled");
        assert_eq!(
            extract_tar(&archive, &dest, FIXTURE, &AtomicBool::new(true)).unwrap_err(),
            "Install cancelled."
        );
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    fn cuda_wheel_extracts_only_pinned_members_and_checks_their_hashes() {
        let fixture = Fixture::new();
        let archive = fixture.0.join("cuda.whl");
        for (name, bytes) in [("good", b"abc"), ("bad", b"xyz")] {
            let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file(FIXTURE[0].archive_path, options).unwrap();
            writer.write_all(bytes).unwrap();
            writer.start_file("unused/extra.dll", options).unwrap();
            writer.write_all(b"extra").unwrap();
            writer.finish().unwrap();
            let dest = fixture.dest(name);
            let result = extract_zip(&archive, &dest, FIXTURE, &AtomicBool::new(false));
            assert_eq!(result.is_ok(), name == "good");
            assert!(!dest.join("unused").exists());
        }
    }
}
