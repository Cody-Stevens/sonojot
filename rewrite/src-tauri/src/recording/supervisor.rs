//! Source recovery belongs to the recorder, never to dictation. Device opens run
//! off the writer thread; at most one open is pending for each source.
use super::*;
use super::issues::IssueCollector;

struct Chunk {
    samples: Vec<f32>,
    sample_rate: u32,
    at: Duration,
    discontinuity: bool,
    dropped_frames: u64,
}
enum InputEvent {
    Chunk(Chunk),
    Gap(SourceLoss),
    Issue(IssueKind),
    IssueAt(IssueKind, Duration),
    End,
}

#[derive(Clone, Copy)]
struct SourceLoss {
    kind: IssueKind,
    start: Duration,
    end: Duration,
    lost: Duration,
    occurrences: u64,
}
impl SourceLoss {
    fn extend(pending: &mut Option<Self>, kind: IssueKind, start: Duration, lost: Duration) {
        let end = start.saturating_add(lost);
        match pending {
            Some(gap) => {
                gap.start = gap.start.min(start);
                gap.end = gap.end.max(end);
                gap.lost = gap.lost.saturating_add(lost);
                gap.occurrences = gap.occurrences.saturating_add(1);
            }
            None => *pending = Some(Self { kind, start, end, lost, occurrences: 1 }),
        }
    }
}

trait Input: Send {
    fn poll(&mut self, now: Duration) -> Option<InputEvent>;
    fn stop(&mut self);
}
struct Opening {
    rx: Receiver<Result<Box<dyn Input>, String>>,
    stop: Arc<AtomicBool>,
    cancel_on_drop: bool,
}
impl Opening {
    fn try_recv(&mut self) -> Result<Result<Box<dyn Input>, String>, TryRecvError> {
        let result = self.rx.try_recv();
        if matches!(result, Ok(Ok(_))) {
            self.cancel_on_drop = false;
        }
        result
    }
}
impl Drop for Opening {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.stop.store(true, Ordering::Release);
        }
    }
}

// The same loop runs against real devices and a deterministic clock/source script.
trait Runtime {
    fn now(&self) -> Duration;
    fn idle(&mut self);
    fn open(&mut self, source: IssueSource, device: Option<String>, epoch: Instant) -> Opening;
}
struct NativeRuntime {
    epoch: Instant,
}
impl Runtime for NativeRuntime {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }
    fn idle(&mut self) {
        thread::sleep(Duration::from_millis(5));
    }
    fn open(&mut self, source: IssueSource, device: Option<String>, epoch: Instant) -> Opening {
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let open_stop = stop.clone();
        let spawn = thread::Builder::new()
            .name("meeting-source-open".into())
            .spawn(move || {
                let result = match source {
                    IssueSource::Microphone => MicrophoneInput::open(device, epoch, open_stop),
                    IssueSource::System => SystemInput::open(epoch, open_stop),
                };
                // A cancelled meeting drops rx. Dropping a late result stops that source.
                let _ = tx.send(result);
            });
        // A spawn failure disconnects rx, which follows the normal retry path.
        drop(spawn);
        Opening {
            rx,
            stop,
            cancel_on_drop: true,
        }
    }
}

struct MicrophoneInput {
    capture: Capture,
    rx: Receiver<AudioMessage>,
    terminal: Option<InputEvent>,
}
impl MicrophoneInput {
    fn open(
        device: Option<String>,
        epoch: Instant,
        stop: Arc<AtomicBool>,
    ) -> Result<Box<dyn Input>, String> {
        let mut capture = Capture::start_with_stop(device, epoch, OverflowPolicy::Gap, stop)?;
        let rx = capture.take_receiver()?;
        Ok(Box::new(Self {
            capture,
            rx,
            terminal: None,
        }))
    }
}
impl Input for MicrophoneInput {
    fn poll(&mut self, _: Duration) -> Option<InputEvent> {
        if let Some(event) = self.terminal.take() {
            return Some(event);
        }
        let event = match self.rx.try_recv() {
            Ok(AudioMessage::Chunk {
                samples,
                sample_rate,
                captured_at,
                discontinuity,
                dropped_frames,
                ..
            }) => Some(InputEvent::Chunk(Chunk {
                samples,
                sample_rate,
                at: captured_at,
                discontinuity,
                dropped_frames,
            })),
            Ok(AudioMessage::Error(_)) => Some(InputEvent::Issue(IssueKind::Disconnected)),
            Ok(AudioMessage::End) | Err(TryRecvError::Disconnected) => Some(InputEvent::End),
            Err(TryRecvError::Empty) if self.capture.failure_cause() != 0 => {
                Some(InputEvent::Issue(IssueKind::Disconnected))
            }
            Err(TryRecvError::Empty) => None,
        };
        if matches!(event, Some(InputEvent::End | InputEvent::Issue(_))) {
            if let Some(gap) = self.capture.take_pending_gap() {
                self.terminal = event;
                return Some(InputEvent::Gap(SourceLoss {
                    kind: IssueKind::Overflow,
                    start: gap.start,
                    end: gap.end,
                    lost: Duration::from_secs_f64(gap.dropped_frames as f64 / gap.sample_rate as f64),
                    occurrences: 1,
                }));
            }
        }
        event
    }
    fn stop(&mut self) {
        self.capture.stop();
    }
}

struct SystemInput {
    capture: SystemAudioCapture,
    queue: SystemReceiver,
    epoch: Instant,
}

#[derive(Default)]
struct AtomicLoss {
    start: AtomicU64,
    end: AtomicU64,
    lost: AtomicU64,
    occurrences: AtomicU64,
}
impl AtomicLoss {
    fn publish(&self, loss: Option<SourceLoss>) {
        if let Some(loss) = loss {
            let nanos = |time: Duration| time.as_nanos().min(u64::MAX as u128) as u64;
            self.start.store(nanos(loss.start), Ordering::Relaxed);
            self.end.store(nanos(loss.end), Ordering::Relaxed);
            self.lost.store(nanos(loss.lost), Ordering::Relaxed);
            self.occurrences.store(loss.occurrences, Ordering::Release);
        } else {
            self.occurrences.store(0, Ordering::Release);
        }
    }

    // Only read after the native owner publishes a failure or finishes. There
    // can then be no concurrent callback and no partially published interval.
    fn read_after_capture(&self, kind: IssueKind) -> Option<SourceLoss> {
        let occurrences = self.occurrences.load(Ordering::Acquire);
        (occurrences != 0).then(|| SourceLoss {
            kind,
            start: Duration::from_nanos(self.start.load(Ordering::Relaxed)),
            end: Duration::from_nanos(self.end.load(Ordering::Relaxed)),
            lost: Duration::from_nanos(self.lost.load(Ordering::Relaxed)),
            occurrences,
        })
    }
}

#[derive(Default)]
struct SystemSignals {
    overflow: AtomicLoss,
    invalid: AtomicLoss,
}
struct SystemPacket {
    chunk: Chunk,
    overflow: Option<SourceLoss>,
    invalid: Option<SourceLoss>,
}
struct SystemQueue {
    tx: mpsc::SyncSender<SystemPacket>,
    signals: Arc<SystemSignals>,
    overflow: Option<SourceLoss>,
    invalid: Option<SourceLoss>,
}
impl SystemQueue {
    fn push(&mut self, chunk: SystemAudioChunk) {
        let valid_rate = (8_000..=384_000).contains(&chunk.sample_rate);
        let duration = if valid_rate {
            Duration::from_secs_f64(chunk.samples.len() as f64 / chunk.sample_rate as f64)
        } else {
            Duration::ZERO
        };
        if !valid_rate || chunk.samples.len() > chunk.sample_rate as usize * MAX_CHUNK_SECONDS {
            SourceLoss::extend(&mut self.invalid, IssueKind::InvalidBuffer, chunk.captured_at, duration);
            self.signals.invalid.publish(self.invalid);
            return;
        }
        if chunk.samples.is_empty() {
            return;
        }
        let at = chunk.captured_at;
        let packet = SystemPacket {
            chunk: Chunk {
                samples: chunk.samples,
                sample_rate: chunk.sample_rate,
                at,
                discontinuity: chunk.discontinuity,
                dropped_frames: 0,
            },
            overflow: self.overflow,
            invalid: self.invalid,
        };
        match self.tx.try_send(packet) {
            Ok(()) => {
                self.overflow = None;
                self.invalid = None;
                self.signals.overflow.publish(None);
                self.signals.invalid.publish(None);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                SourceLoss::extend(&mut self.overflow, IssueKind::Overflow, at, duration);
                self.signals.overflow.publish(self.overflow);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {}
        }
    }
}

struct SystemReceiver {
    rx: Receiver<SystemPacket>,
    signals: Arc<SystemSignals>,
    // At most two loss records and one packet/terminal event are staged.
    pending: VecDeque<InputEvent>,
    ended: bool,
}
impl SystemReceiver {
    fn losses(&mut self, overflow: Option<SourceLoss>, invalid: Option<SourceLoss>) {
        for loss in [overflow, invalid].into_iter().flatten() {
            self.pending.push_back(InputEvent::Gap(loss));
        }
    }

    fn poll(&mut self, failure_at: Option<Duration>, finished: bool) -> Option<InputEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        if self.ended {
            return Some(InputEvent::End);
        }
        if let Ok(packet) = self.rx.try_recv() {
            self.losses(packet.overflow, packet.invalid);
            self.pending.push_back(InputEvent::Chunk(packet.chunk));
            return self.pending.pop_front();
        }
        if failure_at.is_some() || finished {
            // failure_at/finished was acquired before reading the queue. All
            // packets precede the terminal snapshots, even with delayed polling.
            self.losses(
                self.signals.overflow.read_after_capture(IssueKind::Overflow),
                self.signals.invalid.read_after_capture(IssueKind::InvalidBuffer),
            );
            if let Some(at) = failure_at {
                self.pending.push_back(InputEvent::IssueAt(IssueKind::Disconnected, at));
            }
            self.ended = true;
            return self.pending.pop_front().or(Some(InputEvent::End));
        }
        None
    }
}
impl SystemInput {
    fn open(epoch: Instant, stop: Arc<AtomicBool>) -> Result<Box<dyn Input>, String> {
        let (tx, rx) = mpsc::sync_channel(512);
        let signals = Arc::new(SystemSignals::default());
        let mut queue = SystemQueue { tx, signals: signals.clone(), overflow: None, invalid: None };
        let capture = SystemAudioCapture::start_with_stop(
            epoch,
            stop,
            move |chunk| queue.push(chunk),
            |_| {},
        )?;
        Ok(Box::new(Self {
            capture,
            queue: SystemReceiver { rx, signals, pending: VecDeque::new(), ended: false },
            epoch,
        }))
    }
}
impl Input for SystemInput {
    fn poll(&mut self, _: Duration) -> Option<InputEvent> {
        let finished = self.capture.is_finished();
        let event = self.queue.poll(self.capture.failure_at(), finished);
        if matches!(event, Some(InputEvent::IssueAt(IssueKind::Disconnected, _))) {
            // Diagnostics stay on the recorder owner, outside WASAPI callbacks
            // and native cleanup. Only the numeric cause crosses that boundary.
            let kind = match self.capture.failure_cause() {
                2 => "system-output-changed",
                3 => "system-output-unavailable",
                4 => "system-audio-stalled",
                5 => "system-audio-permission-revoked",
                _ => "system-audio-error",
            };
            capture_event(kind, 0, self.epoch, 1);
        }
        event
    }
    fn stop(&mut self) {
        self.capture.request_stop();
    }
}

const PERMISSION_NOTICE: &str = "System audio is off. Allow it in System Settings.";

struct SourceState {
    source: IssueSource,
    enabled: bool,
    input: Option<Box<dyn Input>>,
    opening: Option<Opening>,
    next_retry: Duration,
    retries: u32,
    down_since: Option<Duration>,
    fallback: bool,
    gap: Option<SourceIssue>,
    reset_segment: bool,
    last_end: Duration,
    last_loss_end: Duration,
    received: bool,
    restarting: bool,
    /// The last system open was refused for lack of macOS permission.
    permission_blocked: bool,
}
impl SourceState {
    fn new(source: IssueSource, enabled: bool) -> Self {
        Self {
            source,
            enabled,
            input: None,
            opening: None,
            next_retry: Duration::ZERO,
            retries: 0,
            down_since: None,
            fallback: false,
            gap: None,
            reset_segment: false,
            last_end: Duration::ZERO,
            last_loss_end: Duration::ZERO,
            received: false,
            restarting: false,
            permission_blocked: false,
        }
    }
    fn system_problem(&self, notices: &SharedNotices, problem: Option<SystemProblem>) {
        if self.source == IssueSource::System {
            notices.lock().unwrap_or_else(|p| p.into_inner()).system_problem = problem;
        }
    }
    fn notice(&self, notices: &SharedNotices, message: String, recovered: bool) {
        notices.lock().unwrap_or_else(|p| p.into_inner()).notices[source_index(self.source)] =
            Some((
                message,
                recovered.then(|| Instant::now() + Duration::from_secs(5)),
            ));
    }
    fn begin_gap(
        &mut self,
        kind: IssueKind,
        at: Duration,
        _issues: &mut IssueCollector,
        notices: &SharedNotices,
    ) {
        if self.gap.is_some() {
            return;
        }
        let label = self.source.label();
        let action = match kind {
            IssueKind::Disconnected if self.permission_blocked => "was not allowed by macOS",
            IssueKind::Disconnected => "disconnected",
            IssueKind::Overflow => "queue overflowed",
            IssueKind::InvalidBuffer => "returned an invalid audio buffer",
            IssueKind::Discontinuity => "had an audio gap",
        };
        self.gap = Some(SourceIssue {
            source: self.source,
            started_at_seconds: at.as_secs_f64(),
            ended_at_seconds: None,
            kind,
            occurrences: 1,
            lost_duration_seconds: 0.0,
            message: format!(
                "{label} {action} at {}; recording continued.",
                clock_text(at.as_secs_f64())
            ),
        });
        self.notice(
            notices,
            if kind == IssueKind::Disconnected && self.permission_blocked {
                PERMISSION_NOTICE.to_string()
            } else if kind == IssueKind::Disconnected {
                format!("{label} lost — reconnecting…")
            } else {
                format!("{label} has an audio gap. Recording continues.")
            },
            false,
        );
        if kind == IssueKind::Disconnected {
            self.system_problem(
                notices,
                Some(if self.permission_blocked { SystemProblem::Permission } else { SystemProblem::Lost }),
            );
        }
    }
    fn close_gap(
        &mut self,
        at: Duration,
        issues: &mut IssueCollector,
        notices: &SharedNotices,
        epoch: Instant,
    ) {
        let Some(mut issue) = self.gap.take() else { return };
        let end = at.as_secs_f64().max(issue.started_at_seconds);
        issue.ended_at_seconds = Some(end);
        issue.lost_duration_seconds = end - issue.started_at_seconds;
        let destination = if self.fallback {
            "the default microphone"
        } else {
            self.source.label()
        };
        issue.message.push_str(&format!(
            " {} at {}.",
            if self.restarting {
                format!("Reconnected to {destination}")
            } else {
                format!("{} resumed", self.source.label())
            },
            clock_text(end)
        ));
        capture_event(
            "meeting-gap",
            0,
            epoch,
            ((end - issue.started_at_seconds) * 1000.0).round() as u64,
        );
        issues.push(issue);
        if self.restarting && self.source == IssueSource::Microphone {
            capture_event("microphone-reopened", 0, epoch, u64::from(self.fallback));
        }
        self.notice(
            notices,
            format!("{} reconnected", self.source.label()),
            true,
        );
        self.system_problem(notices, None);
        self.permission_blocked = false;
        self.restarting = false;
        self.down_since = None;
        self.retries = 0;
    }
    fn lost(&mut self, now: Duration, issues: &mut IssueCollector, notices: &SharedNotices) {
        self.begin_gap(
            IssueKind::Disconnected,
            // Silent loopback legitimately emits no packets. Do not label all
            // of that earlier quiet time as a device failure.
            if self.received && self.source == IssueSource::Microphone {
                self.last_end.max(self.last_loss_end)
            } else {
                now
            },
            issues,
            notices,
        );
        self.down_since.get_or_insert(now);
        self.restarting = true;
        if let Some(mut input) = self.input.take() {
            input.stop();
        }
        self.next_retry = now + Duration::from_secs(1);
    }
    fn supervise(
        &mut self,
        runtime: &mut impl Runtime,
        microphone: &Option<String>,
        epoch: Instant,
        issues: &mut IssueCollector,
        notices: &SharedNotices,
    ) {
        if !self.enabled {
            return;
        }
        let now = runtime.now();
        if let Some(opening) = &mut self.opening {
            let failed = match opening.try_recv() {
                Ok(Ok(input)) => {
                    self.input = Some(input);
                    self.opening = None;
                    None
                }
                Ok(Err(error)) => Some(Some(error)),
                Err(TryRecvError::Disconnected) => Some(None),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(error) = failed {
                self.opening = None;
                let permission = self.source == IssueSource::System
                    && error.as_deref().is_some_and(crate::system_audio::is_permission_error);
                if permission && !self.permission_blocked {
                    capture_event("system-audio-permission", 0, epoch, 0);
                }
                if permission {
                    // A loss that turned into a refusal: say why it continues.
                    let was_blocked = self.permission_blocked;
                    if let Some(issue) = self.gap.as_mut().filter(|_| !was_blocked) {
                        issue.message.push_str(" System audio permission is off.");
                    }
                    self.notice(notices, PERMISSION_NOTICE.into(), false);
                }
                self.permission_blocked = permission;
                self.begin_gap(IssueKind::Disconnected, self.last_end, issues, notices);
                if permission {
                    self.system_problem(notices, Some(SystemProblem::Permission));
                }
                self.restarting = true;
                self.down_since.get_or_insert(now);
                self.retries = self.retries.saturating_add(1);
                // A refused permission joins within a second of being allowed.
                self.next_retry = now + if permission {
                    Duration::from_secs(1)
                } else {
                    Duration::from_millis((1000 + u64::from(self.retries) * 500).min(3000))
                };
                if self.source == IssueSource::Microphone
                    && microphone.as_ref().is_some_and(|id| !id.is_empty())
                    && !self.fallback
                {
                    self.next_retry = self
                        .next_retry
                        .min(self.down_since.unwrap_or(now) + Duration::from_secs(5));
                }
            }
        }
        // Cancel a stalled selected-device open at the fallback deadline. The
        // cancelled owner checks Stop before starting a stream if it wakes.
        if self.source == IssueSource::Microphone
            && self.input.is_none()
            && !self.fallback
            && microphone.as_ref().is_some_and(|id| !id.is_empty())
            && self
                .down_since
                .is_some_and(|since| now.saturating_sub(since) >= Duration::from_secs(5))
        {
            self.opening = None;
            self.begin_gap(IssueKind::Disconnected, self.last_end, issues, notices);
            self.fallback = true;
            self.restarting = true;
            self.next_retry = now;
            if let Some(issue) = &mut self.gap {
                issue
                    .message
                    .push_str(" Trying the current default microphone.");
            }
        }
        if self.input.is_none() && self.opening.is_none() && now >= self.next_retry {
            // A refused permission retries every second; log only real restarts.
            if !self.permission_blocked {
                capture_event(
                    if self.source == IssueSource::Microphone {
                        "microphone-reopen-attempt"
                    } else {
                        "system-audio-restart"
                    },
                    0,
                    epoch,
                    u64::from(self.fallback),
                );
            }
            self.opening = Some(runtime.open(
                self.source,
                if self.fallback {
                    None
                } else {
                    microphone.clone()
                },
                epoch,
            ));
            self.down_since.get_or_insert(now);
        }
    }
    fn drain(
        &mut self,
        now: Duration,
        mixer: &mut Mixer,
        issues: &mut IssueCollector,
        notices: &SharedNotices,
        epoch: Instant,
        stopping: bool,
        fatal: &SharedError,
    ) {
        for _ in 0..512 {
            let Some(event) = self.input.as_mut().and_then(|input| input.poll(now)) else {
                break;
            };
            match event {
                InputEvent::Gap(loss) => {
                    if self.source == IssueSource::Microphone {
                        capture_event("microphone-error", 0, epoch, 2);
                    }
                    // This loss has exact boundaries. Record it independently of
                    // a later disconnect, and do not extend it across quiet time.
                    issues.push(SourceIssue {
                        source: self.source,
                        kind: loss.kind,
                        started_at_seconds: loss.start.as_secs_f64(),
                        ended_at_seconds: Some(loss.end.as_secs_f64()),
                        occurrences: loss.occurrences,
                        lost_duration_seconds: loss.lost.as_secs_f64(),
                        message: format!(
                            "{} {} at {} to {}; {:.3} seconds of audio lost; recording continued.",
                            self.source.label(),
                            if loss.kind == IssueKind::Overflow { "queue overflowed" }
                                else { "returned an invalid audio buffer" },
                            clock_text(loss.start.as_secs_f64()),
                            clock_text(loss.end.as_secs_f64()),
                            loss.lost.as_secs_f64(),
                        ),
                    });
                    self.reset_segment = true;
                    self.last_loss_end = self.last_loss_end.max(loss.end);
                    self.notice(
                        notices,
                        format!("{} has an audio gap. Recording continues.", self.source.label()),
                        false,
                    );
                    capture_event(
                        "meeting-gap",
                        0,
                        epoch,
                        loss.lost.as_millis() as u64,
                    );
                }
                InputEvent::Chunk(chunk) => {
                    if chunk.samples.is_empty() {
                        continue;
                    }
                    if !(8_000..=384_000).contains(&chunk.sample_rate)
                        || chunk.samples.len() > chunk.sample_rate as usize * MAX_CHUNK_SECONDS
                    {
                        self.begin_gap(IssueKind::InvalidBuffer, self.last_end, issues, notices);
                        continue;
                    }
                    // Quiet WASAPI output legitimately sends no packets; only an
                    // explicit discontinuity marks a system gap. Microphones also
                    // expose timestamp jumps when a driver omits a callback.
                    let jumped = self.source == IssueSource::Microphone
                        && self.received
                        && chunk.at > self.last_end.max(self.last_loss_end)
                            .saturating_add(Duration::from_millis(20));
                    // A queue loss resets resampling, but does not explain a
                    // separate native discontinuity on the following packet.
                    if chunk.discontinuity || chunk.dropped_frames > 0 || jumped {
                        if chunk.dropped_frames > 0 && self.source == IssueSource::Microphone {
                            capture_event("microphone-error", 0, epoch, 2);
                        }
                        let start = if chunk.dropped_frames > 0 {
                            chunk.at.saturating_sub(Duration::from_secs_f64(
                                chunk.dropped_frames as f64 / chunk.sample_rate as f64,
                            ))
                        } else {
                            self.last_end.max(self.last_loss_end).min(chunk.at)
                        };
                        self.begin_gap(
                            if chunk.dropped_frames > 0 {
                                IssueKind::Overflow
                            } else {
                                IssueKind::Discontinuity
                            },
                            start,
                            issues,
                            notices,
                        );
                    }
                    let restart = self.gap.is_some() || chunk.discontinuity || self.reset_segment;
                    match mixer.push(
                        self.source == IssueSource::Microphone,
                        &chunk.samples,
                        chunk.sample_rate,
                        chunk.at,
                        restart,
                    ) {
                        Ok(()) => {
                            self.close_gap(chunk.at, issues, notices, epoch);
                            if self.reset_segment {
                                self.notice(notices, format!("{} reconnected", self.source.label()), true);
                            }
                            self.reset_segment = false;
                            self.last_end = chunk.at.saturating_add(Duration::from_secs_f64(
                                chunk.samples.len() as f64 / chunk.sample_rate as f64,
                            ));
                            self.received = true;
                            self.down_since = None;
                            self.retries = 0;
                            // Persist repairs per chunk so a long queued backlog
                            // cannot overrun the bounded in-memory repair window.
                            if let Err(message) = mixer.repair_late_samples() {
                                report(fatal, message);
                            }
                        }
                        Err(_) => {
                            self.begin_gap(
                                IssueKind::InvalidBuffer,
                                self.last_end,
                                issues,
                                notices,
                            );
                            // Keep queued good samples, but do not reuse a bad timestamp/filter.
                            let track = if self.source == IssueSource::Microphone {
                                &mut mixer.microphone
                            } else {
                                &mut mixer.system
                            };
                            track.resampler = None;
                        }
                    }
                }
                InputEvent::Issue(IssueKind::Disconnected)
                | InputEvent::IssueAt(IssueKind::Disconnected, _) | InputEvent::End => {
                    let failed_at = match event {
                        InputEvent::IssueAt(_, at) => at,
                        _ => now,
                    };
                    if !stopping
                        && matches!(event, InputEvent::End)
                        && self.source == IssueSource::Microphone
                    {
                        capture_event("microphone-error", 0, epoch, 3);
                    }
                    if stopping {
                        if matches!(event, InputEvent::IssueAt(_, _)) {
                            self.begin_gap(IssueKind::Disconnected, failed_at, issues, notices);
                        }
                        self.input = None;
                    } else {
                        self.lost(failed_at, issues, notices);
                    }
                    break;
                }
                InputEvent::Issue(kind) => self.begin_gap(kind, self.last_end, issues, notices),
                InputEvent::IssueAt(kind, at) => self.begin_gap(kind, at, issues, notices),
            }
        }
    }
}

pub(super) fn record(
    writers: Writers,
    source: Source,
    microphone: Option<String>,
    stop: &AtomicBool,
    error: &SharedError,
    level: Arc<AtomicU32>,
    stems: StemLevels,
    notices: &SharedNotices,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<RecordingOutcome, String> {
    let epoch = Instant::now();
    // A missing permission is recoverable mid-meeting; an unsupported OS is not.
    if source.system()
        && crate::system_audio::status() == crate::system_audio::SystemAudioStatus::Unsupported
    {
        let message = crate::system_audio::UNSUPPORTED_ERROR.to_string();
        let _ = ready.send(Err(message.clone()));
        return Err(message);
    }
    record_with(
        writers,
        source,
        microphone,
        stop,
        error,
        level,
        stems,
        notices,
        ready,
        epoch,
        &mut NativeRuntime { epoch },
    )
}

fn record_with(
    writers: Writers,
    source: Source,
    microphone: Option<String>,
    stop: &AtomicBool,
    error: &SharedError,
    level: Arc<AtomicU32>,
    stems: StemLevels,
    notices: &SharedNotices,
    ready: mpsc::SyncSender<Result<(), String>>,
    epoch: Instant,
    runtime: &mut impl Runtime,
) -> Result<RecordingOutcome, String> {
    let mut mixer = Mixer::new(writers, source, level);
    mixer.stems = stems;
    let mut sources = [
        SourceState::new(IssueSource::Microphone, source.microphone()),
        SourceState::new(IssueSource::System, source.system()),
    ];
    let mut issues = IssueCollector::default();
    // A missing device is recoverable even at startup. Ready means the writer and
    // supervisors are active, not that both devices must already be connected.
    let _ = ready.send(Ok(()));
    let mut checkpoint_at = Duration::ZERO;
    while !stop.load(Ordering::Acquire) && current_error(error).is_none() {
        for state in &mut sources {
            state.supervise(runtime, &microphone, epoch, &mut issues, notices);
            state.drain(
                runtime.now(),
                &mut mixer,
                &mut issues,
                notices,
                epoch,
                false,
                error,
            );
        }
        if let Err(message) =
            mixer.write_until(frames(runtime.now().saturating_sub(REORDER_DELAY)), false)
        {
            report(error, message);
        }
        if runtime.now().saturating_sub(checkpoint_at) >= Duration::from_secs(1) {
            if let Err(message) = mixer.writers.checkpoint() {
                report(error, message);
            }
            checkpoint_at = runtime.now();
        }
        runtime.idle();
    }
    let stop_at = runtime.now();
    for state in &mut sources {
        if let Some(opening) = &mut state.opening {
            if let Ok(Ok(input)) = opening.try_recv() {
                state.input = Some(input);
            }
        }
        state.opening = None;
        if let Some(input) = &mut state.input {
            input.stop();
        }
    }
    let deadline = stop_at + Duration::from_secs(5);
    while sources.iter().any(|state| state.input.is_some()) && runtime.now() < deadline {
        for state in &mut sources {
            state.drain(
                runtime.now(),
                &mut mixer,
                &mut issues,
                notices,
                epoch,
                true,
                error,
            );
        }
        if sources.iter().any(|state| state.input.is_some()) {
            runtime.idle();
        }
    }
    for state in &mut sources {
        if state.input.is_some() {
            state.begin_gap(
                IssueKind::Disconnected,
                state.last_end,
                &mut issues,
                notices,
            );
        }
        state.input = None;
    }
    // A failed file must not discard the other writers' queued audio or tails.
    if let Err(message) = mixer.flush_resamplers() {
        report(error, message);
    }
    let end = frames(stop_at).max(mixer.last_frame());
    if let Err(message) = mixer.write_until(end, true) {
        report(error, message);
    }
    let received_audio = sources.iter().any(|state| state.received);
    if !received_audio {
        // Timeline placeholders alone are not usable captured audio. Leave valid
        // empty WAVs so the meeting layer does not mistake them for a recovery.
        for writer in [
            &mut mixer.writers.audio,
            &mut mixer.writers.microphone,
            &mut mixer.writers.system,
        ] {
            writer.frames = 0;
            if let Err(message) = retry_io(|| writer.writer.set_len(WAV_HEADER_BYTES)) {
                report(
                    error,
                    format!("Cannot finalize empty meeting audio: {message}"),
                );
            }
        }
    }
    if let Err(message) = mixer.writers.checkpoint() {
        report(error, message);
    }
    let duration = mixer.writers.audio.frames as f64 / RATE as f64;
    for state in &mut sources {
        if let Some(mut issue) = state.gap.take() {
            issue.lost_duration_seconds = (duration - issue.started_at_seconds).max(0.0);
            capture_event(
                "meeting-gap",
                0,
                epoch,
                (issue.lost_duration_seconds * 1000.0).round() as u64,
            );
            issues.push(issue);
        }
    }
    let issues = issues.finish();
    // A writer error still returns Err below. Retain structured source loss
    // alongside the diagnostic strings for partial-recording recovery.
    let saved_issues = notices.lock().unwrap_or_else(|p| p.into_inner()).issues.clone();
    *saved_issues.lock().unwrap_or_else(|p| p.into_inner()) = issues.clone();
    let warnings = notices
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .warnings
        .clone();
    *warnings.lock().unwrap_or_else(|p| p.into_inner()) =
        issues.iter().map(|issue| issue.message.clone()).collect();
    match current_error(error) {
        Some(message) => Err(message),
        None if !received_audio => {
            Err("No audio was received. Check the meeting sources and try again.".into())
        }
        None => Ok(RecordingOutcome { duration, issues }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{read_wav, TestFiles};
    use super::*;

    #[test]
    fn silent_loopback_before_a_disconnect_is_not_marked_as_lost_audio() {
        let mut state = SourceState::new(IssueSource::System, true);
        state.received = true;
        state.last_end = Duration::from_millis(500);
        let mut issues = IssueCollector::default();
        state.lost(
            Duration::from_secs(30),
            &mut issues,
            &Arc::new(Mutex::new(SourceNotices::default())),
        );
        assert_eq!(state.gap.as_ref().unwrap().started_at_seconds, 30.0);
        assert_eq!(state.gap.as_ref().unwrap().ended_at_seconds, None);
    }

    fn system_queue() -> (SystemQueue, SystemReceiver) {
        let (tx, rx) = mpsc::sync_channel(1);
        let signals = Arc::new(SystemSignals::default());
        (
            SystemQueue { tx, signals: signals.clone(), overflow: None, invalid: None },
            SystemReceiver { rx, signals, pending: VecDeque::new(), ended: false },
        )
    }
    fn system_packet(millis: u64) -> SystemAudioChunk {
        SystemAudioChunk {
            samples: vec![0.2; BLOCK],
            sample_rate: RATE,
            captured_at: Duration::from_millis(millis),
            discontinuity: false,
        }
    }

    #[test]
    fn system_queue_reports_exact_overflow_before_recovery_after_quiet_time() {
        let (mut queue, mut receiver) = system_queue();
        queue.push(system_packet(0));
        queue.push(system_packet(50));
        queue.push(system_packet(100));
        assert!(matches!(receiver.poll(None, false), Some(InputEvent::Chunk(chunk)) if chunk.at == Duration::ZERO));
        queue.push(system_packet(6000));
        let Some(InputEvent::Gap(loss)) = receiver.poll(None, false) else { panic!("missing overflow") };
        assert_eq!(loss.kind, IssueKind::Overflow);
        assert_eq!(loss.start, Duration::from_millis(50));
        assert_eq!(loss.end, Duration::from_millis(150));
        assert_eq!(loss.lost, Duration::from_millis(100));
        assert_eq!(loss.occurrences, 2);
        assert!(matches!(receiver.poll(None, false), Some(InputEvent::Chunk(chunk))
            if chunk.at == Duration::from_secs(6) && chunk.dropped_frames == 0));
        let mut invalid = system_packet(6100);
        invalid.sample_rate = 0;
        queue.push(invalid);
        queue.push(system_packet(6200));
        assert!(matches!(receiver.poll(None, false), Some(InputEvent::Gap(loss))
            if loss.kind == IssueKind::InvalidBuffer && loss.start == Duration::from_millis(6100)));
        assert!(matches!(receiver.poll(None, false), Some(InputEvent::Chunk(_))));
        // Neither delivered loss may be reported again by the atomic tail.
        assert!(matches!(receiver.poll(None, true), Some(InputEvent::End)));
    }

    #[test]
    fn system_overflow_keeps_an_independent_native_discontinuity_on_recovery() {
        struct DrainingSystemInput(SystemReceiver);
        impl Input for DrainingSystemInput {
            fn poll(&mut self, _: Duration) -> Option<InputEvent> { self.0.poll(None, true) }
            fn stop(&mut self) {}
        }
        let (mut queue, mut receiver) = system_queue();
        queue.push(system_packet(0));
        queue.push(system_packet(50)); // Queue overflow covers 50..100 ms.
        let first = receiver.poll(None, false).unwrap();
        let mut recovered = system_packet(150);
        recovered.discontinuity = true; // Native loss also covers 100..150 ms.
        queue.push(recovered);
        receiver.pending.push_front(first);
        let mut state = SourceState::new(IssueSource::System, true);
        state.input = Some(Box::new(DrainingSystemInput(receiver)));
        let files = TestFiles::new();
        let mut mixer = Mixer::new(Writers::create(&files.paths).unwrap(), Source::System,
            Arc::new(AtomicU32::new(0)));
        let mut issues = IssueCollector::default();
        state.drain(Duration::from_secs(1), &mut mixer, &mut issues,
            &Arc::new(Mutex::new(SourceNotices::default())), Instant::now(), true,
            &Arc::new(Mutex::new(None)));
        let issues = issues.finish();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].kind, IssueKind::Overflow);
        assert_eq!(issues[0].started_at_seconds, 0.05);
        assert_eq!(issues[0].ended_at_seconds, Some(0.1));
        assert_eq!(issues[1].kind, IssueKind::Discontinuity);
        assert_eq!(issues[1].started_at_seconds, 0.1);
        assert_eq!(issues[1].ended_at_seconds, Some(0.15));
        assert!((issues.iter().map(|issue| issue.lost_duration_seconds).sum::<f64>() - 0.1).abs() < 1e-9);
    }

    struct FinishedSystemInput {
        receiver: SystemReceiver,
        failure_at: Duration,
    }
    impl Input for FinishedSystemInput {
        fn poll(&mut self, _: Duration) -> Option<InputEvent> {
            self.receiver.poll(Some(self.failure_at), true)
        }
        fn stop(&mut self) {}
    }

    #[test]
    fn delayed_poll_keeps_atomic_overflow_and_disconnect_at_the_source_times() {
        let (mut queue, receiver) = system_queue();
        queue.push(system_packet(0));
        queue.push(system_packet(50));
        queue.push(system_packet(200)); // quiet output between the two dropped packets
        let mut invalid = system_packet(2500);
        invalid.sample_rate = 0;
        queue.push(invalid);
        let mut state = SourceState::new(IssueSource::System, true);
        state.input = Some(Box::new(FinishedSystemInput {
            receiver,
            failure_at: Duration::from_millis(3250),
        }));
        let files = TestFiles::new();
        let mut mixer = Mixer::new(Writers::create(&files.paths).unwrap(), Source::System,
            Arc::new(AtomicU32::new(0)));
        let mut issues = IssueCollector::default();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let epoch = Instant::now();
        state.drain(Duration::from_secs(30), &mut mixer, &mut issues, &notices,
            epoch, false, &Arc::new(Mutex::new(None)));
        assert!(state.input.is_none());
        assert_eq!(state.gap.as_ref().unwrap().kind, IssueKind::Disconnected);
        assert_eq!(state.gap.as_ref().unwrap().started_at_seconds, 3.25);
        state.close_gap(Duration::from_secs(31), &mut issues, &notices, epoch);
        let issues = issues.finish();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].kind, IssueKind::Overflow);
        assert_eq!(issues[0].started_at_seconds, 0.05);
        assert_eq!(issues[0].ended_at_seconds, Some(0.25));
        assert_eq!(issues[0].lost_duration_seconds, 0.1);
        assert_eq!(issues[0].occurrences, 2);
        assert_eq!(issues[1].kind, IssueKind::InvalidBuffer);
        assert_eq!(issues[1].started_at_seconds, 2.5);
        assert_eq!(issues[1].lost_duration_seconds, 0.0);
        assert_eq!(issues[2].kind, IssueKind::Disconnected);
        assert_eq!(issues[2].started_at_seconds, 3.25);
        assert_eq!(issues[2].ended_at_seconds, Some(31.0));
    }

    #[test]
    fn active_gap_survives_thousands_of_other_source_compactions() {
        let mut state = SourceState::new(IssueSource::Microphone, true);
        let mut issues = IssueCollector::default();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        state.begin_gap(IssueKind::Disconnected, Duration::ZERO, &mut issues, &notices);
        for index in 0..10_000 {
            issues.push(SourceIssue {
                source: IssueSource::System,
                kind: IssueKind::Discontinuity,
                started_at_seconds: index as f64 * 3.0,
                ended_at_seconds: Some(index as f64 * 3.0 + 0.125),
                occurrences: 1,
                lost_duration_seconds: 0.125,
                message: "System audio gap.".into(),
            });
        }
        state.close_gap(Duration::from_secs(30_000), &mut issues, &notices, Instant::now());
        let issues = issues.finish();
        assert!(issues.len() <= 58);
        let microphone = issues.iter().find(|issue| issue.source == IssueSource::Microphone).unwrap();
        assert_eq!(microphone.started_at_seconds, 0.0);
        assert_eq!(microphone.ended_at_seconds, Some(30_000.0));
        assert_eq!(microphone.lost_duration_seconds, 30_000.0);
        assert_eq!(issues.iter().filter(|issue| issue.source == IssueSource::System)
            .map(|issue| issue.occurrences).sum::<u64>(), 10_000);
    }

    #[test]
    fn terminal_microphone_overflow_is_not_counted_again_as_disconnect_time() {
        let mut state = SourceState::new(IssueSource::Microphone, true);
        state.received = true;
        state.last_end = Duration::from_secs(1);
        state.input = Some(Box::new(ScriptInput {
            events: VecDeque::from([
                (Duration::ZERO, InputEvent::Gap(SourceLoss {
                    kind: IssueKind::Overflow,
                    start: Duration::from_secs(1),
                    end: Duration::from_millis(1100),
                    lost: Duration::from_millis(100),
                    occurrences: 1,
                })),
                (Duration::ZERO, InputEvent::Issue(IssueKind::Disconnected)),
            ]),
            stopping: false,
        }));
        let files = TestFiles::new();
        let mut mixer = Mixer::new(Writers::create(&files.paths).unwrap(), Source::Microphone,
            Arc::new(AtomicU32::new(0)));
        let mut issues = IssueCollector::default();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let epoch = Instant::now();
        state.drain(Duration::from_secs(10), &mut mixer, &mut issues, &notices,
            epoch, false, &Arc::new(Mutex::new(None)));
        state.close_gap(Duration::from_secs(2), &mut issues, &notices, epoch);
        let issues = issues.finish();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].ended_at_seconds, Some(1.1));
        assert_eq!(issues[1].started_at_seconds, 1.1);
        assert!((issues.iter().map(|issue| issue.lost_duration_seconds).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[derive(Clone, Copy)]
    enum Scenario {
        Reconnect,
        NeverReconnect,
        SystemOverflow,
        SystemReconnect,
        LateAudio,
        Unavailable,
        StopOnOpen,
        StalledSelected,
        /// macOS refuses system audio until 3 s, then allows it.
        SystemPermissionGranted,
        /// macOS refuses system audio for the whole meeting.
        SystemPermissionDenied,
    }
    struct ScriptInput {
        events: VecDeque<(Duration, InputEvent)>,
        stopping: bool,
    }
    impl Input for ScriptInput {
        fn poll(&mut self, now: Duration) -> Option<InputEvent> {
            if self.events.front().is_some_and(|(at, _)| *at <= now) {
                self.events.pop_front().map(|(_, event)| event)
            } else if self.stopping {
                Some(InputEvent::End)
            } else {
                None
            }
        }
        fn stop(&mut self) {
            self.stopping = true;
        }
    }
    struct ScriptRuntime {
        now: Duration,
        stop_at: Duration,
        stop: Arc<AtomicBool>,
        scenario: Scenario,
        attempts: Vec<(IssueSource, Option<String>, Instant)>,
        pending: Vec<mpsc::SyncSender<Result<Box<dyn Input>, String>>>,
        cancelled_open: Option<Arc<AtomicBool>>,
    }
    impl Runtime for ScriptRuntime {
        fn now(&self) -> Duration {
            self.now
        }
        fn idle(&mut self) {
            self.now += Duration::from_millis(50);
            if self.now >= self.stop_at {
                self.stop.store(true, Ordering::Release);
            }
        }
        fn open(&mut self, source: IssueSource, device: Option<String>, epoch: Instant) -> Opening {
            self.attempts.push((source, device, epoch));
            let (tx, rx) = mpsc::sync_channel(1);
            let opening = Opening {
                rx,
                stop: Arc::new(AtomicBool::new(false)),
                cancel_on_drop: true,
            };
            let microphone = source == IssueSource::Microphone;
            if microphone
                && matches!(self.scenario, Scenario::StalledSelected)
                && self.attempts.last().unwrap().1.is_some()
            {
                self.pending.push(tx);
                self.cancelled_open = Some(opening.stop.clone());
                return opening;
            }
            let recovering = self.now >= Duration::from_secs(2);
            if !microphone
                && (matches!(self.scenario, Scenario::SystemPermissionDenied)
                    || (matches!(self.scenario, Scenario::SystemPermissionGranted)
                        && self.now < Duration::from_secs(3)))
            {
                assert!(tx.send(Err(crate::system_audio::PERMISSION_ERROR.into())).is_ok());
                return opening;
            }
            if matches!(self.scenario, Scenario::Unavailable)
                || (recovering && microphone && matches!(self.scenario, Scenario::NeverReconnect))
            {
                assert!(tx.send(Err("Device unavailable".into())).is_ok());
                return opening;
            }
            let disconnect = match self.scenario {
                Scenario::Reconnect | Scenario::NeverReconnect => microphone,
                Scenario::SystemReconnect => !microphone,
                _ => false,
            };
            let mut events = VecDeque::new();
            let first = if (microphone && matches!(self.scenario, Scenario::StalledSelected))
                || (!microphone && matches!(self.scenario, Scenario::SystemPermissionGranted))
            {
                (self.now.as_millis() / 50) as u64
            } else if recovering && disconnect {
                70
            } else {
                0
            };
            let last = if !recovering && disconnect {
                40
            } else {
                (self.stop_at.as_millis() / 50) as u64
            };
            for block in first..last {
                if !microphone
                    && matches!(self.scenario, Scenario::SystemOverflow)
                    && (40..42).contains(&block)
                {
                    continue;
                }
                let at = Duration::from_millis(block * 50);
                let dropped = if !microphone
                    && matches!(self.scenario, Scenario::SystemOverflow)
                    && block == 42
                {
                    1600
                } else {
                    0
                };
                let delivered =
                    if microphone && matches!(self.scenario, Scenario::LateAudio) && block < 180 {
                        Duration::from_millis(9500)
                    } else {
                        at + Duration::from_millis(50)
                    };
                events.push_back((
                    delivered,
                    InputEvent::Chunk(Chunk {
                        samples: vec![if microphone { 0.8 } else { 0.2 }; BLOCK],
                        sample_rate: RATE,
                        at,
                        discontinuity: dropped > 0,
                        dropped_frames: dropped,
                    }),
                ));
            }
            if !recovering && disconnect {
                events.push_back((
                    Duration::from_secs(2),
                    InputEvent::Issue(IssueKind::Disconnected),
                ));
            }
            assert!(tx
                .send(Ok(Box::new(ScriptInput {
                    events,
                    stopping: false
                }) as Box<dyn Input>))
                .is_ok());
            if matches!(self.scenario, Scenario::StopOnOpen) {
                self.stop.store(true, Ordering::Release);
            }
            opening
        }
    }
    fn run(
        scenario: Scenario,
        files: &TestFiles,
        failing_writes: usize,
    ) -> (Result<RecordingOutcome, String>, ScriptRuntime) {
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        run_with_notices(scenario, files, failing_writes, &notices)
    }
    fn run_with_notices(
        scenario: Scenario,
        files: &TestFiles,
        failing_writes: usize,
        notices: &SharedNotices,
    ) -> (Result<RecordingOutcome, String>, ScriptRuntime) {
        let stop = Arc::new(AtomicBool::new(false));
        let mut runtime = ScriptRuntime {
            now: Duration::ZERO,
            stop_at: Duration::from_secs(10),
            stop: stop.clone(),
            scenario,
            attempts: Vec::new(),
            pending: Vec::new(),
            cancelled_open: None,
        };
        let mut writers = Writers::create(&files.paths).unwrap();
        writers.audio.fail_writes = failing_writes;
        let (ready, _rx) = mpsc::sync_channel(1);
        let result = record_with(
            writers,
            Source::Both,
            Some("chosen input".into()),
            &stop,
            &Arc::new(Mutex::new(None)),
            Arc::new(AtomicU32::new(0)),
            StemLevels::default(),
            notices,
            ready,
            Instant::now(),
            &mut runtime,
        );
        (result, runtime)
    }
    fn assert_aligned(files: &TestFiles, seconds: usize) {
        for path in [
            &files.paths.audio,
            &files.paths.microphone,
            &files.paths.system,
        ] {
            assert_eq!(wav_duration(path).unwrap(), seconds as f64);
            assert_eq!(read_wav(path).len(), seconds * RATE as usize);
        }
    }

    #[test]
    fn microphone_error_reopens_on_same_epoch_and_preserves_full_meeting() {
        let files = TestFiles::new();
        let (outcome, runtime) = run(Scenario::Reconnect, &files, 0);
        let outcome = outcome.unwrap();
        assert_eq!(outcome.duration, 10.0);
        assert_eq!(outcome.issues.len(), 1);
        let issue = &outcome.issues[0];
        assert_eq!(issue.source, IssueSource::Microphone);
        assert_eq!(issue.started_at_seconds, 2.0);
        assert_eq!(issue.ended_at_seconds, Some(3.5));
        assert!(runtime
            .attempts
            .iter()
            .all(|attempt| attempt.2 == runtime.attempts[0].2));
        assert_aligned(&files, 10);
        let microphone = read_wav(&files.paths.microphone);
        assert!(microphone[32_000..56_000].iter().all(|sample| *sample == 0));
        assert!(microphone[..32_000]
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
        assert!(microphone[56_000..]
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
        assert!(read_wav(&files.paths.system)
            .iter()
            .all(|sample| *sample == pcm16(0.2)));
        assert!(read_wav(&files.paths.audio)[32_000..56_000]
            .iter()
            .all(|sample| *sample == pcm16(mix_sample(0.0, 0.2, Source::Both))));
    }

    #[test]
    fn missing_microphone_retries_and_falls_back_while_system_keeps_recording() {
        let files = TestFiles::new();
        let (outcome, runtime) = run(Scenario::NeverReconnect, &files, 0);
        let outcome = outcome.unwrap();
        assert_eq!(outcome.issues.len(), 1);
        assert_eq!(outcome.issues[0].started_at_seconds, 2.0);
        assert_eq!(outcome.issues[0].ended_at_seconds, None);
        assert!(outcome.issues[0].message.contains("default microphone"));
        assert!(runtime
            .attempts
            .iter()
            .any(|attempt| attempt.0 == IssueSource::Microphone && attempt.1.is_none()));
        assert_aligned(&files, 10);
        assert!(read_wav(&files.paths.microphone)[32_000..]
            .iter()
            .all(|sample| *sample == 0));
        assert!(read_wav(&files.paths.system)
            .iter()
            .all(|sample| *sample == pcm16(0.2)));
    }

    #[test]
    fn system_overflow_is_a_timed_gap_and_recording_continues() {
        let files = TestFiles::new();
        let outcome = run(Scenario::SystemOverflow, &files, 0).0.unwrap();
        assert_aligned(&files, 10);
        assert_eq!(outcome.issues.len(), 1);
        assert_eq!(outcome.issues[0].kind, IssueKind::Overflow);
        assert_eq!(outcome.issues[0].source, IssueSource::System);
        assert_eq!(outcome.issues[0].started_at_seconds, 2.0);
        assert_eq!(outcome.issues[0].ended_at_seconds, Some(2.1));
        assert!(read_wav(&files.paths.system)[32_000..33_600]
            .iter()
            .all(|sample| *sample == 0));
        assert!(read_wav(&files.paths.microphone)
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
    }

    #[test]
    fn system_restart_keeps_microphone_and_shared_epoch() {
        let files = TestFiles::new();
        let outcome = run(Scenario::SystemReconnect, &files, 0).0.unwrap();
        assert_aligned(&files, 10);
        assert_eq!(outcome.issues.len(), 1);
        assert_eq!(outcome.issues[0].source, IssueSource::System);
        assert_eq!(outcome.issues[0].ended_at_seconds, Some(3.5));
        assert!(read_wav(&files.paths.system)[32_000..56_000]
            .iter()
            .all(|sample| *sample == 0));
        assert!(read_wav(&files.paths.microphone)
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
    }

    #[test]
    fn late_valid_samples_repair_stems_and_mix_without_changing_length() {
        let files = TestFiles::new();
        let outcome = run(Scenario::LateAudio, &files, 0).0.unwrap();
        assert!(outcome.issues.is_empty());
        assert_aligned(&files, 10);
        assert!(read_wav(&files.paths.microphone)
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
        let expected = pcm16(mix_sample(0.8, 0.2, Source::Both));
        assert!(read_wav(&files.paths.audio)
            .iter()
            .all(|sample| (*sample as i32 - expected as i32).abs() <= 1));
    }

    #[test]
    fn refused_system_permission_records_the_microphone_and_joins_when_allowed() {
        let files = TestFiles::new();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let (outcome, runtime) =
            run_with_notices(Scenario::SystemPermissionGranted, &files, 0, &notices);
        let outcome = outcome.unwrap();
        assert_aligned(&files, 10);
        assert_eq!(outcome.issues.len(), 1);
        let issue = &outcome.issues[0];
        assert_eq!(issue.source, IssueSource::System);
        assert_eq!(issue.started_at_seconds, 0.0);
        assert!(issue.message.starts_with("System audio was not allowed by macOS at 0:00"), "{}", issue.message);
        let joined = issue.ended_at_seconds.unwrap();
        assert!((3.0..=3.3).contains(&joined), "{joined}");
        // Permission retries are prompt: about one per second while refused.
        let system_attempts = runtime.attempts.iter()
            .filter(|attempt| attempt.0 == IssueSource::System).count();
        assert!((4..=5).contains(&system_attempts), "{system_attempts}");
        let system = read_wav(&files.paths.system);
        let joined_frame = (joined * RATE as f64) as usize;
        assert!(system[..joined_frame].iter().all(|sample| *sample == 0));
        assert!(system[joined_frame + RATE as usize..].iter().all(|sample| *sample == pcm16(0.2)));
        assert!(read_wav(&files.paths.microphone).iter().all(|sample| *sample == pcm16(0.8)));
        assert_eq!(notices.lock().unwrap().system_problem, None);
    }

    #[test]
    fn refused_system_permission_stays_visible_for_the_whole_meeting() {
        let files = TestFiles::new();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let outcome = run_with_notices(Scenario::SystemPermissionDenied, &files, 0, &notices)
            .0
            .unwrap();
        assert_aligned(&files, 10);
        assert_eq!(outcome.issues.len(), 1);
        assert_eq!(outcome.issues[0].ended_at_seconds, None);
        assert!(read_wav(&files.paths.system).iter().all(|sample| *sample == 0));
        let notices = notices.lock().unwrap();
        assert_eq!(notices.system_problem, Some(SystemProblem::Permission));
        assert_eq!(
            notices.notices[source_index(IssueSource::System)].as_ref().map(|notice| notice.0.as_str()),
            Some(PERMISSION_NOTICE)
        );
    }

    #[test]
    fn a_lost_system_stream_is_flagged_until_it_reconnects() {
        let mut state = SourceState::new(IssueSource::System, true);
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let mut issues = IssueCollector::default();
        state.lost(Duration::from_secs(4), &mut issues, &notices);
        assert_eq!(notices.lock().unwrap().system_problem, Some(SystemProblem::Lost));
        state.close_gap(Duration::from_secs(5), &mut issues, &notices, Instant::now());
        assert_eq!(notices.lock().unwrap().system_problem, None);
        // A microphone loss never claims the other side is missing.
        let mut microphone = SourceState::new(IssueSource::Microphone, true);
        microphone.lost(Duration::from_secs(6), &mut issues, &notices);
        assert_eq!(notices.lock().unwrap().system_problem, None);
    }

    #[test]
    fn persistent_writer_failure_is_fatal_after_retry() {
        let files = TestFiles::new();
        let (result, runtime) = run(Scenario::Reconnect, &files, usize::MAX);
        assert!(result
            .unwrap_err()
            .contains("injected persistent writer failure"));
        assert!(runtime.now < Duration::from_secs(2));
        assert_eq!(
            wav_duration(&files.paths.microphone).unwrap(),
            runtime.now.as_secs_f64()
        );
        assert_eq!(
            wav_duration(&files.paths.system).unwrap(),
            runtime.now.as_secs_f64()
        );
        assert!(read_wav(&files.paths.microphone)
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
    }

    #[test]
    fn transient_writer_failure_retries_without_duplicating_samples() {
        let files = TestFiles::new();
        run(Scenario::Reconnect, &files, 1).0.unwrap();
        assert_aligned(&files, 10);
    }

    #[test]
    fn no_usable_source_returns_failure_only_after_stop() {
        let files = TestFiles::new();
        let (outcome, runtime) = run(Scenario::Unavailable, &files, 0);
        assert!(outcome.unwrap_err().contains("No audio was received"));
        assert_eq!(runtime.now, Duration::from_secs(10));
        assert_eq!(wav_duration(&files.paths.audio).unwrap(), 0.0);
    }

    #[test]
    fn stop_drains_an_open_that_completed_between_owner_polls() {
        let files = TestFiles::new();
        let outcome = run(Scenario::StopOnOpen, &files, 0).0.unwrap();
        assert_eq!(outcome.duration, 0.05);
        assert_eq!(read_wav(&files.paths.microphone), vec![pcm16(0.8); BLOCK]);
        assert_eq!(read_wav(&files.paths.system), vec![pcm16(0.2); BLOCK]);
    }

    #[test]
    fn stalled_selected_open_is_cancelled_at_five_seconds_and_default_recovers() {
        let files = TestFiles::new();
        let (outcome, runtime) = run(Scenario::StalledSelected, &files, 0);
        let outcome = outcome.unwrap();
        assert!(runtime.cancelled_open.unwrap().load(Ordering::Acquire));
        assert_eq!(runtime.pending.len(), 1);
        assert_eq!(
            runtime
                .attempts
                .iter()
                .filter(|attempt| attempt.0 == IssueSource::Microphone)
                .count(),
            2
        );
        assert_eq!(outcome.issues.len(), 1);
        assert_eq!(outcome.issues[0].started_at_seconds, 0.0);
        assert_eq!(outcome.issues[0].ended_at_seconds, Some(5.0));
        assert!(outcome.issues[0]
            .message
            .contains("Reconnected to the default microphone at 0:05"));
        assert_aligned(&files, 10);
        assert!(read_wav(&files.paths.microphone)[..80_000]
            .iter()
            .all(|sample| *sample == 0));
        assert!(read_wav(&files.paths.microphone)[80_000..]
            .iter()
            .all(|sample| *sample == pcm16(0.8)));
        assert!(read_wav(&files.paths.system)
            .iter()
            .all(|sample| *sample == pcm16(0.2)));
    }
}
