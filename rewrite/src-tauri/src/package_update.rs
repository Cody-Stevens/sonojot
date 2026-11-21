//! Linux package installs (pacman, dpkg, rpm). Finds the package manager that
//! owns this SonoJot, and installs a downloaded, signature-verified package with
//! it: a polkit password dialog when an authentication agent is running,
//! otherwise `sudo` in a terminal window.
//!
//! The updater plugin's own deb/rpm installer can't install a pacman package
//! and has no terminal fallback. The pacman package repackages the .deb, so its
//! binary carries Tauri's Deb bundle type; it must never reach dpkg.
use crate::updater::{InstallPrompt, PackageManager};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Written by the pacman package (see packaging/arch/PKGBUILD).
const MARKER: &str = "/usr/share/sonojot/install-source";
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const TERMINAL_START_TIMEOUT: Duration = Duration::from_secs(20);
const TERMINAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// pkexec answers 127 at once when no agent takes the request. A refused or
/// mistyped password takes longer than this.
const NO_AGENT_ANSWER: Duration = Duration::from_secs(2);

const NOT_A_PACKAGE: &str = "The downloaded update isn't a package for this computer. Check for updates to try again.";
const PASSWORD_CLOSED: &str = "You closed the password prompt, so SonoJot wasn't updated.";
const PASSWORD_REFUSED: &str = "The password wasn't accepted, so SonoJot wasn't updated.";
const MANAGER_FAILED: &str = "The package manager couldn't install the update. Try again, or get it from the download page.";
const TERMINAL_FAILED: &str = "The install in the terminal didn't finish, so SonoJot wasn't updated.";
const TERMINAL_CLOSED: &str = "The terminal closed before the install finished, so SonoJot wasn't updated.";
const TERMINAL_SLOW: &str = "SonoJot stopped waiting for the terminal. If the install finished, quit and reopen SonoJot.";
const NO_TERMINAL: &str = "SonoJot couldn't open a terminal to ask for your password. Get the update from the download page.";
const NOT_INSTALLED: &str = "The package manager finished, but the new version isn't installed. Try again, or get it from the download page.";
const NO_FOLDER: &str = "SonoJot couldn't save the update to install it. Check for updates to try again.";

impl PackageManager {
    fn file_name(self) -> &'static str {
        match self {
            Self::Pacman => "sonojot-update.pkg.tar.zst",
            Self::Dpkg => "sonojot-update.deb",
            Self::Rpm => "sonojot-update.rpm",
        }
    }

    /// The command that installs `file`, after pkexec or sudo. rpm needs
    /// --oldpackage to switch to an older channel build, and --replacepkgs to
    /// reinstall the same version.
    fn install_args(self, file: &Path) -> Vec<OsString> {
        let flags: &[&str] = match self {
            Self::Pacman => &["-U", "--noconfirm"],
            Self::Dpkg => &["-i"],
            Self::Rpm => &["-U", "--oldpackage", "--replacepkgs"],
        };
        flags
            .iter()
            .map(OsString::from)
            .chain([file.as_os_str().to_owned()])
            .collect()
    }

    /// The file starts like this manager's package format.
    fn accepts(self, bytes: &[u8]) -> bool {
        match self {
            // pacman packages are zstd-compressed tar archives.
            Self::Pacman => bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]),
            Self::Dpkg => bytes.starts_with(b"!<arch>\ndebian-binary"),
            Self::Rpm => bytes.starts_with(&[0xed, 0xab, 0xee, 0xdb]),
        }
    }
}

/// A package manager that owns this SonoJot, the package's name and the
/// manager's program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInstall {
    pub manager: PackageManager,
    pub name: String,
    program: PathBuf,
}

#[cfg(test)]
impl PackageInstall {
    pub(crate) fn for_tests(manager: PackageManager) -> Self {
        Self {
            manager,
            name: "sonojot".into(),
            program: PathBuf::from("/nonexistent"),
        }
    }
}

/// Which package manager installed `exe`, if any. pacman comes first: the
/// marker file or `pacman -Qqo` settles it before dpkg or rpm are asked.
pub fn detect(exe: &Path) -> Option<PackageInstall> {
    let marker = std::fs::read_to_string(MARKER)
        .map(|text| text.trim() == "pacman")
        .unwrap_or(false);
    if let Some(pacman) = find_program("pacman") {
        let owner = query(&pacman, &["-Qqo".as_ref(), exe.as_os_str()])
            .and_then(|out| first_name(&out));
        if marker || owner.is_some() {
            return Some(PackageInstall {
                manager: PackageManager::Pacman,
                name: owner.unwrap_or_else(|| "sonojot".into()),
                program: pacman,
            });
        }
    }
    if marker {
        // Packaged for pacman, but pacman isn't here to install an update.
        return None;
    }
    if let Some(dpkg) = find_program("dpkg") {
        if let Some(name) =
            query(&dpkg, &["-S".as_ref(), exe.as_os_str()]).and_then(|out| dpkg_owner(&out))
        {
            return Some(PackageInstall {
                manager: PackageManager::Dpkg,
                name,
                program: dpkg,
            });
        }
    }
    let rpm = find_program("rpm")?;
    let name = query(
        &rpm,
        &["-qf".as_ref(), "--qf".as_ref(), "%{NAME}\n".as_ref(), exe.as_os_str()],
    )
    .and_then(|out| first_name(&out))?;
    Some(PackageInstall {
        manager: PackageManager::Rpm,
        name,
        program: rpm,
    })
}

/// A package name as a manager prints it: one plain word.
fn package_name(text: &str) -> Option<String> {
    let name = text.trim();
    (!name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')))
    .then(|| name.to_owned())
}

fn first_name(output: &str) -> Option<String> {
    package_name(output.lines().next()?)
}

/// `dpkg -S` prints `name: /path` or `name:arch: /path`.
fn dpkg_owner(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    let (owner, path) = line.split_once(": ")?;
    if !path.starts_with('/') {
        return None;
    }
    package_name(owner.split(':').next()?)
}

/// The installed version of the package, as its manager reports it.
fn installed_version(package: &PackageInstall) -> Option<String> {
    let name = package.name.as_str();
    match package.manager {
        PackageManager::Pacman => query(&package.program, &["-Q".as_ref(), name.as_ref()])?
            .split_whitespace()
            .nth(1)
            .map(str::to_owned),
        PackageManager::Dpkg => query(
            &find_program("dpkg-query")?,
            &["-W".as_ref(), "-f=${Version}".as_ref(), name.as_ref()],
        ),
        PackageManager::Rpm => query(
            &package.program,
            &["-q".as_ref(), "--qf".as_ref(), "%{VERSION}".as_ref(), name.as_ref()],
        ),
    }
    .map(|version| version.trim().to_owned())
    .filter(|version| !version.is_empty())
}

/// Whether a manager's version string is the app version `expected`.
/// pacman drops `-` (0.28.10nightly.…) and adds `-pkgrel`; rpm may use `~`.
fn version_matches(installed: &str, expected: &str) -> bool {
    fn normalize(version: &str) -> String {
        version
            .chars()
            .filter(|c| !matches!(c, '-' | '~' | '_' | '+'))
            .collect::<String>()
            .to_ascii_lowercase()
    }
    // A trailing all-digit `-N` is a package release, never part of the
    // app version: those suffixes are `-nightly.…`.
    let installed = match installed.rsplit_once('-') {
        Some((version, release))
            if !release.is_empty() && release.chars().all(|c| c.is_ascii_digit()) =>
        {
            version
        }
        _ => installed,
    };
    normalize(installed) == normalize(expected)
}

/// Runs a read-only query. No answer within ten seconds counts as no answer.
fn query(program: &Path, args: &[&std::ffi::OsStr]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < QUERY_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

const SYSTEM_DIRS: [&str; 4] = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"];

fn executable_in(dirs: impl Iterator<Item = PathBuf>, name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    dirs.filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|file| {
            std::fs::metadata(file)
                .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

/// A program that runs as root (a package manager, pkexec) or answers for one:
/// only from the system folders, never from a PATH entry the user can write to.
fn find_program(name: &str) -> Option<PathBuf> {
    executable_in(SYSTEM_DIRS.into_iter().map(PathBuf::from), name)
}

/// A terminal: the user's PATH first, then the system folders.
fn find_terminal(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return executable_in(path.parent().map(Path::to_path_buf).into_iter(), path.file_name()?.to_str()?);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    executable_in(std::env::split_paths(&path).chain(SYSTEM_DIRS.into_iter().map(PathBuf::from)), name)
}

/// Installs `bytes` (already verified against the release signature) with the
/// package's manager, then confirms that `expected` is the installed version.
/// `prompt` reports how the password is asked for. Errors are user-facing.
pub fn install(
    package: &PackageInstall,
    bytes: &[u8],
    expected: &str,
    prompt: impl Fn(InstallPrompt),
) -> Result<(), String> {
    if !package.manager.accepts(bytes) {
        return Err(NOT_A_PACKAGE.into());
    }
    let folder = Folder::create().ok_or(NO_FOLDER)?;
    let file = folder.0.join(package.manager.file_name());
    write_file(&file, bytes).map_err(|_| NO_FOLDER)?;
    let args = package.manager.install_args(&file);
    let asked = if polkit_agent_running() {
        find_program("pkexec").and_then(|pkexec| {
            prompt(InstallPrompt::Password);
            with_pkexec(&pkexec, &package.program, &args)
        })
    } else {
        None
    };
    match asked {
        Some(result) => result?,
        None => {
            prompt(InstallPrompt::Terminal);
            in_terminal(&folder.0, &package.program, &args)?;
        }
    }
    match installed_version(package) {
        Some(version) if version_matches(&version, expected) => Ok(()),
        _ => Err(NOT_INSTALLED.into()),
    }
}

/// None when no agent answered, so the terminal should ask instead.
fn with_pkexec(pkexec: &Path, program: &Path, args: &[OsString]) -> Option<Result<(), String>> {
    let started = Instant::now();
    let status = Command::new(pkexec)
        .arg(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    pkexec_result(status.code(), started.elapsed())
}

fn pkexec_result(code: Option<i32>, elapsed: Duration) -> Option<Result<(), String>> {
    Some(match code {
        Some(0) => Ok(()),
        Some(126) => Err(PASSWORD_CLOSED.into()),
        Some(127) if elapsed < NO_AGENT_ANSWER => return None,
        Some(127) => Err(PASSWORD_REFUSED.into()),
        _ => Err(MANAGER_FAILED.into()),
    })
}

/// A polkit authentication agent runs as this user. pkexec needs one to show
/// its password dialog; without one it fails, so the terminal asks instead.
fn polkit_agent_running() -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(me) = std::fs::metadata("/proc/self").map(|meta| meta.uid()) else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_name().to_str().is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
            && entry.metadata().is_ok_and(|meta| meta.uid() == me)
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| is_polkit_agent(comm.trim()))
    })
}

/// Process names (cut to 15 characters by the kernel) of authentication
/// agents: polkit-gnome, polkit-kde, hyprpolkitagent, lxpolkit, mate, xfce and
/// others, plus desktops with one built in.
fn is_polkit_agent(comm: &str) -> bool {
    let comm = comm.to_ascii_lowercase();
    ((comm.contains("polkit") || comm.contains("policykit")) && comm != "polkitd")
        || matches!(comm.as_str(), "gnome-shell" | "cinnamon" | "soteria")
}

/// Terminals to try, in order: the desktop's choice, then common ones. Each
/// is a program and the arguments that come before the command it runs.
fn terminals(preferred: Option<&str>) -> Vec<(String, Vec<&'static str>)> {
    fn args(name: &str) -> Vec<&'static str> {
        match name {
            "xdg-terminal-exec" | "foot" | "kitty" => vec![],
            "gnome-terminal" => vec!["--"],
            _ => vec!["-e"],
        }
    }
    let mut list = vec!["xdg-terminal-exec".to_owned()];
    if let Some(name) = preferred
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
    {
        list.push(name.to_owned());
    }
    for name in ["foot", "kitty", "alacritty", "gnome-terminal", "konsole", "xterm"] {
        list.push(name.into());
    }
    let mut seen = Vec::new();
    list.into_iter()
        .filter(|name| {
            let base = Path::new(name).file_name().map(|b| b.to_owned());
            let new = !seen.contains(&base);
            seen.push(base);
            new
        })
        .map(|name| {
            let base = Path::new(&name)
                .file_name()
                .and_then(|b| b.to_str())
                .unwrap_or_default()
                .to_owned();
            (name, args(&base))
        })
        .collect()
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The script the terminal runs. It records its PID once it starts and the
/// command's exit status once it ends, each by an atomic rename.
fn terminal_script(folder: &Path, program: &Path, args: &[OsString]) -> String {
    let folder = folder.to_string_lossy();
    let at = |name: &str| quote(&format!("{folder}/{name}"));
    let command = std::iter::once(program.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(|part| quote(&part.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "printf '%s' \"$$\" > {started_tmp} && mv {started_tmp} {started}\n\
         printf '%s\\n%s\\n\\n' 'SonoJot needs your password to install its update. It will run:' {shown}\n\
         sudo -- {command}\n\
         status=$?\n\
         printf '%s' \"$status\" > {status_tmp} && mv {status_tmp} {status}\n\
         if [ \"$status\" -ne 0 ]; then\n\
         \x20 printf '\\n%s' 'SonoJot was not updated. Press Enter to close this window.'\n\
         \x20 read -r _\n\
         fi\n",
        started_tmp = at("started.tmp"),
        started = at("started"),
        status_tmp = at("status.tmp"),
        status = at("status"),
        shown = quote(&format!("sudo {}", shown_command(program, args))),
    )
}

fn shown_command(program: &Path, args: &[OsString]) -> String {
    std::iter::once(program.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs the install with sudo in a terminal and waits for it to end.
fn in_terminal(folder: &Path, program: &Path, args: &[OsString]) -> Result<(), String> {
    let script = folder.join("install.sh");
    write_file(&script, terminal_script(folder, program, args).as_bytes())
        .map_err(|_| NO_FOLDER)?;
    let started = folder.join("started");
    let preferred = std::env::var("TERMINAL").ok();
    let mut launched = false;
    'terminals: for (name, before) in terminals(preferred.as_deref()) {
        let Some(terminal) = find_terminal(&name) else {
            continue;
        };
        let Ok(mut child) = Command::new(terminal)
            .args(before)
            .arg("sh")
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let since = Instant::now();
        let mut exited = false;
        loop {
            if started.is_file() {
                launched = true;
                // Reap the terminal whenever it closes.
                std::thread::spawn(move || child.wait());
                break 'terminals;
            }
            if !exited {
                match child.try_wait() {
                    // Failed to open: try the next terminal.
                    Ok(Some(status)) if !status.success() => continue 'terminals,
                    // A terminal server opened the window and returned.
                    Ok(Some(_)) => exited = true,
                    _ => {}
                }
            }
            if since.elapsed() > TERMINAL_START_TIMEOUT {
                // It may still open; never start a second password prompt.
                if !exited {
                    std::thread::spawn(move || child.wait());
                }
                return Err(NO_TERMINAL.into());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    if !launched {
        return Err(NO_TERMINAL.into());
    }
    let shell = std::fs::read_to_string(&started)
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .map(|pid| PathBuf::from(format!("/proc/{pid}")));
    let status = folder.join("status");
    let since = Instant::now();
    loop {
        if let Ok(code) = std::fs::read_to_string(&status) {
            return terminal_result(code.trim());
        }
        if shell.as_ref().is_some_and(|proc| !proc.exists()) {
            // The window closed. The status may have landed meanwhile.
            return match std::fs::read_to_string(&status) {
                Ok(code) => terminal_result(code.trim()),
                Err(_) => Err(TERMINAL_CLOSED.into()),
            };
        }
        if since.elapsed() > TERMINAL_TIMEOUT {
            return Err(TERMINAL_SLOW.into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn terminal_result(code: &str) -> Result<(), String> {
    if code == "0" {
        Ok(())
    } else {
        Err(TERMINAL_FAILED.into())
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// A private folder for one install, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn create() -> Option<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!("sonojot-update-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&path).ok()?;
        Some(Self(path))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_manager_has_its_platform_command_and_format() {
        let file = Path::new("/tmp/sonojot-update-x/sonojot-update.pkg.tar.zst");
        assert_eq!(PackageManager::Pacman.platform_key(), "linux-x86_64-pacman");
        assert_eq!(PackageManager::Dpkg.platform_key(), "linux-x86_64-deb");
        assert_eq!(PackageManager::Rpm.platform_key(), "linux-x86_64-rpm");
        assert_eq!(
            PackageManager::Pacman.install_args(file),
            ["-U", "--noconfirm", file.to_str().unwrap()].map(OsString::from)
        );
        assert_eq!(
            PackageManager::Dpkg.install_args(file),
            ["-i", file.to_str().unwrap()].map(OsString::from)
        );
        assert_eq!(
            PackageManager::Rpm.install_args(file),
            ["-U", "--oldpackage", "--replacepkgs", file.to_str().unwrap()].map(OsString::from)
        );
        let zstd = [0x28, 0xb5, 0x2f, 0xfd, 0, 0];
        let deb = b"!<arch>\ndebian-binary   1342943816  0     0     100644  4         `\n";
        let rpm = [0xed, 0xab, 0xee, 0xdb, 3, 0];
        assert!(PackageManager::Pacman.accepts(&zstd));
        assert!(PackageManager::Dpkg.accepts(deb));
        assert!(PackageManager::Rpm.accepts(&rpm));
        for (manager, wrong) in [
            (PackageManager::Pacman, &deb[..]),
            (PackageManager::Dpkg, &rpm[..]),
            (PackageManager::Rpm, &zstd[..]),
            (PackageManager::Pacman, b"\x7fELF".as_slice()),
        ] {
            assert!(!manager.accepts(wrong));
        }
        assert_eq!(serde_json::to_value(PackageManager::Dpkg).unwrap(), "dpkg");
        assert_eq!(serde_json::to_value(InstallPrompt::Terminal).unwrap(), "terminal");
    }

    #[test]
    fn owner_queries_parse_plain_package_names_only() {
        assert_eq!(first_name("sonojot\n").as_deref(), Some("sonojot"));
        assert_eq!(first_name("sonojot-nightly\nother\n").as_deref(), Some("sonojot-nightly"));
        assert_eq!(first_name(""), None);
        assert_eq!(first_name("bad name; rm\n"), None);
        assert_eq!(dpkg_owner("sono-jot: /usr/bin/SonoJot\n").as_deref(), Some("sono-jot"));
        assert_eq!(dpkg_owner("sonojot:amd64: /usr/bin/SonoJot\n").as_deref(), Some("sonojot"));
        assert_eq!(dpkg_owner("dpkg-query: no path found matching pattern"), None);
    }

    #[test]
    fn installed_versions_match_each_managers_spelling() {
        let nightly = "0.28.10-nightly.202611170800";
        assert!(version_matches("0.28.10nightly.202611170800-1", nightly));
        assert!(version_matches("0.28.10-nightly.202611170800", nightly));
        assert!(version_matches("0.28.10~nightly.202611170800", nightly));
        assert!(version_matches("0.28.10-1", "0.28.10"));
        assert!(version_matches("0.28.10", "0.28.10"));
        assert!(!version_matches("0.28.1", "0.28.10"));
        assert!(!version_matches("0.28.10-1", "0.28.1"));
        assert!(!version_matches("0.28.9nightly.202611170800-1", nightly));
        assert!(!version_matches("0.28.10", nightly));
    }

    #[test]
    fn pkexec_exit_codes_become_clear_outcomes() {
        let slow = Duration::from_secs(9);
        assert_eq!(pkexec_result(Some(0), slow), Some(Ok(())));
        assert_eq!(pkexec_result(Some(126), slow), Some(Err(PASSWORD_CLOSED.into())));
        assert_eq!(pkexec_result(Some(127), slow), Some(Err(PASSWORD_REFUSED.into())));
        // An instant 127 means no agent answered: the terminal asks instead.
        assert_eq!(pkexec_result(Some(127), Duration::from_millis(80)), None);
        assert_eq!(pkexec_result(Some(1), slow), Some(Err(MANAGER_FAILED.into())));
        assert_eq!(pkexec_result(None, slow), Some(Err(MANAGER_FAILED.into())));
    }

    #[test]
    fn polkit_agents_are_recognized_by_process_name() {
        for agent in [
            "polkit-gnome-au",
            "polkit-kde-auth",
            "hyprpolkitagent",
            "lxpolkit",
            "lxqt-policykit-",
            "mate-polkit",
            "gnome-shell",
            "soteria",
        ] {
            assert!(is_polkit_agent(agent), "{agent}");
        }
        for other in ["polkitd", "Hyprland", "bash", "SonoJot"] {
            assert!(!is_polkit_agent(other), "{other}");
        }
    }

    #[test]
    fn terminals_start_with_the_desktop_choice_and_never_repeat() {
        let list = terminals(Some("alacritty"));
        let names: Vec<_> = list.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["xdg-terminal-exec", "alacritty", "foot", "kitty", "gnome-terminal", "konsole", "xterm"]
        );
        let args = |name: &str| list.iter().find(|(n, _)| n == name).unwrap().1.clone();
        assert!(args("xdg-terminal-exec").is_empty());
        assert!(args("foot").is_empty());
        assert_eq!(args("alacritty"), ["-e"]);
        assert_eq!(args("gnome-terminal"), ["--"]);
        assert_eq!(args("xterm"), ["-e"]);
        let custom = terminals(Some("/opt/wezterm/bin/wezterm"));
        assert_eq!(custom[1], ("/opt/wezterm/bin/wezterm".into(), vec!["-e"]));
        // xdg-terminal-exec as $TERMINAL (Omarchy) and odd values add nothing.
        assert_eq!(terminals(Some("xdg-terminal-exec")).len(), 7);
        assert_eq!(terminals(Some("foot --server")).len(), 7);
        assert_eq!(terminals(None).len(), 7);
    }

    #[test]
    fn terminal_script_quotes_paths_and_reports_its_status() {
        let folder = Path::new("/tmp/sonojot-update-1");
        let file = folder.join("sonojot-update.pkg.tar.zst");
        let script = terminal_script(
            folder,
            Path::new("/usr/bin/pacman"),
            &PackageManager::Pacman.install_args(&file),
        );
        assert!(script.contains(
            "sudo -- '/usr/bin/pacman' '-U' '--noconfirm' '/tmp/sonojot-update-1/sonojot-update.pkg.tar.zst'\n"
        ));
        assert!(script.contains("mv '/tmp/sonojot-update-1/started.tmp' '/tmp/sonojot-update-1/started'"));
        assert!(script.contains("mv '/tmp/sonojot-update-1/status.tmp' '/tmp/sonojot-update-1/status'"));
        assert!(script.contains("'sudo /usr/bin/pacman -U --noconfirm /tmp/sonojot-update-1/sonojot-update.pkg.tar.zst'"));
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(terminal_result("0"), Ok(()));
        assert_eq!(terminal_result("1"), Err(TERMINAL_FAILED.into()));
        assert_eq!(terminal_result("130"), Err(TERMINAL_FAILED.into()));
    }

    #[test]
    fn the_terminal_script_runs_and_records_its_exit_status() {
        // `sudo` is replaced by a stub that runs the command, so nothing
        // is installed and no password is asked for.
        let folder = Folder::create().unwrap();
        let bin = folder.0.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let stub = bin.join("sudo");
        write_file(&stub, b"#!/bin/sh\n[ \"$1\" = -- ] && shift\nexec \"$@\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        for (program, code) in [("/bin/true", "0"), ("/bin/false", "1")] {
            let _ = std::fs::remove_file(folder.0.join("status"));
            let _ = std::fs::remove_file(folder.0.join("started"));
            let script = folder.0.join(format!("install-{code}.sh"));
            write_file(&script, terminal_script(&folder.0, Path::new(program), &[]).as_bytes())
                .unwrap();
            let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
            let output = Command::new("/bin/sh")
                .arg(&script)
                .env("PATH", path)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            // A failure waits for Enter, which stdin (empty) never gives.
            assert_eq!(output.status.success(), code == "0");
            assert_eq!(std::fs::read_to_string(folder.0.join("status")).unwrap(), code);
            assert!(std::fs::read_to_string(folder.0.join("started"))
                .unwrap()
                .parse::<u32>()
                .is_ok());
        }
    }

    #[test]
    fn root_programs_come_only_from_system_folders() {
        let folder = Folder::create().unwrap();
        let fake = folder.0.join("pacman");
        write_file(&fake, b"#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let found = executable_in(SYSTEM_DIRS.into_iter().map(PathBuf::from), "pacman");
        assert!(found.is_none_or(|path| SYSTEM_DIRS.iter().any(|dir| path.starts_with(dir))));
        assert_eq!(executable_in(std::iter::once(folder.0.clone()), "pacman"), Some(fake));
    }

    #[test]
    fn a_file_of_the_wrong_format_is_refused_before_anything_runs() {
        let package = PackageInstall {
            manager: PackageManager::Pacman,
            name: "sonojot".into(),
            program: PathBuf::from("/nonexistent/pacman"),
        };
        let prompted = std::cell::Cell::new(false);
        assert_eq!(
            install(&package, b"!<arch>\ndebian-binary", "1.0.0", |_| prompted.set(true)),
            Err(NOT_A_PACKAGE.into())
        );
        assert!(!prompted.get());
    }
}
