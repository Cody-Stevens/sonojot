//! Explicitly submitted reports. No background uploads and no transcript/store export.
use crate::{app::Runtime, domain::Settings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::State;

const MAX_REPORT: usize = 64 * 1024;
const MAX_BACKTRACE: usize = 16 * 1024;
const OMITTED_MESSAGE: &str = "Rust panic (message omitted because it may contain private data)";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReportKind {
    Crash,
    Bug,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportApp {
    version: String,
    platform: String,
    arch: String,
    os: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashDetail {
    summary: String,
    location: String,
    backtrace: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    schema: u8,
    kind: ReportKind,
    request_id: String,
    app: ReportApp,
    notes: String,
    crash: Option<CrashDetail>,
    diagnostics: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    receipt_id: String,
    replayed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingCrash {
    pub id: String,
    /// A marker alone cannot distinguish a native crash from a forced quit or power loss.
    pub reason: CrashReason,
    app: ReportApp,
    crash: CrashDetail,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CrashReason {
    Panic,
    UnexpectedExit,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    pid: u32,
    version: String,
    started_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PanicEvidence {
    #[serde(rename = "category", alias = "message")]
    message: String,
    location: String,
    thread: String,
    version: String,
    platform: String,
    at: String,
    backtrace: String,
}

pub struct CrashReports {
    directory: PathBuf,
    pending: Mutex<Option<PendingCrash>>,
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn capped(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

/// Panic formatting can embed transcripts, titles, debug structs or file names.
/// Keep only recognized runtime categories, never arbitrary payload values.
fn panic_message(message: &str) -> String {
    for prefix in [
        "called `Option::unwrap()` on a `None` value",
        "called `Result::unwrap()` on an `Err` value",
        "index out of bounds",
        "attempt to divide by zero",
        "attempt to add with overflow",
        "attempt to subtract with overflow",
        "attempt to multiply with overflow",
        "assertion `left == right` failed",
        "assertion `left != right` failed",
        "internal error: entered unreachable code",
        "not yet implemented",
        "explicit panic",
    ] {
        if message.starts_with(prefix) {
            return prefix.to_owned();
        }
    }
    OMITTED_MESSAGE.into()
}

/// Backtraces contain symbols and source locations, not stack memory. Remove
/// absolute paths (including the build machine's home), not just today's HOME.
fn safe_backtrace(value: &str) -> String {
    static PATH: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let path = PATH.get_or_init(|| {
        regex::Regex::new(r#"(?:[A-Za-z]:[\\/]|/|\\\\)[^\r\n]+"#).expect("path pattern")
    });
    let sanitized = value
        .lines()
        .map(|line| {
            if let Some(location) = line.trim().strip_prefix("at ") {
                // Keep source file and line/column, never any build-directory path.
                let name = location.rsplit(['/', '\\']).next().unwrap_or("");
                if name.len() <= 200
                    && name.contains(".rs:")
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-:".contains(&b))
                {
                    return format!("    at {name}");
                }
                return "    at [path omitted]".into();
            }
            path.replace_all(line, "[path omitted]").into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    capped(&sanitized, MAX_BACKTRACE)
}

fn source_location(file: &str, line: u32) -> String {
    // The file name is enough for grouping; directory names may identify a user.
    let name = file.rsplit(['/', '\\']).next().unwrap_or("unknown");
    let name = if name.len() <= 160
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        name
    } else {
        "unknown"
    };
    format!("{name}:{line}")
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take((MAX_REPORT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_REPORT {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not clear the saved crash report. Try again.".into()),
    }
}

impl CrashReports {
    /// Called only from desktop setup, after workers and secondary instances exit.
    pub fn start(root: &Path) -> Result<Self, String> {
        let directory = root.join("crash");
        fs::create_dir_all(&directory).map_err(|_| "Could not create the crash report folder.")?;
        let panic_path = directory.join("last-panic.json");
        let session_path = directory.join("session.json");
        let mut pending = read_json::<PendingCrash>(&directory.join("pending.json"));
        if panic_path.exists() || (pending.is_none() && session_path.exists()) {
            let panic = read_json::<PanicEvidence>(&panic_path);
            let session = read_json::<Session>(&session_path);
            let mut app = report_app();
            if let Some(ref panic) = panic {
                app.version = capped(&panic.version, 80);
            } else if let Some(session) = session {
                app.version = capped(&session.version, 80);
            }
            pending = Some(PendingCrash {
                id: uuid::Uuid::new_v4().to_string(),
                reason: if panic_path.exists() { CrashReason::Panic } else { CrashReason::UnexpectedExit },
                app,
                crash: panic.map(|p| CrashDetail {
                    summary: panic_message(&p.message),
                    location: capped(&safe_backtrace(&p.location), 300),
                    backtrace: safe_backtrace(&p.backtrace),
                }).unwrap_or(CrashDetail {
                    summary: if panic_path.exists() { "Rust panic (details unavailable)" } else {
                        "Previous session did not exit cleanly; a crash, forced quit or power loss may have caused this."
                    }.into(),
                    location: String::new(), backtrace: String::new(),
                }),
            });
        }
        if let Some(ref pending) = pending {
            crate::store::atomic_write(
                &directory.join("pending.json"),
                &serde_json::to_vec(pending).map_err(|e| e.to_string())?,
            )?;
        }
        // Move the previous evidence into pending.json before writing this run's
        // marker. Dismissal must never erase the current session or a new panic.
        remove_if_present(&panic_path)?;
        crate::store::atomic_write(
            &session_path,
            &serde_json::to_vec(&Session {
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").into(),
                started_at: now(),
            })
            .map_err(|e| e.to_string())?,
        )?;
        Ok(Self {
            directory,
            pending: Mutex::new(pending),
        })
    }

    pub fn install_hook(&self) {
        let path = self.directory.join("last-panic.json");
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
                .unwrap_or("");
            let thread = std::thread::current();
            let name = match thread.name() {
                Some("main") => "main",
                Some("tokio-runtime-worker") => "tokio-runtime-worker",
                _ => "app-thread",
            };
            let evidence = PanicEvidence {
                message: panic_message(message),
                location: info
                    .location()
                    .map(|l| source_location(l.file(), l.line()))
                    .unwrap_or_default(),
                thread: name.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                platform: std::env::consts::OS.into(),
                at: now(),
                backtrace: safe_backtrace(&std::backtrace::Backtrace::force_capture().to_string()),
            };
            // No store mutex: the panic may have happened while holding it.
            if let Ok(bytes) = serde_json::to_vec(&evidence) {
                let _ = fs::write(&path, bytes);
            }
            previous(info);
        }));
    }

    pub fn pending(&self) -> Result<Option<PendingCrash>, String> {
        self.pending
            .lock()
            .map(|p| p.clone())
            .map_err(|_| "Crash report state is unavailable.".into())
    }

    pub fn clean_exit(&self) {
        let _ = remove_if_present(&self.directory.join("session.json"));
    }

    fn dismiss(&self, id: &str) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "Crash report state is unavailable.")?;
        if pending.as_ref().is_some_and(|p| p.id == id) {
            remove_if_present(&self.directory.join("pending.json"))?;
            *pending = None;
        }
        Ok(())
    }
}

fn report_app() -> ReportApp {
    ReportApp {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        os: os_version(),
    }
}

fn os_version() -> String {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        {
            let build: String = key.get_value("CurrentBuildNumber").unwrap_or_default();
            let release: String = key.get_value("DisplayVersion").unwrap_or_default();
            return capped(&format!("Windows {release} (build {build})"), 200);
        }
    }
    #[cfg(target_os = "linux")]
    {
        // PRETTY_NAME is distribution metadata, not hostname or user identity.
        if let Ok(text) = fs::read_to_string("/etc/os-release") {
            if let Some(name) = text.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")) {
                return capped(name.trim_matches('"'), 200);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // Fixed system metadata; do not collect uname's node/host name.
        let mut bytes = [0u8; 128];
        let mut len = bytes.len();
        let result = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if result == 0 && len <= bytes.len() {
            return format!(
                "macOS {}",
                String::from_utf8_lossy(&bytes[..len]).trim_end_matches('\0')
            );
        }
    }
    std::env::consts::OS.into()
}

fn token(value: &str, max: usize) -> bool {
    value.len() <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn diagnostic_event(value: Value) -> Option<Value> {
    let object = value.as_object()?;
    let event = object.get("event")?.as_str()?;
    let model = object.get("model")?.as_str()?;
    if !token(event, 80) || !token(model, 80) {
        return None;
    }
    let mut result = serde_json::Map::new();
    for key in ["v", "at_ms", "pid", "session", "ms", "value"] {
        if let Some(n) = object.get(key).and_then(Value::as_u64) {
            result.insert(key.into(), json!(n));
        }
    }
    result.insert("event".into(), json!(event));
    result.insert("model".into(), json!(model));
    // Nested capture data is deliberately omitted; never copy arbitrary objects.
    Some(Value::Object(result))
}

fn recent_events(root: &Path) -> Vec<Value> {
    let Ok(mut file) = File::open(root.join("diagnostics/recognition.jsonl")) else {
        return vec![];
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let offset = len.saturating_sub(128 * 1024);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return vec![];
    }
    let mut bytes = Vec::new();
    if file.take(128 * 1024).read_to_end(&mut bytes).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    if offset > 0 {
        lines.next();
    }
    let mut events: Vec<_> = lines
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter_map(diagnostic_event)
        .rev()
        .take(100)
        .collect();
    events.reverse();
    events
}

fn diagnostics(root: &Path, settings: &Settings) -> Value {
    let hardware = crate::resource_manager::setup_hardware();
    let model = |id: &str| {
        if token(id, 80) {
            id.to_owned()
        } else {
            "unknown".into()
        }
    };
    json!({
        "setupProfile": settings.setup_profile,
        "models": { "speech": model(&settings.asr_model), "cleanup": model(&settings.cleanup_model),
            "notes": model(&settings.notes_model), "meetingSpeech": model(&settings.meeting_asr_model) },
        "hardware": { "ramBytes": hardware.total_ram_bytes, "physicalCores": hardware.physical_cores,
            "logicalCores": hardware.logical_cores,
            "gpu": hardware.gpu.and_then(|g| g.name).map(|s| capped(&safe_backtrace(&s), 200)) },
        "events": recent_events(root)
    })
}

fn draft(
    kind: ReportKind,
    pending: Option<PendingCrash>,
    diagnostics: Option<Value>,
) -> Result<Report, String> {
    let (app, crash) = match kind {
        ReportKind::Bug => (report_app(), None),
        ReportKind::Crash => {
            let pending = pending.ok_or("There is no saved crash report.")?;
            (pending.app, Some(pending.crash))
        }
    };
    let mut report = Report {
        schema: 1,
        kind,
        request_id: uuid::Uuid::new_v4().to_string(),
        app,
        notes: String::new(),
        crash,
        diagnostics,
    };
    // Reserve worst-case JSON space for 4,000 notes characters (control escapes).
    while serde_json::to_vec(&report)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_REPORT - 24_000
    {
        let events = report
            .diagnostics
            .as_mut()
            .and_then(|d| d.get_mut("events"))
            .and_then(Value::as_array_mut);
        if let Some(events) = events.filter(|e| !e.is_empty()) {
            events.remove(0);
        } else if let Some(crash) = report.crash.as_mut().filter(|c| !c.backtrace.is_empty()) {
            crash.backtrace = capped(&crash.backtrace, crash.backtrace.len() / 2);
        } else {
            return Err("The report is too large.".into());
        }
    }
    Ok(report)
}

fn report_bytes(report: &Report) -> Result<Vec<u8>, String> {
    let uuid = uuid::Uuid::parse_str(&report.request_id).map_err(|_| "Invalid report ID.")?;
    if report.schema != 1
        || uuid.get_version_num() != 4
        || uuid.get_variant() != uuid::Variant::RFC4122
        || uuid.hyphenated().to_string() != report.request_id.to_ascii_lowercase()
        || report.notes.chars().count() > 4000
        || (report.kind == ReportKind::Bug
            && (report.notes.trim().is_empty() || report.crash.is_some()))
        || (report.kind == ReportKind::Crash && report.crash.is_none())
        || report.app.version.is_empty()
        || report.app.version.len() > 80
        || !matches!(report.app.platform.as_str(), "windows" | "macos" | "linux")
        || report.app.arch.is_empty()
        || report.app.arch.len() > 40
        || report.app.os.len() > 200
        || report.crash.as_ref().is_some_and(|c| {
            c.summary.len() > 2000 || c.location.len() > 300 || c.backtrace.len() > MAX_BACKTRACE
        })
        || report.diagnostics.as_ref().is_some_and(|d| !d.is_object())
    {
        return Err("Check the report fields before sending.".into());
    }
    let bytes = serde_json::to_vec(report).map_err(|_| "Could not encode the report.")?;
    if bytes.len() > MAX_REPORT {
        return Err("The report exceeds 64 KB. Shorten your notes.".into());
    }
    Ok(bytes)
}

async fn submit(report: &Report) -> Result<Receipt, String> {
    let body = report_bytes(report)?;
    let url = std::env::var("SONOJOT_REPORT_URL")
        .unwrap_or_else(|_| "https://sonojot.com/api/report".into());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not start the report connection.")?;
    let mut response = client
        .post(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|_| "Could not send the report. Try again or copy it below.")?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the report receipt.")?
    {
        if bytes.len() + chunk.len() > 4096 {
            return Err("The report service returned an invalid receipt.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if status == reqwest::StatusCode::ACCEPTED {
        let receipt: Receipt = serde_json::from_slice(&bytes)
            .map_err(|_| "The report service returned an invalid receipt.")?;
        if receipt.receipt_id.is_empty() || !token(&receipt.receipt_id, 160) {
            return Err("The report service returned an invalid receipt.".into());
        }
        Ok(receipt)
    } else {
        Err(service_error(status.as_u16(), &bytes).into())
    }
}

fn service_error(status: u16, bytes: &[u8]) -> &'static str {
    // Only recognize this fixed message; never echo arbitrary server contents.
    let disabled = serde_json::from_slice::<Value>(bytes)
        .ok()
        .is_some_and(|body| {
            body.get("error").and_then(Value::as_str) == Some("Reports are not enabled yet.")
        });
    match status {
        429 => "Too many reports. Wait a minute and try again.",
        503 if disabled => "Reports are not enabled yet.",
        503 => "Reports are temporarily unavailable. Try again or copy it below.",
        413 => "The report exceeds 64 KB. Shorten your notes.",
        _ => "The report service could not accept this report. Try again or copy it below.",
    }
}

#[tauri::command]
pub fn get_pending_crash(state: State<'_, CrashReports>) -> Result<Option<PendingCrash>, String> {
    state.pending()
}

#[tauri::command]
pub fn dismiss_crash(state: State<'_, CrashReports>, id: String) -> Result<(), String> {
    state.dismiss(&id)
}

#[tauri::command]
pub async fn prepare_report(
    runtime: State<'_, Arc<Runtime>>,
    state: State<'_, CrashReports>,
    kind: ReportKind,
    include_diagnostics: bool,
) -> Result<Report, String> {
    let pending = state.pending()?;
    let runtime = runtime.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let details = if include_diagnostics {
            Some(diagnostics(
                runtime.store.root(),
                &runtime.snapshot()?.settings,
            ))
        } else {
            None
        };
        draft(kind, pending, details)
    })
    .await
    .map_err(|_| "Could not prepare the report.")?
}

#[tauri::command]
pub async fn send_report(report: Report) -> Result<Receipt, String> {
    submit(&report).await
}

#[tauri::command]
pub async fn copy_report(report: Report) -> Result<(), String> {
    report_bytes(&report)?;
    let text = serde_json::to_string_pretty(&report).map_err(|_| "Could not copy the report.")?;
    crate::platform::copy_text(&text).await
}

#[tauri::command]
pub async fn open_report_issues() -> Result<(), String> {
    crate::legal_links::open_https(
        "https://github.com/Cody-Stevens/sonojot/issues".into(),
        "issue tracker",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn service_errors_distinguish_disabled_from_outage_without_echoing_data() {
        assert_eq!(
            service_error(503, br#"{"error":"Reports are not enabled yet."}"#),
            "Reports are not enabled yet."
        );
        assert!(
            service_error(503, br#"{"error":"private upstream details"}"#)
                .contains("temporarily unavailable")
        );
        assert!(!service_error(500, b"private transcript").contains("private"));
    }

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("sonojot-reports-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn panic_file_produces_crash_view_state_and_dismiss_keeps_current_marker() {
        let root = Temp::new();
        let first = CrashReports::start(&root.0).unwrap();
        let evidence = PanicEvidence {
            message: "called `Result::unwrap()` on an `Err` value: private transcript".into(),
            location: "speech.rs:42".into(),
            thread: "main".into(),
            version: "0.28.5".into(),
            platform: "linux".into(),
            at: now(),
            backtrace: "0: sonojot::speech\n at /home/alice/private/speech.rs:42".into(),
        };
        fs::write(
            first.directory.join("last-panic.json"),
            serde_json::to_vec(&evidence).unwrap(),
        )
        .unwrap();
        let second = CrashReports::start(&root.0).unwrap();
        let pending = second.pending().unwrap().unwrap();
        assert_eq!(pending.reason, CrashReason::Panic);
        let report = draft(ReportKind::Crash, Some(pending.clone()), None).unwrap();
        let text = String::from_utf8(report_bytes(&report).unwrap()).unwrap();
        assert!(text.contains("0.28.5"));
        assert!(!text.contains("alice"));
        assert!(!text.contains("private transcript"));
        second.dismiss("stale-id").unwrap();
        assert!(second.pending().unwrap().is_some());
        second.dismiss(&pending.id).unwrap();
        assert!(second.directory.join("session.json").exists());
        second.clean_exit();
        let third = CrashReports::start(&root.0).unwrap();
        assert!(third.pending().unwrap().is_none());
        third.clean_exit();
    }

    #[test]
    fn stale_session_is_honest_and_survives_clean_exit_until_acknowledged() {
        let root = Temp::new();
        CrashReports::start(&root.0).unwrap();
        let next = CrashReports::start(&root.0).unwrap();
        let pending = next.pending().unwrap().unwrap();
        assert_eq!(pending.reason, CrashReason::UnexpectedExit);
        assert!(pending.crash.summary.contains("power loss"));
        next.clean_exit();
        let again = CrashReports::start(&root.0).unwrap();
        assert_eq!(again.pending().unwrap().unwrap().id, pending.id);
        again.dismiss(&pending.id).unwrap();
        again.clean_exit();
        assert!(CrashReports::start(&root.0)
            .unwrap()
            .pending()
            .unwrap()
            .is_none());
    }

    #[test]
    fn panic_payloads_and_absolute_paths_are_not_copied() {
        assert_eq!(
            panic_message("meeting title: confidential"),
            OMITTED_MESSAGE
        );
        assert_eq!(
            panic_message("assertion `left == right` failed\nleft: private words"),
            "assertion `left == right` failed"
        );
        for path in [
            r"C:\Users\alice\Documents\secret.rs",
            "/home/alice/private.rs",
            "/Users/alice/private.rs",
            r"\\server\alice\private.rs",
        ] {
            assert!(
                !safe_backtrace(&format!("0: module::function\n at {path}:55\n1: another"))
                    .contains("alice")
            );
            assert!(!source_location(path, 55).contains("alice"));
        }
        assert!(safe_backtrace(&"é".repeat(MAX_BACKTRACE)).len() <= MAX_BACKTRACE);
        assert_eq!(
            safe_backtrace("0: sonojot::speech\n at /home/alice/src/speech.rs:55:9"),
            "0: sonojot::speech\n    at speech.rs:55:9"
        );
    }

    #[test]
    fn diagnostic_tail_is_bounded_and_only_known_metadata_leaves() {
        let root = Temp::new();
        fs::create_dir(root.0.join("diagnostics")).unwrap();
        let mut file = File::create(root.0.join("diagnostics/recognition.jsonl")).unwrap();
        for n in 0..150 {
            writeln!(
                file,
                "{}",
                json!({"v":1,"event":"decode-complete","model":"moonshine-medium", "value":n,
                "text":"private transcript", "path":"/home/alice", "capture":{"text":"secret"}})
            )
            .unwrap();
        }
        let events = recent_events(&root.0);
        assert_eq!(events.len(), 100);
        assert_eq!(events[0]["value"], 50);
        assert!(!serde_json::to_string(&events).unwrap().contains("private"));
        assert!(diagnostic_event(json!({"event":"load", "model":"C:\\private"})).is_none());
        assert!(diagnostic_event(json!({"event":"load /home/alice", "model":"model"})).is_none());
    }

    #[test]
    fn report_budget_reserves_notes_and_validation_rejects_bad_contracts() {
        let mut report = draft(
            ReportKind::Bug,
            None,
            Some(json!({"events": vec![json!({"value":"x".repeat(1000)});100]})),
        )
        .unwrap();
        report.notes = "\u{0001}".repeat(4000);
        assert!(report_bytes(&report).unwrap().len() <= MAX_REPORT);
        report.notes.push('x');
        assert!(report_bytes(&report).is_err());
        report.notes = "A bug".into();
        report.schema = 2;
        assert!(report_bytes(&report).is_err());
        report.schema = 1;
        report.request_id = "not-a-uuid".into();
        assert!(report_bytes(&report).is_err());
    }

    #[test]
    fn sends_exact_payload_to_local_endpoint_using_environment_override() {
        // This is the only test changing this dedicated override; no real relay is contacted.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/report", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let mut report = draft(ReportKind::Bug, None, None).unwrap();
        report.notes = "Test report".into();
        let expected = report_bytes(&report).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("local report listener: {e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            let mut byte = [0u8; 1];
            while !received.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                received.push(byte[0]);
            }
            let headers = String::from_utf8(received).unwrap().to_lowercase();
            assert!(headers.starts_with("post /api/report http/1.1"));
            assert!(headers.contains("content-type: application/json"));
            let mut body = vec![0; expected.len()];
            socket.read_exact(&mut body).unwrap();
            assert_eq!(body, expected);
            let receipt = r#"{"receiptId":"local-test-receipt","replayed":false}"#;
            write!(socket, "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", receipt.len(), receipt).unwrap();
        });
        let old = std::env::var_os("SONOJOT_REPORT_URL");
        std::env::set_var("SONOJOT_REPORT_URL", url);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(submit(&report));
        if let Some(old) = old {
            std::env::set_var("SONOJOT_REPORT_URL", old);
        } else {
            std::env::remove_var("SONOJOT_REPORT_URL");
        }
        server.join().unwrap();
        assert_eq!(result.unwrap().receipt_id, "local-test-receipt");
    }
}
