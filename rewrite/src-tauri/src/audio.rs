//! Native microphone capture. The CPAL stream is created, used and destroyed
//! on one owner thread; no stream crosses a thread boundary.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_devices() -> Result<Vec<Device>, String> {
    #[cfg(target_os = "macos")]
    {
        // A stalled CoreAudio driver must not hold startup or snapshot requests
        // indefinitely. Retain the pending probe so refresh cannot create an
        // unbounded number of threads when that driver never responds.
        type Probe = Receiver<Result<Vec<Device>, String>>;
        static PROBE: std::sync::Mutex<Option<Probe>> = std::sync::Mutex::new(None);
        let mut probe = PROBE
            .lock()
            .map_err(|_| "Microphone discovery was interrupted.")?;
        if probe.is_none() {
            let (sender, receiver) = mpsc::sync_channel(1);
            thread::Builder::new()
                .name("microphone-discovery".into())
                .spawn(move || {
                    let _ = sender.send(discover_devices());
                })
                .map_err(|e| format!("Cannot inspect microphones: {e}"))?;
            *probe = Some(receiver);
        }
        match probe.as_ref().unwrap().recv_timeout(Duration::from_secs(2)) {
            Ok(result) => { *probe = None; result }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(
                "Microphone discovery timed out. Check connected audio devices, then choose Refresh devices in Microphone.".into(),
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                *probe = None;
                Err("Microphone discovery stopped unexpectedly.".into())
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    discover_devices()
}

#[cfg(target_os = "macos")]
fn discover_devices() -> Result<Vec<Device>, String> {
    use coreaudio::audio_unit::{macos_helpers, Scope};
    // CPAL 0.15's input_devices() opens an AudioUnit to probe formats for
    // each device. Listing names only needs CoreAudio's input-channel metadata.
    let default = macos_helpers::get_default_device_id(true);
    let mut devices = Vec::new();
    for id in macos_helpers::get_audio_device_ids().map_err(|e| e.to_string())? {
        if !macos_helpers::get_audio_device_supports_scope(id, Scope::Input).unwrap_or(false) {
            continue;
        }
        let name = macos_helpers::get_device_name(id).map_err(|e| e.to_string())?;
        devices.push(Device {
            id: name.clone(),
            name,
            is_default: default == Some(id),
        });
    }
    Ok(devices)
}

#[cfg(not(target_os = "macos"))]
fn discover_devices() -> Result<Vec<Device>, String> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    host.input_devices()
        .map_err(|e| e.to_string())?
        .map(|d| {
            let name = d.name().map_err(|e| e.to_string())?;
            Ok(Device {
                id: name.clone(),
                is_default: default.as_ref() == Some(&name),
                name,
            })
        })
        .collect()
}

#[derive(Debug)]
pub enum AudioMessage {
    Chunk {
        samples: Vec<f32>,
        sample_rate: u32,
        rms: f32,
        captured_at: Duration,
        /// Audio was dropped before this chunk. The timestamp still includes it.
        discontinuity: bool,
        /// Missing mono frames since the last successfully queued chunk.
        dropped_frames: u64,
    },
    End,
    Error(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowPolicy {
    /// Dictation must never publish audio with an unreported loss.
    #[default]
    Fail,
    /// Meetings keep the source running and report the missing interval.
    Gap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioGap {
    /// Position of the first dropped frame relative to the supplied epoch.
    pub start: Duration,
    /// Exclusive end of the last dropped chunk relative to the same epoch.
    pub end: Duration,
    pub dropped_frames: u64,
    pub sample_rate: u32,
}

// The callback owns its pending gap. Only its destructor publishes the final
// undelivered gap; the audio callback never locks this mutex.
type PendingGap = Arc<Mutex<Option<AudioGap>>>;

const ERROR_STREAM: u32 = 1;
const ERROR_OVERFLOW: u32 = 2;
const ERROR_DISCONNECTED: u32 = 3;

const POST_ROLL_MIN_MS: u64 = 120;
const POST_ROLL_MAX_MS: u64 = 600;
const POST_ROLL_QUIET_MS: u64 = 200;
const START_CUE_MS: u64 = 55;
const CUE_ACOUSTIC_TAIL_MS: u64 = 100;

fn milliseconds_to_frames(ms: u64, rate: u32) -> u64 {
    ms.saturating_mul(u64::from(rate)).div_ceil(1000)
}

/// Sample-driven policy, also used by the deterministic evaluation replay.
/// Ten-ms power frames span callbacks and retain the pre-Stop quiet history.
struct PostRoll {
    rate: u32,
    frames: u64,
    stop_frame: Option<u64>,
    frame_samples: u32,
    frame_power: f64,
    recent: VecDeque<f64>,
    noise: [u64; 81],
    noise_frames: u64,
    quiet_power: f64,
    reason: Option<&'static str>,
}

impl PostRoll {
    fn new(rate: u32) -> Self {
        Self {
            rate, frames: 0, stop_frame: None, frame_samples: 0, frame_power: 0.0,
            recent: VecDeque::with_capacity(21), noise: [0; 81], noise_frames: 0,
            quiet_power: 0.001_f64.powi(2), reason: None,
        }
    }

    fn noise_quantile(&self, numerator: u64, denominator: u64) -> f64 {
        let target = self.noise_frames.saturating_sub(1) * numerator / denominator;
        let mut count = 0;
        for (bin, frames) in self.noise.iter().enumerate() {
            count += frames;
            if count > target {
                return 10.0_f64.powf((bin as f64 - 80.0) / 10.0);
            }
        }
        0.0
    }

    fn stop_at(&mut self, frame: u64) {
        if self.stop_frame.is_none() {
            self.stop_frame = Some(frame);
            // Freeze the session floor at Stop. The upper-quantile guard keeps
            // sustained speech from becoming its own noise floor (as on Mac).
            self.quiet_power = (self.noise_quantile(1, 20) * 4.0)
                .min(self.noise_quantile(4, 5) * 0.1).max(0.001_f64.powi(2));
        }
    }

    fn observe(&mut self, samples: &[f32], stopping: bool) -> usize {
        if stopping { self.stop_at(self.frames); }
        let frame_size = self.rate.div_ceil(100);
        let mut accepted = 0;
        for &sample in samples {
            if self.reason.is_some() { break; }
            self.frames += 1;
            accepted += 1;
            self.frame_power += f64::from(sample).powi(2);
            self.frame_samples += 1;
            if self.frame_samples == frame_size {
                let power = self.frame_power / f64::from(frame_size);
                self.recent.push_back(power);
                if self.recent.len() > 20 { self.recent.pop_front(); }
                if self.stop_frame.is_none() {
                    let bin = (10.0 * power.max(1e-8).log10() + 80.0).floor().clamp(0.0, 80.0) as usize;
                    self.noise[bin] += 1;
                    self.noise_frames += 1;
                }
                self.frame_power = 0.0;
                self.frame_samples = 0;
            }
            if let Some(stop) = self.stop_frame {
                let tail = self.frames.saturating_sub(stop);
                if tail >= milliseconds_to_frames(POST_ROLL_MAX_MS, self.rate) {
                    self.reason = Some("maximum");
                } else if tail >= milliseconds_to_frames(POST_ROLL_MIN_MS, self.rate)
                    && self.frame_samples == 0
                    && self.recent.len() as u64 * u64::from(frame_size)
                        >= milliseconds_to_frames(POST_ROLL_QUIET_MS, self.rate)
                    && self.recent.iter().all(|power| *power <= self.quiet_power)
                {
                    self.reason = Some("quiet");
                }
            }
        }
        accepted
    }

    fn tail_frames(&self) -> u64 {
        self.stop_frame.map_or(0, |stop| self.frames.saturating_sub(stop))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CueWindow { start: u64, end: u64 }

impl CueWindow {
    fn start_cue(start: u64, rate: u32) -> Self {
        Self { start, end: start.saturating_add(milliseconds_to_frames(START_CUE_MS + CUE_ACOUSTIC_TAIL_MS, rate)) }
    }

    fn suppress(self, samples: &mut [f32], first: u64) -> u64 {
        let begin = self.start.saturating_sub(first).min(samples.len() as u64) as usize;
        let end = self.end.saturating_sub(first).min(samples.len() as u64) as usize;
        samples[begin..end].fill(0.0);
        (end - begin) as u64
    }
}

#[derive(Clone, Copy)]
struct CaptureClock { frame: u64, at: Duration, rate: u32 }

impl CaptureClock {
    fn frame_at(self, at: Duration) -> u64 {
        let offset = |duration: Duration| (duration.as_nanos() * u128::from(self.rate) / 1_000_000_000) as u64;
        if at >= self.at {
            self.frame.saturating_add(offset(at - self.at))
        } else {
            self.frame.saturating_sub(offset(self.at - at))
        }
    }
}

#[derive(Default)]
struct CueState {
    clock: Option<CaptureClock>,
    window: Option<CueWindow>,
    consumed: u64,
    suppressed: u64,
}

impl CueState {
    fn schedule(&mut self, playback_at: Duration) -> bool {
        let Some(clock) = self.clock else { return false };
        let start = clock.frame_at(playback_at);
        // ASR commits consumption under this same short lock. If hardware
        // timestamps lag already-consumed audio, defer the cue to a later
        // output buffer; never retroactively mute or alter preceding speech.
        if start < self.consumed || self.window.is_some() { return false; }
        self.window = Some(CueWindow::start_cue(start, clock.rate));
        true
    }

    fn filter(&mut self, samples: &mut [f32]) {
        if let Some(window) = self.window {
            self.suppressed += window.suppress(samples, self.consumed);
        }
        self.consumed += samples.len() as u64;
    }
}

#[derive(Default)]
struct CaptureCompletion {
    cancel: Arc<AtomicBool>,
    stop_at_ns: AtomicU64,
    closed: AtomicBool,
    tail_frames: AtomicU64,
    rate: AtomicU32,
    reason: AtomicU32,
    wait_ms: AtomicU64,
    cue: Mutex<CueState>,
}

#[derive(Clone)]
pub(crate) struct StartCue {
    completion: Arc<CaptureCompletion>,
    stop: Arc<AtomicBool>,
    epoch: Instant,
}

impl StartCue {
    pub(crate) fn play(self) { play_cue_with_capture("start", Some(self)); }
    fn interrupted(&self) -> bool {
        self.stop.load(Ordering::Acquire) || self.completion.closed.load(Ordering::Acquire)
            || self.completion.cancel.load(Ordering::Acquire)
    }
    pub(crate) fn request_stop(&self) {
        self.completion.mark_stop(self.epoch.elapsed());
        self.stop.store(true, Ordering::Release);
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CaptureDiagnostics {
    pub sample_rate: u32,
    pub post_roll_frames: u64,
    pub post_roll_ms: f64,
    pub post_roll_wait_ms: u64,
    pub post_roll_reason: &'static str,
    pub cue_start_frame: Option<u64>,
    pub cue_end_frame: Option<u64>,
    pub cue_suppressed_frames: u64,
}

impl CaptureCompletion {
    fn mark_stop(&self, at: Duration) -> Duration {
        let proposed = (at.as_nanos() as u64).saturating_add(1);
        let recorded = self.stop_at_ns.compare_exchange(0, proposed, Ordering::AcqRel, Ordering::Acquire)
            .unwrap_or_else(|recorded| recorded);
        let encoded = if recorded == 0 { proposed } else { recorded };
        Duration::from_nanos(encoded - 1)
    }

    fn end_reason(&self, elapsed: Duration, failed: bool) -> Option<u32> {
        if self.cancel.load(Ordering::Acquire) { Some(4) }
        else if failed { Some(5) }
        else if self.closed.load(Ordering::Acquire) { Some(self.reason.load(Ordering::Acquire)) }
        else if elapsed >= Duration::from_millis(POST_ROLL_MAX_MS) { Some(3) }
        else { None }
    }

    fn diagnostics(&self) -> CaptureDiagnostics {
        let rate = self.rate.load(Ordering::Acquire);
        let tail = self.tail_frames.load(Ordering::Acquire);
        let cue = self.cue.lock().unwrap_or_else(|e| e.into_inner());
        CaptureDiagnostics {
            sample_rate: rate, post_roll_frames: tail,
            post_roll_ms: tail as f64 * 1000.0 / f64::from(rate.max(1)),
            post_roll_wait_ms: self.wait_ms.load(Ordering::Acquire),
            post_roll_reason: match self.reason.load(Ordering::Acquire) {
                1 => "quiet", 2 => "maximum", 3 => "wall_timeout", 4 => "cancelled",
                5 => "capture_error", _ => "not_stopped",
            },
            cue_start_frame: cue.window.map(|w| w.start),
            cue_end_frame: cue.window.map(|w| w.end), cue_suppressed_frames: cue.suppressed,
        }
    }
}

/// Hardware-free evaluation of the production post-roll policy. Supply the
/// original mono samples, including audio available after the stop position.
/// The returned cutoff is exclusive; the source slice is never changed.
#[cfg(feature = "fidelity-eval")]
pub fn replay_capture_post_roll(samples: &[f32], rate: u32, stop_frame: usize) -> Result<usize, String> {
    if rate == 0 || rate > 384_000 || stop_frame > samples.len() || samples.iter().any(|s| !s.is_finite()) {
        return Err("Invalid capture replay input.".into());
    }
    let mut policy = PostRoll::new(rate);
    policy.observe(&samples[..stop_frame], false);
    Ok(stop_frame + policy.observe(&samples[stop_frame..], true))
}

/// Apply precisely the production 55-ms start cue plus 100-ms acoustic tail
/// to an evaluation-owned ASR copy. `first_frame` supports arbitrary blocks.
#[cfg(feature = "fidelity-eval")]
pub fn replay_suppress_start_cue(samples: &mut [f32], rate: u32, first_frame: u64, cue_frame: u64) -> Result<u64, String> {
    if rate == 0 || rate > 384_000 { return Err("Invalid capture replay rate.".into()); }
    Ok(CueWindow::start_cue(cue_frame, rate).suppress(samples, first_frame))
}

fn fail_capture(error_cause: &AtomicU32, cause: u32) {
    // Keep the first failure when the data and error callbacks race.
    let _ = error_cause.compare_exchange(0, cause, Ordering::AcqRel, Ordering::Acquire);
}

fn frame_duration(frames: u64, sample_rate: u32) -> Duration {
    let rate = u64::from(sample_rate);
    Duration::new(frames / rate, ((frames % rate) * 1_000_000_000 / rate) as u32)
}

#[derive(Default)]
struct CaptureTimeline {
    anchor: Option<Duration>,
    captured_frames: u64,
}

impl CaptureTimeline {
    fn advance(
        &mut self,
        received_at: Duration,
        delay: Duration,
        device_elapsed: Option<Duration>,
        frames: u64,
        sample_rate: u32,
    ) -> Duration {
        let anchor = *self.anchor.get_or_insert(received_at.saturating_sub(delay));
        let elapsed =
            device_elapsed.unwrap_or_else(|| frame_duration(self.captured_frames, sample_rate));
        // Count every captured frame before trying the queue, including losses.
        self.captured_frames = self.captured_frames.saturating_add(frames);
        anchor.saturating_add(elapsed)
    }
}

struct ChunkQueue {
    sender: SyncSender<AudioMessage>,
    policy: OverflowPolicy,
    error_cause: Arc<AtomicU32>,
    pending_gap: Option<AudioGap>,
    terminal_gap: PendingGap,
}

impl ChunkQueue {
    fn push(&mut self, samples: Vec<f32>, sample_rate: u32, rms: f32, captured_at: Duration) {
        if self.error_cause.load(Ordering::Acquire) != 0 || samples.is_empty() {
            return;
        }
        let frames = samples.len() as u64;
        let dropped_frames = self.pending_gap.map_or(0, |gap| gap.dropped_frames);
        match self.sender.try_send(AudioMessage::Chunk {
            samples,
            sample_rate,
            rms,
            captured_at,
            discontinuity: dropped_frames != 0,
            dropped_frames,
        }) {
            Ok(()) => self.pending_gap = None,
            Err(TrySendError::Full(_)) => match self.policy {
                OverflowPolicy::Fail => fail_capture(&self.error_cause, ERROR_OVERFLOW),
                OverflowPolicy::Gap => {
                    let end = captured_at.saturating_add(frame_duration(frames, sample_rate));
                    let gap = self.pending_gap.get_or_insert(AudioGap {
                        start: captured_at,
                        end,
                        dropped_frames: 0,
                        sample_rate,
                    });
                    gap.start = gap.start.min(captured_at);
                    gap.end = gap.end.max(end);
                    gap.dropped_frames = gap.dropped_frames.saturating_add(frames);
                }
            },
            Err(TrySendError::Disconnected(_)) => {
                fail_capture(&self.error_cause, ERROR_DISCONNECTED);
            }
        }
    }
}

impl Drop for ChunkQueue {
    fn drop(&mut self) {
        if let Some(gap) = self.pending_gap.take() {
            *self.terminal_gap.lock().unwrap_or_else(|e| e.into_inner()) = Some(gap);
        }
    }
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    completion: Arc<CaptureCompletion>,
    epoch: Instant,
    level: Arc<AtomicU32>,
    receiver: Option<Receiver<AudioMessage>>,
    owner: Option<JoinHandle<()>>,
    pending_gap: PendingGap,
    error_cause: Arc<AtomicU32>,
}

// A timed-out native open can still be stuck inside the driver. Keep meeting
// retries from accumulating native owner threads until that owner exits.
// One selected-device lane and one default lane let a five-second fallback
// bypass a stuck selected driver without permitting unbounded native owners.
static MEETING_OWNER_ACTIVE: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
struct MeetingOwnerLease(Option<usize>);
impl Drop for MeetingOwnerLease {
    fn drop(&mut self) {
        if let Some(index) = self.0 { MEETING_OWNER_ACTIVE[index].store(false, Ordering::Release); }
    }
}

static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(1);

// Metadata only: never pass audio, device names, transcript text, or native
// errors to the bounded diagnostic log. No disk I/O occurs in audio callbacks.
pub(crate) fn capture_event(kind: &'static str, attempt: u64, epoch: Instant, value: u64) {
    crate::recognition_diagnostics::event(
        kind,
        "",
        std::process::id(),
        attempt,
        epoch.elapsed().as_millis() as u64,
        value,
    );
}

impl Capture {
    pub fn start(device_id: Option<String>) -> Result<Self, String> {
        Self::start_at(device_id, Instant::now())
    }
    pub fn start_at(device_id: Option<String>, epoch: Instant) -> Result<Self, String> {
        Self::start_at_with_policy(device_id, epoch, OverflowPolicy::Fail)
    }
    pub fn start_at_with_policy(
        device_id: Option<String>,
        epoch: Instant,
        policy: OverflowPolicy,
    ) -> Result<Self, String> {
        Self::start_with_stop(device_id, epoch, policy, Arc::new(AtomicBool::new(false)))
    }
    pub(crate) fn start_with_stop(
        device_id: Option<String>, epoch: Instant, policy: OverflowPolicy, stop: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let lane = (policy == OverflowPolicy::Gap).then(|| usize::from(device_id.as_ref().is_some_and(|id| !id.is_empty())));
        if let Some(index) = lane {
            if MEETING_OWNER_ACTIVE[index].compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
                return Err("The previous microphone capture is still closing. Retrying shortly.".into());
            }
        }
        let lease = MeetingOwnerLease(lane);
        let attempt = NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
        capture_event("microphone-start", attempt, epoch, 0);
        // Dictation fails closed; meetings explicitly account for dropped frames.
        let (tx, receiver) = mpsc::sync_channel(4096);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let level = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let pending_gap = Arc::new(Mutex::new(None));
        let thread_gap = pending_gap.clone();
        let thread_level = level.clone();
        let thread_stop = stop.clone();
        let error_cause = Arc::new(AtomicU32::new(0));
        let owner_error_cause = error_cause.clone();
        let completion = Arc::new(CaptureCompletion::default());
        let owner_completion = completion.clone();
        let owner = thread::Builder::new().name("microphone-owner".into()).spawn(move || {
            let _lease = lease;
            let error_cause = owner_error_cause;
            let completion = owner_completion;
            let queue = ChunkQueue {
                sender: tx.clone(),
                policy,
                error_cause: error_cause.clone(),
                pending_gap: None,
                terminal_gap: thread_gap,
            };
            let stream = build_stream(device_id.as_deref(), queue, epoch, thread_level.clone(), attempt,
                thread_stop.clone(), completion.clone());
            let stream = match stream {
                Ok(stream) => stream,
                Err(error) => { capture_event("microphone-open-error", attempt, epoch, 0); let _ = ready_tx.send(Err(error)); return; }
            };
            if thread_stop.load(Ordering::Acquire) {
                let _ = ready_tx.send(Err("Microphone startup was cancelled.".into()));
                return;
            }
            capture_event("microphone-stream-built", attempt, epoch, 0);
            capture_event("microphone-play", attempt, epoch, 0);
            if let Err(error) = stream.play() {
                capture_event("microphone-play-error", attempt, epoch, 0);
                let _ = ready_tx.send(Err(format!("Cannot start microphone: {error}")));
                return;
            }
            capture_event("microphone-ready", attempt, epoch, 0);
            let _ = ready_tx.send(Ok(()));
            while !thread_stop.load(Ordering::Acquire) && !completion.cancel.load(Ordering::Acquire)
                && error_cause.load(Ordering::Acquire) == 0 {
                thread::sleep(Duration::from_millis(10));
            }
            // The callback ends on real captured quiet, including pre-Stop
            // samples. Wall time is an independent bound for a stalled device.
            let stopped = completion.mark_stop(epoch.elapsed());
            let reason = loop {
                if let Some(reason) = completion.end_reason(epoch.elapsed().saturating_sub(stopped),
                    error_cause.load(Ordering::Acquire) != 0) { break reason; }
                thread::sleep(Duration::from_millis(2));
            };
            completion.closed.store(true, Ordering::Release);
            completion.wait_ms.store(epoch.elapsed().saturating_sub(stopped).as_millis() as u64, Ordering::Release);
            completion.reason.store(reason, Ordering::Release);
            let observed_cause = error_cause.load(Ordering::Acquire);
            if observed_cause != 0 {
                capture_event("microphone-error", attempt, epoch, u64::from(observed_cause));
            }
            let _ = stream.pause();
            drop(stream);
            let cause = error_cause.load(Ordering::Acquire);
            if cause != 0 && observed_cause == 0 {
                capture_event("microphone-error", attempt, epoch, u64::from(cause));
            }
            capture_event("microphone-stopped", attempt, epoch, u64::from(cause != 0));
            let diagnostics = completion.diagnostics();
            capture_event("microphone-post-roll-ms", attempt, epoch, diagnostics.post_roll_ms as u64);
            thread_level.store(0.0f32.to_bits(), Ordering::Relaxed);
            if cause != 0 {
                let _ = tx.send(AudioMessage::Error("Microphone capture failed or its audio queue overflowed; this recording is incomplete.".into()));
            }
            let _ = tx.send(AudioMessage::End);
        }).map_err(|e| e.to_string())?;
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                completion,
                epoch,
                level,
                receiver: Some(receiver),
                owner: Some(owner),
                pending_gap,
                error_cause,
            }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::Release);
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                capture_event("microphone-ready-timeout", attempt, epoch, 0);
                Err("Microphone did not become ready within 8 seconds.".into())
            }
        }
    }
    pub fn take_receiver(&mut self) -> Result<Receiver<AudioMessage>, String> {
        self.receiver
            .take()
            .ok_or_else(|| "Audio receiver was already taken.".into())
    }
    pub fn stop(&self) {
        self.completion.mark_stop(self.epoch.elapsed());
        self.stop.store(true, Ordering::Release);
    }
    pub fn request_stop(&self) {
        self.stop();
    }
    pub fn is_finished(&self) -> bool {
        self.owner.as_ref().map_or(true, JoinHandle::is_finished)
    }
    pub(crate) fn failure_cause(&self) -> u32 {
        self.error_cause.load(Ordering::Acquire)
    }
    /// Take an overflow interval that no later chunk could report. Call after
    /// End (or after is_finished()); this returns None until the stream is dropped.
    /// Drain queued chunks first so their gap reports remain in timeline order.
    pub fn take_pending_gap(&self) -> Option<AudioGap> {
        self.pending_gap
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
    pub(crate) fn stop_handle(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub(crate) fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.completion.cancel.clone()
    }
    pub(crate) fn start_cue(&self) -> StartCue {
        StartCue { completion: self.completion.clone(), stop: self.stop.clone(), epoch: self.epoch }
    }
    pub(crate) fn filter_asr(&self, samples: &mut [f32]) {
        self.completion.cue.lock().unwrap_or_else(|e| e.into_inner()).filter(samples);
    }
    pub(crate) fn diagnostics(&self) -> CaptureDiagnostics { self.completion.diagnostics() }
    pub(crate) fn level_handle(&self) -> Arc<AtomicU32> {
        self.level.clone()
    }
    pub fn join(&mut self) {
        if let Some(owner) = self.owner.take() {
            let _ = owner.join();
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.completion.cancel.store(true, Ordering::Release);
        self.stop();
        // Disconnect an untaken receiver so a full queue cannot
        // block the owner while it sends the terminal messages.
        self.receiver.take();
        // Dropping the handle detaches the native owner. A taken receiver belongs
        // to the consumer; it must keep draining or disconnect before joining.
    }
}

fn build_stream(
    device_id: Option<&str>,
    queue: ChunkQueue,
    epoch: Instant,
    level: Arc<AtomicU32>,
    attempt: u64,
    stop: Arc<AtomicBool>,
    completion: Arc<CaptureCompletion>,
) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = match device_id.filter(|s| !s.is_empty()) {
        Some(id) => {
            #[cfg(target_os = "macos")]
            let devices = host.devices();
            #[cfg(not(target_os = "macos"))]
            let devices = host.input_devices();
            devices
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(id))
                .ok_or_else(|| format!("The selected microphone is unavailable: {id}"))?
        }
        None => host.default_input_device().ok_or(
            "No microphone is available. Check microphone permission and the selected input.",
        )?,
    };
    capture_event("microphone-config-request", attempt, epoch, 0);
    let supported = device
        .default_input_config()
        .map_err(|e| format!("Cannot open microphone: {e}"))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    if config.channels == 0 || config.sample_rate.0 == 0 {
        return Err("The microphone returned an invalid audio format.".into());
    }
    capture_event("microphone-config-ready", attempt, epoch, u64::from(config.sample_rate.0));
    capture_event("microphone-stream-build", attempt, epoch, u64::from(config.channels));
    match format {
        cpal::SampleFormat::F32 => make_stream::<f32>(&device, &config, queue, epoch, level, stop, completion),
        cpal::SampleFormat::I16 => make_stream::<i16>(&device, &config, queue, epoch, level, stop, completion),
        cpal::SampleFormat::U16 => make_stream::<u16>(&device, &config, queue, epoch, level, stop, completion),
        cpal::SampleFormat::I32 => make_stream::<i32>(&device, &config, queue, epoch, level, stop, completion),
        cpal::SampleFormat::F64 => make_stream::<f64>(&device, &config, queue, epoch, level, stop, completion),
        other => Err(format!("Unsupported microphone sample format: {other:?}")),
    }
}

fn make_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut queue: ChunkQueue,
    epoch: Instant,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    completion: Arc<CaptureCompletion>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0;
    let error_cause = queue.error_cause.clone();
    let mut timeline = CaptureTimeline::default();
    let mut first_capture_instant = None;
    let mut post_roll = PostRoll::new(sample_rate);
    completion.rate.store(sample_rate, Ordering::Release);
    device
        .build_input_stream(
            config,
            move |data: &[T], info| {
                let received_at = epoch.elapsed();
                if queue.error_cause.load(Ordering::Acquire) != 0 || completion.closed.load(Ordering::Acquire)
                    || completion.cancel.load(Ordering::Acquire) {
                    return;
                }
                let mut samples: Vec<f32> = data
                    .chunks_exact(channels)
                    .map(|frame| {
                        frame
                            .iter()
                            .map(|&s| cpal::Sample::to_sample::<f32>(s))
                            .sum::<f32>()
                            / channels as f32
                    })
                    .map(|s| {
                        if s.is_finite() {
                            s.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    })
                    .collect();
                if samples.is_empty() {
                    return;
                }
                // Anchor once to the common Instant epoch, then use device capture
                // timestamps. WASAPI supplies QPC for the first captured frame; this
                // follows hardware clock drift without accumulating callback jitter.
                let stamp = info.timestamp();
                let delay = stamp
                    .callback
                    .duration_since(&stamp.capture)
                    .unwrap_or_else(|| frame_duration(samples.len() as u64, sample_rate));
                let first = *first_capture_instant.get_or_insert(stamp.capture);
                let captured_at = timeline.advance(
                    received_at,
                    delay,
                    stamp.capture.duration_since(&first),
                    samples.len() as u64,
                    sample_rate,
                );
                // Clock updates and cue scheduling never block a native audio
                // callback. An older clock still maps the same sample timeline.
                let clock = CaptureClock { frame: post_roll.frames, at: captured_at, rate: sample_rate };
                if let Ok(mut cue) = completion.cue.try_lock() {
                    cue.clock = Some(clock);
                }
                let mut accepted = 0;
                // Observe Stop/cancel within large hardware blocks as well.
                for chunk in samples.chunks(sample_rate.div_ceil(100) as usize) {
                    if completion.closed.load(Ordering::Acquire) || completion.cancel.load(Ordering::Acquire) { break; }
                    if stop.load(Ordering::Acquire) {
                        // A late callback can contain audio captured before
                        // Stop. Count its tail from the hardware timeline, not
                        // the time that this callback finally arrived.
                        post_roll.stop_at(clock.frame_at(completion.mark_stop(epoch.elapsed())));
                    }
                    accepted += post_roll.observe(chunk, false);
                    if post_roll.reason.is_some() { break; }
                }
                samples.truncate(accepted);
                completion.tail_frames.store(post_roll.tail_frames(), Ordering::Release);
                if let Some(reason) = post_roll.reason {
                    let _ = completion.reason.compare_exchange(0, if reason == "quiet" { 1 } else { 2 }, Ordering::AcqRel, Ordering::Acquire);
                    completion.closed.store(true, Ordering::Release);
                }
                if samples.is_empty() { return; }
                let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
                level.store(rms.to_bits(), Ordering::Relaxed);
                queue.push(samples, sample_rate, rms, captured_at);
            },
            move |_| {
                fail_capture(&error_cause, ERROR_STREAM);
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Best-effort short cues on a separate output owner. Never blocks capture or
/// completion, and never writes microphone data to an output device.
pub fn play_cue(kind: &str) {
    play_cue_with_capture(kind, None);
}

struct ToneLifetime {
    epoch: Instant,
    // 0 = unarmed, 1 = awaiting a callback, 2 = playing, 3 = abandoned.
    phase: AtomicU32,
    end_ns: AtomicU64,
}

impl ToneLifetime {
    // Shared by every output callback and its owner, including after playback
    // starts. A cancelled cue must not keep producing sound for another session.
    fn active(&self, cue: Option<&StartCue>) -> bool {
        if cue.is_some_and(StartCue::interrupted) {
            self.phase.store(3, Ordering::Release);
            return false;
        }
        matches!(self.phase.load(Ordering::Acquire), 1 | 2)
    }
}

fn play_cue_with_capture(kind: &str, cue: Option<StartCue>) {
    let (frequency, seconds) = match kind {
        "start" => (660.0, START_CUE_MS as f64 / 1000.0),
        "stop" => (440.0, 0.045),
        "complete" => (880.0, 0.07),
        "error" => (220.0, 0.14),
        _ => return,
    };
    thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return;
        };
        let Ok(supported) = device.default_output_config() else {
            return;
        };
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        if config.channels == 0 || config.sample_rate.0 == 0 { return; }
        // A failed play/build creates no suppression window. Some hosts can
        // invoke a callback during play(), so keep it silent until success.
        let lifetime = Arc::new(ToneLifetime { epoch: Instant::now(), phase: AtomicU32::new(0), end_ns: AtomicU64::new(0) });
        let capture_cue = cue.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => tone::<f32>(&device, &config, frequency, seconds, cue, lifetime.clone()),
            cpal::SampleFormat::I16 => tone::<i16>(&device, &config, frequency, seconds, cue, lifetime.clone()),
            cpal::SampleFormat::U16 => tone::<u16>(&device, &config, frequency, seconds, cue, lifetime.clone()),
            _ => return,
        };
        if let Ok(stream) = stream {
            if stream.play().is_ok() {
                let _ = lifetime.phase.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                loop {
                    if !lifetime.active(capture_cue.as_ref()) { break; }
                    let phase = lifetime.phase.load(Ordering::Acquire);
                    let end = lifetime.end_ns.load(Ordering::Acquire);
                    if phase == 3 || (phase == 2 && end != 0 && lifetime.epoch.elapsed().as_nanos() >= u128::from(end)) { break; }
                    if lifetime.epoch.elapsed() >= Duration::from_secs(1)
                        && lifetime.phase.compare_exchange(1, 3, Ordering::AcqRel, Ordering::Acquire).is_ok() { break; }
                    // Keep the stream through the actual scheduled playback,
                    // even if output startup or cue notification was delayed.
                    thread::sleep(Duration::from_millis(2));
                }
            }
            drop(stream);
        }
    });
}
fn tone<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    frequency: f32,
    seconds: f64,
    cue: Option<StartCue>,
    lifetime: Arc<ToneLifetime>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let rate = config.sample_rate.0 as f32;
    let frames = (seconds * rate as f64) as usize;
    let mut frame = 0;
    let error_lifetime = lifetime.clone();
    device.build_output_stream(
        config,
        move |data: &mut [T], info| {
            if data.is_empty() || !lifetime.active(cue.as_ref()) {
                data.fill(T::from_sample(0.0));
                return;
            }
            if frame == 0 {
                let stamp = info.timestamp();
                let playback_delay = stamp.playback.duration_since(&stamp.callback).unwrap_or_default();
                if let Some(cue) = &cue {
                    if cue.interrupted() {
                        lifetime.phase.store(3, Ordering::Release);
                        data.fill(T::from_sample(0.0));
                        return;
                    } else {
                        let playback_at = cue.epoch.elapsed().saturating_add(playback_delay);
                        let scheduled = cue.completion.cue.try_lock().is_ok_and(|mut state| {
                            if lifetime.phase.compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire).is_err() { return false; }
                            if state.schedule(playback_at) { true } else {
                                let _ = lifetime.phase.compare_exchange(2, 1, Ordering::AcqRel, Ordering::Acquire);
                                false
                            }
                        });
                        if !scheduled {
                            data.fill(T::from_sample(0.0));
                            return;
                        }
                    }
                } else if lifetime.phase.compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire).is_err() {
                    data.fill(T::from_sample(0.0));
                    return;
                }
                let end = lifetime.epoch.elapsed().saturating_add(playback_delay)
                    .saturating_add(Duration::from_secs_f64(seconds + 0.09));
                lifetime.end_ns.store(end.as_nanos() as u64, Ordering::Release);
            }
            for output in data.chunks_mut(channels) {
                let envelope = ((frame as f32 / (rate * 0.005)).min(1.0))
                    .min((frames.saturating_sub(frame) as f32 / (rate * 0.005)).min(1.0));
                let sample = if frame < frames {
                    (std::f32::consts::TAU * frequency * frame as f32 / rate).sin()
                        * 0.06
                        * envelope
                } else {
                    0.0
                };
                for value in output {
                    *value = T::from_sample(sample);
                }
                frame += 1;
            }
        },
        move |_| { error_lifetime.phase.store(3, Ordering::Release); },
        None,
    )
}

/// A Send handle for microphone-only meeting recording. The WAV writer and
/// capture are owned by a worker; Stop finalizes the file before returning.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    done: Receiver<Result<f64, String>>,
    worker: Option<JoinHandle<()>>,
    pub path: PathBuf,
}
impl Recorder {
    pub fn start(path: &Path, device: Option<String>) -> Result<Self, String> {
        Self::start_with_level(path, device, Arc::new(|_| {}))
    }
    pub fn start_with_level(
        path: &Path,
        device: Option<String>,
        on_level: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<Self, String> {
        if path.exists() {
            return Err("Refusing to overwrite an existing recording.".into());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut capture = Capture::start(device)?;
        let receiver = capture.take_receiver()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let output = path.to_owned();
        let (done_tx, done) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("meeting-microphone".into())
            .spawn(move || {
                let result =
                    write_recording(&output, &mut capture, receiver, &thread_stop, &*on_level);
                let _ = done_tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stop,
            done,
            worker: Some(worker),
            path: path.to_owned(),
        })
    }
    pub fn stop(&mut self) -> Result<f64, String> {
        self.stop.store(true, Ordering::Release);
        let result = self
            .done
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| "Timed out finalizing microphone recording.".to_string())?;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        result
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn write_recording(
    path: &Path,
    capture: &mut Capture,
    receiver: Receiver<AudioMessage>,
    stop: &AtomicBool,
    level: &dyn Fn(f32),
) -> Result<f64, String> {
    let mut writer = None;
    let mut frames = 0u64;
    let mut rate = 0u32;
    let mut last_level = Instant::now();
    let mut last_checkpoint = Instant::now();
    let mut capture_error = None;
    loop {
        if stop.load(Ordering::Acquire) {
            capture.stop();
        }
        match receiver.recv_timeout(Duration::from_millis(30)) {
            Ok(AudioMessage::Chunk {
                samples,
                sample_rate,
                rms,
                ..
            }) => {
                if writer.is_none() {
                    rate = sample_rate;
                    writer = Some(crate::recording::PcmWriter::create_at_rate(
                        path,
                        sample_rate,
                    )?);
                }
                if rate != sample_rate {
                    return Err("Microphone sample rate changed during recording.".into());
                }
                frames += samples.len() as u64;
                writer.as_mut().unwrap().write_samples(&samples)?;
                if last_checkpoint.elapsed() >= Duration::from_secs(1) {
                    writer.as_mut().unwrap().checkpoint()?;
                    last_checkpoint = Instant::now();
                }
                if last_level.elapsed() >= Duration::from_millis(50) {
                    level(rms);
                    last_level = Instant::now();
                }
            }
            Ok(AudioMessage::Error(error)) => {
                capture_error = Some(error);
                capture.stop();
            }
            Ok(AudioMessage::End) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("Microphone worker disconnected before finalization.".into())
            }
        }
    }
    capture.join();
    if let Some(mut writer) = writer {
        writer.checkpoint()?;
    }
    if let Some(error) = capture_error {
        return Err(error);
    }
    if rate == 0 {
        return Err("No microphone audio was captured.".into());
    }
    Ok(frames as f64 / rate as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay_tail(rate: u32, before: &[f32], after: &[f32], block: usize) -> PostRoll {
        let mut policy = PostRoll::new(rate);
        for chunk in before.chunks(block) { assert_eq!(policy.observe(chunk, false), chunk.len()); }
        for chunk in after.chunks(block) {
            policy.observe(chunk, true);
            if policy.reason.is_some() { break; }
        }
        policy
    }

    #[test]
    fn post_roll_uses_pre_stop_quiet_and_never_finishes_before_120_ms() {
        for rate in [16_000, 44_100, 48_000, 96_000] {
            let before = vec![0.0; rate as usize];
            let after = vec![0.0; rate as usize];
            let policy = replay_tail(rate, &before, &after, 127);
            assert_eq!(policy.tail_frames(), milliseconds_to_frames(120, rate));
            assert_eq!(policy.reason, Some("quiet"));
        }
    }

    #[test]
    fn post_roll_waits_for_a_full_200_ms_window_on_a_short_session() {
        let policy = replay_tail(16_000, &[], &vec![0.0; 16_000], 17);
        assert_eq!(policy.tail_frames(), 3_200);
        assert_eq!(policy.reason, Some("quiet"));
    }

    #[test]
    fn post_roll_stop_at_zero_30_or_60_ms_after_final_word_keeps_200_ms_quiet() {
        let rate = 16_000;
        for delay in [0, 30, 60] {
            let mut before = vec![0.0; rate as usize];
            before.extend(vec![0.04; 3_200]);
            before.extend(vec![0.0; milliseconds_to_frames(delay, rate) as usize]);
            for block in [1, 127, 512, 16_000] {
                let policy = replay_tail(rate, &before, &vec![0.0; 16_000], block);
                assert_eq!(policy.tail_frames(), milliseconds_to_frames(200 - delay, rate));
                assert_eq!(policy.reason, Some("quiet"));
            }
        }
    }

    #[test]
    fn post_roll_preserves_a_short_word_after_stop_and_ignores_callback_boundaries() {
        let before = vec![0.0; 16_000];
        let mut after = vec![0.04; 2_400]; // 150-ms word, shorter than Silero's trigger.
        after.extend(vec![0.0; 16_000]);
        for block in [1, 61, 319, 512, 8_192] {
            let policy = replay_tail(16_000, &before, &after, block);
            assert_eq!(policy.tail_frames(), 5_600); // word plus 200 ms of real quiet
            assert_eq!(policy.reason, Some("quiet"));
        }
    }

    #[test]
    fn post_roll_checks_all_recent_frames_instead_of_diluting_a_final_consonant() {
        let before = vec![0.0; 16_000];
        let mut after = vec![0.0; 1_600];
        after.extend(vec![0.01; 160]); // Last 10-ms speech frame at 100 ms.
        after.extend(vec![0.0; 16_000]);
        let policy = replay_tail(16_000, &before, &after, 97);
        assert_eq!(policy.tail_frames(), 4_960);
    }

    #[test]
    fn post_roll_freezes_noise_floor_and_truncates_a_large_block_at_600_ms() {
        for rate in [16_000, 44_100, 48_000] {
            let policy = replay_tail(rate, &vec![0.04; rate as usize], &vec![0.04; rate as usize], rate as usize);
            assert_eq!(policy.tail_frames(), milliseconds_to_frames(600, rate));
            assert_eq!(policy.reason, Some("maximum"));
        }
    }

    #[test]
    fn post_roll_quiet_is_relative_to_session_noise_with_speech_headroom() {
        let mut before = vec![0.004; 16_000];
        before.extend(vec![0.04; 16_000]);
        before.extend(vec![0.004; 3_200]);
        let policy = replay_tail(16_000, &before, &vec![0.004; 16_000], 511);
        assert_eq!(policy.tail_frames(), 1_920);
        assert_eq!(policy.reason, Some("quiet"));
    }

    #[test]
    fn late_input_callback_counts_post_roll_from_capture_time_not_delivery_time() {
        let mut policy = PostRoll::new(1_000);
        policy.observe(&[0.0; 200], false);
        // This block arrives at 600 ms but contains frames captured at 200 ms.
        let clock = CaptureClock { frame: 200, at: Duration::from_millis(200), rate: 1_000 };
        policy.stop_at(clock.frame_at(Duration::from_millis(400)));
        assert_eq!(policy.observe(&[0.0; 800], false), 320);
        assert_eq!(policy.frames, 520);
        assert_eq!(policy.tail_frames(), 120);
    }

    #[test]
    fn post_roll_stalled_hardware_is_wall_bounded_without_invented_samples() {
        let completion = CaptureCompletion::default();
        assert_eq!(completion.end_reason(Duration::from_millis(599), false), None);
        assert_eq!(completion.end_reason(Duration::from_millis(600), false), Some(3));
        assert_eq!(completion.diagnostics().post_roll_frames, 0);
        assert_eq!(completion.diagnostics().post_roll_ms, 0.0);
    }

    #[test]
    fn post_roll_cancel_and_capture_error_end_the_wait_immediately() {
        let completion = CaptureCompletion::default();
        assert_eq!(completion.end_reason(Duration::ZERO, true), Some(5));
        completion.cancel.store(true, Ordering::Release);
        assert_eq!(completion.end_reason(Duration::ZERO, false), Some(4));
        assert_eq!(completion.mark_stop(Duration::from_millis(10)), Duration::from_millis(10));
        assert_eq!(completion.mark_stop(Duration::from_secs(1)), Duration::from_millis(10));
    }

    #[test]
    fn cue_suppression_changes_only_its_155_ms_window_at_every_sample_rate() {
        for rate in [16_000, 44_100, 48_000, 96_000] {
            let window = CueWindow::start_cue(u64::from(rate), rate);
            let source = vec![0.3; rate as usize * 2];
            for block in [1, 127, 1_024, source.len()] {
                let mut asr = source.clone();
                let mut first = 0;
                let mut suppressed = 0;
                for chunk in asr.chunks_mut(block) {
                    suppressed += window.suppress(chunk, first);
                    first += chunk.len() as u64;
                }
                assert_eq!(suppressed, milliseconds_to_frames(155, rate));
                assert!(asr[..window.start as usize].iter().all(|v| *v == 0.3));
                assert!(asr[window.start as usize..window.end as usize].iter().all(|v| *v == 0.0));
                assert!(asr[window.end as usize..].iter().all(|v| *v == 0.3));
                assert!(source.iter().all(|v| *v == 0.3));
            }
        }
    }

    #[test]
    fn delayed_cue_uses_playback_position_and_preserves_already_consumed_speech() {
        let mut state = CueState {
            clock: Some(CaptureClock { frame: 8_000, at: Duration::from_millis(500), rate: 16_000 }),
            ..CueState::default()
        };
        let mut preceding = vec![0.1; 9_600];
        state.filter(&mut preceding);
        assert!(preceding.iter().all(|v| *v == 0.1));
        // Thread creation was at capture start; output playback is at 750 ms.
        assert!(state.schedule(Duration::from_millis(750)));
        assert_eq!(state.window, Some(CueWindow { start: 12_000, end: 14_480 }));
        let mut next = vec![0.2; 8_000];
        state.filter(&mut next);
        assert!(next[..2_400].iter().all(|v| *v == 0.2));
        assert!(next[2_400..4_880].iter().all(|v| *v == 0.0));
        assert!(next[4_880..].iter().all(|v| *v == 0.2));
        assert_eq!(state.suppressed, 2_480);
    }

    #[test]
    fn cue_is_deferred_if_playback_would_reach_already_consumed_audio() {
        let mut state = CueState {
            clock: Some(CaptureClock { frame: 8_000, at: Duration::from_millis(500), rate: 16_000 }),
            consumed: 9_600, ..CueState::default()
        };
        assert!(!state.schedule(Duration::from_millis(550)));
        assert!(state.window.is_none());
        assert!(state.schedule(Duration::from_millis(600)));
        assert!(!state.schedule(Duration::from_millis(900))); // only one start cue
    }

    #[test]
    fn missing_cue_or_missing_capture_clock_never_mutes_audio() {
        let mut state = CueState::default();
        assert!(!state.schedule(Duration::from_secs(2)));
        let mut audio = [0.5; 512];
        state.filter(&mut audio);
        assert_eq!(audio, [0.5; 512]);
        assert_eq!(state.suppressed, 0);
        assert!(state.window.is_none());
    }

    #[test]
    fn consecutive_capture_sessions_have_independent_cue_and_post_roll_state() {
        let mut first = CueState {
            clock: Some(CaptureClock { frame: 0, at: Duration::ZERO, rate: 16_000 }),
            ..CueState::default()
        };
        assert!(first.schedule(Duration::ZERO));
        first.filter(&mut [0.2; 512]);
        let mut second = CueState::default();
        let mut audio = [0.8; 512];
        second.filter(&mut audio);
        assert_eq!(audio, [0.8; 512]);
        assert_eq!(second.suppressed, 0);
        assert!(second.window.is_none());
        let finished = replay_tail(16_000, &[0.0; 3_200], &[0.0; 3_200], 512);
        assert!(finished.reason.is_some());
        let mut next = PostRoll::new(16_000);
        assert_eq!(next.observe(&audio, false), audio.len());
        assert!(next.stop_frame.is_none());
        assert!(next.reason.is_none());
        assert_eq!(next.tail_frames(), 0);
    }

    #[test]
    fn cue_cancellation_after_playback_starts_stops_callbacks_and_owner() {
        for condition in ["stop", "cancel", "closed"] {
            let cue = StartCue { completion: Arc::new(CaptureCompletion::default()),
                stop: Arc::new(AtomicBool::new(false)), epoch: Instant::now() };
            let lifetime = ToneLifetime { epoch: Instant::now(), phase: AtomicU32::new(2),
                end_ns: AtomicU64::new(1_000_000_000) };
            assert!(lifetime.active(Some(&cue)));
            match condition {
                "stop" => cue.stop.store(true, Ordering::Release),
                "cancel" => cue.completion.cancel.store(true, Ordering::Release),
                _ => cue.completion.closed.store(true, Ordering::Release),
            }
            assert!(!lifetime.active(Some(&cue)), "{condition}");
            assert_eq!(lifetime.phase.load(Ordering::Acquire), 3);
            // Even a later callback without the capture handle cannot resume it.
            assert!(!lifetime.active(None));
        }
    }

    fn queue(policy: OverflowPolicy) -> (ChunkQueue, Receiver<AudioMessage>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (
            ChunkQueue {
                sender,
                policy,
                error_cause: Arc::new(AtomicU32::new(0)),
                pending_gap: None,
                terminal_gap: Arc::new(Mutex::new(None)),
            },
            receiver,
        )
    }

    fn assert_chunk(message: AudioMessage, at_ms: u64, frames: usize, dropped: u64) {
        match message {
            AudioMessage::Chunk {
                samples,
                sample_rate,
                captured_at,
                discontinuity,
                dropped_frames,
                ..
            } => {
                assert_eq!(sample_rate, 1000);
                assert_eq!(samples.len(), frames);
                assert_eq!(captured_at, Duration::from_millis(at_ms));
                assert_eq!(discontinuity, dropped != 0);
                assert_eq!(dropped_frames, dropped);
            }
            message => panic!("Expected an audio chunk, got {message:?}"),
        }
    }

    #[test]
    fn default_overflow_fails_closed_even_after_queue_drains() {
        assert_eq!(OverflowPolicy::default(), OverflowPolicy::Fail);
        let (mut queue, receiver) = queue(OverflowPolicy::default());
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::ZERO);
        queue.push(vec![0.0; 20], 1000, 0.0, Duration::from_millis(10));
        assert_eq!(queue.error_cause.load(Ordering::Acquire), ERROR_OVERFLOW);
        assert_chunk(receiver.try_recv().unwrap(), 0, 10, 0);
        queue.push(vec![0.0; 30], 1000, 0.0, Duration::from_millis(30));
        assert!(matches!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert_eq!(queue.pending_gap, None);
        assert_eq!(*queue.terminal_gap.lock().unwrap(), None);
    }

    #[test]
    fn meeting_overflow_retains_loss_until_a_chunk_is_queued() {
        let (mut queue, receiver) = queue(OverflowPolicy::Gap);
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::from_millis(500));
        queue.push(vec![0.0; 20], 1000, 0.0, Duration::from_millis(510));
        // This attempted send already carries a gap, but also fails. Both losses
        // must still appear on the next successful delivery.
        queue.push(vec![0.0; 30], 1000, 0.0, Duration::from_millis(530));
        assert_eq!(queue.error_cause.load(Ordering::Acquire), 0);
        assert_eq!(
            queue.pending_gap,
            Some(AudioGap {
                start: Duration::from_millis(510),
                end: Duration::from_millis(560),
                dropped_frames: 50,
                sample_rate: 1000,
            })
        );
        assert_chunk(receiver.try_recv().unwrap(), 500, 10, 0);
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::from_millis(560));
        assert_chunk(receiver.try_recv().unwrap(), 560, 10, 50);
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::from_millis(570));
        assert_chunk(receiver.try_recv().unwrap(), 570, 10, 0);
        let terminal = queue.terminal_gap.clone();
        drop(queue);
        assert_eq!(*terminal.lock().unwrap(), None);
    }

    #[test]
    fn stop_without_a_later_chunk_exposes_pending_gap_once() {
        let (mut queue, receiver) = queue(OverflowPolicy::Gap);
        let capture = Capture {
            stop: Arc::new(AtomicBool::new(false)),
            completion: Arc::new(CaptureCompletion::default()),
            epoch: Instant::now(),
            level: Arc::new(AtomicU32::new(0)),
            receiver: None,
            owner: None,
            pending_gap: queue.terminal_gap.clone(),
            error_cause: queue.error_cause.clone(),
        };
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::from_secs(8));
        queue.push(vec![0.0; 20], 1000, 0.0, Duration::from_millis(8010));
        queue.push(vec![0.0; 30], 1000, 0.0, Duration::from_millis(8030));
        capture.request_stop();
        assert!(capture.take_pending_gap().is_none());
        drop(queue); // Simulate destruction of the stream's data callback.
        assert_chunk(receiver.try_recv().unwrap(), 8000, 10, 0);
        assert_eq!(
            capture.take_pending_gap(),
            Some(AudioGap {
                start: Duration::from_millis(8010),
                end: Duration::from_millis(8060),
                dropped_frames: 50,
                sample_rate: 1000,
            })
        );
        assert!(capture.take_pending_gap().is_none());
    }

    #[test]
    fn disconnected_receiver_fails_both_policies() {
        for policy in [OverflowPolicy::Fail, OverflowPolicy::Gap] {
            let (mut queue, receiver) = queue(policy);
            drop(receiver);
            queue.push(vec![0.0; 10], 1000, 0.0, Duration::ZERO);
            assert_eq!(queue.error_cause.load(Ordering::Acquire), ERROR_DISCONNECTED);
        }
    }

    #[test]
    fn disconnect_after_overflow_preserves_the_undelivered_gap() {
        let (mut queue, receiver) = queue(OverflowPolicy::Gap);
        queue.push(vec![0.0; 10], 1000, 0.0, Duration::ZERO);
        queue.push(vec![0.0; 20], 1000, 0.0, Duration::from_millis(10));
        drop(receiver);
        queue.push(vec![0.0; 30], 1000, 0.0, Duration::from_millis(30));
        assert_eq!(queue.error_cause.load(Ordering::Acquire), ERROR_DISCONNECTED);
        let terminal = queue.terminal_gap.clone();
        drop(queue);
        assert_eq!(
            *terminal.lock().unwrap(),
            Some(AudioGap {
                start: Duration::from_millis(10),
                end: Duration::from_millis(30),
                dropped_frames: 20,
                sample_rate: 1000,
            })
        );
    }

    #[test]
    fn the_first_callback_error_cause_is_retained() {
        for first in [ERROR_STREAM, ERROR_OVERFLOW, ERROR_DISCONNECTED] {
            let cause = AtomicU32::new(0);
            fail_capture(&cause, first);
            for later in [ERROR_STREAM, ERROR_OVERFLOW, ERROR_DISCONNECTED] {
                fail_capture(&cause, later);
                assert_eq!(cause.load(Ordering::Acquire), first);
            }
        }
    }

    #[test]
    fn fallback_timestamps_include_dropped_frames_and_ignore_callback_jitter() {
        let (mut queue, receiver) = queue(OverflowPolicy::Gap);
        let mut timeline = CaptureTimeline::default();
        for (arrival_ms, frames) in [(120, 20), (4000, 30), (9000, 40)] {
            let captured_at = timeline.advance(
                Duration::from_millis(arrival_ms),
                frame_duration(frames, 1000),
                None,
                frames,
                1000,
            );
            queue.push(vec![0.0; frames as usize], 1000, 0.0, captured_at);
        }
        assert_chunk(receiver.try_recv().unwrap(), 100, 20, 0);
        let recovered_at = timeline.advance(
            Duration::from_secs(15),
            Duration::from_millis(10),
            None,
            10,
            1000,
        );
        queue.push(vec![0.0; 10], 1000, 0.0, recovered_at);
        assert_chunk(receiver.try_recv().unwrap(), 190, 10, 70);
        assert_eq!(timeline.anchor, Some(Duration::from_millis(100)));
        assert_eq!(timeline.captured_frames, 100);
    }

    #[test]
    fn device_timestamps_keep_the_original_epoch_anchor() {
        let mut timeline = CaptureTimeline::default();
        assert_eq!(
            timeline.advance(
                Duration::from_millis(505),
                Duration::from_millis(10),
                Some(Duration::ZERO),
                10,
                1000,
            ),
            Duration::from_millis(495)
        );
        assert_eq!(
            timeline.advance(
                Duration::from_secs(9),
                Duration::from_millis(10),
                Some(Duration::from_millis(250)),
                10,
                1000,
            ),
            Duration::from_millis(745)
        );
        let mut before_epoch = CaptureTimeline::default();
        assert_eq!(
            before_epoch.advance(
                Duration::from_millis(5),
                Duration::from_millis(10),
                None,
                10,
                1000,
            ),
            Duration::ZERO
        );
    }
}
