//! macOS focus probe: does building SonoJot's cursor-indicator window take
//! the user's app out of the front?
//!
//! Run it from `scripts/mac-focus-probe.sh`, which bundles it as an app (a
//! GUI session needs LaunchServices), puts TextEdit in front and then launches
//! `--mode plain` (a normal Tauri build, as through 0.28.7) and `--mode quiet`
//! (`build_quiet`, the fix). Each run writes one JSON line to `--out`.
//!
//! macOS 14 ignores activation requests from an app that is not handling user
//! input, so only `--hotkey` reproduces what a dictation shortcut does: the
//! window is built when someone presses that shortcut on the Mac.

#[cfg(target_os = "macos")]
fn main() {
    probe::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("focus_probe runs on macOS only.");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
mod probe {
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tauri::{window::Color, AppHandle, WebviewUrl, WebviewWindowBuilder};
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    struct Args {
        quiet: bool,
        out: PathBuf,
        wait: Duration,
        hotkey: Option<String>,
    }

    fn args() -> Result<Args, String> {
        let mut quiet = None;
        let mut out = None;
        let mut wait = Duration::from_millis(3000);
        let mut hotkey = None;
        let mut values = std::env::args().skip(1);
        while let Some(flag) = values.next() {
            let value = values.next().ok_or(format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--mode" => quiet = Some(match value.as_str() {
                    "plain" => false,
                    "quiet" => true,
                    _ => return Err("--mode is plain or quiet".into()),
                }),
                "--out" => out = Some(value.into()),
                "--hotkey" => hotkey = Some(value),
                "--wait-ms" => {
                    wait = Duration::from_millis(value.parse().map_err(|_| "bad --wait-ms")?)
                }
                _ => return Err(format!("unknown flag {flag}")),
            }
        }
        Ok(Args {
            quiet: quiet.ok_or("--mode is required")?,
            out: out.ok_or("--out is required")?,
            wait,
            hotkey,
        })
    }

    /// The frontmost application's pid and name.
    fn frontmost() -> (i32, String) {
        autoreleasepool(|_| unsafe {
            let workspace: Option<Retained<AnyObject>> =
                msg_send![class!(NSWorkspace), sharedWorkspace];
            let Some(workspace) = workspace else {
                return (0, String::new());
            };
            let application: Option<Retained<AnyObject>> =
                msg_send![&*workspace, frontmostApplication];
            let Some(application) = application else {
                return (0, String::new());
            };
            let pid: i32 = msg_send![&*application, processIdentifier];
            let name: Option<Retained<AnyObject>> = msg_send![&*application, localizedName];
            let name = name
                .map(|name| {
                    let utf8: *const std::ffi::c_char = msg_send![&*name, UTF8String];
                    std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()
                })
                .unwrap_or_default();
            (pid, name)
        })
    }

    fn own_app_active(app: &AppHandle) -> Option<bool> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let active = autoreleasepool(|_| unsafe {
                let application: Option<Retained<AnyObject>> =
                    msg_send![class!(NSApplication), sharedApplication];
                application.is_some_and(|application| msg_send![&*application, isActive])
            });
            let _ = sender.send(active);
        })
        .ok()?;
        receiver.recv_timeout(Duration::from_secs(2)).ok()
    }

    fn measure(app: &AppHandle, quiet: bool, trigger: &str) -> serde_json::Value {
        let own = std::process::id() as i32;
        let before = frontmost();
        // The same window the cursor indicator builds (windows::show_cursor).
        let builder = WebviewWindowBuilder::new(app, "cursor", WebviewUrl::App("index.html".into()))
            .title("Focus probe")
            .inner_size(48.0, 48.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .visible(false);
        let built = if quiet {
            sonojot_core::build_quiet(app, builder)
        } else {
            builder.build()
        };
        std::thread::sleep(Duration::from_millis(1000));
        let after = frontmost();
        let active = own_app_active(app);
        let valid = before.0 != 0 && before.0 != own;
        serde_json::json!({
            "mode": if quiet { "quiet" } else { "plain" },
            "trigger": trigger,
            "built": built.is_ok(),
            "error": built.err().map(|error| error.to_string()),
            "valid": valid,
            "before": { "pid": before.0, "name": before.1 },
            "after": { "pid": after.0, "name": after.1 },
            "probe_active_after": active,
            "focus_kept": valid && after.0 == before.0 && active == Some(false),
        })
    }

    fn finish(app: &AppHandle, out: &Path, result: serde_json::Value) {
        let _ = std::fs::write(out, format!("{result}\n"));
        app.exit(0);
    }

    pub fn main() {
        let args = match args() {
            Ok(args) => args,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
        tauri::Builder::default()
            .plugin(tauri_plugin_global_shortcut::Builder::new().build())
            .setup(move |app| {
                let handle = app.handle().clone();
                let Some(hotkey) = args.hotkey.clone() else {
                    // The launcher hands the front back to TextEdit during this wait.
                    std::thread::spawn(move || {
                        std::thread::sleep(args.wait);
                        finish(&handle, &args.out, measure(&handle, args.quiet, "timer"));
                    });
                    return Ok(());
                };
                let fired = Arc::new(AtomicBool::new(false));
                let (quiet, out) = (args.quiet, args.out);
                handle
                    .global_shortcut()
                    .on_shortcut(hotkey.as_str(), move |app, _, event| {
                        if event.state() != ShortcutState::Pressed
                            || fired.swap(true, Ordering::AcqRel)
                        {
                            return;
                        }
                        let (app, out) = (app.clone(), out.clone());
                        std::thread::spawn(move || {
                            finish(&app, &out, measure(&app, quiet, "hotkey"))
                        });
                    })?;
                Ok(())
            })
            .run(tauri::generate_context!("probe/tauri.conf.json"))
            .expect("focus probe failed to start");
    }
}
