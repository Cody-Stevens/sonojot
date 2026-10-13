//! Media import support: a pinned, checksummed FFmpeg install.
//!
//! Windows x64 installs BtbN's LGPL shared build on request after consent.
//! macOS ARM64 installs the Studio LGPL build with its published corresponding source.
//! Both platforms require consent; user-supplied converters remain supported.
//! The package is fetched
//! over HTTPS with an allowlisted redirect hop, staged inside app data,
//! verified by exact size and SHA-256, unpacked with traversal and link checks
//! under a byte budget, proven by `ffmpeg -version`, and activated by a
//! same-directory rename. `converter.json` is rewritten only after every step
//! succeeds, so a failed or retried install never disturbs a working converter
//! and never half-activates a package. The download runs on a blocking worker
//! and never holds the capture owner, so dictation stays responsive.
use crate::{app::Runtime, setup::SetupStatus, setup_commands::require_idle};
#[cfg(managed_ffmpeg)]
use crate::{app::lock, store};
#[cfg(any(managed_ffmpeg, test))]
use std::path::Component;
#[cfg(any(managed_ffmpeg, test))]
use std::path::Path;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::State;

/// Whether this build can download and verify its own converter.
pub(crate) const AUTOMATIC: bool = cfg!(managed_ffmpeg);

/// Serialize installs; a second click reports instead of queueing a duplicate.
static INSTALL_GATE: Mutex<()> = Mutex::new(());
/// Readable without touching the gate, so Setup can show a background install.
static INSTALLING: AtomicBool = AtomicBool::new(false);

struct InstallingFlag;
impl Drop for InstallingFlag {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Release);
    }
}

pub(crate) fn installing() -> bool {
    INSTALLING.load(Ordering::Acquire)
}

#[tauri::command]
pub async fn install_media_support(
    state: State<'_, Arc<Runtime>>,
) -> Result<SetupStatus, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        install(&runtime)?;
        crate::setup_commands::inspect(&runtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn install(runtime: &Runtime) -> Result<PathBuf, String> {
    crate::download_consent::require_components(runtime, &["ffmpeg"])?;
    let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Media)?;
    let _serialized = INSTALL_GATE
        .try_lock()
        .map_err(|_| "Media support is already being installed.".to_string())?;
    // A cheap fail-fast guard only; the pointer commit rechecks under the
    // capture lock before writing converter.json.
    require_idle(&runtime.snapshot()?)?;
    INSTALLING.store(true, Ordering::Release);
    let _installing = InstallingFlag;
    install_platform(runtime)
}

#[cfg(not(managed_ffmpeg))]
fn install_platform(_runtime: &Runtime) -> Result<PathBuf, String> {
    let reason = "Automatic FFmpeg install is available on Windows x64 and Apple Silicon macOS.";
    Err(format!("{reason} Choose an existing FFmpeg with Locate converter."))
}

/// A pinned release asset. Never point these at a "latest" URL.
#[cfg(any(managed_ffmpeg, test))]
struct Package {
    url: &'static str,
    sha256: &'static str,
    bytes: u64,
    /// The archive's own top-level folder and the versioned install folder
    /// under `tools/`.
    folder: &'static str,
    builder: &'static str,
    license: &'static str,
}

// Retained month-end build; complete archive SHA-256/size verified 2026-10-12.
// GitHub does not mark the release immutable; this hash pins its exact bytes.
// Build flags and source evidence: licenses/downloads/ffmpeg-windows-20260930/.
#[cfg(any(all(windows, target_arch = "x86_64"), test))]
const WINDOWS: Package = Package {
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-30-13-08/ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0.zip",
    sha256: "7157177b8a6cb2174c1650ba8c71b363f2c78cba5330f88c4c02cf5b2b880646",
    bytes: 76_972_461,
    folder: "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0",
    builder: "https://github.com/BtbN/FFmpeg-Builds",
    license: "LGPL-3.0-or-later. See LICENSE.txt and doc/ in this folder; https://ffmpeg.org/legal.html",
};

// Studio build from exact upstream 9.0.2 source, published with complete build materials.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), test))]
const MACOS: Package = Package {
    url: "https://releases.sonojot.com/ffmpeg/9.0.2-sonojot-lgpl-1/ffmpeg-9.0.2-sonojot-lgpl-macos-arm64.zip",
    sha256: "e39430828cf2fb439c92477e85ea2b933f6c3fb3c4b759072f97aa60cbe0bf60",
    bytes: 1_834_974,
    folder: "ffmpeg-9.0.2-sonojot-lgpl-macos-arm64",
    builder: "Cody Stevens Studio LLC; https://releases.sonojot.com/ffmpeg/9.0.2-sonojot-lgpl-1/README.txt",
    license: "LGPL-2.1-or-later. See LICENSE.txt, SOURCE.txt and notices/ in this folder.",
};
/// Notices the macOS package must keep beside the converter (LGPL source offer).
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), test))]
const MACOS_NOTICES: [&str; 5] = [
    "LICENSE.txt",
    "NOTICE.txt",
    "SOURCE.txt",
    "notices/configure-command.txt",
    "notices/ffmpeg-buildconf.txt",
];
#[cfg(all(windows, target_arch = "x86_64"))]
const PACKAGE: Package = WINDOWS;
#[cfg(all(windows, target_arch = "x86_64"))]
const EXECUTABLE: &str = "ffmpeg.exe";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PACKAGE: Package = MACOS;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const EXECUTABLE: &str = "ffmpeg";
#[cfg(managed_ffmpeg)]
const MAX_PACKAGE_ENTRIES: usize = 2_000;
#[cfg(managed_ffmpeg)]
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(managed_ffmpeg)]
const DOWNLOAD_LIMIT: std::time::Duration = std::time::Duration::from_secs(15 * 60);
#[cfg(managed_ffmpeg)]
const VERIFY_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Members must stay inside the pinned archive's own root folder. Absolute,
/// parent, drive-letter and device-name components are all rejected;
/// backslashes count as separators so a member is judged the same on both platforms.
#[cfg(any(managed_ffmpeg, test))]
fn safe_member_name(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let normalized = name.replace('\\', "/");
    let mut relative = PathBuf::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_str()?;
                if text
                    .bytes()
                    .any(|b| b < 0x20 || b"<>:\"|?*".contains(&b))
                {
                    return None;
                }
                if text.ends_with('.') || text.ends_with(' ') {
                    return None;
                }
                let stem = text.split('.').next().unwrap_or_default();
                if matches!(
                    stem.to_ascii_uppercase().as_str(),
                    "CON" | "PRN" | "AUX" | "NUL"
                        | "COM1" | "COM2" | "COM3" | "COM4" | "COM5"
                        | "COM6" | "COM7" | "COM8" | "COM9"
                        | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5"
                        | "LPT6" | "LPT7" | "LPT8" | "LPT9"
                ) {
                    return None;
                }
                relative.push(part);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

/// An exact path check: every member lives under the archive's expected
/// top-level folder, which also pins the package identity to this build.
#[cfg(any(managed_ffmpeg, test))]
fn member_within_package(relative: &Path, root: &str) -> bool {
    relative
        .components()
        .next()
        .is_some_and(|first| first == Component::Normal(std::ffi::OsStr::new(root)))
}

/// Streamed SHA-256 with an exact size bound. Returns the hex digest; the
/// caller compares it against the pinned checksum.
#[cfg(any(managed_ffmpeg, test))]
fn sha256_file(path: &Path, expected_bytes: u64) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("The download could not be read: {e}"))?;
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

#[cfg(managed_ffmpeg)]
fn install_platform(runtime: &Runtime) -> Result<PathBuf, String> {
    let tools = runtime.store.root().join("tools");
    std::fs::create_dir_all(&tools)
        .map_err(|e| format!("Cannot create the app tools folder: {e}"))?;
    runtime.store.check_owned(&tools)?;
    let managed = tools.join(PACKAGE.folder);
    let existing = managed.join("bin").join(EXECUTABLE);
    // A previously verified install is reused cheaply. A corrupt or incomplete
    // one is left untouched; the retry installs into a fresh unique directory.
    let reusable = managed.exists()
        && existing.is_file()
        && check_tree(&managed).is_ok()
        && verify_executable(&existing).is_ok();
    let executable = if reusable {
        existing
    } else {
        let staging = tools.join(format!(".media-install-{}", store::new_id()));
        std::fs::create_dir(&staging)
            .map_err(|e| format!("Cannot create a staging folder: {e}"))?;
        let staged = (|| {
            let archive = staging.join("package.zip");
            download(&archive)?;
            verify_download(&archive)?;
            unpack(&archive, &staging, &PACKAGE)?;
            let _ = std::fs::remove_file(&archive);
            let tree = staging.join(PACKAGE.folder);
            check_tree(&tree)?;
            write_provenance(&tree)?;
            let executable = tree.join("bin").join(EXECUTABLE);
            verify_executable(&executable)?;
            let name = if managed.exists() {
                format!("{}-repair-{}", PACKAGE.folder, store::new_id())
            } else {
                PACKAGE.folder.to_string()
            };
            activate(&tools, &tree, &name)
        })();
        // Only this attempt's unique staging folder is cleaned on failure.
        let _ = std::fs::remove_dir_all(&staging);
        staged?.join("bin").join(EXECUTABLE)
    };
    commit_pointer(runtime, &executable)
}

/// The pointer changes under capture ownership only after every verification
/// step succeeded; a failed install never disturbs a working converter.
#[cfg(managed_ffmpeg)]
fn commit_pointer(runtime: &Runtime, executable: &Path) -> Result<PathBuf, String> {
    let _capture = lock(&runtime.control)?;
    require_idle(&runtime.snapshot()?)?;
    let bytes = serde_json::to_vec(executable).map_err(|e| e.to_string())?;
    store::atomic_write(&runtime.store.root().join("converter.json"), &bytes)?;
    drop(_capture);
    Ok(executable.to_path_buf())
}

#[cfg(managed_ffmpeg)]
fn download(archive: &Path) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::time::Instant;
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("SonoJot/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(30))
        // Total request bound: a stalled stream cannot outlive the limit.
        .timeout(DOWNLOAD_LIMIT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            // GitHub release assets are delivered from *.githubusercontent.com.
            let allowed = attempt.url().scheme() == "https"
                && attempt.url().host_str().is_some_and(|host| {
                    host == "github.com" || host.ends_with(".githubusercontent.com")
                        || host == "releases.sonojot.com"
                });
            if allowed && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|e| format!("Cannot prepare the download: {e}"))?;
    let mut response = client
        .get(PACKAGE.url)
        .send()
        .map_err(|e| format!("The media support download could not start: {e}"))?
        .error_for_status()
        .map_err(|e| format!("The media support download was refused: {e}"))?;
    if let Some(length) = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        if length != PACKAGE.bytes {
            return Err(
                "The server offered an unexpected download size. Check the pinned package details."
                    .into(),
            );
        }
    }
    let file = std::fs::File::create(archive)
        .map_err(|e| format!("Cannot stage the download: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    let deadline = Instant::now() + DOWNLOAD_LIMIT;
    let mut total = 0u64;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err(
                "The media support download exceeded the time limit. Check the connection and try again."
                    .into(),
            );
        }
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("The media support download was interrupted: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > PACKAGE.bytes {
            return Err("The media support download was larger than the pinned package.".into());
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("Cannot save the download: {e}"))?;
    }
    out.flush().map_err(|e| format!("Cannot save the download: {e}"))?;
    if total != PACKAGE.bytes {
        return Err("The media support download was incomplete. Try again.".into());
    }
    Ok(())
}

#[cfg(managed_ffmpeg)]
fn verify_download(archive: &Path) -> Result<(), String> {
    let hash = sha256_file(archive, PACKAGE.bytes)?;
    if !hash.eq_ignore_ascii_case(PACKAGE.sha256) {
        return Err(
            "The downloaded media package did not match its pinned checksum. Nothing was installed; try again."
                .into(),
        );
    }
    Ok(())
}

#[cfg(managed_ffmpeg)]
fn unpack(archive: &Path, staging: &Path, pin: &Package) -> Result<(), String> {
    use std::io::Read;
    // Archive modes are not trusted: only the pinned converter becomes executable.
    #[cfg(target_os = "macos")]
    let executable = Path::new(pin.folder).join("bin").join(EXECUTABLE);
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("The downloaded package could not be opened: {e}"))?;
    let mut package = zip::ZipArchive::new(file)
        .map_err(|e| format!("The downloaded package is not a readable archive: {e}"))?;
    if package.len() > MAX_PACKAGE_ENTRIES {
        return Err("The media package contains more files than expected.".into());
    }
    let mut unpacked = 0u64;
    for index in 0..package.len() {
        let mut entry = package
            .by_index(index)
            .map_err(|e| format!("The media package could not be read: {e}"))?;
        let Some(relative) = safe_member_name(entry.name()) else {
            return Err("The media package contains an unsafe path.".into());
        };
        if !member_within_package(&relative, pin.folder) {
            return Err("The media package contains unexpected files.".into());
        }
        // Linked entries could point outside the staging folder; refuse them.
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("The media package contains links, which are not installed.".into());
        }
        if !matches!(
            entry.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        ) {
            return Err("The media package uses an unsupported compression method.".into());
        }
        unpacked = unpacked
            .checked_add(entry.size())
            .ok_or("The media package is too large.")?;
        if unpacked > MAX_UNPACKED_BYTES {
            return Err("The media package is larger than expected.".into());
        }
        let destination = staging.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&destination)
                .map_err(|e| format!("Cannot unpack the media package: {e}"))?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot unpack the media package: {e}"))?;
        }
        let mut out = std::fs::File::create(&destination)
            .map_err(|e| format!("Cannot unpack the media package: {e}"))?;
        let expected_size = entry.size();
        let copied = std::io::copy(
            &mut entry.by_ref().take(expected_size.saturating_add(1)),
            &mut out,
        )
        .map_err(|e| format!("The media package could not be unpacked: {e}"))?;
        if copied != expected_size {
            return Err("The media package could not be unpacked completely.".into());
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if relative == executable { 0o755 } else { 0o644 };
            std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(mode))
                .map_err(|e| format!("Cannot set converter permissions: {e}"))?;
        }
    }
    Ok(())
}

/// The executable, its shared libraries and the license texts must all be
/// present before activation; `ffmpeg -version` then proves the build loads.
#[cfg(managed_ffmpeg)]
fn check_tree(root: &Path) -> Result<(), String> {
    let bin = root.join("bin");
    if !bin.join(EXECUTABLE).is_file() {
        return Err("The media package did not contain the expected converter.".into());
    }
    #[cfg(windows)]
    let names: Vec<String> = std::fs::read_dir(&bin)
        .map_err(|e| format!("The unpacked converter could not be checked: {e}"))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    #[cfg(windows)]
    for prefix in ["avcodec-", "avformat-", "avutil-"] {
        if !names
            .iter()
            .any(|name| name.starts_with(prefix) && name.ends_with(".dll"))
        {
            return Err("The media package is missing its shared libraries.".into());
        }
    }
    #[cfg(target_os = "macos")]
    for name in MACOS_NOTICES {
        if !root.join(name).is_file() {
            return Err("The media package is missing its license or source information.".into());
        }
    }
    let has_license = std::fs::read_dir(root)
        .map_err(|e| format!("The unpacked converter could not be checked: {e}"))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .any(|name| name.to_ascii_uppercase().starts_with("LICENSE"));
    if !has_license {
        return Err("The media package is missing its license text.".into());
    }
    Ok(())
}

#[cfg(managed_ffmpeg)]
fn write_provenance(root: &Path) -> Result<(), String> {
    let text = format!(
        "SonoJot media support\n\
         Package: {}\n\
         Downloaded: {}\n\
         SHA-256: {}\n\
         Build scripts: {}\n\
         License: {}\n\
         Installed: {}\n",
        PACKAGE.folder,
        PACKAGE.url,
        PACKAGE.sha256,
        PACKAGE.builder,
        PACKAGE.license,
        store::now()
    );
    std::fs::write(root.join("PROVENANCE.txt"), text)
        .map_err(|e| format!("Cannot record the media package source: {e}"))
}

/// A hidden `ffmpeg -version` with a deadline proves the extracted build and
/// its libraries actually load before the pointer changes. The version banner
/// is drained on a side thread so the pipe can never block the deadline.
#[cfg(managed_ffmpeg)]
fn verify_executable(executable: &Path) -> Result<(), String> {
    use std::io::Read;
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Instant;
    let mut command = Command::new(executable);
    command
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("The downloaded converter could not be started: {e}"))?;
    let stdout = child.stdout.take();
    let drain = std::thread::spawn(move || {
        let mut banner = Vec::new();
        if let Some(pipe) = stdout {
            let _ = pipe.take(8192).read_to_end(&mut banner);
        }
        banner
    });
    let deadline = Instant::now() + VERIFY_LIMIT;
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => {
                break Err(
                    "The downloaded converter did not run correctly. Nothing was installed.".into(),
                )
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(
                    "The downloaded converter took too long to answer. Nothing was installed."
                        .into(),
                );
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("The downloaded converter could not be checked: {e}"));
            }
        }
    };
    let banner = drain.join().unwrap_or_default();
    result?;
    if !String::from_utf8_lossy(&banner).contains("ffmpeg version") {
        return Err("The downloaded file is not a working FFmpeg. Nothing was installed.".into());
    }
    Ok(())
}

/// A same-parent rename is the atomic activation point. An existing install
/// is never removed or replaced by this action; callers reuse it instead.
#[cfg(any(managed_ffmpeg, test))]
fn activate(tools: &Path, tree: &Path, name: &str) -> Result<PathBuf, String> {
    let target = tools.join(name);
    if target.exists() {
        return Err("Media support is already installed.".into());
    }
    std::fs::rename(tree, &target)
        .map_err(|e| format!("Cannot activate the media support files: {e}"))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "sonojot-media-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn member_names_stay_inside_the_package_root() {
        let root = WINDOWS.folder;
        let good = safe_member_name(
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/bin/ffmpeg.exe",
        )
        .unwrap();
        assert!(member_within_package(&good, root));
        for name in [
            "../evil.exe",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/../evil.exe",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0\\..\\evil.exe",
            "C:/absolute/evil.exe",
            "/absolute/evil.exe",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/bin/NUL.dll",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/bin/trailing.",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/bin/trailing ",
            "other-root/bin/ffmpeg.exe",
            "ffmpeg-n9.0.2-17-g2a571b6068-win64-lgpl-shared-9.0/bin/bad:stream.dll",
        ] {
            let cleaned = safe_member_name(name);
            assert!(
                cleaned.is_none() || !member_within_package(&cleaned.unwrap(), root),
                "unsafe member accepted: {name}"
            );
        }
    }

    #[test]
    fn hash_check_enforces_exact_size_and_digest() {
        let fixture = Fixture::new();
        let file = fixture.0.join("blob.bin");
        std::fs::write(&file, b"pinned bytes").unwrap();
        use sha2::Digest;
        let digest = sha256_file(&file, 12).unwrap();
        assert_eq!(
            digest,
            format!("{:x}", sha2::Sha256::digest(b"pinned bytes"))
        );
        assert!(sha256_file(&file, 11).is_err());
        assert!(sha256_file(&file, 13).is_err());
        assert!(sha256_file(&fixture.0.join("missing.bin"), 0).is_err());
    }

    #[test]
    fn activation_never_replaces_an_existing_install() {
        let fixture = Fixture::new();
        let tools = fixture.0.join("tools");
        let existing = tools.join("pkg/bin");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("ffmpeg.exe"), b"old").unwrap();
        let tree = fixture.0.join("staging").join("pkg");
        std::fs::create_dir_all(tree.join("bin")).unwrap();
        std::fs::write(tree.join("bin/ffmpeg.exe"), b"new").unwrap();
        let error = activate(&tools, &tree, "pkg").unwrap_err();
        assert!(error.contains("already installed"));
        assert_eq!(
            std::fs::read(existing.join("ffmpeg.exe")).unwrap(),
            b"old"
        );
        assert!(tree.join("bin/ffmpeg.exe").is_file());
        let fresh = activate(&tools, &tree, "fresh-pkg").unwrap();
        assert_eq!(
            std::fs::read(fresh.join("bin/ffmpeg.exe")).unwrap(),
            b"new"
        );
        assert!(!tree.exists());
    }

    #[cfg(managed_ffmpeg)]
    #[test]
    fn mac_archive_retains_notices_and_rejects_outside_members() {
        use std::io::Write;
        let fixture = Fixture::new();
        let archive = fixture.0.join("mac.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        // Archive modes are ignored: every member claims to be executable.
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for name in ["bin/ffmpeg", "LICENSE.txt", "SOURCE.txt", "NOTICE.txt", "notices/ffmpeg-buildconf.txt"] {
            zip.start_file(format!("{}/{name}", MACOS.folder), options).unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let stage = fixture.0.join("stage");
        unpack(&archive, &stage, &MACOS).unwrap();
        assert_eq!(std::fs::read(stage.join(MACOS.folder).join("SOURCE.txt")).unwrap(), b"SOURCE.txt");
        assert!(stage.join(MACOS.folder).join("notices/ffmpeg-buildconf.txt").is_file());
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |name: &str| std::fs::metadata(stage.join(MACOS.folder).join(name)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode("bin/ffmpeg"), 0o755);
            assert_eq!(mode("SOURCE.txt"), 0o644);
        }
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file("outside/evil", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(unpack(&archive, &stage, &MACOS).unwrap_err().contains("unexpected"));
        assert!(!stage.join("outside").exists());
    }

    /// The consent version hashes the reviewed catalog pin, so the code pin
    /// must be the same bytes or users would install what they never reviewed.
    #[test]
    fn code_pins_match_the_reviewed_consent_catalog() {
        let components: serde_json::Value =
            serde_json::from_str(include_str!("../../licenses/download-components.json")).unwrap();
        let consent: serde_json::Value =
            serde_json::from_str(include_str!("../../legal/download-consent.json")).unwrap();
        for (id, platform, pin) in [
            ("ffmpeg-windows", "windows", &WINDOWS),
            ("ffmpeg-macos", "macos", &MACOS),
        ] {
            let component = components["components"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == id)
                .unwrap();
            assert_ne!(component["automaticInstall"], false, "{id}");
            assert_eq!(component["downloadUrls"][0], pin.url, "{id}");
            let archive = &component["artifactPins"][0];
            assert_eq!(archive["url"], pin.url, "{id}");
            assert_eq!(archive["sha256"], pin.sha256, "{id}");
            assert_eq!(archive["bytes"], pin.bytes, "{id}");
            // Fail closed: the backend consent gate only blocks a group it finds.
            assert!(consent["groups"].as_array().unwrap().iter().any(|g| {
                g["id"] == id
                    && g["platform"] == platform
                    && g["models"].as_array().unwrap().iter().any(|m| m == "ffmpeg")
            }), "{id} has no consent group");
        }
    }

    /// The published archive must contain every notice the Mac install requires.
    #[test]
    fn published_mac_archive_contains_required_notices() {
        // Read at test time: a literal include would count as a bundled asset.
        let members: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../licenses/downloads/ffmpeg-macos-9.0.2-sonojot-lgpl-1/archive-members.json"
            ))
            .unwrap(),
        )
        .unwrap();
        for name in MACOS_NOTICES.iter().chain(&["bin/ffmpeg"]) {
            let path = format!("{}/{name}", MACOS.folder);
            assert!(members.iter().any(|m| m["file"] == path.as_str()), "{path}");
        }
    }

}
