//! The transparent, topmost window that draws marks and the popover over the
//! host app. It covers the monitor the field is on, never activates on paste
//! or hover, and lets every click through except on what it draws: on Windows
//! its native window region is exactly the drawn shapes, on Linux GTK's input
//! shape clips input to them, and on macOS the pointer is watched against them.
//! Focus moves to it only for the keyboard flow, and goes back to the field
//! when that closes.

use super::locator::Rect;
use super::RectF;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tauri::{window::Color, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "trust";

static WINDOW: AtomicUsize = AtomicUsize::new(0);
static FOCUSED: AtomicBool = AtomicBool::new(false);

pub fn owns(window: usize) -> bool {
    window != 0 && WINDOW.load(Ordering::Acquire) == window
}

pub fn is_focused() -> bool {
    FOCUSED.load(Ordering::Acquire)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub scale: f64,
    pub work: Rect,
}

pub struct Overlay {
    window: tauri::WebviewWindow,
    #[cfg(target_os = "macos")]
    panel: Option<super::mac_panel::Panel>,
    monitor: Option<Monitor>,
    focused: bool,
    #[cfg(not(windows))]
    regions: std::sync::Arc<std::sync::Mutex<Vec<Rect>>>,
}

impl Overlay {
    pub fn open(app: &AppHandle) -> Result<Self, String> {
        if let Some(old) = app.get_webview_window(LABEL) {
            let _ = old.destroy();
        }
        let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?view=trust".into()))
            .title("SonoJot doubtful words")
            .inner_size(16.0, 16.0)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            // macOS: the overlay is never the key window during hover/click, so
            // without this the first click on a mark, choice or close button
            // would only activate SonoJot and never reach the page.
            .accept_first_mouse(true)
            .visible(false);
        let window = crate::windows::build_quiet(app, builder).map_err(|e| e.to_string())?;
        let mut overlay = Self {
            window,
            #[cfg(target_os = "macos")]
            panel: None,
            monitor: None,
            focused: false,
            #[cfg(not(windows))]
            regions: Default::default(),
        };
        overlay.configure()?;
        Ok(overlay)
    }

    #[cfg(windows)]
    fn hwnd(&self) -> usize {
        self.window.hwnd().map(|h| h.0 as usize).unwrap_or(0)
    }

    #[cfg(windows)]
    fn configure(&mut self) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = self.hwnd();
        if hwnd == 0 {
            return Err("no_window".into());
        }
        WINDOW.store(hwnd, Ordering::Release);
        unsafe {
            let h = hwnd as windows_sys::Win32::Foundation::HWND;
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            let popup = (style & !(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX)) | WS_POPUP;
            SetWindowLongPtrW(h, GWL_STYLE, popup as isize);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex as isize);
            SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
        self.apply_region(&[]);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn configure(&mut self) -> Result<(), String> {
        self.panel = Some(super::mac_panel::Panel::open(&self.window, self.regions.clone())?);
        Ok(())
    }

    /// Linux: the window's GTK input shape is set to exactly what the page
    /// draws (see apply_region), so no pointer watching is needed. Toggling
    /// click-through instead resets the input area to the whole window, which
    /// covers the monitor and swallowed every click.
    #[cfg(target_os = "linux")]
    fn configure(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Cover the monitor that holds `anchor`. Returns its geometry.
    pub fn place(&mut self, app: &AppHandle, anchor: Rect) -> Option<Monitor> {
        let (cx, cy) = anchor.center();
        let monitors = app.available_monitors().ok()?;
        let m = monitors
            .iter()
            .find(|m| {
                let p = m.position();
                let s = m.size();
                cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
            })
            .or_else(|| monitors.first())?;
        let work = m.work_area();
        let monitor = Monitor {
            x: m.position().x,
            y: m.position().y,
            w: m.size().width as i32,
            h: m.size().height as i32,
            scale: m.scale_factor(),
            work: Rect { x: work.position.x, y: work.position.y, w: work.size.width as i32, h: work.size.height as i32 },
        };
        if self.monitor != Some(monitor) {
            let _ = self.window.set_size(tauri::PhysicalSize::new(monitor.w as u32, monitor.h as u32));
            // Wayland windows can't place themselves; Hyprland moves them.
            let _ = crate::windows::move_window(&self.window, monitor.x, monitor.y);
            #[cfg(target_os = "macos")]
            if let Some(panel) = &self.panel { panel.place(); }
            #[cfg(not(target_os = "macos"))]
            if self.monitor.is_none() {
                self.reveal();
            }
            self.monitor = Some(monitor);
        }
        self.monitor
    }

    #[cfg(windows)]
    fn reveal(&self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        unsafe { ShowWindow(self.hwnd() as _, SW_SHOWNOACTIVATE) };
    }

    #[cfg(target_os = "linux")]
    fn reveal(&self) {
        // Input only where the page draws, from the first frame on.
        self.apply_region(&self.regions.lock().map(|r| r.clone()).unwrap_or_default());
        let _ = self.window.show();
    }

    pub fn show(&self, view: &super::View) {
        let _ = self.window.emit_to(LABEL, "trust-view", view);
    }

    /// The webview reports what it draws, in its CSS pixels; that becomes the
    /// window's input (and paint) region.
    pub fn set_regions(&mut self, rects: &[RectF]) {
        let Some(m) = self.monitor else { return };
        let physical: Vec<Rect> = rects
            .iter()
            .filter(|r| r.w > 0.0 && r.h > 0.0 && r.w.is_finite() && r.h.is_finite())
            .take(64)
            .map(|r| Rect {
                x: (r.x * m.scale).floor() as i32,
                y: (r.y * m.scale).floor() as i32,
                w: (r.w * m.scale).ceil() as i32 + 1,
                h: (r.h * m.scale).ceil() as i32 + 1,
            })
            .collect();
        self.apply_region(&physical);
    }

    #[cfg(windows)]
    fn apply_region(&self, rects: &[Rect]) {
        use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR};
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            for r in rects {
                let part = CreateRectRgn(r.x, r.y, r.right(), r.bottom());
                CombineRgn(region, region, part, RGN_OR);
                DeleteObject(part);
            }
            // The system owns the region after this call.
            if SetWindowRgn(self.hwnd() as _, region, 1) == 0 {
                DeleteObject(region);
            }
        }
    }

    #[cfg(not(windows))]
    fn apply_region(&self, rects: &[Rect]) {
        if let Ok(mut r) = self.regions.lock() {
            *r = rects.to_vec();
        }
        #[cfg(target_os = "linux")]
        {
            // GTK's input shape is in logical pixels; an empty one lets every
            // click through. Widget-level shapes are kept until the window is
            // realized, so this is safe before the first show.
            let scale = self.monitor.map_or(1.0, |m| m.scale);
            let logical: Vec<(i32, i32, i32, i32)> = rects
                .iter()
                .map(|r| {
                    let x = (f64::from(r.x) / scale).floor() as i32;
                    let y = (f64::from(r.y) / scale).floor() as i32;
                    let right = (f64::from(r.right()) / scale).ceil() as i32;
                    let bottom = (f64::from(r.bottom()) / scale).ceil() as i32;
                    (x, y, right - x, bottom - y)
                })
                .collect();
            let window = self.window.clone();
            let _ = self.window.run_on_main_thread(move || {
                use gtk::prelude::WidgetExt;
                let Ok(gtk_window) = window.gtk_window() else { return };
                let region = gtk::cairo::Region::create();
                for (x, y, w, h) in logical {
                    let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(x, y, w, h));
                }
                gtk_window.input_shape_combine_region(Some(&region));
            });
        }
    }

    /// The keyboard flow: the popover takes focus so arrows and Enter reach it.
    pub fn take_focus(&mut self) {
        self.focused = true;
        FOCUSED.store(true, Ordering::Release);
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel { panel.focus(true); }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let h = self.hwnd() as windows_sys::Win32::Foundation::HWND;
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & !WS_EX_NOACTIVATE;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex as isize);
        }
        // Windows: raw styles only. The window was shown natively, so the
        // window library's own focusable switch would re-apply stale flags
        // and hide it.
        #[cfg(target_os = "linux")]
        let _ = self.window.set_focusable(true);
        #[cfg(not(target_os = "macos"))]
        let _ = self.window.set_focus();
        // Hyprland opens this window without focus (generated rule) and
        // ignores a client's own request, so ask the compositor.
        #[cfg(target_os = "linux")]
        if crate::hyprland::active() {
            let _ = crate::hyprland::focus_own_window("SonoJot doubtful words");
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOWNA};
            ShowWindow(self.hwnd() as _, SW_SHOWNA);
            SetForegroundWindow(self.hwnd() as _);
        }
    }

    pub fn release_focus(&mut self) {
        if !self.focused {
            return;
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let h = self.hwnd() as windows_sys::Win32::Foundation::HWND;
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 | WS_EX_NOACTIVATE;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex as isize);
        }
        #[cfg(target_os = "linux")]
        let _ = self.window.set_focusable(false);
        #[cfg(target_os = "macos")]
        if let Some(panel) = &self.panel { panel.focus(false); }
        self.focused = false;
        FOCUSED.store(false, Ordering::Release);
    }

    pub fn close(self) {}
}

impl Drop for Overlay {
    fn drop(&mut self) {
        WINDOW.store(0, Ordering::Release);
        FOCUSED.store(false, Ordering::Release);
        #[cfg(target_os = "macos")]
        drop(self.panel.take());
        let _ = self.window.destroy();
    }
}
