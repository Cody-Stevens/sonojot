//! Linux word locator over AT-SPI, the desktop accessibility bus.
//!
//! The field SonoJot pasted into is found as the focused, editable accessible
//! of the target window's process. Its text gives the marked words' character
//! offsets and their rectangles. Wayland clients don't know where their
//! window is, so AT-SPI "screen" coordinates are relative to the window; the
//! window's position comes from Hyprland. XWayland clients report real
//! coordinates.
//!
//! Marks follow the words through edits elsewhere in the field using the
//! shared [`WordTracker`](super::track::WordTracker). A mark ends only when
//! resolved or when its own word is edited or removed (or a gap is filled).
//!
//! Chromium-based apps (Chrome, Brave, Electron apps such as T3 Code) only
//! expose their text when desktop accessibility was on when they started.
//! Apps without trustworthy word positions fail quietly with no marks.
//!
//! Privacy rule (see locator.rs): only the field's text is read transiently
//! to locate the pasted words and notice edits; it is never kept, logged, or
//! sent anywhere.

use super::locator::{Capability, Edge, Focus, Frame, Inert, Locator, Place, Rect, Request, Tracked};
use super::track::WordTracker;
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";
const COMPONENT: &str = "org.a11y.atspi.Component";
const EDITABLE_TEXT: &str = "org.a11y.atspi.EditableText";

// AtspiStateType bits and AtspiRole values.
const STATE_DEFUNCT: u32 = 6;
const STATE_EDITABLE: u32 = 7;
const STATE_FOCUSED: u32 = 12;
const STATE_SHOWING: u32 = 25;
const ROLE_PASSWORD_TEXT: u32 = 40;
const ROLE_TERMINAL: u32 = 60;
/// AtspiCoordType: relative to the screen (for Wayland clients, the window).
const COORD_SCREEN: u32 = 0;

/// The longest the search for the focused field may take at attach.
const SEARCH_BUDGET: Duration = Duration::from_millis(600);
const OVERLAY_TITLE: &str = "SonoJot doubtful words";

fn has_state(states: &[u32], bit: u32) -> bool {
    states.get((bit / 32) as usize).is_some_and(|word| word & (1 << (bit % 32)) != 0)
}

/// Turn on desktop accessibility for this session, so apps that start from
/// now on expose their text. Chromium-based apps read it at startup only.
pub fn enable_accessibility() {
    let Ok(session) = Connection::session() else { return };
    let _ = session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.freedesktop.DBus.Properties"),
        "Set",
        &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
    );
}

fn a11y_bus() -> Result<Connection, String> {
    let session = Connection::session().map_err(|e| e.to_string())?;
    let address: String = session
        .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
        .and_then(|reply| reply.body().deserialize())
        .map_err(|e| e.to_string())?;
    zbus::blocking::connection::Builder::address(address.as_str())
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

/// One accessible object: a bus name and an object path.
#[derive(Clone, Debug, PartialEq)]
struct Node {
    bus: String,
    path: OwnedObjectPath,
}

impl Node {
    fn call<B, R>(&self, conn: &Connection, interface: &str, method: &str, body: &B) -> Result<R, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        conn.call_method(Some(self.bus.as_str()), self.path.as_str(), Some(interface), method, body)
            .and_then(|reply| reply.body().deserialize())
            .map_err(|e| e.to_string())
    }

    fn property<R>(&self, conn: &Connection, interface: &str, name: &str) -> Result<R, String>
    where
        R: TryFrom<zbus::zvariant::OwnedValue>,
    {
        let value: zbus::zvariant::OwnedValue =
            self.call(conn, "org.freedesktop.DBus.Properties", "Get", &(interface, name))?;
        R::try_from(value).map_err(|_| format!("unexpected {name} type"))
    }

    fn children(&self, conn: &Connection) -> Vec<Node> {
        self.call::<_, Vec<(String, OwnedObjectPath)>>(conn, ACCESSIBLE, "GetChildren", &())
            .map(|list| list.into_iter().map(|(bus, path)| Node { bus, path }).collect())
            .unwrap_or_default()
    }

    fn states(&self, conn: &Connection) -> Vec<u32> {
        self.call(conn, ACCESSIBLE, "GetState", &()).unwrap_or_default()
    }

    fn role(&self, conn: &Connection) -> u32 {
        self.call(conn, ACCESSIBLE, "GetRole", &()).unwrap_or(0)
    }

    fn text(&self, conn: &Connection) -> Option<Vec<char>> {
        let count: i32 = self.property(conn, TEXT, "CharacterCount").ok()?;
        let text: String = self.call(conn, TEXT, "GetText", &(0i32, count.max(0))).ok()?;
        Some(text.chars().collect())
    }

    fn caret(&self, conn: &Connection) -> Option<usize> {
        self.property::<i32>(conn, TEXT, "CaretOffset").ok().and_then(|c| usize::try_from(c).ok())
    }

    fn range_extents(&self, conn: &Connection, start: usize, end: usize) -> Option<(i32, i32, i32, i32)> {
        self.call(conn, TEXT, "GetRangeExtents", &(start as i32, end as i32, COORD_SCREEN)).ok()
    }

    fn extents(&self, conn: &Connection) -> Option<(i32, i32, i32, i32)> {
        self.call(conn, COMPONENT, "GetExtents", &(COORD_SCREEN,)).ok()
    }

    fn offset_at(&self, conn: &Connection, x: i32, y: i32) -> Option<i32> {
        self.call(conn, TEXT, "GetOffsetAtPoint", &(x, y, COORD_SCREEN)).ok()
    }
}

/// The process behind a bus name on the accessibility bus.
fn bus_pid(conn: &Connection, bus: &str) -> Option<u32> {
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetConnectionUnixProcessID",
        &(bus,),
    )
    .and_then(|reply| reply.body().deserialize())
    .ok()
}

/// Depth-first search for the focused editable text under `root`, only
/// through showing objects, within the time budget.
fn focused_editable(conn: &Connection, root: &Node, deadline: Instant) -> Option<Node> {
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if Instant::now() > deadline || depth > 80 {
            return None;
        }
        let states = node.states(conn);
        if depth > 0 && !has_state(&states, STATE_SHOWING) {
            continue;
        }
        if has_state(&states, STATE_FOCUSED) && has_state(&states, STATE_EDITABLE) {
            return Some(node);
        }
        // Chromium exposes the focused field's ancestors as focused too in
        // some builds; keep descending either way.
        let children = node.children(conn);
        stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
    }
    None
}

/// Where a Wayland window is, from Hyprland, as a layout origin and the
/// scale of the monitor it is on. XWayland windows report real coordinates.
fn window_origin(window: usize) -> Option<(i32, i32, f64, bool)> {
    let geometry = crate::hyprland::client_geometry(&format!("0x{window:x}"))?;
    Some((geometry.x, geometry.y, geometry.scale, geometry.xwayland))
}

pub struct AtspiLocator;

impl Locator for AtspiLocator {
    fn attach(&mut self, request: &Request) -> Result<Box<dyn Tracked>, String> {
        match AtspiField::attach(request) {
            Ok(field) => Ok(Box::new(field)),
            Err(reason) => {
                // Reasons are short codes, except D-Bus errors.
                let code = if reason.contains(' ') { "dbus_error" } else { reason.as_str() };
                super::log(&format!("atspi_none_{code}"), 0);
                Ok(Box::new(Inert))
            }
        }
    }
}

struct AtspiField {
    conn: Connection,
    node: Node,
    window: usize,
    process: u32,
    tracker: WordTracker,
}

impl AtspiField {
    fn attach(request: &Request) -> Result<Self, String> {
        if request.target.window == 0 || request.target.process == 0 {
            return Err("no_target_window".into());
        }
        let conn = a11y_bus()?;
        let registry = Node { bus: REGISTRY.into(), path: OwnedObjectPath::try_from(ROOT).map_err(|e| e.to_string())? };
        let deadline = Instant::now() + SEARCH_BUDGET;
        let node = registry
            .children(&conn)
            .into_iter()
            .filter(|app| bus_pid(&conn, &app.bus) == Some(request.target.process))
            .find_map(|app| focused_editable(&conn, &app, deadline))
            .ok_or("no_accessible_field")?;
        let role = node.role(&conn);
        if role == ROLE_PASSWORD_TEXT {
            return Err("password_field".into());
        }
        if role == ROLE_TERMINAL {
            return Err("terminal".into());
        }
        let text = node.text(&conn).ok_or("no_text_interface")?;
        let caret = node.caret(&conn).unwrap_or(text.len());
        let tracker = WordTracker::attach(&text, request.text, request.spans, caret).ok_or("pasted_text_not_found")?;
        let field = Self {
            conn,
            node,
            window: request.target.window,
            process: request.target.process,
            tracker,
        };
        if !field.positions_trustworthy() {
            return Err("untrusted_rects".into());
        }
        Ok(field)
    }

    /// Inline only when every visible mark's rectangle checks out: plausible
    /// size, inside the visible field, and hit-testing back to the word itself.
    fn positions_trustworthy(&self) -> bool {
        let field_rect = self.node.extents(&self.conn).and_then(|e| self.to_screen(e));
        let mut visible = 0usize;
        for m in &self.tracker.marks {
            let Some(at) = m.at else { return false };
            let Some((x, y, w, h)) = self.node.range_extents(&self.conn, at, at + m.word.len()) else {
                continue;
            };
            if w <= 0 || h <= 0 {
                continue;
            }
            let Some(screen_rect) = self.to_screen((x, y, w, h)) else { return false };
            if field_rect.is_some_and(|f| !f.contains_rect(&screen_rect, 4)) {
                continue;
            }
            visible += 1;
            let hit = self
                .node
                .offset_at(&self.conn, x + w / 2, y + h / 2)
                .map_or(true, |o| usize::try_from(o).is_ok_and(|o| o >= at && o < at + m.word.len().max(1)));
            if !super::locator::trustworthy(&[screen_rect], field_rect.as_ref(), hit) {
                return false;
            }
        }
        visible > 0 || self.tracker.marks.is_empty()
    }

    fn to_screen(&self, (x, y, w, h): (i32, i32, i32, i32)) -> Option<Rect> {
        let (ox, oy, scale, xwayland) = window_origin(self.window)?;
        let (bx, by) = if xwayland { (0, 0) } else { (ox, oy) };
        Some(Rect {
            x: (f64::from(bx + x) * scale).round() as i32,
            y: (f64::from(by + y) * scale).round() as i32,
            w: (f64::from(w) * scale).round().max(1.0) as i32,
            h: (f64::from(h) * scale).round().max(1.0) as i32,
        })
    }

    fn focus(&self) -> Focus {
        let states = self.node.states(&self.conn);
        if states.is_empty() || has_state(&states, STATE_DEFUNCT) {
            return Focus::Gone;
        }
        if crate::hyprland::client_geometry(&format!("0x{:x}", self.window)).is_none() {
            return Focus::Gone;
        }
        let Some(active) = crate::hyprland::active_window() else { return Focus::Away };
        let ours = active.pid == i64::from(std::process::id());
        if ours && active.title == OVERLAY_TITLE {
            return Focus::InField;
        }
        if active.address == format!("0x{:x}", self.window) && has_state(&states, STATE_FOCUSED) {
            Focus::InField
        } else {
            Focus::Away
        }
    }
}

impl Tracked for AtspiField {
    fn capability(&self) -> Capability {
        Capability::Inline
    }

    fn poll(&mut self) -> Frame {
        let focus = self.focus();
        if focus == Focus::Gone {
            return Frame { focus, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        }
        if focus == Focus::Away {
            // Keep polling cheap while away; marks return when focus comes back.
            return Frame { focus, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: false };
        }
        let Some(text) = self.node.text(&self.conn) else {
            return Frame { focus: Focus::Gone, marks: Vec::new(), retyped: Vec::new(), field: None, text_gone: true };
        };
        self.tracker.follow(&text);
        let field = self.node.extents(&self.conn).and_then(|e| self.to_screen(e));
        let marks: Vec<Place> = self
            .tracker
            .marks
            .iter()
            .map(|m| {
                if m.resolved {
                    return Place::Hidden;
                }
                let Some(at) = m.at else { return Place::Changed };
                let rect = self.node.range_extents(&self.conn, at, at + m.word.len()).and_then(|e| self.to_screen(e));
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
        let text = self.node.text(&self.conn).ok_or("The field can no longer be read.")?;
        self.tracker.follow(&text);
        let mark = self.tracker.marks.get(index).ok_or("unknown_mark")?;
        let at = mark.at.ok_or("The word changed since it was pasted.")?;
        let (start, end) = match mark.edge {
            Edge::Word => (at, at + mark.word.len()),
            Edge::After => (at + mark.word.len(), at + mark.word.len()),
            Edge::Before => (at, at),
        };
        // Apps with editable text (GTK) change it directly, without the clipboard.
        let deleted = start == end
            || self
                .node
                .call::<_, bool>(&self.conn, EDITABLE_TEXT, "DeleteText", &(start as i32, end as i32))
                .is_ok_and(|ok| ok);
        let direct = deleted
            && self
                .node
                .call::<_, bool>(&self.conn, EDITABLE_TEXT, "InsertText", &(start as i32, with, with.chars().count() as i32))
                .is_ok_and(|ok| ok);
        if direct {
            self.tracker.resolve(index);
            return Ok(());
        }
        // Chromium has no editable-text interface: select the word through the
        // app, then paste over the selection.
        if !self.refocus() {
            return Err("The app is not in front.".into());
        }
        let target_address = format!("0x{:x}", self.window);
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            if crate::hyprland::active_window().is_some_and(|w| w.address == target_address) {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let selected = if start == end {
            self.node.call::<_, bool>(&self.conn, TEXT, "SetCaretOffset", &(start as i32,)).unwrap_or(false)
        } else {
            self.node.call::<_, bool>(&self.conn, TEXT, "SetSelection", &(0i32, start as i32, end as i32)).unwrap_or(false)
                || self.node.call::<_, bool>(&self.conn, TEXT, "AddSelection", &(start as i32, end as i32)).unwrap_or(false)
        };
        if !selected {
            return Err("The app did not let SonoJot select the word.".into());
        }
        std::thread::sleep(Duration::from_millis(60));
        let target = crate::platform::capture_paste_target().ok_or("The app is not in front.")?;
        if crate::platform::trust_target(&target).process != self.process {
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
        crate::hyprland::focus(&format!("0x{:x}", self.window))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_bits_read_across_words() {
        let states = [1 << STATE_FOCUSED | 1 << STATE_EDITABLE, 1 << (43 - 32)];
        assert!(has_state(&states, STATE_FOCUSED));
        assert!(has_state(&states, 43));
        assert!(!has_state(&states, STATE_SHOWING));
    }
}
