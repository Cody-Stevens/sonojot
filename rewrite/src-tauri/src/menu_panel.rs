//! macOS menu-bar panel. The status item is the always-available control.
//! A click drops the same paper sheet the Windows screen tab uses, directly
//! under that icon. Recording keeps the sheet up; an idle sheet closes on the
//! next click outside it. The window is ordered forward without becoming key,
//! so a recording can appear while the editor keeps focus.
//!
//! Horizontal positions are CoreGraphics points. Vertical positions share the
//! space tray-icon and tao already use: both subtract a Cocoa y from
//! `CGDisplayPixelsHigh` (pixels), while CoreGraphics uses the main display's
//! point height. `window_space_y` adds that gap. On a 1× display the gap is
//! zero. tao's per-display physical pixels are not used; they only agree on
//! the primary display.

#[cfg(any(test, target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PtRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(any(test, target_os = "macos"))]
fn contains(rect: PtRect, x: f64, y: f64) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.width && y < rect.y + rect.height
}

#[cfg(any(test, target_os = "macos"))]
fn anchor_fits(tray: PtRect, monitor: PtRect) -> bool {
    tray.width > 0.0
        && tray.height > 0.0
        && contains(
            monitor,
            tray.x + tray.width / 2.0,
            tray.y + tray.height / 2.0,
        )
}

/// A click misses the panel when it is outside both the sheet and the status item.
#[cfg(any(test, target_os = "macos"))]
fn click_misses(panel: PtRect, tray: PtRect, x: f64, y: f64) -> bool {
    !contains(panel, x, y) && !contains(tray, x, y)
}

/// Place the sheet under the status item and keep it on that display.
/// Every rectangle shares the window's vertical space, origin at the top left.
#[cfg(any(test, target_os = "macos"))]
fn menu_panel_frame(tray: PtRect, panel_width: f64, panel_height: f64, monitor: PtRect) -> PtRect {
    let max_x = monitor.x + monitor.width - panel_width;
    let x = if max_x < monitor.x {
        monitor.x
    } else {
        (tray.x + tray.width / 2.0 - panel_width / 2.0).clamp(monitor.x, max_x)
    };
    let max_y = (monitor.y + monitor.height - panel_height).max(monitor.y);
    let y = (tray.y + tray.height).clamp(monitor.y, max_y);
    PtRect {
        x,
        y,
        width: panel_width,
        height: panel_height,
    }
}

/// Move a CoreGraphics top-left y into the vertical space shared with tao and
/// tray-icon. Both of those subtract from the main display's pixel height.
#[cfg(any(test, target_os = "macos"))]
fn window_space_y(cg_y: f64, main_pixel_height: f64, main_point_height: f64) -> f64 {
    cg_y + main_pixel_height - main_point_height
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn a_cached_anchor_must_still_belong_to_a_connected_display() {
        let display = PtRect {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let anchor = PtRect {
            x: -100.0,
            y: 0.0,
            width: 40.0,
            height: 24.0,
        };
        assert!(anchor_fits(anchor, display));
        assert!(!anchor_fits(anchor, PtRect { x: 0.0, ..display }));
        assert!(!anchor_fits(
            PtRect {
                width: 0.0,
                ..anchor
            },
            display
        ));
    }

    #[test]
    fn the_sheet_hangs_from_the_status_item() {
        let tray = PtRect {
            x: 1800.0,
            y: 0.0,
            width: 40.0,
            height: 32.0,
        };
        let monitor = PtRect {
            x: 0.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        };
        assert_eq!(
            menu_panel_frame(tray, 600.0, 48.0, monitor),
            PtRect {
                x: 1520.0,
                y: 32.0,
                width: 600.0,
                height: 48.0,
            }
        );
    }

    #[test]
    fn the_sheet_stays_on_the_display_when_the_icon_is_near_an_edge() {
        let monitor = PtRect {
            x: 0.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        };
        let right = PtRect {
            x: 2500.0,
            y: 0.0,
            width: 40.0,
            height: 24.0,
        };
        assert_eq!(menu_panel_frame(right, 600.0, 78.0, monitor).x, 1960.0);
        let left = PtRect {
            x: 4.0,
            y: 100.0,
            width: 30.0,
            height: 24.0,
        };
        let frame = menu_panel_frame(left, 600.0, 48.0, monitor);
        assert_eq!(frame.x, 0.0);
        assert_eq!(frame.y, 124.0);
        let secondary = PtRect {
            x: -1920.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };
        let icon = PtRect {
            x: -1910.0,
            y: 0.0,
            width: 36.0,
            height: 24.0,
        };
        assert_eq!(menu_panel_frame(icon, 600.0, 48.0, secondary).x, -1920.0);
    }

    #[test]
    fn an_outside_click_includes_the_editor_and_spares_the_icon() {
        let panel = PtRect {
            x: 1000.0,
            y: 30.0,
            width: 600.0,
            height: 48.0,
        };
        let tray = PtRect {
            x: 1280.0,
            y: 0.0,
            width: 40.0,
            height: 30.0,
        };
        assert!(click_misses(panel, tray, 40.0, 400.0));
        assert!(!click_misses(panel, tray, 1100.0, 40.0));
        assert!(!click_misses(panel, tray, 1290.0, 10.0));
    }

    #[test]
    fn a_retina_main_display_uses_the_same_vertical_space_as_the_window() {
        // 1512 points tall, backed by 3024 pixels. The menu-bar icon is at the top.
        let pixels = 3024.0;
        let points = 1512.0;
        let display = PtRect {
            x: 0.0,
            y: window_space_y(0.0, pixels, points),
            width: 1512.0,
            height: points,
        };
        let tray = PtRect {
            x: 1400.0,
            y: window_space_y(0.0, pixels, points),
            width: 36.0,
            height: 24.0,
        };
        assert_eq!(tray.y, 1512.0);
        let frame = menu_panel_frame(tray, 600.0, 48.0, display);
        assert_eq!(frame.y, 1536.0);
        let on_sheet = window_space_y(30.0, pixels, points);
        assert!(contains(frame, 1410.0, on_sheet));
        let in_editor = window_space_y(400.0, pixels, points);
        assert!(click_misses(frame, tray, 200.0, in_editor));
        assert_eq!(window_space_y(32.0, 1080.0, 1080.0), 32.0);
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use crate::app::Runtime;
    use crate::domain::AppState;
    use objc2::runtime::AnyObject;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::{
        AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    };

    /// Sheet sizes match `ScreenTab.svelte` when `presentation=menubar`.
    const MENU_WIDTH: f64 = 600.0;
    const IDLE_HEIGHT: f64 = 48.0;
    const RECORDING_HEIGHT: f64 = 78.0;
    const LABEL: &str = "overlay";

    static PRESENT: AtomicU64 = AtomicU64::new(0);
    static ANCHOR: Mutex<Option<(PtRect, Vec<Display>)>> = Mutex::new(None);
    static LAST_TRAY: Mutex<Option<Instant>> = Mutex::new(None);
    static LAST_TOGGLE: Mutex<Option<Instant>> = Mutex::new(None);
    static PASSIVE_READY: AtomicBool = AtomicBool::new(false);
    static SHOWN: AtomicBool = AtomicBool::new(false);
    static CAPTURE: AtomicBool = AtomicBool::new(false);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *const c_void) -> CGPoint;
        fn CGGetActiveDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
        fn CGDisplayModeRelease(mode: *mut c_void);
        fn CGMainDisplayID() -> u32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    /// The pointer in window space. CGEventGetLocation is a CoreGraphics point
    /// (top-left origin); the y shift matches tray-icon and tao.
    fn pointer_point() -> Option<(f64, f64)> {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let point = CGEventGetLocation(event);
            CFRelease(event);
            let (pixels, points) = main_display_heights();
            Some((point.x, window_space_y(point.y, pixels, points)))
        }
    }

    /// A CoreGraphics top-left y in the vertical space tao positions windows in.
    pub(crate) fn to_window_space_y(cg_y: f64) -> f64 {
        let (pixels, points) = main_display_heights();
        window_space_y(cg_y, pixels, points)
    }

    fn main_display_heights() -> (f64, f64) {
        unsafe {
            let id = CGMainDisplayID();
            let bounds = CGDisplayBounds(id);
            (CGDisplayPixelsHigh(id) as f64, bounds.size.height)
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Display {
        rect: PtRect,
        scale: f64,
        primary: bool,
    }

    /// Bounds come from CoreGraphics, in points, then the y origin moves into
    /// window space. tao's monitor size multiplies `CGDisplayPixelsWide` by the
    /// scale a second time, so it is not used here.
    ///
    /// The scale is the display mode's backing pixels per point, which is what
    /// tray-icon multiplies by. `CGDisplayPixelsWide` reports points on Retina
    /// displays, so it cannot supply the scale.
    fn backing_scale(id: u32) -> f64 {
        unsafe {
            let mode = CGDisplayCopyDisplayMode(id);
            if mode.is_null() {
                return 1.0;
            }
            let points = CGDisplayModeGetWidth(mode) as f64;
            let pixels = CGDisplayModeGetPixelWidth(mode) as f64;
            CGDisplayModeRelease(mode);
            if points > 0.0 {
                (pixels / points).max(1.0)
            } else {
                1.0
            }
        }
    }

    fn displays() -> Vec<Display> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        let status =
            unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
        if status != 0 || count == 0 {
            return Vec::new();
        }
        let (pixels, points) = main_display_heights();
        let main = unsafe { CGMainDisplayID() };
        (0..count as usize)
            .map(|index| {
                let id = ids[index];
                let bounds = unsafe { CGDisplayBounds(id) };
                let scale = backing_scale(id);
                Display {
                    rect: PtRect {
                        x: bounds.origin.x,
                        y: window_space_y(bounds.origin.y, pixels, points),
                        width: bounds.size.width,
                        height: bounds.size.height,
                    },
                    scale,
                    primary: id == main,
                }
            })
            .collect()
    }

    fn display_at(displays: &[Display], x: f64, y: f64) -> Option<&Display> {
        displays.iter().find(|display| contains(display.rect, x, y))
    }

    fn home_display(displays: &[Display]) -> Option<&Display> {
        displays
            .iter()
            .find(|display| display.primary)
            .or_else(|| displays.first())
    }

    /// tray-icon scales the status item's frame by its own display. The pointer
    /// sits on that item during tray events, so its display gives the scale back.
    /// Without a pointer, every display whose scale puts the item inside it is a
    /// candidate; the primary display wins ties.
    fn tray_anchor(
        displays: &[Display],
        rect: &tauri::Rect,
        pointer: Option<(f64, f64)>,
    ) -> Option<PtRect> {
        let position = rect.position.to_physical::<f64>(1.0);
        let size = rect.size.to_physical::<f64>(1.0);
        if size.width < 1.0 || size.height < 1.0 {
            return None;
        }
        let scaled = |scale: f64| PtRect {
            x: position.x / scale,
            y: position.y / scale,
            width: size.width / scale,
            height: size.height / scale,
        };
        if let Some((x, y)) = pointer {
            if let Some(display) = display_at(displays, x, y) {
                return Some(scaled(display.scale));
            }
        }
        let fits = |display: &&Display| {
            let anchor = scaled(display.scale);
            contains(
                display.rect,
                anchor.x + anchor.width / 2.0,
                anchor.y + anchor.height / 2.0,
            )
        };
        displays
            .iter()
            .filter(&fits)
            .find(|display| display.primary)
            .or_else(|| displays.iter().find(&fits))
            .or_else(|| home_display(displays))
            .map(|display| scaled(display.scale))
    }

    pub fn observe_tray(_app: &AppHandle, event: &tauri::tray::TrayIconEvent) {
        use tauri::tray::{MouseButtonState, TrayIconEvent};
        let rect = match event {
            TrayIconEvent::Click {
                rect, button_state, ..
            } => {
                if *button_state == MouseButtonState::Down {
                    if let Ok(mut last) = LAST_TRAY.lock() {
                        *last = Some(Instant::now());
                    }
                }
                rect
            }
            TrayIconEvent::DoubleClick { rect, .. }
            | TrayIconEvent::Enter { rect, .. }
            | TrayIconEvent::Leave { rect, .. } => rect,
            // Enter already recorded the item; a Move stream adds nothing.
            _ => return,
        };
        let current = displays();
        if let Some(anchor) = tray_anchor(&current, rect, pointer_point()) {
            if let Ok(mut saved) = ANCHOR.lock() {
                *saved = Some((anchor, current));
            }
        }
    }

    pub fn toggle_allowed() -> bool {
        let Ok(mut previous) = LAST_TOGGLE.lock() else {
            return false;
        };
        let now = Instant::now();
        if previous.is_some_and(|at| now.duration_since(at) < Duration::from_millis(400)) {
            return false;
        }
        *previous = Some(now);
        crate::windows::tab_diagnostic("tab-menu-toggle", 0, 0);
        true
    }

    pub fn set_visible(app: &AppHandle, visible: bool, capture: bool) -> Result<(), String> {
        let generation = PRESENT.fetch_add(1, Ordering::AcqRel) + 1;
        PASSIVE_READY.store(false, Ordering::Release);
        SHOWN.store(visible, Ordering::Release);
        if !visible {
            if let Some(window) = app.get_webview_window(LABEL) {
                window.hide().map_err(|e| e.to_string())?;
            }
            crate::windows::tab_diagnostic("tab-menu-hidden", 0, generation);
            return Ok(());
        }
        let window = match app.get_webview_window(LABEL) {
            Some(window) => window,
            None => create_panel(app)?,
        };
        let displays = displays();
        let anchor = current_anchor(app, &displays).unwrap_or_else(|| fallback_anchor(&displays));
        CAPTURE.store(capture, Ordering::Release);
        place_panel(&window, &displays, anchor, capture)?;
        let shown = window.clone();
        shown
            .clone()
            .run_on_main_thread(move || {
                if PRESENT.load(Ordering::Acquire) != generation {
                    return;
                }
                show_passive(&shown);
                crate::windows::tab_diagnostic("tab-menu-revealed", 0, generation);
            })
            .map_err(|e| e.to_string())?;
        watch_outside_clicks(app.clone(), generation);
        Ok(())
    }

    /// Capture can end while the sheet stays open. Nothing re-requests the tab then,
    /// so every state emit returns an open sheet to the height its paper uses.
    pub fn sync_capture(app: &AppHandle, capture: bool) {
        if !SHOWN.load(Ordering::Acquire) || CAPTURE.swap(capture, Ordering::AcqRel) == capture {
            return;
        }
        if let Some(window) = app.get_webview_window(LABEL) {
            let displays = displays();
            let anchor =
                current_anchor(app, &displays).unwrap_or_else(|| fallback_anchor(&displays));
            let _ = place_panel(&window, &displays, anchor, capture);
        }
    }

    /// The right-click menu pops at the pointer but needs a window to host it.
    /// A login launch has no main window and the sheet has never opened, so the
    /// closed sheet is created to stand in.
    pub fn menu_host(app: &AppHandle) -> Option<tauri::WebviewWindow> {
        app.get_webview_window("main")
            .or_else(|| app.get_webview_window(LABEL))
            .or_else(|| create_panel(app).ok())
            .or_else(|| app.get_webview_window(LABEL))
    }

    fn create_panel(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
        let window = WebviewWindowBuilder::new(
            app,
            LABEL,
            WebviewUrl::App("index.html?view=overlay&presentation=menubar".into()),
        )
        .title("SonoJot")
        .inner_size(MENU_WIDTH, IDLE_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .accept_first_mouse(true)
        .build()
        .map_err(|e| e.to_string())?;
        let _ = window.set_visible_on_all_workspaces(true);
        PASSIVE_READY.store(false, Ordering::Release);
        Ok(window)
    }

    fn current_anchor(app: &AppHandle, displays: &[Display]) -> Option<PtRect> {
        let cached = ANCHOR.lock().ok().and_then(|saved| {
            saved
                .as_ref()
                .filter(|(anchor, layout)| {
                    layout == displays && displays.iter().any(|d| anchor_fits(*anchor, d.rect))
                })
                .map(|(anchor, _)| *anchor)
        });
        // A tray event just supplied the exact display scale. Keep that anchor
        // rather than guessing between mixed-DPI displays with overlapping ranges.
        if recent_tray_click() && cached.is_some() {
            return cached;
        }
        // Query the live status item on every reveal. The menu bar can move
        // between monitors while this panel stays hidden for hours.
        // TrayIcon::rect waits for the main thread, so never call it there.
        if objc2::MainThreadMarker::new().is_none() {
            let fresh = app
                .tray_by_id("sonojot")
                .and_then(|tray| tray.rect().ok().flatten())
                .and_then(|rect| tray_anchor(displays, &rect, None))
                .filter(|anchor| displays.iter().any(|d| anchor_fits(*anchor, d.rect)));
            if let Some(anchor) = fresh {
                if let Ok(mut saved) = ANCHOR.lock() {
                    *saved = Some((anchor, displays.to_vec()));
                }
                return Some(anchor);
            }
        }
        cached
    }

    fn fallback_anchor(displays: &[Display]) -> PtRect {
        match home_display(displays) {
            Some(display) => PtRect {
                x: display.rect.x + (display.rect.width - 40.0) / 2.0,
                y: display.rect.y,
                width: 40.0,
                height: 24.0,
            },
            None => PtRect {
                x: 400.0,
                y: 0.0,
                width: 40.0,
                height: 24.0,
            },
        }
    }

    fn place_panel(
        window: &tauri::WebviewWindow,
        displays: &[Display],
        anchor: PtRect,
        capture: bool,
    ) -> Result<(), String> {
        let display = display_at(
            displays,
            anchor.x + anchor.width / 2.0,
            anchor.y + anchor.height / 2.0,
        )
        .or_else(|| home_display(displays))
        .map(|display| display.rect)
        .unwrap_or(PtRect {
            x: 0.0,
            y: 0.0,
            width: 10_000.0,
            height: 10_000.0,
        });
        let height = if capture {
            RECORDING_HEIGHT
        } else {
            IDLE_HEIGHT
        };
        let frame = menu_panel_frame(anchor, MENU_WIDTH, height, display);
        // x is a CoreGraphics point. y is already in window space, which is the
        // value tao subtracts from the main display's pixel height.
        window
            .set_size(LogicalSize::new(frame.width, frame.height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(LogicalPosition::new(frame.x, frame.y))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn show_passive(window: &tauri::WebviewWindow) {
        let Ok(pointer) = window.ns_window() else {
            return;
        };
        let ns_window = pointer as *mut AnyObject;
        // NSPopUpMenuWindowLevel keeps the sheet above the editor. orderFrontRegardless
        // does not make the window key, so the recording reveal leaves typing where it was.
        // set_always_on_top posts its level change asynchronously and would replace this.
        unsafe {
            let _: () = objc2::msg_send![ns_window, setLevel: 101isize];
            let _: () = objc2::msg_send![ns_window, setHidesOnDeactivate: false];
            let _: () = objc2::msg_send![ns_window, setMovable: false];
            let behavior: usize = 1 | (1 << 4) | (1 << 6) | (1 << 8);
            let _: () = objc2::msg_send![ns_window, setCollectionBehavior: behavior];
            let _: () = objc2::msg_send![ns_window, orderFrontRegardless];
        }
        PASSIVE_READY.store(true, Ordering::Release);
    }

    fn pointer_down() -> bool {
        unsafe { CGEventSourceButtonState(0, 0) || CGEventSourceButtonState(0, 1) }
    }

    fn recent_tray_click() -> bool {
        LAST_TRAY
            .lock()
            .ok()
            .and_then(|last| *last)
            .is_some_and(|last| last.elapsed() < Duration::from_millis(300))
    }

    fn holds_open(app: &AppHandle) -> bool {
        let Some(runtime) = app.try_state::<Arc<Runtime>>() else {
            return true;
        };
        let Ok(snapshot) = runtime.snapshot() else {
            return true;
        };
        if !snapshot.tab_visible {
            return true;
        }
        snapshot.meeting_recording
            || snapshot.error.is_some()
            || matches!(
                snapshot.state,
                AppState::Loading
                    | AppState::Listening
                    | AppState::Finishing
                    | AppState::Cleaning
                    | AppState::Error
            )
    }

    fn pointer_misses(window: &tauri::WebviewWindow) -> bool {
        let Some((x, y)) = pointer_point() else {
            return false;
        };
        let Ok(scale) = window.scale_factor() else {
            return false;
        };
        let Ok(origin) = window.outer_position() else {
            return false;
        };
        let Ok(size) = window.outer_size() else {
            return false;
        };
        // The window reports itself scaled by its own display; undo that scale.
        let origin = origin.to_logical::<f64>(scale);
        let size = size.to_logical::<f64>(scale);
        let panel = PtRect {
            x: origin.x,
            y: origin.y,
            width: size.width,
            height: size.height,
        };
        let tray = ANCHOR
            .lock()
            .ok()
            .and_then(|anchor| anchor.as_ref().map(|(rect, _)| *rect))
            .unwrap_or(PtRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            });
        click_misses(panel, tray, x, y)
    }

    fn watch_outside_clicks(app: AppHandle, generation: u64) {
        tauri::async_runtime::spawn(async move {
            let mut down = pointer_down();
            loop {
                if PRESENT.load(Ordering::Acquire) != generation {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(32)).await;
                if PRESENT.load(Ordering::Acquire) != generation
                    || !PASSIVE_READY.load(Ordering::Acquire)
                {
                    down = pointer_down();
                    continue;
                }
                let pressed = pointer_down();
                let rising = pressed && !down;
                down = pressed;
                if !rising || recent_tray_click() || holds_open(&app) {
                    continue;
                }
                let Some(window) = app.get_webview_window(LABEL) else {
                    continue;
                };
                if !pointer_misses(&window) {
                    continue;
                }
                let app = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    if PRESENT.load(Ordering::Acquire) != generation {
                        return;
                    }
                    if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                        let runtime = runtime.inner().clone();
                        let _ = runtime.set_tab(&app, false, Some(false), false);
                    }
                })
                .await;
            }
        });
    }
}

#[cfg(target_os = "macos")]
pub use native::{menu_host, observe_tray, set_visible, sync_capture, toggle_allowed};
#[cfg(target_os = "macos")]
pub(crate) use native::to_window_space_y;
