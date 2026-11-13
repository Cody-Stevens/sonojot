//! The trust layer: after a dictation is pasted, mark the few words and gaps
//! SonoJot doubts, inside the field the user pasted into.
//!
//! Seams, each in its own module:
//! - `evidence`: per-word confidence, timings, gaps and alternatives, carried
//!   from the raw transcript to the cleaned text (platform independent).
//! - `decision`: a pure function from evidence and sensitivity to marks.
//! - `session`: the last pasted dictation, its marks and their lifetime.
//! - `locator`: where the words are on screen and how to replace one; the
//!   platform layer (`windows_uia`) sits behind its trait.
//! - `overlay`: the transparent window that draws marks, popover and chip.
//!
//! This module only wires them together on one worker thread. The paste always
//! happens first; nothing here can delay or fail it. Errors end the session
//! quietly and are logged as metadata (codes and counts, never text).

pub mod decision;
pub mod evidence;
pub mod locator;
mod overlay;
pub mod session;
#[cfg(windows)]
mod windows_uia;

use crate::domain::TrustSettings;
use decision::{Mark, MarkKind};
use evidence::{AltSource, Evidence, GapEvidence, RawWord};
use locator::{Capability, Edge, Focus, Frame, Locator, Place, Request, Span, Target, Tracked};
use serde::{Deserialize, Serialize};
use session::{Session, Status, Visibility};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// What recognition knew, handed over with the published text.
#[derive(Clone, Debug, Default)]
pub struct Capture {
    /// The guarded raw transcript that cleanup started from.
    pub raw_text: String,
    pub raw_words: Vec<RawWord>,
    /// Section start, end and word count, to place words without timings.
    pub segments: Vec<(f32, f32, usize)>,
    /// 16 kHz mono input as fed to recognition; held in memory only.
    pub audio: Option<Arc<Vec<f32>>>,
}

/// The longest dictation whose audio is held for snippets (memory bound).
pub const MAX_AUDIO_SECONDS: usize = 8 * 60;

enum Source {
    Capture(Arc<Capture>),
    /// Debug injection: marks chosen by the caller.
    Marks(Vec<Mark>, Option<Arc<Vec<f32>>>),
}

pub struct Pasted {
    pub text: String,
    source: Source,
    pub target: Target,
    pub app: Option<String>,
    pub vocabulary: Vec<String>,
    pub dictation_key: String,
    pub hold: bool,
}

impl Pasted {
    pub fn new(text: String, capture: Option<Arc<Capture>>, target: Target, app: Option<String>, vocabulary: Vec<String>, dictation_key: String, hold: bool) -> Self {
        let source = match capture {
            Some(c) => Source::Capture(c),
            None => Source::Marks(Vec::new(), None),
        };
        Self { text, source, target, app, vocabulary, dictation_key, hold }
    }
}

enum Msg {
    Pasted(Box<Pasted>),
    Ui(UiEvent),
    Settings(TrustSettings),
    NewDictation,
    Next,
    VoiceBegin,
    VoiceEnd(Option<String>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UiEvent {
    Ready,
    Regions { rects: Vec<RectF> },
    Opened { mark: u32, via: String },
    Closed { refocus: bool },
    Choose { mark: u32, text: String },
    Keep { mark: u32 },
    Next { from: Option<u32>, back: bool },
    Dismiss,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// State other threads may read without waiting on the worker.
#[derive(Default)]
struct Shared {
    session: u64,
    audio: Option<Arc<Vec<f32>>>,
    spans: Vec<(u32, (f32, f32))>,
    /// A popover or panel choice is open: the dictation key fixes by voice.
    popover: Option<u32>,
    recent_apps: Vec<String>,
}

struct Service {
    tx: Mutex<Sender<Msg>>,
    shared: Arc<Mutex<Shared>>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

fn send(msg: Msg) {
    if let Some(s) = SERVICE.get() {
        if let Ok(tx) = s.tx.lock() {
            let _ = tx.send(msg);
        }
    }
}

fn shared() -> Option<std::sync::MutexGuard<'static, Shared>> {
    SERVICE.get().and_then(|s| s.shared.lock().ok())
}

/// Metadata-only log line: an event code and numbers, never text.
fn log(code: &str, value: u64) {
    crate::recognition_diagnostics::event("trust", code, std::process::id(), 0, 0, value);
}

pub fn init(app: &AppHandle, settings: &TrustSettings) {
    if SERVICE.get().is_some() {
        return;
    }
    let (tx, rx) = channel();
    let shared = Arc::new(Mutex::new(Shared::default()));
    if SERVICE.set(Service { tx: Mutex::new(tx), shared: shared.clone() }).is_err() {
        return;
    }
    let app = app.clone();
    let settings = settings.clone();
    register_next_shortcut(&app, "", &settings.next_shortcut);
    let _ = std::thread::Builder::new().name("trust-layer".into()).spawn(move || {
        let mut worker = Worker::new(app, settings, shared);
        worker.run(rx);
    });
}

pub fn after_paste(pasted: Pasted) {
    send(Msg::Pasted(Box::new(pasted)));
}

/// A new dictation replaces the last one's marks.
pub fn new_dictation() {
    send(Msg::NewDictation);
}

pub fn settings_changed(app: &AppHandle, old: &TrustSettings, new: &TrustSettings) {
    register_next_shortcut(app, &old.next_shortcut, &new.next_shortcut);
    send(Msg::Settings(new.clone()));
}

/// True while a popover or panel is open: the dictation key then fixes the
/// open mark by voice instead of starting a dictation.
pub fn voice_fix_armed() -> bool {
    shared().is_some_and(|s| s.popover.is_some())
}

pub fn voice_begin() {
    send(Msg::VoiceBegin);
}

pub fn voice_end(text: Option<String>) {
    send(Msg::VoiceEnd(text));
}

pub fn next() {
    send(Msg::Next);
}

pub fn ui(event: UiEvent) {
    send(Msg::Ui(event));
}

pub fn recent_apps() -> Vec<String> {
    shared().map(|s| s.recent_apps.clone()).unwrap_or_default()
}

/// True when `window` is SonoJot's own overlay (it may hold focus briefly).
pub(crate) fn overlay_owns(window: usize) -> bool {
    overlay::owns(window)
}

fn register_next_shortcut(app: &AppHandle, old: &str, new: &str) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    if crate::flavor::SYNC_TEST || old == new && !old.is_empty() && app.global_shortcut().is_registered(old.replace(' ', "").as_str()) {
        return;
    }
    if !old.is_empty() {
        let _ = app.global_shortcut().unregister(old.replace(' ', "").as_str());
    }
    if new.is_empty() {
        return;
    }
    let result = app.global_shortcut().on_shortcut(new.replace(' ', "").as_str(), |_, _, event| {
        if event.state() == ShortcutState::Pressed {
            next();
        }
    });
    if result.is_err() {
        log("next_shortcut_unavailable", 0);
    }
}

/// A snippet of the held audio for a mark, ready for the overlay to play.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetData {
    /// 16-bit little-endian PCM at 16 kHz, base64.
    pub pcm: String,
    pub sample_rate: u32,
    pub mark_start: f32,
    pub mark_end: f32,
    pub seconds: f32,
    /// 110 normalized peaks for the waveform.
    pub peaks: Vec<f32>,
}

pub fn snippet(mark: u32) -> Option<SnippetData> {
    let (audio, span) = {
        let s = shared()?;
        (s.audio.clone()?, s.spans.iter().find(|(id, _)| *id == mark)?.1)
    };
    let sn = session::snippet(&audio, span.0, span.1)?;
    use base64::Engine;
    let mut bytes = Vec::with_capacity(sn.samples.len() * 2);
    for s in &sn.samples {
        bytes.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    let bins = 110usize;
    let step = (sn.samples.len() / bins).max(1);
    let mut peaks: Vec<f32> = sn.samples.chunks(step).take(bins).map(|c| c.iter().fold(0f32, |m, v| m.max(v.abs()))).collect();
    let max = peaks.iter().copied().fold(0f32, f32::max).max(1e-6);
    for p in &mut peaks {
        *p /= max;
    }
    Some(SnippetData { pcm: base64::engine::general_purpose::STANDARD.encode(bytes), sample_rate: 16_000, mark_start: sn.mark_start, mark_end: sn.mark_end, seconds: sn.seconds, peaks })
}

/// Compute the marks for a pasted dictation: raw evidence, vocabulary
/// alternatives, gaps from the held audio, carried onto the pasted text.
pub fn marks_for(text: &str, capture: &Capture, vocabulary: &[String], sensitivity: decision::Sensitivity) -> Vec<Mark> {
    let t = decision::thresholds();
    let mut raw = evidence::from_raw(&capture.raw_text, &capture.raw_words);
    let words: Vec<String> = (0..raw.words.len()).map(|i| raw.word(i).to_owned()).collect();
    for (w, word) in raw.words.iter_mut().zip(&words) {
        w.alternatives = evidence::vocabulary_alternatives(word, vocabulary);
    }
    let times: Vec<Option<(f32, f32)>> = raw.words.iter().map(|w| w.time).collect();
    if let Some(audio) = &capture.audio {
        let min = t.gap_seconds.iter().copied().fold(f32::INFINITY, f32::min);
        let rms = evidence::frame_rms(audio);
        raw.gaps = evidence::detect_gaps(&times, &rms, min, t.gap_speech_share)
            .into_iter()
            .map(|(after, time, speech_share)| GapEvidence { after, time, speech_share, alternatives: Vec::new() })
            .collect();
    }
    fill_section_times(&mut raw, &capture.segments);
    let projected = evidence::project(&raw, text);
    decision::decide(&projected, sensitivity, &t)
}

/// Words without their own timing get their section's span (when short), so
/// a snippet still plays the audio around the word.
fn fill_section_times(raw: &mut Evidence, segments: &[(f32, f32, usize)]) {
    if segments.iter().map(|s| s.2).sum::<usize>() != raw.words.len() {
        return;
    }
    let mut i = 0;
    for &(a, b, count) in segments {
        for w in raw.words.iter_mut().skip(i).take(count) {
            if w.time.is_none() && b - a <= 8.0 {
                w.time = Some((a, b));
            }
        }
        i += count;
    }
}

struct Active {
    session: Session,
    tracked: Box<dyn Tracked>,
    /// Tracker span index -> mark id.
    ids: Vec<u32>,
    capability: Capability,
    overlay: Option<overlay::Overlay>,
    frame: Option<Frame>,
    popover: Option<u32>,
    keyboard: bool,
    focus_request: Option<(u32, u64)>,
    message: Option<(String, u64)>,
    announce: Option<(String, u64)>,
    voice: Option<u32>,
    host_dark: Option<bool>,
    dictation_key: String,
    hold: bool,
    target: Target,
    finished_at: Option<Instant>,
    seq: u64,
}

struct Worker {
    app: AppHandle,
    settings: TrustSettings,
    shared: Arc<Mutex<Shared>>,
    active: Option<Active>,
    locator: Option<Box<dyn Locator>>,
    next_session: u64,
    last_view: String,
}

impl Worker {
    fn new(app: AppHandle, settings: TrustSettings, shared: Arc<Mutex<Shared>>) -> Self {
        Self { app, settings, shared, active: None, locator: None, next_session: 1, last_view: String::new() }
    }

    fn run(&mut self, rx: Receiver<Msg>) {
        loop {
            let wait = match &self.active {
                None => None,
                Some(a) if a.finished_at.is_some() => Some(Duration::from_millis(150)),
                Some(a) if a.session.visibility == Visibility::Shown => Some(Duration::from_millis(100)),
                // Away or dismissed: only watch for the user coming back.
                Some(_) => Some(Duration::from_millis(400)),
            };
            let msg = match wait {
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(d) => rx.recv_timeout(d),
            };
            if matches!(msg, Err(RecvTimeoutError::Disconnected)) {
                self.end(session::End::Off);
                break;
            }
            // A failure here must never reach the app: end the session quietly.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match msg {
                Ok(m) => self.handle(m),
                Err(_) => self.tick(),
            }));
            if outcome.is_err() {
                log("worker_panic", 0);
                self.active = None;
                self.last_view.clear();
            }
        }
    }

    fn locator(&mut self) -> &mut Box<dyn Locator> {
        self.locator.get_or_insert_with(|| {
            #[cfg(windows)]
            {
                Box::new(windows_uia::UiaLocator::new())
            }
            #[cfg(not(windows))]
            {
                Box::new(ChipOnly)
            }
        })
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Pasted(p) => self.pasted(*p),
            Msg::NewDictation => {
                if self.active.as_ref().is_some_and(|a| a.voice.is_none()) {
                    self.end(session::End::Replaced);
                }
            }
            Msg::Settings(s) => {
                let off = !s.enabled;
                self.settings = s;
                if off {
                    self.end(session::End::Off);
                } else {
                    self.last_view.clear();
                    self.tick();
                }
            }
            Msg::Next => self.next_mark(None, false),
            Msg::Ui(e) => self.ui(e),
            Msg::VoiceBegin => {
                if let Some(a) = &mut self.active {
                    a.voice = a.popover;
                    self.render();
                }
            }
            Msg::VoiceEnd(text) => self.voice_end(text),
        }
    }

    fn pasted(&mut self, p: Pasted) {
        self.end(session::End::Replaced);
        if !self.settings.enabled {
            return;
        }
        if let Some(app) = &p.app {
            if let Ok(mut s) = self.shared.lock() {
                s.recent_apps.retain(|a| a != app);
                s.recent_apps.insert(0, app.clone());
                s.recent_apps.truncate(12);
            }
            if self.settings.off_apps.iter().any(|a| a.eq_ignore_ascii_case(app)) {
                return;
            }
        }
        let (marks, audio) = match &p.source {
            Source::Capture(c) => (marks_for(&p.text, c, &p.vocabulary, self.settings.sensitivity), c.audio.clone()),
            Source::Marks(m, a) => (m.clone(), a.clone()),
        };
        log("marks", marks.len() as u64);
        if marks.is_empty() {
            notice(&self.app, "Pasted · nothing doubtful");
            return;
        }
        // Give the app a moment to receive the paste keys.
        std::thread::sleep(Duration::from_millis(120));
        let spans = spans_for(&p.text, &marks);
        let request = Request { text: &p.text, spans: &spans, target: &p.target };
        let tracked = match self.locator().attach(&request) {
            Ok(t) => t,
            Err(code) => {
                log(if code == "target_window_gone" { "attach_gone" } else { "attach_failed" }, 0);
                return;
            }
        };
        let capability = tracked.capability();
        log(capability_code(capability), marks.len() as u64);
        if capability == Capability::Nothing {
            return;
        }
        let id = self.next_session;
        self.next_session += 1;
        let audio = audio.filter(|a| a.len() <= MAX_AUDIO_SECONDS * 16_000);
        let session = Session::new(id, p.text.clone(), marks, audio.clone(), Instant::now());
        let ids = session.marks.iter().map(|m| m.id).collect();
        if let Ok(mut s) = self.shared.lock() {
            s.session = id;
            s.audio = audio;
            s.spans = session.marks.iter().filter_map(|m| Some((m.id, m.mark.audio?))).collect();
            s.popover = None;
        }
        let n = session.open_count();
        self.active = Some(Active {
            session,
            tracked,
            ids,
            capability,
            overlay: None,
            frame: None,
            popover: None,
            keyboard: false,
            focus_request: None,
            message: None,
            announce: Some((format!("Dictation pasted. {n} {} to check.", if n == 1 { "word" } else { "spots" }), 1)),
            voice: None,
            host_dark: None,
            dictation_key: p.dictation_key,
            hold: p.hold,
            target: p.target,
            finished_at: None,
            seq: 1,
        });
        notice(&self.app, &format!("Pasted · {n} to check"));
        self.tick();
    }

    fn tick(&mut self) {
        let Some(a) = &mut self.active else { return };
        if let Some(at) = a.finished_at {
            if at.elapsed() > Duration::from_millis(900) {
                self.end(session::End::AllResolved);
            }
            return;
        }
        let frame = a.tracked.poll();
        if frame.focus == Focus::Gone {
            return self.end(session::End::FieldGone);
        }
        if frame.text_gone {
            return self.end(session::End::TextGone);
        }
        for (i, place) in frame.marks.iter().enumerate() {
            if *place == Place::Changed {
                if let Some(id) = a.ids.get(i) {
                    if a.session.edited(*id) && a.popover == Some(*id) {
                        a.popover = None;
                    }
                }
            }
        }
        match frame.focus {
            Focus::InField => {
                a.session.returned();
            }
            Focus::Away if a.voice.is_none() && !a.keyboard => {
                if a.session.visibility == Visibility::Shown {
                    a.session.left_at(Instant::now());
                    a.popover = None;
                }
            }
            _ => {}
        }
        a.frame = Some(frame);
        if a.session.finished() {
            a.finished_at = Some(Instant::now());
            notice(&self.app, "All checked");
        }
        if a.session.expired(Instant::now()) {
            return self.end(session::End::Expired);
        }
        self.render();
    }

    /// Build the overlay's view and send it when it changed. The overlay
    /// exists only while marks or the chip are on screen.
    fn render(&mut self) {
        let Some(a) = &mut self.active else { return };
        let shown = a.session.visibility == Visibility::Shown && a.finished_at.is_none() && a.session.open_count() > 0;
        if let Ok(mut s) = self.shared.lock() {
            s.popover = if shown { a.popover } else { None };
        }
        if !shown {
            if let Some(o) = a.overlay.take() {
                o.close();
                self.last_view.clear();
            }
            return;
        }
        let Some(frame) = &a.frame else { return };
        let Some(anchor) = frame.field.or(frame.anchor) else { return };
        if a.host_dark.is_none() {
            a.host_dark = Some(host_dark(frame.field));
        }
        if a.overlay.is_none() {
            match overlay::Overlay::open(&self.app) {
                Ok(o) => a.overlay = Some(o),
                Err(_) => {
                    log("overlay_failed", 0);
                    return;
                }
            }
        }
        let Some(monitor) = a.overlay.as_mut().and_then(|o| o.place(&self.app, anchor)) else { return };
        let view = build_view(a, frame, &monitor, &self.settings);
        let json = serde_json::to_string(&view).unwrap_or_default();
        if json != self.last_view {
            self.last_view = json;
            if let Some(o) = &a.overlay {
                o.show(&view);
            }
        }
    }

    fn end(&mut self, why: session::End) {
        let Some(mut a) = self.active.take() else { return };
        if let Some(o) = a.overlay.take() {
            o.close();
        }
        if a.keyboard {
            a.tracked.refocus();
        }
        if let Ok(mut s) = self.shared.lock() {
            s.audio = None;
            s.spans.clear();
            s.popover = None;
        }
        self.last_view.clear();
        log(why.code(), a.session.open_count() as u64);
    }

    fn ui(&mut self, e: UiEvent) {
        let Some(a) = &mut self.active else { return };
        match e {
            UiEvent::Ready => {
                self.last_view.clear();
                self.render();
            }
            UiEvent::Regions { rects } => {
                if let Some(o) = &mut a.overlay {
                    o.set_regions(&rects);
                }
            }
            UiEvent::Opened { mark, via } => {
                a.popover = Some(mark);
                log(if via == "hover" { "open_hover" } else if via == "key" { "open_key" } else { "open_click" }, 0);
                self.render();
            }
            UiEvent::Closed { refocus } => {
                a.popover = None;
                a.voice = None;
                if a.keyboard {
                    a.keyboard = false;
                    if let Some(o) = &mut a.overlay {
                        o.release_focus();
                    }
                    if refocus {
                        a.tracked.refocus();
                    }
                }
                self.render();
            }
            UiEvent::Keep { mark } => {
                a.session.keep(mark);
                let was_key = a.keyboard;
                a.popover = None;
                self.after_resolve(mark, was_key, format!("Kept."));
            }
            UiEvent::Choose { mark, text } => self.choose(mark, &text),
            UiEvent::Next { from, back } => self.next_mark(from, back),
            UiEvent::Dismiss => {
                a.session.dismiss(Instant::now());
                a.popover = None;
                if a.keyboard {
                    a.keyboard = false;
                    if let Some(o) = &mut a.overlay {
                        o.release_focus();
                    }
                    a.tracked.refocus();
                }
                self.render();
            }
        }
    }

    fn choose(&mut self, mark: u32, text: &str) {
        let Some(a) = &mut self.active else { return };
        let Some(m) = a.session.get(mark).filter(|m| m.status == Status::Open).cloned() else { return };
        let was_key = a.keyboard;
        if !a.capability.can_replace() {
            // Read-only apps: hand the word over on the clipboard instead.
            let word = text.trim().to_owned();
            tauri::async_runtime::spawn(async move {
                let _ = crate::platform::copy_text(&word).await;
            });
            a.session.keep(mark);
            a.popover = None;
            return self.after_resolve(mark, was_key, "Copied. Paste it where it belongs.".into());
        }
        let Some(index) = a.ids.iter().position(|id| *id == mark) else { return };
        let insertion = match m.mark.kind {
            MarkKind::Word => text.trim().to_owned(),
            MarkKind::Gap => session::gap_insertion(&a.session.text, m.mark.start, text),
        };
        if a.keyboard {
            if let Some(o) = &mut a.overlay {
                o.release_focus();
            }
        }
        match a.tracked.replace(index, &insertion) {
            Ok(()) => {
                a.session.replaced(mark, text.trim());
                a.popover = None;
                log("replaced", 0);
                // Positions moved: find the remaining words again.
                std::thread::sleep(Duration::from_millis(60));
                self.reattach();
                let said = if m.mark.kind == MarkKind::Gap { format!("Added {}.", text.trim()) } else { format!("Replaced {} with {}.", m.mark.heard, text.trim()) };
                self.after_resolve(mark, was_key, said);
            }
            Err(message) => {
                log("replace_failed", 0);
                a.seq += 1;
                a.message = Some((message, a.seq));
                if was_key {
                    if let Some(o) = &mut a.overlay {
                        o.take_focus();
                    }
                }
                self.render();
            }
        }
    }

    fn reattach(&mut self) {
        let Some(a) = &mut self.active else { return };
        let open: Vec<(u32, Mark)> = a.session.open().map(|m| (m.id, m.mark.clone())).collect();
        if open.is_empty() {
            return;
        }
        let marks: Vec<Mark> = open.iter().map(|(_, m)| m.clone()).collect();
        let spans = spans_for(&a.session.text, &marks);
        let text = a.session.text.clone();
        let target = a.target.clone();
        let request = Request { text: &text, spans: &spans, target: &target };
        let attached = match &mut self.locator {
            Some(l) => l.attach(&request).ok(),
            None => None,
        };
        let Some(a) = &mut self.active else { return };
        if let Some(t) = attached.filter(|t| t.capability() != Capability::Nothing) {
            a.capability = t.capability();
            a.tracked = t;
            a.ids = open.iter().map(|(id, _)| *id).collect();
        }
    }

    fn after_resolve(&mut self, mark: u32, was_key: bool, said: String) {
        let Some(a) = &mut self.active else { return };
        a.seq += 1;
        a.message = None;
        let left = a.session.open_count();
        a.announce = Some((if left == 0 { format!("{said} All checked.") } else { format!("{said} {left} left.") }, a.seq));
        a.voice = None;
        if left > 0 && was_key {
            let next = a.session.open().map(|m| m.id).find(|id| *id > mark).or_else(|| a.session.open().map(|m| m.id).next());
            if let Some(next) = next {
                a.focus_request = Some((next, a.seq));
                a.popover = Some(next);
                if let Some(o) = &mut a.overlay {
                    o.take_focus();
                }
                return self.tick();
            }
        }
        if a.keyboard {
            a.keyboard = false;
            if let Some(o) = &mut a.overlay {
                o.release_focus();
            }
            a.tracked.refocus();
        }
        self.tick();
    }

    /// Keyboard: open the next (or previous) open mark, bringing the marks
    /// back first when they were hidden. Focus moves to the popover.
    fn next_mark(&mut self, from: Option<u32>, back: bool) {
        let Some(a) = &mut self.active else { return };
        if a.session.visibility != Visibility::Shown {
            if !a.session.restore() {
                return;
            }
            a.tracked.refocus();
            a.seq += 1;
            let n = a.session.open_count();
            a.announce = Some((format!("Marks back for the last dictation. {n} to check."), a.seq));
        }
        let open: Vec<u32> = a.session.open().map(|m| m.id).collect();
        if open.is_empty() {
            return;
        }
        let current = from.or(a.popover);
        let pos = current.and_then(|c| open.iter().position(|id| *id == c));
        let id = match (pos, back) {
            (Some(p), false) => open[(p + 1) % open.len()],
            (Some(p), true) => open[(p + open.len() - 1) % open.len()],
            (None, _) => open[0],
        };
        a.seq += 1;
        a.focus_request = Some((id, a.seq));
        a.popover = Some(id);
        a.keyboard = true;
        // Draw first so the popover exists, then hand it the keyboard.
        self.tick();
        if let Some(a) = &mut self.active {
            if let Some(o) = &mut a.overlay {
                o.take_focus();
            }
        }
    }

    fn voice_end(&mut self, text: Option<String>) {
        let Some(a) = &mut self.active else { return };
        let Some(mark) = a.voice.take().or(a.popover) else { return };
        let Some(m) = a.session.get(mark).cloned() else { return };
        let Some(text) = text else {
            return self.render();
        };
        match voice_choice(&text, &m.mark) {
            VoiceChoice::Keep => {
                a.session.keep(mark);
                let was_key = a.keyboard;
                a.popover = None;
                self.after_resolve(mark, was_key, "Kept.".into());
            }
            VoiceChoice::Word(w) => self.choose(mark, &w),
            VoiceChoice::Nothing => self.render(),
        }
    }
}

fn capability_code(c: Capability) -> &'static str {
    match c {
        Capability::Inline => "inline",
        Capability::FallbackReplace => "fallback_replace",
        Capability::FallbackRetype => "fallback_retype",
        Capability::FallbackReadOnly => "fallback_read_only",
        Capability::Nothing => "nothing",
    }
}

fn spans_for(text: &str, marks: &[Mark]) -> Vec<Span> {
    let words = evidence::word_ranges(text);
    marks
        .iter()
        .map(|m| match m.kind {
            MarkKind::Word => Span { start: m.start, end: m.end, edge: Edge::Word },
            MarkKind::Gap => match words.iter().rev().find(|(_, b)| *b <= m.start) {
                Some(&(a, b)) => Span { start: a, end: b, edge: Edge::After },
                None => {
                    let (a, b) = words.first().copied().unwrap_or((0, 0));
                    Span { start: a, end: b, edge: Edge::Before }
                }
            },
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum VoiceChoice {
    Keep,
    Word(String),
    Nothing,
}

/// What a spoken fix means: "keep" keeps the mark; otherwise the words,
/// without the sentence punctuation and capital cleanup added, take the
/// spelling of a matching alternative when there is one.
fn voice_choice(spoken: &str, mark: &Mark) -> VoiceChoice {
    let trimmed = spoken.trim().trim_end_matches(['.', '!', '?', ',', ';', ':']).trim();
    if trimmed.is_empty() {
        return VoiceChoice::Nothing;
    }
    let k = evidence::key(trimmed);
    if matches!(k.as_str(), "keep" | "keepit" | "nothingmissing" | "nothing") {
        return VoiceChoice::Keep;
    }
    if let Some(alt) = mark.alternatives.iter().find(|a| evidence::key(&a.text) == k) {
        return VoiceChoice::Word(alt.text.clone());
    }
    let mut word = trimmed.to_owned();
    let starts_lower = mark.heard.chars().next().is_some_and(char::is_lowercase) || mark.kind == MarkKind::Gap;
    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        let rest: String = chars.collect();
        let single = !word.contains(' ');
        if starts_lower && single && first.is_uppercase() && rest.chars().all(|c| !c.is_uppercase()) {
            word = first.to_lowercase().collect::<String>() + &rest;
        }
    }
    VoiceChoice::Word(word)
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ViewAlt {
    text: String,
    label: &'static str,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ViewMark {
    id: u32,
    kind: MarkKind,
    heard: String,
    alternatives: Vec<ViewAlt>,
    has_audio: bool,
    rects: Vec<RectF>,
    /// UTF-16 offsets into `text`, for the fallback panel.
    from: usize,
    to: usize,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Notice {
    text: String,
    seq: u64,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    session: u64,
    mode: &'static str,
    can_replace: bool,
    retype: bool,
    scale: f64,
    /// The usable part of the monitor, in overlay CSS pixels.
    area: RectF,
    host_dark: bool,
    mark_style: &'static str,
    popover_style: &'static str,
    play_on: &'static str,
    dictation_key: String,
    hold: bool,
    text: String,
    marks: Vec<ViewMark>,
    chip: Option<RectF>,
    focus: Option<Notice>,
    voice: Option<u32>,
    message: Option<Notice>,
    announce: Option<Notice>,
}

fn utf16(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].encode_utf16().count()
}

fn build_view(a: &Active, frame: &Frame, m: &overlay::Monitor, settings: &TrustSettings) -> View {
    let css = |r: &locator::Rect| RectF {
        x: (r.x - m.x) as f64 / m.scale,
        y: (r.y - m.y) as f64 / m.scale,
        w: r.w as f64 / m.scale,
        h: r.h as f64 / m.scale,
    };
    let inline = a.capability.inline();
    let marks = a
        .session
        .open()
        .map(|sm| {
            let index = a.ids.iter().position(|id| *id == sm.id);
            let rects: Vec<RectF> = match (inline, index.and_then(|i| frame.marks.get(i))) {
                (true, Some(Place::Visible(rs))) => match sm.mark.kind {
                    MarkKind::Word => rs.iter().map(css).collect(),
                    MarkKind::Gap => {
                        // A thin insertion marker at the word's edge.
                        let before_first = sm.mark.heard.is_empty();
                        let r = if before_first { rs.first() } else { rs.last() };
                        r.map(|r| {
                            let x = if before_first { r.x } else { r.right() };
                            vec![css(&locator::Rect { x: x - 7, y: r.y, w: 14, h: r.h })]
                        })
                        .unwrap_or_default()
                    }
                },
                _ => Vec::new(),
            };
            ViewMark {
                id: sm.id,
                kind: sm.mark.kind,
                heard: sm.mark.heard.clone(),
                alternatives: sm.mark.alternatives.iter().map(|alt| ViewAlt { text: alt.text.clone(), label: alt.source.label() }).collect(),
                has_audio: a.session.has_audio() && sm.mark.audio.is_some(),
                rects,
                from: utf16(&a.session.text, sm.mark.start),
                to: utf16(&a.session.text, sm.mark.end),
            }
        })
        .collect();
    let chip = (!inline).then(|| frame.anchor.as_ref().map(css)).flatten();
    let notice = |n: &Option<(String, u64)>| n.as_ref().map(|(text, seq)| Notice { text: text.clone(), seq: *seq });
    View {
        session: a.session.id,
        mode: if inline { "inline" } else { "chip" },
        can_replace: a.capability.can_replace(),
        retype: a.capability == Capability::FallbackRetype,
        scale: m.scale,
        area: RectF { x: (m.work.x - m.x) as f64 / m.scale, y: (m.work.y - m.y) as f64 / m.scale, w: m.work.w as f64 / m.scale, h: m.work.h as f64 / m.scale },
        host_dark: a.host_dark.unwrap_or(false),
        mark_style: settings.mark_style.as_str(),
        popover_style: settings.popover_style.as_str(),
        play_on: settings.play_on.as_str(),
        dictation_key: a.dictation_key.clone(),
        hold: a.hold,
        text: a.session.text.clone(),
        marks,
        chip,
        focus: a.focus_request.map(|(mark, seq)| Notice { text: mark.to_string(), seq }),
        voice: a.voice,
        message: notice(&a.message),
        announce: notice(&a.announce),
    }
}

fn notice(app: &AppHandle, text: &str) {
    crate::app::set_delivery_notice(app, text);
}

/// Whether the field is dark: a few background pixels just inside its
/// corners (colour only, never text), else the system's app theme.
fn host_dark(field: Option<locator::Rect>) -> bool {
    #[cfg(windows)]
    {
        if let Some(f) = field.filter(|f| f.w > 12 && f.h > 12) {
            use windows_sys::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
            let points = [(f.x + 4, f.y + 4), (f.right() - 6, f.y + 4), (f.x + 4, f.bottom() - 6), (f.right() - 6, f.bottom() - 6)];
            let mut light = Vec::new();
            unsafe {
                let dc = GetDC(std::ptr::null_mut());
                for (x, y) in points {
                    let c = GetPixel(dc, x, y);
                    if c != 0xFFFF_FFFF {
                        let (r, g, b) = ((c & 0xFF) as f32, ((c >> 8) & 0xFF) as f32, ((c >> 16) & 0xFF) as f32);
                        light.push((0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0);
                    }
                }
                ReleaseDC(std::ptr::null_mut(), dc);
            }
            if !light.is_empty() {
                light.sort_by(f32::total_cmp);
                return light[light.len() / 2] < 0.45;
            }
        }
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
            .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
            .is_ok_and(|v| v == 0)
    }
    #[cfg(not(windows))]
    {
        let _ = field;
        false
    }
}

/// Platforms without a locator: a chip near the top right of the screen,
/// choices copied to the clipboard.
#[cfg(not(windows))]
struct ChipOnly;

#[cfg(not(windows))]
impl Locator for ChipOnly {
    fn attach(&mut self, _: &Request) -> Result<Box<dyn Tracked>, String> {
        Ok(Box::new(ChipTracked))
    }
}

#[cfg(not(windows))]
struct ChipTracked;

#[cfg(not(windows))]
impl Tracked for ChipTracked {
    fn capability(&self) -> Capability {
        Capability::FallbackReadOnly
    }
    fn poll(&mut self) -> Frame {
        Frame { focus: Focus::InField, marks: Vec::new(), field: None, anchor: Some(locator::Rect { x: 0, y: 0, w: 1, h: 1 }), text_gone: false }
    }
    fn replace(&mut self, _: usize, _: &str) -> Result<(), String> {
        Err("unsupported".into())
    }
    fn refocus(&mut self) -> bool {
        false
    }
}

/// Debug: paste a dictation with chosen marks into the field in front, so
/// the whole flow can be tried without speaking. Reached through
/// `sonojot-cli invoke trust_debug_inject '{...}'`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugMark {
    /// The word to mark; for a gap, the word the gap follows.
    pub word: String,
    #[serde(default)]
    pub gap: bool,
    /// Which occurrence of the word, from 1.
    #[serde(default)]
    pub occurrence: Option<usize>,
    #[serde(default)]
    pub alternatives: Vec<DebugAlternative>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugAlternative {
    pub text: String,
    #[serde(default)]
    pub source: Option<AltSource>,
}

pub fn debug_marks(text: &str, marks: &[DebugMark]) -> Result<(Vec<Mark>, Arc<Vec<f32>>), String> {
    let words = evidence::word_ranges(text);
    let seconds = (words.len() as f32 * 0.35 + 1.0).min(MAX_AUDIO_SECONDS as f32);
    // A soft, speech-like hum so play and the waveform have something to show.
    let audio: Vec<f32> = (0..(seconds * 16_000.0) as usize)
        .map(|i| {
            let t = i as f32 / 16_000.0;
            let syllable = (0.5 + 0.5 * (t * 2.0 * std::f32::consts::PI * 3.2).sin()).powi(2);
            0.18 * syllable * ((t * 2.0 * std::f32::consts::PI * 180.0).sin() + 0.4 * (t * 2.0 * std::f32::consts::PI * 360.0).sin())
        })
        .collect();
    let mut out = Vec::new();
    for m in marks {
        let nth = m.occurrence.unwrap_or(1).max(1);
        let index = words
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| evidence::key(&text[*a..*b]) == evidence::key(&m.word))
            .nth(nth - 1)
            .map(|(i, _)| i)
            .ok_or_else(|| format!("\"{}\" is not in the text.", m.word))?;
        let (a, b) = words[index];
        let t0 = index as f32 * 0.35 + 0.5;
        out.push(Mark {
            kind: if m.gap { MarkKind::Gap } else { MarkKind::Word },
            start: if m.gap { b } else { a },
            end: b,
            heard: text[a..b].to_owned(),
            alternatives: m
                .alternatives
                .iter()
                .map(|alt| evidence::Alternative { text: alt.text.clone(), source: alt.source.unwrap_or(AltSource::NextGuess) })
                .collect(),
            audio: Some(if m.gap { (t0 + 0.35, t0 + 1.0) } else { (t0, t0 + 0.35) }),
            score: 1.0,
        });
    }
    out.sort_by_key(|m| m.start);
    Ok((out, Arc::new(audio)))
}

pub fn inject(text: String, marks: Vec<Mark>, audio: Arc<Vec<f32>>, target: Target, app: Option<String>, dictation_key: String, hold: bool) {
    send(Msg::Pasted(Box::new(Pasted { text, source: Source::Marks(marks, Some(audio)), target, app, vocabulary: Vec::new(), dictation_key, hold })));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(heard: &str, kind: MarkKind) -> Mark {
        Mark { kind, start: 0, end: heard.len(), heard: heard.into(), alternatives: vec![evidence::Alternative { text: "pH".into(), source: AltSource::Vocabulary }], audio: None, score: 1.0 }
    }

    #[test]
    fn voice_fixes_drop_sentence_cleanup_and_follow_alternatives() {
        assert_eq!(voice_choice("Keep.", &mark("peach", MarkKind::Word)), VoiceChoice::Keep);
        assert_eq!(voice_choice("Nothing missing.", &mark("vendor", MarkKind::Gap)), VoiceChoice::Keep);
        assert_eq!(voice_choice("Ph.", &mark("peach", MarkKind::Word)), VoiceChoice::Word("pH".into()));
        assert_eq!(voice_choice("Has.", &mark("vendor", MarkKind::Gap)), VoiceChoice::Word("has".into()));
        assert_eq!(voice_choice("Marguerite.", &mark("Margaret", MarkKind::Word)), VoiceChoice::Word("Marguerite".into()));
        assert_eq!(voice_choice("Thirty.", &mark("thirteen", MarkKind::Word)), VoiceChoice::Word("thirty".into()));
        assert_eq!(voice_choice("  ", &mark("x", MarkKind::Word)), VoiceChoice::Nothing);
    }

    #[test]
    fn marks_come_from_capture_evidence_on_the_pasted_text() {
        let raw_text = "i also want to add a line about the castrel rollout since the vendor confirmed the new dates";
        let words = evidence::word_ranges(raw_text);
        let raw_words = words
            .iter()
            .enumerate()
            .map(|(i, (a, b))| RawWord {
                word: raw_text[*a..*b].into(),
                confidence: Some(if &raw_text[*a..*b] == "castrel" { 0.2 } else { 0.97 }),
                time: Some((i as f32 * 0.3, i as f32 * 0.3 + 0.28)),
            })
            .collect();
        let capture = Capture { raw_text: raw_text.into(), raw_words, segments: vec![], audio: None };
        let pasted = "I also want to add a line about the Castrel rollout, since the vendor confirmed the new dates.";
        let marks = marks_for(pasted, &capture, &["Kestrel".into()], decision::Sensitivity::Balanced);
        assert_eq!(marks.len(), 1);
        assert_eq!(&pasted[marks[0].start..marks[0].end], "Castrel");
        assert_eq!(marks[0].alternatives[0].text, "Kestrel");
        let spans = spans_for(pasted, &marks);
        assert_eq!(spans[0].edge, Edge::Word);
    }

    #[test]
    fn gap_spans_point_at_their_neighbouring_word() {
        let text = "since the vendor confirmed";
        let at = text.find("vendor").unwrap() + 6;
        let gap = Mark { kind: MarkKind::Gap, start: at, end: at, heard: "vendor".into(), alternatives: vec![], audio: None, score: 1.0 };
        let spans = spans_for(text, &[gap]);
        assert_eq!(&text[spans[0].start..spans[0].end], "vendor");
        assert_eq!(spans[0].edge, Edge::After);
        let first = Mark { kind: MarkKind::Gap, start: 0, end: 0, heard: String::new(), alternatives: vec![], audio: None, score: 1.0 };
        assert_eq!(spans_for(text, &[first])[0].edge, Edge::Before);
    }

    #[test]
    fn debug_marks_find_words_and_gaps() {
        let (marks, audio) = debug_marks(
            "the vendor confirmed the plan",
            &[
                DebugMark { word: "the".into(), gap: false, occurrence: Some(2), alternatives: vec![] },
                DebugMark { word: "vendor".into(), gap: true, occurrence: None, alternatives: vec![DebugAlternative { text: "has".into(), source: None }] },
            ],
        )
        .unwrap();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].kind, MarkKind::Gap);
        assert_eq!(marks[1].start, "the vendor confirmed ".len());
        assert!(!audio.is_empty());
        assert!(debug_marks("abc", &[DebugMark { word: "zzz".into(), gap: false, occurrence: None, alternatives: vec![] }]).is_err());
    }
}
