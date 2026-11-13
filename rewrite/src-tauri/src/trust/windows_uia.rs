//! Windows locator: UI Automation text ranges for most apps, Win32 Edit
//! messages for classic edit controls (Notepad), and a retype path for
//! terminals. Proven in the November 13 spike; see FEASIBILITY.md.
//!
//! Privacy: this module asks the app for (1) the location of the exact text
//! SonoJot pasted, by search, (2) the text of each marked word, to notice
//! edits, and (3) rectangles. It never reads the rest of the field.

use super::locator::{Capability, Edge, Focus, Frame, Locator, Place, Rect, Request, Span, Target, Tracked};
use std::time::{Duration, Instant};
use windows::core::{Interface, BOOL, BSTR};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, SAFEARRAY};
use windows::Win32::UI::Accessibility::*;
use windows_sys::Win32::UI::WindowsAndMessaging as wm;

pub struct UiaLocator {
    automation: Option<IUIAutomation>,
}

impl UiaLocator {
    pub fn new() -> Self {
        // The trust thread is a plain worker; MTA lets COM calls block safely.
        let automation = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .or_else(|_| CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
                .ok()
        };
        Self { automation }
    }
}

fn hwnd(value: usize) -> HWND {
    HWND(value as *mut core::ffi::c_void)
}

fn sys(value: usize) -> windows_sys::Win32::Foundation::HWND {
    value as windows_sys::Win32::Foundation::HWND
}

fn root_of(window: usize) -> usize {
    unsafe { wm::GetAncestor(sys(window), wm::GA_ROOT) as usize }
}

fn foreground() -> usize {
    unsafe { root_of(wm::GetForegroundWindow() as usize) }
}

fn window_alive(window: usize) -> bool {
    window != 0 && unsafe { wm::IsWindow(sys(window)) != 0 }
}

fn class_name(window: usize) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { wm::GetClassNameW(sys(window), buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// A process we cannot open for a token query runs at a higher integrity
/// level (an elevated app): UIPI would also block typing into it.
fn elevated_or_inaccessible(process: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let elevated = |process_handle| {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(process_handle, TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenElevation, &mut info as *mut _ as *mut _, std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len);
            CloseHandle(token);
            (ok != 0).then_some(info.TokenIsElevated != 0)
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process);
        if handle.is_null() {
            return true;
        }
        let theirs = elevated(handle);
        CloseHandle(handle);
        let ours = elevated(GetCurrentProcess()).unwrap_or(false);
        match theirs {
            None => true,
            Some(t) => t && !ours,
        }
    }
}

fn is_terminal(class: &str) -> bool {
    matches!(class, "ConsoleWindowClass" | "CASCADIA_HOSTING_WINDOW_CLASS" | "PseudoConsoleWindow" | "mintty" | "VirtualConsoleClass")
}

fn rects_of(range: &IUIAutomationTextRange) -> Vec<Rect> {
    unsafe {
        let Ok(array) = range.GetBoundingRectangles() else { return Vec::new() };
        if array.is_null() {
            return Vec::new();
        }
        let sa: &SAFEARRAY = &*array;
        let count = sa.rgsabound[0].cElements as usize;
        let mut out = Vec::new();
        if !sa.pvData.is_null() && sa.cbElements == 8 {
            let values = std::slice::from_raw_parts(sa.pvData as *const f64, count);
            for c in values.chunks_exact(4) {
                out.push(Rect { x: c[0].round() as i32, y: c[1].round() as i32, w: c[2].round() as i32, h: c[3].round() as i32 });
            }
        }
        let _ = windows::Win32::System::Ole::SafeArrayDestroy(array);
        out
    }
}

fn text_of(range: &IUIAutomationTextRange, max: i32) -> Option<String> {
    unsafe { range.GetText(max).ok().map(|b| b.to_string()) }
}

fn to_rect(r: RECT) -> Rect {
    Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top }
}

/// The GUI caret of a window's thread, in screen pixels.
fn gui_caret(window: usize) -> Option<Rect> {
    use windows_sys::Win32::Foundation::POINT as SPOINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    unsafe {
        let thread = wm::GetWindowThreadProcessId(sys(window), std::ptr::null_mut());
        let mut info = wm::GUITHREADINFO { cbSize: std::mem::size_of::<wm::GUITHREADINFO>() as u32, ..std::mem::zeroed() };
        if wm::GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndCaret.is_null() {
            return None;
        }
        let mut p = SPOINT { x: info.rcCaret.left, y: info.rcCaret.top };
        if ClientToScreen(info.hwndCaret, &mut p) == 0 {
            return None;
        }
        Some(Rect { x: p.x, y: p.y, w: (info.rcCaret.right - info.rcCaret.left).max(1), h: (info.rcCaret.bottom - info.rcCaret.top).max(8) })
    }
}

fn window_rect(window: usize) -> Option<Rect> {
    unsafe {
        let mut r = std::mem::zeroed();
        (wm::GetWindowRect(sys(window), &mut r) != 0).then(|| Rect { x: r.left, y: r.top, w: r.right - r.left, h: r.bottom - r.top })
    }
}

fn modifiers_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].iter().any(|k| unsafe { GetAsyncKeyState(*k as i32) } as u16 & 0x8000 != 0)
}

fn wait_for_modifiers() -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(1200);
    while modifiers_down() {
        if Instant::now() > deadline {
            return Err("Let go of the modifier keys, then try again.".into());
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Ok(())
}

/// Type text into the foreground window as Unicode key events. The app's own
/// undo history records it like typing.
pub(crate) fn type_text(text: &str, backspaces: usize) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let key = |vk: u16, scan: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let mut inputs = Vec::new();
    for _ in 0..backspaces {
        inputs.push(key(VK_BACK, 0, 0));
        inputs.push(key(VK_BACK, 0, KEYEVENTF_KEYUP));
    }
    for unit in text.encode_utf16() {
        inputs.push(key(0, unit, KEYEVENTF_UNICODE));
        inputs.push(key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        return Err("Windows blocked the typed fix.".into());
    }
    Ok(())
}

/// Bring the host window back to the front (after the overlay held focus).
fn bring_back(window: usize) -> bool {
    unsafe {
        if foreground() == window {
            return true;
        }
        wm::SetForegroundWindow(sys(window));
        let deadline = Instant::now() + Duration::from_millis(400);
        while Instant::now() < deadline {
            if foreground() == window {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

impl Locator for UiaLocator {
    fn attach(&mut self, request: &Request) -> Result<Box<dyn Tracked>, String> {
        let target = request.target.clone();
        if !window_alive(target.window) {
            return Err("target_window_gone".into());
        }
        if elevated_or_inaccessible(target.process) {
            return Ok(Box::new(Inert(Capability::Nothing)));
        }
        let focused_class = class_name(target.focused);
        if focused_class.eq_ignore_ascii_case("Edit") {
            if let Some(edit) = EditField::attach(request, &target) {
                return Ok(Box::new(edit));
            }
        }
        let terminal = is_terminal(&class_name(root_of(target.window))) || is_terminal(&focused_class);
        let Some(automation) = self.automation.clone() else {
            return Ok(Box::new(Chip::new(target, Capability::FallbackReadOnly)));
        };
        // Chromium and Electron build their accessibility tree on the first
        // query; ask again for up to a second before giving up on positions.
        let deadline = Instant::now() + Duration::from_millis(1200);
        loop {
            match UiaField::attach(&automation, request, &target, terminal) {
                Ok(field) => return Ok(Box::new(field)),
                Err(AttachError::Password) => return Ok(Box::new(Inert(Capability::Nothing))),
                Err(AttachError::Retry(_)) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(150)),
                Err(AttachError::Retry(_)) | Err(AttachError::NoText) => {
                    return Ok(Box::new(Chip::new(target, Capability::FallbackReadOnly)));
                }
            }
        }
    }
}

enum AttachError {
    Password,
    NoText,
    Retry(&'static str),
}

/// Nothing to show (password, elevated) or nothing to track.
struct Inert(Capability);

impl Tracked for Inert {
    fn capability(&self) -> Capability {
        self.0
    }
    fn poll(&mut self) -> Frame {
        Frame { focus: Focus::Gone, marks: Vec::new(), field: None, anchor: None, text_gone: true }
    }
    fn replace(&mut self, _: usize, _: &str) -> Result<(), String> {
        Err("unsupported".into())
    }
    fn refocus(&mut self) -> bool {
        false
    }
}

/// An app SonoJot cannot read positions from: a chip by the caret or window.
struct Chip {
    target: Target,
    capability: Capability,
}

impl Chip {
    fn new(target: Target, capability: Capability) -> Self {
        Self { target, capability }
    }
}

impl Tracked for Chip {
    fn capability(&self) -> Capability {
        self.capability
    }
    fn poll(&mut self) -> Frame {
        if !window_alive(self.target.window) {
            return Frame { focus: Focus::Gone, marks: Vec::new(), field: None, anchor: None, text_gone: true };
        }
        let fg = foreground();
        let focus = if fg == self.target.window || super::overlay_owns(fg) { Focus::InField } else { Focus::Away };
        let field = window_rect(self.target.window);
        Frame { focus, marks: Vec::new(), field, anchor: gui_caret(self.target.window).or(field), text_gone: false }
    }
    fn replace(&mut self, _: usize, _: &str) -> Result<(), String> {
        Err("This app does not let SonoJot change its text.".into())
    }
    fn refocus(&mut self) -> bool {
        bring_back(self.target.window)
    }
}

struct Located {
    span: Span,
    word: String,
    range: Option<IUIAutomationTextRange>,
}

struct UiaField {
    automation: IUIAutomation,
    target: Target,
    /// The focused element SonoJot pasted into.
    element: IUIAutomationElement,
    pattern: IUIAutomationTextPattern,
    marks: Vec<Located>,
    /// The end of the pasted text, for the caret-at-end check.
    end: Option<IUIAutomationTextRange>,
    capability: Capability,
    terminal: bool,
    /// Pasted text from each mark's start to the end, for retyping.
    text: String,
}

// The trust thread owns these COM pointers alone (MTA).
unsafe impl Send for UiaField {}

fn text_pattern_of(element: &IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok() }
}

impl UiaField {
    fn attach(automation: &IUIAutomation, request: &Request, target: &Target, terminal: bool) -> Result<Self, AttachError> {
        unsafe {
            let element = automation
                .GetFocusedElement()
                .ok()
                .filter(|e| e.CurrentProcessId().ok() == Some(target.process as i32))
                .or_else(|| automation.ElementFromHandle(hwnd(target.focused)).ok())
                .ok_or(AttachError::Retry("no_element"))?;
            if element.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) {
                return Err(AttachError::Password);
            }
            // The editing host may sit inside a document that owns the pattern.
            let walker = automation.ControlViewWalker().map_err(|_| AttachError::NoText)?;
            let mut holder = Some(element.clone());
            let mut pattern = None;
            for _ in 0..6 {
                let Some(e) = holder.take() else { break };
                if let Some(p) = text_pattern_of(&e) {
                    pattern = Some(p);
                    break;
                }
                holder = walker.GetParentElement(&e).ok();
            }
            let pattern = pattern.ok_or(AttachError::Retry("no_text_pattern"))?;
            let document = pattern.DocumentRange().map_err(|_| AttachError::Retry("no_document"))?;
            // Search only before the caret: the paste ends there.
            let search = document.Clone().map_err(|_| AttachError::NoText)?;
            if let Some(caret) = caret_range(&pattern) {
                let _ = search.MoveEndpointByRange(TextPatternRangeEndpoint_End, &caret, TextPatternRangeEndpoint_Start);
            }
            let text = request.text;
            let mut marks = Vec::new();
            let mut found_any = false;
            for span in request.spans {
                let word = text[span.start..span.end].to_owned();
                let range = find_word(&search, text, span).or_else(|| find_word(&document, text, span));
                found_any |= range.is_some();
                marks.push(Located { span: span.clone(), word, range });
            }
            if !found_any && !request.spans.is_empty() {
                return Err(AttachError::Retry("text_not_found"));
            }
            // The last line of the paste gives the end anchor for retyping.
            let last_line = text.trim_end().rsplit('\n').next().unwrap_or("").trim();
            let end = (!last_line.is_empty())
                .then(|| search.FindText(&BSTR::from(last_line), true, false).ok())
                .flatten();
            let mut field = UiaField {
                automation: automation.clone(),
                target: target.clone(),
                element,
                pattern,
                marks,
                end,
                capability: Capability::Inline,
                terminal,
                text: text.to_owned(),
            };
            field.capability = field.classify();
            Ok(field)
        }
    }

    /// Inline only when every visible mark passes the mismatch check.
    fn classify(&self) -> Capability {
        if self.terminal {
            return Capability::FallbackRetype;
        }
        let field = self.field_rect();
        let mut visible = 0;
        for m in &self.marks {
            let Some(range) = &m.range else { return Capability::FallbackReplace };
            let rects = rects_of(range);
            if rects.is_empty() {
                continue;
            }
            visible += 1;
            let hit = self.hit_matches(range, &rects[0]);
            if !super::locator::trustworthy(&rects, field.as_ref(), hit) {
                return Capability::FallbackReplace;
            }
        }
        if visible == 0 && !self.marks.is_empty() {
            return Capability::FallbackReplace;
        }
        Capability::Inline
    }

    /// The insertion point at the left edge of a word's first rectangle must
    /// be the word's own start (at most one character in). Providers that do
    /// not answer point queries are trusted on their rectangles alone.
    fn hit_matches(&self, range: &IUIAutomationTextRange, first: &Rect) -> bool {
        unsafe {
            let point = POINT { x: first.x + 2, y: first.y + first.h / 2 };
            let Ok(hit) = self.pattern.RangeFromPoint(point) else { return true };
            let before = hit.CompareEndpoints(TextPatternRangeEndpoint_Start, range, TextPatternRangeEndpoint_Start).unwrap_or(0);
            if before < 0 {
                return false;
            }
            let Ok(lead) = range.Clone() else { return true };
            if lead.MoveEndpointByRange(TextPatternRangeEndpoint_End, &hit, TextPatternRangeEndpoint_Start).is_err() {
                return true;
            }
            // Only characters of the pasted word itself are read here.
            text_of(&lead, 8).is_some_and(|t| t.chars().count() <= 1)
        }
    }

    fn field_rect(&self) -> Option<Rect> {
        unsafe { self.element.CurrentBoundingRectangle().ok().map(to_rect).filter(|r| r.w > 0 && r.h > 0) }
    }

    fn focus(&self) -> Focus {
        if !window_alive(self.target.window) {
            return Focus::Gone;
        }
        let fg = foreground();
        if super::overlay_owns(fg) {
            return Focus::InField;
        }
        if fg != self.target.window {
            return Focus::Away;
        }
        unsafe {
            match self.automation.GetFocusedElement() {
                Ok(focused) => match self.automation.CompareElements(&focused, &self.element) {
                    Ok(same) if same.as_bool() => Focus::InField,
                    _ => Focus::Away,
                },
                Err(_) => Focus::Away,
            }
        }
    }

    fn caret_at_end(&self) -> bool {
        let (Some(end), Some(caret)) = (&self.end, caret_range(&self.pattern)) else { return false };
        unsafe { caret.CompareEndpoints(TextPatternRangeEndpoint_Start, end, TextPatternRangeEndpoint_End).ok() == Some(0) }
    }
}

fn caret_range(pattern: &IUIAutomationTextPattern) -> Option<IUIAutomationTextRange> {
    unsafe {
        if let Ok(p2) = pattern.cast::<IUIAutomationTextPattern2>() {
            let mut active = BOOL(0);
            if let Ok(r) = p2.GetCaretRange(&mut active) {
                return Some(r);
            }
        }
        let selection = pattern.GetSelection().ok()?;
        (selection.Length().ok()? > 0).then(|| selection.GetElement(0).ok()).flatten()
    }
}

/// Find a word through a unique needle of its own line, searching backward so
/// the occurrence nearest the caret (the paste) wins.
fn find_word(search: &IUIAutomationTextRange, text: &str, span: &Span) -> Option<IUIAutomationTextRange> {
    let (a, b) = super::locator::needle(text, span.start, span.end);
    let needle = &text[a..b];
    let word = &text[span.start..span.end];
    unsafe {
        let outer = search.FindText(&BSTR::from(needle), true, false).ok()?;
        // Which occurrence of the word inside the needle is ours.
        let nth = text[a..span.start].matches(word).count();
        let scope = outer.Clone().ok()?;
        for _ in 0..nth {
            let hit = scope.FindText(&BSTR::from(word), false, false).ok()?;
            scope.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &hit, TextPatternRangeEndpoint_End).ok()?;
        }
        let found = scope.FindText(&BSTR::from(word), false, false).ok()?;
        (text_of(&found, word.len() as i32 + 4)?.trim() == word).then_some(found)
    }
}

impl Tracked for UiaField {
    fn capability(&self) -> Capability {
        self.capability
    }

    fn poll(&mut self) -> Frame {
        let focus = self.focus();
        if focus == Focus::Gone {
            return Frame { focus, marks: Vec::new(), field: None, anchor: None, text_gone: true };
        }
        let field = self.field_rect();
        let mut changed = 0;
        let marks: Vec<Place> = self
            .marks
            .iter()
            .map(|m| {
                let Some(range) = &m.range else { return Place::Hidden };
                let now = text_of(range, m.word.len() as i32 + 4).unwrap_or_default();
                if now.trim() != m.word {
                    changed += 1;
                    return Place::Changed;
                }
                if !self.capability.inline() {
                    return Place::Hidden;
                }
                let rects: Vec<Rect> = rects_of(range).into_iter().filter(|r| field.is_none_or(|f| f.contains_rect(r, 2))).collect();
                if rects.is_empty() { Place::Hidden } else { Place::Visible(rects) }
            })
            .collect();
        let text_gone = !self.marks.is_empty() && changed == self.marks.len();
        let anchor = self
            .end
            .as_ref()
            .and_then(|r| rects_of(r).last().copied())
            .map(|r| Rect { x: r.right(), y: r.y, w: 1, h: r.h })
            .filter(|r| field.is_none_or(|f| f.contains_rect(r, 4)))
            .or_else(|| gui_caret(self.target.window))
            .or(field);
        Frame { focus, marks, field, anchor, text_gone }
    }

    fn replace(&mut self, index: usize, with: &str) -> Result<(), String> {
        let Some(m) = self.marks.get(index) else { return Err("unknown_mark".into()) };
        let (span, word) = (m.span.clone(), m.word.clone());
        let range = m.range.clone().ok_or("The word is no longer where SonoJot pasted it.")?;
        if text_of(&range, word.len() as i32 + 4).unwrap_or_default().trim() != word {
            return Err("The word changed since it was pasted.".into());
        }
        if !self.refocus() {
            return Err("The app is not in front.".into());
        }
        wait_for_modifiers()?;
        let edge = span.edge;
        if self.terminal {
            // Delete back and retype the tail, only while the caret is at the end.
            if !self.caret_at_end() {
                return Err("Move the cursor back to the end of the dictation first.".into());
            }
            let (from, keep) = match edge {
                Edge::Word => (span.start, span.end),
                Edge::After => (span.end, span.end),
                Edge::Before => (span.start, span.start),
            };
            let tail = &self.text[from..];
            let new_tail = format!("{with}{}", &self.text[keep..]);
            return type_text(&new_tail, tail.chars().count());
        }
        unsafe {
            let caret = caret_range(&self.pattern).and_then(|c| c.Clone().ok());
            let target = range.Clone().map_err(|e| e.to_string())?;
            // A gap is an empty range at the edge of its neighbouring word.
            match edge {
                Edge::Word => {}
                Edge::After => target.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &range, TextPatternRangeEndpoint_End).map_err(|e| e.to_string())?,
                Edge::Before => target.MoveEndpointByRange(TextPatternRangeEndpoint_End, &range, TextPatternRangeEndpoint_Start).map_err(|e| e.to_string())?,
            }
            let _ = self.element.SetFocus();
            target.Select().map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(30));
            type_text(with, 0)?;
            std::thread::sleep(Duration::from_millis(40));
            if let Some(caret) = caret {
                let _ = caret.Select();
            }
        }
        Ok(())
    }

    fn refocus(&mut self) -> bool {
        if !bring_back(self.target.window) {
            return false;
        }
        unsafe {
            let _ = self.element.SetFocus();
        }
        true
    }
}

/// A classic Win32 Edit control (Notepad on Windows 10). It has no UIA text
/// pattern, but answers position messages. SonoJot never reads its text:
/// the paste ends at the caret, so offsets follow from the pasted length,
/// and edits are noticed from the text length and caret alone.
struct EditField {
    target: Target,
    edit: usize,
    /// UTF-16 offsets of each span, and of the whole paste.
    spans: Vec<(usize, usize)>,
    edges: Vec<Edge>,
    start: usize,
    end: usize,
    length: usize,
    changed: Vec<bool>,
}

const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EM_LINEFROMCHAR: u32 = 0x00C9;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_POSFROMCHAR: u32 = 0x00D6;
const WM_GETTEXTLENGTH: u32 = 0x000E;

fn send(window: usize, msg: u32, w: usize, l: isize) -> isize {
    unsafe { wm::SendMessageW(sys(window), msg, w, l) }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl EditField {
    fn attach(request: &Request, target: &Target) -> Option<Self> {
        let edit = target.focused;
        let length = send(edit, WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
        let mut sel_start = 0u32;
        let mut sel_end = 0u32;
        send(edit, EM_GETSEL, &mut sel_start as *mut u32 as usize, &mut sel_end as *mut u32 as isize);
        // Edit controls store line breaks as CR LF.
        let stored = request.text.replace("\r\n", "\n").replace('\n', "\r\n");
        let pasted = utf16_len(&stored);
        let end = sel_end as usize;
        if sel_start != sel_end || end < pasted || end > length {
            return None;
        }
        let start = end - pasted;
        // Structure check without reading: the line count must agree.
        let lines = |i: usize| send(edit, EM_LINEFROMCHAR, i, 0);
        if lines(end) - lines(start) != stored.matches('\n').count() as isize {
            return None;
        }
        let offset = |byte: usize| {
            let before = &request.text[..byte];
            utf16_len(&before.replace("\r\n", "\n").replace('\n', "\r\n"))
        };
        let spans = request.spans.iter().map(|s| (start + offset(s.start), start + offset(s.end))).collect::<Vec<_>>();
        let edges = request.spans.iter().map(|s| s.edge).collect();
        Some(Self { target: target.clone(), edit, changed: vec![false; spans.len()], spans, edges, start, end, length })
    }

    fn client_to_screen(&self, x: i32, y: i32) -> (i32, i32) {
        use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
        let mut p = windows_sys::Win32::Foundation::POINT { x, y };
        unsafe { ClientToScreen(sys(self.edit), &mut p) };
        (p.x, p.y)
    }

    fn pos(&self, index: usize) -> Option<(i32, i32)> {
        let r = send(self.edit, EM_POSFROMCHAR, index, 0);
        if r == -1 {
            return None;
        }
        Some(((r & 0xFFFF) as i16 as i32, ((r >> 16) & 0xFFFF) as i16 as i32))
    }

    fn line_height(&self, index: usize) -> i32 {
        let line = send(self.edit, EM_LINEFROMCHAR, index, 0);
        let next = send(self.edit, EM_LINEINDEX, (line + 1) as usize, 0);
        let prev = if line > 0 { send(self.edit, EM_LINEINDEX, (line - 1) as usize, 0) } else { -1 };
        let y = self.pos(index).map(|p| p.1).unwrap_or(0);
        let from = |i: isize| (i >= 0).then(|| self.pos(i as usize)).flatten().map(|p| (p.1 - y).abs()).filter(|h| *h > 4);
        from(next).or_else(|| from(prev)).unwrap_or(16)
    }

    fn client(&self) -> Option<Rect> {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
        unsafe {
            let mut r = std::mem::zeroed();
            if GetClientRect(sys(self.edit), &mut r) == 0 {
                return None;
            }
            let (x, y) = self.client_to_screen(r.left, r.top);
            Some(Rect { x, y, w: r.right - r.left, h: r.bottom - r.top })
        }
    }

    fn rects(&self, a: usize, b: usize) -> Vec<Rect> {
        let Some((x0, y0)) = self.pos(a) else { return Vec::new() };
        let last = b.saturating_sub(1).max(a);
        let Some((x1, y1)) = self.pos(last) else { return Vec::new() };
        let h = self.line_height(a);
        let right = match self.pos(b) {
            Some((x, y)) if y == y1 && x > x1 => x,
            _ => x1 + ((x1 - x0) / (last - a).max(1) as i32).max(6),
        };
        if y1 != y0 {
            return Vec::new();
        }
        let (sx, sy) = self.client_to_screen(x0, y0);
        vec![Rect { x: sx, y: sy, w: (right - x0).max(1), h }]
    }

    /// Follow typing elsewhere: text inserted before the paste shifts it, after
    /// it changes nothing, inside it marks the words it touched as changed.
    fn follow(&mut self) {
        let length = send(self.edit, WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
        if length == self.length {
            return;
        }
        let delta = length as isize - self.length as isize;
        self.length = length;
        let mut caret = 0u32;
        send(self.edit, EM_GETSEL, 0, &mut caret as *mut u32 as isize);
        let at = (caret as isize - delta.max(0)).max(0) as usize;
        let shift = |v: &mut usize| *v = (*v as isize + delta).max(0) as usize;
        if at <= self.start {
            shift(&mut self.start);
            shift(&mut self.end);
            for s in &mut self.spans {
                shift(&mut s.0);
                shift(&mut s.1);
            }
        } else if at < self.end {
            for (i, s) in self.spans.iter_mut().enumerate() {
                if at <= s.1 && at + delta.unsigned_abs() >= s.0 {
                    self.changed[i] = true;
                } else if at <= s.0 {
                    shift(&mut s.0);
                    shift(&mut s.1);
                }
            }
            shift(&mut self.end);
        }
    }
}

impl Tracked for EditField {
    fn capability(&self) -> Capability {
        Capability::Inline
    }

    fn poll(&mut self) -> Frame {
        if !window_alive(self.edit) {
            return Frame { focus: Focus::Gone, marks: Vec::new(), field: None, anchor: None, text_gone: true };
        }
        self.follow();
        let fg = foreground();
        let focused_here = unsafe {
            let thread = wm::GetWindowThreadProcessId(sys(self.edit), std::ptr::null_mut());
            let mut info = wm::GUITHREADINFO { cbSize: std::mem::size_of::<wm::GUITHREADINFO>() as u32, ..std::mem::zeroed() };
            wm::GetGUIThreadInfo(thread, &mut info) != 0 && info.hwndFocus as usize == self.edit
        };
        let focus = if super::overlay_owns(fg) || (fg == self.target.window && focused_here) { Focus::InField } else { Focus::Away };
        let field = self.client();
        let gone = self.length < self.end;
        let marks = self
            .spans
            .iter()
            .zip(&self.changed)
            .map(|(&(a, b), changed)| {
                if *changed || gone {
                    return Place::Changed;
                }
                let rects: Vec<Rect> = self.rects(a, b).into_iter().filter(|r| field.is_none_or(|f| f.contains_rect(r, 2))).collect();
                if rects.is_empty() { Place::Hidden } else { Place::Visible(rects) }
            })
            .collect();
        Frame { focus, marks, field, anchor: gui_caret(self.edit).or(field), text_gone: gone }
    }

    fn replace(&mut self, index: usize, with: &str) -> Result<(), String> {
        self.follow();
        let &(a, b) = self.spans.get(index).ok_or("unknown_mark")?;
        if self.changed[index] {
            return Err("The word changed since it was pasted.".into());
        }
        let (a, b) = match self.edges[index] {
            Edge::Word => (a, b),
            Edge::After => (b, b),
            Edge::Before => (a, a),
        };
        let mut caret = 0u32;
        send(self.edit, EM_GETSEL, 0, &mut caret as *mut u32 as isize);
        let wide: Vec<u16> = with.encode_utf16().chain(std::iter::once(0)).collect();
        send(self.edit, EM_SETSEL, a, b as isize);
        // Undoable replacement; the system marshals this message across processes.
        send(self.edit, EM_REPLACESEL, 1, wide.as_ptr() as isize);
        let delta = (wide.len() - 1) as isize - (b - a) as isize;
        let caret = if caret as usize >= b { (caret as isize + delta) as usize } else { caret as usize };
        send(self.edit, EM_SETSEL, caret, caret as isize);
        self.length = (self.length as isize + delta) as usize;
        self.end = (self.end as isize + delta) as usize;
        for (i, s) in self.spans.iter_mut().enumerate() {
            if i != index && s.0 >= b {
                s.0 = (s.0 as isize + delta) as usize;
                s.1 = (s.1 as isize + delta) as usize;
            }
        }
        let s = &mut self.spans[index];
        s.1 = (s.1 as isize + delta) as usize;
        Ok(())
    }

    fn refocus(&mut self) -> bool {
        bring_back(self.target.window)
    }
}
