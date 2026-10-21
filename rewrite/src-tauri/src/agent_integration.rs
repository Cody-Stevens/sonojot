//! Puts `sonojot-cli` on PATH and points coding agents at it.
//!
//! Agents that look for MCP servers find a `sonojot` entry that runs
//! `sonojot-cli mcp`, a stub whose only tool says to use the CLI. Only
//! agents that are installed are touched, entries someone else wrote are
//! left alone, and config files keep their other contents byte for byte.
//! The app runs this once per install location; the installer and
//! `sonojot-cli agents install|remove` run it on demand.

use serde::{de::{MapAccess, Visitor}, Deserialize, Deserializer, Serialize};
use serde_json::{json, value::RawValue, Value};
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const SERVER_NAME: &str = "sonojot";
const MARKER_FILE: &str = "agent-integration.json";
const TOOL_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Check,
    Install,
    Remove,
}

/// One PATH or agent target and what happened to it.
#[derive(Clone, Debug, Serialize)]
pub struct Item {
    pub target: String,
    /// present, added, updated, removed, absent, skipped or failed.
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn item(target: &str, state: &'static str, detail: Option<String>) -> Item {
    Item { target: target.into(), state, detail }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    cli: PathBuf,
    removed: bool,
}

/// The CLI installed beside an app executable.
pub fn cli_beside(executable: &Path) -> PathBuf {
    executable.with_file_name(if cfg!(windows) { "sonojot-cli.exe" } else { "sonojot-cli" })
}

/// Development builds run from Cargo's target folder and are never registered.
pub fn is_development_build(path: &Path) -> bool {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    parts.windows(2).any(|pair| pair[0] == "target" && matches!(pair[1].as_str(), "debug" | "release"))
}

/// Runs at app startup. Registers once per install location and never again
/// after the user removes it with `sonojot-cli agents remove`.
pub fn ensure_once(data_dir: &Path) {
    let Ok(executable) = std::env::current_exe() else { return };
    if let Some(items) = ensure(&cli_beside(&executable), data_dir) {
        let summary = items.iter().map(|i| format!("{}={}", i.target, i.state)).collect::<Vec<_>>().join(" ");
        crate::recognition_diagnostics::event("agent-integration", &summary, std::process::id(), 0, 0, 0);
    }
}

/// Installs unless this location is already done or the user removed it.
/// Returns None when nothing ran.
pub fn ensure(cli: &Path, data_dir: &Path) -> Option<Vec<Item>> {
    if is_development_build(cli) || !cli.is_file() {
        return None;
    }
    if let Ok(saved) = fs::read(data_dir.join(MARKER_FILE)).map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice::<Marker>(&bytes).map_err(|e| e.to_string())) {
        if saved.removed || saved.cli == cli {
            return None;
        }
    }
    Some(run(Mode::Install, cli, data_dir))
}

/// After an uninstall, a later install registers again.
pub fn forget(data_dir: &Path) {
    let _ = fs::remove_file(data_dir.join(MARKER_FILE));
}

/// Checks, installs or removes every target and records the outcome.
pub fn run(mode: Mode, cli: &Path, data_dir: &Path) -> Vec<Item> {
    let mut items = vec![path_item(mode, cli)];
    if let Some(home) = home() {
        items.extend(claude_code(mode, cli, &home));
        items.extend(codex(mode, cli, &home));
        for target in json_targets(&home) {
            items.extend(json_target(mode, cli, &target));
        }
    }
    if mode != Mode::Check {
        let marker = Marker { cli: cli.to_path_buf(), removed: mode == Mode::Remove };
        if let Ok(bytes) = serde_json::to_vec_pretty(&marker) {
            let _ = fs::create_dir_all(data_dir);
            let _ = fs::write(data_dir.join(MARKER_FILE), bytes);
        }
    }
    items
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|p| p.is_dir())
}

/// A `sonojot-cli` entry from any SonoJot install, not just this one.
fn ours(command: &str) -> bool {
    Path::new(command).file_name().and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("sonojot-cli") || name.eq_ignore_ascii_case("sonojot-cli.exe"))
}

// ---------------------------------------------------------------- PATH

#[cfg(windows)]
fn path_item(mode: Mode, cli: &Path) -> Item {
    use winreg::{enums::*, RegKey, RegValue};
    const TARGET: &str = "PATH";
    let Some(dir) = cli.parent() else { return item(TARGET, "failed", Some("The CLI has no folder.".into())) };
    let dir = dir.to_string_lossy().into_owned();
    let environment = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE) {
        Ok(key) => key,
        Err(e) => return item(TARGET, "failed", Some(format!("Cannot open the user environment: {e}"))),
    };
    let (text, kind) = match environment.get_raw_value("Path") {
        Ok(value) => {
            let units: Vec<u16> = value.bytes.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
            (String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned(), value.vtype)
        }
        Err(_) => (String::new(), REG_EXPAND_SZ),
    };
    let same = |entry: &str| expand(entry).trim_end_matches('\\').eq_ignore_ascii_case(dir.trim_end_matches('\\'));
    let entries: Vec<&str> = text.split(';').filter(|e| !e.trim().is_empty()).collect();
    let present = entries.iter().any(|e| same(e));
    let updated = match mode {
        Mode::Check => return item(TARGET, if present { "present" } else { "absent" }, Some(dir)),
        Mode::Install if present => return item(TARGET, "present", Some(dir)),
        Mode::Remove if !present => return item(TARGET, "absent", Some(dir)),
        Mode::Install => { let mut next = entries.clone(); next.push(&dir); next.join(";") }
        Mode::Remove => entries.iter().filter(|e| !same(e)).copied().collect::<Vec<_>>().join(";"),
    };
    let mut bytes: Vec<u8> = updated.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    bytes.shrink_to_fit();
    if let Err(e) = environment.set_raw_value("Path", &RegValue { bytes: bytes.into(), vtype: kind }) {
        return item(TARGET, "failed", Some(format!("Cannot update the user PATH: {e}")));
    }
    announce_environment_change();
    item(TARGET, if mode == Mode::Install { "added" } else { "removed" }, Some(dir))
}

#[cfg(windows)]
fn expand(entry: &str) -> String {
    let mut out = String::new();
    let mut rest = entry.trim();
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                rest = &after[end + 1..];
            }
            None => { out.push_str(&rest[start..]); rest = ""; }
        }
    }
    out.push_str(rest);
    out
}

/// New terminals pick up the PATH without signing out.
#[cfg(windows)]
fn announce_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
    let name: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0usize;
    // SAFETY: a broadcast with a NUL-terminated string that outlives the call.
    unsafe {
        SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, name.as_ptr() as isize, SMTO_ABORTIFHUNG, 2000, &mut result);
    }
}

#[cfg(unix)]
fn path_item(mode: Mode, cli: &Path) -> Item {
    use std::os::unix::fs::symlink;
    const TARGET: &str = "PATH";
    // Linux packages install the CLI into /usr/bin already.
    if let Some(dir) = cli.parent() {
        if std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|entry| entry == dir)) {
            return item(TARGET, if mode == Mode::Remove { "skipped" } else { "present" },
                Some(format!("{} is already on PATH", dir.display())));
        }
    }
    let Some(home) = home() else { return item(TARGET, "failed", Some("HOME is not set.".into())) };
    let bin = home.join(".local/bin");
    let link = bin.join("sonojot-cli");
    let current = fs::read_link(&link).ok();
    let detail = Some(link.display().to_string());
    let state = match mode {
        Mode::Check => if current.as_deref() == Some(cli) { "present" } else { "absent" },
        Mode::Install => match &current {
            Some(target) if target == cli => "present",
            Some(target) if ours(&target.to_string_lossy()) => {
                if fs::remove_file(&link).and_then(|_| symlink(cli, &link)).is_err() { "failed" } else { "updated" }
            }
            Some(_) => return item(TARGET, "skipped", Some(format!("{} points somewhere else", link.display()))),
            None if link.exists() => return item(TARGET, "skipped", Some(format!("{} already exists", link.display()))),
            None => if fs::create_dir_all(&bin).and_then(|_| symlink(cli, &link)).is_err() { "failed" } else { "added" },
        },
        Mode::Remove => match &current {
            Some(target) if ours(&target.to_string_lossy()) => if fs::remove_file(&link).is_err() { "failed" } else { "removed" },
            _ => "absent",
        },
    };
    #[cfg(target_os = "macos")]
    if matches!(state, "added" | "updated" | "present" | "removed") {
        shell_profile(mode, &home);
    }
    item(TARGET, state, detail)
}

/// macOS shells don't include ~/.local/bin. Add it to ~/.zprofile unless a
/// zsh startup file already mentions it.
#[cfg(target_os = "macos")]
fn shell_profile(mode: Mode, home: &Path) {
    const BLOCK: &str = "\n# Added by SonoJot: sonojot-cli on PATH\nexport PATH=\"$HOME/.local/bin:$PATH\"\n";
    let profile = home.join(".zprofile");
    let text = fs::read_to_string(&profile).unwrap_or_default();
    match mode {
        Mode::Install => {
            let mentioned = [".zprofile", ".zshrc", ".zshenv"].iter()
                .any(|name| fs::read_to_string(home.join(name)).is_ok_and(|t| t.contains(".local/bin")));
            if !mentioned {
                let _ = fs::write(&profile, format!("{text}{BLOCK}"));
            }
        }
        Mode::Remove if text.contains(BLOCK) => { let _ = fs::write(&profile, text.replacen(BLOCK, "", 1)); }
        _ => {}
    }
}

// ---------------------------------------------------------------- Claude Code

/// Claude Code owns ~/.claude.json and rewrites it constantly, so it is only
/// changed through Claude Code's own `claude mcp` commands.
fn claude_code(mode: Mode, cli: &Path, home: &Path) -> Option<Item> {
    const TARGET: &str = "Claude Code";
    let Some(claude) = find_program("claude", home) else {
        return (home.join(".claude.json").is_file()).then(|| item(TARGET, "skipped", Some("The claude command was not found.".into())));
    };
    let current = run_tool(&claude, &["mcp", "get", SERVER_NAME]);
    let (exists, text) = match &current {
        Ok((ok, text)) => (*ok, text.as_str()),
        Err(error) => return Some(item(TARGET, "failed", Some(error.clone()))),
    };
    let cli_text = cli.to_string_lossy();
    let this_install = exists && text.contains(cli_text.as_ref());
    let any_install = exists && (text.contains("sonojot-cli"));
    let add = || run_tool(&claude, &[OsStr::new("mcp"), OsStr::new("add"), OsStr::new("--scope"), OsStr::new("user"),
        OsStr::new(SERVER_NAME), OsStr::new("--"), cli.as_os_str(), OsStr::new("mcp")]);
    let remove = || run_tool(&claude, &["mcp", "remove", "--scope", "user", SERVER_NAME]);
    let outcome = |result: Result<(bool, String), String>, state: &'static str| match result {
        Ok((true, _)) => item(TARGET, state, None),
        Ok((false, text)) => item(TARGET, "failed", Some(text.trim().chars().take(300).collect())),
        Err(error) => item(TARGET, "failed", Some(error)),
    };
    Some(match mode {
        Mode::Check => item(TARGET, if this_install { "present" } else if exists { "skipped" } else { "absent" }, None),
        Mode::Install if this_install => item(TARGET, "present", None),
        Mode::Install if any_install => { let _ = remove(); outcome(add(), "updated") }
        Mode::Install if exists => item(TARGET, "skipped", Some("Another server is already named sonojot.".into())),
        Mode::Install => outcome(add(), "added"),
        Mode::Remove if any_install => outcome(remove(), "removed"),
        Mode::Remove => item(TARGET, "absent", None),
    })
}

fn find_program(name: &str, home: &Path) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd")]
    } else {
        vec![name.into()]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // Apps started from Finder or the Start menu may not see shell PATH additions.
    dirs.extend([home.join(".local/bin"), home.join(".claude").join("local"), home.join("AppData/Roaming/npm"),
        PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    dirs.iter().flat_map(|dir| names.iter().map(move |name| dir.join(name))).find(|path| path.is_file())
}

/// Runs a tool with a time limit and returns (success, stdout + stderr).
fn run_tool<S: AsRef<OsStr>>(program: &Path, args: &[S]) -> Result<(bool, String), String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|e| format!("Cannot run {}: {e}", program.display()))?;
    let readers: Vec<_> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)]
        .into_iter().flatten()
        .map(|mut stream| std::thread::spawn(move || { let mut text = String::new(); let _ = stream.read_to_string(&mut text); text }))
        .collect();
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TOOL_TIMEOUT => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                kill_tree(&mut child);
                return Err(format!("{} did not finish in time.", program.display()));
            }
        }
    };
    let text = readers.into_iter().filter_map(|reader| reader.join().ok()).collect::<Vec<_>>().join("\n");
    Ok((status.success(), text))
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(0x0800_0000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

// ---------------------------------------------------------------- Codex

/// Codex reads `[mcp_servers.<name>]` tables from config.toml. The table is
/// appended or edited as text so the rest of the file is untouched.
fn codex(mode: Mode, cli: &Path, home: &Path) -> Option<Item> {
    const TARGET: &str = "Codex";
    let dir = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
    if !dir.is_dir() {
        return None;
    }
    let path = dir.join("config.toml");
    let text = fs::read_to_string(&path).unwrap_or_default();
    let (next, state) = codex_edit(&text, mode, &cli.to_string_lossy());
    if let Some(next) = next {
        if let Err(e) = write_atomically(&path, next.as_bytes()) {
            return Some(item(TARGET, "failed", Some(e)));
        }
    }
    Some(item(TARGET, state, Some(path.display().to_string())))
}

fn codex_edit(text: &str, mode: Mode, cli: &str) -> (Option<String>, &'static str) {
    let header = format!("[mcp_servers.{SERVER_NAME}]");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|line| line.trim() == header);
    let end = start.map(|start| lines.iter().skip(start + 1).position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset));
    let quoted = toml_string(cli);
    let table = start.zip(end).map(|(s, e)| &lines[s..e]);
    let command = table.and_then(|table| table.iter().find(|line| line.trim_start().starts_with("command")).copied());
    let is_ours = command.is_some_and(|line| line.contains("sonojot-cli"));
    let current = command.is_some_and(|line| line.contains(&quoted));
    match mode {
        Mode::Check => (None, if current { "present" } else if table.is_some() { "skipped" } else { "absent" }),
        Mode::Install if current => (None, "present"),
        Mode::Install if table.is_some() && !is_ours => (None, "skipped"),
        Mode::Install if table.is_some() => {
            let next: Vec<String> = lines.iter().map(|line| if Some(*line) == command { format!("command = {quoted}") } else { (*line).into() }).collect();
            (Some(next.join("\n") + "\n"), "updated")
        }
        Mode::Install => {
            let mut next = text.trim_end().to_owned();
            if !next.is_empty() { next.push_str("\n\n"); }
            next.push_str(&format!("{header}\n# Added by SonoJot: points agents to sonojot-cli\ncommand = {quoted}\nargs = [\"mcp\"]\n"));
            (Some(next), "added")
        }
        Mode::Remove if is_ours => {
            let (start, end) = (start.unwrap(), end.unwrap());
            let mut next: Vec<&str> = lines[..start].to_vec();
            while next.last().is_some_and(|line| line.trim().is_empty()) { next.pop(); }
            next.extend(&lines[end..]);
            (Some(next.join("\n") + "\n"), "removed")
        }
        Mode::Remove => (None, if table.is_some() { "skipped" } else { "absent" }),
    }
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

// ---------------------------------------------------------------- JSON configs

struct JsonTarget {
    name: &'static str,
    /// The agent's folder; nothing happens unless it exists.
    dir: PathBuf,
    file: PathBuf,
    key: &'static str,
    opencode: bool,
}

fn json_targets(home: &Path) -> Vec<JsonTarget> {
    let roaming = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"))
    } else if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else {
        home.join(".config")
    };
    let opencode = home.join(".config").join("opencode");
    let opencode_file = ["opencode.json", "opencode.jsonc"].iter().map(|name| opencode.join(name))
        .find(|path| path.is_file()).unwrap_or_else(|| opencode.join("opencode.json"));
    let target = |name, dir: PathBuf, file: &str, key| JsonTarget { name, file: dir.join(file), dir, key, opencode: false };
    vec![
        target("Gemini CLI", home.join(".gemini"), "settings.json", "mcpServers"),
        target("Cursor", home.join(".cursor"), "mcp.json", "mcpServers"),
        target("Windsurf", home.join(".codeium").join("windsurf"), "mcp_config.json", "mcpServers"),
        target("Claude Desktop", roaming.join("Claude"), "claude_desktop_config.json", "mcpServers"),
        JsonTarget { name: "opencode", dir: opencode, file: opencode_file, key: "mcp", opencode: true },
    ]
}

fn json_target(mode: Mode, cli: &Path, target: &JsonTarget) -> Option<Item> {
    if !target.dir.is_dir() {
        return None;
    }
    let text = match fs::read_to_string(&target.file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => return Some(item(target.name, "failed", Some(e.to_string()))),
    };
    let entry = if target.opencode {
        json!({ "type": "local", "command": [cli, "mcp"], "enabled": true })
    } else {
        json!({ "command": cli, "args": ["mcp"] })
    };
    let detail = Some(target.file.display().to_string());
    match json_edit(&text, target.key, &entry, mode) {
        Err(reason) => Some(item(target.name, "skipped", Some(format!("{}: {reason}", target.file.display())))),
        Ok((None, state)) => Some(item(target.name, state, detail)),
        Ok((Some(next), state)) => Some(match write_atomically(&target.file, next.as_bytes()) {
            Ok(()) => item(target.name, state, detail),
            Err(e) => item(target.name, "failed", Some(e)),
        }),
    }
}

/// A JSON object whose keys keep their order and whose values keep their text.
struct Ordered(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Ordered {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("a JSON object") }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Box<RawValue>>()? {
                    entries.push(entry);
                }
                Ok(Ordered(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

impl Ordered {
    fn render(&self, indent: usize, newline: &str) -> String {
        if self.0.is_empty() {
            return "{}".into();
        }
        let pad = " ".repeat(indent + 2);
        let body = self.0.iter()
            .map(|(key, value)| format!("{pad}{}: {}", serde_json::to_string(key).unwrap_or_default(), value.get()))
            .collect::<Vec<_>>().join(&format!(",{newline}"));
        format!("{{{newline}{body}{newline}{}}}", " ".repeat(indent))
    }
}

fn entry_command(value: &Value) -> Option<String> {
    let command = &value["command"];
    command.as_str().or_else(|| command.get(0).and_then(Value::as_str)).map(str::to_owned)
}

/// Adds, updates or removes `key.sonojot`. Returns the new text when it changes.
fn json_edit(text: &str, key: &str, entry: &Value, mode: Mode) -> Result<(Option<String>, &'static str), String> {
    let mut root: Ordered = serde_json::from_str(text).map_err(|_| "not plain JSON, left alone".to_string())?;
    let servers_index = root.0.iter().position(|(name, _)| name == key);
    let mut servers: Ordered = match servers_index {
        Some(index) => serde_json::from_str(root.0[index].1.get()).map_err(|_| format!("`{key}` is not an object"))?,
        None => Ordered(Vec::new()),
    };
    let existing = servers.0.iter().position(|(name, _)| name == SERVER_NAME);
    let existing_value: Option<Value> = existing.and_then(|index| serde_json::from_str(servers.0[index].1.get()).ok());
    let existing_command = existing_value.as_ref().and_then(entry_command);
    let is_ours = existing_command.as_deref().is_some_and(ours);
    let current = existing_value.as_ref() == Some(entry);
    let state = match mode {
        Mode::Check => return Ok((None, if current { "present" } else if existing.is_some() { "skipped" } else { "absent" })),
        Mode::Install if current => return Ok((None, "present")),
        Mode::Install if existing.is_some() && !is_ours => return Ok((None, "skipped")),
        Mode::Remove if !is_ours => return Ok((None, if existing.is_some() { "skipped" } else { "absent" })),
        Mode::Install => {
            let raw = RawValue::from_string(serde_json::to_string(entry).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            match existing {
                Some(index) => { servers.0[index].1 = raw; "updated" }
                None => { servers.0.push((SERVER_NAME.into(), raw)); "added" }
            }
        }
        Mode::Remove => { servers.0.remove(existing.unwrap()); "removed" }
    };
    // Keep the file's line endings; untouched values keep theirs anyway.
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let rendered = RawValue::from_string(servers.render(2, newline)).map_err(|e| e.to_string())?;
    match servers_index {
        // Removing the only server drops the key the install added.
        Some(index) if servers.0.is_empty() => { root.0.remove(index); }
        Some(index) => root.0[index].1 = rendered,
        None => root.0.push((key.into(), rendered)),
    }
    Ok((Some(root.render(0, newline) + newline), state))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("sonojot-tmp");
    fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, path))
        .map_err(|e| { let _ = fs::remove_file(&temporary); format!("Cannot write {}: {e}", path.display()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLI: &str = r"C:\Apps\SonoJot\sonojot-cli.exe";

    #[test]
    fn json_edit_keeps_other_settings_verbatim() {
        let crlf = json_edit("{\r\n  \"theme\": \"Dark\"\r\n}", "mcpServers", &json!({ "command": CLI }), Mode::Install).unwrap().0.unwrap();
        assert!(!crlf.replace("\r\n", "").contains('\n'));
        let text = "{\n  \"theme\": \"Dark\",\n  \"hooks\": { \"a\": [1,   2] }\n}\n";
        let entry = json!({ "command": CLI, "args": ["mcp"] });
        let (added, state) = json_edit(text, "mcpServers", &entry, Mode::Install).unwrap();
        let added = added.unwrap();
        assert_eq!(state, "added");
        assert!(added.starts_with("{\n  \"theme\": \"Dark\",\n  \"hooks\": { \"a\": [1,   2] },\n  \"mcpServers\": {"));
        assert_eq!(json_edit(&added, "mcpServers", &entry, Mode::Install).unwrap(), (None, "present"));
        let moved = json!({ "command": r"D:\SonoJot\sonojot-cli.exe", "args": ["mcp"] });
        assert_eq!(json_edit(&added, "mcpServers", &moved, Mode::Install).unwrap().1, "updated");
        let (removed, state) = json_edit(&added, "mcpServers", &entry, Mode::Remove).unwrap();
        assert_eq!(state, "removed");
        let removed: Value = serde_json::from_str(&removed.unwrap()).unwrap();
        assert_eq!(removed, json!({ "theme": "Dark", "hooks": { "a": [1, 2] } }));
    }

    #[test]
    fn json_edit_leaves_someone_elses_server_and_jsonc_alone() {
        let text = r#"{"mcpServers": {"sonojot": {"command": "node", "args": ["x"]}}}"#;
        let entry = json!({ "command": CLI, "args": ["mcp"] });
        assert_eq!(json_edit(text, "mcpServers", &entry, Mode::Install).unwrap(), (None, "skipped"));
        assert_eq!(json_edit(text, "mcpServers", &entry, Mode::Remove).unwrap(), (None, "skipped"));
        assert!(json_edit("{ // comment\n}", "mcpServers", &entry, Mode::Install).is_err());
    }

    #[test]
    fn codex_table_is_appended_updated_and_removed() {
        let text = "model = \"x\"\n\n[mcp_servers.mail]\ncommand = \"mail\"\n";
        let (added, state) = codex_edit(text, Mode::Install, CLI);
        let added = added.unwrap();
        assert_eq!(state, "added");
        assert!(added.contains("[mcp_servers.sonojot]\n# Added by SonoJot: points agents to sonojot-cli\ncommand = \"C:\\\\Apps\\\\SonoJot\\\\sonojot-cli.exe\"\nargs = [\"mcp\"]\n"));
        assert_eq!(codex_edit(&added, Mode::Install, CLI).1, "present");
        assert_eq!(codex_edit(&added, Mode::Install, r"D:\sonojot-cli.exe").1, "updated");
        let (removed, state) = codex_edit(&added, Mode::Remove, CLI);
        assert_eq!(state, "removed");
        assert_eq!(removed.unwrap(), text);
    }

    #[test]
    fn development_builds_are_recognized() {
        assert!(is_development_build(Path::new(r"C:\src\rewrite\src-tauri\target\release\SonoJot.exe")));
        assert!(!is_development_build(Path::new(r"C:\Users\me\AppData\Local\Programs\SonoJot\SonoJot.exe")));
    }
}
