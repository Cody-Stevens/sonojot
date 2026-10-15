//! Job-scoped Granite NAR process. Only the child loads transcribe.cpp.
//! Length-prefixed stdin/stdout frames carry 16 kHz mono PCM and complete text;
//! cancellation, a deadline, or any protocol/native failure retires the child.
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::AtomicBool, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const FLAG: &str = "--granite-worker";
const OWNER_ENV: &str = "SONOJOT_GRANITE_OWNER";
const MAGIC: &[u8; 8] = b"SGNAR001";
const VERSION: u32 = 1;
const POLL: Duration = Duration::from_millis(20);
const LOAD_LIMIT: Duration = Duration::from_secs(180);
const RUN_LIMIT: Duration = Duration::from_secs(120);
const MAX_TEXT: usize = 64 * 1024;
const MAX_LOAD: usize = 32 * 1024;
const INPUT_TOO_LONG: &str = "Granite input too long for the model context; use a shorter section.";
/// Parent VAD sections are normally about 20 seconds. Reject oversized input
/// before allocation/native inference, well below NAR's 384-second ceiling.
pub(crate) const MAX_CHUNK_SAMPLES: usize = 60 * 16_000;
const HEADER: usize = 13; // version:u32, sequence:u64, operation:u8
const MAX_REQUEST: usize = HEADER + MAX_CHUNK_SAMPLES * 4;
const MAX_RESPONSE: usize = MAX_TEXT * 6 + 1024; // JSON escaping

#[derive(Serialize, Deserialize)]
#[serde(tag = "source", deny_unknown_fields)]
enum ModelSource {
    Installed {
        root: PathBuf,
    },
    // Tests can read the audited benchmark cache without copying gigabytes or
    // manufacturing production receipts. Its fixed hashes are checked afresh.
    #[cfg(all(test, windows))]
    Benchmark {
        model: PathBuf,
        runtime: PathBuf,
    },
}
impl ModelSource {
    fn runtime(&self) -> PathBuf {
        match self {
            Self::Installed { root } => crate::granite_model::runtime_dir(root),
            #[cfg(all(test, windows))]
            Self::Benchmark { runtime, .. } => runtime.clone(),
        }
    }

    fn verify(&self) -> Result<(PathBuf, PathBuf), String> {
        match self {
            Self::Installed { root } => {
                crate::granite_model::verify_for_load(root)?;
                Ok((crate::granite_model::model_path(root), self.runtime()))
            }
            #[cfg(all(test, windows))]
            Self::Benchmark { model, runtime } => {
                tests::verify_benchmark(model, runtime)?;
                Ok((model.clone(), runtime.clone()))
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Reply {
    Ready {
        backend: String,
    },
    Text {
        text: String,
    },
    /// Rejected before inference; the parent may retry shorter sections using
    /// the same resident model. No partial text accompanies this reply.
    InputTooLong,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    sequence: u64,
    result: Result<Reply, String>,
}

fn check(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    crate::ai::check_cancel(cancel)?;
    if Instant::now() >= deadline {
        return Err("Granite transcription exceeded its time limit.".into());
    }
    Ok(())
}

fn validate_samples(samples: &[f32]) -> Result<(), String> {
    if samples.is_empty() || samples.len() > MAX_CHUNK_SAMPLES {
        return Err("Granite requires an audio section between 1 sample and 60 seconds.".into());
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err("Granite received non-finite audio samples.".into());
    }
    Ok(())
}

fn write_frame(writer: &mut impl Write, bytes: &[u8], limit: usize) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Granite frame size.",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

fn read_frame(reader: &mut impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Granite frame size.",
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn request(sequence: u64, operation: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.push(operation);
    bytes.extend_from_slice(payload);
    bytes
}

fn parse_request(bytes: &[u8], expected: u64) -> Result<(u8, &[u8]), String> {
    if bytes.len() < HEADER
        || u32::from_le_bytes(bytes[..4].try_into().unwrap()) != VERSION
        || u64::from_le_bytes(bytes[4..12].try_into().unwrap()) != expected
    {
        return Err("Invalid Granite request sequence or version.".into());
    }
    Ok((bytes[12], &bytes[HEADER..]))
}

fn decode_samples(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.is_empty() || bytes.len() > MAX_CHUNK_SAMPLES * 4 || bytes.len() % 4 != 0 {
        return Err("Invalid Granite audio frame size.".into());
    }
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect();
    validate_samples(&samples)?;
    Ok(samples)
}

/// One model stays resident for the meeting job. Drop this before diarization.
pub(crate) struct GraniteWorker {
    child: ContainedChild,
    requests: Option<mpsc::SyncSender<Vec<u8>>>,
    replies: mpsc::Receiver<Result<Response, String>>,
    io: Option<JoinHandle<()>>,
    sequence: u64,
    stopped: bool,
}

impl GraniteWorker {
    pub(crate) fn load(root: &Path, cancel: &AtomicBool) -> Result<Self, String> {
        crate::ai::check_cancel(cancel)?;
        if !cfg!(any(
            all(windows, target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64")
        )) {
            return Err(
                "Granite requires Windows x64 with NVIDIA CUDA or Apple Silicon with Metal.".into(),
            );
        }
        let root = root
            .canonicalize()
            .map_err(|_| "Cannot locate Granite model storage.")?;
        let source = ModelSource::Installed { root };
        // Test builds dispatch the exact same worker loop through an ignored
        // unit-test entry, so no separately built UI binary is required.
        #[cfg(test)]
        let command = tests::child_command("native");
        #[cfg(not(test))]
        let command = {
            // Do not use recognition::worker_executable: its evaluator sibling
            // does not dispatch Granite and could otherwise start another CLI.
            let current = std::env::current_exe().map_err(|_| "Cannot locate Granite worker.")?;
            if !current
                .file_stem()
                .is_some_and(|name| name == "sonojot" || name == "SonoJot")
            {
                return Err("Granite worker must be launched from SonoJot.".into());
            }
            let mut command = Command::new(current);
            command.arg(FLAG);
            command
        };
        Self::start(source, command, cancel, cfg!(test))
    }

    fn start(
        source: ModelSource,
        mut command: Command,
        cancel: &AtomicBool,
        harness: bool,
    ) -> Result<Self, String> {
        check(cancel, Instant::now() + LOAD_LIMIT)?;
        let runtime = source.runtime();
        configure_command(&mut command, &runtime)?;
        let payload =
            serde_json::to_vec(&source).map_err(|_| "Cannot encode Granite model path.")?;
        if payload.len() > MAX_LOAD {
            return Err("Granite model path is too long.".into());
        }
        let mut worker = Self::spawn(command, harness)?;
        let reply = worker.exchange(1, &payload, cancel, LOAD_LIMIT)?;
        match reply {
            Reply::Ready { backend } if valid_backend(&backend) => Ok(worker),
            _ => Err("Granite did not load the required GPU backend.".into()),
        }
    }

    fn spawn(mut command: Command, harness: bool) -> Result<Self, String> {
        command.env(OWNER_ENV, std::process::id().to_string());
        let mut child = ContainedChild::spawn(&mut command)?;
        let mut input = child
            .child
            .stdin
            .take()
            .ok_or("Missing Granite input pipe.")?;
        let mut output = child
            .child
            .stdout
            .take()
            .ok_or("Missing Granite output pipe.")?;
        let (send_request, receive_request) = mpsc::sync_channel::<Vec<u8>>(1);
        let (send_reply, receive_reply) = mpsc::sync_channel(1);
        let io = thread::Builder::new()
            .name("granite-ipc".into())
            .spawn(move || {
                let startup = read_magic(&mut output, harness);
                if let Err(error) = startup {
                    let _ = send_reply.send(Err(error));
                    return;
                }
                while let Ok(bytes) = receive_request.recv() {
                    let result = (|| {
                        write_frame(&mut input, &bytes, MAX_REQUEST)
                            .map_err(|_| "Granite worker stopped while receiving audio.")?;
                        let bytes = read_frame(&mut output, MAX_RESPONSE)
                            .map_err(|_| "Granite worker stopped without a complete response.")?;
                        serde_json::from_slice::<Response>(&bytes)
                            .map_err(|_| "Granite worker returned an invalid response.".into())
                    })();
                    let failed = result.is_err();
                    if send_reply.send(result).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|_| "Cannot start Granite IPC reader.")?;
        Ok(Self {
            child,
            requests: Some(send_request),
            replies: receive_reply,
            io: Some(io),
            sequence: 0,
            stopped: false,
        })
    }

    fn exchange(
        &mut self,
        operation: u8,
        payload: &[u8],
        cancel: &AtomicBool,
        limit: Duration,
    ) -> Result<Reply, String> {
        let deadline = Instant::now() + limit;
        let result = (|| {
            check(cancel, deadline)?;
            if self.stopped {
                return Err("Granite worker is no longer running.".into());
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or("Granite request sequence exhausted.")?;
            self.requests
                .as_ref()
                .ok_or("Granite worker is no longer running.")?
                .send(request(self.sequence, operation, payload))
                .map_err(|_| "Granite worker stopped before the request.")?;
            loop {
                check(cancel, deadline)?;
                match self.replies.recv_timeout(POLL) {
                    Ok(result) => {
                        let response = result?;
                        check(cancel, deadline)?;
                        if response.version != VERSION || response.sequence != self.sequence {
                            return Err("Granite worker returned a mismatched response.".into());
                        }
                        if response.result.is_ok()
                            && self
                                .child
                                .child
                                .try_wait()
                                .map_err(|_| "Cannot inspect Granite worker.")?
                                .is_some()
                        {
                            return Err("Granite worker exited during transcription.".into());
                        }
                        return response.result;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err("Granite worker stopped without a complete response.".into());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if self
                            .child
                            .child
                            .try_wait()
                            .map_err(|_| "Cannot inspect Granite worker.")?
                            .is_some()
                        {
                            return Err("Granite worker exited during transcription.".into());
                        }
                    }
                }
            }
        })();
        if result.is_err() {
            self.stop();
        }
        result
    }

    pub(crate) fn transcribe(
        &mut self,
        samples: &[f32],
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        let result = (|| {
            crate::ai::check_cancel(cancel)?;
            validate_samples(samples)?;
            let mut bytes = Vec::with_capacity(samples.len() * 4);
            for sample in samples {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            match self.exchange(2, &bytes, cancel, RUN_LIMIT)? {
                Reply::Text { text } if text.len() <= MAX_TEXT && !text.contains('\0') => Ok(text),
                Reply::InputTooLong => Err(INPUT_TOO_LONG.into()),
                _ => Err("Granite worker returned invalid transcript text.".into()),
            }
        })();
        if result.as_ref().is_err_and(|error| error != INPUT_TOO_LONG) {
            self.stop();
        }
        result
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.requests.take();
        self.child.terminate(); // closes pipes even when native code/IPC is blocked
        if let Some(io) = self.io.take() {
            let _ = io.join();
        }
    }
}
impl Drop for GraniteWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn configure_command(command: &mut Command, runtime: &Path) -> Result<(), String> {
    let runtime = runtime
        .canonicalize()
        .map_err(|_| "Granite runtime is not installed.")?;
    let mut paths = vec![runtime.clone()];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    command
        .current_dir(&runtime)
        .env(
            "PATH",
            std::env::join_paths(paths).map_err(|_| "Invalid Granite runtime search path.")?,
        )
        .env_remove("GGML_BACKEND_PATH")
        .env(
            "TRANSCRIBE_BACKENDS",
            if cfg!(windows) {
                "cuda,cpu"
            } else {
                "metal,cpu"
            },
        );
    #[cfg(target_os = "macos")]
    command.env("DYLD_LIBRARY_PATH", &runtime);
    Ok(())
}

fn read_magic(reader: &mut impl Read, harness: bool) -> Result<(), String> {
    // libtest writes a banner before entering an ignored child test. Only
    // test-built callers may skip that bounded banner; production is exact.
    #[cfg(test)]
    if harness {
        let mut window = [0; 8];
        for _ in 0..4096 {
            window.rotate_left(1);
            reader
                .read_exact(&mut window[7..])
                .map_err(|_| "Granite worker failed to start.")?;
            if &window == MAGIC {
                return Ok(());
            }
        }
        return Err("Invalid Granite test worker handshake.".into());
    }
    let _ = harness;
    let mut magic = [0; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|_| "Granite worker failed to start.")?;
    if &magic != MAGIC {
        return Err("Invalid Granite worker handshake.".into());
    }
    Ok(())
}

// OwnedChild forces null stdio, so this pipe-based worker owns equivalent job
// containment locally instead of changing the protected recognition module.
struct ContainedChild {
    child: Child,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
#[cfg(windows)]
unsafe impl Send for ContainedChild {} // exclusive process/job ownership
impl ContainedChild {
    fn spawn(command: &mut Command) -> Result<Self, String> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = crate::child_lifetime::spawn(command)
            .map_err(|_| "Cannot start Granite worker.")?;
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
                return Err("Cannot contain Granite worker.".into());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                owned.job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(owned.job, owned.child.as_raw_handle()) == 0
            {
                return Err("Cannot contain Granite worker.".into());
            }
        }
        #[cfg(not(windows))]
        let _ = &mut owned;
        Ok(owned)
    }
    fn terminate(&mut self) {
        #[cfg(windows)]
        if !self.job.is_null() {
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
            }
        }
        let _ = crate::child_lifetime::kill(&mut self.child);
        let _ = self.child.wait();
    }
}
impl Drop for ContainedChild {
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

/// Must run before Tauri. A malformed worker invocation exits; it never opens UI.
pub fn dispatch_worker() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(FLAG)) {
        return false;
    }
    let result = if args.next().is_some() {
        Err("Unexpected Granite worker arguments.".into())
    } else {
        worker_entry()
    };
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

fn worker_entry() -> Result<(), String> {
    arm_owner_watch()?;
    let mut output = protocol_output()?;
    output
        .write_all(MAGIC)
        .and_then(|()| output.flush())
        .map_err(|_| "Cannot start Granite protocol.")?;
    worker_loop(&mut io::stdin().lock(), &mut output, |source| {
        let (model, runtime) = source.verify()?;
        Native::load(&model, &runtime)
    })
}

trait Engine {
    fn backend(&self) -> &str;
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String>;
}

fn worker_loop<E: Engine>(
    input: &mut impl Read,
    output: &mut impl Write,
    load: impl FnOnce(ModelSource) -> Result<E, String>,
) -> Result<(), String> {
    let mut load = Some(load);
    let mut engine: Option<E> = None;
    let mut sequence = 0u64;
    loop {
        let bytes = read_frame(input, MAX_REQUEST)
            .map_err(|_| "Granite input pipe closed or malformed.")?;
        sequence = sequence
            .checked_add(1)
            .ok_or("Granite request sequence exhausted.")?;
        let (operation, payload) = parse_request(&bytes, sequence)?;
        let result = match operation {
            1 if engine.is_none() && payload.len() <= MAX_LOAD => (|| {
                let source = serde_json::from_slice(payload)
                    .map_err(|_| "Invalid Granite model request.")?;
                let loaded = load.take().ok_or("Granite was already loaded.")?(source)?;
                let backend = loaded.backend().to_owned();
                if !valid_backend(&backend) {
                    return Err("Granite requires its GPU backend.".into());
                }
                engine = Some(loaded);
                Ok(Reply::Ready { backend })
            })(),
            2 if engine.is_some() => decode_samples(payload).and_then(|samples| {
                let text = match engine.as_mut().unwrap().transcribe(&samples) {
                    Err(error) if error == INPUT_TOO_LONG => return Ok(Reply::InputTooLong),
                    result => result?,
                };
                if text.len() > MAX_TEXT || text.contains('\0') {
                    return Err("Granite returned invalid transcript text.".into());
                }
                Ok(Reply::Text { text })
            }),
            _ => Err("Invalid Granite worker operation.".into()),
        };
        let failed = result.is_err();
        let response = serde_json::to_vec(&Response {
            version: VERSION,
            sequence,
            result,
        })
        .map_err(|_| "Cannot encode Granite response.")?;
        write_frame(output, &response, MAX_RESPONSE).map_err(|_| "Granite output pipe closed.")?;
        if failed {
            return Err("Granite request failed.".into());
        }
    }
}

fn valid_backend(backend: &str) -> bool {
    // Match the device budgeted by the resource manager. A different CUDA
    // device's free memory must not authorize this load.
    backend.eq_ignore_ascii_case(if cfg!(windows) { "CUDA0" } else { "MTL0" })
}

// Parent-death monitoring is armed before verification/native work. Windows
// jobs cover the app lifetime; the watcher also closes the spawn/assign race.
// macOS uses the same EVFILT_PROC/NOTE_EXIT guardian mechanism as llama_asr.
fn arm_owner_watch() -> Result<(), String> {
    let owner = std::env::var(OWNER_ENV)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 1 && *pid != std::process::id())
        .ok_or("Missing Granite worker owner.")?;
    std::env::remove_var(OWNER_ENV);
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Threading::*;
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, owner) };
        if handle.is_null() {
            return Err("Granite worker owner has exited.".into());
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        thread::Builder::new()
            .name("granite-owner-watch".into())
            .spawn(move || unsafe {
                WaitForSingleObject(handle.as_raw_handle(), INFINITE);
                TerminateProcess(GetCurrentProcess(), 1);
            })
            .map_err(|_| "Cannot watch Granite worker owner.")?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        if unsafe { libc::getppid() } != owner as libc::pid_t {
            return Err("Granite worker owner has exited.".into());
        }
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err("Cannot watch Granite worker owner.".into());
        }
        let queue = unsafe { OwnedFd::from_raw_fd(fd) };
        let event = libc::kevent {
            ident: owner as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
            || unsafe { libc::kevent(fd, &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) } < 0
            || unsafe { libc::getppid() } != owner as libc::pid_t
        {
            return Err("Granite worker owner has exited.".into());
        }
        thread::Builder::new()
            .name("granite-owner-watch".into())
            .spawn(move || {
                let mut event: libc::kevent = unsafe { std::mem::zeroed() };
                loop {
                    let result = unsafe {
                        libc::kevent(
                            queue.as_raw_fd(),
                            std::ptr::null(),
                            0,
                            &mut event,
                            1,
                            std::ptr::null(),
                        )
                    };
                    if result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    unsafe { libc::_exit(1) }
                }
            })
            .map_err(|_| "Cannot watch Granite worker owner.")?;
        return Ok(());
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = owner;
        Err("Granite is not available on this platform.".into())
    }
}

/// Retain a private copy of the protocol pipe, then route native stdout to
/// stderr at both the OS and C-runtime layers before loading any library.
#[cfg(windows)]
fn protocol_output() -> Result<File, String> {
    use std::os::windows::io::{BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
        fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
    }
    #[link(name = "ucrt")]
    extern "C" {
        fn _open_osfhandle(handle: isize, flags: i32) -> i32;
        fn _dup2(source: i32, destination: i32) -> i32;
        fn _close(descriptor: i32) -> i32;
    }
    unsafe {
        let stdout = GetStdHandle(-11i32 as u32);
        if stdout.is_null() || stdout as isize == -1 {
            return Err("Missing Granite protocol output.".into());
        }
        let pipe = BorrowedHandle::borrow_raw(stdout)
            .try_clone_to_owned()
            .map_err(|_| "Cannot retain Granite protocol pipe.")?;
        let stderr = GetStdHandle(-12i32 as u32);
        if stderr.is_null() || stderr as isize == -1 {
            return Err("Missing Granite log output.".into());
        }
        // A GUI-subsystem executable may have valid inherited Win32 standard
        // handles but unopened CRT descriptors. Open our own stderr descriptor.
        let error_handle = BorrowedHandle::borrow_raw(stderr)
            .try_clone_to_owned()
            .map_err(|_| "Cannot retain Granite log output.")?
            .into_raw_handle();
        let error_fd = _open_osfhandle(error_handle as isize, 0x8001); // binary, write only
        if error_fd < 0 {
            drop(OwnedHandle::from_raw_handle(error_handle));
            return Err("Cannot open Granite native log output.".into());
        }
        let redirected = _dup2(error_fd, 1) == 0;
        if error_fd != 1 {
            _close(error_fd);
        }
        if !redirected || SetStdHandle(-11i32 as u32, stderr) == 0 {
            return Err("Cannot isolate Granite native output.".into());
        }
        Ok(File::from(pipe))
    }
}
#[cfg(unix)]
fn protocol_output() -> Result<File, String> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) };
    if fd < 0 {
        return Err("Cannot retain Granite protocol pipe.".into());
    }
    let pipe = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        return Err("Cannot isolate Granite native output.".into());
    }
    Ok(pipe)
}

// Exact C layouts from transcribe.cpp v0.3.1, commit
// 3f32fbcc7bb3246851a0234263438bc3c0fa1cac/include/transcribe.h.
// Query native sizeof/alignof before calling any *_init that writes a struct.
use std::ffi::{c_char, c_int, c_void, CStr, CString};
#[repr(C)]
struct LoadParams {
    struct_size: u64,
    backend: c_int,
    device: *mut c_void,
}
#[repr(C)]
struct SessionParams {
    struct_size: u64,
    n_threads: c_int,
    kv_type: c_int,
    n_ctx: i32,
}
#[repr(C)]
struct DeviceInfo {
    struct_size: u64,
    name: *const c_char,
    description: *const c_char,
    kind: *const c_char,
    device_id: *const c_char,
    memory_total: u64,
    memory_free: u64,
    device_type: c_int,
}
#[repr(C)]
struct RunParams {
    struct_size: u64,
    task: c_int,
    timestamps: c_int,
    pnc: c_int,
    itn: c_int,
    diarize: c_int,
    language: *const c_char,
    target_language: *const c_char,
    keep_special_tags: bool,
    family: *const c_void,
    spec_k_drafts: i32,
    vocabulary: *const *const c_char,
    n_vocabulary: i32,
    prompt: *const c_char,
    prefix: *const c_char,
}

struct Native {
    model: *mut c_void,
    session: *mut c_void,
    backend: String,
    model_free: unsafe extern "C" fn(*mut c_void),
    session_free: unsafe extern "C" fn(*mut c_void),
    run_init: unsafe extern "C" fn(*mut RunParams),
    run: unsafe extern "C" fn(*mut c_void, *const f32, c_int, *const RunParams) -> c_int,
    full_text: unsafe extern "C" fn(*const c_void) -> *const c_char,
    was_truncated: unsafe extern "C" fn(*const c_void) -> bool,
    _library: libloading::Library, // freed after session/model, never before
}

impl Native {
    fn load(model: &Path, runtime: &Path) -> Result<Self, String> {
        let runtime = runtime
            .canonicalize()
            .map_err(|_| "Cannot resolve Granite runtime.")?;
        let model = model
            .canonicalize()
            .map_err(|_| "Cannot resolve Granite model.")?;
        let path_string = |path: &Path| -> Result<CString, String> {
            CString::new(
                path.to_str()
                    .ok_or("Granite paths must be valid Unicode.")?,
            )
            .map_err(|_| "Invalid Granite path.".into())
        };
        let model_path = path_string(&model)?;
        let runtime_path = path_string(&runtime)?;
        #[cfg(windows)]
        prepare_windows_loader(&runtime)?;
        let name = if cfg!(windows) {
            "transcribe.dll"
        } else {
            "libtranscribe.dylib"
        };
        // Verification is complete before this entry. Paths are absolute;
        // transcribe_init_backends scans this verified directory only.
        let library = unsafe { libloading::Library::new(runtime.join(name)) }
            .map_err(|_| "Cannot load the Granite runtime and its GPU dependencies.")?;
        unsafe {
            macro_rules! symbol {
                ($name:literal, $ty:ty) => {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|_| concat!("Granite runtime is missing ", $name, "."))?
                };
            }
            let version = symbol!(
                "transcribe_version",
                unsafe extern "C" fn() -> *const c_char
            );
            let commit = symbol!(
                "transcribe_version_commit",
                unsafe extern "C" fn() -> *const c_char
            );
            let version = native_string(version())?;
            let commit = native_string(commit())?;
            if version != "0.3.1"
                || commit.len() < 7
                || !crate::granite_model::RUNTIME_REVISION.starts_with(&commit)
            {
                return Err("Granite runtime version does not match the pinned release.".into());
            }
            let size = symbol!(
                "transcribe_abi_struct_size",
                unsafe extern "C" fn(c_int) -> usize
            );
            let align = symbol!(
                "transcribe_abi_struct_align",
                unsafe extern "C" fn(c_int) -> usize
            );
            for (id, expected_size, expected_align) in [
                (
                    0,
                    std::mem::size_of::<LoadParams>(),
                    std::mem::align_of::<LoadParams>(),
                ),
                (
                    1,
                    std::mem::size_of::<SessionParams>(),
                    std::mem::align_of::<SessionParams>(),
                ),
                (
                    2,
                    std::mem::size_of::<RunParams>(),
                    std::mem::align_of::<RunParams>(),
                ),
                (
                    13,
                    std::mem::size_of::<DeviceInfo>(),
                    std::mem::align_of::<DeviceInfo>(),
                ),
            ] {
                if size(id) != expected_size || align(id) != expected_align {
                    return Err("Granite runtime C ABI does not match this application.".into());
                }
            }
            let init_backends = symbol!(
                "transcribe_init_backends",
                unsafe extern "C" fn(*const c_char) -> c_int
            );
            let available = symbol!(
                "transcribe_backend_available",
                unsafe extern "C" fn(c_int) -> bool
            );
            let device_count = symbol!("transcribe_device_count", unsafe extern "C" fn() -> c_int);
            let device_get = symbol!(
                "transcribe_device_get",
                unsafe extern "C" fn(c_int) -> *mut c_void
            );
            let device_info_init = symbol!(
                "transcribe_device_info_init",
                unsafe extern "C" fn(*mut DeviceInfo)
            );
            let device_info = symbol!(
                "transcribe_device_get_info",
                unsafe extern "C" fn(*mut c_void, *mut DeviceInfo) -> c_int
            );
            let load_init = symbol!(
                "transcribe_model_load_params_init",
                unsafe extern "C" fn(*mut LoadParams)
            );
            let load = symbol!(
                "transcribe_model_load_file",
                unsafe extern "C" fn(*const c_char, *const LoadParams, *mut *mut c_void) -> c_int
            );
            let session_init = symbol!(
                "transcribe_session_params_init",
                unsafe extern "C" fn(*mut SessionParams)
            );
            let open = symbol!(
                "transcribe_session_init",
                unsafe extern "C" fn(*mut c_void, *const SessionParams, *mut *mut c_void) -> c_int
            );
            let backend = symbol!(
                "transcribe_model_backend",
                unsafe extern "C" fn(*const c_void) -> *const c_char
            );
            let model_free = symbol!("transcribe_model_free", unsafe extern "C" fn(*mut c_void));
            let session_free =
                symbol!("transcribe_session_free", unsafe extern "C" fn(*mut c_void));
            let run_init = symbol!(
                "transcribe_run_params_init",
                unsafe extern "C" fn(*mut RunParams)
            );
            let run = symbol!(
                "transcribe_run",
                unsafe extern "C" fn(*mut c_void, *const f32, c_int, *const RunParams) -> c_int
            );
            let full_text = symbol!(
                "transcribe_full_text",
                unsafe extern "C" fn(*const c_void) -> *const c_char
            );
            let was_truncated = symbol!(
                "transcribe_was_truncated",
                unsafe extern "C" fn(*const c_void) -> bool
            );
            let mut native = Self {
                model: std::ptr::null_mut(),
                session: std::ptr::null_mut(),
                backend: String::new(),
                model_free,
                session_free,
                run_init,
                run,
                full_text,
                was_truncated,
                _library: library,
            };
            let requested = if cfg!(windows) { 5 } else { 2 }; // CUDA / Metal, never AUTO
            if init_backends(runtime_path.as_ptr()) != 0 || !available(requested) {
                return Err(if cfg!(windows) {
                    "Granite requires a working NVIDIA CUDA driver and its CUDA 12 runtime."
                } else {
                    "Granite requires a working Apple Metal backend."
                }
                .into());
            }
            let mut params: LoadParams = std::mem::zeroed();
            load_init(&mut params);
            params.backend = requested;
            // The library otherwise tries another GPU when the first fails.
            // Bind the budgeted device before allocating any model weights.
            for index in 0..device_count().clamp(0, 64) {
                let device = device_get(index);
                if device.is_null() {
                    continue;
                }
                let mut info: DeviceInfo = std::mem::zeroed();
                device_info_init(&mut info);
                status(device_info(device, &mut info), "device query")?;
                if valid_backend(&native_string(info.name)?) {
                    params.device = device;
                    break;
                }
            }
            if params.device.is_null() {
                return Err("Granite cannot use the GPU selected by the memory budget.".into());
            }
            status(
                load(model_path.as_ptr(), &params, &mut native.model),
                "model load",
            )?;
            if native.model.is_null() {
                return Err("Granite returned no model.".into());
            }
            native.backend = native_string(backend(native.model))?;
            if !valid_backend(&native.backend) {
                return Err("Granite did not load the required GPU backend.".into());
            }
            let mut params: SessionParams = std::mem::zeroed();
            session_init(&mut params);
            params.n_threads = 6;
            status(
                open(native.model, &params, &mut native.session),
                "session load",
            )?;
            if native.session.is_null() {
                return Err("Granite returned no session.".into());
            }
            Ok(native)
        }
    }
}
impl Engine for Native {
    fn backend(&self) -> &str {
        &self.backend
    }
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        validate_samples(samples)?;
        unsafe {
            let mut params: RunParams = std::mem::zeroed();
            (self.run_init)(&mut params);
            params.timestamps = 0; // NAR supplies no word/segment alignment
            status(
                (self.run)(
                    self.session,
                    samples.as_ptr(),
                    samples.len() as c_int,
                    &params,
                ),
                "transcription",
            )?;
            if (self.was_truncated)(self.session) {
                return Err("Granite returned an incomplete transcript.".into());
            }
            native_string((self.full_text)(self.session))
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            if !self.session.is_null() {
                (self.session_free)(self.session);
            }
            if !self.model.is_null() {
                (self.model_free)(self.model);
            }
        }
    }
}
unsafe fn native_string(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("Granite returned a missing string.".into());
    }
    let text = CStr::from_ptr(pointer)
        .to_str()
        .map_err(|_| "Granite returned invalid UTF-8.")?;
    if text.len() > MAX_TEXT {
        return Err("Granite returned too much text.".into());
    }
    Ok(text.to_owned())
}
fn status(code: c_int, operation: &str) -> Result<(), String> {
    match code {
        0 => Ok(()),
        17 => Err(INPUT_TOO_LONG.into()),
        18 => Err("Granite returned an incomplete transcript.".into()),
        _ => Err(format!(
            "Granite {operation} failed (native status {code})."
        )),
    }
}

#[cfg(windows)]
fn prepare_windows_loader(runtime: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDefaultDllDirectories(flags: u32) -> i32;
        fn AddDllDirectory(directory: *const u16) -> *mut c_void;
    }
    let directory: Vec<u16> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
    // This child loads one runtime then exits. Keep its directory cookie for
    // its entire lifetime; no executable/current-directory DLL search remains.
    unsafe {
        if SetDefaultDllDirectories(0x0000_0800 | 0x0000_0400) == 0 // SYSTEM32 | USER_DIRS
            || AddDllDirectory(directory.as_ptr()).is_null()
        {
            return Err("Cannot configure Granite DLL search directory.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{atomic::Ordering, Arc},
    };

    #[test]
    fn frames_reject_oversized_truncated_and_malformed_audio() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, b"hello", 5).unwrap();
        assert_eq!(read_frame(&mut Cursor::new(bytes), 5).unwrap(), b"hello");
        for bytes in [
            vec![],
            vec![0; 4],
            (MAX_REQUEST as u32 + 1).to_le_bytes().to_vec(),
            vec![2, 0, 0, 0, 1],
        ] {
            assert!(read_frame(&mut Cursor::new(bytes), MAX_REQUEST).is_err());
        }
        assert!(write_frame(&mut Vec::new(), &[1; 5], 4).is_err());
        assert!(decode_samples(&[]).is_err());
        assert!(decode_samples(&[1, 2, 3]).is_err());
        assert!(decode_samples(&f32::NAN.to_le_bytes()).is_err());
        assert!(validate_samples(&[f32::INFINITY]).is_err());
        assert!(validate_samples(&vec![0.; MAX_CHUNK_SAMPLES + 1]).is_err());
        assert!(validate_samples(&vec![0.; MAX_CHUNK_SAMPLES]).is_ok());
        let pcm: Vec<u8> = [-0.75f32, 0.0, 0.5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        assert_eq!(decode_samples(&pcm).unwrap(), [-0.75, 0.0, 0.5]);
    }

    #[test]
    fn protocol_requires_version_sequence_gpu_and_complete_output() {
        let good = request(1, 2, &[0; 4]);
        assert_eq!(parse_request(&good, 1).unwrap(), (2, &[0u8; 4][..]));
        assert!(parse_request(&good, 2).is_err());
        for length in 0..HEADER {
            assert!(parse_request(&good[..length], 1).is_err());
        }
        let mut wrong_version = good;
        wrong_version[0] = 2;
        assert!(parse_request(&wrong_version, 1).is_err());
        assert!(!valid_backend("CPU"));
        assert!(!valid_backend("CPU (CUDA available)"));
        assert!(!valid_backend("CUDA1"));
        assert!(!valid_backend("MTL1"));
        assert!(valid_backend(if cfg!(windows) { "CUDA0" } else { "MTL0" }));
        assert!(status(17, "transcription").unwrap_err().contains("shorter"));
        assert!(status(18, "transcription")
            .unwrap_err()
            .contains("incomplete"));
        assert!(status(8, "transcription").is_err());
        assert!(read_magic(&mut Cursor::new(b"noiseSGNAR001"), false).is_err());
        assert!(read_magic(&mut Cursor::new(b"running 1 test\nSGNAR001"), true).is_ok());
        assert!(read_magic(&mut Cursor::new(vec![0; 4096]), true).is_err());
    }

    pub(super) fn child_command(behavior: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "granite_worker::tests::worker_child_entry",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("SONOJOT_GRANITE_FIXTURE", behavior);
        command
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn fixture(behavior: &str) -> GraniteWorker {
        let mut worker = GraniteWorker::spawn(child_command(behavior), true).unwrap();
        let source = serde_json::to_vec(&ModelSource::Installed {
            root: PathBuf::from("unused-fixture-root"),
        })
        .unwrap();
        assert!(matches!(
            worker
                .exchange(1, &source, &AtomicBool::new(false), Duration::from_secs(15))
                .unwrap(),
            Reply::Ready { .. }
        ));
        worker
    }

    struct Fixture {
        behavior: String,
        calls: usize,
        nested: Option<GraniteWorker>,
    }
    impl Engine for Fixture {
        fn backend(&self) -> &str {
            if cfg!(windows) {
                "CUDA0"
            } else {
                "MTL0"
            }
        }
        fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
            match self.behavior.as_str() {
                "crash" => std::process::exit(71),
                "hang" => loop {
                    thread::sleep(Duration::from_secs(1));
                },
                "failure" => return Err("Granite fixture native failure.".into()),
                "too_long" if samples.len() > 2 => return Err(INPUT_TOO_LONG.into()),
                #[cfg(any(windows, target_os = "macos"))]
                "owner" => {
                    let nested = fixture("normal");
                    let pid = nested.child.child.id();
                    self.nested = Some(nested);
                    return Ok(pid.to_string());
                }
                _ => {}
            }
            // These must not corrupt the private protocol pipe, even when a
            // native runtime writes to stdout instead of its logging callback.
            println!("native stdout noise");
            #[cfg(windows)]
            unsafe {
                #[link(name = "ucrt")]
                extern "C" {
                    fn puts(message: *const c_char) -> c_int;
                    fn fflush(stream: *mut c_void) -> c_int;
                }
                puts(c"native C stdout noise".as_ptr());
                fflush(std::ptr::null_mut());
            }
            #[cfg(unix)]
            unsafe {
                libc::puts(c"native C stdout noise".as_ptr());
                libc::fflush(std::ptr::null_mut());
            }
            self.calls += 1;
            Ok(format!(
                "chunk {}: {} samples, first {}",
                self.calls,
                samples.len(),
                samples[0]
            ))
        }
    }

    /// Dedicated subprocess entry. Ordinary `cargo test --lib` never runs it;
    /// running all ignored tests directly is also harmless without its env.
    #[test]
    #[ignore]
    fn worker_child_entry() {
        let Ok(behavior) = std::env::var("SONOJOT_GRANITE_FIXTURE") else {
            return;
        };
        std::env::remove_var("SONOJOT_GRANITE_FIXTURE");
        let result = if behavior == "native" {
            worker_entry()
        } else {
            (|| {
                arm_owner_watch()?;
                let mut output = protocol_output()?;
                output.write_all(MAGIC).unwrap();
                output.flush().unwrap();
                let mut input = io::stdin().lock();
                if ["partial", "malformed", "wrong_sequence"].contains(&behavior.as_str()) {
                    // A valid load followed by an interrupted or invalid result
                    // exercises the parent against a genuinely separate process.
                    let load = read_frame(&mut input, MAX_REQUEST).unwrap();
                    assert_eq!(parse_request(&load, 1).unwrap().0, 1);
                    let reply = Response {
                        version: VERSION,
                        sequence: 1,
                        result: Ok(Reply::Ready {
                            backend: if cfg!(windows) { "CUDA0" } else { "MTL0" }.into(),
                        }),
                    };
                    write_frame(
                        &mut output,
                        &serde_json::to_vec(&reply).unwrap(),
                        MAX_RESPONSE,
                    )
                    .unwrap();
                    let _ = read_frame(&mut input, MAX_REQUEST).unwrap();
                    match behavior.as_str() {
                        "partial" => {
                            output.write_all(&200u32.to_le_bytes()).unwrap();
                            output.write_all(b"{\"partial transcript\":").unwrap();
                            output.flush().unwrap();
                            std::process::exit(72);
                        }
                        "malformed" => write_frame(&mut output, b"not JSON", MAX_RESPONSE).unwrap(),
                        _ => {
                            let reply = Response {
                                version: VERSION,
                                sequence: 99,
                                result: Ok(Reply::Text {
                                    text: "must not escape".into(),
                                }),
                            };
                            write_frame(
                                &mut output,
                                &serde_json::to_vec(&reply).unwrap(),
                                MAX_RESPONSE,
                            )
                            .unwrap();
                        }
                    }
                    loop {
                        thread::sleep(Duration::from_secs(1));
                    }
                }
                worker_loop(&mut input, &mut output, |_| {
                    if behavior == "load_hang" {
                        loop {
                            thread::sleep(Duration::from_secs(1));
                        }
                    }
                    Ok(Fixture {
                        behavior,
                        calls: 0,
                        nested: None,
                    })
                })
            })()
        };
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_spawn_chunks_reuse_model_and_keep_stdout_clean() {
        let mut worker = fixture("normal");
        let pid = worker.child.child.id();
        for count in 1..=2 {
            assert_eq!(
                worker
                    .transcribe(&[-0.25, 0.5, 0.0], &AtomicBool::new(false))
                    .unwrap(),
                format!("chunk {count}: 3 samples, first -0.25")
            );
            assert_eq!(worker.child.child.id(), pid);
        }
        let process = observe_process(pid);
        drop(worker);
        assert_exited(process);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_cancel_and_deadline_kill_and_reap() {
        for behavior in ["hang", "load_hang"] {
            let cancel = Arc::new(AtomicBool::new(false));
            let mut worker = if behavior == "hang" {
                fixture(behavior)
            } else {
                GraniteWorker::spawn(child_command(behavior), true).unwrap()
            };
            let signal = Arc::clone(&cancel);
            let trigger = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                signal.store(true, Ordering::Release);
            });
            let started = Instant::now();
            let error = if behavior == "hang" {
                worker.transcribe(&[0.0; 160], &cancel).unwrap_err()
            } else {
                let source = serde_json::to_vec(&ModelSource::Installed {
                    root: PathBuf::from("unused"),
                })
                .unwrap();
                worker
                    .exchange(1, &source, &cancel, LOAD_LIMIT)
                    .unwrap_err()
            };
            trigger.join().unwrap();
            assert_eq!(error, crate::ai::check_cancel(&cancel).unwrap_err());
            assert!(worker.child.child.try_wait().unwrap().is_some());
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        let mut worker = fixture("hang");
        let started = Instant::now();
        assert!(worker
            .exchange(
                2,
                &0.0f32.to_le_bytes(),
                &AtomicBool::new(false),
                Duration::from_millis(100)
            )
            .unwrap_err()
            .contains("time limit"));
        assert!(worker.child.child.try_wait().unwrap().is_some());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_crash_partial_and_invalid_results_are_errors() {
        for behavior in ["crash", "partial", "malformed", "wrong_sequence", "failure"] {
            let mut worker = fixture(behavior);
            let result = worker.transcribe(&[0.25; 160], &AtomicBool::new(false));
            assert!(result.is_err(), "{behavior} returned text: {result:?}");
            if behavior == "failure" {
                assert_eq!(result.unwrap_err(), "Granite fixture native failure.");
            }
            assert!(worker.child.child.try_wait().unwrap().is_some());
            assert!(worker
                .transcribe(&[0.0; 160], &AtomicBool::new(false))
                .is_err());
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_rejects_bad_audio_and_cancel_before_load() {
        let mut worker = fixture("normal");
        assert!(worker
            .transcribe(&[f32::NAN], &AtomicBool::new(false))
            .is_err());
        assert!(worker.child.child.try_wait().unwrap().is_some());
        let cancel = AtomicBool::new(true);
        assert_eq!(
            GraniteWorker::load(Path::new("missing"), &cancel)
                .err()
                .unwrap(),
            crate::ai::check_cancel(&cancel).unwrap_err()
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_input_limit_allows_shorter_retry_on_the_same_model() {
        let mut worker = fixture("too_long");
        let pid = worker.child.child.id();
        assert_eq!(
            worker
                .transcribe(&[0.0; 4], &AtomicBool::new(false))
                .unwrap_err(),
            INPUT_TOO_LONG
        );
        assert_eq!(
            worker
                .transcribe(&[0.0; 2], &AtomicBool::new(false))
                .unwrap(),
            "chunk 1: 2 samples, first 0"
        );
        assert_eq!(worker.child.child.id(), pid);
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn worker_dies_when_its_owner_crashes() {
        let mut owner = fixture("owner");
        let nested_pid: u32 = owner
            .transcribe(&[0.0; 160], &AtomicBool::new(false))
            .unwrap()
            .parse()
            .unwrap();
        let nested = observe_process(nested_pid);
        // Deliberately kill only the owner process, bypassing its Rust Drop.
        // Windows closes its inner job; macOS NOTE_EXIT wakes the child's guard.
        owner.child.child.kill().unwrap();
        owner.child.child.wait().unwrap();
        assert_exited(nested);
    }

    #[cfg(windows)]
    fn observe_process(pid: u32) -> std::os::windows::io::OwnedHandle {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::Threading::*;
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null());
        unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) }
    }
    #[cfg(windows)]
    fn assert_exited(process: std::os::windows::io::OwnedHandle) {
        use std::os::windows::io::AsRawHandle;
        assert_eq!(
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    process.as_raw_handle(),
                    5000,
                )
            },
            0
        );
    }
    #[cfg(target_os = "macos")]
    fn observe_process(pid: u32) -> u32 {
        pid
    }
    #[cfg(target_os = "macos")]
    fn assert_exited(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    // Audited benchmark binaries, not editable native-files.json/receipts.
    // These CUDA 12.8 DLLs came from the benchmark's Torch environment. Shipping
    // installations always use granite_model::verify_for_load and its own pins.
    #[cfg(windows)]
    const BENCHMARK_FILES: &[(&str, u64, &str)] = &[
        (
            "transcribe.dll",
            1881600,
            "1b7f4b8dfb5ddbd19b19c7145140825f3571adcae92a547fd3676275f3a06eb1",
        ),
        (
            "ggml.dll",
            61952,
            "450b405b8232c4e596663461d03cc3e7d6f1dd2b32b7f4a715cf98e1d8ee373d",
        ),
        (
            "ggml-base.dll",
            670208,
            "c2dcbf222f9e0bc186b2548fb92e0ce5a78049c382463f3f214b0a41cf94430a",
        ),
        (
            "ggml-cpu.dll",
            845824,
            "77cf62b9330328a37026bc098c09191ccb7a246c3df7e783bb010c1b116937d3",
        ),
        (
            "ggml-cuda.dll",
            199469056,
            "d58ba63c633345e40f83a0d739f14dc0726bc1196c3e338a850751c0ad4dbc89",
        ),
        (
            "ggml-vulkan.dll",
            42624000,
            "6dc70d501941ade39286dfd8f293c3a4a7e95058ec335ba24e758db78107e03b",
        ),
        (
            "cudart64_12.dll",
            573952,
            "c2c9a9c22a9bcba90e261825968836787b331038047a26770cffb7a583c28344",
        ),
        (
            "cublas64_12.dll",
            113716224,
            "9513540e4ec4c51ee9e7304138c2cc255c29a8c181f9e80c38efa25738becd99",
        ),
        (
            "cublasLt64_12.dll",
            674667520,
            "b199d1ff892a81b7fd3d57ba1781549609b41500b36008fef326038393ad46c7",
        ),
    ];

    #[cfg(windows)]
    pub(super) fn verify_benchmark(model: &Path, runtime: &Path) -> Result<(), String> {
        if !crate::model_verification::uncached(
            model,
            2498105472,
            "4762df797b9efa1ce0d73d2f5128517c86fdfdba2e9d00cd5cca97e624760b21",
        )? {
            return Err("Granite benchmark model did not match its pinned SHA-256.".into());
        }
        for (name, bytes, hash) in BENCHMARK_FILES {
            if !crate::model_verification::uncached(&runtime.join(name), *bytes, hash)? {
                return Err(format!(
                    "Granite benchmark runtime failed verification: {name}."
                ));
            }
        }
        // Prevent ggml's CPU-variant discovery from opening an extra DLL.
        for entry in std::fs::read_dir(runtime).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
                && !BENCHMARK_FILES
                    .iter()
                    .any(|(name, _, _)| path.file_name().is_some_and(|file| file == *name))
            {
                return Err("Unexpected DLL in Granite benchmark runtime.".into());
            }
        }
        Ok(())
    }

    /// Opt-in real CUDA worker test; does not alter the cache or user storage.
    /// SONOJOT_GRANITE_TEST_ROOT uses normal installed-file verification.
    /// Otherwise CACHE/MODEL/RUNTIME/WAV may override the audited cache paths.
    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    #[ignore]
    fn real_windows_ted_worker() {
        let cache = std::env::var_os("SONOJOT_GRANITE_TEST_CACHE").map(PathBuf::from).unwrap_or_else(||
            PathBuf::from("../evaluation/.cache/asr-bench-2026-10"));
        let native = cache.join("runtime/transcribe-cpp");
        let wav = std::env::var_os("SONOJOT_GRANITE_TEST_WAV")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                cache.join("corpora/ted-round2/audio/ted26r2-AlGore_2009-short.wav")
            });
        let installed = std::env::var_os("SONOJOT_GRANITE_TEST_ROOT").map(PathBuf::from);
        let source = if let Some(root) = &installed {
            if !crate::granite_model::model_path(root).is_file()
                || !crate::granite_model::runtime_dir(root).is_dir()
                || !wav.is_file()
            {
                eprintln!("SKIP Granite TED test: installed model, runtime, or WAV is absent.");
                return;
            }
            ModelSource::Installed {
                root: root.canonicalize().unwrap(),
            }
        } else {
            let model = std::env::var_os("SONOJOT_GRANITE_TEST_MODEL")
                .map(PathBuf::from)
                .unwrap_or_else(|| native.join("granite-speech-4.1-2b-nar-Q8_0.gguf"));
            let runtime = std::env::var_os("SONOJOT_GRANITE_TEST_RUNTIME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    native.join("windows-cuda/transcribe-native-windows-x86_64-cuda")
                });
            if !model.is_file()
                || !wav.is_file()
                || BENCHMARK_FILES
                    .iter()
                    .any(|(name, _, _)| !runtime.join(name).is_file())
            {
                eprintln!("SKIP Granite TED test: cached model, DLLs, or WAV are absent.");
                return;
            }
            ModelSource::Benchmark {
                model: model.canonicalize().unwrap(),
                runtime: runtime.canonicalize().unwrap(),
            }
        };
        let mut reader = hound::WavReader::open(&wav).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (
                spec.sample_rate,
                spec.channels,
                spec.bits_per_sample,
                spec.sample_format
            ),
            (16_000, 1, 16, hound::SampleFormat::Int),
            "TED fixture must be 16 kHz mono PCM16"
        );
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect();
        validate_samples(&samples).unwrap();
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let mut worker =
            GraniteWorker::start(source, child_command("native"), &cancel, true).unwrap();
        let loaded = started.elapsed();
        let inference = Instant::now();
        let recognition =
            crate::granite_speech::transcribe_file(&mut worker, &wav, &cancel).unwrap();
        let text = recognition.text;
        let elapsed = inference.elapsed();
        drop(worker); // model/child gone before reporting success
        assert!(
            !text.trim().is_empty(),
            "Granite returned an empty TED transcript"
        );
        println!("Granite TED: {text}");
        println!("Granite sections: {}", recognition.segments.len());
        println!("Granite timing: verification + spawn + load {:.3}s; transcription {:.3}s; audio {:.3}s; total {:.3}s",
            loaded.as_secs_f64(), elapsed.as_secs_f64(), samples.len() as f64 / 16000.0, started.elapsed().as_secs_f64());
    }
}
