//! Hyprland integration over its IPC socket: the dictation shortcut, paste
//! delivery and window placement. Wayland lets no client grab keys, read the
//! global pointer or position its own windows, so on Hyprland the compositor
//! does all three for SonoJot.
//!
//! SonoJot owns one generated config file, `$XDG_CONFIG_HOME/hypr/sonojot.lua`
//! (or `sonojot.conf` for hyprlang configs). It holds the shortcut binds and
//! window rules, and is rewritten when the shortcut changes. The user's own
//! config loads it through one line added to `bindings.lua`.
use serde::Deserialize;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_millis(800);

fn socket() -> Option<PathBuf> {
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("hypr");
    if let Some(signature) = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE") {
        let path = runtime.join(signature).join(".socket.sock");
        return path.exists().then_some(path);
    }
    // Started outside the session environment (a terminal, ssh, some
    // launchers): use the newest running instance's socket, found once.
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            std::fs::read_dir(&runtime)
                .ok()?
                .flatten()
                .map(|entry| entry.path().join(".socket.sock"))
                .filter(|path| UnixStream::connect(path).is_ok())
                .max_by_key(|path| path.metadata().and_then(|m| m.modified()).ok())
        })
        .clone()
        .filter(|path| path.exists())
}

/// SonoJot is running inside a Hyprland session it can reach.
pub fn active() -> bool {
    socket().is_some()
}

/// One IPC request. `command` uses hyprctl's wire format, for example
/// `j/activewindow` or `dispatch <dispatcher>`.
pub fn request(command: &str) -> Result<String, String> {
    let path = socket().ok_or("Hyprland is not running.")?;
    let mut stream = UnixStream::connect(path).map_err(|e| format!("Cannot reach Hyprland: {e}"))?;
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    stream.set_write_timeout(Some(TIMEOUT)).ok();
    stream
        .write_all(command.as_bytes())
        .map_err(|e| format!("Cannot reach Hyprland: {e}"))?;
    let mut reply = String::new();
    stream
        .take(4 << 20)
        .read_to_string(&mut reply)
        .map_err(|e| format!("Hyprland did not answer: {e}"))?;
    Ok(reply)
}

/// Hyprland 0.56 and later read a Lua config, and `dispatch` takes a Lua
/// dispatcher expression. Earlier versions use hyprlang and `sendshortcut`.
pub fn lua() -> bool {
    static LUA: OnceLock<bool> = OnceLock::new();
    *LUA.get_or_init(|| request("eval return 0").is_ok_and(|reply| reply.trim() == "ok"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct Window {
    pub address: String,
    #[serde(default)]
    pub pid: i64,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Window {
    /// Terminals paste with Shift+Insert. Omarchy tags them `terminal`; other
    /// setups fall back to well-known terminal classes.
    pub fn is_terminal(&self) -> bool {
        const CLASSES: &[&str] = &[
            "alacritty", "kitty", "foot", "footclient", "com.mitchellh.ghostty",
            "org.wezfurlong.wezterm", "org.gnome.console", "org.gnome.terminal",
            "konsole", "xterm", "st", "urxvt", "tilix", "terminator", "warp",
        ];
        self.tags.iter().any(|tag| tag.trim_end_matches('*') == "terminal")
            || CLASSES.contains(&self.class.to_ascii_lowercase().as_str())
    }
}

/// The focused window, or None when nothing has focus.
pub fn active_window() -> Option<Window> {
    let reply = request("j/activewindow").ok()?;
    serde_json::from_str::<Window>(&reply)
        .ok()
        .filter(|window| window.address.starts_with("0x") && window.address.len() > 2)
}

fn address_ok(address: &str) -> bool {
    address.len() > 2
        && address.starts_with("0x")
        && address[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn focus(address: &str) -> bool {
    if !address_ok(address) {
        return false;
    }
    let command = if lua() {
        format!("dispatch hl.dsp.focus({{window=\"address:{address}\"}})")
    } else {
        format!("dispatch focuswindow address:{address}")
    };
    request(&command).is_ok_and(|reply| reply.trim() == "ok")
}

/// Send the paste chord to the focused window, the way Omarchy's universal
/// paste does: an explicit key-down and key-up, so the synthetic key state
/// cannot stick. Ctrl+V, or Shift+Insert in terminals.
pub fn send_paste(target: &Window) -> bool {
    let (mods, key) = if target.is_terminal() { ("SHIFT", "Insert") } else { ("CTRL", "V") };
    if !lua() {
        return address_ok(&target.address)
            && request(&format!("dispatch sendshortcut {mods},{key},address:{}", target.address))
                .is_ok_and(|reply| reply.trim() == "ok");
    }
    let state = |state: &str| {
        request(&format!(
            "dispatch hl.dsp.send_key_state({{mods=\"{mods}\", key=\"{key}\", state=\"{state}\"}})"
        ))
        .is_ok_and(|reply| reply.trim() == "ok")
    };
    let down = state("down");
    std::thread::sleep(Duration::from_millis(30));
    // Always release, even when the press was rejected.
    let up = state("up");
    down && up
}

#[derive(Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

/// The pointer in global layout coordinates.
pub fn cursor_position() -> Option<(i32, i32)> {
    let point: Point = serde_json::from_str(&request("j/cursorpos").ok()?).ok()?;
    Some((point.x, point.y))
}

#[derive(Deserialize)]
struct Monitor {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default = "one")]
    scale: f64,
    /// Left, top, right and bottom space held by bars, in layout pixels.
    #[serde(default)]
    reserved: [i32; 4],
}

fn one() -> f64 {
    1.0
}

fn monitors() -> Vec<Monitor> {
    request("j/monitors")
        .ok()
        .and_then(|reply| serde_json::from_str(&reply).ok())
        .unwrap_or_default()
}

fn monitor_at(monitors: &[Monitor], x: f64, y: f64) -> Option<&Monitor> {
    monitors.iter().find(|m| {
        let (w, h) = (f64::from(m.width) / m.scale, f64::from(m.height) / m.scale);
        x >= f64::from(m.x) && y >= f64::from(m.y) && x < f64::from(m.x) + w && y < f64::from(m.y) + h
    })
}

/// The pointer in the physical pixels Tauri uses for monitors and windows:
/// layout coordinates times the scale of the monitor under the pointer.
pub fn cursor_physical() -> Option<(i32, i32)> {
    let (x, y) = cursor_position()?;
    let scale = monitor_at(&monitors(), f64::from(x), f64::from(y)).map_or(1.0, |m| m.scale);
    Some(((f64::from(x) * scale).round() as i32, (f64::from(y) * scale).round() as i32))
}

/// Height of the bar along the top of the monitor holding this physical
/// point, in physical pixels.
pub fn top_reserved_physical(x: i32, y: i32) -> i32 {
    let monitors = monitors();
    monitors
        .iter()
        .find(|m| {
            let (left, top) = ((f64::from(m.x) * m.scale) as i32, (f64::from(m.y) * m.scale) as i32);
            x >= left && y >= top && x < left + m.width && y < top + m.height
        })
        .map_or(0, |m| (f64::from(m.reserved[1]) * m.scale).round() as i32)
}

#[derive(Deserialize)]
struct Client {
    address: String,
    pid: i64,
    title: String,
    at: [i32; 2],
}

fn own_client(title: &str) -> Option<Client> {
    let clients: Vec<Client> = serde_json::from_str(&request("j/clients").ok()?).ok()?;
    let own = i64::from(std::process::id());
    clients
        .into_iter()
        .find(|client| client.pid == own && client.title == title)
}

fn own_window(title: &str) -> Option<String> {
    own_client(title).map(|client| client.address)
}

/// Where Hyprland has one of SonoJot's windows, in layout pixels.
pub fn window_position(title: &str) -> Option<(i32, i32)> {
    own_client(title).map(|client| (client.at[0], client.at[1]))
}

/// Move one of SonoJot's own windows, found by title, to a layout position.
/// A window shown a moment ago may not be mapped yet; the latest requested
/// position is then applied from a helper thread once it appears (up to 1 s).
pub fn move_window(title: &str, x: i32, y: i32) -> Result<(), String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static PENDING: Mutex<Option<HashMap<String, (i32, i32)>>> = Mutex::new(None);
    if let Some(address) = own_window(title) {
        if let Ok(mut pending) = PENDING.lock() {
            pending.get_or_insert_with(HashMap::new).remove(title);
        }
        return move_address(&address, x, y);
    }
    let first = {
        let mut pending = PENDING.lock().map_err(|_| "Window moves are unavailable.")?;
        pending.get_or_insert_with(HashMap::new).insert(title.to_owned(), (x, y)).is_none()
    };
    if first {
        let title = title.to_owned();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(15));
                if let Some(address) = own_window(&title) {
                    let target = PENDING
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.get_or_insert_with(HashMap::new).remove(&title));
                    if let Some((x, y)) = target {
                        let _ = move_address(&address, x, y);
                    }
                    return;
                }
            }
            if let Ok(mut pending) = PENDING.lock() {
                pending.get_or_insert_with(HashMap::new).remove(&title);
            }
        });
    }
    Ok(())
}

fn move_address(address: &str, x: i32, y: i32) -> Result<(), String> {
    let command = if lua() {
        format!("dispatch hl.dsp.window.move({{x={x}, y={y}, window=\"address:{address}\"}})")
    } else {
        format!("dispatch movewindowpixel exact {x} {y},address:{address}")
    };
    match request(&command)?.trim() {
        "ok" => Ok(()),
        reply => Err(format!("Hyprland could not move the window: {reply}")),
    }
}

// ---------------------------------------------------------------------------
// Generated config: shortcut binds and window rules.

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("hypr"))
}

/// One shortcut converted to Hyprland's notation.
#[derive(Debug, PartialEq, Eq)]
struct Keys {
    /// For example `["CTRL", "SHIFT"]`.
    mods: Vec<&'static str>,
    /// An xkb keysym name, for example `space`, `F9` or `d`.
    key: String,
}

fn parse_hotkey(hotkey: &str) -> Result<Keys, String> {
    let mut mods = Vec::new();
    let mut key = None;
    for part in hotkey.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "commandorcontrol" | "cmdorctrl" | "cmdorcontrol" => Some("CTRL"),
            "shift" => Some("SHIFT"),
            "alt" | "option" => Some("ALT"),
            "super" | "meta" | "cmd" | "command" | "win" | "windows" => Some("SUPER"),
            _ => None,
        };
        match modifier {
            Some(modifier) if !mods.contains(&modifier) => mods.push(modifier),
            Some(_) => {}
            None if key.is_none() => key = Some(keysym(part)?),
            None => return Err(format!("A shortcut has one main key: {hotkey}")),
        }
    }
    let key = key.ok_or_else(|| format!("The shortcut needs a main key: {hotkey}"))?;
    Ok(Keys { mods, key })
}

fn keysym(key: &str) -> Result<String, String> {
    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "space" => "space",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Prior",
        "pagedown" => "Next",
        "arrowup" | "up" => "Up",
        "arrowdown" | "down" => "Down",
        "arrowleft" | "left" => "Left",
        "arrowright" | "right" => "Right",
        "`" | "backquote" | "grave" => "grave",
        "-" | "minus" => "minus",
        "=" | "equal" => "equal",
        "," | "comma" => "comma",
        "." | "period" => "period",
        "/" | "slash" => "slash",
        ";" | "semicolon" => "semicolon",
        "'" | "quote" => "apostrophe",
        "[" | "bracketleft" => "bracketleft",
        "]" | "bracketright" => "bracketright",
        "\\" | "backslash" => "backslash",
        "pause" => "Pause",
        "scrolllock" => "Scroll_Lock",
        "printscreen" => "Print",
        _ => "",
    };
    if !named.is_empty() {
        return Ok(named.into());
    }
    if key.len() == 1 && key.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Ok(lower);
    }
    if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        if (1..=24).contains(&number) {
            return Ok(format!("F{number}"));
        }
    }
    Err(format!("SonoJot cannot bind {key} in Hyprland. Choose a letter, digit, F-key or Space."))
}

/// xkb names of the physical keys behind each modifier, for the hold watcher.
fn modifier_keys(modifier: &str) -> &'static [&'static str] {
    match modifier {
        "CTRL" => &["Control_L", "Control_R"],
        "SHIFT" => &["Shift_L", "Shift_R"],
        "ALT" => &["Alt_L", "Alt_R", "ISO_Level3_Shift"],
        _ => &["Super_L", "Super_R"],
    }
}

fn lua_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

const HEADER: &str = "Generated by SonoJot. Settings > Shortcuts rewrites this file; edit the shortcut there.";

/// Window rules for SonoJot's own windows. Overlays float above everything
/// without taking focus. The main window gets none, so it tiles and snaps
/// like any other app.
const LUA_RULES: &str = r#"
hl.window_rule({ match = { class = "^SonoJot$", title = "^SonoJot (screen tab|cursor status)$" },
  tag = "-default-opacity", float = true, pin = true, no_initial_focus = true,
  border_size = 0, no_shadow = true, no_blur = true, no_anim = true, no_dim = true, opacity = "1 1" })
hl.window_rule({ match = { class = "^SonoJot$", title = "^SonoJot screen tab$" },
  size = { 600, 104 }, move = { "(monitor_w-window_w)/2", "-200" } })
hl.window_rule({ match = { class = "^SonoJot$", title = "^SonoJot cursor status$" }, size = { 48, 48 } })
"#;

const CONF_RULES: &str = r#"
windowrule = float on, pin on, no_initial_focus on, border_size 0, no_shadow on, no_blur on, no_anim on, no_dim on, opacity 1 1, match:class ^SonoJot$, match:title ^SonoJot (screen tab|cursor status)$
windowrule = size 600 104, move ((monitor_w-window_w)/2) -200, match:class ^SonoJot$, match:title ^SonoJot screen tab$
windowrule = size 48 48, match:class ^SonoJot$, match:title ^SonoJot cursor status$
"#;

fn lua_config(keys: &Keys, hold: bool, executable: &Path) -> String {
    let combo = keys
        .mods
        .iter()
        .copied()
        .chain(std::iter::once(keys.key.as_str()))
        .collect::<Vec<_>>()
        .join(" + ");
    let command = |action: &str| lua_string(&format!("{} --shortcut {action}", shell_quote(executable)));
    let mut out = format!("-- {HEADER}\n");
    if hold {
        // A release bind misses the release when a modifier comes up before
        // the key. Instead the press starts a watcher that sends release once
        // any part of the shortcut is no longer held, whatever the order.
        let groups = keys
            .mods
            .iter()
            .map(|modifier| {
                let names = modifier_keys(modifier)
                    .iter()
                    .map(|name| format!("hl.is_key_down({})", lua_string(name)))
                    .collect::<Vec<_>>()
                    .join(" or ");
                format!(" and ({names})")
            })
            .collect::<String>();
        out.push_str(&format!(
            "local sonojot_watch = nil\n\
             hl.bind({combo}, function()\n\
             \x20 if sonojot_watch then return end\n\
             \x20 hl.exec_cmd({press})\n\
             \x20 sonojot_watch = hl.timer(function()\n\
             \x20   if not (hl.is_key_down({key}){groups}) then\n\
             \x20     sonojot_watch:set_enabled(false)\n\
             \x20     sonojot_watch = nil\n\
             \x20     hl.exec_cmd({release})\n\
             \x20   end\n\
             \x20 end, {{ timeout = 15, type = \"repeat\" }})\n\
             end, {{ description = \"SonoJot: hold to dictate\" }})\n",
            combo = lua_string(&combo),
            press = command("press"),
            release = command("release"),
            key = lua_string(&keys.key),
        ));
    } else {
        out.push_str(&format!(
            "hl.bind({}, hl.dsp.exec_cmd({}), {{ description = \"SonoJot: start or stop dictation\" }})\n",
            lua_string(&combo),
            command("press"),
        ));
    }
    out.push_str(LUA_RULES);
    out
}

fn conf_config(keys: &Keys, hold: bool, executable: &Path) -> String {
    let mods = keys.mods.join(" ");
    let exe = shell_quote(executable);
    let mut out = format!("# {HEADER}\n");
    out.push_str(&format!("bindd = {mods}, {}, SonoJot dictation, exec, {exe} --shortcut press\n", keys.key));
    if hold {
        // hyprlang has no key-state query: release the main key last.
        out.push_str(&format!("bindr = {mods}, {}, exec, {exe} --shortcut release\n", keys.key));
    }
    out.push_str(CONF_RULES);
    out
}

/// The line that loads the generated file from the user's own config.
fn include_line(lua: bool, generated: &Path) -> String {
    if lua {
        format!(
            "-- SonoJot dictation shortcut and window rules.\npcall(dofile, {})\n",
            lua_string(&generated.to_string_lossy())
        )
    } else {
        format!("# SonoJot dictation shortcut and window rules.\nsource = {}\n", generated.display())
    }
}

/// Where to add the include line: Omarchy's personal bindings file when it
/// exists, otherwise the main config.
fn include_target(dir: &Path, lua: bool) -> PathBuf {
    let (bindings, main) = if lua {
        ("bindings.lua", "hyprland.lua")
    } else {
        ("bindings.conf", "hyprland.conf")
    };
    let bindings = dir.join(bindings);
    if bindings.exists() { bindings } else { dir.join(main) }
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool, String> {
    if std::fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return Ok(false);
    }
    let temporary = path.with_extension("tmp-sonojot");
    std::fs::write(&temporary, contents)
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(true)
}

/// Bind the dictation shortcut in Hyprland. Writes the generated file, adds
/// one include line to the user's config the first time, then reloads
/// Hyprland's config when anything changed.
pub fn install_shortcut(hotkey: &str, hold: bool, executable: &Path) -> Result<(), String> {
    let keys = parse_hotkey(hotkey)?;
    let lua = lua();
    let dir = config_dir().ok_or("Cannot find the Hyprland config folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let generated = dir.join(if lua { "sonojot.lua" } else { "sonojot.conf" });
    let contents = if lua {
        lua_config(&keys, hold, executable)
    } else {
        conf_config(&keys, hold, executable)
    };
    let mut changed = write_if_changed(&generated, &contents)?;
    let target = include_target(&dir, lua);
    let current = std::fs::read_to_string(&target).unwrap_or_default();
    let name = generated.file_name().and_then(|name| name.to_str()).unwrap_or("sonojot");
    if !current.contains(name) {
        let mut updated = current;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push('\n');
        updated.push_str(&include_line(lua, &generated));
        std::fs::write(&target, updated)
            .map_err(|e| format!("Cannot add SonoJot to {}: {e}", target.display()))?;
        changed = true;
    }
    if changed {
        reload()?;
    }
    Ok(())
}

fn reload() -> Result<(), String> {
    let reply = request("reload config-only")?;
    if reply.trim() != "ok" {
        return Err(format!("Hyprland could not reload its config: {}", reply.trim()));
    }
    Ok(())
}

/// Free the shortcut when SonoJot quits, as a released key grab would be on
/// Windows: the generated file keeps only its window rules until the next
/// launch binds the shortcut again. A crash leaves the bind; the key then
/// runs a messenger that finds no app.
pub fn release_shortcut() {
    let lua = lua();
    let Some(dir) = config_dir() else { return };
    let generated = dir.join(if lua { "sonojot.lua" } else { "sonojot.conf" });
    if !generated.exists() {
        return;
    }
    let contents = if lua {
        format!("-- {HEADER}\n-- SonoJot is not running, so no shortcut is bound.\n{LUA_RULES}")
    } else {
        format!("# {HEADER}\n# SonoJot is not running, so no shortcut is bound.\n{CONF_RULES}")
    };
    if write_if_changed(&generated, &contents).unwrap_or(false) {
        let _ = reload();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_convert_to_hyprland_notation() {
        assert_eq!(
            parse_hotkey("Ctrl+Space").unwrap(),
            Keys { mods: vec!["CTRL"], key: "space".into() }
        );
        assert_eq!(
            parse_hotkey("Alt + Shift + D").unwrap(),
            Keys { mods: vec!["ALT", "SHIFT"], key: "d".into() }
        );
        assert_eq!(parse_hotkey("F9").unwrap(), Keys { mods: vec![], key: "F9".into() });
        assert_eq!(
            parse_hotkey("Super+`").unwrap(),
            Keys { mods: vec!["SUPER"], key: "grave".into() }
        );
        assert!(parse_hotkey("Ctrl+Shift").is_err());
        assert!(parse_hotkey("Ctrl+A+B").is_err());
        assert!(parse_hotkey("Ctrl+F99").is_err());
    }

    #[test]
    fn hold_config_watches_every_modifier_and_quotes_the_path() {
        let keys = parse_hotkey("Ctrl+Space").unwrap();
        let config = lua_config(&keys, true, Path::new("/opt/Sono Jot/it's/SonoJot"));
        assert!(config.contains("hl.bind(\"CTRL + space\", function()"));
        assert!(config.contains("hl.is_key_down(\"space\") and (hl.is_key_down(\"Control_L\") or hl.is_key_down(\"Control_R\"))"));
        assert!(config.contains(r#"hl.exec_cmd("'/opt/Sono Jot/it'\\''s/SonoJot' --shortcut release")"#));
        let toggle = lua_config(&keys, false, Path::new("/usr/bin/SonoJot"));
        assert!(toggle.contains(r#"hl.bind("CTRL + space", hl.dsp.exec_cmd("'/usr/bin/SonoJot' --shortcut press")"#));
        assert!(!toggle.contains("release"));
    }

    #[test]
    fn terminals_are_recognised_by_tag_or_class() {
        let window = |class: &str, tags: &[&str]| Window {
            address: "0x1".into(),
            pid: 1,
            class: class.into(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        };
        assert!(window("Alacritty", &[]).is_terminal());
        assert!(window("custom", &["terminal*"]).is_terminal());
        assert!(!window("chromium", &["chromium-based-browser*"]).is_terminal());
    }
}
