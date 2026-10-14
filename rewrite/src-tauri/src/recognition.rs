//! Versioned local IPC for one native model per owned process. The desktop
//! process never loads recognition DLLs. A child can die at any native call.
use crate::{
    asr::{RecognitionResult, Recognizer},
    domain::RecognitionCompleteness,
    recognition_diagnostics as diag,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_SAMPLES: usize = 16_384;
const MAX_TEXT: usize = 1024 * 1024;
const REPLAY_QUOTA: u64 = 64 * 1024 * 1024;
// Qwen's measured cold load is ~94 seconds. These are hard deadlines, not
// heartbeat checks after a native call has already returned.
const LOAD_LIMIT: Duration = Duration::from_secs(240);
const DECODE_LIMIT: Duration = Duration::from_secs(180);
const START_LIMIT: Duration = Duration::from_secs(15);
const DROP_LIMIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(10);
static NATIVE_WORKER: AtomicBool = AtomicBool::new(false);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
static NEVER_CANCEL: AtomicBool = AtomicBool::new(false);

type ProgressHandler = std::sync::Arc<dyn Fn(crate::domain::RecognitionProgress) + Send + Sync>;
struct ProgressState {
    handler: ProgressHandler,
    stage: &'static str,
    started: Instant,
    last: Instant,
}
thread_local! {
    static PROGRESS: std::cell::RefCell<Option<ProgressState>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct ProgressGuard;
pub(crate) fn observe_progress(handler: ProgressHandler) -> ProgressGuard {
    PROGRESS.with(|slot| {
        *slot.borrow_mut() = Some(ProgressState {
            handler,
            stage: "loading",
            started: Instant::now(),
            last: Instant::now(),
        })
    });
    progress("loading");
    ProgressGuard
}
impl Drop for ProgressGuard {
    fn drop(&mut self) {
        PROGRESS.with(|slot| slot.borrow_mut().take());
    }
}
fn progress(stage: &'static str) {
    PROGRESS.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            if state.stage != stage {
                state.started = Instant::now();
            }
            state.stage = stage;
            state.last = Instant::now();
            (state.handler)(crate::domain::RecognitionProgress {
                stage: stage.into(),
                elapsed_ms: state.started.elapsed().as_millis() as u64,
            });
        }
    });
}
fn progress_tick() {
    PROGRESS.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            if state.last.elapsed() >= Duration::from_millis(500) {
                state.last = Instant::now();
                (state.handler)(crate::domain::RecognitionProgress {
                    stage: state.stage.into(),
                    elapsed_ms: state.started.elapsed().as_millis() as u64,
                });
            }
        }
    });
}

pub(crate) fn is_worker() -> bool {
    NATIVE_WORKER.load(Ordering::Acquire)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    request: u64,
    session: u64,
    op: Operation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Operation {
    Load { root: PathBuf, model: String },
    Begin,
    Audio { samples: Vec<f32>, rate: u32 },
    Finish { observed: bool },
    Abort,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    request: u64,
    session: u64,
    body: Reply,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Reply {
    Loaded {
        model: String,
    },
    Ack,
    Text {
        raw: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_text: Option<String>,
        warnings: Vec<String>,
    },
    Finished {
        result: RecognitionResult,
        details: Option<serde_json::Value>,
    },
    // Native errors may contain paths. Only this fixed code crosses IPC.
    Error,
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "Invalid recognition frame.")?;
    if bytes.len() > MAX_FRAME {
        return Err("Recognition frame exceeds its limit.".into());
    }
    let mut frame = Vec::with_capacity(bytes.len() + 4);
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(&bytes);
    Ok(frame)
}

fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> Result<T, String> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| "Recognition worker disconnected.")?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("Invalid recognition frame size.".into());
    }
    let mut bytes = vec![0; len];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Recognition worker disconnected.")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid recognition frame.".into())
}

fn check_wait(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Recognition cancelled.".into());
    }
    if Instant::now() >= deadline {
        return Err("Recognition worker deadline exceeded.".into());
    }
    progress_tick();
    Ok(())
}

// Both writes and reads are interruptible, including a child that stops
// reading mid-frame. No reader thread can outlive its owned child.
struct Interruptible<'a> {
    socket: &'a mut TcpStream,
    cancel: &'a AtomicBool,
    deadline: Instant,
}
impl Read for Interruptible<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            check_wait(self.cancel, self.deadline).map_err(io::Error::other)?;
            match self.socket.read(bytes) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue
                }
                result => return result,
            }
        }
    }
}
impl Write for Interruptible<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            check_wait(self.cancel, self.deadline).map_err(io::Error::other)?;
            match self.socket.write(bytes) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue
                }
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Shared process containment for native recognition and diarization workers.
pub(crate) struct OwnedChild {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
// The job handle is owned, process-wide and has no thread affinity. All child
// and job operations require exclusive access; moving ownership is safe.
#[cfg(windows)]
unsafe impl Send for OwnedChild {}
impl OwnedChild {
    pub(crate) fn spawn(command: &mut Command) -> Result<Self, String> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = command
            .spawn()
            .map_err(|_| "Cannot start native worker.")?;
        let mut owned = Self {
            child,
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        };
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::*;
            owned.job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if owned.job.is_null() {
                return Err("Cannot contain native worker.".into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                owned.job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(owned.job, owned.child.as_raw_handle() as _) == 0
            {
                return Err("Cannot contain native worker.".into());
            }
        }
        Ok(owned)
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn terminate(&mut self) {
        let _ = self.child.kill();
        let deadline = Instant::now() + DROP_LIMIT;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                break;
            }
            thread::sleep(POLL);
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.terminate();
        #[cfg(windows)]
        if !self.job.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
    }
}

struct Connection {
    socket: TcpStream,
    child: OwnedChild,
    request: u64,
    model: String,
}
impl Connection {
    fn spawn(cancel: &AtomicBool) -> Result<Self, String> {
        #[cfg(test)]
        if let Some(behavior) = tests::next_child() {
            return Self::spawn_command(tests::child_command(behavior), cancel, true);
        }
        let exe = worker_executable()?;
        Self::spawn_command(Command::new(exe), cancel, false)
    }

    fn spawn_command(
        mut command: Command,
        cancel: &AtomicBool,
        test_harness: bool,
    ) -> Result<Self, String> {
        check_wait(cancel, Instant::now() + START_LIMIT)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "Cannot bind local recognition IPC.")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "Cannot configure recognition IPC.")?;
        let address = listener
            .local_addr()
            .map_err(|_| "Cannot address recognition IPC.")?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        if test_harness {
            // Only unit tests pass true. The fixture runs in a separate test
            // executable, never in the process running assertions.
            command.env("SONOJOT_TEST_WORKER_ENDPOINT", address.to_string());
        } else {
            command.arg("--recognition-worker").arg(address.to_string());
        }
        // This token authenticates a local connection, never persisted or logged.
        command.env("SONOJOT_RECOGNITION_TOKEN", &token);
        diag::prepare_worker_diagnostics(&mut command);
        let mut child = OwnedChild::spawn(&mut command)?;
        let deadline = Instant::now() + START_LIMIT;
        loop {
            check_wait(cancel, deadline)?;
            if child
                .child
                .try_wait()
                .map_err(|_| "Cannot inspect recognition worker.")?
                .is_some()
            {
                return Err("Recognition worker exited during startup.".into());
            }
            match listener.accept() {
                Ok((mut socket, peer)) => {
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    // Short socket timeouts wake on data immediately, unlike
                    // sleeping after every nonblocking WouldBlock. Tiny device
                    // callbacks must not acquire a fixed 10 ms IPC delay.
                    socket
                        .set_nonblocking(false)
                        .map_err(|_| "Cannot configure recognition IPC.")?;
                    socket
                        .set_read_timeout(Some(POLL))
                        .map_err(|_| "Cannot configure recognition IPC.")?;
                    socket
                        .set_write_timeout(Some(POLL))
                        .map_err(|_| "Cannot configure recognition IPC.")?;
                    let mut proof = [0; 32];
                    let mut io = Interruptible {
                        socket: &mut socket,
                        cancel,
                        deadline,
                    };
                    if io.read_exact(&mut proof).is_err() {
                        check_wait(cancel, deadline)?;
                        return Err("Recognition worker handshake failed.".into());
                    }
                    if proof != token.as_bytes() {
                        return Err("Recognition worker authentication failed.".into());
                    }
                    let _ = socket.set_nodelay(true);
                    return Ok(Self {
                        socket,
                        child,
                        request: 0,
                        model: String::new(),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(_) => return Err("Recognition IPC accept failed.".into()),
            }
        }
    }

    fn exchange(
        &mut self,
        session: u64,
        op: Operation,
        cancel: &AtomicBool,
        limit: Duration,
    ) -> Result<Reply, String> {
        if let Operation::Load { model, .. } = &op {
            self.model = model.clone();
        }
        let started = Instant::now();
        self.request = self
            .request
            .checked_add(1)
            .ok_or("Recognition request ID exhausted.")?;
        let request = Request {
            version: VERSION,
            request: self.request,
            session,
            op,
        };
        let frame = encode(&request)?;
        let deadline = Instant::now() + limit;
        let result = (|| {
            let mut io = Interruptible {
                socket: &mut self.socket,
                cancel,
                deadline,
            };
            io.write_all(&frame)
                .map_err(|_| "Recognition worker write failed.")?;
            let response: Response = read_frame(&mut io)?;
            validate_response(&response, self.request, session)?;
            if !matches!(
                (&request.op, &response.body),
                (_, Reply::Error)
                    | (Operation::Load { .. }, Reply::Loaded { .. })
                    | (
                        Operation::Begin | Operation::Abort | Operation::Shutdown,
                        Reply::Ack
                    )
                    | (Operation::Audio { .. }, Reply::Text { .. })
                    | (Operation::Finish { .. }, Reply::Finished { .. })
            ) {
                return Err("Unexpected recognition response type.".into());
            }
            Ok(response.body)
        })();
        if result.is_err() {
            let timed_out = Instant::now() >= deadline;
            let reason = if cancel.load(Ordering::Acquire) {
                "cancel"
            } else if timed_out {
                "deadline"
            } else {
                "ipc-or-exit"
            };
            let code = self
                .child
                .child
                .try_wait()
                .ok()
                .flatten()
                .map(exit_value)
                .unwrap_or(0);
            diag::event(
                reason,
                &self.model,
                self.child.child.id(),
                session,
                started.elapsed().as_millis() as u64,
                code,
            );
            self.child.terminate();
            if cancel.load(Ordering::Acquire) {
                return Err("Recognition cancelled.".into());
            }
            if timed_out {
                return Err("Recognition worker deadline exceeded.".into());
            }
        }
        result
    }
}

/// Content-free exit metadata for diagnostics: the exit code, or 128 + signal
/// on Unix (137 = SIGKILL, which is how macOS memory pressure kills a worker).
pub(crate) fn exit_value(status: std::process::ExitStatus) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal as u32 as u64;
        }
    }
    status.code().map_or(0, |code| code as u32 as u64)
}

fn validate_response(response: &Response, request: u64, session: u64) -> Result<(), String> {
    if response.version != VERSION || response.request != request || response.session != session {
        return Err("Stale or incompatible recognition response.".into());
    }
    let valid_warnings =
        |warnings: &[String]| warnings.len() <= 64 && warnings.iter().all(|s| s.len() <= 2048);
    match &response.body {
        Reply::Text { raw, original_text, warnings }
            if raw.len() > MAX_TEXT
                || original_text.as_ref().is_some_and(|text| text.len() > MAX_TEXT)
                || !valid_warnings(warnings) => {
            Err("Invalid recognition result size.".into())
        }
        Reply::Finished { result, details }
            if result.text.len() > MAX_TEXT
                || result.original_text.as_ref().is_some_and(|text| text.len() > MAX_TEXT)
                || !valid_warnings(&result.warnings)
                || details.as_ref().is_some_and(|d| !d.is_object()) =>
        {
            Err("Invalid recognition result size.".into())
        }
        Reply::Loaded { model } if model.len() > 80 => {
            Err("Invalid recognition model identity.".into())
        }
        _ => Ok(()),
    }
}

pub(crate) fn worker_executable() -> Result<PathBuf, String> {
    let current =
        std::env::current_exe().map_err(|_| "Cannot locate recognition worker executable.")?;
    // The opt-in evaluator dispatches the same worker before parsing its CLI.
    // Rust test harnesses still need an application sibling: they must never
    // recursively launch themselves or load native recognition in process.
    if current
        .file_stem()
        .is_some_and(|s| {
            s == "sonojot" || s == "SonoJot"
                || (cfg!(feature = "fidelity-eval") && s == "fidelity_eval")
        })
    {
        return Ok(current);
    }
    let parent = current
        .parent()
        .ok_or("Cannot locate recognition worker directory.")?;
    let directory = if parent.file_name().is_some_and(|s| s == "deps") {
        parent.parent().unwrap_or(parent)
    } else {
        parent
    };
    // Feature-gated tests may use the evaluator sibling. It dispatches both
    // recognition and punctuation workers before parsing evaluation arguments.
    #[cfg(feature = "fidelity-eval")]
    {
        let evaluator = directory.join(if cfg!(windows) { "fidelity_eval.exe" } else { "fidelity_eval" });
        if evaluator.is_file() { return Ok(evaluator); }
    }
    let names = if cfg!(windows) {
        ["sonojot.exe", "SonoJot.exe"]
    } else {
        ["sonojot", "SonoJot"]
    };
    for name in names {
        let sibling = directory.join(name);
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    Err(
        "Build the sonojot binary alongside fidelity_eval/tests to use isolated recognition."
            .into(),
    )
}

pub struct Remote {
    root: PathBuf,
    model: String,
    connection: Option<Connection>,
    load_retried: bool,
}
impl Remote {
    pub(crate) fn is_alive(&mut self) -> bool {
        self.connection
            .as_mut()
            .is_some_and(|connection| matches!(connection.child.child.try_wait(), Ok(None)))
    }
    /// The worker's PID and exit value once it has exited (see `exit_value`).
    pub(crate) fn exit(&mut self) -> Option<(u32, u64)> {
        let connection = self.connection.as_mut()?;
        let status = connection.child.child.try_wait().ok().flatten()?;
        Some((connection.child.child.id(), exit_value(status)))
    }
    #[cfg(test)]
    pub(crate) fn pid_for_test(&self) -> u32 {
        self.connection.as_ref().map_or(0, |c| c.child.child.id())
    }
    #[cfg(test)]
    pub(crate) fn kill_worker_for_test(&mut self) {
        let connection = self.connection.as_mut().expect("worker is resident");
        connection.child.child.kill().unwrap();
        let _ = connection.child.child.wait();
    }
    pub(crate) fn load(root: &Path, model: &str, cancel: &AtomicBool) -> Result<Self, String> {
        let mut remote = Self {
            root: root.to_owned(),
            model: model.to_owned(),
            connection: None,
            load_retried: false,
        };
        if remote.connect(cancel).is_err() {
            check_wait(cancel, Instant::now() + START_LIMIT)?;
            remote.load_retried = true;
            diag::event("load-retry", model, 0, 0, 0, 1);
            remote.connect(cancel)?;
        }
        Ok(remote)
    }

    fn connect(&mut self, cancel: &AtomicBool) -> Result<(), String> {
        progress("loading");
        self.connection.take();
        let mut connection = Connection::spawn(cancel)?;
        let pid = connection.child.child.id();
        let start = Instant::now();
        diag::event("load-start", &self.model, pid, 0, 0, 0);
        let reply = connection.exchange(
            0,
            Operation::Load {
                root: self.root.clone(),
                model: self.model.clone(),
            },
            cancel,
            LOAD_LIMIT,
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                diag::event(
                    "load-failed",
                    &self.model,
                    pid,
                    0,
                    start.elapsed().as_millis() as u64,
                    0,
                );
                return Err(error);
            }
        };
        if !matches!(reply, Reply::Loaded { ref model } if model == &self.model) {
            diag::event(
                "load-failed",
                &self.model,
                pid,
                0,
                start.elapsed().as_millis() as u64,
                0,
            );
            return Err("The selected recognition model could not be loaded in its worker.".into());
        }
        diag::event(
            "load-complete",
            &self.model,
            pid,
            0,
            start.elapsed().as_millis() as u64,
            0,
        );
        self.connection = Some(connection);
        Ok(())
    }

    fn call(&mut self, session: u64, op: Operation, cancel: &AtomicBool) -> Result<Reply, String> {
        self.connection
            .as_mut()
            .ok_or("Recognition worker is unavailable.")?
            .exchange(session, op, cancel, DECODE_LIMIT)
    }

    pub(crate) fn stream<'a>(
        &'a mut self,
        cancel: Option<&'a AtomicBool>,
    ) -> Result<RemoteStream<'a>, String> {
        let cancel = cancel.unwrap_or(&NEVER_CANCEL);
        if self.connection.is_none() {
            self.connect(cancel)?;
        }
        let session = NEXT_SESSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| "Recognition session ID exhausted.")?;
        let mut retried = std::mem::take(&mut self.load_retried);
        if !matches!(self.call(session, Operation::Begin, cancel), Ok(Reply::Ack)) {
            check_wait(cancel, Instant::now() + START_LIMIT)?;
            if retried {
                self.connection.take();
                return Err("Recognition retry exhausted before the stream began.".into());
            }
            retried = true;
            self.connect(cancel)?;
            if !matches!(self.call(session, Operation::Begin, cancel)?, Reply::Ack) {
                return Err("Cannot begin recognition stream.".into());
            }
        }
        // Warm reuse must also leave the initial loading state.
        progress("transcribing");
        Ok(RemoteStream {
            remote: self,
            cancel,
            session,
            replay: if retried { None } else { Replay::new().ok() },
            retried,
            raw: String::new(),
            original_text: None,
            warnings: Vec::new(),
            failed: false,
            finished: false,
            started: Instant::now(),
            first_result: false,
            last_decode_log: Instant::now(),
        })
    }
}
impl Drop for Remote {
    fn drop(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            let pid = connection.child.child.id();
            diag::event("drop-start", &self.model, pid, 0, 0, 0);
            let start = Instant::now();
            let acknowledged = matches!(
                connection.exchange(0, Operation::Shutdown, &NEVER_CANCEL, DROP_LIMIT),
                Ok(Reply::Ack)
            );
            // An acknowledgment follows native drop. Termination also releases
            // pinned DLLs and catches a child hanging during process teardown.
            connection.child.terminate();
            diag::event(
                "drop-complete",
                &self.model,
                pid,
                0,
                start.elapsed().as_millis() as u64,
                acknowledged as u64,
            );
        }
    }
}

struct Replay {
    file: crate::audio_backlog::TemporaryFile,
    bytes: u64,
}
impl Replay {
    fn new() -> io::Result<Self> {
        Ok(Self {
            file: crate::audio_backlog::TemporaryFile::create()?,
            bytes: 0,
        })
    }
    fn append(&mut self, op: &Operation) -> Result<(), String> {
        let bytes = encode(op)?;
        if self.bytes + bytes.len() as u64 > REPLAY_QUOTA {
            return Err("Replay quota reached.".into());
        }
        crate::audio_backlog::reserve_temporary_bytes(bytes.len() as u64)
            .map_err(|_| "Replay disk floor reached.")?;
        self.file
            .seek(SeekFrom::Start(self.bytes))
            .map_err(|_| "Replay seek failed.")?;
        self.file
            .write_all(&bytes)
            .map_err(|_| "Replay write failed.")?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
}

pub struct RemoteStream<'a> {
    remote: &'a mut Remote,
    cancel: &'a AtomicBool,
    session: u64,
    replay: Option<Replay>,
    retried: bool,
    raw: String,
    original_text: Option<String>,
    warnings: Vec<String>,
    failed: bool,
    finished: bool,
    started: Instant,
    first_result: bool,
    last_decode_log: Instant,
}
impl RemoteStream<'_> {
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }
    pub(crate) fn original_text(&self) -> Option<&str> {
        self.original_text.as_deref()
    }
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }
    fn accept_text(&mut self, reply: Reply) -> Result<(), String> {
        let Reply::Text { raw, original_text, warnings } = reply else {
            return Err("Unexpected recognition response.".into());
        };
        self.raw = raw; // complete snapshot: replay never appends duplicate text
        self.original_text = original_text;
        self.warnings = warnings;
        if !self.first_result && !self.raw.is_empty() {
            self.first_result = true;
            diag::event(
                "first-result",
                &self.remote.model,
                self.pid(),
                self.session,
                self.started.elapsed().as_millis() as u64,
                0,
            );
        }
        Ok(())
    }
    fn pid(&self) -> u32 {
        self.remote
            .connection
            .as_ref()
            .map_or(0, |c| c.child.child.id())
    }

    fn recover(&mut self) -> Result<(), String> {
        check_wait(self.cancel, Instant::now() + START_LIMIT)?;
        if self.retried {
            return Err("Recognition retry exhausted.".into());
        }
        let mut replay = self
            .replay
            .take()
            .ok_or("Recognition replay is unavailable.")?;
        self.retried = true;
        diag::event("retry", &self.remote.model, self.pid(), self.session, 0, 1);
        self.remote.connect(self.cancel)?;
        progress("recovering");
        if !matches!(
            self.remote
                .call(self.session, Operation::Begin, self.cancel)?,
            Reply::Ack
        ) {
            return Err("Recognition replay could not begin.".into());
        }
        replay
            .file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "Replay seek failed.")?;
        // Keep the previous acknowledged snapshot until the entire replay
        // succeeds. A second crash cannot replace it with a shorter prefix.
        let mut last = None;
        while replay
            .file
            .stream_position()
            .map_err(|_| "Replay position failed.")?
            < replay.bytes
        {
            let op: Operation = read_frame(&mut replay.file)?;
            if !matches!(op, Operation::Audio { .. }) {
                return Err("Invalid replay operation.".into());
            }
            let reply = self.remote.call(self.session, op, self.cancel)?;
            if !matches!(reply, Reply::Text { .. }) {
                return Err("Recognition replay failed.".into());
            }
            last = Some(reply);
        }
        if let Some(last) = last {
            self.accept_text(last)?;
        }
        progress("transcribing");
        // Only one retry is allowed, so release its audio immediately.
        Ok(())
    }

    fn fail(&mut self) {
        diag::event(
            "review-required",
            &self.remote.model,
            self.pid(),
            self.session,
            self.started.elapsed().as_millis() as u64,
            0,
        );
        self.failed = true;
        self.replay.take();
        self.remote.connection.take();
        self.warnings.push("Recognition was interrupted. Available raw text is incomplete and requires review; unrecognized audio may remain.".into());
    }

    pub(crate) fn add(&mut self, samples: &[f32], rate: u32) -> Result<(), String> {
        check_wait(self.cancel, Instant::now() + START_LIMIT)?;
        if rate == 0 || rate > 384_000 {
            return Err("Invalid input sample rate.".into());
        }
        if self.failed {
            return Ok(());
        }
        for chunk in samples.chunks(MAX_SAMPLES) {
            let op = Operation::Audio {
                samples: chunk
                    .iter()
                    .map(|v| {
                        if v.is_finite() {
                            v.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    })
                    .collect(),
                rate,
            };
            if self.replay.as_mut().is_some_and(|r| r.append(&op).is_err()) {
                self.replay.take();
                diag::event(
                    "replay-unavailable",
                    &self.remote.model,
                    self.pid(),
                    self.session,
                    0,
                    0,
                );
            }
            let start = Instant::now();
            let sampled = self.last_decode_log.elapsed() >= Duration::from_secs(1);
            if sampled {
                diag::event(
                    "decode-start",
                    &self.remote.model,
                    self.pid(),
                    self.session,
                    0,
                    chunk.len() as u64,
                );
            }
            let result = self
                .remote
                .call(self.session, op, self.cancel)
                .and_then(|r| self.accept_text(r));
            if result.is_err() && self.recover().is_err() {
                check_wait(self.cancel, Instant::now() + START_LIMIT)?;
                self.fail();
                break;
            }
            if sampled || start.elapsed() >= Duration::from_millis(250) {
                self.last_decode_log = Instant::now();
                diag::event(
                    "decode-complete",
                    &self.remote.model,
                    self.pid(),
                    self.session,
                    start.elapsed().as_millis() as u64,
                    chunk.len() as u64,
                );
            }
        }
        Ok(())
    }

    pub(crate) fn update(&mut self) -> Result<String, String> {
        check_wait(self.cancel, Instant::now() + START_LIMIT)?;
        Ok(self.raw.clone())
    }

    pub(crate) fn finish(
        mut self,
        observed: bool,
    ) -> Result<(RecognitionResult, Option<serde_json::Value>), String> {
        let started = Instant::now();
        diag::event(
            "finish-start",
            &self.remote.model,
            self.pid(),
            self.session,
            0,
            0,
        );
        check_wait(self.cancel, Instant::now() + START_LIMIT)?;
        if !self.failed {
            let mut reply =
                self.remote
                    .call(self.session, Operation::Finish { observed }, self.cancel);
            if !matches!(reply, Ok(Reply::Finished { .. })) {
                if self.recover().is_ok() {
                    reply =
                        self.remote
                            .call(self.session, Operation::Finish { observed }, self.cancel);
                } else {
                    reply = Err("Recognition recovery failed.".into());
                }
            }
            check_wait(self.cancel, Instant::now() + START_LIMIT)?;
            if let Ok(Reply::Finished { result, details }) = reply {
                diag::event(
                    "finish-result",
                    &self.remote.model,
                    self.pid(),
                    self.session,
                    started.elapsed().as_millis() as u64,
                    u64::from(result.completeness == RecognitionCompleteness::Complete),
                );
                self.finished = true;
                return Ok((result, details));
            }
            self.fail();
        }
        self.finished = true;
        Ok((
            RecognitionResult {
                original_text: self.original_text.clone(),
                segments: Vec::new(),
                text: self.raw.clone(),
                completeness: RecognitionCompleteness::Incomplete,
                warnings: self.warnings.clone(),
            },
            None,
        ))
    }
}
impl Drop for RemoteStream<'_> {
    fn drop(&mut self) {
        if !self.finished {
            // Cancellation must not wait for a native stream destructor.
            self.remote.connection.take();
        }
    }
}

/// Called by main before any Tauri initialization. This is public for a
/// future evaluation entry point; native load remains crate-private.
pub fn dispatch_worker() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--recognition-worker")) {
        return false;
    }
    let result = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or("Missing worker endpoint.")
        .and_then(|address| worker_main(&address));
    std::process::exit(if result.is_ok() { 0 } else { 70 });
}

fn worker_socket(address: &str) -> Result<TcpStream, &'static str> {
    let address: std::net::SocketAddr = address.parse().map_err(|_| "Invalid worker endpoint.")?;
    if !address.ip().is_loopback() {
        return Err("Worker endpoint must be local.");
    }
    let mut socket = TcpStream::connect_timeout(&address, START_LIMIT)
        .map_err(|_| "Worker connection failed.")?;
    socket
        .set_read_timeout(Some(START_LIMIT))
        .map_err(|_| "Worker timeout failed.")?;
    socket
        .set_write_timeout(Some(START_LIMIT))
        .map_err(|_| "Worker timeout failed.")?;
    let token = std::env::var("SONOJOT_RECOGNITION_TOKEN").map_err(|_| "Worker token missing.")?;
    std::env::remove_var("SONOJOT_RECOGNITION_TOKEN");
    if token.len() != 32 {
        return Err("Worker token invalid.");
    }
    socket
        .write_all(token.as_bytes())
        .map_err(|_| "Worker handshake failed.")?;
    let _ = socket.set_nodelay(true);
    Ok(socket)
}

fn worker_main(address: &str) -> Result<(), &'static str> {
    let mut socket = worker_socket(address)?;
    diag::install_worker_crash_capture();
    let load: Request = read_frame(&mut socket).map_err(|_| "Load frame failed.")?;
    if load.version != VERSION || load.request != 1 || load.session != 0 {
        return Err("Invalid load sequence.");
    }
    let Operation::Load { root, model } = &load.op else {
        return Err("Expected load.");
    };
    if model.is_empty()
        || model.len() > 80
        || !model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err("Invalid model ID.");
    }
    NATIVE_WORKER.store(true, Ordering::Release);
    std::env::set_var("MOONSHINE_ORT_SINGLE_THREAD", "1");
    let mut recognizer = match Recognizer::load_native(root, model) {
        Ok(r) => r,
        Err(_) => {
            send_reply(&mut socket, &load, Reply::Error)?;
            return Err("Native model load failed.");
        }
    };
    send_reply(
        &mut socket,
        &load,
        Reply::Loaded {
            model: recognizer.model_id.clone(),
        },
    )?;
    // Idle waits are bounded but long enough for retained model policy. EOF
    // exits immediately; Windows also owns this process through its job.
    socket
        .set_read_timeout(Some(Duration::from_secs(24 * 60 * 60)))
        .map_err(|_| "Worker timeout failed.")?;
    let mut last_request = 1;
    let mut last_session = 0;
    loop {
        let request = receive_request(&mut socket, &mut last_request)?;
        match request.op {
            Operation::Begin if request.session > last_session => {
                last_session = request.session;
                let stream = recognizer.stream().map_err(|_| "Native stream failed.")?;
                send_reply(&mut socket, &request, Reply::Ack)?;
                serve_stream(&mut socket, stream, last_session, &mut last_request)?;
            }
            Operation::Shutdown if request.session == 0 => {
                drop(recognizer);
                send_reply(&mut socket, &request, Reply::Ack)?;
                return Ok(());
            }
            _ => return Err("Unexpected idle operation."),
        }
    }
}

fn receive_request(socket: &mut TcpStream, last: &mut u64) -> Result<Request, &'static str> {
    let request: Request = read_frame(socket).map_err(|_| "Request read failed.")?;
    if request.version != VERSION
        || request.request != last.checked_add(1).ok_or("Request overflow.")?
    {
        return Err("Invalid request sequence.");
    }
    *last = request.request;
    Ok(request)
}
fn send_reply(socket: &mut TcpStream, request: &Request, body: Reply) -> Result<(), &'static str> {
    let response = Response {
        version: VERSION,
        request: request.request,
        session: request.session,
        body,
    };
    validate_response(&response, request.request, request.session)
        .map_err(|_| "Invalid native result.")?;
    socket
        .write_all(&encode(&response).map_err(|_| "Response encoding failed.")?)
        .map_err(|_| "Response write failed.")
}

fn serve_stream(
    socket: &mut TcpStream,
    mut stream: crate::asr::Stream<'_>,
    session: u64,
    last: &mut u64,
) -> Result<(), &'static str> {
    let mut pending_seconds = 0.0;
    let mut raw = String::new();
    let mut original_text = None;
    loop {
        let request = receive_request(socket, last)?;
        if request.session != session {
            return Err("Stale stream request.");
        }
        let reply = match &request.op {
            Operation::Audio { samples, rate } => {
                if samples.len() > MAX_SAMPLES
                    || *rate == 0
                    || *rate > 384_000
                    || samples.iter().any(|v| !v.is_finite() || v.abs() > 1.0)
                {
                    return Err("Invalid normalized audio.");
                }
                stream
                    .add(samples, *rate)
                    .map_err(|_| "Native decode failed.")?;
                pending_seconds += samples.len() as f64 / f64::from(*rate);
                // Capture callbacks can be only a few milliseconds long.
                // Keep Moonshine's existing ~650 ms publication cadence,
                // including while draining queued audio after Stop.
                if pending_seconds >= 0.65 {
                    raw = stream.update().map_err(|_| "Native update failed.")?;
                    original_text = stream.original_text().map(str::to_owned);
                    pending_seconds = 0.0;
                }
                Reply::Text {
                    raw: raw.clone(),
                    original_text: original_text.clone(),
                    warnings: stream.warnings(),
                }
            }
            Operation::Finish { observed } => {
                #[cfg(feature = "fidelity-eval")]
                let result = if *observed {
                    stream.finish_observed().map(|(r, d)| (r, Some(d)))
                } else {
                    stream.finish_recognition().map(|r| (r, None))
                };
                #[cfg(not(feature = "fidelity-eval"))]
                let result = {
                    let _ = observed;
                    stream.finish_recognition().map(|r| (r, None))
                };
                let (result, details) = result.map_err(|_| "Native finish failed.")?;
                send_reply(socket, &request, Reply::Finished { result, details })?;
                return Ok(());
            }
            Operation::Abort => {
                drop(stream);
                send_reply(socket, &request, Reply::Ack)?;
                return Ok(());
            }
            _ => return Err("Unexpected stream operation."),
        };
        send_reply(socket, &request, reply)?;
    }
}

#[cfg(test)]
#[path = "recognition_tests.rs"]
mod tests;
