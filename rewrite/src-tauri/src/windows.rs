use crate::app::Runtime;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{window::Color, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

static CURSOR_GENERATION: AtomicU64 = AtomicU64::new(0);
static TAB_GENERATION: AtomicU64 = AtomicU64::new(0);
static EDGE_WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static TAB_DRAGGING: AtomicBool = AtomicBool::new(false);
// Match ScreenTab.svelte: the symbol hangs below, and moves with, the sheet.
const TAB_WIDTH: f64 = 600.0;
const TAB_HEIGHT: f64 = 74.0;
const RECORDING_HEIGHT: f64 = 104.0;
// Reserve the taller recording canvas; the native region clips idle's unused pixels.
const TAB_WINDOW_HEIGHT: f64 = RECORDING_HEIGHT;
const LIP_WIDTH: f64 = 50.0;
const LIP_HEIGHT: f64 = 26.0;
// This is a virtual, click-through hot zone, not an input-taking window.
// Depth is physical pixels at every DPI; the horizontal margin is logical.
const EDGE_HIT_DEPTH: i32 = 2;
const EDGE_MARGIN: f64 = 32.0;
const EDGE_DEBOUNCE: Duration = Duration::from_millis(16);
const TEMPORARY_GRACE: Duration = Duration::from_millis(5000);
const EDGE_POLL: Duration = Duration::from_millis(16);
const MONITOR_POLL: Duration = Duration::from_millis(500);

struct TabGeometry {
    monitor: Option<String>,
    placement: Option<(i32, i32, u32, u32, u64)>,
    height: f64,
    full_height: f64,
    visible: bool,
    // Reveal time of an edge-shown tab; cleared on pin, explicit show or hide.
    temporary_since: Option<Instant>,
}

static TAB_GEOMETRY: Mutex<TabGeometry> = Mutex::new(TabGeometry {
    monitor: None,
    placement: None,
    height: LIP_HEIGHT,
    full_height: TAB_HEIGHT,
    visible: false,
    temporary_since: None,
});

pub fn show_settings(app: &AppHandle) -> Result<(), String> {
    show_settings_page(app, "dictation")
}

pub fn show_settings_page(app: &AppHandle, page: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window.unminimize().map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    let url = match page {
        "report" => "index.html?page=report",
        "meetings" => "index.html?page=meetings",
        "setup" => "index.html?page=setup",
        "appearance" => "index.html?page=appearance",
        "microphone" => "index.html?page=microphone",
        "shortcuts" => "index.html?page=shortcuts",
        _ => "index.html",
    };
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App(url.into()))
        .title("SonoJot")
        .icon(
            tauri::image::Image::from_bytes(include_bytes!("../../src/brand/marks/app/icon.png"))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?
        .decorations(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .shadow(false)
        .inner_size(1220.0, 860.0)
        .min_inner_size(740.0, 540.0)
        .center()
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The persistent tab reuses the original overlay. Capture lifetime never owns it.
/// `temporary` marks an edge reveal: its grace clock starts once, then the edge
/// watcher conceals it again. Every hidden tab stays discoverable at the edge.
#[cfg_attr(target_os = "macos", allow(unreachable_code, unused_variables))]
pub fn set_screen_tab(
    app: &AppHandle,
    visible: bool,
    expanded: bool,
    focus: bool,
    capture: bool,
    temporary: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return crate::menu_panel::set_visible(app, visible, capture);
    }
    ensure_edge_watcher(app)?;
    if !visible {
        TAB_GENERATION.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut geometry) = TAB_GEOMETRY.lock() {
            geometry.visible = false;
            geometry.height = 0.0;
            geometry.temporary_since = None;
        }
        // The hidden window keeps a zero region and passthrough; the footprint
        // stays virtual so discovery works before the overlay exists.
        if let Some(window) = app.get_webview_window("overlay") {
            // tao panics on a GTK window that was never shown; a hidden
            // Linux window takes no input anyway.
            if !cfg!(target_os = "linux") || window.is_visible().unwrap_or(false) {
                window
                    .set_ignore_cursor_events(true)
                    .map_err(|e| e.to_string())?;
            }
            window.hide().map_err(|e| e.to_string())?;
            configure_tab_window(&window, false)?;
            apply_tab_region(&window, 0.0)?;
        }
        return Ok(());
    }
    TAB_GENERATION.fetch_add(1, Ordering::AcqRel);
    let window = match app.get_webview_window("overlay") {
        Some(window) => window,
        None => create_screen_tab(app)?,
    };
    let full_height = if capture {
        RECORDING_HEIGHT
    } else {
        TAB_HEIGHT
    };
    {
        let mut geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        if geometry.full_height != full_height {
            // Preserve the visible amount during a mode change, then settle to its new size.
            geometry.height = geometry.height.min(full_height);
            geometry.full_height = full_height;
        }
    }
    configure_tab_window(&window, focus)?;
    place_screen_tab(&window)?;
    let start = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?.height;
    apply_tab_region(&window, start)?;
    window
        .set_ignore_cursor_events(false)
        .map_err(|e| e.to_string())?;
    window.show().map_err(|e| e.to_string())?;
    // Commit visibility only after native show succeeds. Otherwise a transient
    // failure would leave a hidden tab marked visible and disable all retries.
    {
        let mut geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        geometry.visible = true;
        if temporary {
            geometry.temporary_since.get_or_insert_with(Instant::now);
        } else {
            geometry.temporary_since = None;
        }
    }
    if focus {
        window.set_focus().map_err(|e| e.to_string())?;
    }
    reveal_screen_tab(window, if expanded { full_height } else { LIP_HEIGHT });
    Ok(())
}

pub fn screen_tab_has_focus(app: &AppHandle) -> bool {
    app.get_webview_window("overlay")
        .is_some_and(|window| window.is_focused().unwrap_or(false))
}

/// Recheck a pending conceal against live geometry, cursor and focus. The edge
/// watcher only pre-filters; this runs under the runtime's tab gate so a pointer
/// that re-entered after the decision still keeps the tab visible.
pub fn temporary_conceal_due(app: &AppHandle) -> bool {
    if TAB_DRAGGING.load(Ordering::Acquire) {
        return false;
    }
    let (visible, since, revealed) = match TAB_GEOMETRY.lock() {
        Ok(geometry) => (
            geometry.visible,
            geometry.temporary_since,
            revealed_rect(geometry.placement, geometry.height),
        ),
        Err(_) => return false,
    };
    let Some(since) = since.filter(|_| visible) else {
        return false;
    };
    let occupied = poll_cursor(app)
        .zip(revealed)
        .is_some_and(|(point, rect)| inside(rect, point))
        || screen_tab_has_focus(app);
    temporary_expired(since, Instant::now(), occupied)
}

/// Physical-pixel rectangle: left, top, right, bottom.
type EdgeRect = (i32, i32, i32, i32);

fn inside(rect: EdgeRect, point: (i32, i32)) -> bool {
    point.0 >= rect.0 && point.0 < rect.2 && point.1 >= rect.1 && point.1 < rect.3
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabMonitor {
    name: Option<String>,
    rect: EdgeRect,
    scale: f64,
    primary: bool,
}

fn tab_monitor<'a>(
    saved: Option<&str>,
    cursor: Option<(i32, i32)>,
    monitors: &'a [TabMonitor],
) -> Option<&'a TabMonitor> {
    saved
        .and_then(|name| monitors.iter().find(|m| m.name.as_deref() == Some(name)))
        .or_else(|| {
            if saved.is_none() {
                cursor.and_then(|point| monitors.iter().find(|m| inside(m.rect, point)))
            } else {
                None
            }
        })
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

/// Resolve the live display, never a hidden window's old coordinates. Unplugging
/// the saved display falls back to primary, exactly as visible placement does.
fn edge_footprint(
    saved: Option<&str>,
    cursor: (i32, i32),
    monitors: &[TabMonitor],
) -> Option<EdgeRect> {
    let monitor = tab_monitor(saved, Some(cursor), monitors)?;
    let (left, top, right, _) = monitor.rect;
    let width = ((TAB_WIDTH + 2.0 * EDGE_MARGIN) * monitor.scale).round() as i32;
    let x = left + (right - left - width) / 2;
    Some((
        x.max(left),
        top,
        (x + width).min(right),
        top + EDGE_HIT_DEPTH,
    ))
}

/// The screen pixels the revealed paper occupies: a band of the current reveal
/// height hanging from the display edge.
fn revealed_rect(placement: Option<(i32, i32, u32, u32, u64)>, height: f64) -> Option<EdgeRect> {
    let (x, y, width, _, scale) = placement?;
    let scale = f64::from_bits(scale);
    Some((x, y, x + width as i32, y + (height * scale).round() as i32))
}

#[derive(Default)]
struct EdgeReveal {
    footprint: Option<EdgeRect>,
    since: Option<Instant>,
    last_poll: Option<Instant>,
    generation: u64,
    visible: bool,
    wait_for_exit: bool,
    retry_at: Option<Instant>,
}

impl EdgeReveal {
    fn tick(
        &mut self,
        visible: bool,
        generation: u64,
        footprint: Option<EdgeRect>,
        occupied: bool,
        now: Instant,
    ) -> bool {
        let interrupted = self
            .last_poll
            .is_some_and(|last| now.duration_since(last) > MONITOR_POLL);
        if self.footprint != footprint || self.generation != generation || interrupted {
            self.since = None;
            // A failed native show also advances the generation. Keep its retry
            // backoff unless the footprint changes or polling was suspended.
            if self.footprint != footprint || interrupted {
                self.retry_at = None;
            }
            // An explicit Hide while the pointer is still at the edge wins.
            self.wait_for_exit = occupied && (self.wait_for_exit || (self.visible && !visible));
        }
        self.last_poll = Some(now);
        self.footprint = footprint;
        self.generation = generation;
        self.visible = visible;
        if !occupied {
            self.since = None;
            self.wait_for_exit = false;
            self.retry_at = None;
            return false;
        }
        if visible || self.wait_for_exit || self.retry_at.is_some_and(|retry| now < retry) {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) < EDGE_DEBOUNCE {
            return false;
        }
        // A transient native failure is retried, without a 60 Hz error loop.
        self.retry_at = Some(now + Duration::from_secs(1));
        self.since = None;
        true
    }
}

fn temporary_expired(revealed: Instant, now: Instant, occupied: bool) -> bool {
    !occupied && now.duration_since(revealed) >= TEMPORARY_GRACE
}

#[cfg(windows)]
fn poll_cursor(_: &AppHandle) -> Option<(i32, i32)> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut point = POINT { x: 0, y: 0 };
    unsafe { (GetCursorPos(&mut point) != 0).then_some((point.x, point.y)) }
}

#[cfg(not(windows))]
fn poll_cursor(app: &AppHandle) -> Option<(i32, i32)> {
    #[cfg(target_os = "linux")]
    if crate::hyprland::active() {
        return crate::hyprland::cursor_physical();
    }
    app.get_webview_window("overlay")
        .and_then(|window| window.cursor_position().ok())
        .map(|p| (p.x as i32, p.y as i32))
}

/// Wayland clients cannot position their windows; on Hyprland the compositor
/// moves them. Coordinates are Tauri's physical pixels.
fn move_window(window: &tauri::WebviewWindow, x: i32, y: i32) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::hyprland::active() {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let title = window.title().map_err(|e| e.to_string())?;
        let logical = |value: i32| (f64::from(value) / scale).round() as i32;
        return crate::hyprland::move_window(&title, logical(x), logical(y));
    }
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

/// A bar along the top of the display (Omarchy's, on Linux) stays visible:
/// the tab hangs below it instead of under it.
fn top_bar_height(x: i32, y: i32) -> i32 {
    #[cfg(target_os = "linux")]
    if crate::hyprland::active() {
        return crate::hyprland::top_reserved_physical(x, y);
    }
    let _ = (x, y);
    0
}

fn tab_monitors(app: &AppHandle) -> Result<Vec<TabMonitor>, String> {
    let primary = app.primary_monitor().map_err(|e| e.to_string())?;
    Ok(app
        .available_monitors()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            TabMonitor {
                name: monitor.name().cloned(),
                rect: (
                    position.x,
                    position.y,
                    position.x + size.width as i32,
                    position.y + size.height as i32,
                ),
                scale: monitor.scale_factor(),
                primary: primary.as_ref().is_some_and(|m| m.position() == position),
            }
        })
        .collect())
}

pub(crate) fn tab_diagnostic(kind: &'static str, ms: u64, value: u64) {
    crate::recognition_diagnostics::event(kind, "screen-tab", std::process::id(), 0, ms, value);
}

/// Revalidate queued work under the runtime's tab gate. Hide, pointer movement,
/// display removal and DPI changes can all invalidate a poll before it executes.
pub(crate) fn edge_reveal_due(app: &AppHandle, generation: u64) -> bool {
    let Ok(monitors) = tab_monitors(app) else {
        return false;
    };
    let Some(cursor) = poll_cursor(app) else {
        return false;
    };
    let Ok(geometry) = TAB_GEOMETRY.lock() else {
        return false;
    };
    !geometry.visible
        && TAB_GENERATION.load(Ordering::Acquire) == generation
        && edge_footprint(geometry.monitor.as_deref(), cursor, &monitors)
            .is_some_and(|rect| inside(rect, cursor))
}

/// The process owns this poll, not the hidden WebView or the async capture pool.
/// It also maintains display geometry before the first overlay is ever created.
fn ensure_edge_watcher(app: &AppHandle) -> Result<(), String> {
    if EDGE_WATCH_STARTED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let app = app.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("screen-tab-edge".into())
        .spawn(move || {
            let mut reveal = EdgeReveal::default();
            let mut monitors = Vec::new();
            let mut refreshed = None;
            let mut placement_failed = false;
            tab_diagnostic("tab-edge-watcher-started", 0, EDGE_POLL.as_millis() as u64);
            loop {
                std::thread::sleep(EDGE_POLL);
                let now = Instant::now();
                if let Some(last) = reveal.last_poll {
                    let gap = now.duration_since(last);
                    if gap > Duration::from_secs(2) {
                        tab_diagnostic("tab-edge-poll-resumed", gap.as_millis() as u64, 0);
                    }
                }
                if refreshed.is_none_or(|at| now.duration_since(at) >= MONITOR_POLL) {
                    refreshed = Some(now);
                    // Empty/error results deliberately invalidate the old hot zone.
                    let next = tab_monitors(&app).unwrap_or_default();
                    if next != monitors {
                        tab_diagnostic("tab-displays-refreshed", 0, next.len() as u64);
                        monitors = next;
                    }
                    if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                        let failed = runtime.refresh_tab_placement(&app, &monitors).is_err();
                        if failed != placement_failed {
                            tab_diagnostic(
                                if failed {
                                    "tab-placement-failed"
                                } else {
                                    "tab-placement-recovered"
                                },
                                0,
                                monitors.len() as u64,
                            );
                            placement_failed = failed;
                        }
                    }
                }
                let cursor = poll_cursor(&app);
                let (visible, temporary_since, revealed, saved, generation) = {
                    let Ok(geometry) = TAB_GEOMETRY.lock() else {
                        continue;
                    };
                    (
                        geometry.visible,
                        geometry.temporary_since,
                        revealed_rect(geometry.placement, geometry.height),
                        geometry.monitor.clone(),
                        TAB_GENERATION.load(Ordering::Acquire),
                    )
                };
                let footprint =
                    cursor.and_then(|point| edge_footprint(saved.as_deref(), point, &monitors));
                let occupied = cursor
                    .zip(footprint)
                    .is_some_and(|(point, rect)| inside(rect, point));
                if reveal.tick(visible, generation, footprint, occupied, now) {
                    tab_diagnostic(
                        "tab-edge-trigger",
                        EDGE_DEBOUNCE.as_millis() as u64,
                        generation,
                    );
                    if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                        if runtime
                            .inner()
                            .reveal_temporary_tab(&app, generation)
                            .is_err()
                        {
                            tab_diagnostic("tab-edge-reveal-failed", 0, generation);
                        }
                    }
                }
                if let Some(since) = temporary_since.filter(|_| visible) {
                    let occupied = cursor
                        .zip(revealed)
                        .is_some_and(|(point, rect)| inside(rect, point))
                        || screen_tab_has_focus(&app);
                    if temporary_expired(since, Instant::now(), occupied) {
                        if let Some(runtime) = app.try_state::<Arc<Runtime>>() {
                            let _ = runtime.inner().conceal_temporary_tab(&app);
                        }
                    }
                }
            }
        })
    {
        EDGE_WATCH_STARTED.store(false, Ordering::Release);
        return Err(error.to_string());
    }
    Ok(())
}

fn create_screen_tab(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let window = WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("index.html?view=overlay".into()),
    )
    .title("SonoJot screen tab")
    .inner_size(TAB_WIDTH, TAB_WINDOW_HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .background_color(Color(0, 0, 0, 0))
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;
    {
        let mut geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        geometry.placement = None;
        geometry.height = 0.0;
        geometry.full_height = TAB_HEIGHT;
        geometry.visible = false;
        geometry.temporary_since = None;
    }
    Ok(window)
}

fn place_screen_tab(window: &tauri::WebviewWindow) -> Result<(), String> {
    place_screen_tab_on_monitors(window, &tab_monitors(window.app_handle())?)
}

pub(crate) fn place_screen_tab_on_monitors(
    window: &tauri::WebviewWindow,
    monitors: &[TabMonitor],
) -> Result<(), String> {
    if TAB_DRAGGING.load(Ordering::Acquire) {
        return Ok(());
    }
    // Reassert the native z-order without activating or moving the user's editor.
    keep_tab_on_top(window)?;
    let saved = TAB_GEOMETRY
        .lock()
        .map_err(|e| e.to_string())?
        .monitor
        .clone();
    let monitor = tab_monitor(saved.as_deref(), poll_cursor(window.app_handle()), monitors)
        .ok_or("No display is available for the screen tab.")?;
    let scale = monitor.scale;
    let width = (TAB_WIDTH * scale).round() as u32;
    let height = (TAB_WINDOW_HEIGHT * scale).round() as u32;
    let x = monitor.rect.0 + (monitor.rect.2 - monitor.rect.0 - width as i32) / 2;
    let y = monitor.rect.1 + top_bar_height(monitor.rect.0, monitor.rect.1);
    let placement = (x, y, width, height, scale.to_bits());
    let (changed, region_height, full_height, visible) = {
        let geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        let changed = geometry.placement != Some(placement);
        (
            changed,
            geometry.height,
            geometry.full_height,
            geometry.visible,
        )
    };
    // Hyprland can apply its own placement as the window maps. This runs on
    // every display poll, so a misplaced tab is corrected within 500 ms.
    #[cfg(target_os = "linux")]
    let changed = changed
        || (visible
            && crate::hyprland::active()
            && window.title().is_ok_and(|title| {
                let expected = (
                    (f64::from(x) / scale).round() as i32,
                    (f64::from(y - ((full_height - region_height) * scale).round() as i32) / scale).round() as i32,
                );
                crate::hyprland::window_position(&title).is_some_and(|at| at != expected)
            }));
    if changed {
        move_window(window, x, y - ((full_height - region_height) * scale).round() as i32)?;
        window
            .set_size(tauri::PhysicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        {
            let mut geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
            geometry.monitor = monitor.name.clone();
            geometry.placement = Some(placement);
        }
        if let Err(error) = apply_tab_region(window, if visible { region_height } else { 0.0 }) {
            // Do not cache a placement whose input region failed to update.
            // The monitor poll will retry it even if display geometry stays fixed.
            TAB_GEOMETRY.lock().map_err(|e| e.to_string())?.placement = None;
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn keep_tab_on_top(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0;
    if unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn keep_tab_on_top(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.set_always_on_top(true).map_err(|e| e.to_string())
}

/// Drag the lip using global physical coordinates; snap to the active display at release.
/// The native button check also finishes a drag when the WebView loses pointer capture.
#[tauri::command]
pub async fn drag_screen_tab(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "overlay" {
        return Err("Drag the screen tab using its logo.".into());
    }
    #[cfg(not(windows))]
    {
        return Err("Screen-tab monitor dragging is currently available on Windows.".into());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
        if TAB_DRAGGING.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        struct DragGuard;
        impl Drop for DragGuard {
            fn drop(&mut self) {
                TAB_DRAGGING.store(false, Ordering::Release);
            }
        }
        let guard = DragGuard;
        TAB_GENERATION.fetch_add(1, Ordering::AcqRel);
        let start = window.cursor_position().map_err(|e| e.to_string())?;
        let origin = window.outer_position().map_err(|e| e.to_string())?;
        let began = Instant::now();
        let result = async {
            while unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } < 0
                && began.elapsed() < Duration::from_secs(60)
            {
                if !TAB_GEOMETRY.lock().map_err(|e| e.to_string())?.visible {
                    break;
                }
                let point = window.cursor_position().map_err(|e| e.to_string())?;
                window
                    .set_position(tauri::PhysicalPosition::new(
                        origin.x + (point.x - start.x).round() as i32,
                        origin.y + (point.y - start.y).round() as i32,
                    ))
                    .map_err(|e| e.to_string())?;
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
            Ok::<(), String>(())
        }
        .await;
        // Clear the old monitor even if release occurs in a gap or a display disappears.
        // place_screen_tab resolves only active displays, falling back to primary/first.
        {
            let mut geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
            geometry.monitor = None;
            geometry.placement = None;
        }
        drop(guard);
        let placed = place_screen_tab(&window);
        result.and(placed)
    }
}

fn reveal_screen_tab(window: tauri::WebviewWindow, target: f64) {
    let generation = TAB_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let start = TAB_GEOMETRY.lock().map(|g| g.height).unwrap_or(LIP_HEIGHT);
    let animated = tab_animation_enabled();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            if TAB_GENERATION.load(Ordering::Acquire) != generation {
                break;
            }
            let fraction = if animated {
                (started.elapsed().as_secs_f64() / 0.2).min(1.0)
            } else {
                1.0
            };
            let height = start + (target - start) * (1.0 - (1.0 - fraction).powi(3));
            // Read Tauri metadata before taking our mutex: its dispatcher may use the UI thread.
            let Ok(region) = tab_region_writer(&window) else {
                break;
            };
            // SetWindowRgn sends window messages. Run it on the UI thread without a held
            // mutex, and discard queued frames after Hide or a direction reversal.
            if window
                .run_on_main_thread(move || {
                    {
                        let Ok(mut geometry) = TAB_GEOMETRY.lock() else {
                            return;
                        };
                        if !geometry.visible || TAB_GENERATION.load(Ordering::Acquire) != generation
                        {
                            return;
                        }
                        geometry.height = height;
                    }
                    let _ = region(height);
                })
                .is_err()
            {
                break;
            }
            if fraction >= 1.0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }
    });
}

#[cfg(windows)]
fn configure_tab_window(window: &tauri::WebviewWindow, focus: bool) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    window.set_focusable(focus).map_err(|e| e.to_string())?;
    configure_cursor_window(window)?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 | WS_EX_TOOLWINDOW;
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (if focus {
                style & !WS_EX_NOACTIVATE
            } else {
                style | WS_EX_NOACTIVATE
            }) as isize,
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn configure_tab_window(window: &tauri::WebviewWindow, focus: bool) -> Result<(), String> {
    window.set_focusable(focus).map_err(|e| e.to_string())
}

fn apply_tab_region(window: &tauri::WebviewWindow, height: f64) -> Result<(), String> {
    tab_region_writer(window)?(height)
}

#[cfg(windows)]
fn tab_region_writer(
    window: &tauri::WebviewWindow,
) -> Result<impl Fn(f64) -> Result<(), String> + Send + 'static, String> {
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let (placement, full_height) = {
        let geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        (geometry.placement, geometry.full_height)
    };
    Ok(move |height: f64| {
        if TAB_DRAGGING.load(Ordering::Acquire) {
            return Ok(());
        }
        let region = screen_tab_region(scale, height, full_height)?;
        unsafe {
            if SetWindowRgn(hwnd as _, region, 1) == 0 {
                DeleteObject(region);
                return Err(std::io::Error::last_os_error().to_string());
            }
            // SetWindowRgn takes ownership on success.
            if let Some((x, y, _, _, _)) = placement {
                // Move pixels and their native input region together. NOACTIVATE preserves
                // the current editor even when a recording reveals a previously hidden tab.
                if SetWindowPos(
                    hwnd as _,
                    std::ptr::null_mut(),
                    x,
                    y - ((full_height - height) * scale).round() as i32,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
            }
        }
        Ok(())
    })
}

#[cfg(windows)]
fn screen_tab_region(
    scale: f64,
    height: f64,
    full_height: f64,
) -> Result<windows_sys::Win32::Graphics::Gdi::HRGN, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, RGN_AND, RGN_OR,
    };
    let pixel = |value: f64| (value * scale).round() as i32;
    unsafe {
        let sheet_height = full_height - LIP_HEIGHT;
        let clipped_top = full_height - height;
        // The full paper shape stays fixed in window coordinates. Its upper portion
        // is cut at the display edge while the window slides down, including on stacked displays.
        let region = if height <= 0.0 {
            CreateRectRgn(0, 0, 0, 0)
        } else {
            CreateRoundRectRgn(
                pixel((TAB_WIDTH - LIP_WIDTH) / 2.0),
                pixel(sheet_height - 8.0),
                pixel((TAB_WIDTH + LIP_WIDTH) / 2.0) + 1,
                pixel(full_height) + 1,
                pixel(16.0),
                pixel(16.0),
            )
        };
        if region.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if height > LIP_HEIGHT {
            let body = CreateRoundRectRgn(
                0,
                pixel(-8.0),
                pixel(TAB_WIDTH) + 1,
                pixel(sheet_height) + 1,
                pixel(16.0),
                pixel(16.0),
            );
            if body.is_null() {
                DeleteObject(region);
                return Err(std::io::Error::last_os_error().to_string());
            }
            let combined = CombineRgn(region, region, body, RGN_OR);
            DeleteObject(body);
            if combined == 0 {
                DeleteObject(region);
                return Err("Cannot shape the screen tab.".into());
            }
        }
        let bounds = CreateRectRgn(0, pixel(clipped_top), pixel(TAB_WIDTH), pixel(full_height));
        if bounds.is_null() {
            DeleteObject(region);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let clipped = CombineRgn(region, region, bounds, RGN_AND);
        DeleteObject(bounds);
        if clipped == 0 {
            DeleteObject(region);
            return Err("Cannot clip the screen tab.".into());
        }
        Ok(region)
    }
}

#[cfg(target_os = "linux")]
fn tab_region_writer(
    window: &tauri::WebviewWindow,
) -> Result<impl Fn(f64) -> Result<(), String> + Send + 'static, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let (placement, full_height) = {
        let geometry = TAB_GEOMETRY.lock().map_err(|e| e.to_string())?;
        (geometry.placement, geometry.full_height)
    };
    let window = window.clone();
    Ok(move |height: f64| {
        if TAB_DRAGGING.load(Ordering::Acquire) {
            return Ok(());
        }
        // The same paper shape as the Windows region, in logical pixels: the
        // sheet takes input only while revealed, the hanging lip always.
        let sheet_height = full_height - LIP_HEIGHT;
        let clipped_top = full_height - height;
        let mut rectangles = Vec::new();
        if height > 0.0 {
            rectangles.push(((TAB_WIDTH - LIP_WIDTH) / 2.0, sheet_height - 8.0, LIP_WIDTH, LIP_HEIGHT + 8.0));
        }
        if height > LIP_HEIGHT {
            rectangles.push((0.0, 0.0, TAB_WIDTH, sheet_height));
        }
        let shown = window.clone();
        window
            .run_on_main_thread(move || {
                use gtk::prelude::WidgetExt;
                let Ok(gtk_window) = shown.gtk_window() else { return };
                let region = gtk::cairo::Region::create();
                for (x, y, w, h) in &rectangles {
                    let top = y.max(clipped_top);
                    if y + h > top {
                        let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(
                            x.round() as i32,
                            top.round() as i32,
                            w.round() as i32,
                            (y + h - top).round() as i32,
                        ));
                    }
                }
                gtk_window.input_shape_combine_region(Some(&region));
            })
            .map_err(|e| e.to_string())?;
        if let Some((x, y, _, _, _)) = placement {
            move_window(&window, x, y - ((full_height - height) * scale).round() as i32)?;
        }
        Ok(())
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn tab_region_writer(
    _: &tauri::WebviewWindow,
) -> Result<impl Fn(f64) -> Result<(), String> + Send + 'static, String> {
    Ok(|_| Ok(()))
}

#[cfg(windows)]
fn tab_animation_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };
    let mut enabled: i32 = 0;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        ) != 0
            && enabled != 0
    }
}

#[cfg(not(windows))]
fn tab_animation_enabled() -> bool {
    false
}

/// A click-through status ring. It never takes keyboard focus or intercepts input.
pub fn show_cursor(app: &AppHandle) -> Result<(), String> {
    close_cursor(app);
    let generation = CURSOR_GENERATION.load(Ordering::Acquire);
    let window = WebviewWindowBuilder::new(
        app,
        "cursor",
        WebviewUrl::App("index.html?view=cursor".into()),
    )
    .title("SonoJot cursor status")
    .inner_size(48.0, 48.0)
    .min_inner_size(48.0, 48.0)
    .max_inner_size(48.0, 48.0)
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
    let window = build_quiet(app, window).map_err(|e| e.to_string())?;
    // GTK has no native window to shape until show realizes it, and tao
    // panics on an unrealized one, so Linux makes it click-through after show.
    #[cfg(not(target_os = "linux"))]
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    position_cursor(&window);
    show_cursor_window(&window)?;
    #[cfg(target_os = "linux")]
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    configure_cursor_window(&window)?;
    window
        .set_size(tauri::LogicalSize::new(48.0, 48.0))
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        let mut previous = None;
        while CURSOR_GENERATION.load(Ordering::Acquire) == generation {
            if let Some(position) = cursor_position(&window) {
                if previous != Some(position) {
                    let _ = move_window(&window, position.0, position.1);
                    previous = Some(position);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(33)).await;
        }
    });
    Ok(())
}

/// Build a window that must leave the user's current app in front. On macOS,
/// wry activates SonoJot whenever it creates a webview; that brought an open
/// SonoJot window forward and took focus from the field being dictated into.
/// Tauri creates the window later on the main thread, so the guard is lifted
/// by a main-thread task queued behind the creation.
#[cfg(target_os = "macos")]
pub fn build_quiet(
    app: &AppHandle,
    builder: WebviewWindowBuilder<'_, tauri::Wry, AppHandle>,
) -> tauri::Result<tauri::WebviewWindow> {
    crate::mac_paste::begin_quiet_build();
    let built = builder.build();
    if app.run_on_main_thread(crate::mac_paste::end_quiet_build).is_err() {
        crate::mac_paste::end_quiet_build();
    }
    built
}

#[cfg(not(target_os = "macos"))]
pub fn build_quiet(
    _: &AppHandle,
    builder: WebviewWindowBuilder<'_, tauri::Wry, AppHandle>,
) -> tauri::Result<tauri::WebviewWindow> {
    builder.build()
}

pub fn close_cursor(app: &AppHandle) {
    CURSOR_GENERATION.fetch_add(1, Ordering::AcqRel);
    if let Some(window) = app.get_webview_window("cursor") {
        let _ = window.destroy();
    }
}

pub fn finish_cursor(app: &AppHandle) {
    let generation = CURSOR_GENERATION.load(Ordering::Acquire);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
        if CURSOR_GENERATION.load(Ordering::Acquire) == generation {
            close_cursor(&app);
        }
    });
}

fn position_cursor(window: &tauri::WebviewWindow) {
    if let Some((x, y)) = cursor_position(window) {
        let _ = move_window(window, x, y);
    }
}

#[cfg(windows)]
fn configure_cursor_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_MAXIMIZEBOX,
        WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    // Tao retains caption styles even for undecorated windows. Those styles
    // impose a 136px minimum on Windows; this non-interactive popup needs none.
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let popup = (style
            & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX))
            | WS_POPUP;
        SetWindowLongPtrW(hwnd, GWL_STYLE, popup as isize);
        if SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn configure_cursor_window(_: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

/// macOS: Tauri's show is makeKeyAndOrderFront. The indicator only floats
/// beside the caret, so it is ordered front without becoming key, like the
/// menu bar sheet, and never pulls SonoJot's main window forward.
#[cfg(target_os = "macos")]
fn show_cursor_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    let shown = window.clone();
    window
        .run_on_main_thread(move || {
            let Ok(pointer) = shown.ns_window() else {
                return;
            };
            let ns_window = pointer as *mut objc2::runtime::AnyObject;
            unsafe {
                let _: () = objc2::msg_send![ns_window, setHidesOnDeactivate: false];
                let _: () = objc2::msg_send![ns_window, orderFrontRegardless];
            }
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "macos"))]
fn show_cursor_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())
}

fn cursor_position(window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    let pointer = || {
        #[cfg(target_os = "linux")]
        if crate::hyprland::active() {
            return crate::hyprland::cursor_physical().map(|(x, y)| (f64::from(x), f64::from(y)));
        }
        window.cursor_position().ok().map(|p| (p.x, p.y))
    };
    let anchor = caret_anchor().or_else(pointer)?;
    let monitor = window.available_monitors().ok()?.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        anchor.0 >= p.x as f64
            && anchor.1 >= p.y as f64
            && anchor.0 < p.x as f64 + s.width as f64
            && anchor.1 < p.y as f64 + s.height as f64
    })?;
    let scale = monitor.scale_factor();
    let bounds = monitor.work_area();
    let size = (48.0 * scale).ceil() as i32;
    let x = (anchor.0 + 9.0 * scale).round() as i32;
    let y = (anchor.1 + 10.0 * scale).round() as i32;
    Some((
        x.clamp(
            bounds.position.x,
            (bounds.position.x + bounds.size.width as i32 - size).max(bounds.position.x),
        ),
        y.clamp(
            bounds.position.y,
            (bounds.position.y + bounds.size.height as i32 - size).max(bounds.position.y),
        ),
    ))
}

#[cfg(windows)]
fn caret_anchor() -> Option<(f64, f64)> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
        },
    };
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..std::mem::zeroed()
        };
        if GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndCaret.is_null() {
            return None;
        }
        let mut point = POINT {
            x: info.rcCaret.right,
            y: info.rcCaret.top,
        };
        if ClientToScreen(info.hwndCaret, &mut point) == 0 {
            return None;
        }
        Some((point.x as f64, point.y as f64))
    }
}

#[cfg(not(windows))]
fn caret_anchor() -> Option<(f64, f64)> {
    None
}

#[cfg(all(test, windows))]
mod screen_tab_tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{DeleteObject, GetRgnBox, PtInRegion, NULLREGION},
    };

    #[test]
    fn real_gdi_regions_exclude_transparent_pixels_at_supported_scales() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let pixel = |n: f64| (n * scale).round() as i32;
            for full_height in [TAB_HEIGHT, RECORDING_HEIGHT] {
                for height in [0.0, 12.0, LIP_HEIGHT, LIP_HEIGHT + 10.0, full_height] {
                    let region = screen_tab_region(scale, height, full_height).unwrap();
                    let top = pixel(full_height - height);
                    let sheet_bottom = full_height - LIP_HEIGHT;
                    unsafe {
                        let mut bounds: RECT = std::mem::zeroed();
                        let kind = GetRgnBox(region, &mut bounds);
                        if height == 0.0 {
                            assert_eq!(kind, NULLREGION);
                            assert_eq!(PtInRegion(region, pixel(TAB_WIDTH / 2.0), pixel(12.0)), 0);
                        } else {
                            assert_eq!(bounds.top, top, "paper must meet the display edge");
                            assert_eq!(bounds.bottom, pixel(full_height));
                            assert!(bounds.bottom <= pixel(TAB_WINDOW_HEIGHT));
                            assert_ne!(PtInRegion(region, pixel(TAB_WIDTH / 2.0), top), 0);
                            assert_eq!(PtInRegion(region, pixel(TAB_WIDTH / 2.0), top - 1), 0);
                            assert_eq!(PtInRegion(region, pixel(TAB_WIDTH), top), 0);
                            assert_eq!(
                                PtInRegion(
                                    region,
                                    pixel(TAB_WIDTH / 2.0),
                                    pixel(TAB_WINDOW_HEIGHT) - 1
                                ) != 0,
                                full_height == TAB_WINDOW_HEIGHT,
                                "idle's unused recording canvas must not intercept input"
                            );
                            assert_eq!(
                                PtInRegion(region, pixel(TAB_WIDTH / 2.0), pixel(full_height)),
                                0
                            );
                            assert_eq!(
                                PtInRegion(
                                    region,
                                    pixel((TAB_WIDTH - LIP_WIDTH) / 2.0),
                                    pixel(full_height) - 1
                                ),
                                0,
                                "rounded cut must not intercept its corner"
                            );
                            if height <= LIP_HEIGHT {
                                assert_eq!(bounds.left, pixel((TAB_WIDTH - LIP_WIDTH) / 2.0));
                                assert_eq!(bounds.right, pixel((TAB_WIDTH + LIP_WIDTH) / 2.0));
                                assert_eq!(PtInRegion(region, pixel(40.0), top), 0);
                            } else {
                                assert_eq!(bounds.left, 0);
                                assert_eq!(bounds.right, pixel(TAB_WIDTH));
                                assert_ne!(
                                    PtInRegion(region, pixel(TAB_WIDTH / 2.0), pixel(sheet_bottom)),
                                    0,
                                    "the hover corridor must stay connected"
                                );
                                assert_ne!(
                                    PtInRegion(
                                        region,
                                        pixel(TAB_WIDTH / 2.0),
                                        pixel(full_height) - 1
                                    ),
                                    0
                                );
                                assert_eq!(PtInRegion(region, 0, pixel(sheet_bottom) - 1), 0);
                            }
                        }
                        DeleteObject(region);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod edge_reveal_tests {
    use super::*;

    const FOOTPRINT: EdgeRect = (628, 0, 1292, 2);

    #[test]
    fn entry_reveals_within_two_polls_and_failed_reveal_retries() {
        let start = Instant::now();
        let mut reveal = EdgeReveal::default();
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, start));
        assert!(!reveal.tick(
            false,
            1,
            Some(FOOTPRINT),
            true,
            start + Duration::from_millis(15)
        ));
        assert!(reveal.tick(false, 1, Some(FOOTPRINT), true, start + EDGE_POLL));
        assert!(!reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * 2));
        // A failed native show advances the generation before returning its error.
        for i in 3..65 {
            assert!(!reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * i));
        }
        assert!(reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * 65));
    }

    #[test]
    fn exit_and_reentry_restart_the_debounce() {
        let start = Instant::now();
        let mut reveal = EdgeReveal::default();
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, start));
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), false, start + EDGE_POLL));
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, start + EDGE_POLL * 2));
        assert!(reveal.tick(false, 1, Some(FOOTPRINT), true, start + EDGE_POLL * 3));
    }

    #[test]
    fn explicit_hide_at_the_edge_requires_exit_before_reveal() {
        let start = Instant::now();
        let mut reveal = EdgeReveal::default();
        assert!(!reveal.tick(true, 1, Some(FOOTPRINT), true, start));
        assert!(!reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL));
        assert!(!reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * 2));
        assert!(!reveal.tick(false, 2, Some(FOOTPRINT), false, start + EDGE_POLL * 3));
        assert!(!reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * 4));
        assert!(reveal.tick(false, 2, Some(FOOTPRINT), true, start + EDGE_POLL * 5));
    }

    #[test]
    fn hours_hidden_and_sleep_resume_do_not_disable_discovery() {
        let start = Instant::now();
        let mut reveal = EdgeReveal::default();
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), false, start));
        let later = start + Duration::from_secs(8 * 3600);
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, later));
        assert!(reveal.tick(false, 1, Some(FOOTPRINT), true, later + EDGE_POLL));
        // A long suspended poll cannot use its old debounce sample.
        let resumed = later + Duration::from_secs(3600);
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, resumed));
        assert!(reveal.tick(false, 1, Some(FOOTPRINT), true, resumed + EDGE_POLL));
    }

    #[test]
    fn geometry_change_or_missing_pointer_restarts_entry() {
        let start = Instant::now();
        let mut reveal = EdgeReveal::default();
        assert!(!reveal.tick(false, 1, Some(FOOTPRINT), true, start));
        let moved = (1920, -1440, 2916, -1438);
        assert!(!reveal.tick(false, 1, Some(moved), true, start + EDGE_POLL));
        assert!(!reveal.tick(false, 1, None, false, start + EDGE_POLL * 2));
        assert!(!reveal.tick(false, 1, Some(moved), true, start + EDGE_POLL * 3));
        assert!(reveal.tick(false, 1, Some(moved), true, start + EDGE_POLL * 4));
    }

    fn monitor(name: &str, rect: EdgeRect, scale: f64, primary: bool) -> TabMonitor {
        TabMonitor {
            name: Some(name.into()),
            rect,
            scale,
            primary,
        }
    }

    #[test]
    fn footprint_has_margin_but_only_two_physical_rows_at_every_dpi() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let displays = [monitor("primary", (0, 0, 2560, 1440), scale, true)];
            let rect = edge_footprint(None, (1280, 0), &displays).unwrap();
            assert_eq!(
                rect.2 - rect.0,
                ((TAB_WIDTH + 2.0 * EDGE_MARGIN) * scale).round() as i32
            );
            assert!(inside(rect, (rect.0, 0)));
            assert!(inside(rect, (rect.2 - 1, 1)));
            assert!(!inside(rect, (1280, 2)));
            assert!(!inside(rect, (1280, -1)));
            assert!(!inside(rect, (rect.0 - 1, 0)));
            assert!(!inside(rect, (rect.2, 0)));
        }
    }

    #[test]
    fn hidden_startup_and_visible_placement_choose_the_same_display() {
        let displays = [
            monitor("primary", (0, 0, 1920, 1080), 1.0, true),
            monitor("upper", (-2560, -1440, 0, 0), 1.5, false),
        ];
        let point = (-1280, -1440);
        assert_eq!(
            tab_monitor(None, Some(point), &displays)
                .unwrap()
                .name
                .as_deref(),
            Some("upper")
        );
        assert_eq!(
            edge_footprint(None, point, &displays),
            Some((-1778, -1440, -782, -1438))
        );
        assert_eq!(
            edge_footprint(Some("primary"), point, &displays),
            Some(FOOTPRINT)
        );
    }

    #[test]
    fn unplug_and_dpi_changes_use_live_geometry_instead_of_old_placement() {
        let mut displays = vec![
            monitor("primary", (0, 0, 1920, 1080), 1.0, true),
            monitor("secondary", (1920, 0, 4480, 1440), 1.0, false),
        ];
        assert_eq!(
            edge_footprint(Some("secondary"), (3200, 0), &displays),
            Some((2868, 0, 3532, 2))
        );
        displays[1].scale = 2.0;
        displays[1].rect = (0, -1440, 2560, 0);
        assert_eq!(
            edge_footprint(Some("secondary"), (1280, -1440), &displays),
            Some((616, -1440, 1944, -1438))
        );
        displays.pop();
        assert_eq!(
            edge_footprint(Some("secondary"), (960, 0), &displays),
            Some(FOOTPRINT)
        );
        assert_eq!(edge_footprint(Some("secondary"), (960, 0), &[]), None);
    }

    #[test]
    fn hot_zone_stays_within_a_narrow_display() {
        let displays = [monitor("small", (-400, 0, 0, 700), 1.0, true)];
        assert_eq!(
            edge_footprint(None, (-200, 0), &displays),
            Some((-400, 0, 0, 2))
        );
    }

    #[test]
    fn temporary_reveal_stays_through_grace_and_occupancy() {
        let revealed = Instant::now();
        let later = revealed + TEMPORARY_GRACE;
        assert!(!temporary_expired(
            revealed,
            later - Duration::from_millis(1),
            false
        ));
        assert!(temporary_expired(revealed, later, false));
        assert!(!temporary_expired(
            revealed,
            later + Duration::from_secs(30),
            true
        ));
        assert!(temporary_expired(
            revealed,
            later + Duration::from_secs(31),
            false
        ));
        let placement = (1000, 0, 900, 156, 1.5f64.to_bits());
        assert_eq!(
            revealed_rect(Some(placement), RECORDING_HEIGHT),
            Some((1000, 0, 1900, 156))
        );
    }
}
