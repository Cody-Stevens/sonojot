use super::*;
use std::{cell::RefCell, collections::VecDeque, sync::Arc};

thread_local! {
    static CHILDREN: RefCell<Option<VecDeque<&'static str>>> = const { RefCell::new(None) };
}

pub(super) fn next_child() -> Option<&'static str> {
    CHILDREN.with(|children| {
        children
            .borrow_mut()
            .as_mut()
            .map(|q| q.pop_front().expect("unexpected extra worker/retry"))
    })
}

pub(super) fn child_command(behavior: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "recognition::tests::fake_child",
        "--nocapture",
    ]);
    command.env("SONOJOT_TEST_WORKER_BEHAVIOR", behavior);
    command
}

struct Children;
impl Children {
    fn set(behaviors: &[&'static str]) -> Self {
        CHILDREN.with(|c| {
            assert!(c.borrow().is_none());
            *c.borrow_mut() = Some(behaviors.iter().copied().collect());
        });
        Self
    }
}
impl Drop for Children {
    fn drop(&mut self) {
        let remaining = CHILDREN.with(|c| c.borrow_mut().take().unwrap());
        if !thread::panicking() {
            assert!(remaining.is_empty(), "expected workers were never started");
        }
    }
}

fn hang() -> ! {
    loop {
        thread::park_timeout(Duration::from_secs(5));
    }
}

// Launched only by child_command, in a separate OS process. No native library,
// app initialization, user data, microphone, or installed executable is used.
#[test]
#[ignore = "subprocess fixture"]
fn fake_child() {
    let Ok(address) = std::env::var("SONOJOT_TEST_WORKER_ENDPOINT") else {
        return;
    };
    let behavior = std::env::var("SONOJOT_TEST_WORKER_BEHAVIOR").unwrap();
    let mut socket = worker_socket(&address).unwrap();
    let mut raw = String::new();
    let mut count = 0;
    let mut last = 0;
    let mut active = 0;
    let mut sessions = 0;
    loop {
        let request: Request = match read_frame(&mut socket) {
            Ok(r) => r,
            Err(_) => return,
        };
        assert_eq!(request.version, VERSION);
        assert_eq!(request.request, last + 1);
        last = request.request;
        let reply = match &request.op {
            Operation::Load { model, .. } => {
                if behavior == "hang-load" {
                    hang();
                }
                if behavior == "crash-load" {
                    std::process::exit(77);
                }
                Reply::Loaded {
                    model: if behavior == "wrong-model" {
                        "different-model".into()
                    } else {
                        model.clone()
                    },
                }
            }
            Operation::Begin => {
                assert!(request.session > active);
                active = request.session;
                sessions += 1;
                raw.clear();
                count = 0;
                Reply::Ack
            }
            Operation::Audio { samples, rate } => {
                assert_eq!(request.session, active);
                assert!(!samples.is_empty() && samples.len() <= MAX_SAMPLES);
                assert!(*rate > 0 && *rate <= 384_000);
                assert!(samples.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
                count += 1;
                if behavior == "hang-audio" {
                    hang();
                }
                if behavior == "crash-first" || (matches!(behavior.as_str(), "crash-second" | "guarded-crash-second") && count == 2) {
                    std::process::exit(78);
                }
                if behavior == "guarded-second-session-crash" && sessions == 2 && count == 2 {
                    std::process::exit(78);
                }
                if behavior == "invalid" {
                    socket.write_all(&[1, 0, 0, 0, b'!']).unwrap();
                    hang();
                }
                if behavior == "oversize" {
                    socket
                        .write_all(&((MAX_FRAME + 1) as u32).to_le_bytes())
                        .unwrap();
                    hang();
                }
                for value in samples {
                    if !raw.is_empty() {
                        raw.push(' ');
                    }
                    raw.push_str(&format!("{value:.1}"));
                }
                if behavior == "wrong-kind" {
                    Reply::Ack
                } else {
                    Reply::Text {
                        raw: raw.clone(),
                        original_text: behavior.starts_with("guarded").then(|| format!("{raw} go go go go")),
                        warnings: vec![],
                        committed_seconds: None,
                    }
                }
            }
            Operation::Finish { .. } => {
                if behavior == "crash-finish" {
                    std::process::exit(79);
                }
                Reply::Finished {
                    result: RecognitionResult {
                        original_text: behavior.starts_with("guarded").then(|| format!("{raw} go go go go")),
                        segments: Vec::new(),
                        text: raw.clone(),
                        completeness: RecognitionCompleteness::Complete,
                        warnings: vec![],
                    },
                    details: None,
                }
            }
            Operation::Shutdown => {
                if behavior == "hang-shutdown" {
                    hang();
                }
                send_reply(&mut socket, &request, Reply::Ack).unwrap();
                return;
            }
            Operation::Abort => Reply::Ack,
        };
        let session = if behavior == "stale" && matches!(request.op, Operation::Audio { .. }) {
            request.session + 1
        } else {
            request.session
        };
        let response = Response {
            version: VERSION,
            request: request.request,
            session,
            body: reply,
        };
        socket.write_all(&encode(&response).unwrap()).unwrap();
    }
}

fn remote() -> Remote {
    Remote::load(Path::new("unused-model-root"), "fake-model", &NEVER_CANCEL).unwrap()
}

#[test]
fn warm_stream_publishes_ready_without_another_load() {
    let _children = Children::set(&["success"]);
    let mut remote = remote();
    for _ in 0..2 {
        let phases = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = phases.clone();
        let _guard = observe_progress(Arc::new(move |p| observed.lock().unwrap().push(p.stage)));
        let stream = remote.stream(None).unwrap();
        stream.finish(false).unwrap();
        assert_eq!(*phases.lock().unwrap(), ["loading", "transcribing"]);
    }
}

#[test]
fn crash_replays_acknowledged_and_pending_audio_once_without_duplicates() {
    let _children = Children::set(&["crash-second", "success"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.1], 16_000).unwrap();
    assert_eq!(stream.update().unwrap(), "0.1");
    stream.add(&[0.2], 16_000).unwrap();
    stream.add(&[0.3], 16_000).unwrap();
    let (result, _) = stream.finish(false).unwrap();
    assert_eq!(result.text, "0.1 0.2 0.3");
    assert_eq!(result.completeness, RecognitionCompleteness::Complete);
}

#[test]
fn second_crash_preserves_acknowledged_snapshot_and_requires_review() {
    let _children = Children::set(&["crash-second", "crash-first"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.1], 16_000).unwrap();
    stream.add(&[0.2], 16_000).unwrap();
    assert!(stream.failed());
    let (result, _) = stream.finish(false).unwrap();
    assert_eq!(result.text, "0.1");
    assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
    assert!(!result.warnings.is_empty());
    assert!(remote.connection.is_none());
}

#[test]
fn finish_crash_replays_before_finalizing() {
    let _children = Children::set(&["crash-finish", "success"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.1, 0.2], 16_000).unwrap();
    let result = stream.finish(false).unwrap().0;
    assert_eq!(result.text, "0.1 0.2");
    assert_eq!(result.completeness, RecognitionCompleteness::Complete);
}

#[test]
fn missing_replay_returns_partial_instead_of_retrying_or_marking_complete() {
    let _children = Children::set(&["crash-second"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.1], 16_000).unwrap();
    stream.replay = None;
    stream.add(&[0.2], 16_000).unwrap();
    let result = stream.finish(false).unwrap().0;
    assert_eq!(result.text, "0.1");
    assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
}

#[test]
fn replay_quota_releases_audio_and_keeps_successful_recognition_complete() {
    let _children = Children::set(&["success"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.replay.as_mut().unwrap().bytes = REPLAY_QUOTA;
    stream.add(&[0.1], 16_000).unwrap();
    assert!(stream.replay.is_none());
    assert_eq!(
        stream.finish(false).unwrap().0.completeness,
        RecognitionCompleteness::Complete
    );
}

#[test]
fn malformed_oversize_stale_and_wrong_kind_responses_terminate_child() {
    for behavior in ["invalid", "oversize", "stale", "wrong-kind"] {
        let mut connection =
            Connection::spawn_command(child_command(behavior), &NEVER_CANCEL, true).unwrap();
        connection
            .exchange(1, Operation::Begin, &NEVER_CANCEL, START_LIMIT)
            .unwrap();
        assert!(
            connection
                .exchange(
                    1,
                    Operation::Audio {
                        samples: vec![0.1],
                        rate: 16_000
                    },
                    &NEVER_CANCEL,
                    Duration::from_secs(2)
                )
                .is_err(),
            "{behavior}"
        );
        assert!(
            connection.child.child.try_wait().unwrap().is_some(),
            "{behavior} worker survived"
        );
    }
}

#[test]
fn stalled_load_and_decode_have_finite_deadlines_and_terminate_child() {
    for behavior in ["hang-load", "hang-audio"] {
        let mut connection =
            Connection::spawn_command(child_command(behavior), &NEVER_CANCEL, true).unwrap();
        let op = if behavior == "hang-load" {
            Operation::Load {
                root: PathBuf::new(),
                model: "fake-model".into(),
            }
        } else {
            Operation::Audio {
                samples: vec![0.1],
                rate: 16_000,
            }
        };
        let start = Instant::now();
        let error = connection
            .exchange(0, op, &NEVER_CANCEL, Duration::from_millis(100))
            .unwrap_err();
        assert!(error.contains("deadline"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(connection.child.child.try_wait().unwrap().is_some());
    }
}

#[test]
fn cancellation_interrupts_decode_without_retry() {
    let _children = Children::set(&["hang-audio"]);
    let mut remote = remote();
    assert!(remote.is_alive());
    let cancel = Arc::new(AtomicBool::new(false));
    let mut stream = remote.stream(Some(&cancel)).unwrap();
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        trigger.store(true, Ordering::Release);
    });
    let started = Instant::now();
    assert!(stream
        .add(&[0.1], 16_000)
        .unwrap_err()
        .contains("cancelled"));
    drop(stream);
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(remote.connection.is_none());
    assert!(
        !remote.is_alive(),
        "cancelled workers must not be counted as resident"
    );
}

#[test]
fn cancellation_interrupts_cold_load() {
    let _children = Children::set(&["hang-load"]);
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = cancel.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        trigger.store(true, Ordering::Release);
    });
    let started = Instant::now();
    assert!(Remote::load(Path::new("unused"), "fake-model", &cancel)
        .err()
        .unwrap()
        .contains("cancel"));
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn wrong_model_and_load_crash_do_not_silently_fallback() {
    for behavior in ["wrong-model", "crash-load"] {
        let _children = Children::set(&[behavior, behavior]);
        assert!(Remote::load(Path::new("unused"), "fake-model", &NEVER_CANCEL).is_err());
    }
}

#[test]
fn load_retry_uses_the_same_single_retry_budget_as_decode() {
    let _children = Children::set(&["crash-load", "crash-second"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.1], 16_000).unwrap();
    stream.add(&[0.2], 16_000).unwrap();
    let result = stream.finish(false).unwrap().0;
    assert_eq!(result.text, "0.1");
    assert_eq!(result.completeness, RecognitionCompleteness::Incomplete);
}

#[test]
fn audio_is_normalized_bounded_and_sessions_increase() {
    let _children = Children::set(&["success"]);
    let mut remote = remote();
    let mut stream = remote.stream(None).unwrap();
    let first = stream.session;
    let mut samples = vec![0.0; MAX_SAMPLES + 1];
    samples[..3].copy_from_slice(&[f32::NAN, -2.0, 2.0]);
    stream.add(&samples, 48_000).unwrap();
    let result = stream.finish(false).unwrap().0;
    assert!(result.text.starts_with("0.0 -1.0 1.0"));
    assert_eq!(result.text.split_whitespace().count(), samples.len());
    let mut second = remote.stream(None).unwrap();
    assert!(second.session > first);
    second.add(&[0.2], 16_000).unwrap();
    assert_eq!(second.finish(false).unwrap().0.text, "0.2");
}

#[test]
fn consecutive_worker_sessions_never_share_audio_text_or_replay() {
    for retry_second in [false, true] {
        let _children = if retry_second {
            Children::set(&["guarded-second-session-crash", "guarded"])
        } else {
            Children::set(&["guarded"])
        };
        let mut remote = remote();
        let original_pid = remote.connection.as_ref().unwrap().child.child.id();
        let mut first = remote.stream(None).unwrap();
        let first_session = first.session;
        first.add(&[0.1, 0.2], 16_000).unwrap();
        first.add(&[0.3], 48_000).unwrap();
        assert_eq!(first.update().unwrap(), "0.1 0.2 0.3");
        let first_result = first.finish(false).unwrap().0;
        assert_eq!(first_result.text, "0.1 0.2 0.3");
        assert_eq!(first_result.original_text(), "0.1 0.2 0.3 go go go go");
        assert_eq!(first_result.completeness, RecognitionCompleteness::Complete);

        let mut second = remote.stream(None).unwrap();
        assert_eq!(second.pid(), original_pid, "exercise warm worker reuse");
        assert!(second.session > first_session);
        assert_eq!(second.update().unwrap(), "");
        assert_eq!(second.original_text(), None);
        assert!(second.warnings().is_empty());
        assert_eq!(second.replay.as_ref().unwrap().bytes, 0);
        second.add(&[0.8], 44_100).unwrap();
        assert_eq!(second.update().unwrap(), "0.8");
        assert_eq!(second.original_text(), Some("0.8 go go go go"));
        // In the retry case, crash after this request reaches the worker. The
        // replacement must receive this session's acknowledged/pending audio
        // alone, including its rate changes, and await the complete Finish.
        second.add(&[0.9, -0.7], 16_000).unwrap();
        let second_result = second.finish(false).unwrap().0;
        assert_eq!(second_result.text, "0.8 0.9 -0.7");
        assert_eq!(second_result.original_text(), "0.8 0.9 -0.7 go go go go");
        assert_eq!(second_result.completeness, RecognitionCompleteness::Complete);
        assert!(second_result.warnings.is_empty());
        assert_eq!(first_result.text, "0.1 0.2 0.3");
    }
}

/// Field failure 2026-10-14: the idle resident worker exited (macOS memory
/// pressure) right as the next dictation started. Whether the death is seen
/// before admission or only after it, the next job reloads once and succeeds.
#[test]
fn resident_worker_that_died_between_jobs_is_reloaded_for_the_next_job() {
    use crate::{
        domain::{MemoryMode, ModelRetention},
        resource_manager as rm,
        speech::{self, FailureKind},
    };
    let _gate = rm::tests::test_gate();
    rm::reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    let root = Path::new("unused-model-root");
    let model = "fake-model";
    for died_after_admission in [false, true] {
        let _children = Children::set(&["success", "success"]);
        let mut failure = FailureKind::Other;
        let mut recognizer = None;
        rm::acquire_asr_with_admission(model, "dictation").unwrap();
        speech::ensure_recognizer(root, &mut recognizer, model, &NEVER_CANCEL, 1, false, &mut failure)
            .unwrap();
        rm::notify_asr_finished();
        let first_pid = recognizer.as_mut().unwrap().worker_pid_for_test();
        recognizer.as_mut().unwrap().kill_worker_for_test();
        assert!(recognizer.as_mut().unwrap().worker_exit().is_some());

        // Next job. The pre-admission check normally drops the dead worker;
        // when the death lands after it, admission still sees a resident.
        let exited = !died_after_admission
            && speech::drop_unusable_recognizer(&mut recognizer, model, 2);
        assert_eq!(exited, !died_after_admission);
        assert_eq!(rm::model_state_blocking(model).1, died_after_admission);
        rm::acquire_asr_with_admission(model, "dictation").unwrap();
        speech::ensure_recognizer(root, &mut recognizer, model, &NEVER_CANCEL, 2, exited, &mut failure)
            .unwrap();
        let active = recognizer.as_mut().unwrap();
        assert!(active.is_alive());
        assert_ne!(active.worker_pid_for_test(), first_pid);
        assert!(rm::model_state_blocking(model).1);
        let mut stream = active.stream_interruptible(&NEVER_CANCEL).unwrap();
        stream.add(&[0.1], 16_000).unwrap();
        let result = stream.finish_recognition().unwrap();
        assert_eq!(result.text, "0.1");
        assert_eq!(result.completeness, RecognitionCompleteness::Complete);
        rm::notify_asr_finished();
        drop(recognizer);
        rm::confirm_asr_released();
    }
    rm::reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
}

/// A worker that dies after the last liveness check is replaced by the
/// stream's single Begin reconnect, before any audio is consumed.
#[test]
fn worker_death_before_begin_reconnects_once() {
    let _children = Children::set(&["success", "success"]);
    let mut remote = remote();
    remote.kill_worker_for_test();
    let mut stream = remote.stream(None).unwrap();
    stream.add(&[0.2], 16_000).unwrap();
    assert_eq!(stream.finish(false).unwrap().0.text, "0.2");
    assert!(remote.is_alive());
}

#[test]
fn killed_worker_reports_its_exit_value() {
    let _children = Children::set(&["success"]);
    let mut remote = remote();
    assert!(remote.exit().is_none());
    remote.kill_worker_for_test();
    let (_, value) = remote.exit().unwrap();
    // 128 + SIGKILL on Unix (the macOS memory-pressure signature); Windows
    // TerminateProcess exits with code 1.
    assert_eq!(value, if cfg!(unix) { 137 } else { 1 });
}

#[test]
fn stalled_native_drop_is_bounded() {
    let _children = Children::set(&["hang-shutdown"]);
    let remote = remote();
    let started = Instant::now();
    drop(remote);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
#[cfg(windows)]
fn closing_owner_job_kills_worker_without_a_shutdown_request() {
    let mut connection =
        Connection::spawn_command(child_command("success"), &NEVER_CANCEL, true).unwrap();
    let job = std::mem::replace(&mut connection.child.job, std::ptr::null_mut());
    assert!(!job.is_null());
    assert_ne!(
        unsafe { windows_sys::Win32::Foundation::CloseHandle(job) },
        0
    );
    let started = Instant::now();
    while connection.child.child.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "worker survived owner job close"
        );
        thread::sleep(POLL);
    }
}

#[test]
fn native_load_is_rejected_in_parent_before_opening_any_libraries() {
    assert!(!is_worker());
    assert!(
        Recognizer::load_native(Path::new("unused"), "moonshine-medium")
            .err()
            .unwrap()
            .contains("owned worker")
    );
    assert!(
        Recognizer::load_native(Path::new("unused"), crate::gpu_asr::QWEN_FP16_CUDA_ID)
            .err()
            .unwrap()
            .contains("owned worker")
    );
}

#[test]
fn response_validation_rejects_wrong_version_request_and_oversized_text() {
    for (version, request, session) in [(VERSION + 1, 1, 1), (VERSION, 2, 1), (VERSION, 1, 2)] {
        assert!(validate_response(
            &Response {
                version,
                request,
                session,
                body: Reply::Ack
            },
            1,
            1
        )
        .is_err());
    }
    assert!(validate_response(
        &Response {
            version: VERSION,
            request: 1,
            session: 1,
            body: Reply::Text {
                raw: "a".repeat(MAX_TEXT + 1),
                original_text: None,
                warnings: vec![],
                committed_seconds: None
            }
        },
        1,
        1
    )
    .is_err());
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    bytes.push(b'!');
    assert!(read_frame::<Response>(&mut &bytes[..]).is_err());
    assert!(read_frame::<Response>(&mut &((MAX_FRAME + 1) as u32).to_le_bytes()[..]).is_err());
}

#[test]
fn worker_preserves_original_in_finished_results_and_failed_replay_snapshots() {
    for crash in [false, true] {
        let behaviors: &[&str] = if crash { &["guarded-crash-second", "crash-first"] } else { &["guarded"] };
        let _children = Children::set(behaviors);
        let mut remote = remote();
        let mut stream = remote.stream(None).unwrap();
        stream.add(&[0.1], 16_000).unwrap();
        assert_eq!(stream.update().unwrap(), "0.1");
        assert_eq!(stream.original_text(), Some("0.1 go go go go"));
        if crash { stream.add(&[0.2], 16_000).unwrap(); }
        let result = stream.finish(false).unwrap().0;
        assert_eq!(result.text, "0.1");
        assert_eq!(result.original_text(), "0.1 go go go go");
        assert_eq!(result.completeness == RecognitionCompleteness::Incomplete, crash);
    }
}

#[test]
fn original_text_roundtrips_in_worker_frames_and_obeys_byte_limits() {
    for finished in [false, true] {
        for length in [MAX_TEXT, MAX_TEXT + 1] {
            let mut original = "\u{00e9}".repeat(MAX_TEXT / 2);
            if length > MAX_TEXT { original.push('!'); }
            let body = if finished {
                Reply::Finished {
                    result: RecognitionResult {
                        text: String::new(), original_text: Some(original), segments: Vec::new(),
                        completeness: RecognitionCompleteness::Incomplete, warnings: vec!["omitted".into()],
                    }, details: None,
                }
            } else {
                Reply::Text { raw: String::new(), original_text: Some(original), warnings: vec!["omitted".into()], committed_seconds: None }
            };
            let response = Response { version: VERSION, request: 1, session: 1, body };
            let bytes = encode(&response).unwrap();
            let decoded: Response = read_frame(&mut &bytes[..]).unwrap();
            assert_eq!(validate_response(&decoded, 1, 1).is_ok(), length <= MAX_TEXT);
            let retained = match &decoded.body {
                Reply::Text { raw, original_text, .. } => {
                    assert!(raw.is_empty());
                    original_text.as_ref().unwrap()
                }
                Reply::Finished { result, .. } => {
                    assert!(result.text.is_empty());
                    result.original_text.as_ref().unwrap()
                }
                _ => unreachable!(),
            };
            assert_eq!(retained.len(), length);
        }
    }
    let legacy: Reply = serde_json::from_str(r#"{"kind":"Text","raw":"Old words.","warnings":[]}"#).unwrap();
    assert!(matches!(legacy, Reply::Text { original_text: None, .. }));
    let legacy: RecognitionResult = serde_json::from_str(
        r#"{"text":"Old words.","completeness":"complete","warnings":[]}"#,
    ).unwrap();
    assert!(legacy.original_text.is_none());
    assert_eq!(legacy.original_text(), "Old words.");
}

#[test]
fn background_helpers_get_the_two_highest_allowed_processors() {
    assert_eq!(background_affinity(0b1111_1111, 2), 0b1100_0000);
    assert_eq!(background_affinity(0b0101_0101, 2), 0b0101_0000);
    assert_eq!(background_affinity(0b0000_0011, 2), 0b0000_0011);
    assert_eq!(background_affinity(0b0000_0100, 2), 0b0000_0100);
}
