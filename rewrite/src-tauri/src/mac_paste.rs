//! macOS text-delivery primitives: the frontmost application, the general
//! pasteboard, Accessibility trust and a synthesized Command+V.
//!
//! Everything here is plain Objective-C messaging through `objc2` and C calls
//! into system frameworks; no extra crates. AppKit calls that touch
//! `NSApplication` or Text Input Sources hop to the main thread with a short
//! timeout, so a busy main thread degrades to copy-only instead of hanging
//! delivery.

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Imp, Sel};
use objc2::{class, msg_send, sel};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicI32, AtomicU16, AtomicUsize, Ordering};
use std::sync::{Once, OnceLock};
use std::time::Duration;

/// `kVK_ANSI_V`: the V key on an ANSI keyboard, used when the layout lookup fails.
const ANSI_V: u16 = 9;
const COMMAND_KEY: u16 = 55;
const FLAG_COMMAND: u64 = 0x0010_0000;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const HID_SYSTEM_STATE: i32 = 1;
const HID_EVENT_TAP: u32 = 0;

#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(
        queue: *const DispatchQueue,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
    fn pthread_main_np() -> i32;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventSourceFlagsState(state: i32) -> u64;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    static kCFBooleanTrue: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn LMGetKbdType() -> u8;
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const c_void,
        key: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        characters: *mut u16,
    ) -> i32;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSPasteboardTypeString: &'static AnyObject;
    static NSWorkspaceDidActivateApplicationNotification: &'static AnyObject;
    static NSWorkspaceApplicationKey: &'static AnyObject;
}

fn own_pid() -> i32 {
    std::process::id() as i32
}

/// Run `work` on the AppKit main thread and wait at most `timeout` for it.
/// Already on the main thread, it runs inline. A late result is discarded.
pub fn on_main<R: Send + 'static>(
    timeout: Duration,
    work: impl FnOnce() -> R + Send + 'static,
) -> Option<R> {
    if unsafe { pthread_main_np() } != 0 {
        return Some(work());
    }
    type Job = Box<dyn FnOnce() + Send>;
    extern "C" fn run(context: *mut c_void) {
        let job = unsafe { Box::from_raw(context.cast::<Job>()) };
        job();
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let job: Box<Job> = Box::new(Box::new(move || {
        let _ = sender.send(work());
    }));
    unsafe {
        dispatch_async_f(
            &raw const _dispatch_main_q,
            Box::into_raw(job).cast(),
            run,
        )
    };
    receiver.recv_timeout(timeout).ok()
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Ask macOS to list SonoJot under Accessibility (switched off) and show its
/// alert. It always asks: the permission assistant calls it whenever the user
/// presses Allow, so an entry removed from the list (or by a reset) is listed again.
pub fn prompt_accessibility() -> bool {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast(),
        );
        if options.is_null() {
            return AXIsProcessTrusted();
        }
        let trusted = AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
        trusted
    }
}

fn running_application(pid: i32) -> Option<Retained<AnyObject>> {
    unsafe {
        msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid
        ]
    }
}

/// The captured application's bundle ID; no activation or main-thread wait.
pub fn app_bundle_identifier(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    autoreleasepool(|_| unsafe {
        let application = running_application(pid)?;
        let identifier: Option<Retained<AnyObject>> = msg_send![&*application, bundleIdentifier];
        let identifier = identifier?;
        let utf8: *const c_char = msg_send![&*identifier, UTF8String];
        if utf8.is_null() {
            return None;
        }
        let identifier = CStr::from_ptr(utf8).to_str().ok()?;
        (!identifier.is_empty()).then(|| identifier.to_owned())
    })
}

pub fn frontmost_pid() -> Option<i32> {
    autoreleasepool(|_| unsafe {
        let workspace: Option<Retained<AnyObject>> =
            msg_send![class!(NSWorkspace), sharedWorkspace];
        let application: Option<Retained<AnyObject>> =
            msg_send![&*workspace?, frontmostApplication];
        let pid: i32 = msg_send![&*application?, processIdentifier];
        (pid > 0).then_some(pid)
    })
}

pub fn app_running(pid: i32) -> bool {
    autoreleasepool(|_| {
        running_application(pid).is_some_and(|application| {
            let terminated: bool = unsafe { msg_send![&*application, isTerminated] };
            !terminated
        })
    })
}

pub fn is_own_pid(pid: i32) -> bool {
    pid == own_pid()
}

/// Hand activation from SonoJot back to `pid`. Only call while SonoJot is the
/// active application: macOS 14+ grants activation that the active app yields.
pub fn activate(pid: i32) -> bool {
    on_main(Duration::from_millis(250), move || {
        autoreleasepool(|_| unsafe {
            let Some(target) = running_application(pid) else {
                return false;
            };
            let application: Option<Retained<AnyObject>> =
                msg_send![class!(NSApplication), sharedApplication];
            if let Some(application) = application {
                let yields: bool = msg_send![
                    &*application,
                    respondsToSelector: sel!(yieldActivationToApplication:)
                ];
                if yields {
                    let _: () = msg_send![&*application, yieldActivationToApplication: &*target];
                }
            }
            // Default options bring the target's key and main windows forward.
            msg_send![&*target, activateWithOptions: 0usize]
        })
    })
    .unwrap_or(false)
}

static LAST_EXTERNAL: AtomicI32 = AtomicI32::new(0);

extern "C-unwind" fn application_activated(_: *mut AnyObject, _: Sel, notification: *mut AnyObject) {
    let activated = || -> Option<i32> {
        if notification.is_null() {
            return None;
        }
        unsafe {
            let info: Option<Retained<AnyObject>> = msg_send![notification, userInfo];
            let application: Option<Retained<AnyObject>> =
                msg_send![&*info?, objectForKey: NSWorkspaceApplicationKey];
            let pid: i32 = msg_send![&*application?, processIdentifier];
            Some(pid)
        }
    };
    autoreleasepool(|_| {
        let pid = activated().or_else(frontmost_pid);
        if let Some(pid) = pid.filter(|pid| *pid > 0 && !is_own_pid(*pid)) {
            LAST_EXTERNAL.store(pid, Ordering::Release);
        }
    });
}

/// Track the last application other than SonoJot that became active. A click on
/// the menu bar sheet can activate SonoJot before the record command arrives;
/// this is the app the user was working in.
pub fn observe_activations() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        if let Some(pid) = frontmost_pid().filter(|pid| !is_own_pid(*pid)) {
            LAST_EXTERNAL.store(pid, Ordering::Release);
        }
        let _ = on_main(Duration::from_secs(2), || {
            autoreleasepool(|_| unsafe {
                let class = AnyClass::get(c"SonoJotActivationObserver").or_else(|| {
                    let mut builder =
                        ClassBuilder::new(c"SonoJotActivationObserver", class!(NSObject))?;
                    // The function signature matches `applicationActivated:`.
                    builder.add_method(
                        sel!(applicationActivated:),
                        application_activated
                            as extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    );
                    Some(builder.register())
                });
                let Some(class) = class else { return };
                let observer: Option<Retained<AnyObject>> = msg_send![class, new];
                let Some(observer) = observer else { return };
                let workspace: Option<Retained<AnyObject>> =
                    msg_send![class!(NSWorkspace), sharedWorkspace];
                let Some(workspace) = workspace else { return };
                let center: Option<Retained<AnyObject>> =
                    msg_send![&*workspace, notificationCenter];
                let Some(center) = center else { return };
                let _: () = msg_send![
                    &*center,
                    addObserver: &*observer,
                    selector: sel!(applicationActivated:),
                    name: NSWorkspaceDidActivateApplicationNotification,
                    object: Option::<&AnyObject>::None
                ];
                // The observer lives for the whole process.
                std::mem::forget(observer);
            })
        });
    });
}

/// Nonzero while SonoJot builds a window that must not activate it.
static QUIET_BUILDS: AtomicUsize = AtomicUsize::new(0);
static ACTIVATE: OnceLock<Imp> = OnceLock::new();
static ACTIVATE_IGNORING: OnceLock<Imp> = OnceLock::new();

extern "C-unwind" fn quiet_activate(this: *mut AnyObject, cmd: Sel) {
    if QUIET_BUILDS.load(Ordering::Acquire) > 0 {
        return;
    }
    if let Some(original) = ACTIVATE.get() {
        let original: extern "C-unwind" fn(*mut AnyObject, Sel) =
            unsafe { std::mem::transmute(*original) };
        original(this, cmd);
    }
}

extern "C-unwind" fn quiet_activate_ignoring(this: *mut AnyObject, cmd: Sel, flag: Bool) {
    if QUIET_BUILDS.load(Ordering::Acquire) > 0 {
        return;
    }
    if let Some(original) = ACTIVATE_IGNORING.get() {
        let original: extern "C-unwind" fn(*mut AnyObject, Sel, Bool) =
            unsafe { std::mem::transmute(*original) };
        original(this, cmd, flag);
    }
}

/// wry activates the app every time it creates a webview, even for a window
/// built unfocused. A shortcut press lets that activation through, which put
/// SonoJot's windows in front of the app being dictated into and took its text
/// field's focus. Between `begin_quiet_build` and `end_quiet_build`, both
/// NSApplication activation calls do nothing; every other caller is unchanged.
pub fn begin_quiet_build() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| unsafe {
        let application = class!(NSApplication);
        if let Some(method) = application.instance_method(sel!(activate)) {
            // Keep the original first, so a call racing the swap still reaches it.
            if ACTIVATE.set(method.implementation()).is_ok() {
                method.set_implementation(std::mem::transmute::<
                    extern "C-unwind" fn(*mut AnyObject, Sel),
                    Imp,
                >(quiet_activate));
            }
        }
        if let Some(method) = application.instance_method(sel!(activateIgnoringOtherApps:)) {
            if ACTIVATE_IGNORING.set(method.implementation()).is_ok() {
                method.set_implementation(std::mem::transmute::<
                    extern "C-unwind" fn(*mut AnyObject, Sel, Bool),
                    Imp,
                >(quiet_activate_ignoring));
            }
        }
    });
    QUIET_BUILDS.fetch_add(1, Ordering::AcqRel);
}

pub fn end_quiet_build() {
    let _ = QUIET_BUILDS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        count.checked_sub(1)
    });
}

/// The app that was active just before SonoJot, while it is still running.
pub fn previous_external_pid() -> Option<i32> {
    let pid = LAST_EXTERNAL.load(Ordering::Acquire);
    (pid > 0 && !is_own_pid(pid) && app_running(pid)).then_some(pid)
}

fn general_pasteboard() -> Option<Retained<AnyObject>> {
    unsafe { msg_send![class!(NSPasteboard), generalPasteboard] }
}

/// Replace the general pasteboard with plain text and return its change count.
pub fn set_pasteboard(text: &str) -> Result<u32, String> {
    let text = CString::new(text).map_err(|_| "Clipboard text contains a NUL character.")?;
    autoreleasepool(|_| unsafe {
        let pasteboard = general_pasteboard().ok_or("The macOS pasteboard is unavailable.")?;
        let string: Option<Retained<AnyObject>> =
            msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
        let string = string.ok_or("The text could not be prepared for the pasteboard.")?;
        let _: isize = msg_send![&*pasteboard, clearContents];
        let written: bool =
            msg_send![&*pasteboard, setString: &*string, forType: NSPasteboardTypeString];
        if !written {
            return Err("macOS did not accept the pasteboard text.".into());
        }
        let count: isize = msg_send![&*pasteboard, changeCount];
        Ok(count as u32)
    })
}

pub fn pasteboard_change_count() -> Option<u32> {
    autoreleasepool(|_| unsafe {
        let pasteboard = general_pasteboard()?;
        let count: isize = msg_send![&*pasteboard, changeCount];
        Some(count as u32)
    })
}

pub fn pasteboard_has_text(expected: &str) -> bool {
    autoreleasepool(|_| unsafe {
        let Some(pasteboard) = general_pasteboard() else {
            return false;
        };
        let string: Option<Retained<AnyObject>> =
            msg_send![&*pasteboard, stringForType: NSPasteboardTypeString];
        let Some(string) = string else {
            return false;
        };
        let utf8: *const c_char = msg_send![&*string, UTF8String];
        !utf8.is_null() && CStr::from_ptr(utf8).to_bytes() == expected.as_bytes()
    })
}

pub fn modifiers_pressed() -> bool {
    let flags = unsafe { CGEventSourceFlagsState(HID_SYSTEM_STATE) };
    flags & (FLAG_COMMAND | FLAG_SHIFT | FLAG_CONTROL | FLAG_OPTION) != 0
}

/// First key code whose Command-modified character is `wanted`.
pub(crate) fn key_for_character(
    translate: impl Fn(u16) -> Option<char>,
    wanted: char,
) -> Option<u16> {
    (0..128).find(|key| translate(*key) == Some(wanted))
}

/// Text Input Sources must be read on the main thread.
fn layout_key_for_v() -> Option<u16> {
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let found = if data.is_null() {
            None
        } else {
            let layout = CFDataGetBytePtr(data).cast::<c_void>();
            let keyboard = LMGetKbdType() as u32;
            key_for_character(
                |key| {
                    let mut dead_keys = 0u32;
                    let mut length = 0usize;
                    let mut characters = [0u16; 4];
                    // kUCKeyActionDisplay, Command held, no dead keys. With
                    // "Dvorak - QWERTY ⌘" this yields the QWERTY V key.
                    let status = UCKeyTranslate(
                        layout,
                        key,
                        3,
                        1,
                        keyboard,
                        1,
                        &mut dead_keys,
                        characters.len(),
                        &mut length,
                        characters.as_mut_ptr(),
                    );
                    (status == 0 && length == 1)
                        .then(|| char::from_u32(characters[0] as u32))
                        .flatten()
                        .map(|c| c.to_ascii_lowercase())
                },
                'v',
            )
        };
        CFRelease(source);
        found
    }
}

/// The key that types V with Command held in the current layout.
fn paste_key() -> u16 {
    static LAST: AtomicU16 = AtomicU16::new(ANSI_V);
    match on_main(Duration::from_millis(150), layout_key_for_v).flatten() {
        Some(key) => {
            LAST.store(key, Ordering::Relaxed);
            key
        }
        None => LAST.load(Ordering::Relaxed),
    }
}

/// Post Command+V to the HID event tap, which delivers to the frontmost app.
/// Returns false when an event could not be created; nothing is posted then.
pub fn post_paste() -> bool {
    let key = paste_key();
    unsafe {
        let source = CGEventSourceCreate(HID_SYSTEM_STATE);
        let events = [
            (COMMAND_KEY, true, FLAG_COMMAND),
            (key, true, FLAG_COMMAND),
            (key, false, FLAG_COMMAND),
            (COMMAND_KEY, false, 0),
        ]
        .map(|(code, down, flags)| {
            let event = CGEventCreateKeyboardEvent(source, code, down);
            if !event.is_null() {
                CGEventSetFlags(event, flags);
            }
            event
        });
        let complete = events.iter().all(|event| !event.is_null());
        if complete {
            for event in events {
                CGEventPost(HID_EVENT_TAP, event);
            }
        }
        for event in events.into_iter().filter(|event| !event.is_null()) {
            CFRelease(event);
        }
        if !source.is_null() {
            CFRelease(source);
        }
        complete
    }
}

/// Path of the running app bundle, or the executable outside a bundle.
pub fn app_bundle_path() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(std::path::Path::to_path_buf)
        .or(Some(executable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_lookup_finds_the_first_matching_key() {
        let dvorak = |key: u16| match key {
            9 => Some('k'),
            47 => Some('v'),
            _ => None,
        };
        assert_eq!(key_for_character(dvorak, 'v'), Some(47));
        assert_eq!(key_for_character(|_| None, 'v'), None);
    }

    /// Replaces the real pasteboard, so it runs only on request:
    /// `cargo test --lib mac_paste -- --ignored`.
    #[test]
    #[ignore]
    fn pasteboard_round_trip_reports_a_new_change_count() {
        let before = pasteboard_change_count().unwrap();
        let count = set_pasteboard("SonoJot pasteboard test ✓").unwrap();
        assert_ne!(count, before);
        assert_eq!(pasteboard_change_count(), Some(count));
        assert!(pasteboard_has_text("SonoJot pasteboard test ✓"));
        assert!(!pasteboard_has_text("something else"));
    }

    #[test]
    fn own_process_is_never_an_external_target() {
        assert!(is_own_pid(std::process::id() as i32));
        assert!(!is_own_pid(0));
    }
}
