//! Ephemeral, disk-backed audio between microphone capture and synchronous ASR.
//!
//! Start this immediately after taking the capture receiver, before loading a
//! model. A capture stop is not cancellation: queued audio and the device tail
//! are drained through the explicit `End`. Errors invalidate the whole capture.
use crate::audio::AudioMessage;
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
        Arc, Condvar, Mutex, MutexGuard,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CANCEL_POLL: Duration = Duration::from_millis(20);
const HEADER_BYTES: usize = 24;
const IO_BUFFER_BYTES: usize = 8192;
// A single device callback should be much smaller than this (4 MiB of PCM).
// Reject oversized records instead of allowing disk data to request arbitrary
// allocations. Capture's existing queue and the caller's returned chunk remain
// outside this module's memory budget.
const MAX_CHUNK_SAMPLES: usize = 1_048_576;
const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const QUOTA_BYTES: u64 = 256 * 1024 * 1024;
const DISK_FLOOR_BYTES: u64 = 512 * 1024 * 1024;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// A single-consumer backlog. No PCM queue or per-record index grows in memory.
///
/// `Error` and `End` are each terminal: subsequent receives are disconnected.
/// Cancellation returns `Error`, never `End`. Drop stops capture and joins the
/// spool worker without holding its mutex. Local filesystem I/O is synchronous;
/// cancellation can take the time of an in-flight file operation plus 20 ms.
pub struct AudioBacklog {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    finished: bool,
}

impl AudioBacklog {
    pub fn start(
        receiver: Receiver<AudioMessage>,
        capture_stop: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if cancel.load(Ordering::Acquire) {
            capture_stop.store(true, Ordering::Release);
            return Err("Dictation cancelled.".into());
        }
        let file = TemporaryFile::create().map_err(|error| {
            capture_stop.store(true, Ordering::Release);
            incomplete(format!("could not create its temporary file: {error}"))
        })?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                spool: Some(SpoolQueue::new(file)),
                completion: Completion::Capturing,
            }),
            changed: Condvar::new(),
            capture_stop,
            cancel,
            peak_bytes: AtomicU64::new(0),
        });
        let producer = shared.clone();
        let worker = thread::Builder::new()
            .name("dictation-audio-backlog".into())
            .spawn(move || produce(receiver, producer))
            .map_err(|error| {
                let message = incomplete(format!("could not start its worker: {error}"));
                shared.fail(&mut shared.lock(), message.clone());
                message
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
            finished: false,
        })
    }

    pub(crate) fn peak_bytes(&self) -> u64 {
        self.shared.peak_bytes.load(Ordering::Relaxed)
    }

    /// Wait for committed audio, capture completion, or failure. As with a file
    /// read, reading an available record can outlast `timeout`; the timeout
    /// bounds waiting for availability. A zero timeout still reads ready data.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<AudioMessage, RecvTimeoutError> {
        if self.finished {
            return Err(RecvTimeoutError::Disconnected);
        }
        let started = Instant::now();
        let mut state = self.shared.lock();
        loop {
            self.shared.check_cancel(&mut state);
            // Failure wins over previously committed audio. The caller must
            // discard its partial transcript rather than finalize it.
            match &state.completion {
                Completion::Failed(error) => {
                    self.finished = true;
                    return Ok(AudioMessage::Error(error.clone()));
                }
                Completion::Cancelled => {
                    self.finished = true;
                    return Ok(AudioMessage::Error("Dictation cancelled.".into()));
                }
                Completion::Consumed => return Err(RecvTimeoutError::Disconnected),
                Completion::Capturing | Completion::End => {}
            }
            if let Some(spool) = &mut state.spool {
                if spool.has_audio() {
                    match spool.read_chunk() {
                        Ok(chunk) => return Ok(chunk),
                        Err(error) => {
                            self.shared.fail(
                                &mut state,
                                incomplete(format!("could not read captured audio: {error}")),
                            );
                            continue;
                        }
                    }
                }
            }
            if matches!(state.completion, Completion::End) {
                state.spool.take();
                state.completion = Completion::Consumed;
                self.finished = true;
                self.shared.changed.notify_all();
                return Ok(AudioMessage::End);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            state = self.shared.wait(state, remaining.min(CANCEL_POLL));
        }
    }
}

impl Drop for AudioBacklog {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            self.shared.cancel_capture(&mut state);
        }
        // The producer never waits for a consumer channel send. Closing its
        // receiver also releases a capture owner blocked on a terminal send.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Completion {
    Capturing,
    End,
    Failed(String),
    Cancelled,
    Consumed,
}

struct State {
    spool: Option<SpoolQueue>,
    completion: Completion,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    capture_stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    peak_bytes: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poison| {
            let mut state = poison.into_inner();
            self.fail(&mut state, incomplete("worker state was interrupted"));
            state
        })
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>, timeout: Duration) -> MutexGuard<'a, State> {
        match self.changed.wait_timeout(state, timeout) {
            Ok((state, _)) => state,
            Err(poison) => {
                let (mut state, _) = poison.into_inner();
                self.fail(&mut state, incomplete("worker state was interrupted"));
                state
            }
        }
    }

    fn check_cancel(&self, state: &mut State) {
        if self.cancel.load(Ordering::Acquire) {
            self.cancel_capture(state);
        }
    }

    fn cancel_capture(&self, state: &mut State) {
        self.capture_stop.store(true, Ordering::Release);
        state.spool.take();
        if matches!(state.completion, Completion::Capturing | Completion::End) {
            state.completion = Completion::Cancelled;
        }
        self.changed.notify_all();
    }

    fn fail(&self, state: &mut State, message: String) {
        self.capture_stop.store(true, Ordering::Release);
        state.spool.take();
        if matches!(state.completion, Completion::Capturing | Completion::End) {
            state.completion = Completion::Failed(message);
        }
        self.changed.notify_all();
    }
}

// Also closes the file and reports incomplete capture if the producer unwinds.
// The worker owns Shared, never AudioBacklog or its JoinHandle; there is no cycle.
struct ProducerExit(Arc<Shared>);

impl Drop for ProducerExit {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        if state.spool.is_some() {
            self.0.fail(
                &mut state,
                incomplete("worker exited before buffered audio was consumed"),
            );
        }
        self.0.changed.notify_all();
    }
}

fn produce(receiver: Receiver<AudioMessage>, shared: Arc<Shared>) {
    let _exit = ProducerExit(shared.clone());
    loop {
        {
            let mut state = shared.lock();
            shared.check_cancel(&mut state);
            if state.spool.is_none() {
                break;
            }
        }
        let message = match receiver.recv_timeout(CANCEL_POLL) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                shared.fail(
                    &mut shared.lock(),
                    incomplete("capture disconnected before End"),
                );
                break;
            }
        };
        let mut state = shared.lock();
        shared.check_cancel(&mut state);
        let Some(spool) = &mut state.spool else {
            break;
        };
        match message {
            AudioMessage::Chunk {
                samples,
                sample_rate,
                rms,
                captured_at,
                discontinuity,
                dropped_frames,
            } => {
                if discontinuity || dropped_frames != 0 {
                    shared.fail(&mut state, incomplete("capture reported missing audio"));
                    break;
                }
                if let Err(error) = spool.write_chunk(&samples, sample_rate, rms, captured_at) {
                    shared.fail(
                        &mut state,
                        incomplete(format!("could not save captured audio: {error}")),
                    );
                    break;
                }
                shared
                    .peak_bytes
                    .fetch_max(spool.disk_bytes(), Ordering::Relaxed);
                shared.changed.notify_all();
            }
            AudioMessage::End => {
                state.completion = Completion::End;
                shared.changed.notify_all();
                break;
            }
            AudioMessage::Error(error) => {
                shared.fail(&mut state, incomplete(format!("capture failed: {error}")));
                break;
            }
        }
    }
    drop(receiver);
    // ASR may still be loading or decoding when capture ends. Keep watching
    // cancellation until the consumer drains or drops, so idle ASR cannot keep
    // the file open after cancellation. Drop wakes this wait and joins us.
    let mut state = shared.lock();
    loop {
        shared.check_cancel(&mut state);
        if state.spool.is_none() {
            break;
        }
        state = shared.wait(state, CANCEL_POLL);
    }
}

fn incomplete(reason: impl std::fmt::Display) -> String {
    format!("Dictation audio backlog {reason}; this recording is incomplete.")
}

// Segments contain at most 8 MiB; their combined size is quota-bounded. No
// per-record index. Reading the last record closes and reclaims that segment.
struct SpoolQueue {
    segments: VecDeque<Spool>,
    quota: u64,
    #[cfg(test)]
    available: Option<u64>,
}

impl SpoolQueue {
    fn new(file: TemporaryFile) -> Self {
        Self {
            segments: VecDeque::from([Spool {
                file,
                committed: 0,
                consumed: 0,
            }]),
            quota: QUOTA_BYTES,
            #[cfg(test)]
            available: None,
        }
    }

    fn has_audio(&self) -> bool {
        self.segments.iter().any(|s| s.consumed < s.committed)
    }

    fn disk_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.committed).sum()
    }

    fn write_chunk(
        &mut self,
        samples: &[f32],
        rate: u32,
        rms: f32,
        at: Duration,
    ) -> io::Result<()> {
        if samples.len() > MAX_CHUNK_SAMPLES {
            return Err(io::Error::other("audio chunk exceeds 4 MiB"));
        }
        let bytes = HEADER_BYTES as u64 + samples.len() as u64 * 4;
        if self.disk_bytes().saturating_add(bytes) > self.quota {
            return Err(io::Error::other("temporary audio quota reached"));
        }
        #[cfg(test)]
        let free = self
            .available
            .map(Ok)
            .unwrap_or_else(temporary_disk_available)?;
        #[cfg(not(test))]
        let free = temporary_disk_available()?;
        check_disk_floor(free, bytes)?;
        if self
            .segments
            .back()
            .is_none_or(|s| s.committed + bytes > SEGMENT_BYTES)
        {
            self.segments.push_back(Spool {
                file: TemporaryFile::create()?,
                committed: 0,
                consumed: 0,
            });
        }
        self.segments
            .back_mut()
            .unwrap()
            .write_chunk(samples, rate, rms, at)
    }

    fn read_chunk(&mut self) -> io::Result<AudioMessage> {
        let multiple = self.segments.len() > 1;
        let segment = self
            .segments
            .front_mut()
            .ok_or_else(|| io::Error::other("empty spool"))?;
        let chunk = segment.read_chunk()?;
        // Retain one small drained file for reuse instead of creating/deleting
        // a file for every microphone callback. Reclaim at half a segment.
        if segment.consumed == segment.committed
            && (multiple || segment.committed >= SEGMENT_BYTES / 2)
        {
            self.segments.pop_front();
        }
        Ok(chunk)
    }
}

fn check_disk_floor(available: u64, bytes: u64) -> io::Result<()> {
    if available.saturating_sub(bytes) < DISK_FLOOR_BYTES {
        Err(io::Error::other("temporary disk safety floor reached"))
    } else {
        Ok(())
    }
}

pub(crate) fn reserve_temporary_bytes(bytes: u64) -> io::Result<()> {
    check_disk_floor(temporary_disk_available()?, bytes)
}

fn temporary_disk_available() -> io::Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = std::env::temp_dir()
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut available = 0;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(available)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(std::env::temp_dir().as_os_str().as_bytes())
            .map_err(io::Error::other)?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(io::Error::other(
            "temporary disk free-space query unsupported on this platform",
        ))
    }
}

struct Spool {
    file: TemporaryFile,
    committed: u64,
    consumed: u64,
}

impl Spool {
    fn write_chunk(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        rms: f32,
        captured_at: Duration,
    ) -> io::Result<()> {
        if samples.len() > MAX_CHUNK_SAMPLES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "audio chunk exceeds 4 MiB",
            ));
        }
        let record_bytes = HEADER_BYTES as u64 + samples.len() as u64 * 4;
        let next = self.committed.checked_add(record_bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "audio backlog offset overflow")
        })?;
        // Header: sample count, rate, RMS bits, timestamp seconds/nanoseconds.
        let mut header = [0u8; HEADER_BYTES];
        header[0..4].copy_from_slice(&(samples.len() as u32).to_le_bytes());
        header[4..8].copy_from_slice(&sample_rate.to_le_bytes());
        header[8..12].copy_from_slice(&rms.to_bits().to_le_bytes());
        header[12..20].copy_from_slice(&captured_at.as_secs().to_le_bytes());
        header[20..24].copy_from_slice(&captured_at.subsec_nanos().to_le_bytes());
        self.file.seek(SeekFrom::Start(self.committed))?;
        self.file.write_all(&header)?;
        let mut bytes = [0u8; IO_BUFFER_BYTES];
        for block in samples.chunks(IO_BUFFER_BYTES / 4) {
            for (sample, output) in block.iter().zip(bytes.chunks_exact_mut(4)) {
                output.copy_from_slice(&sample.to_bits().to_le_bytes());
            }
            self.file.write_all(&bytes[..block.len() * 4])?;
        }
        // File writes are unbuffered in this process. No fsync is needed for
        // ephemeral data; update the watermark only after every write succeeds.
        self.committed = next;
        Ok(())
    }

    fn read_chunk(&mut self) -> io::Result<AudioMessage> {
        let available = self.committed - self.consumed;
        if available < HEADER_BYTES as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "partial audio header",
            ));
        }
        self.file.seek(SeekFrom::Start(self.consumed))?;
        let mut header = [0u8; HEADER_BYTES];
        self.file.read_exact(&mut header)?;
        let count = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
        let sample_rate = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let rms = f32::from_bits(u32::from_le_bytes(header[8..12].try_into().unwrap()));
        let secs = u64::from_le_bytes(header[12..20].try_into().unwrap());
        let nanos = u32::from_le_bytes(header[20..24].try_into().unwrap());
        let record_bytes = HEADER_BYTES as u64 + count as u64 * 4;
        if count > MAX_CHUNK_SAMPLES || nanos >= 1_000_000_000 || record_bytes > available {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid or partial audio record",
            ));
        }
        let mut samples = Vec::new();
        samples.try_reserve_exact(count).map_err(io::Error::other)?;
        let mut bytes = [0u8; IO_BUFFER_BYTES];
        while samples.len() < count {
            let n = (count - samples.len()).min(IO_BUFFER_BYTES / 4) * 4;
            self.file.read_exact(&mut bytes[..n])?;
            samples.extend(
                bytes[..n]
                    .chunks_exact(4)
                    .map(|sample| f32::from_bits(u32::from_le_bytes(sample.try_into().unwrap()))),
            );
        }
        self.consumed += record_bytes;
        Ok(AudioMessage::Chunk {
            samples,
            sample_rate,
            rms,
            captured_at: Duration::new(secs, nanos),
            discontinuity: false,
            dropped_frames: 0,
        })
    }
}

pub(crate) struct TemporaryFile {
    // Option permits close-before-unlink on platforms without delete-on-close.
    file: Option<File>,
    remove_on_drop: Option<PathBuf>,
    #[cfg(test)]
    original_path: PathBuf,
    #[cfg(test)]
    write_bytes_left: Option<usize>,
    #[cfg(test)]
    read_bytes_left: Option<usize>,
}

impl TemporaryFile {
    pub(crate) fn create() -> io::Result<Self> {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..32 {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "sonojot-audio-{}-{epoch:x}-{id:x}.tmp",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                // FILE_FLAG_DELETE_ON_CLOSE; deny other opens while audio lives.
                // No temporary attribute: let Windows reclaim its file cache
                // normally instead of preferring to keep the entire spool in RAM.
                options.custom_flags(0x0400_0000).share_mode(0);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = match options.open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            };
            // The open Unix inode survives unlink and is reclaimed on close,
            // including process exit. Windows gets that guarantee from its flag.
            #[cfg(unix)]
            if let Err(error) = std::fs::remove_file(&path) {
                drop(file);
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
            return Ok(Self {
                file: Some(file),
                remove_on_drop: if cfg!(any(windows, unix)) {
                    None
                } else {
                    Some(path.clone())
                },
                #[cfg(test)]
                original_path: path,
                #[cfg(test)]
                write_bytes_left: None,
                #[cfg(test)]
                read_bytes_left: None,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique audio spool",
        ))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        self.file.take();
        // Never unlink a Windows name after closing: delete-on-close already
        // removed our file, and another process could have reused its name.
        if let Some(path) = &self.remove_on_drop {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Read for TemporaryFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        #[cfg(test)]
        if let Some(left) = &mut self.read_bytes_left {
            if *left == 0 {
                return Err(io::Error::other("injected spool read failure"));
            }
            let n = bytes.len().min(*left);
            let n = self.file.as_mut().unwrap().read(&mut bytes[..n])?;
            *left -= n;
            return Ok(n);
        }
        self.file.as_mut().unwrap().read(bytes)
    }
}

impl Write for TemporaryFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(test)]
        if let Some(left) = &mut self.write_bytes_left {
            if *left == 0 {
                return Err(io::Error::other("injected spool write failure"));
            }
            let n = self
                .file
                .as_mut()
                .unwrap()
                .write(&bytes[..bytes.len().min(*left)])?;
            *left -= n;
            return Ok(n);
        }
        self.file.as_mut().unwrap().write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().unwrap().flush()
    }
}

impl Seek for TemporaryFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.as_mut().unwrap().seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, SyncSender};

    const TEST_WAIT: Duration = Duration::from_secs(5);

    fn start(
        capacity: usize,
    ) -> (
        SyncSender<AudioMessage>,
        AudioBacklog,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        let (tx, rx) = mpsc::sync_channel(capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let backlog = AudioBacklog::start(rx, stop.clone(), cancel.clone()).unwrap();
        (tx, backlog, stop, cancel)
    }

    fn chunk(index: usize) -> AudioMessage {
        AudioMessage::Chunk {
            samples: (0..41 + index % 3)
                .map(|i| (index * 53 + i) as f32 / 100_000.0)
                .collect(),
            sample_rate: [16_000, 44_100, 48_000][index % 3],
            rms: f32::from_bits(0x3e00_0000 + index as u32),
            captured_at: Duration::new(index as u64 / 11, (index as u32 * 7919) % 1_000_000_000),
            discontinuity: false,
            dropped_frames: 0,
        }
    }

    #[test]
    fn capture_gaps_fail_closed_before_dictation_spooling() {
        let (tx, mut backlog, stop, _) = start(1);
        tx.send(AudioMessage::Chunk {
            samples: vec![0.25; 160],
            sample_rate: 16_000,
            rms: 0.25,
            captured_at: Duration::from_millis(20),
            discontinuity: true,
            dropped_frames: 160,
        }).unwrap();
        assert_incomplete(&mut backlog, "capture reported missing audio");
        join_worker(&mut backlog);
        assert!(stop.load(Ordering::Acquire));
    }

    fn assert_chunk(actual: AudioMessage, expected: AudioMessage) {
        match (actual, expected) {
            (
                AudioMessage::Chunk {
                    samples,
                    sample_rate,
                    rms,
                    captured_at,
                    ..
                },
                AudioMessage::Chunk {
                    samples: want_samples,
                    sample_rate: want_rate,
                    rms: want_rms,
                    captured_at: want_at,
                    ..
                },
            ) => {
                assert_eq!(samples.len(), want_samples.len());
                for (actual, expected) in samples.iter().zip(want_samples) {
                    assert_eq!(actual.to_bits(), expected.to_bits());
                }
                assert_eq!(sample_rate, want_rate);
                assert_eq!(rms.to_bits(), want_rms.to_bits());
                assert_eq!(captured_at, want_at);
            }
            (actual, expected) => panic!("expected {expected:?}, received {actual:?}"),
        }
    }

    fn wait_for(backlog: &AudioBacklog, predicate: impl Fn(&State) -> bool) {
        let started = Instant::now();
        let mut state = backlog.shared.lock();
        while !predicate(&state) {
            assert!(
                started.elapsed() < TEST_WAIT,
                "spool worker did not make progress"
            );
            state = backlog.shared.wait(state, CANCEL_POLL);
        }
    }

    fn join_worker(backlog: &mut AudioBacklog) {
        let worker = backlog.worker.take().unwrap();
        let started = Instant::now();
        while !worker.is_finished() {
            assert!(started.elapsed() < TEST_WAIT, "spool worker did not exit");
            thread::sleep(Duration::from_millis(1));
        }
        worker.join().unwrap();
    }

    fn path(backlog: &AudioBacklog) -> PathBuf {
        backlog
            .shared
            .lock()
            .spool
            .as_ref()
            .unwrap()
            .segments
            .front()
            .unwrap()
            .file
            .original_path
            .clone()
    }

    fn assert_incomplete(backlog: &mut AudioBacklog, detail: &str) {
        match backlog.recv_timeout(TEST_WAIT).unwrap() {
            AudioMessage::Error(error) => {
                assert!(error.contains("incomplete"), "{error}");
                assert!(error.contains(detail), "{error}");
            }
            other => panic!("expected incomplete-capture error, received {other:?}"),
        }
        assert!(matches!(
            backlog.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn held_then_slow_consumer_preserves_more_than_4096_chunks() {
        const COUNT: usize = 12_288;
        let (tx, mut backlog, stop, _) = start(4096);
        let temporary_path = path(&backlog);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let sender = thread::spawn(move || {
            for index in 0..COUNT {
                tx.send(chunk(index)).unwrap();
            }
            tx.send(AudioMessage::End).unwrap();
            done_tx.send(()).unwrap();
        });
        // No consumer receives until the bounded source has sent 3x its limit.
        // This would block if inference were still responsible for draining it.
        done_rx.recv_timeout(TEST_WAIT).unwrap();
        wait_for(&backlog, |state| {
            matches!(state.completion, Completion::End)
        });
        {
            let state = backlog.shared.lock();
            let spool = state.spool.as_ref().unwrap();
            let expected: u64 = (0..COUNT)
                .map(|i| (HEADER_BYTES + (41 + i % 3) * 4) as u64)
                .sum();
            assert_eq!(spool.segments.iter().map(|s| s.consumed).sum::<u64>(), 0);
            assert_eq!(spool.disk_bytes(), expected);
            assert_eq!(
                spool
                    .segments
                    .iter()
                    .map(|s| s.file.file.as_ref().unwrap().metadata().unwrap().len())
                    .sum::<u64>(),
                expected
            );
        }
        for index in 0..COUNT {
            assert_chunk(backlog.recv_timeout(TEST_WAIT).unwrap(), chunk(index));
            if index % 128 == 0 {
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(matches!(
            backlog.recv_timeout(TEST_WAIT),
            Ok(AudioMessage::End)
        ));
        assert!(matches!(
            backlog.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Disconnected)
        ));
        join_worker(&mut backlog);
        sender.join().unwrap();
        assert!(!temporary_path.exists());
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn stop_drains_queued_chunks_and_late_partial_tail_without_timestamp_rounding() {
        let (tx, mut backlog, stop, _) = start(8);
        tx.send(chunk(0)).unwrap();
        tx.send(chunk(1)).unwrap();
        stop.store(true, Ordering::Release);
        assert_chunk(backlog.recv_timeout(TEST_WAIT).unwrap(), chunk(0));
        assert_chunk(backlog.recv_timeout(TEST_WAIT).unwrap(), chunk(1));
        // Capture has stopped, but no End has arrived; a delayed tail is valid.
        assert!(matches!(
            backlog.recv_timeout(Duration::from_millis(30)),
            Err(RecvTimeoutError::Timeout)
        ));
        let tail = || AudioMessage::Chunk {
            samples: vec![-0.0, f32::from_bits(0x7fc0_1234), 0.125],
            sample_rate: 32_000,
            rms: -0.0,
            captured_at: Duration::new(u64::MAX, 999_999_999),
            discontinuity: false,
            dropped_frames: 0,
        };
        tx.send(tail()).unwrap();
        tx.send(AudioMessage::End).unwrap();
        assert_chunk(backlog.recv_timeout(TEST_WAIT).unwrap(), tail());
        assert!(matches!(
            backlog.recv_timeout(TEST_WAIT),
            Ok(AudioMessage::End)
        ));
        join_worker(&mut backlog);
    }

    #[test]
    fn multiblock_and_empty_chunks_round_trip_while_producer_is_active() {
        let (tx, mut backlog, _, _) = start(1);
        let samples: Vec<f32> = (0..IO_BUFFER_BYTES / 4 * 3 + 7)
            .map(|i| f32::from_bits(0x3f00_0000 + i as u32))
            .collect();
        let expected = samples.clone();
        let sender = thread::spawn(move || {
            tx.send(AudioMessage::Chunk {
                samples,
                sample_rate: 192_000,
                rms: 0.75,
                captured_at: Duration::from_nanos(123_456_789),
                discontinuity: false,
                dropped_frames: 0,
            })
            .unwrap();
            tx.send(AudioMessage::Chunk {
                samples: Vec::new(),
                sample_rate: 8_000,
                rms: 0.0,
                captured_at: Duration::ZERO,
                discontinuity: false,
                dropped_frames: 0,
            })
            .unwrap();
            tx.send(AudioMessage::End).unwrap();
        });
        assert_chunk(
            backlog.recv_timeout(TEST_WAIT).unwrap(),
            AudioMessage::Chunk {
                samples: expected,
                sample_rate: 192_000,
                rms: 0.75,
                captured_at: Duration::from_nanos(123_456_789),
                discontinuity: false,
                dropped_frames: 0,
            },
        );
        assert_chunk(
            backlog.recv_timeout(TEST_WAIT).unwrap(),
            AudioMessage::Chunk {
                samples: Vec::new(),
                sample_rate: 8_000,
                rms: 0.0,
                captured_at: Duration::ZERO,
                discontinuity: false,
                dropped_frames: 0,
            },
        );
        assert!(matches!(
            backlog.recv_timeout(TEST_WAIT),
            Ok(AudioMessage::End)
        ));
        sender.join().unwrap();
        join_worker(&mut backlog);
    }

    #[test]
    fn cancellation_closes_files_without_consumer_receives_even_after_end() {
        for ended in [false, true] {
            let (tx, mut backlog, stop, cancel) = start(8);
            let temporary_path = path(&backlog);
            tx.send(chunk(0)).unwrap();
            if ended {
                tx.send(AudioMessage::End).unwrap();
                wait_for(&backlog, |state| {
                    matches!(state.completion, Completion::End)
                });
            } else {
                wait_for(&backlog, |state| {
                    state.spool.as_ref().is_some_and(|s| s.has_audio())
                });
            }
            cancel.store(true, Ordering::Release);
            // The producer must observe cancellation while ASR is busy.
            join_worker(&mut backlog);
            assert!(backlog.shared.lock().spool.is_none());
            assert!(!temporary_path.exists());
            assert!(stop.load(Ordering::Acquire));
            assert!(matches!(
                backlog.recv_timeout(TEST_WAIT),
                Ok(AudioMessage::Error(_))
            ));
            assert!(matches!(
                backlog.recv_timeout(Duration::ZERO),
                Err(RecvTimeoutError::Disconnected)
            ));
            assert!(tx.send(AudioMessage::End).is_err());
        }
    }

    #[test]
    fn cancellation_wakes_a_long_receive() {
        let (tx, mut backlog, stop, cancel) = start(1);
        let waiter = thread::spawn(move || backlog.recv_timeout(Duration::from_secs(60)));
        cancel.store(true, Ordering::Release);
        let started = Instant::now();
        while !waiter.is_finished() {
            assert!(started.elapsed() < TEST_WAIT);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(matches!(waiter.join().unwrap(), Ok(AudioMessage::Error(_))));
        assert!(stop.load(Ordering::Acquire));
        assert!(tx.send(AudioMessage::End).is_err());
    }

    #[test]
    fn dropping_consumer_releases_blocked_sender_and_worker() {
        let (tx, backlog, stop, _) = start(1);
        let temporary_path = path(&backlog);
        let weak = Arc::downgrade(&backlog.shared);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let sender = thread::spawn(move || {
            while tx.send(chunk(0)).is_ok() {}
            // Models the capture owner attempting terminal sends after failure.
            assert!(tx
                .send(AudioMessage::Error("capture stopped".into()))
                .is_err());
            assert!(tx.send(AudioMessage::End).is_err());
            done_tx.send(()).unwrap();
        });
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
        let dropper = thread::spawn(move || {
            drop(backlog);
            dropped_tx.send(()).unwrap();
        });
        dropped_rx.recv_timeout(TEST_WAIT).unwrap();
        done_rx.recv_timeout(TEST_WAIT).unwrap();
        dropper.join().unwrap();
        sender.join().unwrap();
        assert!(
            weak.upgrade().is_none(),
            "worker retained backlog after Drop"
        );
        assert!(stop.load(Ordering::Acquire));
        assert!(!temporary_path.exists());
    }

    #[test]
    fn partial_write_failure_invalidates_committed_audio_and_cannot_yield_end() {
        let (tx, mut backlog, stop, _) = start(8);
        let temporary_path = path(&backlog);
        tx.send(chunk(0)).unwrap();
        wait_for(&backlog, |state| {
            state.spool.as_ref().is_some_and(|s| s.has_audio())
        });
        {
            let mut state = backlog.shared.lock();
            state
                .spool
                .as_mut()
                .unwrap()
                .segments
                .back_mut()
                .unwrap()
                .file
                .write_bytes_left = Some(HEADER_BYTES + 3);
        }
        tx.send(chunk(1)).unwrap();
        let _ = tx.send(AudioMessage::End);
        wait_for(&backlog, |state| {
            matches!(state.completion, Completion::Failed(_))
        });
        assert_incomplete(&mut backlog, "injected spool write failure");
        join_worker(&mut backlog);
        assert!(stop.load(Ordering::Acquire));
        assert!(!temporary_path.exists());
    }

    #[test]
    fn partial_read_failure_after_capture_end_invalidates_recording() {
        let (tx, mut backlog, stop, _) = start(8);
        let temporary_path = path(&backlog);
        tx.send(chunk(0)).unwrap();
        tx.send(chunk(1)).unwrap();
        tx.send(AudioMessage::End).unwrap();
        wait_for(&backlog, |state| {
            matches!(state.completion, Completion::End)
        });
        assert_chunk(backlog.recv_timeout(TEST_WAIT).unwrap(), chunk(0));
        {
            let mut state = backlog.shared.lock();
            state
                .spool
                .as_mut()
                .unwrap()
                .segments
                .front_mut()
                .unwrap()
                .file
                .read_bytes_left = Some(HEADER_BYTES + 3);
        }
        assert_incomplete(&mut backlog, "injected spool read failure");
        join_worker(&mut backlog);
        assert!(stop.load(Ordering::Acquire));
        assert!(!temporary_path.exists());
    }

    #[test]
    fn capture_error_or_disconnect_is_never_successful_completion() {
        for explicit_error in [false, true] {
            let (tx, mut backlog, stop, _) = start(8);
            tx.send(chunk(0)).unwrap();
            if explicit_error {
                tx.send(AudioMessage::Error("device failed".into()))
                    .unwrap();
                let _ = tx.send(AudioMessage::End);
            }
            drop(tx);
            wait_for(&backlog, |state| {
                matches!(state.completion, Completion::Failed(_))
            });
            assert_incomplete(
                &mut backlog,
                if explicit_error {
                    "device failed"
                } else {
                    "disconnected before End"
                },
            );
            join_worker(&mut backlog);
            assert!(stop.load(Ordering::Acquire));
        }
    }

    #[test]
    fn incomplete_write_never_advances_commit_boundary() {
        let mut spool = Spool {
            file: TemporaryFile::create().unwrap(),
            committed: 0,
            consumed: 0,
        };
        spool
            .write_chunk(&[0.25, 0.5], 48_000, 0.1, Duration::from_nanos(1))
            .unwrap();
        let committed = spool.committed;
        spool.file.write_bytes_left = Some(HEADER_BYTES + 1);
        assert!(spool
            .write_chunk(&[0.75], 16_000, 0.2, Duration::from_nanos(2))
            .is_err());
        assert_eq!(spool.committed, committed);
        assert_eq!(
            spool.file.file.as_ref().unwrap().metadata().unwrap().len(),
            committed + HEADER_BYTES as u64 + 1
        );
        assert_chunk(
            spool.read_chunk().unwrap(),
            AudioMessage::Chunk {
                samples: vec![0.25, 0.5],
                sample_rate: 48_000,
                rms: 0.1,
                captured_at: Duration::from_nanos(1),
                discontinuity: false,
                dropped_frames: 0,
            },
        );
        assert_eq!(spool.consumed, spool.committed);
        assert!(spool.read_chunk().is_err());
    }

    #[test]
    fn oversized_and_corrupt_records_fail_before_large_allocations() {
        let (tx, mut backlog, stop, _) = start(1);
        tx.send(AudioMessage::Chunk {
            samples: vec![0.0; MAX_CHUNK_SAMPLES + 1],
            sample_rate: 48_000,
            rms: 0.0,
            captured_at: Duration::ZERO,
            discontinuity: false,
            dropped_frames: 0,
        })
        .unwrap();
        assert_incomplete(&mut backlog, "exceeds 4 MiB");
        join_worker(&mut backlog);
        assert!(stop.load(Ordering::Acquire));

        let (tx, mut backlog, _, _) = start(1);
        tx.send(chunk(0)).unwrap();
        wait_for(&backlog, |state| {
            state.spool.as_ref().is_some_and(|s| s.has_audio())
        });
        {
            let mut state = backlog.shared.lock();
            let file = &mut state
                .spool
                .as_mut()
                .unwrap()
                .segments
                .front_mut()
                .unwrap()
                .file;
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(&u32::MAX.to_le_bytes()).unwrap();
        }
        assert_incomplete(&mut backlog, "invalid or partial audio record");
        join_worker(&mut backlog);
    }

    #[test]
    fn worker_unwind_reports_failure_and_releases_file() {
        let (tx, mut backlog, stop, _) = start(1);
        let temporary_path = path(&backlog);
        let shared = backlog.shared.clone();
        let panicked = thread::spawn(move || {
            let _exit = ProducerExit(shared.clone());
            let _state = shared.lock();
            panic!("injected worker panic while holding state");
        });
        assert!(panicked.join().is_err());
        assert_incomplete(&mut backlog, "worker state was interrupted");
        join_worker(&mut backlog);
        assert!(stop.load(Ordering::Acquire));
        assert!(!temporary_path.exists());
        assert!(tx.send(AudioMessage::End).is_err());
    }

    #[test]
    fn consumed_segments_reclaim_disk_and_allow_more_than_total_quota() {
        let mut queue = SpoolQueue::new(TemporaryFile::create().unwrap());
        let samples = vec![0.25; MAX_CHUNK_SAMPLES];
        let record = HEADER_BYTES as u64 + samples.len() as u64 * 4;
        queue.quota = record * 2;
        queue.available = Some(u64::MAX);
        for _ in 0..4 {
            queue
                .write_chunk(&samples, 16_000, 0.25, Duration::ZERO)
                .unwrap();
            queue
                .write_chunk(&samples, 16_000, 0.25, Duration::ZERO)
                .unwrap();
            assert_eq!(queue.segments.len(), 2);
            assert_eq!(queue.disk_bytes(), record * 2);
            assert!(queue
                .write_chunk(&samples, 16_000, 0.25, Duration::ZERO)
                .is_err());
            let consumed_path = queue.segments.front().unwrap().file.original_path.clone();
            queue.read_chunk().unwrap();
            assert_eq!(queue.disk_bytes(), record);
            assert!(!consumed_path.exists());
            queue.read_chunk().unwrap();
            assert_eq!(queue.disk_bytes(), 0);
            assert!(queue.segments.is_empty());
        }
    }

    #[test]
    fn disk_floor_and_quota_fail_before_writing() {
        let mut queue = SpoolQueue::new(TemporaryFile::create().unwrap());
        let bytes = HEADER_BYTES as u64 + 4;
        queue.available = Some(DISK_FLOOR_BYTES + bytes - 1);
        assert!(queue
            .write_chunk(&[0.25], 16_000, 0.25, Duration::ZERO)
            .unwrap_err()
            .to_string()
            .contains("floor"));
        assert_eq!(queue.disk_bytes(), 0);
        queue.available = Some(DISK_FLOOR_BYTES + bytes);
        queue.quota = bytes - 1;
        assert!(queue
            .write_chunk(&[0.25], 16_000, 0.25, Duration::ZERO)
            .unwrap_err()
            .to_string()
            .contains("quota"));
        assert_eq!(queue.disk_bytes(), 0);
        queue.quota = bytes;
        queue
            .write_chunk(&[0.25], 16_000, 0.25, Duration::ZERO)
            .unwrap();
        assert_eq!(queue.disk_bytes(), bytes);
    }
}
