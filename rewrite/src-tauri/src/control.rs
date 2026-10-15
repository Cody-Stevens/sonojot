//! Shortcut handoff on Linux. Wayland lets no application grab keys, so the
//! compositor binds the shortcut and runs `SonoJot --shortcut press` (and
//! `release` in hold mode). That short-lived process passes the event to the
//! running app over a user-only Unix socket and exits before any GUI or
//! worker code starts.
use std::{
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::ShortcutState;

fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    runtime
        .is_absolute()
        .then(|| runtime.join("sonojot").join("control.sock"))
}

/// Handle `--shortcut press|release` from argv. Returns the exit code when
/// this process was only a messenger.
pub fn forward_from_args() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--shortcut" {
            let action = args.next().unwrap_or_default();
            return Some(match send(&action) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("SonoJot: {error}");
                    1
                }
            });
        }
    }
    None
}

fn send(action: &str) -> Result<(), String> {
    if !matches!(action, "press" | "release") {
        return Err("Use --shortcut press or --shortcut release.".into());
    }
    let path = socket_path().ok_or("XDG_RUNTIME_DIR is not set.")?;
    let mut stream =
        UnixStream::connect(&path).map_err(|_| "SonoJot is not running.".to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();
    stream
        .write_all(format!("{action}\n").as_bytes())
        .map_err(|e| format!("Cannot reach SonoJot: {e}"))?;
    let mut reply = String::new();
    let _ = stream.take(64).read_to_string(&mut reply);
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(format!("SonoJot did not accept the shortcut: {}", reply.trim()))
    }
}

/// Listen for shortcut events for the life of the app. Only one SonoJot runs
/// per session (single-instance), so a leftover socket is stale.
pub fn serve(app: &AppHandle) -> Result<(), String> {
    let path = socket_path().ok_or("XDG_RUNTIME_DIR is not set.")?;
    let dir = path.parent().ok_or("Invalid control socket path.")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("Cannot open the shortcut socket: {e}"))?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let app = app.clone();
    std::thread::Builder::new()
        .name("shortcut-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                handle(&app, stream);
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn handle(app: &AppHandle, mut stream: UnixStream) {
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();
    let mut buffer = [0u8; 32];
    let mut length = 0;
    while length < buffer.len() {
        match stream.read(&mut buffer[length..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                length += n;
                if buffer[..length].contains(&b'\n') {
                    break;
                }
            }
        }
    }
    let reply = match std::str::from_utf8(&buffer[..length]).map(str::trim) {
        Ok("press") => {
            crate::app::shortcut_event(app, ShortcutState::Pressed);
            "ok"
        }
        Ok("release") => {
            crate::app::shortcut_event(app, ShortcutState::Released);
            "ok"
        }
        _ => "unknown request",
    };
    let _ = stream.write_all(format!("{reply}\n").as_bytes());
}
