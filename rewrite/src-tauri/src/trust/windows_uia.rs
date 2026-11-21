//! Windows locator: UI Automation text ranges for most apps, and Win32 Edit
//! messages for classic edit controls (Notepad). Apps without trustworthy word
//! positions (password fields, elevated windows, terminals, opaque canvases)
//! return `Nothing` quietly.
//!
//! Privacy: UIA asks the app only for the pasted words' ranges (via search),
//! their immediate 1-character boundaries, and their screen rectangles.
//! Classic Edit controls read the field text transiently to follow manual edits
//! and drop the buffer immediately; nothing is ever kept or logged.

use super::locator::{Capability, Edge, Focus, Frame, Inert, Locator, Place, Rect, Request, Span, Target, Tracked};
use super::track::{char_range_to_utf16, utf16_to_char_offset, WordTracker};
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
            return Ok(Box::new(Inert));
        }
        let focused_class = class_name(target.focused);
        if focused_class.eq_ignore_ascii_case("Edit") {
            if let Some(edit) = EditField::attach(request, &target) {
                return Ok(Box::new(edit));
            }
        }
        if is_terminal(&class_name(root_of(target.window))) || is_terminal(&focused_class) {
            return Ok(Box::new(Inert));
        }
        let Some(automation) = self.automation.clone() else {
            return Ok(Box::new(Inert));
        };
        // Chromium and Electron build their accessibility tree on the first
        // query; ask again for up to a second before giving up on positions.
        let deadline = Instant::now() + Duration::from_millis(1200);
        loop {
            match UiaField::attach(&automation, request, &target) {
                Ok(field) => return Ok(Box::new(field)),
                Err(AttachError::Password) | Err(AttachError::NoText) => return Ok(Box::new(Inert)),
                Err(AttachError::Retry(_)) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(150)),
                Err(AttachError::Retry(_)) => return Ok(Box::new(Inert)),
            }
        }
    }
}

enum AttachError {
    Password,
    NoText,
    Retry(&'static str),
}

struct Located {
    span: Span,
    word: String,
    /// Immediate same-line neighbour phrases from the pasted text, for
    /// relocating the word after edits elsewhere without reading the document.
    left_phrase: String,
    right_phrase: String,
    range: Option<IUIAutomationTextRange>,
    resolved: bool,
}

struct UiaField {
    automation: IUIAutomation,
    target: Target,
    /// The focused element SonoJot pasted into.
    element: IUIAutomationElement,
    pattern: IUIAutomationTextPattern,
    marks: Vec<Located>,
    text: String,
}

// The trust thread owns these COM pointers alone (MTA).
unsafe impl Send for UiaField {}

fn text_pattern_of(element: &IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok() }
}

/// Immediate same-line context phrases `(left + word, word + right)` around `span`.
fn context_phrases(text: &str, span: &Span) -> (String, String) {
    let line_start = text[..span.start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[span.end..].find('\n').map_or(text.len(), |i| span.end + i);
    let left_start = text[line_start..span.start]
        .trim_end()
        .rfind(' ')
        .map_or(line_start, |i| line_start + i + 1);
    let right_end = {
        let tail = &text[span.end..line_end];
        let trimmed = tail.trim_start_matches(' ');
        let skipped = tail.len() - trimmed.len();
        trimmed.find(' ').map_or(line_end, |i| span.end + skipped + i)
    };
    let left = if left_start < span.start { text[left_start..span.end].to_owned() } else { String::new() };
    let right = if right_end > span.end { text[span.start..right_end].to_owned() } else { String::new() };
    (left, right)
}

impl UiaField {
    fn attach(automation: &IUIAutomation, request: &Request, target: &Target) -> Result<Self, AttachError> {
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
                let (left_phrase, right_phrase) = context_phrases(text, span);
                let range = find_word(&search, text, span).or_else(|| find_word(&document, text, span));
                found_any |= range.is_some();
                marks.push(Located { span: span.clone(), word, left_phrase, right_phrase, range, resolved: false });
            }
            if !found_any && !request.spans.is_empty() {
                return Err(AttachError::Retry("text_not_found"));
            }
            let field = UiaField {
                automation: automation.clone(),
                target: target.clone(),
                element,
                pattern,
                marks,
                text: text.to_owned(),
            };
            if !field.positions_trustworthy() {
                return Err(AttachError::NoText);
            }
            Ok(field)
        }
    }

    /// Inline only when every visible mark passes the mismatch check.
    fn positions_trustworthy(&self) -> bool {
        let field = self.field_rect();
        let mut visible = 0;
        for m in &self.marks {
            let Some(range) = &m.range else { return false };
            let rects = rects_of(range);
            if rects.is_empty() {
                continue;
            }
            if field.is_some_and(|f| !rects.iter().all(|r| f.contains_rect(r, 4))) {
                continue;
            }
            visible += 1;
            let hit = self.hit_matches(range, &rects[0]);
            if !super::locator::trustworthy(&rects, field.as_ref(), hit) {
                return false;
            }
        }
        visible > 0 || self.marks.is_empty()
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
            let Ok(focused) = self.automation.GetFocusedElement() else {
                return Focus::Away;
            };
            if self.automation.CompareElements(&focused, &self.element).is_ok_and(|same| same.as_bool()) {
                return Focus::InField;
            }
            // In rich/contenteditable editors, focus may sit on a child node
            // inside the editing host element.
            if let Ok(walker) = self.automation.ControlViewWalker() {
                let mut cur = focused;
                for _ in 0..4 {
                    let Ok(parent) = walker.GetParentElement(&cur) else { break };
                    if self.automation.CompareElements(&parent, &self.element).is_ok_and(|same| same.as_bool()) {
                        return Focus::InField;
                    }
                    cur = parent;
                }
            }
            Focus::Away
        }
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

/// True when `range` holds `word` and is not embedded inside a longer word.
/// Reads only `word` plus at most one neighbouring character on each side.
fn matches_word_with_boundaries(range: &IUIAutomationTextRange, word: &str) -> bool {
    let char_len = word.chars().count() as i32;
    let Some(actual) = text_of(range, char_len + 4) else {
        return false;
    };
    if actual != word {
        return false;
    }
    unsafe {
        let Ok(expanded) = range.Clone() else { return true };
        let moved_left = expanded.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1).unwrap_or(0);
        let moved_right = expanded.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1).unwrap_or(0);
        let Some(wide) = text_of(&expanded, char_len + 8) else { return true };
        let chars: Vec<char> = wide.chars().collect();
        let word_starts_alnum = word.chars().next().is_some_and(char::is_alphanumeric);
        let word_ends_alnum = word.chars().last().is_some_and(char::is_alphanumeric);
        if moved_left < 0 && word_starts_alnum && chars.first().is_some_and(|c| c.is_alphanumeric()) {
            return false;
        }
        if moved_right > 0 && word_ends_alnum && chars.last().is_some_and(|c| c.is_alphanumeric()) {
            return false;
        }
        true
    }
}

/// Find `word` inside a matched `phrase` range.
fn find_word_in_phrase(scope: &IUIAutomationTextRange, phrase: &str, word: &str, at_end: bool) -> Option<IUIAutomationTextRange> {
    if phrase.len() <= word.len() {
        return None;
    }
    unsafe {
        let outer = scope.FindText(&BSTR::from(phrase), true, false).ok()?;
        let found = outer.FindText(&BSTR::from(word), at_end, false).ok()?;
        matches_word_with_boundaries(&found, word).then_some(found)
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
        matches_word_with_boundaries(&found, word).then_some(found)
    }
}

/// Find a marked word again after the user edited elsewhere in the field. A UIA
/// range in Chromium and many editors is a fixed offset, so typing or pasting
/// before the word leaves the range over other text even though the word is
/// untouched. Try (1) the unique phrase, (2) immediate left/right neighbour
/// phrases, (3) nearby search around the old range, and (4) a unique match in
/// the document (for large pastes even when neighbours changed). FindText
/// locates text without reading the rest of the field.
fn relocate(pattern: &IUIAutomationTextPattern, text: &str, mark: &Located) -> Option<IUIAutomationTextRange> {
    unsafe {
        let document = pattern.DocumentRange().ok()?;
        // Only use the unique needle when it carries neighbour context; a bare
        // word needle is handled below with uniqueness/proximity guards.
        let (na, nb) = super::locator::needle(text, mark.span.start, mark.span.end);
        if nb - na > mark.word.len() {
            if let Some(found) = find_word(&document, text, &mark.span) {
                return Some(found);
            }
        }
        if let Some(found) = find_word_in_phrase(&document, &mark.left_phrase, &mark.word, true) {
            return Some(found);
        }
        if let Some(found) = find_word_in_phrase(&document, &mark.right_phrase, &mark.word, false) {
            return Some(found);
        }
        if let Some(old) = mark.range.as_ref().and_then(|r| r.Clone().ok()) {
            let reach = mark.word.chars().count() as i32 + 128;
            let _ = old.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -reach);
            let _ = old.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, reach);
            if let Ok(found) = old.FindText(&BSTR::from(mark.word.as_str()), false, false) {
                if matches_word_with_boundaries(&found, &mark.word) {
                    return Some(found);
                }
            }
        }
        // Large paste before a unique word whose neighbours were also edited:
        // accept only when the word occurs once in the entire field.
        let fwd = document.FindText(&BSTR::from(mark.word.as_str()), false, false).ok()?;
        let bwd = document.FindText(&BSTR::from(mark.word.as_str()), true, false).ok()?;
        if fwd.CompareEndpoints(TextPatternRangeEndpoint_Start, &bwd, TextPatternRangeEndpoint_Start).ok() == Some(0)
            && matches_word_with_boundaries(&fwd, &mark.word)
        {
            return Some(fwd);
        }
        None
    }
}

/// The mark's range if it still holds the word, after relocating it if needed.
fn current(pattern: &IUIAutomationTextPattern, text: &str, mark: &mut Located) -> Option<IUIAutomationTextRange> {
    let range = mark.range.as_ref()?;
    if matches_word_with_boundaries(range, &mark.word) {
        return Some(range.clone());
    }
    let found = relocate(pattern, text, mark)?;
    mark.range = Some(found.clone());
    Some(found)
}

impl Tracked for UiaField {
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
        let field = self.field_rect();
        let mut open_total = 0usize;
        let mut changed = 0usize;
        let (pattern, text) = (&self.pattern, &self.text);
        let marks: Vec<Place> = self
            .marks
            .iter_mut()
            .map(|m| {
                if m.resolved {
                    return Place::Hidden;
                }
                open_total += 1;
                if m.range.is_none() {
                    changed += 1;
                    return Place::Changed;
                }
                let Some(range) = current(pattern, text, m) else {
                    m.range = None;
                    changed += 1;
                    return Place::Changed;
                };
                let rects: Vec<Rect> = rects_of(&range).into_iter().filter(|r| field.is_none_or(|f| f.contains_rect(r, 2))).collect();
                if rects.is_empty() { Place::Hidden } else { Place::Visible(rects) }
            })
            .collect();
        let text_gone = open_total > 0 && changed == open_total;
        Frame { focus, marks, retyped: Vec::new(), field, text_gone }
    }

    fn replace(&mut self, index: usize, with: &str) -> Result<(), String> {
        let (pattern, text) = (&self.pattern, &self.text);
        let Some(m) = self.marks.get_mut(index) else { return Err("unknown_mark".into()) };
        if m.range.is_none() {
            return Err("The word is no longer where SonoJot pasted it.".into());
        }
        let range = current(pattern, text, m).ok_or("The word changed since it was pasted.")?;
        let span = m.span.clone();
        if !self.refocus() {
            return Err("The app is not in front.".into());
        }
        wait_for_modifiers()?;
        let edge = span.edge;
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
        // Keep internal text and remaining span offsets aligned after our edit.
        let (rep_start, rep_end) = match edge {
            Edge::Word => (span.start, span.end),
            Edge::After => (span.end, span.end),
            Edge::Before => (span.start, span.start),
        };
        if rep_end <= self.text.len() && self.text.is_char_boundary(rep_start) && self.text.is_char_boundary(rep_end) {
            self.text.replace_range(rep_start..rep_end, with);
            let delta = with.len() as isize - (rep_end - rep_start) as isize;
            for (i, other) in self.marks.iter_mut().enumerate() {
                if i != index && other.span.start >= rep_end {
                    other.span.start = (other.span.start as isize + delta) as usize;
                    other.span.end = (other.span.end as isize + delta) as usize;
                }
            }
        }
        self.marks[index].resolved = true;
        Ok(())
    }

    fn resolve(&mut self, index: usize) {
        if let Some(m) = self.marks.get_mut(index) {
            m.resolved = true;
        }
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
/// pattern, but answers position and text messages. Text is read transiently
/// (up to 256K UTF-16 units) so marks follow arbitrary manual edits accurately
/// and dropped immediately; nothing is stored or logged.
struct EditField {
    target: Target,
    edit: usize,
    tracker: WordTracker,
}

const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EM_LINEFROMCHAR: u32 = 0x00C9;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_POSFROMCHAR: u32 = 0x00D6;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const MAX_EDIT_UTF16: usize = 262_144;

fn send(window: usize, msg: u32, w: usize, l: isize) -> isize {
    unsafe { wm::SendMessageW(sys(window), msg, w, l) }
}

fn read_edit_chars(edit: usize) -> Option<Vec<char>> {
    let len = send(edit, WM_GETTEXTLENGTH, 0, 0).max(0) as usize;
    if len > MAX_EDIT_UTF16 {
        return None;
    }
    let mut buf = vec![0u16; len + 1];
    let copied = send(edit, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize).max(0) as usize;
    let slice = &buf[..copied.min(len)];
    Some(String::from_utf16_lossy(slice).chars().collect())
}

impl EditField {
    fn attach(request: &Request, target: &Target) -> Option<Self> {
        let edit = target.focused;
        let text = read_edit_chars(edit)?;
        let mut sel_start = 0u32;
        let mut sel_end = 0u32;
        send(edit, EM_GETSEL, &mut sel_start as *mut u32 as usize, &mut sel_end as *mut u32 as isize);
        let caret = utf16_to_char_offset(&text, sel_end as usize);
        // Edit controls store line breaks as CR LF, while pasted text uses LF.
        let stored = request.text.replace("\r\n", "\n").replace('\n', "\r\n");
        let spans = map_crlf_spans(request.text, request.spans);
        let tracker = WordTracker::attach(&text, &stored, &spans, caret)?;
        Some(Self { target: target.clone(), edit, tracker })
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
}

/// Adjust LF-based byte spans into CRLF-normalized byte spans.
fn map_crlf_spans(text: &str, spans: &[Span]) -> Vec<Span> {
    let map = |byte: usize| text[..byte].replace("\r\n", "\n").replace('\n', "\r\n").len();
    spans.iter().map(|s| Span { start: map(s.start), end: map(s.end), edge: s.edge }).collect()
}

impl Tracked for EditField {
    fn capability(&self) -> Capability {
        Capability::Inline
    }

    fn poll(&mut self) -> Frame {
        if !window_alive(self.edit) {
            return Frame { focus: Focus::Gone, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        }
        let fg = foreground();
        let focused_here = unsafe {
            let thread = wm::GetWindowThreadProcessId(sys(self.edit), std::ptr::null_mut());
            let mut info = wm::GUITHREADINFO { cbSize: std::mem::size_of::<wm::GUITHREADINFO>() as u32, ..std::mem::zeroed() };
            wm::GetGUIThreadInfo(thread, &mut info) != 0 && info.hwndFocus as usize == self.edit
        };
        let focus = if super::overlay_owns(fg) || (fg == self.target.window && focused_here) { Focus::InField } else { Focus::Away };
        if focus == Focus::Away {
            return Frame { focus, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: false };
        }
        let Some(text) = read_edit_chars(self.edit) else {
            return Frame { focus: Focus::Gone, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        };
        self.tracker.follow(&text);
        let field = self.client();
        let marks = self
            .tracker
            .marks
            .iter()
            .map(|m| {
                if m.resolved {
                    return Place::Hidden;
                }
                let Some(at) = m.at else { return Place::Changed };
                let (a, len) = char_range_to_utf16(&text, at, at + m.word.len());
                let rects: Vec<Rect> = self.rects(a, a + len).into_iter().filter(|r| field.is_none_or(|f| f.contains_rect(r, 2))).collect();
                if rects.is_empty() { Place::Hidden } else { Place::Visible(rects) }
            })
            .collect();
        let retyped: Vec<Option<String>> = self.tracker.marks.iter().map(|m| m.retyped.clone()).collect();
        let text_gone = text.is_empty() || self.tracker.all_unresolved_gone();
        Frame { focus, marks, retyped, field, text_gone }
    }

    fn replace(&mut self, index: usize, with: &str) -> Result<(), String> {
        let text = read_edit_chars(self.edit).ok_or("The field can no longer be read.")?;
        self.tracker.follow(&text);
        let mark = self.tracker.marks.get(index).ok_or("unknown_mark")?;
        let at = mark.at.ok_or("The word changed since it was pasted.")?;
        let (char_start, char_end) = match mark.edge {
            Edge::Word => (at, at + mark.word.len()),
            Edge::After => (at + mark.word.len(), at + mark.word.len()),
            Edge::Before => (at, at),
        };
        let (a, len) = char_range_to_utf16(&text, char_start, char_end);
        let b = a + len;
        let mut caret = 0u32;
        send(self.edit, EM_GETSEL, 0, &mut caret as *mut u32 as isize);
        let wide: Vec<u16> = with.encode_utf16().chain(std::iter::once(0)).collect();
        send(self.edit, EM_SETSEL, a, b as isize);
        // Undoable replacement; the system marshals this message across processes.
        send(self.edit, EM_REPLACESEL, 1, wide.as_ptr() as isize);
        let delta = (wide.len() - 1) as isize - (b - a) as isize;
        let caret = if caret as usize >= b { (caret as isize + delta) as usize } else { caret as usize };
        send(self.edit, EM_SETSEL, caret, caret as isize);
        self.tracker.resolve(index);
        Ok(())
    }

    fn resolve(&mut self, index: usize) {
        self.tracker.resolve(index);
    }

    fn refocus(&mut self) -> bool {
        bring_back(self.target.window)
    }
}
