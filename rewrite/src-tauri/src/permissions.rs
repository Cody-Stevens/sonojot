//! macOS permission assistant: open the right System Settings pane and float a
//! paper helper beside it, carrying the SonoJot icon as a real file drag of
//! the running app bundle (for when SonoJot is not listed yet).
//!
//! Each permission is a [`PermissionSpec`]: its copy, the System Settings pane
//! URL, a `granted` check that is polled while the helper is open, a
//! `request` made each time the user asks (macOS lists the app or shows its
//! own prompt) and whether its list accepts the dragged app. Add a spec to
//! [`SPECS`] to reuse the assistant for another permission; the commands, the
//! setup Permissions step and the Svelte helper are driven by the spec.
//!
//! Once `granted` turns true, the helper closes, a `permission-status` event
//! reports it and SonoJot comes back to the front. The helper never blocks
//! setup; it closes with its own button, when System Settings closes, or
//! after 15 minutes. Other platforms need no permission and report granted.
//!
//! macOS ties an entry to the app's code signature, and apps cannot read
//! whether it is switched on. An entry left by an older, differently signed
//! build looks switched on but grants nothing, and System Settings may not
//! let the user remove it. [`permission_reset`] clears SonoJot's own entry
//! with `tccutil`, asks macOS again and reopens the pane.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub const STATUS_EVENT: &str = "permission-status";
pub const HELPER_LABEL: &str = "permission-helper";

/// One operating-system permission the assistant can help with.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // The pane and request are macOS-only.
pub struct PermissionSpec {
    /// Stable id used by commands, events and `?kind=` in the helper URL.
    pub kind: &'static str,
    /// Helper heading.
    pub title: &'static str,
    /// Helper explanation shown next to the icon.
    pub body: &'static str,
    /// Shown with the reset action after a few seconds without access. A
    /// switched-on entry left by an older build (for example an ad-hoc signed
    /// one) grants nothing until it is reset.
    pub stale_hint: &'static str,
    /// `x-apple.systempreferences:` URL of the pane.
    pub settings_url: &'static str,
    /// Polled while the helper is open.
    pub granted: fn() -> bool,
    /// Called each time the assistant opens. True when no Settings visit is
    /// needed now: access is granted or macOS is showing its own prompt.
    pub request: Option<fn() -> bool>,
    /// Clears SonoJot's own entry so macOS asks again.
    pub reset: Option<PermissionReset>,
    /// The pane's list takes an app dragged in (or added with +). The
    /// Microphone list does not: macOS adds apps only when they ask.
    pub draggable: bool,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // Reset runs only on macOS.
pub struct PermissionReset {
    /// `tccutil` service name (the TCC service without `kTCCService`).
    pub tcc_service: &'static str,
    /// Asks macOS again after the reset. True when no Settings visit is
    /// needed now: access is granted or macOS is showing its own prompt.
    pub prompt: fn() -> bool,
}

/// Asks macOS every time the user presses Allow, not once per run: if the
/// user removed SonoJot's entry from the list, macOS lists it again (switched off)
/// and shows its alert, so the list never depends on the icon drag. After a
/// reset the old entry is gone, so the same call lists SonoJot again.
fn accessibility_prompt() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::mac_paste::prompt_accessibility()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

pub static SPECS: &[PermissionSpec] = &[
    PermissionSpec {
        kind: "accessibility",
        title: "Allow SonoJot to paste",
        body: "Switch SonoJot on in the list. If it isn't listed, click\u{a0}+ under the list and choose SonoJot in Applications, or drag this icon into the list.",
        stale_hint: "Already on but not working?",
        settings_url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        granted: crate::platform::paste_permission_granted,
        request: Some(accessibility_prompt),
        reset: Some(PermissionReset {
            tcc_service: "Accessibility",
            prompt: accessibility_prompt,
        }),
        draggable: true,
    },
    PermissionSpec {
        kind: "system_audio",
        title: "Allow SonoJot to record system audio",
        body: "Switch SonoJot on under System Audio Recording Only. If it isn't listed, click\u{a0}+ under that list and choose SonoJot in Applications, or drag this icon in.",
        stale_hint: "Already on but not recording?",
        settings_url: crate::system_audio::SETTINGS_URL,
        granted: crate::system_audio::permission_granted,
        // An unanswered permission shows macOS's own prompt instead of Settings.
        request: Some(crate::system_audio::request_permission_prompted),
        // kTCCServiceAudioCapture. After the reset the permission is
        // unanswered again, so the request shows macOS's prompt.
        reset: Some(PermissionReset {
            tcc_service: "AudioCapture",
            prompt: crate::system_audio::request_permission_prompted,
        }),
        draggable: true,
    },
    PermissionSpec {
        kind: "microphone",
        title: "Allow SonoJot to use the microphone",
        // The Microphone list has no + and takes no drops: macOS lists an app
        // once it has asked, which `request` does when it never has.
        body: "Switch SonoJot on in the Microphone list.",
        stale_hint: "Already on but not hearing you?",
        settings_url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
        granted: microphone::granted,
        // Never asked: macOS shows its own prompt instead of Settings.
        request: Some(microphone::request),
        // kTCCServiceMicrophone. After the reset macOS asks again.
        reset: Some(PermissionReset {
            tcc_service: "Microphone",
            prompt: microphone::request,
        }),
        draggable: false,
    },
];

/// Microphone access from AVFoundation's authorization status, the check
/// macOS itself applies when dictation or a meeting opens the microphone.
mod microphone {
    /// AVAuthorizationStatus.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Authorization {
        NotDetermined,
        Restricted,
        Denied,
        Authorized,
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn from_raw(value: isize) -> Authorization {
        match value {
            0 => Authorization::NotDetermined,
            1 => Authorization::Restricted,
            3 => Authorization::Authorized,
            _ => Authorization::Denied,
        }
    }

    /// True when no Settings visit is needed now: access is on, or macOS is
    /// about to show its own prompt.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn needs_no_settings(status: Authorization) -> bool {
        matches!(status, Authorization::Authorized | Authorization::NotDetermined)
    }

    #[cfg(target_os = "macos")]
    mod native {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject, Bool};

        #[link(name = "AVFoundation", kind = "framework")]
        unsafe extern "C" {
            static AVMediaTypeAudio: &'static AnyObject;
        }

        fn capture_device() -> Option<&'static AnyClass> {
            AnyClass::get(c"AVCaptureDevice")
        }

        /// Raw AVAuthorizationStatus (an NSInteger). Without AVFoundation it
        /// reports authorized, and opening the microphone asks instead.
        pub fn status() -> isize {
            let Some(class) = capture_device() else { return 3 };
            unsafe { msg_send![class, authorizationStatusForMediaType: AVMediaTypeAudio] }
        }

        /// Shows macOS's microphone prompt. The answer arrives later on
        /// another queue; setup and the helper poll the status.
        pub fn ask() {
            let Some(class) = capture_device() else { return };
            // AVFoundation copies the block.
            let done = block2::RcBlock::new(|_granted: Bool| {});
            unsafe {
                let _: () = msg_send![
                    class,
                    requestAccessForMediaType: AVMediaTypeAudio,
                    completionHandler: &*done
                ];
            }
        }
    }

    pub fn granted() -> bool {
        #[cfg(target_os = "macos")]
        {
            from_raw(native::status()) == Authorization::Authorized
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }

    pub fn request() -> bool {
        #[cfg(target_os = "macos")]
        {
            let status = from_raw(native::status());
            if status == Authorization::NotDetermined {
                native::ask();
            }
            needs_no_settings(status)
        }
        #[cfg(not(target_os = "macos"))]
        {
            true
        }
    }
}

pub fn spec(kind: &str) -> Result<&'static PermissionSpec, String> {
    SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        .ok_or_else(|| format!("Unknown permission: {kind}."))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub kind: &'static str,
    /// True where this permission exists (macOS).
    pub required: bool,
    pub granted: bool,
    /// The helper is open for this permission.
    pub assistant_open: bool,
    pub title: &'static str,
    pub body: &'static str,
    pub stale_hint: &'static str,
    /// [`permission_reset`] can clear this permission's entry.
    pub can_reset: bool,
    /// The helper offers the app icon to drag into the list.
    pub draggable: bool,
}

pub fn status(app: &AppHandle, spec: &'static PermissionSpec) -> PermissionStatus {
    let required = cfg!(target_os = "macos");
    #[cfg(target_os = "macos")]
    let assistant_open = native::open_kind(app) == Some(spec.kind);
    #[cfg(not(target_os = "macos"))]
    let assistant_open = {
        let _ = app;
        false
    };
    PermissionStatus {
        kind: spec.kind,
        required,
        granted: !required || (spec.granted)(),
        assistant_open,
        title: spec.title,
        body: spec.body,
        stale_hint: spec.stale_hint,
        can_reset: required && spec.reset.is_some(),
        draggable: spec.draggable,
    }
}

fn announce(app: &AppHandle, spec: &'static PermissionSpec) -> PermissionStatus {
    let current = status(app, spec);
    let _ = app.emit(STATUS_EVENT, current.clone());
    current
}

#[tauri::command]
pub fn permission_status(app: AppHandle, kind: String) -> Result<PermissionStatus, String> {
    Ok(status(&app, spec(&kind)?))
}

/// `open` starts the assistant (or reports that access is already granted);
/// `close` dismisses the helper. Neither waits for the user.
#[tauri::command]
pub async fn permission_assistant(
    app: AppHandle,
    kind: String,
    action: String,
) -> Result<PermissionStatus, String> {
    let spec = spec(&kind)?;
    match action.as_str() {
        "open" => {
            #[cfg(target_os = "macos")]
            native::open(&app, spec)?;
            Ok(announce(&app, spec))
        }
        "close" => {
            #[cfg(target_os = "macos")]
            native::close(&app);
            Ok(announce(&app, spec))
        }
        _ => Err("Unknown permission assistant action.".into()),
    }
}

/// Clears SonoJot's own entry for the permission, asks macOS again and
/// reopens the System Settings pane with the helper. For an entry left by an
/// older build: it looks switched on, grants nothing and may not be removable.
#[tauri::command]
pub async fn permission_reset(app: AppHandle, kind: String) -> Result<PermissionStatus, String> {
    let spec = spec(&kind)?;
    let reset = spec.reset.as_ref().ok_or("This permission cannot be reset.")?;
    #[cfg(target_os = "macos")]
    {
        native::reset(&app, spec, reset).await?;
        Ok(announce(&app, spec))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, reset);
        Err("This permission is needed only on macOS.".into())
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const TCCUTIL: &str = "/usr/bin/tccutil";

/// A reverse-DNS bundle identifier: dot-separated labels of ASCII letters,
/// digits and hyphens. It is passed to `tccutil` as one argument, never
/// through a shell; this also keeps it from looking like an option.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn valid_bundle_id(id: &str) -> bool {
    id.len() <= 255
        && id.contains('.')
        && id.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// `tccutil reset <service> <bundle id>`, as separate arguments.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn tccutil_reset_args(service: &str, bundle_id: &str) -> Result<[String; 3], String> {
    if service.is_empty() || !service.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(format!("Unknown permission service: {service}."));
    }
    if !valid_bundle_id(bundle_id) {
        return Err(format!("SonoJot's bundle identifier is not valid: {bundle_id}."));
    }
    Ok(["reset".into(), service.into(), bundle_id.into()])
}

/// Error for a failed `tccutil`, with its own message when it printed one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn reset_failure(stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        "macOS could not reset the permission.".into()
    } else {
        format!("macOS could not reset the permission: {detail}")
    }
}

/// Starts the native file drag of the app bundle from the helper's icon.
/// Synchronous commands run on the main thread, which AppKit drags require.
#[tauri::command]
pub fn permission_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != HELPER_LABEL {
        return Err("Drag the icon from the SonoJot helper.".into());
    }
    #[cfg(target_os = "macos")]
    {
        native::start_drag(&window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("This permission is needed only on macOS.".into())
    }
}

/// NSDragOperation offered by the helper's icon: copy | link | generic
/// outside the app, as a Finder drag offers them. Never move or delete, so
/// dropping the icon on the Trash or a folder cannot move the app. Nothing
/// inside the helper itself.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn drag_operations(context: isize) -> usize {
    const COPY: usize = 1;
    const LINK: usize = 2;
    const GENERIC: usize = 4;
    // NSDraggingContextOutsideApplication
    if context == 0 {
        COPY | LINK | GENERIC
    } else {
        0
    }
}

/// Pasteboard types a Finder drag of an app carries: the modern file URL and
/// the legacy filenames list (a property-list array of paths). Older AppKit
/// lists, such as the privacy panes, may register only for the legacy type.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FILE_URL_TYPE: &std::ffi::CStr = c"public.file-url";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FILENAMES_TYPE: &std::ffi::CStr = c"NSFilenamesPboardType";

/// Top-left rectangle in CoreGraphics points.
#[cfg(any(test, target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(any(test, target_os = "macos"))]
impl Frame {
    fn right(self) -> f64 {
        self.x + self.width
    }
    fn bottom(self) -> f64 {
        self.y + self.height
    }
    fn contains(self, other: Frame) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

/// Place the helper next to System Settings without covering its list:
/// below the window, else beside its lower edge (where the add button is),
/// else inside its bottom-right corner. Without a Settings window, it sits
/// near the top of the display. Always kept on the display.
#[cfg(any(test, target_os = "macos"))]
fn helper_frame(settings: Option<Frame>, display: Frame, width: f64, height: f64) -> Frame {
    const GAP: f64 = 12.0;
    const MARGIN: f64 = 8.0;
    let usable = Frame {
        x: display.x + MARGIN,
        y: display.y + MARGIN,
        width: (display.width - 2.0 * MARGIN).max(width),
        height: (display.height - 2.0 * MARGIN).max(height),
    };
    let sized = |x: f64, y: f64| Frame {
        x,
        y,
        width,
        height,
    };
    let candidates = match settings {
        Some(window) => vec![
            sized(window.x + (window.width - width) / 2.0, window.bottom() + GAP),
            sized(window.right() + GAP, window.bottom() - height),
            sized(window.x - GAP - width, window.bottom() - height),
            sized(window.right() - width - 16.0, window.bottom() - height - 16.0),
        ],
        None => vec![sized(
            display.x + (display.width - width) / 2.0,
            display.y + display.height * 0.18,
        )],
    };
    let chosen = candidates
        .iter()
        .copied()
        .find(|frame| usable.contains(*frame))
        .unwrap_or(*candidates.last().expect("at least one candidate"));
    sized(
        chosen.x.clamp(usable.x, usable.right() - width),
        chosen.y.clamp(usable.y, usable.bottom() - height),
    )
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use crate::mac_paste;
    use objc2::encode::{Encode, Encoding};
    use objc2::rc::{autoreleasepool, Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, MessageReceiver, Sel};
    use objc2::{class, msg_send, sel};
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tauri::{LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

    const WIDTH: f64 = 520.0;
    // Room for the reset offer under the instruction, and for the + and
    // drag fallback in the longest (system audio) instruction.
    const HEIGHT: f64 = 192.0;
    const SETTINGS_BUNDLE: &str = "com.apple.systempreferences";
    /// NSFloatingWindowLevel: above System Settings, below menus and alerts.
    const FLOATING_LEVEL: isize = 3;
    const POLL: Duration = Duration::from_millis(400);
    const LIFETIME: Duration = Duration::from_secs(15 * 60);

    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static OPEN_KIND: Mutex<Option<&'static str>> = Mutex::new(None);

    /// The permission the open helper serves.
    pub fn open_kind(app: &AppHandle) -> Option<&'static str> {
        app.get_webview_window(HELPER_LABEL)?;
        *OPEN_KIND.lock().ok()?
    }

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    unsafe impl Encode for CGPoint {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl Encode for CGSize {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl Encode for CGRect {
        const ENCODING: Encoding =
            Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: *const c_void, rect: *mut CGRect) -> bool;
        fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGMainDisplayID() -> u32;
        static kCGWindowOwnerPID: *const c_void;
        static kCGWindowLayer: *const c_void;
        static kCGWindowBounds: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, kind: isize, value: *mut c_void) -> bool;
        fn CFRelease(value: *const c_void);
    }

    fn frame(rect: CGRect) -> Frame {
        Frame {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        }
    }

    fn settings_pids() -> Vec<i32> {
        let Ok(bundle) = CString::new(SETTINGS_BUNDLE) else {
            return Vec::new();
        };
        autoreleasepool(|_| unsafe {
            let identifier: Option<Retained<AnyObject>> =
                msg_send![class!(NSString), stringWithUTF8String: bundle.as_ptr()];
            let Some(identifier) = identifier else {
                return Vec::new();
            };
            let running: Option<Retained<AnyObject>> = msg_send![
                class!(NSRunningApplication),
                runningApplicationsWithBundleIdentifier: &*identifier
            ];
            let Some(running) = running else {
                return Vec::new();
            };
            let count: usize = msg_send![&*running, count];
            (0..count)
                .filter_map(|index| {
                    let application: Option<Retained<AnyObject>> =
                        msg_send![&*running, objectAtIndex: index];
                    let pid: i32 = msg_send![&*application?, processIdentifier];
                    Some(pid)
                })
                .collect()
        })
    }

    fn cf_i32(dictionary: *const c_void, key: *const c_void) -> Option<i32> {
        let mut value = 0i32;
        unsafe {
            let number = CFDictionaryGetValue(dictionary, key);
            // kCFNumberSInt32Type
            (!number.is_null() && CFNumberGetValue(number, 3, (&raw mut value).cast()))
                .then_some(value)
        }
    }

    /// Frontmost on-screen System Settings window. Owner and bounds need no
    /// Screen Recording permission; window titles would, so none are read.
    fn settings_window() -> Option<Frame> {
        let pids = settings_pids();
        if pids.is_empty() {
            return None;
        }
        unsafe {
            // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements
            let list = CGWindowListCopyWindowInfo(1 | (1 << 4), 0);
            if list.is_null() {
                return None;
            }
            let found = (0..CFArrayGetCount(list)).find_map(|index| {
                let window = CFArrayGetValueAtIndex(list, index);
                let pid = cf_i32(window, kCGWindowOwnerPID)?;
                if !pids.contains(&pid) || cf_i32(window, kCGWindowLayer) != Some(0) {
                    return None;
                }
                let bounds = CFDictionaryGetValue(window, kCGWindowBounds);
                let mut rect = CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize {
                        width: 0.0,
                        height: 0.0,
                    },
                };
                (!bounds.is_null()
                    && CGRectMakeWithDictionaryRepresentation(bounds, &mut rect)
                    && rect.size.width > 200.0
                    && rect.size.height > 200.0)
                    .then(|| frame(rect))
            });
            CFRelease(list);
            found
        }
    }

    fn display_for(settings: Option<Frame>) -> Frame {
        unsafe {
            let mut id = CGMainDisplayID();
            if let Some(window) = settings {
                let center = CGPoint {
                    x: window.x + window.width / 2.0,
                    y: window.y + window.height / 2.0,
                };
                let mut found = 0u32;
                let mut count = 0u32;
                if CGGetDisplaysWithPoint(center, 1, &mut found, &mut count) == 0 && count > 0 {
                    id = found;
                }
            }
            frame(CGDisplayBounds(id))
        }
    }

    fn place(window: &tauri::WebviewWindow, settings: Option<Frame>) {
        let target = helper_frame(settings, display_for(settings), WIDTH, HEIGHT);
        let _ = window.set_position(LogicalPosition::new(
            target.x,
            crate::menu_panel::to_window_space_y(target.y),
        ));
    }

    /// Float above System Settings without activating SonoJot.
    fn float(window: &tauri::WebviewWindow) {
        let shown = window.clone();
        let _ = window.run_on_main_thread(move || {
            let Ok(pointer) = shown.ns_window() else {
                return;
            };
            let ns_window = pointer as *mut AnyObject;
            unsafe {
                let _: () = msg_send![ns_window, setLevel: FLOATING_LEVEL];
                let _: () = msg_send![ns_window, setHidesOnDeactivate: false];
                // canJoinAllSpaces | fullScreenAuxiliary
                let behavior: usize = 1 | (1 << 8);
                let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
                let _: () = msg_send![ns_window, orderFrontRegardless];
            }
        });
    }

    fn helper(
        app: &AppHandle,
        spec: &'static PermissionSpec,
    ) -> Result<tauri::WebviewWindow, String> {
        if let Some(window) = app.get_webview_window(HELPER_LABEL) {
            if OPEN_KIND.lock().ok().and_then(|kind| *kind) == Some(spec.kind) {
                return Ok(window);
            }
            let _ = window.destroy();
        }
        WebviewWindowBuilder::new(
            app,
            HELPER_LABEL,
            WebviewUrl::App(format!("index.html?view=permission&kind={}", spec.kind).into()),
        )
        .title(spec.title)
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .accept_first_mouse(true)
        .visible_on_all_workspaces(true)
        .build()
        .map_err(|e| e.to_string())
    }

    fn bring_sonojot_forward(app: &AppHandle) {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.unminimize();
            let _ = main.show();
            let _ = main.set_focus();
        }
        let _ = mac_paste::on_main(Duration::from_millis(250), || unsafe {
            let application: Option<Retained<AnyObject>> =
                msg_send![class!(NSApplication), sharedApplication];
            if let Some(application) = application {
                let modern: bool = msg_send![&*application, respondsToSelector: sel!(activate)];
                if modern {
                    let _: () = msg_send![&*application, activate];
                }
            }
        });
    }

    pub fn open(app: &AppHandle, spec: &'static PermissionSpec) -> Result<(), String> {
        if (spec.granted)() {
            close(app);
            return Ok(());
        }
        // Accessibility: macOS lists the app (switched off) and alerts once.
        if spec.request.is_some_and(|request| request()) {
            close(app);
            return Ok(());
        }
        show(app, spec)
    }

    /// Opens the pane and floats the helper beside it.
    fn show(app: &AppHandle, spec: &'static PermissionSpec) -> Result<(), String> {
        std::process::Command::new("/usr/bin/open")
            .arg(spec.settings_url)
            .spawn()
            .map_err(|e| format!("Cannot open System Settings: {e}"))?;
        let window = helper(app, spec)?;
        if let Ok(mut kind) = OPEN_KIND.lock() {
            *kind = Some(spec.kind);
        }
        let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
        watch(app.clone(), window, spec, generation);
        Ok(())
    }

    /// The running app's bundle identifier, from its bundle; the configured
    /// identifier when running outside a bundle (development builds).
    fn bundle_identifier(app: &AppHandle) -> String {
        let from_bundle = autoreleasepool(|_| unsafe {
            let bundle: Option<Retained<AnyObject>> = msg_send![class!(NSBundle), mainBundle];
            let identifier: Option<Retained<AnyObject>> = msg_send![&*bundle?, bundleIdentifier];
            let utf8: *const c_char = msg_send![&*identifier?, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
        });
        from_bundle.unwrap_or_else(|| app.config().identifier.clone())
    }

    /// `tccutil reset`, then ask macOS again; open the pane with the helper
    /// unless access is back or macOS is showing its own prompt. An open
    /// helper for this permission stays (the reset may come from it) and is
    /// watched afresh.
    pub async fn reset(
        app: &AppHandle,
        spec: &'static PermissionSpec,
        reset: &'static PermissionReset,
    ) -> Result<(), String> {
        let args = tccutil_reset_args(reset.tcc_service, &bundle_identifier(app))?;
        let output = tauri::async_runtime::spawn_blocking(move || {
            std::process::Command::new(TCCUTIL).args(&args).output()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Cannot run tccutil: {e}"))?;
        if !output.status.success() {
            return Err(reset_failure(&output.stderr));
        }
        if (reset.prompt)() || (spec.granted)() {
            close(app);
            return Ok(());
        }
        show(app, spec)
    }

    pub fn close(app: &AppHandle) {
        GENERATION.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut kind) = OPEN_KIND.lock() {
            *kind = None;
        }
        if let Some(window) = app.get_webview_window(HELPER_LABEL) {
            let _ = window.destroy();
        }
    }

    fn watch(
        app: AppHandle,
        window: tauri::WebviewWindow,
        spec: &'static PermissionSpec,
        generation: u64,
    ) {
        tauri::async_runtime::spawn(async move {
            let started = Instant::now();
            let mut shown = false;
            let mut last_frame: Option<Frame> = None;
            let mut settings_seen = false;
            let mut settings_missing_since: Option<Instant> = None;
            loop {
                if GENERATION.load(Ordering::Acquire) != generation {
                    return;
                }
                if (spec.granted)() {
                    close(&app);
                    announce(&app, spec);
                    bring_sonojot_forward(&app);
                    return;
                }
                if started.elapsed() >= LIFETIME {
                    close(&app);
                    announce(&app, spec);
                    return;
                }
                let settings = tauri::async_runtime::spawn_blocking(settings_window)
                    .await
                    .ok()
                    .flatten();
                match settings {
                    Some(_) => {
                        settings_seen = true;
                        settings_missing_since = None;
                    }
                    None if settings_seen => {
                        let since = *settings_missing_since.get_or_insert_with(Instant::now);
                        // The user closed System Settings: the helper has no target.
                        if since.elapsed() >= Duration::from_millis(2500) {
                            close(&app);
                            announce(&app, spec);
                            return;
                        }
                    }
                    None => {}
                }
                let ready = settings.is_some() || started.elapsed() >= Duration::from_millis(1500);
                if ready && (settings != last_frame || !shown) {
                    if settings.is_some() || !shown {
                        place(&window, settings);
                        last_frame = settings;
                    }
                    if !shown {
                        float(&window);
                        shown = true;
                        announce(&app, spec);
                    }
                }
                tokio::time::sleep(POLL).await;
            }
        });
    }

    extern "C-unwind" fn operation_mask(
        _: *mut AnyObject,
        _: Sel,
        _session: *mut AnyObject,
        context: isize,
    ) -> usize {
        drag_operations(context)
    }

    /// Local diagnostics only (stderr, never sent anywhere): whether the
    /// list accepted the drop. Operation 0 means it was refused.
    extern "C-unwind" fn drag_ended(
        _: *mut AnyObject,
        _: Sel,
        _session: *mut AnyObject,
        _point: CGPoint,
        operation: usize,
    ) {
        eprintln!("Permission helper drag ended with operation {operation}.");
    }

    /// One drag source for the process. AppKit does not retain it.
    fn drag_source() -> Option<Retained<AnyObject>> {
        unsafe {
            let class = AnyClass::get(c"SonoJotDragSource").or_else(|| {
                let mut builder = ClassBuilder::new(c"SonoJotDragSource", class!(NSObject))?;
                if let Some(protocol) = AnyProtocol::get(c"NSDraggingSource") {
                    builder.add_protocol(protocol);
                }
                builder.add_method(
                    sel!(draggingSession:sourceOperationMaskForDraggingContext:),
                    operation_mask as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, isize) -> usize,
                );
                builder.add_method(
                    sel!(draggingSession:endedAtPoint:operation:),
                    drag_ended as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, CGPoint, usize),
                );
                Some(builder.register())
            })?;
            thread_local! {
                static SOURCE: std::cell::OnceCell<Option<Retained<AnyObject>>> = const { std::cell::OnceCell::new() };
            }
            SOURCE.with(|source| {
                source
                    .get_or_init(|| msg_send![class, new])
                    .clone()
            })
        }
    }

    fn ns_string(text: &CStr) -> Option<Retained<AnyObject>> {
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    /// The app as Finder puts it on a drag pasteboard: `public.file-url` (the
    /// URL string) and the legacy `NSFilenamesPboardType` array of paths.
    /// The URL alone is all some drop targets read; others, like older
    /// AppKit lists, register only for the filenames type.
    fn finder_pasteboard_item(url: &AnyObject, path: &AnyObject) -> Option<Retained<AnyObject>> {
        unsafe {
            let item: Option<Retained<AnyObject>> = msg_send![class!(NSPasteboardItem), new];
            let item = item?;
            let url_string: Option<Retained<AnyObject>> = msg_send![url, absoluteString];
            let url_string = url_string?;
            let file_url_type = ns_string(FILE_URL_TYPE)?;
            let wrote_url: bool =
                msg_send![&*item, setString: &*url_string, forType: &*file_url_type];
            let paths: Option<Retained<AnyObject>> = msg_send![class!(NSArray), arrayWithObject: path];
            let paths = paths?;
            let filenames_type = ns_string(FILENAMES_TYPE)?;
            let wrote_paths: bool =
                msg_send![&*item, setPropertyList: &*paths, forType: &*filenames_type];
            if !wrote_paths {
                eprintln!("Permission helper drag carries only the file URL.");
            }
            wrote_url.then_some(item)
        }
    }

    pub fn start_drag(window: &tauri::WebviewWindow) -> Result<(), String> {
        let path = mac_paste::app_bundle_path().ok_or("Cannot find the SonoJot app.")?;
        let is_directory = path.is_dir();
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let ns_window = window.ns_window().map_err(|e| e.to_string())? as *mut AnyObject;
        let source = drag_source().ok_or("macOS could not prepare the drag.")?;
        autoreleasepool(|_| unsafe {
            let application: Option<Retained<AnyObject>> =
                msg_send![class!(NSApplication), sharedApplication];
            let application = application.ok_or("The application is unavailable.")?;
            let event: Option<Retained<AnyObject>> = msg_send![&*application, currentEvent];
            let event = event.ok_or("Hold the icon and drag it into the list.")?;
            // NSEventTypeLeftMouseDown or NSEventTypeLeftMouseDragged.
            let kind: usize = MessageReceiver::send_message(&*event, Sel::register(c"type"), ());
            if kind != 1 && kind != 6 {
                return Err("Hold the icon and drag it into the list.".into());
            }
            let view: Option<Retained<AnyObject>> = msg_send![ns_window, contentView];
            let view = view.ok_or("The helper window is not ready.")?;
            let path: Option<Retained<AnyObject>> =
                msg_send![class!(NSString), stringWithUTF8String: path.as_ptr()];
            let path = path.ok_or("Cannot find the SonoJot app.")?;
            let url: Option<Retained<AnyObject>> =
                msg_send![class!(NSURL), fileURLWithPath: &*path, isDirectory: is_directory];
            let url = url.ok_or("Cannot find the SonoJot app.")?;
            let workspace: Option<Retained<AnyObject>> =
                msg_send![class!(NSWorkspace), sharedWorkspace];
            let workspace = workspace.ok_or("The workspace is unavailable.")?;
            let icon: Option<Retained<AnyObject>> = msg_send![&*workspace, iconForFile: &*path];
            let icon = icon.ok_or("Cannot read the SonoJot icon.")?;
            let size = CGSize {
                width: 64.0,
                height: 64.0,
            };
            let _: () = msg_send![&*icon, setSize: size];
            let writer = finder_pasteboard_item(&url, &path).ok_or("macOS could not prepare the drag.")?;
            let item: Allocated<AnyObject> = msg_send![class!(NSDraggingItem), alloc];
            let item: Option<Retained<AnyObject>> = msg_send![item, initWithPasteboardWriter: &*writer];
            let item = item.ok_or("macOS could not prepare the drag.")?;
            let location: CGPoint = msg_send![ns_window, mouseLocationOutsideOfEventStream];
            let point: CGPoint = msg_send![
                &*view,
                convertPoint: location,
                fromView: Option::<&AnyObject>::None
            ];
            let frame = CGRect {
                origin: CGPoint {
                    x: point.x - size.width / 2.0,
                    y: point.y - size.height / 2.0,
                },
                size,
            };
            let _: () = msg_send![&*item, setDraggingFrame: frame, contents: &*icon];
            let items: Option<Retained<AnyObject>> =
                msg_send![class!(NSArray), arrayWithObject: &*item];
            let items = items.ok_or("macOS could not prepare the drag.")?;
            // An AppKit exception here must not cross into Rust.
            objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                let _: Option<Retained<AnyObject>> = msg_send![
                    &*view,
                    beginDraggingSessionWithItems: &*items,
                    event: &*event,
                    source: &*source
                ];
            }))
            .map_err(|_| "macOS could not start the drag. Try again.".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY: Frame = Frame {
        x: 0.0,
        y: 0.0,
        width: 1512.0,
        height: 982.0,
    };

    #[test]
    fn every_permission_has_copy_and_a_settings_pane() {
        assert!(spec("accessibility").is_ok());
        assert!(spec("camera").is_err());
        for spec in SPECS {
            assert!(spec.settings_url.starts_with("x-apple.systempreferences:"));
            assert!(!spec.title.is_empty() && !spec.body.is_empty());
            assert!(!spec.stale_hint.is_empty());
            // System Settings may not let the user remove an old entry.
            assert!(spec.stale_hint.starts_with("Already on but "), "{}", spec.stale_hint);
            let reset = spec.reset.as_ref().expect("every permission can be reset");
            assert!(tccutil_reset_args(reset.tcc_service, "studio.codystevens.eavesdropper.next").is_ok());
            assert_eq!(SPECS.iter().filter(|other| other.kind == spec.kind).count(), 1);
        }
    }

    #[test]
    fn only_lists_that_take_drops_offer_the_icon() {
        for spec in SPECS {
            let mentions_drag = spec.body.contains("drag");
            assert_eq!(spec.draggable, mentions_drag, "{}", spec.kind);
            // A list that takes the app also has + as the fallback.
            assert_eq!(spec.draggable, spec.body.contains("click\u{a0}+"), "{}", spec.kind);
        }
        let microphone = spec("microphone").unwrap();
        assert!(!microphone.draggable);
        assert!(microphone.settings_url.ends_with("Privacy_Microphone"));
        assert_eq!(microphone.reset.as_ref().unwrap().tcc_service, "Microphone");
        assert!(spec("accessibility").unwrap().draggable);
        assert!(spec("system_audio").unwrap().draggable);
    }

    #[test]
    fn microphone_asks_macos_only_when_it_never_has() {
        use microphone::{from_raw, needs_no_settings, Authorization};
        assert_eq!(from_raw(0), Authorization::NotDetermined);
        assert_eq!(from_raw(1), Authorization::Restricted);
        assert_eq!(from_raw(2), Authorization::Denied);
        assert_eq!(from_raw(3), Authorization::Authorized);
        assert_eq!(from_raw(42), Authorization::Denied);
        assert!(needs_no_settings(Authorization::Authorized));
        assert!(needs_no_settings(Authorization::NotDetermined));
        assert!(!needs_no_settings(Authorization::Denied));
        assert!(!needs_no_settings(Authorization::Restricted));
    }

    #[test]
    fn the_icon_drag_copies_or_links_and_never_moves() {
        const MOVE: usize = 16;
        const DELETE: usize = 32;
        let outside = drag_operations(0);
        assert_eq!(outside, 1 | 2 | 4);
        assert_eq!(outside & (MOVE | DELETE), 0);
        assert_eq!(drag_operations(1), 0);
        assert_eq!(FILE_URL_TYPE.to_str().unwrap(), "public.file-url");
        assert_eq!(FILENAMES_TYPE.to_str().unwrap(), "NSFilenamesPboardType");
    }

    #[test]
    fn tccutil_gets_separate_arguments_for_the_service_and_bundle() {
        assert_eq!(
            tccutil_reset_args("Accessibility", "studio.codystevens.eavesdropper.next").unwrap(),
            ["reset", "Accessibility", "studio.codystevens.eavesdropper.next"]
        );
        assert_eq!(TCCUTIL, "/usr/bin/tccutil");
        assert!(tccutil_reset_args("", "studio.codystevens.app").is_err());
        assert!(tccutil_reset_args("All Services", "studio.codystevens.app").is_err());
        assert!(tccutil_reset_args("Accessibility;", "studio.codystevens.app").is_err());
    }

    #[test]
    fn bundle_id_must_be_reverse_dns() {
        for id in ["studio.codystevens.eavesdropper.next", "com.example.My-App2"] {
            assert!(valid_bundle_id(id), "{id}");
        }
        let long = format!("com.{}", "a".repeat(260));
        for id in [
            "",
            "sonojot",
            ".studio.codystevens",
            "studio.codystevens.",
            "studio..codystevens",
            "-reset.studio",
            "studio.codystevens next",
            "studio.codystevens;rm",
            "studio.codystevens\nnext",
            "studio.codystevens/next",
            "studio.c\u{f6}dy",
            long.as_str(),
        ] {
            assert!(!valid_bundle_id(id), "{id:?}");
            assert!(tccutil_reset_args("Accessibility", id).is_err());
        }
    }

    #[test]
    fn reset_failure_keeps_tccutil_message() {
        assert_eq!(reset_failure(b"  \n"), "macOS could not reset the permission.");
        assert_eq!(
            reset_failure(b"tccutil: No such bundle identifier\n"),
            "macOS could not reset the permission: tccutil: No such bundle identifier"
        );
    }

    #[test]
    fn helper_sits_below_settings_when_there_is_room() {
        let settings = Frame {
            x: 400.0,
            y: 100.0,
            width: 700.0,
            height: 600.0,
        };
        let placed = helper_frame(Some(settings), DISPLAY, 440.0, 118.0);
        assert_eq!(placed.x, 530.0);
        assert_eq!(placed.y, 712.0);
    }

    #[test]
    fn helper_moves_beside_a_tall_settings_window() {
        let settings = Frame {
            x: 300.0,
            y: 40.0,
            width: 700.0,
            height: 900.0,
        };
        let placed = helper_frame(Some(settings), DISPLAY, 440.0, 118.0);
        assert_eq!(placed.x, 1012.0);
        assert_eq!(placed.y, 940.0 - 118.0);
        // No room on either side: overlap the bottom-right corner, on screen.
        let wide = Frame {
            x: 20.0,
            width: 1470.0,
            ..settings
        };
        let placed = helper_frame(Some(wide), DISPLAY, 440.0, 118.0);
        assert!(DISPLAY.contains(placed));
        assert_eq!(placed.x, 1490.0 - 440.0 - 16.0);
    }

    #[test]
    fn helper_without_settings_waits_near_the_top_of_the_display() {
        let display = Frame {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let placed = helper_frame(None, display, 440.0, 118.0);
        assert_eq!(placed.x, -1920.0 + (1920.0 - 440.0) / 2.0);
        assert!(display.contains(placed));
    }
}
