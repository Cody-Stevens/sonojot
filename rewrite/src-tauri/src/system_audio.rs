//! System audio capture for meetings. Windows uses default-render-device WASAPI
//! loopback; macOS 14.2+ uses a Core Audio global process tap behind a private
//! aggregate device. Native objects stay on one dedicated owner thread, and
//! silence has a position in the recording's common timeline.

use std::sync::{mpsc, Arc};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SystemAudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Position of the first sample relative to the epoch supplied to start().
    pub captured_at: Duration,
    /// A packet loss or unreliable timestamp after the initial packet.
    pub discontinuity: bool,
}

/// Whether meetings can record this computer's audio right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioStatus {
    /// Capture can start. Windows loopback needs no permission.
    Ready,
    /// macOS has not asked yet. Requesting shows the system prompt.
    NotDetermined,
    /// macOS permission is off. Only System Settings can turn it on.
    Denied,
    /// This OS cannot capture system audio (macOS before 14.2, other platforms).
    #[default]
    Unsupported,
}

impl SystemAudioStatus {
    /// A permission problem that the user can fix while a meeting records.
    pub fn fixable(self) -> bool {
        matches!(self, Self::NotDetermined | Self::Denied)
    }
}

/// Stable prefixes let the recorder tell a permission refusal from a device fault.
pub const PERMISSION_ERROR: &str =
    "System audio permission is off. Allow SonoJot in System Settings, Privacy & Security, Screen & System Audio Recording.";
pub const UNSUPPORTED_ERROR: &str = "Recording system audio needs macOS 14.2 or later.";

pub fn is_permission_error(message: &str) -> bool {
    message.starts_with(PERMISSION_ERROR)
}

/// Truthful per OS version and, on macOS, the current privacy permission.
pub fn status() -> SystemAudioStatus {
    #[cfg(windows)]
    {
        SystemAudioStatus::Ready
    }
    #[cfg(target_os = "macos")]
    {
        native::status()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        SystemAudioStatus::Unsupported
    }
}

pub fn available() -> bool {
    status() == SystemAudioStatus::Ready
}

/// Shows the macOS prompt when it has not been answered. Returns the status
/// after the request was issued; the answer arrives asynchronously.
pub fn request_permission() -> SystemAudioStatus {
    #[cfg(target_os = "macos")]
    {
        native::request_permission()
    }
    #[cfg(not(target_os = "macos"))]
    {
        status()
    }
}

/// Privacy & Security, Screen & System Audio Recording, at the "System Audio
/// Recording Only" list (macOS 15+; older versions open Privacy & Security).
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AudioCapture";

/// `permissions::PermissionSpec::granted`. An OS without taps has nothing to grant.
pub fn permission_granted() -> bool {
    !status().fixable()
}

/// `permissions::PermissionSpec::request`: true when no Settings visit is
/// needed now, because access is allowed or macOS is showing its own prompt.
pub fn request_permission_prompted() -> bool {
    match request_permission() {
        SystemAudioStatus::NotDetermined => true,
        status => !status.fixable(),
    }
}

/// The source a meeting actually records. A permission problem keeps the
/// system side, so it joins the recording as soon as it is allowed; an OS
/// that cannot capture system audio drops it.
pub fn recording_source(saved: &str, status: SystemAudioStatus) -> Result<&'static str, String> {
    match (saved, status) {
        ("microphone", _) | ("both", SystemAudioStatus::Unsupported) => Ok("microphone"),
        ("both", _) => Ok("both"),
        ("system", SystemAudioStatus::Unsupported) => {
            Err(format!("{UNSUPPORTED_ERROR} Choose Microphone only in Meetings."))
        }
        ("system", _) => Ok("system"),
        _ => Err("Choose a meeting audio source.".into()),
    }
}

pub struct SystemAudioCapture {
    stop_tx: mpsc::SyncSender<()>,
    cancel: Arc<AtomicBool>,
    failure: Arc<FailureTime>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

// Written at the failing native operation, before Stop/Release/COM cleanup can
// block. The recorder reads the epoch-relative time without callback I/O.
struct FailureTime {
    at: AtomicU64,
    cause: AtomicU32,
}
impl Default for FailureTime {
    fn default() -> Self {
        Self { at: AtomicU64::new(u64::MAX), cause: AtomicU32::new(0) }
    }
}
impl FailureTime {
    fn record(&self, at: Duration) {
        self.record_cause(at, 1);
    }
    fn record_cause(&self, at: Duration, cause: u32) {
        // The native owner is the only writer, including its outer error handler.
        if self.at.load(Ordering::Relaxed) != u64::MAX { return; }
        let nanos = at.as_nanos().min((u64::MAX - 1) as u128) as u64;
        self.cause.store(cause, Ordering::Relaxed);
        self.at.store(nanos, Ordering::Release);
    }
    fn get(&self) -> Option<Duration> {
        let nanos = self.at.load(Ordering::Acquire);
        (nanos != u64::MAX).then(|| Duration::from_nanos(nanos))
    }
}

// The returned guard owns a started stream. If cancellation races Start itself,
// dropping that guard stops the stream before readiness or any audio callback.
fn start_unless_cancelled<T>(
    cancel: &AtomicBool,
    start: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("System audio startup was cancelled.".into());
    }
    let stream = start()?;
    if cancel.load(Ordering::Acquire) {
        drop(stream);
        return Err("System audio startup was cancelled.".into());
    }
    Ok(stream)
}

static OWNER_ACTIVE: AtomicBool = AtomicBool::new(false);
struct OwnerLease;
impl Drop for OwnerLease {
    fn drop(&mut self) { OWNER_ACTIVE.store(false, Ordering::Release); }
}

impl SystemAudioCapture {
    /// Callbacks run on the capture thread and must return promptly. Forward chunks
    /// to a bounded recorder queue, and explicitly report any dropped audio.
    pub fn start(
        epoch: Instant,
        on_chunk: impl FnMut(SystemAudioChunk) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_with_stop(epoch, Arc::new(AtomicBool::new(false)), on_chunk, on_error)
    }

    pub(crate) fn start_with_stop(
        epoch: Instant,
        cancel: Arc<AtomicBool>,
        on_chunk: impl FnMut(SystemAudioChunk) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_checked(epoch, cancel, true, on_chunk, on_error)
    }

    /// Diagnostics only: starts even when the permission check fails, to
    /// observe what macOS delivers (an unpermitted tap records silence).
    #[doc(hidden)]
    pub fn start_ignoring_permission(
        epoch: Instant,
        on_chunk: impl FnMut(SystemAudioChunk) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_checked(epoch, Arc::new(AtomicBool::new(false)), false, on_chunk, on_error)
    }

    fn start_checked(
        epoch: Instant,
        cancel: Arc<AtomicBool>,
        check_permission: bool,
        on_chunk: impl FnMut(SystemAudioChunk) + Send + 'static,
        on_error: impl FnMut(String) + Send + 'static,
    ) -> Result<Self, String> {
        match status() {
            SystemAudioStatus::Ready => {}
            SystemAudioStatus::NotDetermined | SystemAudioStatus::Denied if check_permission => {
                return Err(PERMISSION_ERROR.into());
            }
            SystemAudioStatus::NotDetermined | SystemAudioStatus::Denied => {}
            SystemAudioStatus::Unsupported => return Err(UNSUPPORTED_ERROR.into()),
        }
        if cancel.load(Ordering::Acquire) {
            return Err("System audio startup was cancelled.".into());
        }
        if OWNER_ACTIVE.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err("The previous system audio capture is still closing. Retrying shortly.".into());
        }
        let lease = OwnerLease;
        Self::start_owner(epoch, cancel, on_error, move |stop, ready, cancel, failure| {
            let _lease = lease;
            #[cfg(target_os = "macos")]
            {
                native::run(epoch, stop, ready, cancel, failure, check_permission, on_chunk)
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = check_permission;
                native::run(epoch, stop, ready, cancel, failure, on_chunk)
            }
        })
    }

    fn start_owner(
        epoch: Instant,
        cancel: Arc<AtomicBool>,
        mut on_error: impl FnMut(String) + Send + 'static,
        run: impl FnOnce(mpsc::Receiver<()>, &mpsc::SyncSender<Result<(), String>>,
            &AtomicBool, &FailureTime) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let owner_cancel = cancel.clone();
        let failure = Arc::new(FailureTime::default());
        let owner_failure = failure.clone();
        let worker = std::thread::Builder::new()
            .name("sonojot-loopback".into())
            .spawn(move || {
                let result = run(stop_rx, &ready_tx, &owner_cancel, &owner_failure);
                if let Err(error) = &result {
                    if !owner_cancel.load(Ordering::Acquire) {
                        owner_failure.record(epoch.elapsed());
                    }
                    // Sending twice is harmless: the startup receiver has been dropped.
                    let _ = ready_tx.try_send(Err(error.clone()));
                    on_error(error.clone());
                }
                result
            })
            .map_err(|e| format!("Cannot start system audio capture thread: {e}"))?;
        let capture = Self { stop_tx, cancel, failure, worker: Some(worker) };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if capture.cancel.load(Ordering::Acquire) {
                return Err("System audio startup was cancelled.".into());
            }
            match ready_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(Ok(())) => {
                    if capture.cancel.load(Ordering::Acquire) {
                        return Err("System audio startup was cancelled.".into());
                    }
                    return Ok(capture);
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(error) => return Err(format!(
                    "System audio did not start within five seconds: {error}"
                )),
            }
        }
    }

    pub(crate) fn failure_at(&self) -> Option<Duration> { self.failure.get() }
    pub(crate) fn failure_cause(&self) -> u32 {
        self.failure.get().map_or(0, |_| self.failure.cause.load(Ordering::Relaxed))
    }

    /// Wake the owner without waiting for a driver or callback to return.
    pub fn request_stop(&self) {
        self.cancel.store(true, Ordering::Release);
        let _ = self.stop_tx.try_send(());
    }

    /// Once true, stop() can reap the owner without waiting for native cleanup.
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().map_or(true, JoinHandle::is_finished)
    }

    /// Stop and reap with a bounded wait. A stuck owner is detached on timeout;
    /// its COM objects remain on that owner. Recorders should request_stop() on
    /// all sources first, keep draining, and reap after is_finished().
    pub fn stop(mut self) -> Result<(), String> {
        self.finish()
    }

    fn finish(&mut self) -> Result<(), String> {
        self.request_stop();
        let deadline = Instant::now() + Duration::from_millis(250);
        while !self.is_finished() {
            if Instant::now() >= deadline {
                return Err(
                    "System audio capture did not stop within 250 ms; its owner was detached."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| "System audio capture thread panicked.".to_string())?,
            None => Ok(()),
        }
    }
}

impl Drop for SystemAudioCapture {
    fn drop(&mut self) {
        self.request_stop();
        // Dropping a JoinHandle detaches it. Never join a native owner here: a
        // failed output driver must not stop the healthy microphone's recorder.
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug)]
enum Encoding {
    Float,
    Pcm,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug)]
struct Format {
    channels: usize,
    bits: u16,
    block_align: usize,
    sample_rate: u32,
    encoding: Encoding,
}

#[cfg(any(windows, test))]
impl Format {
    fn validate(self) -> Result<Self, String> {
        let sample_supported = match self.encoding {
            Encoding::Float => matches!(self.bits, 32 | 64),
            Encoding::Pcm => matches!(self.bits, 8 | 16 | 24 | 32),
        };
        if !sample_supported
            || self.channels == 0
            || self.channels > 64
            || !(8000..=384000).contains(&self.sample_rate)
            || self.block_align != self.channels * self.bits as usize / 8
        {
            return Err("The default output device uses an unsupported audio format.".into());
        }
        Ok(self)
    }

    fn mono(self, bytes: &[u8], frames: usize, silent: bool) -> Result<Vec<f32>, String> {
        if silent {
            return Ok(vec![0.0; frames]);
        }
        let needed = frames
            .checked_mul(self.block_align)
            .ok_or("Audio packet size overflow.")?;
        if bytes.len() < needed {
            return Err("System audio packet was shorter than its frame count.".into());
        }
        let sample_bytes = self.bits as usize / 8;
        let mut mono = Vec::with_capacity(frames);
        for frame in bytes[..needed].chunks_exact(self.block_align) {
            let mut sum = 0.0f64;
            for sample in frame.chunks_exact(sample_bytes) {
                let value = match (self.encoding, self.bits) {
                    (Encoding::Float, 32) => f32::from_le_bytes(sample.try_into().unwrap()) as f64,
                    (Encoding::Float, 64) => f64::from_le_bytes(sample.try_into().unwrap()),
                    (Encoding::Pcm, 8) => (sample[0] as f64 - 128.0) / 128.0,
                    (Encoding::Pcm, 16) => {
                        i16::from_le_bytes(sample.try_into().unwrap()) as f64 / 32768.0
                    }
                    (Encoding::Pcm, 24) => {
                        ((i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8) as f64)
                            / 8388608.0
                    }
                    (Encoding::Pcm, 32) => {
                        i32::from_le_bytes(sample.try_into().unwrap()) as f64 / 2147483648.0
                    }
                    _ => return Err("Unsupported system audio sample format.".into()),
                };
                sum += if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
            mono.push((sum / self.channels as f64) as f32);
        }
        Ok(mono)
    }
}

/// Map an OS monotonic timestamp into the same timeline used by microphone capture.
#[cfg(any(windows, test))]
fn timestamp_from_anchor(packet_hns: u64, anchor_hns: u64, anchor_elapsed: Duration) -> Duration {
    if packet_hns >= anchor_hns {
        anchor_elapsed.saturating_add(Duration::from_nanos(
            packet_hns.saturating_sub(anchor_hns).saturating_mul(100),
        ))
    } else {
        anchor_elapsed.saturating_sub(Duration::from_nanos(
            anchor_hns.saturating_sub(packet_hns).saturating_mul(100),
        ))
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::Media::KernelStreaming::{
        KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE,
    };
    use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    use windows::Win32::System::Com::*;
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    use windows_sys::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
    };

    struct ProAudio(windows_sys::Win32::Foundation::HANDLE);
    impl ProAudio {
        fn enter() -> Self {
            let mut task_index = 0;
            // This owner thread serves meeting loopback only. Registration is
            // best effort: an unavailable MMCSS service must not lose audio.
            Self(unsafe { AvSetMmThreadCharacteristicsW(windows_sys::w!("Pro Audio"), &mut task_index) })
        }
    }
    impl Drop for ProAudio {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // MMCSS requires reversion on the same thread that registered.
                unsafe { AvRevertMmThreadCharacteristics(self.0); }
            }
        }
    }

    struct ComApartment;
    impl ComApartment {
        fn enter() -> Result<Self, String> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
                .map_err(|e| format!("Cannot initialize audio COM thread: {e}"))?;
            Ok(Self)
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct MixFormat(*mut WAVEFORMATEX);
    impl Drop for MixFormat {
        fn drop(&mut self) {
            unsafe {
                CoTaskMemFree(Some(self.0.cast()));
            }
        }
    }

    struct EndpointId(windows::core::PWSTR);
    impl Drop for EndpointId {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
        }
    }

    fn endpoint_id(device: &IMMDevice) -> Result<Vec<u16>, String> {
        let id = EndpointId(
            unsafe { device.GetId() }
                .map_err(|_| "Cannot identify the default system audio output.".to_string())?,
        );
        if id.0.is_null() {
            return Err("The default system audio output returned no endpoint ID.".into());
        }
        // Endpoint IDs are opaque and remain in memory. Never log an ID or name.
        Ok(unsafe { id.0.as_wide() }.to_vec())
    }

    fn check_default_output(
        enumerator: &IMMDeviceEnumerator,
        initial_id: &[u16],
        epoch: Instant,
        failure: &FailureTime,
    ) -> Result<(), String> {
        let current_id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }
            .map_err(|_| "The default system audio output is unavailable.".to_string())
            .and_then(|device| endpoint_id(&device));
        match current_id {
            Ok(id) if id == initial_id => Ok(()),
            Ok(_) => {
                failure.record_cause(epoch.elapsed(), 2);
                Err("The default system audio output changed; restart system capture.".into())
            }
            Err(error) => {
                failure.record_cause(epoch.elapsed(), 3);
                Err(error)
            }
        }
    }

    struct Stream<'a>(&'a IAudioClient);
    impl Drop for Stream<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.0.Stop() };
        }
    }

    struct Packet<'a> {
        capture: &'a IAudioCaptureClient,
        frames: u32,
    }
    impl Drop for Packet<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.capture.ReleaseBuffer(self.frames) };
        }
    }

    fn format_from_native(native: *const WAVEFORMATEX) -> Result<Format, String> {
        if native.is_null() {
            return Err("The output device returned no audio format.".into());
        }
        let wave = unsafe { *native };
        let encoding = if wave.wFormatTag == WAVE_FORMAT_EXTENSIBLE as u16 && wave.cbSize >= 22 {
            let extended = unsafe { *native.cast::<WAVEFORMATEXTENSIBLE>() };
            let subformat = extended.SubFormat;
            if subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                Encoding::Float
            } else if subformat == KSDATAFORMAT_SUBTYPE_PCM {
                Encoding::Pcm
            } else {
                return Err("The output device uses an unsupported extended audio format.".into());
            }
        } else {
            match wave.wFormatTag {
                1 => Encoding::Pcm,
                3 => Encoding::Float,
                _ => {
                    return Err("The output device is neither PCM nor floating-point audio.".into())
                }
            }
        };
        Format {
            channels: wave.nChannels as usize,
            bits: wave.wBitsPerSample,
            block_align: wave.nBlockAlign as usize,
            sample_rate: wave.nSamplesPerSec,
            encoding,
        }
        .validate()
    }

    fn qpc_anchor(epoch: Instant) -> Result<(u64, Duration), String> {
        let mut frequency = 0;
        let mut counter = 0;
        if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
            return Err("Windows monotonic audio clock is unavailable.".into());
        }
        let before = epoch.elapsed();
        if unsafe { QueryPerformanceCounter(&mut counter) } == 0 || counter < 0 {
            return Err("Cannot read the Windows monotonic audio clock.".into());
        }
        let after = epoch.elapsed();
        let midpoint = before + after.saturating_sub(before) / 2;
        let hns = (counter as u128 * 10_000_000 / frequency as u128) as u64;
        Ok((hns, midpoint))
    }

    pub(super) fn run(
        epoch: Instant,
        stop: mpsc::Receiver<()>,
        ready: &mpsc::SyncSender<Result<(), String>>,
        cancel: &AtomicBool,
        failure: &FailureTime,
        mut on_chunk: impl FnMut(SystemAudioChunk),
    ) -> Result<(), String> {
        let fail = |message: String| {
            failure.record(epoch.elapsed());
            message
        };
        if cancel.load(Ordering::Acquire) {
            return Err("System audio startup was cancelled.".into());
        }
        let _apartment = ComApartment::enter().map_err(fail)?;
        let _priority = ProAudio::enter();
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|e| fail(format!("Cannot enumerate Windows audio devices: {e}")))?;
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }
            .map_err(|e| fail(format!("No default system audio output is available: {e}")))?;
        let initial_id = endpoint_id(&device).map_err(fail)?;
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|e| fail(format!("Cannot open the default system audio output: {e}")))?;
        let wave = MixFormat(
            unsafe { client.GetMixFormat() }
                .map_err(|e| fail(format!("Cannot read the system audio format: {e}")))?,
        );
        let format = format_from_native(wave.0).map_err(fail)?;
        // Poll a shared loopback stream every 5 ms. A quiet output may produce no
        // events/packets; that is normal, and stop is still immediately wakeable.
        unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                5_000_000,
                0,
                wave.0,
                None,
            )
        }
        .map_err(|e| fail(format!("Cannot initialize WASAPI system audio capture: {e}")))?;
        let capture: IAudioCaptureClient = unsafe { client.GetService() }
            .map_err(|e| fail(format!("Cannot open WASAPI loopback packets: {e}")))?;
        let (anchor_hns, anchor_elapsed) = qpc_anchor(epoch).map_err(fail)?;
        if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            cancel.store(true, Ordering::Release);
        }
        let _stream = start_unless_cancelled(cancel, || {
            unsafe { client.Start() }
                .map_err(|e| fail(format!("Cannot start WASAPI loopback capture: {e}")))?;
            Ok(Stream(&client))
        })?;
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let mut first_packet = true;
        let mut previous_end = Duration::ZERO;
        // Loopback stays bound to the endpoint opened above. Check the default
        // even when silence produces no packets, so the recorder can restart it.
        let mut last_output_check = Instant::now();
        loop {
            let stopping = cancel.load(Ordering::Acquire) || match stop.recv_timeout(Duration::from_millis(5)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
            };
            // Drain packets already produced on stop; never capture a new tail.
            if stopping {
                unsafe { client.Stop() }.map_err(|e| fail(format!("Cannot stop WASAPI capture: {e}")))?;
            } else if last_output_check.elapsed() >= Duration::from_millis(250) {
                check_default_output(&enumerator, &initial_id, epoch, failure).map_err(fail)?;
                last_output_check = Instant::now();
            }
            for _ in 0..128 {
                let next = unsafe { capture.GetNextPacketSize() }
                    .map_err(|e| fail(format!("System audio device stopped delivering packets: {e}")))?;
                if next == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                let mut position = 0;
                let mut timestamp = 0;
                unsafe {
                    capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut position),
                        Some(&mut timestamp),
                    )
                }
                .map_err(|e| fail(format!("Cannot read a system audio packet: {e}")))?;
                let packet = Packet {
                    capture: &capture,
                    frames,
                };
                if frames == 0 {
                    drop(packet);
                    break;
                }
                if frames > format.sample_rate * 2 {
                    return Err(fail("The system audio device returned an oversized packet.".into()));
                }
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                // SILENT packets do not require a valid data pointer. Do not even
                // construct a slice from it until this flag has been inspected.
                let bytes = if silent {
                    &[][..]
                } else {
                    if data.is_null() {
                        return Err(fail(
                            "The system audio device returned a null nonsilent packet.".into()
                        ));
                    }
                    unsafe {
                        std::slice::from_raw_parts(data, frames as usize * format.block_align)
                    }
                };
                let samples = format.mono(bytes, frames as usize, silent).map_err(fail)?;
                drop(packet); // release the driver buffer before invoking recorder code
                let timestamp_error =
                    flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 || timestamp == 0;
                let packet_duration =
                    Duration::from_secs_f64(frames as f64 / format.sample_rate as f64);
                let captured_at = if timestamp_error {
                    epoch
                        .elapsed()
                        .saturating_sub(packet_duration)
                        .max(previous_end)
                } else {
                    timestamp_from_anchor(timestamp, anchor_hns, anchor_elapsed)
                };
                let discontinuity = (!first_packet
                    && flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0)
                    || timestamp_error;
                previous_end = captured_at.saturating_add(packet_duration);
                first_packet = false;
                on_chunk(SystemAudioChunk {
                    samples,
                    sample_rate: format.sample_rate,
                    captured_at,
                    discontinuity,
                });
            }
            if stopping {
                return Ok(());
            }
        }
    }

    #[cfg(test)]
    mod hardware_tests {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[test]
        #[ignore = "Opt-in hardware test renders digital silence for 300 ms and counts loopback packets; no file or audible test tone."]
        fn windows_loopback_receives_render_packets() {
            let _apartment = ComApartment::enter().unwrap();
            let enumerator: IMMDeviceEnumerator =
                unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.unwrap();
            let device =
                unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.unwrap();
            let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.unwrap();
            let wave = MixFormat(unsafe { client.GetMixFormat() }.unwrap());
            unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 1_000_000, 0, wave.0, None) }
                .unwrap();
            let render: IAudioRenderClient = unsafe { client.GetService() }.unwrap();
            let buffer_frames = unsafe { client.GetBufferSize() }.unwrap();
            let count = Arc::new(AtomicUsize::new(0));
            let received = count.clone();
            let (errors, error_rx) = mpsc::channel();
            let capture = SystemAudioCapture::start(
                Instant::now(),
                move |chunk| {
                    assert!(chunk.samples.iter().all(|v| v.is_finite()));
                    received.fetch_add(chunk.samples.len(), Ordering::Relaxed);
                },
                move |error| {
                    let _ = errors.send(error);
                },
            )
            .unwrap();
            unsafe {
                render.GetBuffer(buffer_frames).unwrap();
                render
                    .ReleaseBuffer(buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                    .unwrap();
                client.Start().unwrap();
            }
            let _stream = Stream(&client);
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
                let free = buffer_frames - unsafe { client.GetCurrentPadding() }.unwrap();
                if free > 0 {
                    unsafe {
                        render.GetBuffer(free).unwrap();
                        render
                            .ReleaseBuffer(free, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                            .unwrap();
                    }
                }
            }
            capture.stop().unwrap();
            assert!(error_rx.try_recv().is_err());
            let frames = count.load(Ordering::Relaxed);
            assert!(
                frames > 0,
                "No loopback packets arrived during the render stream."
            );
            eprintln!(
                "Loopback received {frames} mono frames while the test rendered digital silence."
            );
        }
    }
}

/// Average every channel of every buffer into one mono timeline. Taps deliver
/// either one interleaved buffer or one buffer per channel; both reduce here.
#[cfg(any(target_os = "macos", test))]
fn downmix(buffers: &[(&[f32], usize)]) -> Vec<f32> {
    let frames = buffers
        .iter()
        .filter(|(_, channels)| *channels > 0)
        .map(|(data, channels)| data.len() / channels)
        .min()
        .unwrap_or(0);
    let channels: usize = buffers.iter().map(|(_, channels)| channels).sum();
    if frames == 0 || channels == 0 {
        return Vec::new();
    }
    let mut mono = vec![0.0f32; frames];
    for (data, width) in buffers.iter().filter(|(_, channels)| *channels > 0) {
        for (sum, frame) in mono.iter_mut().zip(data.chunks_exact(*width)) {
            for value in frame {
                if value.is_finite() {
                    *sum += value.clamp(-1.0, 1.0);
                }
            }
        }
    }
    let scale = 1.0 / channels as f32;
    mono.iter_mut().for_each(|value| *value *= scale);
    mono
}

/// "14.2.1" -> taps exist. macOS 26+ numbering stays above 14.
#[cfg(any(target_os = "macos", test))]
fn version_supports_taps(version: &str) -> bool {
    let mut parts = version.trim().split('.').map(|part| part.parse::<u32>().ok());
    match (parts.next().flatten(), parts.next().flatten().unwrap_or(0)) {
        (Some(major), minor) => major > 14 || (major == 14 && minor >= 2),
        _ => false,
    }
}

/// TCCAccessPreflight: 0 granted, 1 denied, anything else not yet asked.
#[cfg(any(target_os = "macos", test))]
fn permission_from_preflight(result: i32) -> SystemAudioStatus {
    match result {
        0 => SystemAudioStatus::Ready,
        1 => SystemAudioStatus::Denied,
        _ => SystemAudioStatus::NotDetermined,
    }
}

#[cfg(target_os = "macos")]
mod native {
    //! Core Audio process tap: a global stereo tap of every process's output,
    //! wrapped in a private tap-only aggregate device whose input IOProc
    //! delivers the mix. It needs only the "System Audio Recording" privacy
    //! permission (NSAudioCaptureUsageDescription), not Screen Recording.
    use super::*;
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use objc2::msg_send;
    use objc2::rc::{autoreleasepool, Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::sync::OnceLock;

    type ObjectId = u32;
    type OsStatus = i32;

    #[repr(C)]
    struct PropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default, Debug)]
    #[allow(dead_code)] // Core Audio layout; only some fields are read.
    struct StreamFormat {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct SmpteTime {
        subframes: i16,
        subframe_divisor: i16,
        counter: u32,
        kind: u32,
        flags: u32,
        hours: i16,
        minutes: i16,
        seconds: i16,
        frames: i16,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct TimeStamp {
        sample_time: f64,
        host_time: u64,
        rate_scalar: f64,
        word_clock_time: u64,
        smpte_time: SmpteTime,
        flags: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct Buffer {
        channels: u32,
        byte_size: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct BufferList {
        count: u32,
        buffers: [Buffer; 1],
    }

    type IoProc = unsafe extern "C" fn(
        ObjectId,
        *const TimeStamp,
        *const BufferList,
        *const TimeStamp,
        *mut BufferList,
        *const TimeStamp,
        *mut c_void,
    ) -> OsStatus;
    type IoProcId = Option<IoProc>;

    const fn code(value: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*value)
    }
    const SCOPE_GLOBAL: u32 = code(b"glob");
    const TAP_FORMAT: u32 = code(b"tfmt");
    const LINEAR_PCM: u32 = code(b"lpcm");
    const FORMAT_IS_FLOAT: u32 = 1;
    const SAMPLE_TIME_VALID: u32 = 1;
    const HOST_TIME_VALID: u32 = 2;
    const TCC_SERVICE: &str = "kTCCServiceAudioCapture";

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioHardwareCreateAggregateDevice(
            description: CFDictionaryRef,
            device: *mut ObjectId,
        ) -> OsStatus;
        fn AudioHardwareDestroyAggregateDevice(device: ObjectId) -> OsStatus;
        fn AudioObjectGetPropertyData(
            object: ObjectId,
            address: *const PropertyAddress,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> OsStatus;
        fn AudioDeviceCreateIOProcID(
            device: ObjectId,
            procedure: IoProc,
            client: *mut c_void,
            id: *mut IoProcId,
        ) -> OsStatus;
        fn AudioDeviceDestroyIOProcID(device: ObjectId, id: IoProcId) -> OsStatus;
        fn AudioDeviceStart(device: ObjectId, id: IoProcId) -> OsStatus;
        fn AudioDeviceStop(device: ObjectId, id: IoProcId) -> OsStatus;
    }
    // NSArray, NSUUID and NSString back the tap description.
    #[link(name = "Foundation", kind = "framework")]
    extern "C" {}

    #[repr(C)]
    #[derive(Default)]
    struct TimebaseInfo {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut TimebaseInfo) -> c_int;
    }

    type CreateTap = unsafe extern "C" fn(*mut AnyObject, *mut ObjectId) -> OsStatus;
    type DestroyTap = unsafe extern "C" fn(ObjectId) -> OsStatus;
    struct TapApi {
        create: CreateTap,
        destroy: DestroyTap,
    }

    fn os_version() -> Option<String> {
        let mut buffer = [0u8; 64];
        let mut size = buffer.len();
        let result = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return None;
        }
        CStr::from_bytes_until_nul(&buffer[..size.min(buffer.len())])
            .ok()
            .and_then(|text| text.to_str().ok())
            .map(str::to_owned)
    }

    /// The minimum app version is macOS 14.0; taps arrived in 14.2. Bind them
    /// at run time so the app still launches on 14.0 and 14.1.
    fn tap_api() -> Option<&'static TapApi> {
        static API: OnceLock<Option<TapApi>> = OnceLock::new();
        API.get_or_init(|| {
            if !os_version().is_some_and(|version| version_supports_taps(&version)) {
                return None;
            }
            let create = unsafe {
                libc::dlsym(libc::RTLD_DEFAULT, c"AudioHardwareCreateProcessTap".as_ptr())
            };
            let destroy = unsafe {
                libc::dlsym(libc::RTLD_DEFAULT, c"AudioHardwareDestroyProcessTap".as_ptr())
            };
            if create.is_null() || destroy.is_null() || AnyClass::get(c"CATapDescription").is_none() {
                return None;
            }
            Some(TapApi {
                create: unsafe { std::mem::transmute::<*mut c_void, CreateTap>(create) },
                destroy: unsafe { std::mem::transmute::<*mut c_void, DestroyTap>(destroy) },
            })
        })
        .as_ref()
    }

    // TCC has no public API for this permission. The private TCC framework
    // calls are the documented-by-practice route used by capture tools; when
    // they are missing, report Ready and let the first tap ask instead.
    type Preflight = unsafe extern "C" fn(CFStringRef, CFDictionaryRef) -> c_int;
    type Request =
        unsafe extern "C" fn(CFStringRef, CFDictionaryRef, *const block2::Block<dyn Fn(Bool)>);
    struct Tcc {
        preflight: Preflight,
        request: Request,
    }
    fn tcc() -> Option<&'static Tcc> {
        static TCC: OnceLock<Option<Tcc>> = OnceLock::new();
        TCC.get_or_init(|| {
            let handle = unsafe {
                libc::dlopen(
                    c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(),
                    libc::RTLD_NOW,
                )
            };
            if handle.is_null() {
                return None;
            }
            let preflight = unsafe { libc::dlsym(handle, c"TCCAccessPreflight".as_ptr()) };
            let request = unsafe { libc::dlsym(handle, c"TCCAccessRequest".as_ptr()) };
            if preflight.is_null() || request.is_null() {
                return None;
            }
            Some(Tcc {
                preflight: unsafe { std::mem::transmute::<*mut c_void, Preflight>(preflight) },
                request: unsafe { std::mem::transmute::<*mut c_void, Request>(request) },
            })
        })
        .as_ref()
    }

    fn permission() -> SystemAudioStatus {
        let Some(tcc) = tcc() else { return SystemAudioStatus::Ready };
        let service = CFString::new(TCC_SERVICE);
        permission_from_preflight(unsafe {
            (tcc.preflight)(service.as_concrete_TypeRef(), std::ptr::null())
        })
    }

    pub(super) fn status() -> SystemAudioStatus {
        if tap_api().is_none() {
            return SystemAudioStatus::Unsupported;
        }
        permission()
    }

    pub(super) fn request_permission() -> SystemAudioStatus {
        let current = status();
        if current == SystemAudioStatus::NotDetermined {
            if let Some(tcc) = tcc() {
                let service = CFString::new(TCC_SERVICE);
                // TCC copies the completion block; the UI polls the result.
                let done = block2::RcBlock::new(|_granted: Bool| {});
                unsafe {
                    (tcc.request)(service.as_concrete_TypeRef(), std::ptr::null(), &*done);
                }
            }
        }
        current
    }

    fn status_text(status: OsStatus) -> String {
        let bytes = status.to_be_bytes();
        if bytes.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' ') {
            format!("'{}'", String::from_utf8_lossy(&bytes))
        } else {
            status.to_string()
        }
    }

    struct Tap {
        id: ObjectId,
        uid: String,
        destroy: DestroyTap,
    }
    impl Drop for Tap {
        fn drop(&mut self) {
            unsafe { (self.destroy)(self.id) };
        }
    }

    fn create_tap(api: &TapApi) -> Result<Tap, String> {
        autoreleasepool(|_| unsafe {
            let class = AnyClass::get(c"CATapDescription").ok_or(UNSUPPORTED_ERROR)?;
            let array = AnyClass::get(c"NSArray").ok_or("Foundation is unavailable.")?;
            // Exclude no process: SonoJot itself plays nothing during a meeting.
            let none: Retained<AnyObject> = msg_send![array, array];
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            let description: Option<Retained<AnyObject>> =
                msg_send![allocated, initStereoGlobalTapButExcludeProcesses: &*none];
            let description = description.ok_or("Cannot describe the system audio tap.")?;
            // Private: the tap and its aggregate are invisible to other apps.
            let _: () = msg_send![&*description, setPrivate: Bool::YES];
            let uuid: Option<Retained<AnyObject>> = msg_send![&*description, UUID];
            let uuid = uuid.ok_or("The system audio tap has no identifier.")?;
            let text: Option<Retained<AnyObject>> = msg_send![&*uuid, UUIDString];
            let text = text.ok_or("The system audio tap has no identifier.")?;
            let utf8: *const c_char = msg_send![&*text, UTF8String];
            if utf8.is_null() {
                return Err("The system audio tap has no identifier.".into());
            }
            let uid = CStr::from_ptr(utf8).to_string_lossy().into_owned();
            let mut id = 0;
            let status = (api.create)(Retained::as_ptr(&description) as *mut AnyObject, &mut id);
            if status != 0 || id == 0 {
                return Err(format!(
                    "Cannot create the system audio tap (Core Audio {}).",
                    status_text(status)
                ));
            }
            Ok(Tap { id, uid, destroy: api.destroy })
        })
    }

    fn tap_format(tap: ObjectId) -> Result<StreamFormat, String> {
        let address = PropertyAddress { selector: TAP_FORMAT, scope: SCOPE_GLOBAL, element: 0 };
        let mut format = StreamFormat::default();
        let mut size = std::mem::size_of::<StreamFormat>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                tap,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&mut format as *mut StreamFormat).cast(),
            )
        };
        if status != 0 {
            return Err(format!("Cannot read the system audio format (Core Audio {}).", status_text(status)));
        }
        if format.format_id != LINEAR_PCM
            || format.format_flags & FORMAT_IS_FLOAT == 0
            || format.bits_per_channel != 32
            || !(8000.0..=384000.0).contains(&format.sample_rate)
            || format.channels_per_frame == 0
        {
            return Err("System audio uses an unsupported audio format.".into());
        }
        Ok(format)
    }

    struct Aggregate(ObjectId);
    impl Drop for Aggregate {
        fn drop(&mut self) {
            unsafe { AudioHardwareDestroyAggregateDevice(self.0) };
        }
    }

    fn create_aggregate(tap_uid: &str) -> Result<Aggregate, String> {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let uid = format!(
            "studio.codystevens.sonojot.meeting-audio.{}.{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let key = |name: &str| CFString::new(name).as_CFType();
        let text = |value: &str| CFString::new(value).as_CFType();
        let tap = CFDictionary::from_CFType_pairs(&[
            (key("uid"), text(tap_uid)),
            (key("drift"), CFBoolean::true_value().as_CFType()),
        ]);
        let taps = CFArray::from_CFTypes(&[tap.as_CFType()]);
        let none: CFArray<CFType> = CFArray::from_CFTypes(&[]);
        // Tap-only: no output sub-device, so changing speakers or headphones
        // does not interrupt the tap, and no headset microphone is mixed in.
        let description = CFDictionary::from_CFType_pairs(&[
            (key("name"), text("SonoJot meeting audio")),
            (key("uid"), text(&uid)),
            (key("private"), CFBoolean::true_value().as_CFType()),
            (key("stacked"), CFBoolean::false_value().as_CFType()),
            (key("tapautostart"), CFBoolean::true_value().as_CFType()),
            (key("subdevices"), none.as_CFType()),
            (key("taps"), taps.as_CFType()),
        ]);
        let mut id = 0;
        let status =
            unsafe { AudioHardwareCreateAggregateDevice(description.as_concrete_TypeRef(), &mut id) };
        if status != 0 || id == 0 {
            return Err(format!(
                "Cannot open the system audio tap device (Core Audio {}).",
                status_text(status)
            ));
        }
        Ok(Aggregate(id))
    }

    struct Packet {
        samples: Vec<f32>,
        captured_at: Duration,
        discontinuity: bool,
    }

    /// Shared with the real-time IOProc. Everything is lock-free except the
    /// bounded try_send; the owner thread runs recorder code.
    struct Context {
        tx: mpsc::SyncSender<Packet>,
        clock: HostClock,
        frames_per_second: f64,
        last_callback: AtomicU64,
        next_sample_time: AtomicU64,
        dropped: AtomicBool,
    }

    #[derive(Clone, Copy)]
    struct HostClock {
        anchor_ticks: u64,
        anchor_elapsed: Duration,
        numer: u64,
        denom: u64,
    }
    impl HostClock {
        fn new(epoch: Instant) -> Result<Self, String> {
            let mut info = TimebaseInfo::default();
            if unsafe { mach_timebase_info(&mut info) } != 0 || info.denom == 0 {
                return Err("The macOS audio clock is unavailable.".into());
            }
            let before = epoch.elapsed();
            let ticks = unsafe { mach_absolute_time() };
            let after = epoch.elapsed();
            Ok(Self {
                anchor_ticks: ticks,
                anchor_elapsed: before + after.saturating_sub(before) / 2,
                numer: info.numer as u64,
                denom: info.denom as u64,
            })
        }
        fn nanos(&self, ticks: u64) -> u64 {
            (ticks as u128 * self.numer as u128 / self.denom as u128).min(u64::MAX as u128) as u64
        }
        /// Same mapping as WASAPI QPC timestamps: host time into the epoch timeline.
        fn elapsed(&self, ticks: u64) -> Duration {
            if ticks >= self.anchor_ticks {
                self.anchor_elapsed
                    .saturating_add(Duration::from_nanos(self.nanos(ticks - self.anchor_ticks)))
            } else {
                self.anchor_elapsed
                    .saturating_sub(Duration::from_nanos(self.nanos(self.anchor_ticks - ticks)))
            }
        }
    }

    unsafe extern "C" fn io_proc(
        _device: ObjectId,
        _now: *const TimeStamp,
        input: *const BufferList,
        input_time: *const TimeStamp,
        _output: *mut BufferList,
        _output_time: *const TimeStamp,
        client: *mut c_void,
    ) -> OsStatus {
        if client.is_null() {
            return 0;
        }
        let context = &*(client as *const Context);
        let now = mach_absolute_time();
        context.last_callback.store(now.max(1), Ordering::Release);
        if input.is_null() {
            return 0;
        }
        let list = &*input;
        let buffers = std::slice::from_raw_parts(list.buffers.as_ptr(), list.count as usize);
        let mut views: [(&[f32], usize); 8] = [(&[] as &[f32], 0usize); 8];
        let mut used = 0;
        for buffer in buffers.iter().take(views.len()) {
            if buffer.data.is_null() || buffer.channels == 0 {
                continue;
            }
            let values = buffer.byte_size as usize / std::mem::size_of::<f32>();
            views[used] = (
                std::slice::from_raw_parts(buffer.data as *const f32, values),
                buffer.channels as usize,
            );
            used += 1;
        }
        let samples = downmix(&views[..used]);
        if samples.is_empty() {
            return 0;
        }
        let frames = samples.len();
        let duration = Duration::from_secs_f64(frames as f64 / context.frames_per_second);
        let mut discontinuity = context.dropped.swap(false, Ordering::AcqRel);
        let captured_at = if !input_time.is_null() && (*input_time).flags & HOST_TIME_VALID != 0 {
            context.clock.elapsed((*input_time).host_time)
        } else {
            discontinuity = true;
            context.clock.elapsed(now).saturating_sub(duration)
        };
        if !input_time.is_null() && (*input_time).flags & SAMPLE_TIME_VALID != 0 {
            let sample_time = (*input_time).sample_time;
            let expected = f64::from_bits(context.next_sample_time.load(Ordering::Relaxed));
            if expected.is_finite() && (sample_time - expected).abs() > 1.0 {
                discontinuity = true;
            }
            context
                .next_sample_time
                .store((sample_time + frames as f64).to_bits(), Ordering::Relaxed);
        }
        if let Err(mpsc::TrySendError::Full(_)) =
            context.tx.try_send(Packet { samples, captured_at, discontinuity })
        {
            context.dropped.store(true, Ordering::Release);
        }
        0
    }

    /// Owns the client context until Core Audio has released the IOProc.
    struct Procedure {
        device: ObjectId,
        id: IoProcId,
        context: *mut Context,
    }
    impl Procedure {
        fn create(device: ObjectId, context: Context) -> Result<Self, String> {
            let context = Box::into_raw(Box::new(context));
            let mut id: IoProcId = None;
            let status =
                unsafe { AudioDeviceCreateIOProcID(device, io_proc, context.cast(), &mut id) };
            if status != 0 || id.is_none() {
                drop(unsafe { Box::from_raw(context) });
                return Err(format!(
                    "Cannot receive system audio (Core Audio {}).",
                    status_text(status)
                ));
            }
            Ok(Self { device, id, context })
        }
        fn context(&self) -> &Context {
            unsafe { &*self.context }
        }
    }
    impl Drop for Procedure {
        fn drop(&mut self) {
            unsafe {
                AudioDeviceDestroyIOProcID(self.device, self.id);
                drop(Box::from_raw(self.context));
            }
        }
    }

    /// AudioDeviceStop waits for an in-flight IOProc before returning.
    struct Running<'a>(&'a Procedure);
    impl Drop for Running<'_> {
        fn drop(&mut self) {
            unsafe { AudioDeviceStop(self.0.device, self.0.id) };
        }
    }

    pub(super) fn run(
        epoch: Instant,
        stop: mpsc::Receiver<()>,
        ready: &mpsc::SyncSender<Result<(), String>>,
        cancel: &AtomicBool,
        failure: &FailureTime,
        check_permission: bool,
        mut on_chunk: impl FnMut(SystemAudioChunk),
    ) -> Result<(), String> {
        let fail = |message: String| {
            failure.record(epoch.elapsed());
            message
        };
        if cancel.load(Ordering::Acquire) {
            return Err("System audio startup was cancelled.".into());
        }
        let api = tap_api().ok_or_else(|| fail(UNSUPPORTED_ERROR.into()))?;
        let tap = create_tap(api).map_err(fail)?;
        let format = tap_format(tap.id).map_err(fail)?;
        let aggregate = create_aggregate(&tap.uid).map_err(fail)?;
        let (tx, rx) = mpsc::sync_channel(512);
        let sample_rate = format.sample_rate.round() as u32;
        let procedure = Procedure::create(
            aggregate.0,
            Context {
                tx,
                clock: HostClock::new(epoch).map_err(fail)?,
                frames_per_second: format.sample_rate,
                last_callback: AtomicU64::new(0),
                next_sample_time: AtomicU64::new(f64::NAN.to_bits()),
                dropped: AtomicBool::new(false),
            },
        )
        .map_err(fail)?;
        if !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty)) {
            cancel.store(true, Ordering::Release);
        }
        let stream = start_unless_cancelled(cancel, || {
            let status = unsafe { AudioDeviceStart(procedure.device, procedure.id) };
            if status != 0 {
                return Err(fail(format!(
                    "Cannot start system audio capture (Core Audio {}).",
                    status_text(status)
                )));
            }
            Ok(Running(&procedure))
        })?;
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let started = Instant::now();
        let mut permission_checked = Instant::now();
        let mut deliver = |packet: Packet| {
            on_chunk(SystemAudioChunk {
                samples: packet.samples,
                sample_rate,
                captured_at: packet.captured_at,
                discontinuity: packet.discontinuity,
            })
        };
        loop {
            let stopping = cancel.load(Ordering::Acquire)
                || match stop.recv_timeout(Duration::from_millis(5)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                };
            if stopping {
                // Stop first, then deliver what was already captured; no new tail.
                drop(stream);
                while let Ok(packet) = rx.try_recv() {
                    deliver(packet);
                }
                return Ok(());
            }
            for _ in 0..256 {
                let Ok(packet) = rx.try_recv() else { break };
                deliver(packet);
            }
            // A tap's IOProc runs continuously, with zeros when nothing plays.
            // No callbacks at all means the device died: report it promptly.
            let last = procedure.context().last_callback.load(Ordering::Acquire);
            let stalled = if last == 0 {
                started.elapsed() >= Duration::from_secs(3)
            } else {
                let now = unsafe { mach_absolute_time() };
                let clock = procedure.context().clock;
                clock.elapsed(now).saturating_sub(clock.elapsed(last)) >= Duration::from_secs(2)
            };
            if stalled {
                failure.record_cause(epoch.elapsed(), 4);
                return Err("System audio stopped delivering audio.".into());
            }
            if check_permission && permission_checked.elapsed() >= Duration::from_secs(1) {
                permission_checked = Instant::now();
                // Turning the permission off makes the tap record silence.
                if permission() == SystemAudioStatus::Denied {
                    failure.record_cause(epoch.elapsed(), 5);
                    return Err(PERMISSION_ERROR.into());
                }
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod native {
    use super::*;
    pub(super) fn run(
        _: Instant,
        _: mpsc::Receiver<()>,
        _: &mpsc::SyncSender<Result<(), String>>,
        _: &AtomicBool,
        _: &FailureTime,
        _: impl FnMut(SystemAudioChunk),
    ) -> Result<(), String> {
        Err("System audio capture is not implemented on this platform.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(bits: u16, channels: usize, encoding: Encoding) -> Format {
        Format {
            bits,
            channels,
            encoding,
            block_align: channels * bits as usize / 8,
            sample_rate: 48000,
        }
        .validate()
        .unwrap()
    }

    #[test]
    fn stereo_float_downmix_rejects_nonfinite_values() {
        let bytes: Vec<u8> = [0.5f32, -0.25, f32::NAN, 0.5]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(
            format(32, 2, Encoding::Float)
                .mono(&bytes, 2, false)
                .unwrap(),
            [0.125, 0.25]
        );
    }

    #[test]
    fn silent_packets_need_no_audio_memory() {
        assert_eq!(
            format(32, 2, Encoding::Float).mono(&[], 32, true).unwrap(),
            vec![0.0; 32]
        );
    }

    #[test]
    fn pcm24_sign_and_short_packets_are_checked() {
        assert_eq!(
            format(24, 1, Encoding::Pcm)
                .mono(&[0, 0, 128, 0, 0, 64], 2, false)
                .unwrap(),
            [-1.0, 0.5]
        );
        assert!(format(16, 2, Encoding::Pcm)
            .mono(&[0, 0], 2, false)
            .is_err());
    }

    #[test]
    fn timestamps_preserve_silence_gaps_and_saturate_before_epoch() {
        assert_eq!(
            timestamp_from_anchor(20_000_000, 10_000_000, Duration::from_millis(100)),
            Duration::from_millis(1100)
        );
        assert_eq!(
            timestamp_from_anchor(9_500_000, 10_000_000, Duration::from_millis(100)),
            Duration::from_millis(50)
        );
        assert_eq!(
            timestamp_from_anchor(1, 10_000_000, Duration::from_millis(100)),
            Duration::ZERO
        );
    }

    #[test]
    fn tap_downmix_handles_interleaved_and_planar_buffers() {
        // One interleaved stereo buffer.
        let stereo = [0.5f32, -0.5, 1.0, 0.0, f32::NAN, 0.25];
        assert_eq!(downmix(&[(&stereo, 2)]), [0.0, 0.5, 0.125]);
        // Two planar buffers; the shorter one bounds the frame count.
        let left = [0.25f32, 2.0, 0.5];
        let right = [0.25f32, 0.0];
        assert_eq!(downmix(&[(&left, 1), (&right, 1)]), [0.25, 0.5]);
        assert!(downmix(&[]).is_empty());
        assert!(downmix(&[(&[] as &[f32], 2)]).is_empty());
    }

    #[test]
    fn taps_need_macos_14_2_or_later() {
        for (version, supported) in [
            ("13.6.9", false),
            ("14.0", false),
            ("14.1.2", false),
            ("14.2", true),
            ("14.4.1", true),
            ("15.0", true),
            ("26.1", true),
            ("27.2", true),
            ("", false),
            ("garbage", false),
        ] {
            assert_eq!(version_supports_taps(version), supported, "{version}");
        }
    }

    #[test]
    fn preflight_results_map_to_distinct_permission_states() {
        assert_eq!(permission_from_preflight(0), SystemAudioStatus::Ready);
        assert_eq!(permission_from_preflight(1), SystemAudioStatus::Denied);
        assert_eq!(permission_from_preflight(2), SystemAudioStatus::NotDetermined);
        assert_eq!(permission_from_preflight(-1), SystemAudioStatus::NotDetermined);
        assert!(SystemAudioStatus::Denied.fixable());
        assert!(SystemAudioStatus::NotDetermined.fixable());
        assert!(!SystemAudioStatus::Ready.fixable());
        assert!(!SystemAudioStatus::Unsupported.fixable());
    }

    #[test]
    fn recording_source_keeps_a_fixable_system_side_and_drops_an_unsupported_one() {
        use SystemAudioStatus::*;
        for status in [Ready, NotDetermined, Denied, Unsupported] {
            assert_eq!(recording_source("microphone", status), Ok("microphone"));
        }
        for status in [Ready, NotDetermined, Denied] {
            assert_eq!(recording_source("both", status), Ok("both"));
            assert_eq!(recording_source("system", status), Ok("system"));
        }
        assert_eq!(recording_source("both", Unsupported), Ok("microphone"));
        assert!(recording_source("system", Unsupported).unwrap_err().starts_with(UNSUPPORTED_ERROR));
        assert!(recording_source("speakers", Ready).is_err());
    }

    #[test]
    fn permission_errors_are_recognized_by_prefix_only() {
        assert!(is_permission_error(PERMISSION_ERROR));
        assert!(!is_permission_error(UNSUPPORTED_ERROR));
        assert!(!is_permission_error("System audio stopped delivering audio."));
    }

    #[cfg(windows)]
    #[test]
    fn windows_loopback_needs_no_permission() {
        assert_eq!(status(), SystemAudioStatus::Ready);
        assert!(available());
        assert!(permission_granted());
        assert!(request_permission_prompted());
    }

    #[test]
    fn capture_handle_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<SystemAudioCapture>();
    }

    #[test]
    fn repeated_stop_requests_never_wait_for_the_owner() {
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let capture = SystemAudioCapture {
            stop_tx,
            cancel: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(FailureTime::default()),
            worker: None,
        };
        capture.request_stop();
        capture.request_stop(); // The one-slot stop queue is already full.
        assert_eq!(stop_rx.try_recv(), Ok(()));
        assert_eq!(stop_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    fn dropping_a_stalled_owner_returns_before_the_owner() {
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let (ended_tx, ended_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = ended_tx.send(());
            Ok(())
        });
        let capture = SystemAudioCapture {
            stop_tx,
            cancel: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(FailureTime::default()),
            worker: Some(worker),
        };
        assert!(!capture.is_finished());
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(capture);
            let _ = dropped_tx.send(());
        });
        let dropped = dropped_rx.recv_timeout(Duration::from_secs(1));
        let stopped = stop_rx.try_recv();
        // Always release the synthetic owner, including when the assertion fails.
        let _ = release_tx.send(());
        dropper.join().unwrap();
        ended_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(dropped.is_ok(), "Drop waited for the stalled capture owner");
        assert_eq!(stopped, Ok(()));
    }

    #[test]
    fn stop_times_out_without_waiting_for_native_cleanup() {
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let (ended_tx, ended_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = ended_tx.send(());
            Ok(())
        });
        let capture = SystemAudioCapture {
            stop_tx,
            cancel: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(FailureTime::default()),
            worker: Some(worker),
        };
        let (result_tx, result_rx) = mpsc::channel();
        let stopper = std::thread::spawn(move || {
            let _ = result_tx.send(capture.stop());
        });
        let result = result_rx.recv_timeout(Duration::from_secs(2));
        let stopped = stop_rx.try_recv();
        let _ = release_tx.send(());
        stopper.join().unwrap();
        ended_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(result, Ok(Err(error)) if error.contains("250 ms")));
        assert_eq!(stopped, Ok(()));
    }

    #[test]
    fn cancelled_pending_open_never_starts_after_blocked_native_open_returns() {
        let cancel = Arc::new(AtomicBool::new(false));
        let owner_cancel = cancel.clone();
        let starts = Arc::new(AtomicU64::new(0));
        let owner_starts = starts.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let opener = std::thread::spawn(move || {
            let result = SystemAudioCapture::start_owner(
                Instant::now(), owner_cancel, |_| {},
                move |_, ready, cancel, _| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap(); // deliberately blocked native activation
                    let result = start_unless_cancelled(cancel, || {
                        owner_starts.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    });
                    if result.is_ok() { let _ = ready.send(Ok(())); }
                    completed_tx.send(()).unwrap();
                    result
                },
            );
            let _ = result_tx.send(result.map(|_| ()));
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(2));
        cancel.store(true, Ordering::Release);
        let result = result_rx.recv_timeout(Duration::from_secs(2));
        // Release even on a failed assertion; this test leaves no blocked owner.
        let _ = release_tx.send(());
        let completed = completed_rx.recv_timeout(Duration::from_secs(2));
        opener.join().unwrap();
        assert!(entered.is_ok());
        assert!(matches!(result, Ok(Err(error)) if error.contains("cancelled")));
        assert!(completed.is_ok());
        assert_eq!(starts.load(Ordering::Acquire), 0);
    }

    #[test]
    fn cancellation_during_start_drops_the_late_stream_before_readiness() {
        struct StreamGuard(Arc<AtomicU64>);
        impl Drop for StreamGuard {
            fn drop(&mut self) { self.0.fetch_add(1, Ordering::Relaxed); }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let owner_cancel = cancel.clone();
        let stopped = Arc::new(AtomicU64::new(0));
        let owner_stopped = stopped.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            start_unless_cancelled(&owner_cancel, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap(); // Start itself is blocked
                Ok(StreamGuard(owner_stopped))
            }).map(|_| ())
        });
        let entered = entered_rx.recv_timeout(Duration::from_secs(2));
        cancel.store(true, Ordering::Release);
        let _ = release_tx.send(());
        let result = owner.join().unwrap();
        assert!(entered.is_ok());
        assert!(matches!(result, Err(error) if error.contains("cancelled")));
        assert_eq!(stopped.load(Ordering::Acquire), 1);
    }

    #[test]
    fn failure_time_survives_delayed_cleanup_and_later_error_reporting() {
        let (release_tx, release_rx) = mpsc::channel();
        let (failed_tx, failed_rx) = mpsc::channel();
        let capture = SystemAudioCapture::start_owner(
            Instant::now(), Arc::new(AtomicBool::new(false)), |_| {},
            move |_, ready, _, failure| {
                ready.send(Ok(())).unwrap();
                failure.record_cause(Duration::from_millis(3250), 2);
                failed_tx.send(()).unwrap();
                release_rx.recv().unwrap(); // native cleanup has not finished
                failure.record(Duration::from_secs(90));
                Err("synthetic disconnect".into())
            },
        ).unwrap();
        let failed = failed_rx.recv_timeout(Duration::from_secs(2));
        let at = capture.failure_at();
        let cause = capture.failure_cause();
        let finished = capture.is_finished();
        let _ = release_tx.send(());
        assert!(failed.is_ok());
        assert!(!finished);
        assert_eq!(at, Some(Duration::from_millis(3250)));
        assert_eq!(cause, 2);
        // Preserve the first failure even after the owner completes.
        let failure = capture.failure.clone();
        assert!(capture.stop().unwrap_err().contains("synthetic disconnect"));
        assert_eq!(failure.get(), at);
        assert_eq!(failure.cause.load(Ordering::Relaxed), 2);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Opt-in hardware smoke captures system audio for 250 ms; no file or playback."]
    fn windows_loopback_start_stop() {
        let (error_tx, error_rx) = mpsc::channel();
        let stream = SystemAudioCapture::start(
            Instant::now(),
            |chunk| {
                assert!(chunk.sample_rate >= 8000);
                assert!(chunk.samples.iter().all(|v| v.is_finite()));
            },
            move |error| {
                let _ = error_tx.send(error);
            },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(250));
        stream.stop().unwrap();
        assert!(error_rx.try_recv().is_err());
    }
}
