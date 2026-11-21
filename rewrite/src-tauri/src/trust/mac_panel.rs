//! AppKit host for the trust webview. Tauri still owns the webview and its IPC;
//! only its native view moves into a real NSPanel, created non-activating.
//! Changing a live NSWindow's class/style is not enough: AppKit establishes
//! the WindowServer activation flags during NSPanel initialization.

use super::locator::Rect;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSPoint, NSRect};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::WebviewWindow;

extern "C" fn can_become_key(_: &AnyObject, _: Sel) -> Bool {
    Bool::new(super::overlay::is_focused())
}

extern "C" fn cannot_become_main(_: &AnyObject, _: Sel) -> Bool {
    Bool::NO
}

fn panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut class = ClassBuilder::new(c"SonoJotTrustPanel", class!(NSPanel))
            .expect("unique trust panel class");
        unsafe {
            class.add_method(
                sel!(canBecomeKeyWindow),
                can_become_key as extern "C" fn(_, _) -> _,
            );
            class.add_method(
                sel!(canBecomeMainWindow),
                cannot_become_main as extern "C" fn(_, _) -> _,
            );
        }
        class.register()
    })
}

pub struct Panel {
    window: WebviewWindow,
    // The retained pointer is only read, messaged and released on the main
    // thread. Queued work sees zero after close, including the pointer watcher.
    native: Arc<AtomicUsize>,
    watcher: tauri::async_runtime::JoinHandle<()>,
}

impl Panel {
    pub fn open(window: &WebviewWindow, regions: Arc<Mutex<Vec<Rect>>>) -> Result<Self, String> {
        let native = Arc::new(AtomicUsize::new(0));
        let ready = native.clone();
        window
            .with_webview(move |webview| unsafe {
                let host = &*webview.ns_window().cast::<AnyObject>();
                let view = &*webview.inner().cast::<AnyObject>();
                let frame: NSRect = msg_send![host, frame];
                let allocated: Allocated<AnyObject> = msg_send![panel_class(), alloc];
                // NSWindowStyleMaskNonactivatingPanel, NSBackingStoreBuffered.
                let panel: Retained<AnyObject> = msg_send![allocated,
                initWithContentRect: frame, styleMask: 1usize << 7,
                backing: 2usize, defer: false];
                let _: () = msg_send![&*panel, setReleasedWhenClosed: false];
                let _: () = msg_send![&*panel, setHidesOnDeactivate: false];
                let _: () = msg_send![&*panel, setBecomesKeyOnlyIfNeeded: true];
                let _: () = msg_send![&*panel, setFloatingPanel: true];
                let _: () = msg_send![&*panel, setLevel: 3isize]; // NSFloatingWindowLevel
                let _: () = msg_send![&*panel, setOpaque: false];
                let clear: *mut AnyObject = msg_send![class!(NSColor), clearColor];
                let _: () = msg_send![&*panel, setBackgroundColor: clear];
                let _: () = msg_send![&*panel, setHasShadow: false];
                let _: () = msg_send![&*panel, setMovable: false];
                let _: () = msg_send![&*panel, setAcceptsMouseMovedEvents: true];
                let _: () = msg_send![&*panel, setIgnoresMouseEvents: true];
                let _: () = msg_send![view, removeFromSuperview];
                let _: () = msg_send![view, setAutoresizingMask: 2usize | 16];
                let _: () = msg_send![&*panel, setContentView: view];
                // Track hover even while the host editor remains the active app.
                let allocated: Allocated<AnyObject> = msg_send![class!(NSTrackingArea), alloc];
                let area: Retained<AnyObject> = msg_send![allocated,
                initWithRect: NSRect::ZERO,
                options: 0x01usize | 0x02 | 0x80 | 0x200,
                owner: view, userInfo: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![view, addTrackingArea: &*area];
                ready.store(Retained::into_raw(panel) as usize, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;

        let watched = native.clone();
        let window_for_watch = window.clone();
        let watcher = tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(33)).await;
                let native = watched.clone();
                let regions = regions.clone();
                if window_for_watch
                    .run_on_main_thread(move || unsafe {
                        let ptr = native.load(Ordering::Acquire) as *mut AnyObject;
                        let Some(panel) = ptr.as_ref() else { return };
                        let visible: bool = msg_send![panel, isVisible];
                        if !visible {
                            return;
                        }
                        // Cocoa window-local points avoid mixing the primary
                        // display's pointer scale with this window's Retina scale.
                        let point: NSPoint = msg_send![panel, mouseLocationOutsideOfEventStream];
                        let view: *mut AnyObject = msg_send![panel, contentView];
                        let bounds: NSRect = msg_send![view, bounds];
                        let scale: f64 = msg_send![panel, backingScaleFactor];
                        let x = (point.x * scale).floor() as i32;
                        let y = ((bounds.size.height - point.y) * scale).floor() as i32;
                        let inside = regions.lock().is_ok_and(|r| {
                            r.iter()
                                .any(|r| x >= r.x && y >= r.y && x < r.right() && y < r.bottom())
                        });
                        let ignores: bool = msg_send![panel, ignoresMouseEvents];
                        if ignores == inside {
                            let _: () = msg_send![panel, setIgnoresMouseEvents: !inside];
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            window: window.clone(),
            native,
            watcher,
        })
    }

    pub fn place(&self) {
        let native = self.native.clone();
        let window = self.window.clone();
        let _ = self.window.run_on_main_thread(move || {
            // Tao defers set_size/set_position to the main dispatch queue even
            // when called on the main thread. Copy their completed frame there,
            // after those jobs, rather than showing the initial 16px host frame.
            dispatch2::DispatchQueue::main().exec_async(move || unsafe {
                let ptr = native.load(Ordering::Acquire) as *mut AnyObject;
                let Some(panel) = ptr.as_ref() else { return };
                let Ok(host) = window.ns_window() else { return };
                let frame: NSRect = msg_send![host.cast::<AnyObject>(), frame];
                let _: () = msg_send![panel, setFrame: frame, display: true];
                let _: () = msg_send![panel, orderFrontRegardless];
            });
        });
    }

    pub fn focus(&self, take: bool) {
        let native = self.native.clone();
        let _ = self.window.run_on_main_thread(move || unsafe {
            let ptr = native.load(Ordering::Acquire) as *mut AnyObject;
            let Some(panel) = ptr.as_ref() else { return };
            if take {
                let view: *mut AnyObject = msg_send![panel, contentView];
                let _: bool = msg_send![panel, makeFirstResponder: view];
                // Tauri's set_focus also activates NSApplication. A
                // non-activating panel can take just the keyboard instead.
                let _: () = msg_send![panel, makeKeyWindow];
            } else {
                let _: () = msg_send![panel, resignKeyWindow];
            }
        });
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.watcher.abort();
        let native = self.native.clone();
        let window = self.window.clone();
        let _ = self.window.with_webview(move |webview| unsafe {
            let ptr = native.swap(0, Ordering::AcqRel) as *mut AnyObject;
            let Some(panel) = Retained::from_raw(ptr) else {
                return;
            };
            let _: () = msg_send![&*panel, orderOut: std::ptr::null::<AnyObject>()];
            let view = &*webview.inner().cast::<AnyObject>();
            let _: () = msg_send![view, removeFromSuperview];
            // PlatformWebview::ns_window follows the reparented WKWebView;
            // WebviewWindow::ns_window still identifies Tauri's original host.
            if let Ok(host) = window.ns_window() {
                let _: () = msg_send![host.cast::<AnyObject>(), setContentView: view];
            }
            let _: () = msg_send![&*panel, close];
        });
    }
}
