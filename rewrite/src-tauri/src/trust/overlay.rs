//! The transparent, topmost window that draws marks, the popover and the
//! fallback chip over the host app. It covers the monitor the field is on,
//! never activates on paste or hover, and lets every click through except on
//! what it draws: on Windows its native window region is exactly the drawn
//! shapes, so the pointer needs no polling. Focus moves to it only for the
//! keyboard flow, and goes back to the field when that closes.

use super::locator::Rect;
use super::RectF;
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::{window::Color, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "trust";

static WINDOW: AtomicUsize = AtomicUsize::new(0);

pub fn owns(window: usize) -> bool {
    window != 0 && WINDOW.load(Ordering::Acquire) == window
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
            .visible(false);
        let window = crate::windows::build_quiet(app, builder).map_err(|e| e.to_string())?;
        let overlay = Self {
            window,
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
    fn configure(&self) -> Result<(), String> {
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

    #[cfg(not(windows))]
    fn configure(&self) -> Result<(), String> {
        // Without native window regions, the pointer is watched instead.
        let window = self.window.clone();
        let regions = self.regions.clone();
        tauri::async_runtime::spawn(async move {
            let mut through = None;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(33)).await;
                let Ok(position) = window.outer_position() else { break };
                let inside = window.cursor_position().ok().is_some_and(|p| {
                    let (x, y) = (p.x as i32 - position.x, p.y as i32 - position.y);
                    regions.lock().map(|r| r.iter().any(|r| x >= r.x && y >= r.y && x < r.right() && y < r.bottom())).unwrap_or(false)
                });
                if through != Some(!inside) {
                    through = Some(!inside);
                    if window.set_ignore_cursor_events(!inside).is_err() {
                        break;
                    }
                }
            }
        });
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
            let _ = self.window.set_position(tauri::PhysicalPosition::new(monitor.x, monitor.y));
            let _ = self.window.set_size(tauri::PhysicalSize::new(monitor.w as u32, monitor.h as u32));
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

    #[cfg(not(windows))]
    fn reveal(&self) {
        let _ = self.window.set_ignore_cursor_events(true);
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
    }

    /// The keyboard flow: the popover takes focus so arrows and Enter reach it.
    pub fn take_focus(&mut self) {
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
        #[cfg(not(windows))]
        let _ = self.window.set_focusable(true);
        let _ = self.window.set_focus();
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOWNA};
            ShowWindow(self.hwnd() as _, SW_SHOWNA);
            SetForegroundWindow(self.hwnd() as _);
        }
        self.focused = true;
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
        #[cfg(not(windows))]
        let _ = self.window.set_focusable(false);
        self.focused = false;
    }

    pub fn close(self) {}
}

impl Drop for Overlay {
    fn drop(&mut self) {
        WINDOW.store(0, Ordering::Release);
        let _ = self.window.destroy();
    }
}
