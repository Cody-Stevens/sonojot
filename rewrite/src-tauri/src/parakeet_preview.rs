//! Disposable drafts from the resident Parakeet decoder. No text from here is
//! ever a recognition result. The capture owner enables this only while caught
//! up with live audio; files, meetings, recovery and Stop never request a pass.
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const RATE: usize = 16_000;
const TAIL: usize = 5 * RATE;
const MIN_AUDIO: usize = RATE / 2;
const INTERVAL: Duration = Duration::from_millis(700);
const BUDGET: Duration = Duration::from_millis(200);
const MAX_AGE: Duration = Duration::from_millis(1000);
pub(crate) const MAX_CAPTURE_LAG: Duration = Duration::from_millis(250);

/// Publication and expiry share a lock so a meter tick cannot clear a newer
/// draft. The existing capture meter ticks even while primary inference blocks.
pub(crate) struct Display {
    state: Mutex<(String, Option<Instant>)>,
    callback: Arc<dyn Fn(crate::speech::Event) + Send + Sync>,
    session_id: u64,
}

impl Display {
    pub(crate) fn new(
        session_id: u64,
        callback: Arc<dyn Fn(crate::speech::Event) + Send + Sync>,
    ) -> Self {
        Self {
            state: Mutex::new((String::new(), None)),
            callback,
            session_id,
        }
    }

    pub(crate) fn publish(&self, mut text: String, until: Instant) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if until <= Instant::now() {
            text.clear();
        }
        state.1 = (!text.is_empty()).then_some(until);
        if state.0 != text {
            state.0 = text.clone();
            (self.callback)(crate::speech::Event::Preview {
                session_id: self.session_id,
                text,
            });
        }
    }

    pub(crate) fn clear(&self) {
        self.publish(String::new(), Instant::now());
    }

    pub(crate) fn expire(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.1.is_some_and(|until| until <= Instant::now()) {
            state.0.clear();
            state.1 = None;
            (self.callback)(crate::speech::Event::Preview {
                session_id: self.session_id,
                text: String::new(),
            });
        }
    }
}

pub(crate) fn supported(model: &str) -> bool {
    model == crate::optional_asr::PARAKEET_FT2_ID
        && cfg!(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(
                any(target_os = "windows", target_os = "linux"),
                target_arch = "x86_64"
            )
        ))
}

struct Request {
    generation: u64,
    started: Instant,
    audio: Vec<f32>,
}

struct Reply {
    generation: u64,
    started: Instant,
    elapsed: Duration,
    text: Option<String>,
}

pub(crate) struct Preview {
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<Reply>,
    generation: Arc<AtomicU64>,
    audio: VecDeque<f32>,
    end: u64,
    committed: u64,
    in_flight: bool,
    allowed: bool,
    disabled: bool,
    last_started: Option<Instant>,
    shown_at: Option<Instant>,
    text: String,
}

impl Preview {
    pub(crate) fn start(
        origin: u64,
        decode: impl Fn(&[f32]) -> Option<String> + Send + 'static,
    ) -> Option<Self> {
        let (tx, requests) = mpsc::sync_channel::<Request>(1);
        let (replies, rx) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = generation.clone();
        std::thread::Builder::new()
            .name("parakeet-preview".into())
            .spawn(move || {
                background_priority();
                // The decoder owns a reference to the existing model and library.
                // Dropping a session invalidates work and disconnects this queue;
                // it never waits for the native call or blocks final recognition.
                while let Ok(request) = requests.recv() {
                    let started = Instant::now();
                    let text = (worker_generation.load(Ordering::Acquire) == request.generation
                        && request.started.elapsed() < MAX_AGE)
                        .then(|| decode(&request.audio))
                        .flatten();
                    if replies
                        .send(Reply {
                            generation: request.generation,
                            started: request.started,
                            elapsed: started.elapsed(),
                            text,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self {
            tx,
            rx,
            generation,
            audio: VecDeque::with_capacity(TAIL),
            end: origin,
            committed: 0,
            in_flight: false,
            allowed: false,
            disabled: false,
            last_started: None,
            shown_at: None,
            text: String::new(),
        })
    }

    pub(crate) fn allow(&mut self, allowed: bool) {
        if self.allowed && !allowed {
            self.invalidate();
        }
        self.allowed = allowed;
    }

    pub(crate) fn push(&mut self, audio: &[f32]) {
        self.end += audio.len() as u64;
        self.audio.extend(audio);
        self.trim();
    }

    fn trim(&mut self) {
        let keep = TAIL.min(self.end.saturating_sub(self.committed) as usize);
        self.audio.drain(..self.audio.len().saturating_sub(keep));
    }

    fn invalidate(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.text.clear();
        self.shown_at = None;
    }

    pub(crate) fn poll(&mut self, committed: u64, speaking: bool) -> &str {
        if committed != self.committed {
            self.committed = committed;
            self.trim();
            self.invalidate();
        }
        if let Ok(reply) = self.rx.try_recv() {
            self.in_flight = false;
            // A slow machine gets no more speculative decodes this session.
            // Native inference is not cancellable; the budget rejects its
            // result and future work, it does not pretend to interrupt it.
            if reply.elapsed > BUDGET {
                self.disabled = true;
                self.invalidate();
            } else if self.allowed
                && reply.generation == self.generation.load(Ordering::Acquire)
                && reply.started.elapsed() < MAX_AGE
            {
                if let Some(text) = reply.text {
                    self.text = text;
                    self.shown_at = Some(reply.started);
                }
            }
        }
        if self.shown_at.is_some_and(|at| at.elapsed() >= MAX_AGE) {
            self.invalidate();
        }
        if self.allowed
            && !self.disabled
            && !self.in_flight
            && speaking
            && self.audio.len() >= MIN_AUDIO
            && self.last_started.is_none_or(|at| at.elapsed() >= INTERVAL)
        {
            let started = Instant::now();
            let request = Request {
                generation: self.generation.load(Ordering::Acquire),
                started,
                audio: self.audio.iter().copied().collect(),
            };
            if self.tx.try_send(request).is_ok() {
                self.in_flight = true;
                self.last_started = Some(started);
            }
        }
        &self.text
    }
}

fn background_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 5);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Preview, mpsc::Receiver<Request>, mpsc::SyncSender<Reply>) {
        let (tx, requests) = mpsc::sync_channel(1);
        let (replies, rx) = mpsc::sync_channel(1);
        let preview = Preview {
            tx,
            rx,
            generation: Arc::new(AtomicU64::new(0)),
            audio: VecDeque::new(),
            end: 0,
            committed: 0,
            in_flight: false,
            allowed: true,
            disabled: false,
            last_started: None,
            shown_at: None,
            text: String::new(),
        };
        (preview, requests, replies)
    }

    #[test]
    fn display_expires_without_a_worker_reply_and_rejects_late_publication() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = events.clone();
        let display = Display::new(
            7,
            Arc::new(move |event| {
                if let crate::speech::Event::Preview { session_id, text } = event {
                    assert_eq!(session_id, 7);
                    received.lock().unwrap().push(text);
                }
            }),
        );
        let fresh = Instant::now() + Duration::from_secs(1);
        display.publish("fresh draft".into(), fresh);
        display.expire();
        assert_eq!(events.lock().unwrap().len(), 1);
        display.state.lock().unwrap().1 = Some(Instant::now());
        display.expire();
        display.publish("late draft".into(), Instant::now() - Duration::from_secs(1));
        assert_eq!(*events.lock().unwrap(), ["fresh draft", ""]);
        display.publish("next draft".into(), fresh);
        display.clear();
        assert_eq!(
            *events.lock().unwrap(),
            ["fresh draft", "", "next draft", ""]
        );
    }

    fn reply(request: Request, elapsed: Duration) -> Reply {
        Reply {
            generation: request.generation,
            started: request.started,
            elapsed,
            text: Some("draft".into()),
        }
    }

    #[test]
    fn only_recent_uncommitted_audio_is_offered_and_only_one_pass_can_run() {
        let (mut preview, requests, _) = fixture();
        preview.push(&vec![1.0; 10 * RATE]);
        preview.poll(8 * RATE as u64, true);
        let request = requests.try_recv().unwrap();
        assert_eq!(request.audio.len(), 2 * RATE);
        preview.last_started = None;
        preview.push(&vec![2.0; 10 * RATE]);
        preview.poll(8 * RATE as u64, true);
        assert_eq!(preview.audio.len(), TAIL);
        assert!(
            requests.try_recv().is_err(),
            "an unfinished pass must not queue another"
        );
    }

    #[test]
    fn a_commit_or_capture_backlog_rejects_an_in_flight_draft() {
        for commit in [false, true] {
            let (mut preview, requests, replies) = fixture();
            preview.push(&vec![0.2; RATE]);
            preview.poll(0, true);
            let request = requests.try_recv().unwrap();
            if commit {
                preview.poll(RATE as u64, false);
            } else {
                preview.allow(false);
            }
            replies
                .send(reply(request, Duration::from_millis(20)))
                .unwrap();
            assert_eq!(
                preview.poll(if commit { RATE as u64 } else { 0 }, false),
                ""
            );
        }
    }

    #[test]
    fn slow_pass_disables_preview_and_expired_results_are_never_shown() {
        for slow in [false, true] {
            let (mut preview, requests, replies) = fixture();
            preview.push(&vec![0.2; RATE]);
            preview.poll(0, true);
            let mut result = reply(
                requests.try_recv().unwrap(),
                if slow {
                    BUDGET + Duration::from_millis(1)
                } else {
                    Duration::ZERO
                },
            );
            if !slow {
                result.started = Instant::now() - MAX_AGE;
            }
            replies.send(result).unwrap();
            assert_eq!(preview.poll(0, false), "");
            assert_eq!(preview.disabled, slow);
        }
    }

    #[test]
    fn a_fresh_draft_expires_and_silence_does_not_start_a_decode() {
        let (mut preview, requests, replies) = fixture();
        preview.push(&vec![0.2; RATE]);
        preview.poll(0, false);
        assert!(requests.try_recv().is_err());
        preview.poll(0, true);
        replies
            .send(reply(requests.try_recv().unwrap(), Duration::ZERO))
            .unwrap();
        assert_eq!(preview.poll(0, false), "draft");
        preview.shown_at = Some(Instant::now() - MAX_AGE);
        assert_eq!(preview.poll(0, false), "");
    }

    #[test]
    fn dropping_preview_does_not_wait_for_native_inference() {
        let (entered, start) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let (exited, end) = mpsc::channel();
        let mut preview = Preview::start(0, move |_| {
            entered.send(()).unwrap();
            gate.recv().unwrap();
            exited.send(()).unwrap();
            Some("late draft".into())
        })
        .unwrap();
        preview.allow(true);
        preview.push(&vec![0.2; RATE]);
        preview.poll(0, true);
        start.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(preview);
        release.send(()).unwrap();
        end.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.invalidate();
    }
}
