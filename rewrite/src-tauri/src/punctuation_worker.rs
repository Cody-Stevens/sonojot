//! Warm punctuation helper. Like diarization, only the contained child enters
//! ORT; like recognition, bounded authenticated loopback frames carry requests.
//! No transcript or IPC file is written. One session belongs to one child.
use crate::{
    domain::ModelRetention,
    punctuation::{Failure, Mark, Result, Selection},
    recognition::OwnedChild,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, TryLockError,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_TEXT: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(10);
const START_LIMIT: Duration = Duration::from_secs(15);
const WORK_LIMIT: Duration = Duration::from_secs(30);
static NATIVE_WORKER: AtomicBool = AtomicBool::new(false);
static PINS: AtomicUsize = AtomicUsize::new(0);
static CLIENT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();

pub(crate) fn is_worker() -> bool {
    NATIVE_WORKER.load(Ordering::Acquire)
}
fn client() -> &'static Mutex<Option<Connection>> {
    CLIENT.get_or_init(|| Mutex::new(None))
}

fn worker_executable() -> Result<PathBuf> {
    // Opt-in native tests can exercise an already-built release helper without
    // rebuilding the UI binary or ever loading the runtime in the test process.
    #[cfg(test)]
    if let Some(executable) = std::env::var_os("SONOJOT_PUNCT_TEST_WORKER") {
        return Ok(PathBuf::from(executable));
    }
    #[cfg(feature = "fidelity-eval")]
    {
        let executable = std::env::current_exe().map_err(|_| Failure::Runtime)?;
        if executable
            .file_stem()
            .is_some_and(|stem| stem == "punctuation_probe" || stem == "fidelity_eval")
        {
            return Ok(executable);
        }
    }
    crate::recognition::worker_executable().map_err(|_| Failure::Runtime)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub pid: u32,
    /// On Windows this is the complete set of loaded onnxruntime.dll modules.
    pub libraries: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    request: u64,
    operation: Operation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Operation {
    Load { selection: Selection },
    Predict { text: String },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    request: u64,
    result: Result<Reply>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Reply {
    Ready {
        evidence: RuntimeEvidence,
    },
    Labels {
        labels: Vec<Mark>,
        /// Regex-word BIO labels from the same inference; absent for v1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        spans: Option<Vec<crate::written_form::BioLabel>>,
    },
}

fn prediction_reply(reply: Reply, count: usize) -> Result<crate::punctuation::Prediction> {
    match reply {
        Reply::Labels { labels, spans }
            if labels.len() == count && spans.as_ref().is_none_or(|spans| spans.len() == count) =>
        {
            Ok(crate::punctuation::Prediction { labels, spans })
        }
        _ => Err(Failure::Output),
    }
}

fn check(cancel: &dyn Fn() -> bool, deadline: Instant) -> Result<()> {
    if cancel() {
        Err(Failure::Cancelled)
    } else if Instant::now() >= deadline {
        Err(Failure::Runtime)
    } else {
        Ok(())
    }
}

fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| Failure::Output)?;
    if bytes.len() > MAX_FRAME {
        return Err(Failure::Output);
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|_| Failure::Runtime)?;
    writer.write_all(&bytes).map_err(|_| Failure::Runtime)
}
fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| Failure::Runtime)?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(Failure::Output);
    }
    let mut bytes = vec![0; len];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| Failure::Runtime)?;
    serde_json::from_slice(&bytes).map_err(|_| Failure::Output)
}

struct Interruptible<'a> {
    socket: &'a mut TcpStream,
    cancel: &'a dyn Fn() -> bool,
    deadline: Instant,
}
fn retry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}
impl Read for Interruptible<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            check(self.cancel, self.deadline)
                .map_err(|_| io::Error::other("Punctuation wait stopped."))?;
            match self.socket.read(bytes) {
                Err(e) if retry(&e) => continue,
                result => return result,
            }
        }
    }
}
impl Write for Interruptible<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            check(self.cancel, self.deadline)
                .map_err(|_| io::Error::other("Punctuation wait stopped."))?;
            match self.socket.write(bytes) {
                Err(e) if retry(&e) => continue,
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Connection {
    socket: TcpStream,
    _child: OwnedChild,
    selection: Selection,
    evidence: RuntimeEvidence,
    request: u64,
    last_used: Instant,
}
impl Connection {
    fn load(selection: &Selection, cancel: &dyn Fn() -> bool) -> Result<Self> {
        let deadline = Instant::now() + START_LIMIT;
        check(cancel, deadline)?;
        if !selection.available() {
            return Err(Failure::Missing);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| Failure::Runtime)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Failure::Runtime)?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let mut command = Command::new(worker_executable()?);
        command
            .arg("--punctuation-worker")
            .arg(
                listener
                    .local_addr()
                    .map_err(|_| Failure::Runtime)?
                    .to_string(),
            )
            .env("SONOJOT_PUNCTUATION_TOKEN", &token);
        crate::recognition_diagnostics::prepare_worker_diagnostics(&mut command);
        let mut child = OwnedChild::spawn(&mut command).map_err(|_| Failure::Runtime)?;
        let mut socket = loop {
            check(cancel, deadline)?;
            if child.try_wait().map_err(|_| Failure::Runtime)?.is_some() {
                return Err(Failure::Runtime);
            }
            match listener.accept() {
                Ok((socket, peer)) if peer.ip().is_loopback() => break socket,
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(_) => return Err(Failure::Runtime),
            }
        };
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|_| Failure::Runtime)?;
        socket
            .set_write_timeout(Some(POLL))
            .map_err(|_| Failure::Runtime)?;
        socket.set_nodelay(true).map_err(|_| Failure::Runtime)?;
        let mut proof = [0; 32];
        Interruptible {
            socket: &mut socket,
            cancel,
            deadline,
        }
        .read_exact(&mut proof)
        .map_err(|_| Failure::Runtime)?;
        if proof != token.as_bytes() {
            return Err(Failure::Runtime);
        }
        let mut connection = Self {
            socket,
            _child: child,
            selection: selection.clone(),
            request: 0,
            evidence: RuntimeEvidence {
                pid: 0,
                libraries: vec![],
            },
            last_used: Instant::now(),
        };
        match connection.exchange(
            Operation::Load {
                selection: selection.clone(),
            },
            cancel,
        )? {
            Reply::Ready { evidence } => {
                if evidence.pid == std::process::id() || evidence.pid == 0 {
                    return Err(Failure::Runtime);
                }
                #[cfg(windows)]
                verify_libraries(
                    &evidence.libraries,
                    &crate::diarization_model::runtime_library(&selection.root),
                    true,
                )?;
                connection.evidence = evidence;
                Ok(connection)
            }
            _ => Err(Failure::Output),
        }
    }
    fn exchange(&mut self, operation: Operation, cancel: &dyn Fn() -> bool) -> Result<Reply> {
        self.request = self.request.checked_add(1).ok_or(Failure::Output)?;
        let deadline = Instant::now() + WORK_LIMIT;
        let mut io = Interruptible {
            socket: &mut self.socket,
            cancel,
            deadline,
        };
        let request = Request {
            version: VERSION,
            request: self.request,
            operation,
        };
        let result = (|| {
            write_frame(&mut io, &request)?;
            let response: Response = read_frame(&mut io)?;
            if response.version != VERSION || response.request != self.request {
                return Err(Failure::Output);
            }
            response.result
        })();
        check(cancel, deadline)?;
        self.last_used = Instant::now();
        result
    }
}

fn lock_client(cancel: &dyn Fn() -> bool) -> Result<MutexGuard<'static, Option<Connection>>> {
    let deadline = Instant::now() + WORK_LIMIT;
    loop {
        check(cancel, deadline)?;
        match client().try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => return Err(Failure::Busy),
            Err(TryLockError::WouldBlock) => thread::sleep(POLL),
        }
    }
}
fn ensure_loaded(
    slot: &mut Option<Connection>,
    selection: &Selection,
    cancel: &dyn Fn() -> bool,
) -> Result<()> {
    if slot.as_mut().is_some_and(|worker| {
        worker.selection != *selection || !matches!(worker._child.try_wait(), Ok(None))
    }) {
        slot.take();
    }
    if slot.is_none() {
        *slot = Some(Connection::load(selection, cancel)?);
    }
    Ok(())
}
pub(crate) fn wait_ready(selection: &Selection, cancel: &AtomicBool) -> Result<RuntimeEvidence> {
    prepare(selection, &|| cancel.load(Ordering::Acquire))
}
fn prepare(selection: &Selection, cancel: &dyn Fn() -> bool) -> Result<RuntimeEvidence> {
    let mut slot = lock_client(cancel)?;
    ensure_loaded(&mut slot, selection, cancel)?;
    let worker = slot.as_mut().ok_or(Failure::Load)?;
    worker.last_used = Instant::now();
    Ok(worker.evidence.clone())
}
pub(crate) fn predict(
    selection: &Selection,
    text: &str,
    count: usize,
    cancel: &AtomicBool,
) -> Result<crate::punctuation::Prediction> {
    if text.len() > MAX_TEXT {
        return Err(Failure::Output);
    }
    let cancelled = || cancel.load(Ordering::Acquire);
    let mut slot = lock_client(&cancelled)?;
    ensure_loaded(&mut slot, selection, &cancelled)?;
    let result = slot
        .as_mut()
        .ok_or(Failure::Load)?
        .exchange(Operation::Predict { text: text.into() }, &cancelled)
        .and_then(|reply| prediction_reply(reply, count));
    // A failed/cancelled exchange cannot leave a late response for the next job.
    if result.is_err() {
        slot.take();
    }
    result
}

/// Own this guard for the entire capture/cleanup job, including cancellation.
/// Immediate retention cannot evict a session between prewarm and Stop.
pub struct Prewarm {
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
pub fn prewarm(root: &Path, cancel: Arc<AtomicBool>) -> Prewarm {
    PINS.fetch_add(1, Ordering::AcqRel);
    let stop = Arc::new(AtomicBool::new(false));
    let selection = Selection::new(
        root,
        std::env::var_os("SONOJOT_PUNCT_MODEL_DIR").map(PathBuf::from),
    );
    let stopped = stop.clone();
    let cancelled = cancel.clone();
    let thread = thread::Builder::new()
        .name("punctuation-prewarm".into())
        .spawn(move || {
            let start = Instant::now();
            let result = prepare(&selection, &|| {
                stopped.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire)
            });
            crate::recognition_diagnostics::event(
                if result.is_ok() {
                    "punctuation-prewarm-ready"
                } else {
                    "punctuation-prewarm-unavailable"
                },
                crate::punctuation_model::ID,
                std::process::id(),
                0,
                start.elapsed().as_millis() as u64,
                0,
            );
        })
        .ok();
    Prewarm {
        stop,
        cancel,
        thread,
    }
}
impl Drop for Prewarm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        PINS.fetch_sub(1, Ordering::AcqRel);
        if self.cancel.load(Ordering::Acquire) {
            release_cached();
        } else {
            release_idle(crate::resource_manager::current_retention());
        }
    }
}
pub(crate) fn release_idle(retention: ModelRetention) -> bool {
    release_if(|worker| crate::punctuation::expired(worker.last_used.elapsed(), retention))
}
pub(crate) fn release_cached() -> bool {
    release_if(|_| true)
}
fn release_if(eligible: impl FnOnce(&Connection) -> bool) -> bool {
    let Ok(mut slot) = client().try_lock() else {
        return false;
    };
    if PINS.load(Ordering::Acquire) == 0 && slot.as_ref().is_some_and(eligible) {
        slot.take(); // OwnedChild kills, waits and closes its Windows job.
        true
    } else {
        false
    }
}

/// Every executable capable of spawning this helper dispatches before its CLI/UI.
pub fn dispatch_worker() -> bool {
    let mut args = std::env::args();
    args.next();
    if args.next().as_deref() != Some("--punctuation-worker") {
        return false;
    }
    let result = args
        .next()
        .filter(|_| args.next().is_none())
        .ok_or(Failure::Runtime)
        .and_then(|address| worker_loop(&address));
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
fn worker_loop(address: &str) -> Result<()> {
    let address: std::net::SocketAddr = address.parse().map_err(|_| Failure::Runtime)?;
    if !address.ip().is_loopback() {
        return Err(Failure::Runtime);
    }
    let token = std::env::var("SONOJOT_PUNCTUATION_TOKEN").map_err(|_| Failure::Runtime)?;
    std::env::remove_var("SONOJOT_PUNCTUATION_TOKEN");
    if token.len() != 32 {
        return Err(Failure::Runtime);
    }
    let mut socket =
        TcpStream::connect_timeout(&address, START_LIMIT).map_err(|_| Failure::Runtime)?;
    socket.set_nodelay(true).map_err(|_| Failure::Runtime)?;
    socket
        .write_all(token.as_bytes())
        .map_err(|_| Failure::Runtime)?;
    NATIVE_WORKER.store(true, Ordering::Release);
    let mut selection = None;
    let mut previous = 0;
    loop {
        let request: Request = read_frame(&mut socket)?;
        if request.version != VERSION || request.request != previous + 1 {
            return Err(Failure::Output);
        }
        previous = request.request;
        let result = match request.operation {
            Operation::Load {
                selection: requested,
            } if selection.is_none() => {
                crate::punctuation::worker_load(&requested).and_then(|()| {
                    let evidence = runtime_evidence(&requested.root)?;
                    selection = Some(requested);
                    Ok(Reply::Ready { evidence })
                })
            }
            Operation::Predict { text } if text.len() <= MAX_TEXT => selection
                .as_ref()
                .ok_or(Failure::Load)
                .and_then(|selection| {
                    if !selection.unchanged() {
                        return Err(Failure::Load);
                    }
                    crate::punctuation::worker_predict(selection, &text).map(|prediction| {
                        Reply::Labels {
                            labels: prediction.labels,
                            spans: prediction.spans,
                        }
                    })
                }),
            _ => Err(Failure::Output),
        };
        let failed = result.is_err();
        write_frame(
            &mut socket,
            &Response {
                version: VERSION,
                request: previous,
                result,
            },
        )?;
        if failed {
            return Err(Failure::Runtime);
        }
    }
}

#[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64"), all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn initialize_runtime(root: &Path) -> Result<()> {
    if !is_worker() {
        return Err(Failure::Runtime);
    }
    let expected = crate::diarization_model::runtime_library(root);
    #[cfg(windows)]
    verify_libraries(&loaded_libraries()?, &expected, false)?;
    // This helper must be a fresh process. Never accept an already committed
    // environment or resolve a second ORT path after a model/root switch.
    if !ort::init_from(&expected)
        .map_err(|_| Failure::Runtime)?
        .with_name("sonojot-punctuation")
        .with_telemetry(false)
        .with_logger(Arc::new(|_, _, _, _, _| {}))
        .commit()
    {
        return Err(Failure::Runtime);
    }
    runtime_evidence(root)?;
    Ok(())
}
fn runtime_evidence(root: &Path) -> Result<RuntimeEvidence> {
    #[cfg(windows)]
    let libraries = {
        let libraries = loaded_libraries()?;
        verify_libraries(
            &libraries,
            &crate::diarization_model::runtime_library(root),
            true,
        )?;
        libraries
    };
    #[cfg(not(windows))]
    let libraries = vec![crate::diarization_model::runtime_library(root)];
    Ok(RuntimeEvidence {
        pid: std::process::id(),
        libraries,
    })
}

#[cfg(windows)]
fn verify_libraries(libraries: &[PathBuf], expected: &Path, initialized: bool) -> Result<()> {
    if libraries.len() != usize::from(initialized) {
        return Err(Failure::Runtime);
    }
    if let Some(actual) = libraries.first() {
        if std::fs::canonicalize(actual).map_err(|_| Failure::Runtime)?
            != std::fs::canonicalize(expected).map_err(|_| Failure::Runtime)?
        {
            return Err(Failure::Runtime);
        }
    }
    Ok(())
}

/// Query the current process, not a guessed DLL search path. Kernel32 exports
/// these APIs on supported Windows versions; no new runtime dependency needed.
#[cfg(windows)]
fn loaded_libraries() -> Result<Vec<PathBuf>> {
    use std::{
        ffi::{c_void, OsString},
        os::windows::ffi::OsStringExt,
    };
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32EnumProcessModules(
            process: *mut c_void,
            modules: *mut *mut c_void,
            bytes: u32,
            needed: *mut u32,
        ) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, filename: *mut u16, size: u32) -> u32;
    }
    let mut modules = vec![std::ptr::null_mut(); 1024];
    let mut needed = 0;
    let bytes = std::mem::size_of_val(modules.as_slice()) as u32;
    if unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            modules.as_mut_ptr(),
            bytes,
            &mut needed,
        )
    } == 0
        || needed > bytes
    {
        return Err(Failure::Runtime);
    }
    let mut paths = Vec::new();
    for module in modules
        .into_iter()
        .take(needed as usize / std::mem::size_of::<*mut c_void>())
    {
        let mut buffer = vec![0u16; 32768];
        let count = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) }
            as usize;
        if count == 0 || count >= buffer.len() {
            return Err(Failure::Runtime);
        }
        let path = PathBuf::from(OsString::from_wide(&buffer[..count]));
        if path.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("onnxruntime.dll")
        }) {
            paths.push(path);
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_never_waits_for_active_ipc() {
        let _gate = crate::resource_manager::tests::test_gate();
        let _held = client().lock().unwrap();
        assert!(!release_cached());
        assert!(!release_idle(ModelRetention::Immediately));
    }

    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64"), all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn desktop_cannot_enter_native_loader() {
        assert!(!is_worker());
        let selection = Selection::new(Path::new("missing-punctuation-test-root"), None);
        #[cfg(windows)]
        let before = loaded_libraries().unwrap();
        assert_eq!(
            crate::punctuation::worker_load(&selection),
            Err(Failure::Runtime)
        );
        #[cfg(windows)]
        assert_eq!(loaded_libraries().unwrap(), before);
    }

    #[test]
    fn ipc_rejects_oversized_frames_and_unknown_labels() {
        assert!(read_frame::<Response>(&mut &((MAX_FRAME + 1) as u32).to_le_bytes()[..]).is_err());
        assert!(read_frame::<Response>(&mut &0u32.to_le_bytes()[..]).is_err());
        let invalid = serde_json::json!({"version":1,"request":1,"result":{"Ok":{"kind":"Labels","labels":["rewrite"]}}});
        let mut frame = Vec::new();
        write_frame(&mut frame, &invalid).unwrap();
        assert!(read_frame::<Response>(&mut frame.as_slice()).is_err());
    }

    #[test]
    fn ipc_accepts_optional_written_form_head_and_rejects_unknown_classes() {
        for spans in [None, Some(serde_json::json!(["B-MONEY", "I-MONEY"]))] {
            let mut reply = serde_json::json!({"kind":"Labels","labels":["none","period"]});
            if let Some(spans) = spans {
                reply["spans"] = spans;
            }
            let decoded: Reply = serde_json::from_value(reply).unwrap();
            let Reply::Labels { labels, .. } = decoded else {
                panic!("expected labels")
            };
            assert_eq!(labels.len(), 2);
        }
        assert!(serde_json::from_value::<Reply>(serde_json::json!({
            "kind":"Labels","labels":["none"],"spans":["B-REWRITE"]
        }))
        .is_err());
    }

    #[test]
    fn ipc_validates_both_heads_without_dropping_present_span_labels() {
        use crate::written_form::{BioLabel, Class};
        let spans = vec![BioLabel::B(Class::Money), BioLabel::I(Class::Money)];
        for expected in [None, Some(spans)] {
            let reply = Reply::Labels {
                labels: vec![Mark::None, Mark::Period],
                spans: expected.clone(),
            };
            let mut frame = Vec::new();
            write_frame(&mut frame, &reply).unwrap();
            let reply: Reply = read_frame(&mut frame.as_slice()).unwrap();
            let prediction = prediction_reply(reply, 2).unwrap();
            assert_eq!(prediction.labels, [Mark::None, Mark::Period]);
            assert_eq!(prediction.spans, expected);
        }
        for (count, marks, spans) in [
            (2, 1, None),
            (2, 2, Some(vec![])),
            (2, 2, Some(vec![BioLabel::O])),
            (2, 2, Some(vec![BioLabel::O; 3])),
            (0, 0, Some(vec![BioLabel::O])),
        ] {
            let reply = Reply::Labels {
                labels: vec![Mark::None; marks],
                spans,
            };
            assert_eq!(prediction_reply(reply, count), Err(Failure::Output));
        }
        // Empty text still distinguishes a present head from a legacy package.
        for spans in [None, Some(vec![])] {
            let reply = Reply::Labels {
                labels: vec![],
                spans: spans.clone(),
            };
            assert_eq!(prediction_reply(reply, 0).unwrap().spans, spans);
        }
        let legacy = serde_json::to_value(Reply::Labels {
            labels: vec![Mark::Period],
            spans: None,
        })
        .unwrap();
        assert!(legacy.get("spans").is_none());
    }

    #[test]
    fn unresponsive_socket_wait_observes_cancellation_and_deadline() {
        for cancelled in [true, false] {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let _peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(POLL)).unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            let timer = thread::spawn(move || {
                thread::sleep(Duration::from_millis(30));
                signal.store(cancelled, Ordering::Release);
            });
            let started = Instant::now();
            let mut io = Interruptible {
                socket: &mut socket,
                cancel: &|| cancel.load(Ordering::Acquire),
                deadline: started + Duration::from_millis(100),
            };
            assert!(read_frame::<Response>(&mut io).is_err());
            timer.join().unwrap();
            assert!(started.elapsed() < Duration::from_secs(1));
        }
    }

    #[cfg(windows)]
    #[test]
    fn runtime_identity_rejects_existing_or_duplicate_modules() {
        let path = std::env::current_exe().unwrap();
        assert!(verify_libraries(&[], &path, false).is_ok());
        assert!(verify_libraries(&[path.clone()], &path, false).is_err());
        assert!(verify_libraries(&[], &path, true).is_err());
        assert!(verify_libraries(&[path.clone(), path.clone()], &path, true).is_err());
        assert!(verify_libraries(&[path.clone()], &path, true).is_ok());
        assert!(verify_libraries(&[path], Path::new("wrong-runtime.dll"), true).is_err());
    }

    /// Opt-in native smoke/benchmark. Uses only the explicitly supplied isolated
    /// package/root. Run in release for meaningful Stop-to-text stage timings.
    #[cfg(windows)]
    #[test]
    fn real_worker_prewarm_retention_and_runtime_identity() {
        let Some(directory) = std::env::var_os("SONOJOT_PUNCT_MODEL_DIR") else {
            return;
        };
        let root = PathBuf::from(
            std::env::var_os("SONOJOT_PUNCT_TEST_MODELS_ROOT").expect("isolated root required"),
        );
        let _gate = crate::resource_manager::tests::test_gate();
        crate::resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            ModelRetention::Immediately,
        );
        release_cached();
        let before = loaded_libraries().unwrap();
        assert!(before.is_empty(), "The test parent must not load ORT.");
        let selection = Selection::new(&root, Some(PathBuf::from(directory)));
        let text =
            "the problem is that. I need the. Feature and I want to use it today ".repeat(20);
        let cancel = Arc::new(AtomicBool::new(false));
        let cold = crate::punctuation::run(&root, &text, &[], &cancel);
        assert_eq!(cold.stage, "punctuation_model");
        assert!(client().lock().unwrap().is_none());
        let prewarm_started = Instant::now();
        let hold = prewarm(&root, cancel.clone());
        let evidence = wait_ready(&selection, &cancel).unwrap();
        let prewarm_ms = prewarm_started.elapsed().as_secs_f64() * 1000.0;
        assert_ne!(evidence.pid, std::process::id());
        assert_eq!(evidence.libraries.len(), 1);
        assert!(!release_idle(ModelRetention::Immediately));
        assert!(!release_cached()); // pressure also respects capture ownership
        let mut warm_ms = Vec::new();
        for _ in 0..5 {
            let warm = crate::punctuation::run(&root, &text, &[], &cancel);
            assert_eq!(warm.stage, "punctuation_model");
            assert_eq!(warm.text, cold.text);
            warm_ms.push(warm.elapsed_ms);
            assert_eq!(wait_ready(&selection, &cancel).unwrap().pid, evidence.pid);
        }
        // Cancel while the native child is working. The parent must destroy
        // that connection rather than let its late reply reach the next job.
        let signal = cancel.clone();
        let timer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            signal.store(true, Ordering::Release);
        });
        let started = Instant::now();
        assert_eq!(
            predict(&selection, &text.repeat(10), 3000, &cancel),
            Err(Failure::Cancelled)
        );
        timer.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(client().lock().unwrap().is_none());
        // The same capture owner can recover with a fresh, authenticated child.
        cancel.store(false, Ordering::Release);
        assert_ne!(wait_ready(&selection, &cancel).unwrap().pid, evidence.pid);
        drop(hold);
        assert!(client().lock().unwrap().is_none());
        assert_eq!(PINS.load(Ordering::Acquire), 0);
        assert_eq!(loaded_libraries().unwrap(), before);
        eprintln!(
            "PUNCT_WORKER_EVIDENCE {}",
            serde_json::json!({
                "words": 300, "cold_ms": cold.elapsed_ms, "prewarm_ms": prewarm_ms,
                "warm_ms": warm_ms, "worker": evidence, "parent_ort_modules": before,
                "immediate_released": true
            })
        );
        let hold = prewarm(&root, cancel.clone());
        cancel.store(true, Ordering::Release);
        let start = Instant::now();
        drop(hold);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(client().lock().unwrap().is_none());
        crate::resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            ModelRetention::Minutes10,
        );
    }
}
