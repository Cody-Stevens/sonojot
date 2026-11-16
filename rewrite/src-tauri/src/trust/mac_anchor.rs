//! macOS: where the fallback chip should sit. The text caret of the focused
//! field through Accessibility when the app exposes it, else the pointer.
//! Only the caret's position is read, never any text.
//!
//! Accessibility and CoreGraphics report points with a top-left origin on the
//! main display. tao places windows in physical pixels, a display's origin
//! times its scale, so a point on a display maps to `point * scale`.

use super::locator::Rect;
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CFRange {
    location: isize,
    length: isize,
}

const AX_VALUE_CGRECT: u32 = 3;
const AX_VALUE_CFRANGE: u32 = 4;

type CFStringRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
    fn AXUIElementCopyAttributeValue(element: *const c_void, attribute: CFStringRef, value: *mut *const c_void) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: *const c_void,
        attribute: CFStringRef,
        parameter: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> *const c_void;
    fn AXValueGetValue(value: *const c_void, kind: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGGetActiveDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

fn cf(text: &'static str) -> core_foundation::string::CFString {
    core_foundation::string::CFString::from_static_string(text)
}

unsafe fn copy(element: *const c_void, attribute: &'static str) -> Option<*const c_void> {
    use core_foundation::base::TCFType;
    let name = cf(attribute);
    let mut value = std::ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef() as _, &mut value) };
    (status == 0 && !value.is_null()).then_some(value)
}

/// The caret of the focused text field, in points: a thin rectangle at the
/// right edge of the character before it. None when Accessibility is off or
/// the app does not expose a caret (Electron and many web views).
fn caret_points() -> Option<(f64, f64, f64)> {
    use core_foundation::base::TCFType;
    if !crate::mac_paste::accessibility_trusted() {
        return None;
    }
    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return None;
        }
        AXUIElementSetMessagingTimeout(system, 0.25);
        let focused = copy(system, "AXFocusedUIElement");
        CFRelease(system);
        let focused = focused?;
        let mut range = CFRange::default();
        let selected = copy(focused, "AXSelectedTextRange");
        let has_range = selected.is_some_and(|value| {
            let ok = AXValueGetValue(value, AX_VALUE_CFRANGE, &mut range as *mut CFRange as *mut c_void);
            CFRelease(value);
            ok
        });
        if !has_range || range.location < 0 {
            CFRelease(focused);
            return None;
        }
        let end = range.location + range.length;
        let (probe, right) = if end > 0 { (CFRange { location: end - 1, length: 1 }, true) } else { (CFRange { location: 0, length: 0 }, false) };
        let parameter = AXValueCreate(AX_VALUE_CFRANGE, &probe as *const CFRange as *const c_void);
        let mut bounds = CGRect::default();
        let mut found = false;
        if !parameter.is_null() {
            let name = cf("AXBoundsForRange");
            let mut value = std::ptr::null();
            if AXUIElementCopyParameterizedAttributeValue(focused, name.as_concrete_TypeRef() as _, parameter, &mut value) == 0 && !value.is_null() {
                found = AXValueGetValue(value, AX_VALUE_CGRECT, &mut bounds as *mut CGRect as *mut c_void);
                CFRelease(value);
            }
            CFRelease(parameter);
        }
        CFRelease(focused);
        let (x, y, w, h) = (bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height);
        let plausible = found && x.is_finite() && y.is_finite() && (4.0..=200.0).contains(&h) && (0.0..=400.0).contains(&w) && !(x == 0.0 && y == 0.0);
        plausible.then_some((if right { x + w } else { x }, y, h))
    }
}

fn pointer_points() -> Option<(f64, f64)> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
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

/// Where the chip goes, in tao's physical pixels, and whether that is the
/// caret (near the text) rather than the pointer.
pub fn anchor() -> Option<(Rect, bool)> {
    let displays = displays();
    if let Some((x, y, h)) = caret_points() {
        if let Some(rect) = super::physical_rect(x, y, h, &displays) {
            return Some((rect, true));
        }
    }
    let (x, y) = pointer_points()?;
    // Just below the pointer, like the dictation ring.
    super::physical_rect(x, y, 24.0, &displays).map(|r| (r, false))
}
