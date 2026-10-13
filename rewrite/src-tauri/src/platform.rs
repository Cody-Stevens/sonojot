//! Native text delivery and login registration. Capture the paste target before
//! showing app UI. Delivery may return from this app to that captured target, but
//! never displaces another foreground application or restores old clipboard data.

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
#[cfg_attr(not(windows), allow(dead_code))] // Focus recovery is Windows-only.
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
            Self::OwnApp => DeliveryOutcome::copied("restore_not_completed", "Windows did not return focus from SonoJot to the original text target. Text is ready to paste."),
            Self::NoForeground | Self::TargetFocusPending => DeliveryOutcome::copied("focus_not_ready", "The original text target did not regain keyboard focus. Text is ready to paste."),
            Self::Ready => DeliveryOutcome::copied("modifiers_held", "A shortcut modifier is still held. Text is ready to paste."),
        }
    }
}

pub fn capture_paste_target() -> Option<PasteTarget> {
    native::capture_target()
}

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
    cfg!(windows)
}

/// Explains platform/permission limitations without claiming untested support.
pub fn auto_paste_unavailable_reason() -> Option<String> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(target_os = "macos")]
    {
        let permission = if native::accessibility_trusted() {
            "granted"
        } else {
            "not granted"
        };
        Some(format!("Text is copied to the macOS pasteboard. Automatic paste is not implemented; Accessibility permission is {permission}."))
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
        let Some(target) = target.as_ref() else {
            return DeliveryOutcome::copied(
                "target_missing",
                "No external text target was captured. Text is ready to paste.",
            );
        };
        // Finishing from the overlay may focus this app. It is safe to hand focus
        // back only while this process owns foreground and the exact target survives.
        // A deliberate switch to any other app always ends delivery without input.
        let deadline = Instant::now() + Duration::from_millis(450);
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
            if focus == FocusState::Ready && !native::modifiers_pressed() {
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
    // A separate name deliberately leaves the old C# installation untouched.
    const RUN_VALUE: &str = "EavesdropperNext";

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
        for previous in legacy_startup_executables(executable) {
            if startup_enabled(&previous)? {
                set_startup_enabled(true, executable)?;
                break;
            }
        }
        Ok(())
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

#[cfg(not(windows))]
mod native {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub(super) fn capture_target() -> Option<PasteTarget> {
        None
    }
    pub(super) fn focus_state(_: &PasteTarget) -> FocusState {
        FocusState::TargetInvalid
    }
    pub(super) fn restore_from_own_app(_: &PasteTarget) -> bool {
        false
    }
    pub(super) fn focus_diagnostics(_: Option<&PasteTarget>) -> String {
        format!(
            "platform={} automatic_paste=unsupported",
            std::env::consts::OS
        )
    }
    pub(super) fn clipboard_matches(_: u32) -> bool {
        false
    }
    pub(super) fn modifiers_pressed() -> bool {
        false
    }
    pub(super) fn paste_if_current(_: &PasteTarget, _: u32, _: &str) -> DeliveryOutcome {
        DeliveryOutcome::copied(
            "unsupported",
            "Automatic paste is unavailable on this platform.",
        )
    }

    pub(super) fn set_clipboard(text: &str) -> Result<u32, String> {
        #[cfg(target_os = "macos")]
        let candidates: Vec<(&str, Vec<&str>)> = vec![("/usr/bin/pbcopy", vec![])];
        #[cfg(not(target_os = "macos"))]
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
    pub(super) fn accessibility_trusted() -> bool {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        unsafe { AXIsProcessTrusted() }
    }

    fn registration_path() -> Result<PathBuf, String> {
        let user_dir = std::env::var_os("HOME").ok_or("The user home directory is unavailable.")?;
        #[cfg(target_os = "macos")]
        {
            Ok(PathBuf::from(user_dir)
                .join("Library/LaunchAgents/studio.codystevens.eavesdropper-next.plist"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let config_dir = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|v| v.is_absolute())
                .unwrap_or_else(|| PathBuf::from(user_dir).join(".config"));
            Ok(config_dir.join("autostart/eavesdropper-next.desktop"))
        }
    }

    fn registration_contents(executable: &Path) -> String {
        registration_contents_with_brand(executable, true)
    }

    fn registration_contents_with_brand(executable: &Path, branded: bool) -> String {
        let path = executable.to_string_lossy();
        #[cfg(target_os = "macos")]
        {
            let escaped = path
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;");
            // Associate the stable legacy job label with the SonoJot bundle in
            // System Settings. launchd.plist(5) defines this display metadata.
            let association = if branded {
                "<key>AssociatedBundleIdentifiers</key><string>studio.codystevens.eavesdropper.next</string>"
            } else {
                ""
            };
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>studio.codystevens.eavesdropper-next</string>{association}<key>ProgramArguments</key><array><string>{escaped}</string><string>--startup</string></array><key>RunAtLoad</key><true/></dict></plist>\n")
        }
        #[cfg(not(target_os = "macos"))]
        {
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

    fn registration_needs_migration(actual: &str, executable: &Path) -> bool {
        if actual == registration_contents(executable) {
            return false;
        }
        legacy_startup_executables(executable)
            .iter()
            .any(|previous| {
                let legacy = registration_contents_with_brand(previous, false);
                // Earlier macOS builds opened settings at login and omitted --startup.
                #[cfg(target_os = "macos")]
                if actual == legacy.replace("<string>--startup</string>", "") {
                    return true;
                }
                actual == legacy
                    || actual == registration_contents(previous)
                    || actual == registration_contents_with_brand(executable, false)
            })
    }

    pub(super) fn migrate_startup_registration(executable: &Path) -> Result<(), String> {
        match std::fs::read_to_string(registration_path()?) {
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
            let parent = path
                .parent()
                .ok_or("Invalid login registration directory.")?;
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create login registration directory: {e}"))?;
            let temporary = path.with_extension("next-tmp");
            std::fs::write(&temporary, registration_contents(executable))
                .map_err(|e| format!("Cannot write login registration: {e}"))?;
            std::fs::rename(&temporary, &path)
                .map_err(|e| format!("Cannot install login registration: {e}"))
        } else {
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
        assert!(xml.contains("<key>AssociatedBundleIdentifiers</key><string>studio.codystevens.eavesdropper.next</string>"));
    }

    #[cfg(test)]
    #[test]
    fn login_migration_matches_only_the_same_installation() {
        let executable = Path::new("/Applications/SonoJot.app/Contents/MacOS/SonoJot");
        let previous = executable.with_file_name("eavesdropper-next");
        let old = registration_contents_with_brand(&previous, false);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_target_can_cross_threads() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<PasteTarget>();
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
