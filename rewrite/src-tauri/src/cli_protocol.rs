//! The local channel between `sonojot-cli` and the running app.
//!
//! One request per connection: a JSON line in, a JSON line out. The endpoint
//! is a named pipe on Windows that refuses remote clients, and a Unix socket
//! in the per-user data folder with owner-only permissions elsewhere.
//! Commands use the same names and camelCase arguments as the app's own
//! window, so anything the GUI can do is reachable here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped when the request or response shape changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;
/// Requests carry settings or vocabulary at most; refuse anything larger.
pub const MAX_MESSAGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub version: u32,
    pub command: String,
    #[serde(default)]
    pub args: Value,
    /// Reply as soon as the command starts. Long jobs (transcription, notes)
    /// keep running in the app; poll `get_meeting` for their status.
    #[serde(default)]
    pub detach: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn from_result(result: Result<Value, String>) -> Self {
        match result {
            Ok(value) => Self { ok: true, result: Some(value), error: None },
            Err(error) => Self { ok: false, result: None, error: Some(error) },
        }
    }
}

/// `\\.\pipe\SonoJot-cli-<user>` on Windows.
#[cfg(windows)]
pub fn endpoint() -> std::path::PathBuf {
    let user: String = std::env::var("USERNAME").unwrap_or_default().chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64).collect();
    format!(r"\\.\pipe\SonoJot-cli-{}", if user.is_empty() { "user" } else { &user }).into()
}

/// `cli.sock` in the SonoJot data folder elsewhere.
#[cfg(unix)]
pub fn endpoint() -> std::path::PathBuf {
    data_dir().join("cli.sock")
}

/// The per-user SonoJot data folder, found without opening the library.
pub fn data_dir() -> std::path::PathBuf {
    let parent = if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|_| cfg!(windows)) {
        std::path::PathBuf::from(local)
    } else if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|_| cfg!(not(target_os = "macos"))) {
        std::path::PathBuf::from(data)
    } else {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { home.join(".local/share") }
    };
    parent.join(crate::store::DATA_DIR_NAME)
}

/// Why a call failed: the app isn't listening, or it answered with an error.
#[derive(Debug)]
pub enum CallError {
    NotRunning(String),
    Failed(String),
}

/// Sends one request and waits for the reply. Blocking; used by the CLI.
pub fn call(request: &Request) -> Result<Value, CallError> {
    use std::io::{BufRead, BufReader, Read, Write};
    let mut stream = connect()?;
    let mut line = serde_json::to_vec(request).map_err(|e| CallError::Failed(e.to_string()))?;
    line.push(b'\n');
    stream.write_all(&line).and_then(|_| stream.flush())
        .map_err(|e| CallError::Failed(format!("Cannot send the request to SonoJot: {e}")))?;
    let mut reply = String::new();
    BufReader::new(stream.take(MAX_MESSAGE_BYTES * 4)).read_line(&mut reply)
        .map_err(|e| CallError::Failed(format!("SonoJot closed the connection: {e}")))?;
    if reply.trim().is_empty() {
        return Err(CallError::Failed("SonoJot closed the connection without replying.".into()));
    }
    let response: Response = serde_json::from_str(&reply)
        .map_err(|e| CallError::Failed(format!("SonoJot sent an unreadable reply: {e}")))?;
    if response.ok {
        Ok(response.result.unwrap_or(Value::Null))
    } else {
        Err(CallError::Failed(response.error.unwrap_or_else(|| "SonoJot reported an unknown error.".into())))
    }
}

#[cfg(windows)]
fn connect() -> Result<std::fs::File, CallError> {
    const ERROR_PIPE_BUSY: i32 = 231;
    let path = endpoint();
    let started = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => return Ok(file),
            // Every server instance is mid-handoff; a new one appears within milliseconds.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && started.elapsed().as_secs() < 10 => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(CallError::NotRunning(e.to_string())),
        }
    }
}

#[cfg(unix)]
fn connect() -> Result<std::os::unix::net::UnixStream, CallError> {
    std::os::unix::net::UnixStream::connect(endpoint()).map_err(|e| CallError::NotRunning(e.to_string()))
}
