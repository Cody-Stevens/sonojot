//! Bounded metadata-only diagnostics. Never pass native errors, text, or paths.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const LIMIT: u64 = 512 * 1024;
static LOG: OnceLock<Mutex<PathBuf>> = OnceLock::new();
static DUMPS_ALLOWED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn set_privacy(no_history: bool) {
    DUMPS_ALLOWED.store(!no_history, std::sync::atomic::Ordering::Release);
}

pub(crate) fn prepare_worker_diagnostics(command: &mut std::process::Command) {
    // Never inherit a stale dump setting from the invoking shell.
    command.env_remove("SONOJOT_CRASH_DUMP");
    if !DUMPS_ALLOWED.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let Some(log) = LOG.get() else { return };
    let Ok(root) = log.lock() else { return };
    let directory = root.join("crash-dumps");
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    prune_dumps(&directory);
    command.env(
        "SONOJOT_CRASH_DUMP",
        directory.join(format!("recognition-{}.dmp", uuid::Uuid::new_v4())),
    );
}

fn prune_dumps(directory: &Path) {
    // Only files created by this collector; never traverse links/subdirectories.
    let mut files: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?;
            if !name.starts_with("recognition-")
                || !name.ends_with(".dmp")
                || !e.file_type().ok()?.is_file()
            {
                return None;
            }
            let metadata = e.metadata().ok()?;
            Some((e.path(), metadata.modified().ok()?, metadata.len()))
        })
        .collect();
    files.sort_by_key(|(_, at, _)| std::cmp::Reverse(*at));
    let mut total = 0u64;
    for (index, (path, at, bytes)) in files.into_iter().enumerate() {
        total = total.saturating_add(bytes);
        if index >= 2
            || total > 64 * 1024 * 1024
            || at.elapsed().is_ok_and(|age| age.as_secs() > 7 * 24 * 3600)
        {
            let _ = fs::remove_file(path);
        }
    }
}

/// Capture a worker-native exception before Windows destroys its address space.
/// MiniDumpNormal includes stacks and modules, not full process/heap memory.
/// It may still contain private stack bytes, so no-history disables collection.
#[cfg(windows)]
pub(crate) fn install_worker_crash_capture() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Diagnostics::Debug::*;
    static PATH: OnceLock<Vec<u16>> = OnceLock::new();
    unsafe extern "system" fn capture(exception: *const EXCEPTION_POINTERS) -> i32 {
        use windows_sys::Win32::{Foundation::*, Storage::FileSystem::*, System::Threading::*};
        let Some(path) = PATH.get() else {
            return EXCEPTION_CONTINUE_SEARCH;
        };
        let file = CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        );
        if file != INVALID_HANDLE_VALUE {
            let info = MINIDUMP_EXCEPTION_INFORMATION {
                ThreadId: GetCurrentThreadId(),
                ExceptionPointers: exception as *mut _,
                ClientPointers: 0,
            };
            MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                MiniDumpNormal,
                &info,
                std::ptr::null(),
                std::ptr::null(),
            );
            CloseHandle(file);
        }
        EXCEPTION_CONTINUE_SEARCH
    }
    let Some(path) = std::env::var_os("SONOJOT_CRASH_DUMP") else {
        return;
    };
    std::env::remove_var("SONOJOT_CRASH_DUMP");
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.extension().is_none_or(|ext| ext != "dmp") {
        return;
    }
    if PATH
        .set(path.as_os_str().encode_wide().chain(Some(0)).collect())
        .is_ok()
    {
        unsafe {
            SetUnhandledExceptionFilter(Some(capture));
        }
    }
}
#[cfg(not(windows))]
pub(crate) fn install_worker_crash_capture() {}

pub(crate) fn initialize() {
    let root = crate::environment::var_os("DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| crate::store::default_root().ok());
    if let Some(root) = root {
        let directory = root.join("diagnostics");
        if fs::create_dir_all(&directory).is_ok() {
            let _ = LOG.set(Mutex::new(directory));
        }
    }
}

pub(crate) fn event(kind: &'static str, model: &str, pid: u32, session: u64, ms: u64, value: u64) {
    let Some(log) = LOG.get() else { return };
    let Ok(directory) = log.lock() else { return };
    append_line(
        &directory,
        &event_line(kind, model, pid, session, ms, value),
        LIMIT,
    );
}

pub(crate) fn capture_completion(model: &str, session: u64, capture: &crate::audio::CaptureDiagnostics) {
    let Some(log) = LOG.get() else { return };
    let Ok(directory) = log.lock() else { return };
    append_line(&directory, &capture_completion_line(model, session, capture), LIMIT);
}

fn capture_completion_line(model: &str, session: u64, capture: &crate::audio::CaptureDiagnostics) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&event_line(
        "capture-completion", model, std::process::id(), session,
        capture.post_roll_wait_ms, capture.post_roll_frames,
    )).expect("diagnostic metadata is valid JSON");
    value["capture"] = serde_json::to_value(capture).expect("capture metadata is serializable");
    value.to_string() + "\n"
}

fn event_line(
    kind: &'static str,
    model: &str,
    pid: u32,
    session: u64,
    ms: u64,
    value: u64,
) -> String {
    // IDs are catalog labels, never an arbitrary path or error message.
    let model = if model.len() <= 80
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        model
    } else {
        "invalid-model-id"
    };
    serde_json::json!({
        "v":1,"at_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "event":kind,"model":model,"pid":pid,"session":session,"ms":ms,"value":value
    })
    .to_string()
        + "\n"
}

fn append_line(directory: &Path, line: &str, limit: u64) {
    if line.len() as u64 > limit {
        return;
    }
    let path = directory.join("recognition.jsonl");
    if fs::metadata(&path).is_ok_and(|m| m.len().saturating_add(line.len() as u64) > limit) {
        let old = directory.join("recognition.2.jsonl");
        let previous = directory.join("recognition.1.jsonl");
        let _ = fs::remove_file(&old);
        if previous.exists() && fs::rename(&previous, &old).is_err() {
            return;
        }
        if fs::rename(&path, &previous).is_err() {
            return;
        }
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_metadata_records_real_sample_tail_separately_from_wall_wait() {
        let capture = crate::audio::CaptureDiagnostics {
            sample_rate: 48_000, post_roll_frames: 5_760, post_roll_ms: 120.0,
            post_roll_wait_ms: 600, post_roll_reason: "wall_timeout",
            cue_start_frame: Some(24_000), cue_end_frame: Some(31_440), cue_suppressed_frames: 7_440,
        };
        let line = capture_completion_line("fake-model", 7, &capture);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["event"], "capture-completion");
        assert_eq!(value["session"], 7);
        assert_eq!(value["capture"]["post_roll_frames"], 5_760);
        assert_eq!(value["capture"]["post_roll_ms"], 120.0);
        assert_eq!(value["capture"]["post_roll_wait_ms"], 600);
        assert_eq!(value["capture"]["cue_start_frame"], 24_000);
        assert_eq!(value["capture"].as_object().unwrap().len(), 8);
        assert!(!line.contains("samples"));
        assert!(!line.contains("text"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "isolated native exception fixture"]
    fn native_dump_fixture() {
        if std::env::var_os("SONOJOT_DUMP_TEST").is_none() {
            return;
        }
        install_worker_crash_capture();
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(0x0002);
            windows_sys::Win32::System::Diagnostics::Debug::RaiseException(
                0xE042_1001,
                1,
                0,
                std::ptr::null(),
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_worker_exception_writes_a_local_minidump() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let directory =
            std::env::temp_dir().join(format!("sonojot-dump-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("recognition-test.dmp");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "recognition_diagnostics::tests::native_dump_fixture",
            ])
            .env("SONOJOT_DUMP_TEST", "1")
            .env("SONOJOT_CRASH_DUMP", &path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let status = child.wait().unwrap();
        assert!(!status.success());
        let dump = fs::read(&path).unwrap();
        assert!(dump.starts_with(b"MDMP"));
        assert!(dump.len() < 64 * 1024 * 1024);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn diagnostic_metadata_rejects_paths_and_has_no_content_fields() {
        let line = event_line(
            "load-failed",
            r"C:\Users\someone\private-model",
            42,
            7,
            94_000,
            1,
        );
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["model"], "invalid-model-id");
        assert_eq!(value["pid"], 42);
        assert!(!line.contains("someone"));
        assert_eq!(value.as_object().unwrap().len(), 8);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&event_line(
                "load-start",
                "qwen3-asr-1.7b-fp16-cuda",
                42,
                7,
                0,
                0
            ))
            .unwrap()["model"],
            "qwen3-asr-1.7b-fp16-cuda"
        );
    }

    #[test]
    fn rotation_retains_only_three_bounded_files() {
        let directory =
            std::env::temp_dir().join(format!("sonojot-diagnostics-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let line = event_line("decode-complete", "fake-model", 42, 7, 3, 16_000);
        let limit = line.len() as u64 * 2;
        for _ in 0..11 {
            append_line(&directory, &line, limit);
        }
        let entries: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        assert_eq!(entries.len(), 3);
        for entry in entries {
            assert!(entry.metadata().unwrap().len() <= limit);
            for line in fs::read_to_string(entry.path()).unwrap().lines() {
                assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
            }
            fs::remove_file(entry.path()).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
}
