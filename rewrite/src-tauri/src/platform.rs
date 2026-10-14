//! Native text delivery and login registration. Capture the paste target before
//! showing app UI. Delivery may return from this app to that captured target, but
//! never displaces another foreground application or restores old clipboard data.
//! Windows sends Ctrl+V with SendInput; macOS posts Command+V to the HID event
//! tap once Accessibility access is granted. Both leave the text on the clipboard.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteTarget {
    #[cfg(windows)]
    foreground: usize,
    #[cfg(windows)]
    focused: usize,
    #[cfg(windows)]
    process_id: u32,
    #[cfg(windows)]
    thread_id: u32,
    #[cfg(windows)]
    focused_process_id: u32,
    #[cfg(windows)]
    focused_thread_id: u32,
    /// The frontmost application. macOS exposes no focused-control identity
    /// without Accessibility queries, so the application is the target.
    #[cfg(target_os = "macos")]
    pid: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryOutcome {
    /// The OS accepted the paste key events; the destination does not acknowledge insertion.
    pub paste_sent: bool,
    /// Present when text was copied but automatic insertion was skipped.
    pub reason: Option<String>,
    /// Stable code for diagnostics; no transcript or window title is included.
    pub reason_code: &'static str,
    /// Numeric window/focus metadata and timing, suitable for a local diagnostic log.
    pub diagnostics: String,
}

impl DeliveryOutcome {
    fn copied(reason_code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            paste_sent: false,
            reason: Some(reason.into()),
            reason_code,
            diagnostics: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // macOS uses only some states.
enum FocusState {
    Ready,
    OwnApp,
    NoForeground,
    TargetFocusPending,
    TargetFocusChanged,
    OtherApp,
    TargetInvalid,
}

impl FocusState {
    fn fallback(self) -> DeliveryOutcome {
        match self {
            Self::TargetInvalid => DeliveryOutcome::copied("target_invalid", "The original text target was closed or replaced. Text is ready to paste."),
            Self::OtherApp => DeliveryOutcome::copied("external_focus_changed", "Focus changed to another application. Text is ready to paste."),
            Self::TargetFocusChanged => DeliveryOutcome::copied("target_control_changed", "The focused control in the original application changed. Text is ready to paste."),
            Self::OwnApp => DeliveryOutcome::copied("restore_not_completed", if cfg!(target_os = "macos") { "macOS did not return focus from SonoJot to the original app. Text is ready to paste." } else { "Windows did not return focus from SonoJot to the original text target. Text is ready to paste." }),
            Self::NoForeground | Self::TargetFocusPending => DeliveryOutcome::copied("focus_not_ready", "The original text target did not regain keyboard focus. Text is ready to paste."),
            Self::Ready => DeliveryOutcome::copied("modifiers_held", "A shortcut modifier is still held. Text is ready to paste."),
        }
    }
}

pub fn capture_paste_target() -> Option<PasteTarget> {
    native::capture_target()
}

/// For a click on the macOS menu bar sheet. The click can activate SonoJot
/// before the command runs, so fall back to the app that was active just
/// before SonoJot. Elsewhere this is `capture_paste_target`.
pub fn capture_paste_target_or_previous() -> Option<PasteTarget> {
    #[cfg(target_os = "macos")]
    {
        native::capture_target().or_else(|| {
            crate::mac_paste::previous_external_pid().map(|pid| PasteTarget { pid })
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        native::capture_target()
    }
}

/// Start tracking application activations (macOS); a no-op elsewhere.
pub fn observe_app_activations() {
    #[cfg(target_os = "macos")]
    crate::mac_paste::observe_activations();
}

/// macOS Accessibility trust, which posting the paste keys requires.
/// Other platforms need no permission and report true.
pub fn paste_permission_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::mac_paste::accessibility_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
const MAC_PERMISSION_REASON: &str =
    "Automatic paste needs Accessibility access. Until then, text is copied; press Command+V to paste.";

/// Returns from our explicitly focused controller only; never activates over
/// another application and never injects input or touches the clipboard.
pub fn restore_target_focus(target: &PasteTarget) -> bool {
    native::restore_from_own_app(target)
}

/// Capture-time diagnostics contain handles and process IDs, never window titles
/// or document contents. Call immediately after capturing the target.
pub fn paste_target_diagnostics(target: Option<&PasteTarget>) -> String {
    native::focus_diagnostics(target)
}

pub fn auto_paste_available() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// Explains platform/permission limitations without claiming untested support.
pub fn auto_paste_unavailable_reason() -> Option<String> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(target_os = "macos")]
    {
        (!paste_permission_granted()).then(|| MAC_PERMISSION_REASON.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Some(
            "Automatic paste is unavailable on this platform. Use the clipboard paste shortcut."
                .into(),
        )
    }
}

static DELIVERY_GATE: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn delivery_gate() -> &'static tokio::sync::Mutex<()> {
    DELIVERY_GATE.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Leaves the supplied text on the clipboard. Retrying clipboard contention never blocks the UI.
pub async fn copy_text(text: &str) -> Result<(), String> {
    let _guard = delivery_gate().lock().await;
    copy_with_retry(text).await.map(|_| ())
}

/// Acquire the OS delivery gate before claiming a session-bound manual copy.
/// The returned claim stays alive through clipboard contention/retries, so the
/// caller can prevent a new capture from overtaking an accepted review copy.
pub(crate) async fn copy_text_after_claim<G>(
    text: &str,
    claim: impl FnOnce() -> Result<G, String>,
) -> Result<(), String> {
    let _gate = delivery_gate().lock().await;
    let _claim = claim()?;
    copy_with_retry(text).await.map(|_| ())
}

async fn copy_with_retry(text: &str) -> Result<(u32, u8), String> {
    if text.contains('\0') {
        return Err("Clipboard text contains a NUL character.".into());
    }
    let mut last_error = String::new();
    for (attempt, delay_ms) in [0, 8, 16, 24, 32, 48, 64].into_iter().enumerate() {
        if delay_ms != 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let owned = text.to_owned();
        match tokio::task::spawn_blocking(move || native::set_clipboard(&owned)).await {
            Ok(Ok(sequence)) => return Ok((sequence, attempt as u8 + 1)),
            Ok(Err(error)) => last_error = error,
            Err(error) => return Err(format!("Clipboard worker failed: {error}")),
        }
        // Other platforms do not expose transient Windows clipboard contention.
        if !cfg!(windows) {
            break;
        }
    }
    Err(format!(
        "Could not copy text to the clipboard: {last_error}"
    ))
}

/// The target must come from the start of this dictation session, not completion.
/// Late recognizer/cleanup results must never call this function a second time.
pub async fn deliver_text(
    text: &str,
    target: Option<PasteTarget>,
    auto_paste: bool,
) -> Result<DeliveryOutcome, String> {
    let started = Instant::now();
    let _guard = delivery_gate().lock().await;
    let initial_focus = native::focus_diagnostics(target.as_ref());
    let (sequence, clipboard_attempts) = copy_with_retry(text).await?;
    let mut restore_attempted = false;
    let mut restore_accepted = false;
    let result = async {
        if !auto_paste {
            return DeliveryOutcome::copied("disabled", "Automatic paste is turned off.");
        }
        if !auto_paste_available() {
            return DeliveryOutcome::copied(
                "unsupported",
                auto_paste_unavailable_reason().unwrap_or_default(),
            );
        }
        if !paste_permission_granted() {
            return DeliveryOutcome::copied(
                "permission_missing",
                auto_paste_unavailable_reason().unwrap_or_default(),
            );
        }
        let Some(target) = target.as_ref() else {
            return DeliveryOutcome::copied(
                "target_missing",
                "No external text target was captured. Text is ready to paste.",
            );
        };
        // Finishing from the overlay may focus this app. It is safe to hand focus
        // back only while this process owns foreground and the exact target survives.
        // A deliberate switch to any other app always ends delivery without input.
        // macOS activation is asynchronous and the target's window needs a
        // moment to become key, so it gets a longer deadline and a short settle.
        let (window, settle) = if cfg!(target_os = "macos") {
            (Duration::from_millis(1000), Duration::from_millis(60))
        } else {
            (Duration::from_millis(450), Duration::ZERO)
        };
        let deadline = Instant::now() + window;
        let mut ready_since: Option<Instant> = None;
        loop {
            let focus = native::focus_state(target);
            match focus {
                FocusState::OtherApp | FocusState::TargetInvalid => return focus.fallback(),
                FocusState::TargetFocusChanged if !restore_attempted => return focus.fallback(),
                FocusState::OwnApp if !restore_attempted => {
                    restore_attempted = true;
                    restore_accepted = native::restore_from_own_app(target);
                }
                _ => {}
            }
            if !native::clipboard_matches(sequence) {
                return DeliveryOutcome::copied(
                    "clipboard_changed",
                    "The clipboard changed before automatic paste; no keys were sent.",
                );
            }
            if focus == FocusState::Ready {
                ready_since.get_or_insert_with(Instant::now);
            } else {
                ready_since = None;
            }
            let settled = !restore_attempted
                || ready_since.is_some_and(|since| since.elapsed() >= settle);
            if focus == FocusState::Ready && settled && !native::modifiers_pressed() {
                let outcome = native::paste_if_current(target, sequence, text);
                // These two codes are emitted only before any key was sent. Never
                // retry a successful or partially accepted input batch.
                if !matches!(outcome.reason_code, "clipboard_busy" | "modifiers_held")
                    || Instant::now() >= deadline
                {
                    return outcome;
                }
            }
            if Instant::now() >= deadline {
                return native::focus_state(target).fallback();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    .await;
    let mut outcome = result;
    outcome.diagnostics = format!(
        "code={} elapsed_ms={} clipboard_attempts={} restore_attempted={} restore_accepted={} initial=[{}] final=[{}]",
        outcome.reason_code, started.elapsed().as_millis(), clipboard_attempts,
        restore_attempted, restore_accepted, initial_focus,
        native::focus_diagnostics(target.as_ref()),
    );
    Ok(outcome)
}

/// The installer/composition layer supplies the stable installed executable.
/// Never derive this from the current debug process or search legacy installations.
pub fn startup_enabled(installed_executable: &Path) -> Result<bool, String> {
    let executable = validated_startup_executable(installed_executable)?;
    native::startup_enabled(&executable)
}

/// Update only this installation's existing login item after a binary rename.
/// An absent, disabled or unrelated registration is never enabled here.
pub fn reconcile_startup_registration(installed_executable: &Path) -> Result<bool, String> {
    let executable = validated_startup_executable(installed_executable)?;
    native::migrate_startup_registration(&executable)?;
    native::startup_enabled(&executable)
}

fn legacy_startup_executables(executable: &Path) -> Vec<PathBuf> {
    let (branded, cargo, previous) = if cfg!(windows) {
        ("SonoJot.exe", "sonojot.exe", "eavesdropper-next.exe")
    } else {
        ("SonoJot", "sonojot", "eavesdropper-next")
    };
    if !executable
        .file_name()
        .is_some_and(|name| name == branded || name == cargo)
    {
        return Vec::new();
    }
    [previous, cargo]
        .into_iter()
        .map(|name| executable.with_file_name(name))
        .filter(|path| path != executable)
        .collect()
}

/// Move a login-item file written under the pre-SonoJot name to its SonoJot
/// name. Only a file this installation wrote (`belongs`) is moved: the new file
/// is written first, then the old one removed, so a failure never leaves the
/// user without a registration. A disabled or unrelated file stays untouched.
#[cfg_attr(windows, allow(dead_code))]
fn hand_over_registration_file(
    previous: &Path,
    current: &Path,
    belongs: impl Fn(&str) -> bool,
    contents: &str,
) -> Result<bool, String> {
    let actual = match std::fs::read_to_string(previous) {
        Ok(actual) => actual,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Cannot read the previous login registration: {e}")),
    };
    if !belongs(&actual) {
        return Ok(false);
    }
    write_registration_file(current, contents)?;
    match std::fs::remove_file(previous) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(format!("Cannot remove the previous login registration: {e}")),
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn write_registration_file(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Invalid login registration directory.")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Cannot create login registration directory: {e}"))?;
    let temporary = path.with_extension("next-tmp");
    std::fs::write(&temporary, contents)
        .map_err(|e| format!("Cannot write login registration: {e}"))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("Cannot install login registration: {e}"))
}

pub fn set_startup_enabled(enabled: bool, installed_executable: &Path) -> Result<bool, String> {
    let executable = validated_startup_executable(installed_executable)?;
    native::set_startup_enabled(enabled, &executable)?;
    let actual = native::startup_enabled(&executable)?;
    if actual != enabled {
        return Err(
            "Login registration did not match the requested state after writing it.".into(),
        );
    }
    Ok(actual)
}

fn validated_startup_executable(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || !path.is_file() {
        return Err(
            "Launch at login requires an existing, absolute installed executable path.".into(),
        );
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve installed executable: {e}"))?;
    let raw = canonical
        .to_str()
        .ok_or("The installed executable path is not valid Unicode.")?;
    if raw.contains('"') || raw.chars().any(char::is_control) {
        return Err("The installed executable path contains unsupported characters.".into());
    }
    let components: Vec<_> = canonical
        .components()
        .map(|v| v.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    if components
        .windows(2)
        .any(|p| p[0] == "target" && matches!(p[1].as_str(), "debug" | "release"))
    {
        return Err(
            "Launch at login must use an installed executable, not a Cargo build output.".into(),
        );
    }
    #[cfg(windows)]
    if canonical
        .extension()
        .and_then(|v| v.to_str())
        .is_none_or(|v| !v.eq_ignore_ascii_case("exe"))
    {
        return Err("Launch at login requires an installed .exe file.".into());
    }
    Ok(canonical)
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::mem::size_of;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GlobalFree, HWND};
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use winreg::{enums::*, RegKey};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    // Task Manager keeps each Run value's enabled/disabled choice here, by name.
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const RUN_VALUE: &str = "SonoJot";
    // Builds up to 0.28.3 registered under this name. The C# app's own
    // "Eavesdropper" value is a different name and is never touched.
    const PREVIOUS_RUN_VALUE: &str = "EavesdropperNext";

    #[derive(Clone, Copy, Debug, Default)]
    struct Foreground {
        window: usize,
        focused: usize,
        process_id: u32,
        thread_id: u32,
        focus_query_ok: bool,
    }

    fn foreground() -> Foreground {
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() || IsWindow(window) == 0 {
                return Foreground::default();
            }
            let mut process_id = 0;
            let thread_id = GetWindowThreadProcessId(window, &mut process_id);
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = size_of::<GUITHREADINFO>() as u32;
            let focus_query_ok = thread_id != 0 && GetGUIThreadInfo(thread_id, &mut info) != 0;
            Foreground {
                window: window as usize,
                focused: info.hwndFocus as usize,
                process_id,
                thread_id,
                focus_query_ok,
            }
        }
    }

    fn focus_belongs_to(window: usize, focused: usize) -> bool {
        focused != 0
            && (focused == window || unsafe { IsChild(window as HWND, focused as HWND) } != 0)
    }

    pub(super) fn capture_target() -> Option<PasteTarget> {
        let current = foreground();
        if current.process_id == 0
            || current.process_id == std::process::id()
            || !current.focus_query_ok
            || !focus_belongs_to(current.window, current.focused)
        {
            return None;
        }
        let mut focused_process_id = 0;
        let focused_thread_id =
            unsafe { GetWindowThreadProcessId(current.focused as HWND, &mut focused_process_id) };
        if focused_process_id == 0 || focused_thread_id == 0 {
            return None;
        }
        Some(PasteTarget {
            foreground: current.window,
            focused: current.focused,
            process_id: current.process_id,
            thread_id: current.thread_id,
            focused_process_id,
            focused_thread_id,
        })
    }

    fn target_valid(target: &PasteTarget) -> bool {
        unsafe {
            if IsWindow(target.foreground as HWND) == 0
                || IsWindow(target.focused as HWND) == 0
                || !focus_belongs_to(target.foreground, target.focused)
            {
                return false;
            }
            let mut process_id = 0;
            let thread_id = GetWindowThreadProcessId(target.foreground as HWND, &mut process_id);
            let mut focused_process_id = 0;
            let focused_thread_id =
                GetWindowThreadProcessId(target.focused as HWND, &mut focused_process_id);
            process_id == target.process_id
                && thread_id == target.thread_id
                && focused_process_id == target.focused_process_id
                && focused_thread_id == target.focused_thread_id
        }
    }

    fn classify_focus(target: &PasteTarget, current: Foreground, valid: bool) -> FocusState {
        if !valid {
            return FocusState::TargetInvalid;
        }
        if current.window == 0 {
            return FocusState::NoForeground;
        }
        if current.process_id == std::process::id() {
            return FocusState::OwnApp;
        }
        if current.window != target.foreground
            || current.process_id != target.process_id
            || current.thread_id != target.thread_id
        {
            return FocusState::OtherApp;
        }
        if !current.focus_query_ok || current.focused == 0 {
            return FocusState::TargetFocusPending;
        }
        if current.focused != target.focused {
            return FocusState::TargetFocusChanged;
        }
        FocusState::Ready
    }

    pub(super) fn focus_state(target: &PasteTarget) -> FocusState {
        classify_focus(target, foreground(), target_valid(target))
    }

    pub(super) fn restore_from_own_app(target: &PasteTarget) -> bool {
        if !target_valid(target) {
            return false;
        }
        let current = foreground();
        if current.window == 0 || current.process_id != std::process::id() {
            return false;
        }
        // No thread-input attachment, synthesized Alt, or forced child focus.
        // Windows/the target restores its own saved keyboard focus on activation.
        // If that is not the captured control, bounded delivery falls back to copy.
        unsafe { SetForegroundWindow(target.foreground as HWND) != 0 }
    }

    pub(super) fn focus_diagnostics(target: Option<&PasteTarget>) -> String {
        let current = foreground();
        let capture_status = if current.window == 0 {
            "no_foreground"
        } else if current.process_id == std::process::id() {
            "own_app"
        } else if !current.focus_query_ok {
            "focus_query_failed"
        } else if current.focused == 0 {
            "no_focused_control"
        } else if !focus_belongs_to(current.window, current.focused) {
            "focus_outside_window"
        } else {
            "external_target"
        };
        let target_info = target.map(|target| format!("target_pid={} target_thread={} target_window={:#x} target_focus={:#x} target_focus_pid={} target_focus_thread={} target_valid={}", target.process_id, target.thread_id, target.foreground, target.focused, target.focused_process_id, target.focused_thread_id, target_valid(target))).unwrap_or_else(|| "target=none".into());
        format!("{target_info} observed_pid={} observed_thread={} observed_window={:#x} observed_focus={:#x} focus_query_ok={} capture_status={} modifiers_held={}", current.process_id, current.thread_id, current.window, current.focused, current.focus_query_ok, capture_status, modifiers_pressed())
    }

    struct ClipboardOwner(HWND);
    impl Drop for ClipboardOwner {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    struct OpenClipboardGuard;
    impl Drop for OpenClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }

    pub(super) fn set_clipboard(text: &str) -> Result<u32, String> {
        let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            );
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let _owner = ClipboardOwner(hwnd);
            if OpenClipboard(hwnd) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let _clipboard = OpenClipboardGuard;
            // Allocate before emptying so allocation failure preserves the clipboard.
            let memory = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * size_of::<u16>());
            if memory.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let destination = GlobalLock(memory).cast::<u16>();
            if destination.is_null() {
                let error = std::io::Error::last_os_error().to_string();
                GlobalFree(memory);
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), destination, utf16.len());
            GlobalUnlock(memory);
            if EmptyClipboard() == 0 || SetClipboardData(13 /* CF_UNICODETEXT */, memory).is_null()
            {
                let error = std::io::Error::last_os_error().to_string();
                GlobalFree(memory);
                return Err(error);
            }
            // Closing the clipboard/owner can itself advance the sequence number.
            // Inspect text as well as this token before injecting, so an unrelated
            // writer in the close/read gap can never replace our paste payload.
            drop(_clipboard);
            drop(_owner);
            Ok(GetClipboardSequenceNumber())
        }
    }

    pub(super) fn clipboard_matches(sequence: u32) -> bool {
        unsafe { GetClipboardSequenceNumber() == sequence }
    }

    pub(super) fn modifiers_pressed() -> bool {
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(*key as i32) < 0 })
    }

    fn key_input(key: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn clipboard_has_text(text: &str) -> bool {
        unsafe {
            let data = GetClipboardData(13);
            if data.is_null() {
                return false;
            }
            let expected: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            if GlobalSize(data) < expected.len() * size_of::<u16>() {
                return false;
            }
            let pointer = GlobalLock(data).cast::<u16>();
            if pointer.is_null() {
                return false;
            }
            let matches =
                std::slice::from_raw_parts(pointer, expected.len()) == expected.as_slice();
            GlobalUnlock(data);
            matches
        }
    }

    pub(super) fn paste_if_current(
        target: &PasteTarget,
        sequence: u32,
        text: &str,
    ) -> DeliveryOutcome {
        // Inspect the payload while holding the clipboard, then release it so
        // the destination can read as soon as its paste shortcut is dispatched.
        if unsafe { OpenClipboard(null_mut()) } == 0 {
            return DeliveryOutcome::copied(
                "clipboard_busy",
                "The clipboard is busy. Text is ready to paste.",
            );
        }
        let _clipboard = OpenClipboardGuard;
        let focus = focus_state(target);
        if focus != FocusState::Ready {
            return focus.fallback();
        }
        if !clipboard_matches(sequence) {
            return DeliveryOutcome::copied(
                "clipboard_changed",
                "The clipboard changed before automatic paste; no keys were sent.",
            );
        }
        if !clipboard_has_text(text) {
            return DeliveryOutcome::copied(
                "clipboard_payload_changed",
                "The clipboard text changed before automatic paste; no keys were sent.",
            );
        }
        if modifiers_pressed() {
            return FocusState::Ready.fallback();
        }
        drop(_clipboard);
        // SendInput targets the foreground input queue: Windows provides no atomic
        // focus-check-and-paste primitive. Keep this final check adjacent to it.
        let focus = focus_state(target);
        if focus != FocusState::Ready {
            return focus.fallback();
        }
        if !clipboard_matches(sequence) {
            return DeliveryOutcome::copied(
                "clipboard_changed",
                "The clipboard changed before automatic paste; no keys were sent.",
            );
        }
        if modifiers_pressed() {
            return FocusState::Ready.fallback();
        }
        let inputs = [
            key_input(VK_CONTROL, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL, true),
        ];
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent == inputs.len() as u32 {
            DeliveryOutcome {
                paste_sent: true,
                reason: None,
                reason_code: "paste_sent",
                diagnostics: String::new(),
            }
        } else {
            // Release only keys this incomplete batch may have depressed. Never retry Ctrl+V.
            if sent > 0 {
                let release = [key_input(VK_V, true), key_input(VK_CONTROL, true)];
                unsafe {
                    SendInput(
                        release.len() as u32,
                        release.as_ptr(),
                        size_of::<INPUT>() as i32,
                    );
                }
            }
            DeliveryOutcome::copied("input_rejected", "Windows did not accept the complete paste shortcut. Text remains on the clipboard (an elevated target may reject input).")
        }
    }

    fn startup_command(executable: &Path) -> String {
        // Canonical paths carry an extended path prefix which is unnecessary in a Run value.
        let path = executable.to_string_lossy();
        let normal = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            path.strip_prefix(r"\\?\").unwrap_or(&path).to_string()
        };
        format!("\"{normal}\" --startup")
    }

    pub(super) fn startup_enabled(executable: &Path) -> Result<bool, String> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let key = match root.open_subkey_with_flags(RUN_KEY, KEY_READ) {
            Ok(key) => key,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("Cannot read launch-at-login registration: {e}")),
        };
        match key.get_value::<String, _>(RUN_VALUE) {
            Ok(actual) => Ok(actual == startup_command(executable)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("Cannot read launch-at-login value: {e}")),
        }
    }

    pub(super) fn migrate_startup_registration(executable: &Path) -> Result<(), String> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        match root.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE) {
            Ok(run) => {
                let approved = root.open_subkey_with_flags(APPROVED_KEY, KEY_READ | KEY_WRITE).ok();
                hand_over_run_value(&run, approved.as_ref(), executable)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Cannot open launch-at-login registration: {e}")),
        }
        for previous in legacy_startup_executables(executable) {
            if startup_enabled(&previous)? {
                set_startup_enabled(true, executable)?;
                break;
            }
        }
        Ok(())
    }

    /// The command was written for this installation, by this or an earlier build.
    fn run_command_belongs_to(command: &str, executable: &Path) -> bool {
        command == startup_command(executable)
            || legacy_startup_executables(executable)
                .iter()
                .any(|previous| command == startup_command(previous))
    }

    /// Rename this installation's `EavesdropperNext` Run value to `SonoJot`,
    /// carrying Task Manager's enabled/disabled choice so a disabled entry
    /// stays disabled. The new value is written before the old one is removed.
    fn hand_over_run_value(
        run: &RegKey,
        approved: Option<&RegKey>,
        executable: &Path,
    ) -> Result<bool, String> {
        let Ok(command) = run.get_value::<String, _>(PREVIOUS_RUN_VALUE) else {
            return Ok(false);
        };
        if !run_command_belongs_to(&command, executable) {
            return Ok(false);
        }
        if let Some(approved) = approved {
            if let Ok(state) = approved.get_raw_value(PREVIOUS_RUN_VALUE) {
                approved
                    .set_raw_value(RUN_VALUE, &state)
                    .map_err(|e| format!("Cannot carry the launch-at-login state: {e}"))?;
                let _ = approved.delete_value(PREVIOUS_RUN_VALUE);
            }
        }
        run.set_value(RUN_VALUE, &startup_command(executable))
            .map_err(|e| format!("Cannot register launch at login: {e}"))?;
        match run.delete_value(PREVIOUS_RUN_VALUE) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(format!("Cannot remove the previous launch-at-login value: {e}")),
        }
    }

    pub(super) fn set_startup_enabled(enabled: bool, executable: &Path) -> Result<(), String> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root
            .create_subkey(RUN_KEY)
            .map_err(|e| format!("Cannot open launch-at-login registration: {e}"))?;
        if enabled {
            key.set_value(RUN_VALUE, &startup_command(executable))
                .map_err(|e| format!("Cannot register launch at login: {e}"))
        } else {
            // A leftover pre-SonoJot value for this installation would still launch it.
            if key
                .get_value::<String, _>(PREVIOUS_RUN_VALUE)
                .is_ok_and(|command| run_command_belongs_to(&command, executable))
            {
                let _ = key.delete_value(PREVIOUS_RUN_VALUE);
            }
            match key.delete_value(RUN_VALUE) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("Cannot remove launch-at-login registration: {e}")),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A scratch HKCU key stands in for Run and StartupApproved\Run; the
        /// real login registration is never read or written by these tests.
        struct ScratchKeys {
            name: String,
            run: RegKey,
            approved: RegKey,
        }

        impl ScratchKeys {
            fn new() -> Self {
                let name = format!(r"Software\SonoJotTest-{}", uuid::Uuid::new_v4().simple());
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let (run, _) = root.create_subkey(format!(r"{name}\Run")).unwrap();
                let (approved, _) = root.create_subkey(format!(r"{name}\Approved")).unwrap();
                Self { name, run, approved }
            }
        }

        impl Drop for ScratchKeys {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.name);
            }
        }

        #[test]
        fn previous_run_value_hands_over_to_sonojot_with_its_task_manager_state() {
            let keys = ScratchKeys::new();
            let executable = Path::new(r"C:\Users\a\AppData\Local\Programs\Eavesdropper Next\SonoJot.exe");
            keys.run
                .set_value(PREVIOUS_RUN_VALUE, &startup_command(executable))
                .unwrap();
            // Task Manager's "disabled" record: 0x03 followed by a FILETIME.
            let disabled = winreg::RegValue {
                bytes: vec![3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
                vtype: REG_BINARY,
            };
            keys.approved.set_raw_value(PREVIOUS_RUN_VALUE, &disabled).unwrap();

            assert!(hand_over_run_value(&keys.run, Some(&keys.approved), executable).unwrap());
            assert_eq!(
                keys.run.get_value::<String, _>(RUN_VALUE).unwrap(),
                startup_command(executable)
            );
            assert!(keys.run.get_value::<String, _>(PREVIOUS_RUN_VALUE).is_err());
            assert_eq!(keys.approved.get_raw_value(RUN_VALUE).unwrap(), disabled);
            assert!(keys.approved.get_raw_value(PREVIOUS_RUN_VALUE).is_err());
            // Nothing left to hand over the second time.
            assert!(!hand_over_run_value(&keys.run, Some(&keys.approved), executable).unwrap());
        }

        #[test]
        fn previous_run_value_for_the_old_binary_name_points_at_sonojot() {
            let keys = ScratchKeys::new();
            let executable = Path::new(r"C:\Programs\Eavesdropper Next\SonoJot.exe");
            keys.run
                .set_value(
                    PREVIOUS_RUN_VALUE,
                    &startup_command(&executable.with_file_name("eavesdropper-next.exe")),
                )
                .unwrap();
            assert!(hand_over_run_value(&keys.run, None, executable).unwrap());
            assert_eq!(
                keys.run.get_value::<String, _>(RUN_VALUE).unwrap(),
                startup_command(executable)
            );
            assert!(keys.run.get_value::<String, _>(PREVIOUS_RUN_VALUE).is_err());
        }

        #[test]
        fn another_installations_run_value_is_left_alone() {
            let keys = ScratchKeys::new();
            let executable = Path::new(r"C:\Programs\SonoJot\SonoJot.exe");
            let other = startup_command(Path::new(r"D:\Elsewhere\SonoJot.exe"));
            keys.run.set_value(PREVIOUS_RUN_VALUE, &other).unwrap();
            assert!(!hand_over_run_value(&keys.run, Some(&keys.approved), executable).unwrap());
            assert_eq!(keys.run.get_value::<String, _>(PREVIOUS_RUN_VALUE).unwrap(), other);
            assert!(keys.run.get_value::<String, _>(RUN_VALUE).is_err());
            // An absent value is not an error.
            let empty = ScratchKeys::new();
            assert!(!hand_over_run_value(&empty.run, None, executable).unwrap());
        }

        fn captured_fixture() -> PasteTarget {
            PasteTarget {
                foreground: 100,
                focused: 101,
                process_id: std::process::id().wrapping_add(1),
                thread_id: 200,
                focused_process_id: std::process::id().wrapping_add(1),
                focused_thread_id: 200,
            }
        }

        fn foreground_fixture(target: &PasteTarget) -> Foreground {
            Foreground {
                window: target.foreground,
                focused: target.focused,
                process_id: target.process_id,
                thread_id: target.thread_id,
                focus_query_ok: true,
            }
        }

        #[test]
        fn only_our_foreground_can_request_target_restoration() {
            let target = captured_fixture();
            let current = foreground_fixture(&target);
            assert_eq!(classify_focus(&target, current, true), FocusState::Ready);
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        window: 300,
                        process_id: std::process::id(),
                        ..current
                    },
                    true
                ),
                FocusState::OwnApp
            );
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        window: 300,
                        process_id: target.process_id.wrapping_add(1),
                        ..current
                    },
                    true
                ),
                FocusState::OtherApp
            );
            // Another window in the same external process is still a deliberate
            // destination change, not permission to return to the saved window.
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        window: 300,
                        ..current
                    },
                    true
                ),
                FocusState::OtherApp
            );
        }

        #[test]
        fn changed_control_differs_from_transient_missing_focus() {
            let target = captured_fixture();
            let current = foreground_fixture(&target);
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        focused: 102,
                        ..current
                    },
                    true
                ),
                FocusState::TargetFocusChanged
            );
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        focused: 0,
                        ..current
                    },
                    true
                ),
                FocusState::TargetFocusPending
            );
            assert_eq!(
                classify_focus(
                    &target,
                    Foreground {
                        focus_query_ok: false,
                        ..current
                    },
                    true
                ),
                FocusState::TargetFocusPending
            );
            assert_eq!(
                classify_focus(&target, Foreground::default(), true),
                FocusState::NoForeground
            );
        }

        #[test]
        fn invalid_target_never_becomes_restorable() {
            let target = captured_fixture();
            let own = Foreground {
                window: 300,
                process_id: std::process::id(),
                ..foreground_fixture(&target)
            };
            assert_eq!(
                classify_focus(&target, own, false),
                FocusState::TargetInvalid
            );
            assert_eq!(
                classify_focus(&target, foreground_fixture(&target), false),
                FocusState::TargetInvalid
            );
        }

        #[test]
        fn login_command_quotes_the_whole_executable() {
            assert_eq!(
                startup_command(Path::new(r"C:\Program Files\SonoJot\sonojot.exe")),
                r#""C:\Program Files\SonoJot\sonojot.exe" --startup"#
            );
            assert_eq!(
                startup_command(Path::new(r"\\?\C:\Users\Test User\sonojot.exe")),
                r#""C:\Users\Test User\sonojot.exe" --startup"#
            );
        }
    }
}

/// Pure macOS focus classification; compiled everywhere so it is unit-tested on Windows too.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn classify_app_focus(target_pid: i32, target_running: bool, frontmost: Option<i32>, own_pid: i32) -> FocusState {
    if !target_running {
        return FocusState::TargetInvalid;
    }
    match frontmost {
        None => FocusState::NoForeground,
        Some(pid) if pid == own_pid => FocusState::OwnApp,
        Some(pid) if pid != target_pid => FocusState::OtherApp,
        Some(_) => FocusState::Ready,
    }
}

#[cfg(not(windows))]
mod native {
    use super::*;
    #[cfg(not(target_os = "macos"))]
    use std::io::Write;
    #[cfg(not(target_os = "macos"))]
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    use crate::mac_paste as mac;

    #[cfg(target_os = "macos")]
    pub(super) fn capture_target() -> Option<PasteTarget> {
        mac::frontmost_pid()
            .filter(|pid| !mac::is_own_pid(*pid))
            .map(|pid| PasteTarget { pid })
    }
    #[cfg(target_os = "macos")]
    pub(super) fn focus_state(target: &PasteTarget) -> FocusState {
        classify_app_focus(
            target.pid,
            mac::app_running(target.pid),
            mac::frontmost_pid(),
            std::process::id() as i32,
        )
    }
    /// Activation moves only from SonoJot back to the captured app, never
    /// away from a third app the user switched to.
    #[cfg(target_os = "macos")]
    pub(super) fn restore_from_own_app(target: &PasteTarget) -> bool {
        mac::app_running(target.pid)
            && mac::frontmost_pid().is_some_and(mac::is_own_pid)
            && mac::activate(target.pid)
    }
    #[cfg(target_os = "macos")]
    pub(super) fn focus_diagnostics(target: Option<&PasteTarget>) -> String {
        let frontmost = mac::frontmost_pid();
        let status = match frontmost {
            None => "no_foreground",
            Some(pid) if mac::is_own_pid(pid) => "own_app",
            Some(_) => "external_target",
        };
        let target_info = target
            .map(|target| format!("target_pid={} target_running={}", target.pid, mac::app_running(target.pid)))
            .unwrap_or_else(|| "target=none".into());
        format!(
            "{target_info} observed_pid={} capture_status={status} accessibility={} modifiers_held={}",
            frontmost.unwrap_or(0),
            mac::accessibility_trusted(),
            mac::modifiers_pressed()
        )
    }
    #[cfg(target_os = "macos")]
    pub(super) fn clipboard_matches(sequence: u32) -> bool {
        mac::pasteboard_change_count() == Some(sequence)
    }
    #[cfg(target_os = "macos")]
    pub(super) fn modifiers_pressed() -> bool {
        mac::modifiers_pressed()
    }
    #[cfg(target_os = "macos")]
    pub(super) fn paste_if_current(target: &PasteTarget, sequence: u32, text: &str) -> DeliveryOutcome {
        // CGEventPost targets whichever app is frontmost when the events are
        // processed; there is no atomic check-and-paste. Keep the checks adjacent.
        let focus = focus_state(target);
        if focus != FocusState::Ready {
            return focus.fallback();
        }
        if !clipboard_matches(sequence) {
            return DeliveryOutcome::copied(
                "clipboard_changed",
                "The clipboard changed before automatic paste; no keys were sent.",
            );
        }
        if !mac::pasteboard_has_text(text) {
            return DeliveryOutcome::copied(
                "clipboard_payload_changed",
                "The clipboard text changed before automatic paste; no keys were sent.",
            );
        }
        if mac::modifiers_pressed() {
            return FocusState::Ready.fallback();
        }
        if !mac::accessibility_trusted() {
            return DeliveryOutcome::copied("permission_missing", MAC_PERMISSION_REASON);
        }
        if focus_state(target) != FocusState::Ready {
            return focus_state(target).fallback();
        }
        if mac::post_paste() {
            DeliveryOutcome {
                paste_sent: true,
                reason: None,
                reason_code: "paste_sent",
                diagnostics: String::new(),
            }
        } else {
            DeliveryOutcome::copied(
                "input_rejected",
                "macOS could not create the paste keystrokes. Text remains on the clipboard.",
            )
        }
    }
    #[cfg(target_os = "macos")]
    pub(super) fn set_clipboard(text: &str) -> Result<u32, String> {
        mac::set_pasteboard(text)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn capture_target() -> Option<PasteTarget> {
        None
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn focus_state(_: &PasteTarget) -> FocusState {
        FocusState::TargetInvalid
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn restore_from_own_app(_: &PasteTarget) -> bool {
        false
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn focus_diagnostics(_: Option<&PasteTarget>) -> String {
        format!(
            "platform={} automatic_paste=unsupported",
            std::env::consts::OS
        )
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn clipboard_matches(_: u32) -> bool {
        false
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn modifiers_pressed() -> bool {
        false
    }
    #[cfg(not(target_os = "macos"))]
    pub(super) fn paste_if_current(_: &PasteTarget, _: u32, _: &str) -> DeliveryOutcome {
        DeliveryOutcome::copied(
            "unsupported",
            "Automatic paste is unavailable on this platform.",
        )
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn set_clipboard(text: &str) -> Result<u32, String> {
        let candidates = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            vec![("wl-copy", vec!["--type", "text/plain;charset=utf-8"])]
        } else if std::env::var_os("DISPLAY").is_some() {
            vec![
                ("xclip", vec!["-selection", "clipboard", "-in"]),
                ("xsel", vec!["--clipboard", "--input"]),
            ]
        } else {
            return Err("No desktop clipboard session is available.".into());
        };
        for (program, args) in candidates {
            let mut child = match Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(format!("Cannot open native clipboard utility: {e}")),
            };
            let write_result = child
                .stdin
                .take()
                .ok_or("Clipboard input is unavailable.")?
                .write_all(text.as_bytes());
            if let Err(e) = write_result {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Cannot write clipboard text: {e}"));
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                match child
                    .try_wait()
                    .map_err(|e| format!("Cannot check clipboard utility: {e}"))?
                {
                    Some(status) if status.success() => return Ok(0),
                    Some(_) => return Err("The native clipboard utility rejected the text.".into()),
                    None if std::time::Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("The native clipboard utility timed out.".into());
                    }
                    None => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
        Err("Clipboard support needs wl-copy on Wayland, or xclip/xsel on X11.".into())
    }

    #[cfg(target_os = "macos")]
    const LABEL: &str = "studio.codystevens.sonojot";
    // Builds up to 0.28.3 used this launchd label and file name.
    #[cfg(target_os = "macos")]
    const PREVIOUS_LABEL: &str = "studio.codystevens.eavesdropper-next";
    #[cfg(not(target_os = "macos"))]
    const LABEL: &str = "sonojot";
    // Builds up to 0.28.3 used this autostart file name.
    #[cfg(not(target_os = "macos"))]
    const PREVIOUS_LABEL: &str = "eavesdropper-next";

    fn registration_path_for(label: &str) -> Result<PathBuf, String> {
        let user_dir = std::env::var_os("HOME").ok_or("The user home directory is unavailable.")?;
        #[cfg(target_os = "macos")]
        {
            Ok(PathBuf::from(user_dir).join(format!("Library/LaunchAgents/{label}.plist")))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let config_dir = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|v| v.is_absolute())
                .unwrap_or_else(|| PathBuf::from(user_dir).join(".config"));
            Ok(config_dir.join(format!("autostart/{label}.desktop")))
        }
    }

    fn registration_path() -> Result<PathBuf, String> {
        registration_path_for(LABEL)
    }

    fn registration_contents(executable: &Path) -> String {
        registration_contents_for(executable, true, LABEL)
    }

    fn registration_contents_for(executable: &Path, branded: bool, label: &str) -> String {
        let path = executable.to_string_lossy();
        #[cfg(target_os = "macos")]
        {
            let escaped = path
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;");
            // Associate the job with the SonoJot bundle in System Settings.
            // launchd.plist(5) defines this display metadata.
            let association = if branded {
                "<key>AssociatedBundleIdentifiers</key><string>studio.codystevens.eavesdropper.next</string>"
            } else {
                ""
            };
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{label}</string>{association}<key>ProgramArguments</key><array><string>{escaped}</string><string>--startup</string></array><key>RunAtLoad</key><true/></dict></plist>\n")
        }
        #[cfg(not(target_os = "macos"))]
        {
            // The file name carries the label; the contents do not.
            let _ = label;
            // Desktop Entry Exec uses its own escaping, not shell interpolation.
            let escaped = path
                .replace('\\', "\\\\\\\\")
                .replace('`', "\\\\`")
                .replace('$', "\\\\$")
                .replace('%', "%%");
            let name = if branded { "SonoJot" } else { "Eavesdropper" };
            let arguments = if branded { " --startup" } else { "" };
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=\"{escaped}\"{arguments}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n")
        }
    }

    /// Contents an earlier build wrote for this installation under `label`.
    fn earlier_registration(actual: &str, executable: &Path, label: &str) -> bool {
        legacy_startup_executables(executable)
            .iter()
            .any(|previous| {
                let legacy = registration_contents_for(previous, false, label);
                // Earlier macOS builds opened settings at login and omitted --startup.
                #[cfg(target_os = "macos")]
                if actual == legacy.replace("<string>--startup</string>", "") {
                    return true;
                }
                actual == legacy
                    || actual == registration_contents_for(previous, true, label)
                    || actual == registration_contents_for(executable, false, label)
            })
    }

    fn registration_needs_migration(actual: &str, executable: &Path) -> bool {
        actual != registration_contents(executable)
            && earlier_registration(actual, executable, LABEL)
    }

    /// A registration written under the pre-SonoJot label for this installation.
    fn previous_registration(actual: &str, executable: &Path) -> bool {
        actual == registration_contents_for(executable, true, PREVIOUS_LABEL)
            || earlier_registration(actual, executable, PREVIOUS_LABEL)
    }

    pub(super) fn migrate_startup_registration(executable: &Path) -> Result<(), String> {
        let path = registration_path()?;
        hand_over_registration_file(
            &registration_path_for(PREVIOUS_LABEL)?,
            &path,
            |actual| previous_registration(actual, executable),
            &registration_contents(executable),
        )?;
        match std::fs::read_to_string(&path) {
            Ok(actual) if registration_needs_migration(&actual, executable) => {
                set_startup_enabled(true, executable)
            }
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Cannot read login registration: {e}")),
        }
    }

    pub(super) fn startup_enabled(executable: &Path) -> Result<bool, String> {
        match std::fs::read_to_string(registration_path()?) {
            Ok(actual) => Ok(actual == registration_contents(executable)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("Cannot read login registration: {e}")),
        }
    }

    pub(super) fn set_startup_enabled(enabled: bool, executable: &Path) -> Result<(), String> {
        let path = registration_path()?;
        if enabled {
            write_registration_file(&path, &registration_contents(executable))
        } else {
            // A leftover pre-SonoJot file for this installation would still launch it.
            let previous = registration_path_for(PREVIOUS_LABEL)?;
            if std::fs::read_to_string(&previous)
                .is_ok_and(|actual| previous_registration(&actual, executable))
            {
                let _ = std::fs::remove_file(&previous);
            }
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("Cannot remove login registration: {e}")),
            }
        }
    }

    #[cfg(all(test, target_os = "macos"))]
    #[test]
    fn login_agent_starts_in_the_menu_bar() {
        let xml = registration_contents(Path::new(
            "/Applications/SonoJot.app/Contents/MacOS/SonoJot",
        ));
        assert!(xml.contains(
            "<string>/Applications/SonoJot.app/Contents/MacOS/SonoJot</string><string>--startup</string>"
        ));
        assert!(xml.contains("<key>Label</key><string>studio.codystevens.sonojot</string>"));
        assert!(xml.contains("<key>AssociatedBundleIdentifiers</key><string>studio.codystevens.eavesdropper.next</string>"));
        assert!(registration_path()
            .unwrap()
            .ends_with("Library/LaunchAgents/studio.codystevens.sonojot.plist"));
    }

    #[cfg(test)]
    #[test]
    fn login_migration_matches_only_the_same_installation() {
        let executable = Path::new("/Applications/SonoJot.app/Contents/MacOS/SonoJot");
        let previous = executable.with_file_name("eavesdropper-next");
        let old = registration_contents_for(&previous, false, LABEL);
        assert!(registration_needs_migration(&old, executable));
        let interim = registration_contents(&executable.with_file_name("sonojot"));
        assert!(registration_needs_migration(&interim, executable));
        #[cfg(target_os = "macos")]
        assert!(registration_needs_migration(
            &old.replace("<string>--startup</string>", ""),
            executable
        ));
        assert!(!registration_needs_migration("", executable));
        assert!(!registration_needs_migration(
            &registration_contents(executable),
            executable
        ));
        let other = Path::new("/other/SonoJot.app/Contents/MacOS/SonoJot");
        assert!(!registration_needs_migration(&old, other));
        assert!(!registration_needs_migration(&interim, other));
        let disabled = old
            .replace(
                "<key>RunAtLoad</key><true/>",
                "<key>RunAtLoad</key><false/>",
            )
            .replace(
                "X-GNOME-Autostart-enabled=true",
                "X-GNOME-Autostart-enabled=false",
            );
        assert!(!registration_needs_migration(&disabled, executable));
        assert!(!registration_needs_migration(
            &old,
            Path::new("/Applications/another-app")
        ));
    }

    #[cfg(test)]
    #[test]
    fn previous_label_hands_over_to_the_sonojot_label() {
        let executable = Path::new("/Applications/SonoJot.app/Contents/MacOS/SonoJot");
        // The registration from 0.20.7 to 0.28.3, and the original eavesdropper-next one.
        let branded = registration_contents_for(executable, true, PREVIOUS_LABEL);
        let original = registration_contents_for(
            &executable.with_file_name("eavesdropper-next"),
            false,
            PREVIOUS_LABEL,
        );
        assert!(previous_registration(&branded, executable));
        assert!(previous_registration(&original, executable));
        assert!(!previous_registration(
            &branded,
            Path::new("/other/SonoJot.app/Contents/MacOS/SonoJot")
        ));
        let disabled = branded
            .replace("<key>RunAtLoad</key><true/>", "<key>RunAtLoad</key><false/>")
            .replace("X-GNOME-Autostart-enabled=true", "X-GNOME-Autostart-enabled=false");
        assert!(!previous_registration(&disabled, executable));

        let directory = std::env::temp_dir()
            .join(format!("sonojot-login-{}", uuid::Uuid::new_v4().simple()));
        let old_path = directory.join("old.plist");
        let new_path = directory.join("new/new.plist");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&old_path, &branded).unwrap();
        let current = registration_contents(executable);
        assert!(hand_over_registration_file(
            &old_path,
            &new_path,
            |actual| previous_registration(actual, executable),
            &current
        )
        .unwrap());
        assert!(!old_path.exists());
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), current);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_focus_returns_only_from_our_app_to_the_captured_app() {
        let own = 100;
        assert_eq!(classify_app_focus(200, true, Some(200), own), FocusState::Ready);
        assert_eq!(classify_app_focus(200, true, Some(own), own), FocusState::OwnApp);
        // A deliberate switch to a third app ends delivery without keys.
        assert_eq!(classify_app_focus(200, true, Some(300), own), FocusState::OtherApp);
        assert_eq!(classify_app_focus(200, true, None, own), FocusState::NoForeground);
        // A quit target is never activated, even while SonoJot is frontmost.
        assert_eq!(classify_app_focus(200, false, Some(own), own), FocusState::TargetInvalid);
        assert_eq!(classify_app_focus(200, false, Some(200), own), FocusState::TargetInvalid);
    }

    #[test]
    fn auto_paste_is_offered_on_windows_and_macos() {
        assert_eq!(
            auto_paste_available(),
            cfg!(windows) || cfg!(target_os = "macos")
        );
        if cfg!(windows) {
            assert!(paste_permission_granted());
            assert_eq!(auto_paste_unavailable_reason(), None);
        }
    }

    #[test]
    fn captured_target_can_cross_threads() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<PasteTarget>();
    }

    #[test]
    fn registration_file_hands_over_only_when_it_belongs_to_this_installation() {
        let directory = std::env::temp_dir()
            .join(format!("sonojot-login-file-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&directory).unwrap();
        let previous = directory.join("studio.codystevens.eavesdropper-next.plist");
        let current = directory.join("agents/studio.codystevens.sonojot.plist");

        // Absent: nothing to do.
        assert!(!hand_over_registration_file(&previous, &current, |_| true, "new").unwrap());
        assert!(!current.exists());

        // Unrelated or disabled: both files stay as they are.
        std::fs::write(&previous, "other").unwrap();
        assert!(!hand_over_registration_file(&previous, &current, |a| a == "ours", "new").unwrap());
        assert_eq!(std::fs::read_to_string(&previous).unwrap(), "other");
        assert!(!current.exists());

        // Ours: the new file is written and the old one removed.
        std::fs::write(&previous, "ours").unwrap();
        assert!(hand_over_registration_file(&previous, &current, |a| a == "ours", "new").unwrap());
        assert!(!previous.exists());
        assert_eq!(std::fs::read_to_string(&current).unwrap(), "new");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_rejects_relative_and_missing_executables() {
        assert!(validated_startup_executable(Path::new("app.exe")).is_err());
        assert!(validated_startup_executable(
            &std::env::temp_dir().join("sonojot-does-not-exist.exe")
        )
        .is_err());
    }

    #[test]
    fn nul_text_is_rejected_before_any_clipboard_write() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        assert!(runtime
            .block_on(copy_text("left\0right"))
            .unwrap_err()
            .contains("NUL"));
    }
}
