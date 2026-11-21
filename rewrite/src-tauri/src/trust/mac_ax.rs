//! macOS word locator over Accessibility (`AXUIElement`).
//!
//! Finds the focused editable element of the target application, locates the
//! pasted words through `AXValue` and `AXSelectedTextRange`, and reads each
//! word's on-screen rectangle through `AXBoundsForRange` (clipped to any
//! enclosing `AXScrollArea` / `AXWindow`). Marks follow edits elsewhere via
//! the shared [`WordTracker`](super::track::WordTracker).
//!
//! Accessibility and CoreGraphics report points with a top-left origin on the
//! main display; tao places windows in physical pixels (`point * scale` of the
//! holding display). `CFRange` indices are UTF-16 code units.
//!
//! Apps without trustworthy word positions (or when Accessibility permission
//! is off, or in password fields and terminals) fail quietly with `Nothing`.
//!
//! Privacy rule (see locator.rs): `AXValue` is read transiently only to locate
//! the pasted words and notice edits; it is never kept, logged, or sent.

use super::locator::{Capability, Edge, Focus, Frame, Inert, Locator, Place, Rect, Request, Tracked};
use super::track::{char_range_to_utf16, utf16_to_char_offset, WordTracker};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;
use std::time::{Duration, Instant};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct CFRange {
    location: isize,
    length: isize,
}

const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
const AX_VALUE_CGRECT: u32 = 3;
const AX_VALUE_CFRANGE: u32 = 4;

const AX_ERROR_SUCCESS: i32 = 0;
const AX_ERROR_INVALID_UI_ELEMENT: i32 = -25202;

/// Maximum `AXValue` length we will read transiently (256K UTF-16 code units).
const MAX_VALUE_UNITS: usize = 262_144;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        parameter: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        value: *const c_void,
    ) -> i32;
    fn AXUIElementIsAttributeSettable(
        element: *const c_void,
        attribute: CFStringRef,
        settable: *mut u8,
    ) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *const c_void;
    fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFEqual(cf1: *const c_void, cf2: *const c_void) -> u8;
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    static kCFBooleanTrue: *const c_void;
}

fn cf(text: &'static str) -> CFString {
    CFString::from_static_string(text)
}

/// Owned wrapper around `AXUIElementRef`.
struct AxElement(*const c_void);

// AXUIElement references are thread-safe and owned solely by the trust thread.
unsafe impl Send for AxElement {}

impl Clone for AxElement {
    fn clone(&self) -> Self {
        unsafe { Self(CFRetain(self.0)) }
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

impl AxElement {
    fn application(pid: i32) -> Option<Self> {
        let raw = unsafe { AXUIElementCreateApplication(pid) };
        if raw.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(raw, 0.25) };
        Some(Self(raw))
    }

    fn system_wide() -> Option<Self> {
        let raw = unsafe { AXUIElementCreateSystemWide() };
        if raw.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(raw, 0.25) };
        Some(Self(raw))
    }

    fn adopt(raw: *const c_void) -> Option<Self> {
        if raw.is_null() {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(raw, 0.25) };
        Some(Self(raw))
    }

    fn eq(&self, other: &Self) -> bool {
        unsafe { CFEqual(self.0, other.0) != 0 }
    }

    fn pid(&self) -> Option<i32> {
        let mut pid = 0i32;
        let status = unsafe { AXUIElementGetPid(self.0, &mut pid) };
        (status == AX_ERROR_SUCCESS && pid > 0).then_some(pid)
    }

    fn copy_attr(&self, name: &'static str) -> Result<*const c_void, i32> {
        let attr = cf(name);
        let mut value = std::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(self.0, attr.as_concrete_TypeRef(), &mut value) };
        if status == AX_ERROR_SUCCESS && !value.is_null() {
            Ok(value)
        } else {
            Err(status)
        }
    }

    fn copy_element(&self, name: &'static str) -> Option<Self> {
        self.copy_attr(name).ok().and_then(Self::adopt)
    }

    fn copy_string(&self, name: &'static str) -> Option<String> {
        let raw = self.copy_attr(name).ok()?;
        unsafe {
            if CFGetTypeID(raw) != CFStringGetTypeID() {
                CFRelease(raw);
                return None;
            }
            let s = CFString::wrap_under_create_rule(raw as CFStringRef);
            Some(s.to_string())
        }
    }

    fn role_status(&self) -> Result<String, i32> {
        let raw = self.copy_attr("AXRole")?;
        unsafe {
            if CFGetTypeID(raw) != CFStringGetTypeID() {
                CFRelease(raw);
                return Err(-1);
            }
            let s = CFString::wrap_under_create_rule(raw as CFStringRef);
            Ok(s.to_string())
        }
    }

    fn number_of_chars(&self) -> Option<usize> {
        let raw = self.copy_attr("AXNumberOfCharacters").ok()?;
        let n = unsafe {
            let num = core_foundation::number::CFNumber::wrap_under_create_rule(raw as _);
            num.to_i64()
        };
        n.and_then(|v| usize::try_from(v).ok())
    }

    fn selected_range(&self) -> Option<CFRange> {
        let raw = self.copy_attr("AXSelectedTextRange").ok()?;
        let mut range = CFRange::default();
        let ok = unsafe {
            let ok = AXValueGetValue(raw, AX_VALUE_CFRANGE, &mut range as *mut CFRange as *mut c_void) != 0;
            CFRelease(raw);
            ok
        };
        (ok && range.location >= 0 && range.length >= 0).then_some(range)
    }

    fn set_selected_range(&self, location: usize, length: usize) -> bool {
        let range = CFRange { location: location as isize, length: length as isize };
        let val = unsafe { AXValueCreate(AX_VALUE_CFRANGE, &range as *const CFRange as *const c_void) };
        if val.is_null() {
            return false;
        }
        let attr = cf("AXSelectedTextRange");
        let status = unsafe { AXUIElementSetAttributeValue(self.0, attr.as_concrete_TypeRef(), val) };
        unsafe { CFRelease(val) };
        status == AX_ERROR_SUCCESS
    }

    fn is_settable(&self, name: &'static str) -> bool {
        let attr = cf(name);
        let mut settable = 0u8;
        let status = unsafe { AXUIElementIsAttributeSettable(self.0, attr.as_concrete_TypeRef(), &mut settable) };
        status == AX_ERROR_SUCCESS && settable != 0
    }

    fn set_string(&self, name: &'static str, text: &str) -> bool {
        let attr = cf(name);
        let value = CFString::new(text);
        let status = unsafe {
            AXUIElementSetAttributeValue(self.0, attr.as_concrete_TypeRef(), value.as_concrete_TypeRef() as *const c_void)
        };
        status == AX_ERROR_SUCCESS
    }

    fn set_bool_true(&self, name: &'static str) -> bool {
        let attr = cf(name);
        let status = unsafe { AXUIElementSetAttributeValue(self.0, attr.as_concrete_TypeRef(), kCFBooleanTrue) };
        status == AX_ERROR_SUCCESS
    }

    fn position(&self) -> Option<CGPoint> {
        let raw = self.copy_attr("AXPosition").ok()?;
        let mut point = CGPoint::default();
        let ok = unsafe {
            let ok = AXValueGetValue(raw, AX_VALUE_CGPOINT, &mut point as *mut CGPoint as *mut c_void) != 0;
            CFRelease(raw);
            ok
        };
        (ok && point.x.is_finite() && point.y.is_finite()).then_some(point)
    }

    fn size(&self) -> Option<CGSize> {
        let raw = self.copy_attr("AXSize").ok()?;
        let mut size = CGSize::default();
        let ok = unsafe {
            let ok = AXValueGetValue(raw, AX_VALUE_CGSIZE, &mut size as *mut CGSize as *mut c_void) != 0;
            CFRelease(raw);
            ok
        };
        (ok && size.width.is_finite() && size.height.is_finite() && size.width > 0.0 && size.height > 0.0).then_some(size)
    }

    fn rect_points(&self) -> Option<CGRect> {
        let origin = self.position()?;
        let size = self.size()?;
        Some(CGRect { origin, size })
    }

    fn bounds_for_range(&self, location: usize, length: usize) -> Option<CGRect> {
        let range = CFRange { location: location as isize, length: length as isize };
        let param = unsafe { AXValueCreate(AX_VALUE_CFRANGE, &range as *const CFRange as *const c_void) };
        if param.is_null() {
            return None;
        }
        let attr = cf("AXBoundsForRange");
        let mut out = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyParameterizedAttributeValue(self.0, attr.as_concrete_TypeRef(), param, &mut out)
        };
        unsafe { CFRelease(param) };
        if status != AX_ERROR_SUCCESS || out.is_null() {
            return None;
        }
        let mut rect = CGRect::default();
        let ok = unsafe {
            let ok = AXValueGetValue(out, AX_VALUE_CGRECT, &mut rect as *mut CGRect as *mut c_void) != 0;
            CFRelease(out);
            ok
        };
        let plausible = ok
            && rect.origin.x.is_finite()
            && rect.origin.y.is_finite()
            && rect.size.width.is_finite()
            && rect.size.height.is_finite()
            && rect.size.width > 0.0
            && rect.size.height > 0.0
            && !(rect.origin.x == 0.0 && rect.origin.y == 0.0 && rect.size.width <= 1.0 && rect.size.height <= 1.0);
        plausible.then_some(rect)
    }

    fn range_for_position(&self, point: CGPoint) -> Option<CFRange> {
        let param = unsafe { AXValueCreate(AX_VALUE_CGPOINT, &point as *const CGPoint as *const c_void) };
        if param.is_null() {
            return None;
        }
        let attr = cf("AXRangeForPosition");
        let mut out = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyParameterizedAttributeValue(self.0, attr.as_concrete_TypeRef(), param, &mut out)
        };
        unsafe { CFRelease(param) };
        if status != AX_ERROR_SUCCESS || out.is_null() {
            return None;
        }
        let mut range = CFRange::default();
        let ok = unsafe {
            let ok = AXValueGetValue(out, AX_VALUE_CFRANGE, &mut range as *mut CFRange as *mut c_void) != 0;
            CFRelease(out);
            ok
        };
        (ok && range.location >= 0).then_some(range)
    }
}

fn intersect_cgrect(a: CGRect, b: CGRect) -> Option<CGRect> {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let r = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    (r > x && bottom > y).then_some(CGRect {
        origin: CGPoint { x, y },
        size: CGSize { width: r - x, height: bottom - y },
    })
}

fn displays() -> Vec<super::Display> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
        return Vec::new();
    }
    ids[..count as usize]
        .iter()
        .map(|&id| unsafe {
            let b = CGDisplayBounds(id);
            let mode = CGDisplayCopyDisplayMode(id);
            let scale = if mode.is_null() {
                1.0
            } else {
                let points = CGDisplayModeGetWidth(mode) as f64;
                let pixels = CGDisplayModeGetPixelWidth(mode) as f64;
                CGDisplayModeRelease(mode);
                if points > 0.0 { (pixels / points).max(1.0) } else { 1.0 }
            };
            super::Display { x: b.origin.x, y: b.origin.y, w: b.size.width, h: b.size.height, scale }
        })
        .collect()
}

fn is_terminal_bundle(bundle: &str) -> bool {
    matches!(
        bundle,
        "com.apple.Terminal"
            | "com.googlecode.iterm2"
            | "com.mitchellh.ghostty"
            | "org.alacritty"
            | "net.kovidgoyal.kitty"
            | "com.github.wez.wezterm"
            | "dev.warp.Warp-Stable"
            | "co.zeit.hyper"
    )
}

// Stay inside the focused control's ancestry. Never search sibling editors or
// read values from a web page/window merely because it contains the paste.
fn editable_role(role: &str) -> bool {
    matches!(role, "AXTextField" | "AXTextArea" | "AXComboBox")
}

fn focused_chain(focused: AxElement, pid: i32) -> Result<Vec<AxElement>, &'static str> {
    let mut chain = Vec::new();
    let mut current = focused;
    for _ in 0..6 {
        if current.pid() != Some(pid) { break; }
        // Check the entire short chain before reading ANY value. A secure
        // field can expose a non-secure-looking focused child.
        if current.copy_string("AXSubrole").as_deref() == Some("AXSecureTextField") {
            return Err("password_field");
        }
        let role = current.role_status().unwrap_or_default();
        if matches!(role.as_str(), "AXWindow" | "AXApplication" | "AXWebArea") { break; }
        let parent = current.copy_element("AXParent");
        chain.push(current);
        let Some(parent) = parent else { break };
        if chain.iter().any(|el| el.eq(&parent)) { break; }
        current = parent;
    }
    Ok(chain)
}

fn retry_ready<T>(
    mut attempt: impl FnMut() -> Result<T, &'static str>,
    mut wait: impl FnMut() -> bool,
) -> Result<T, &'static str> {
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err("password_field") => return Err("password_field"),
            Err(code) => if !wait() { return Err(code); },
        }
    }
}

pub struct MacAxLocator;

impl Locator for MacAxLocator {
    fn attach(&mut self, request: &Request) -> Result<Box<dyn Tracked>, String> {
        match MacAxField::attach(request) {
            Ok(field) => Ok(Box::new(field)),
            Err(code) => {
                super::log(&format!("mac_ax_none_{code}"), 0);
                Ok(Box::new(Inert))
            }
        }
    }
}

struct MacAxField {
    app: AxElement,
    element: AxElement,
    pid: i32,
    tracker: WordTracker,
}

impl MacAxField {
    fn focused_in(app: &AxElement, pid: i32) -> Option<AxElement> {
        if let Some(el) = app.copy_element("AXFocusedUIElement") {
            if el.pid() == Some(pid) { return Some(el); }
        }
        let sys = AxElement::system_wide()?;
        let el = sys.copy_element("AXFocusedUIElement")?;
        (el.pid() == Some(pid)).then_some(el)
    }

    fn read_text(element: &AxElement) -> Option<Vec<char>> {
        if let Some(count) = element.number_of_chars() {
            if count > MAX_VALUE_UNITS {
                return None;
            }
        }
        let text = element.copy_string("AXValue")?;
        if text.encode_utf16().count() > MAX_VALUE_UNITS {
            return None;
        }
        Some(text.chars().collect())
    }

    fn attach(request: &Request) -> Result<Self, &'static str> {
        if !crate::mac_paste::accessibility_trusted() {
            return Err("not_trusted");
        }
        let pid = request.target.process as i32;
        if pid <= 0 {
            return Err("no_pid");
        }
        if crate::mac_paste::app_bundle_identifier(pid).is_some_and(|b| is_terminal_bundle(&b)) {
            return Err("terminal");
        }
        let app = AxElement::application(pid).ok_or("no_app")?;
        // Chromium's app object must opt in before the first focus/value read.
        // A partial tree can already return AXValue (often empty or stale), so
        // treating only a missing value as the opt-in signal misses Electron.
        app.set_bool_true("AXManualAccessibility");
        let deadline = Instant::now() + Duration::from_millis(1200);
        retry_ready(
            || Self::attach_ready(&app, pid, request),
            || {
                if Instant::now() >= deadline { return false; }
                std::thread::sleep(Duration::from_millis(100));
                Instant::now() < deadline
            },
        )
    }

    fn attach_ready(app: &AxElement, pid: i32, request: &Request) -> Result<Self, &'static str> {
        let focused = Self::focused_in(app, pid).ok_or("no_focused_element")?;
        let chain = focused_chain(focused, pid)?;
        let mut failure = "pasted_text_not_found";
        for element in chain {
            // Chromium may focus a static-text child of the editable field.
            // Read value, selection and bounds from the SAME editable ancestor;
            // their UTF-16 offsets cannot be mixed with a descendant's text.
            if !editable_role(&element.role_status().unwrap_or_default()) { continue; }
            let Some(text) = Self::read_text(&element) else { continue };
            let Some(range) = element.selected_range() else { continue };
            let caret = utf16_to_char_offset(&text, range.location.saturating_add(range.length) as usize);
            let Some(tracker) = WordTracker::attach(&text, request.text, request.spans, caret) else { continue };
            let field = Self { app: app.clone(), element, pid, tracker };
            // AXValue can be ready a few frames before AXBoundsForRange. Retry
            // focus resolution AND geometry instead of permanently going inert.
            if field.positions_trustworthy(&text) { return Ok(field); }
            failure = "untrusted_rects";
        }
        Err(failure)
    }

    /// Visible viewport of the text field in points: the element's own rect
    /// intersected with enclosing scroll areas and its window so scrolled-out
    /// words clip to `Hidden` instead of floating over toolbars.
    fn visible_field_points(&self) -> Option<CGRect> {
        let mut rect = self.element.rect_points()?;
        let mut current = self.element.clone();
        for _ in 0..6 {
            let Some(parent) = current.copy_element("AXParent") else { break };
            if let Ok(role) = parent.role_status() {
                if matches!(role.as_str(), "AXScrollArea" | "AXWindow" | "AXWebArea" | "AXSplitGroup") {
                    if let Some(parent_rect) = parent.rect_points() {
                        rect = intersect_cgrect(rect, parent_rect)?;
                    }
                }
                if role == "AXWindow" {
                    break;
                }
            }
            current = parent;
        }
        Some(rect)
    }

    fn visible_field_rect(&self, displays: &[super::Display]) -> Option<Rect> {
        let r = self.visible_field_points()?;
        super::physical_rect(r.origin.x, r.origin.y, r.size.width, r.size.height, displays)
    }

    fn positions_trustworthy(&self, text: &[char]) -> bool {
        let displays = displays();
        let field_rect = self.visible_field_rect(&displays);
        let mut visible = 0usize;
        for m in &self.tracker.marks {
            let Some(at) = m.at else { return false };
            let (loc, len) = char_range_to_utf16(text, at, at + m.word.len());
            let Some(pt_rect) = self.element.bounds_for_range(loc, len) else { continue };
            let Some(screen_rect) = super::physical_rect(
                pt_rect.origin.x,
                pt_rect.origin.y,
                pt_rect.size.width,
                pt_rect.size.height,
                &displays,
            ) else {
                continue;
            };
            if field_rect.is_some_and(|f| !f.contains_rect(&screen_rect, 4)) {
                continue;
            }
            visible += 1;
            let center = CGPoint {
                x: pt_rect.origin.x + pt_rect.size.width / 2.0,
                y: pt_rect.origin.y + pt_rect.size.height / 2.0,
            };
            let hit = self.element.range_for_position(center).map_or(true, |r| {
                let pos = r.location.max(0) as usize;
                pos >= loc.saturating_sub(1) && pos <= loc + len
            });
            if !super::locator::trustworthy(&[screen_rect], field_rect.as_ref(), hit) {
                return false;
            }
        }
        visible > 0 || self.tracker.marks.is_empty()
    }

    fn focus(&self) -> Focus {
        if !crate::mac_paste::app_running(self.pid) {
            return Focus::Gone;
        }
        if matches!(self.element.role_status(), Err(AX_ERROR_INVALID_UI_ELEMENT)) {
            return Focus::Gone;
        }
        let Some(frontmost) = crate::mac_paste::frontmost_pid() else {
            return Focus::Away;
        };
        if crate::mac_paste::is_own_pid(frontmost) {
            return if super::overlay_focused() { Focus::InField } else { Focus::Away };
        }
        if frontmost != self.pid {
            return Focus::Away;
        }
        match Self::focused_in(&self.app, self.pid) {
            Some(focused) if focused.eq(&self.element) || focused_chain(focused.clone(), self.pid)
                .is_ok_and(|chain| chain.iter().any(|el| el.eq(&self.element))) => Focus::InField,
            _ => Focus::Away,
        }
    }
}

impl Tracked for MacAxField {
    fn capability(&self) -> Capability {
        Capability::Inline
    }

    fn poll(&mut self) -> Frame {
        let focus = self.focus();
        if focus == Focus::Gone {
            return Frame { focus, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        }
        if focus == Focus::Away {
            return Frame { focus, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: false };
        }
        let Some(text) = Self::read_text(&self.element) else {
            return Frame { focus: Focus::Gone, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        };
        self.tracker.follow(&text);
        let displays = displays();
        let field = self.visible_field_rect(&displays);
        let marks: Vec<Place> = self
            .tracker
            .marks
            .iter()
            .map(|m| {
                if m.resolved {
                    return Place::Hidden;
                }
                let Some(at) = m.at else { return Place::Changed };
                let (loc, len) = char_range_to_utf16(&text, at, at + m.word.len());
                let rect = self.element.bounds_for_range(loc, len).and_then(|pt| {
                    super::physical_rect(pt.origin.x, pt.origin.y, pt.size.width, pt.size.height, &displays)
                });
                match rect {
                    Some(r) if r.w > 0 && r.h > 0 && field.is_none_or(|f| f.contains_rect(&r, 2)) => Place::Visible(vec![r]),
                    _ => Place::Hidden,
                }
            })
            .collect();
        let retyped: Vec<Option<String>> = self.tracker.marks.iter().map(|m| m.retyped.clone()).collect();
        let text_gone = text.is_empty() || self.tracker.all_unresolved_gone();
        Frame { focus, marks, retyped, field, text_gone }
    }

    fn replace(&mut self, index: usize, with: &str) -> Result<(), String> {
        let text = Self::read_text(&self.element).ok_or("The field can no longer be read.")?;
        self.tracker.follow(&text);
        let mark = self.tracker.marks.get(index).ok_or("unknown_mark")?;
        let at = mark.at.ok_or("The word changed since it was pasted.")?;
        let (char_start, char_end) = match mark.edge {
            Edge::Word => (at, at + mark.word.len()),
            Edge::After => (at + mark.word.len(), at + mark.word.len()),
            Edge::Before => (at, at),
        };
        let (loc, len) = char_range_to_utf16(&text, char_start, char_end);
        let saved_caret = self.element.selected_range();

        if !self.refocus() {
            return Err("The app is not in front.".into());
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            if crate::mac_paste::frontmost_pid() == Some(self.pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }

        if !self.element.set_selected_range(loc, len) {
            return Err("The app did not let SonoJot select the word.".into());
        }

        // Direct AX write avoids touching the clipboard and integrates with
        // native Cocoa undo. Verify the text actually changed before trusting it.
        if self.element.is_settable("AXSelectedText") && self.element.set_string("AXSelectedText", with) {
            std::thread::sleep(Duration::from_millis(25));
            let expected_chars: Vec<char> = with.chars().collect();
            if let Some(updated) = Self::read_text(&self.element) {
                if updated.get(char_start..char_start + expected_chars.len()) == Some(&expected_chars[..]) {
                    if let Some(caret) = saved_caret {
                        let new_len = with.encode_utf16().count() as isize;
                        let delta = new_len - len as isize;
                        let end = (loc + len) as isize;
                        let restored = if caret.location >= end {
                            (caret.location + delta).max(0) as usize
                        } else if caret.location >= loc as isize {
                            (loc as isize + new_len).max(0) as usize
                        } else {
                            caret.location.max(0) as usize
                        };
                        let _ = self.element.set_selected_range(restored, 0);
                    }
                    self.tracker.resolve(index);
                    return Ok(());
                }
            }
        }

        // Fallback for Chromium/Electron apps where AXSelectedText is read-only:
        // paste over the selected range through SonoJot's guarded delivery path.
        std::thread::sleep(Duration::from_millis(40));
        let target = crate::platform::capture_paste_target().ok_or("The app is not in front.")?;
        if crate::platform::trust_target(&target).process != self.pid as u32 {
            return Err("The app is not in front.".into());
        }
        let outcome = tauri::async_runtime::block_on(crate::platform::deliver_text(with, Some(target), true))?;
        if outcome.paste_sent {
            self.tracker.resolve(index);
            Ok(())
        } else {
            Err(outcome.reason.unwrap_or_else(|| "The fix was copied but not pasted.".into()))
        }
    }

    fn resolve(&mut self, index: usize) {
        self.tracker.resolve(index);
    }

    fn refocus(&mut self) -> bool {
        if crate::mac_paste::frontmost_pid() == Some(self.pid) {
            return true;
        }
        crate::mac_paste::activate(self.pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_mac_retries_partial_tree_value_and_bounds() {
        let mut responses = [
            Err("no_focused_element"),
            Err("pasted_text_not_found"),
            Err("untrusted_rects"),
            Ok("ready"),
        ].into_iter();
        let mut waits = 0;
        let result = retry_ready(|| responses.next().unwrap(), || { waits += 1; true });
        assert_eq!(result, Ok("ready"));
        assert_eq!(waits, 3);
    }

    #[test]
    fn trust_mac_stops_at_deadline_and_never_retries_password_fields() {
        let mut remaining = 2;
        let result: Result<(), _> = retry_ready(|| Err("untrusted_rects"), || {
            remaining -= 1;
            remaining > 0
        });
        assert_eq!(result, Err("untrusted_rects"));
        let result: Result<(), _> = retry_ready(|| Err("password_field"), || panic!("must not wait"));
        assert_eq!(result, Err("password_field"));
    }

    #[test]
    fn trust_mac_only_reads_editable_fields() {
        for role in ["AXTextField", "AXTextArea", "AXComboBox"] { assert!(editable_role(role)); }
        for role in ["AXWebArea", "AXWindow", "AXStaticText", "AXApplication", ""] { assert!(!editable_role(role)); }
    }
}
