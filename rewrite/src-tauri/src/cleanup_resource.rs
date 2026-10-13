//! Round 6 cleanup resource hooks for the shared resource coordinator.
//!
//! Cleanup-only. This module never touches settings, history, clipboard or
//! other workers. It exposes safe observations and idle/job-cache transitions
//! the parent (resource manager) can call. All functions are prompt
//! best-effort: they never retain transcript text, never block on model load,
//! and never claim GPU execution without observed numeric offload (see
//! `crate::acceleration`, the sole owner of GPU labels).
//!
//! No-history contract: Rust holds no prompt cache beyond the in-flight HTTP
//! request body. llama-server slot KV (`cache_prompt:true`) is genuinely
//! erased per job with `POST /slots/{id}?action=erase` (pinned build 10900;
//! weights stay warm), and every job path erases under its owning SERVER
//! guard before release across success, fallback, error, Stop and cancel,
//! including notes generation. A failed erase forces process shutdown before
//! reuse (fail-closed). `end_job_report` / `clear_job_cache_report` expose
//! the actual confirmation; `private()` is the privacy gate. The launch
//! passes `--slot-save-path <owned empty temp dir>` solely to enable the
//! erase endpoint (the pinned server gates ALL slot actions on that flag;
//! save/restore are never called, the erase handler writes no files, and the
//! directory is verified to stay empty), plus `--cache-ram 0` to disable the
//! independent server-side RAM prompt cache, plus no `--log-prompts-dir`, so
//! there is no disk KV cache and no prompt log file. Only numeric `n_erased`
//! leaves this module; transcript text is never logged.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Per-job erasure confirmation. Re-exported from the cleanup owner so the
/// coordinator reads the exact same type. See `crate::cleanup` for field
/// semantics; `private()` is the privacy gate.
pub use super::JobEraseReport;

/// True while a cleanup/notes generation slot is held or a server child is
/// alive. Non-blocking; never starts a model.
pub fn is_busy() -> bool {
    super::is_busy()
}

/// Snapshot of the current cleanup worker for Setup/diagnostics:
/// `(pid, backend)` when a child is alive, else None. Backend is the
/// launch label (`cpu`/`cuda`/`metal`), not a capability claim. Only numeric
/// offload in `acceleration` counts as GPU execution.
pub fn worker_status() -> Option<(u32, String)> {
    super::runtime_status()
}

/// Release the warm model when idle. Returns true when a live child was
/// stopped. Safe to call from idle timers: active jobs hold the server mutex,
/// so `try_lock` fails and nothing is evicted. New activity cancels eviction
/// by acquiring the lock first (see parent coordinator).
pub fn release_idle() -> bool {
    super::release_idle()
}

/// End-of-job hook. Must be called at every job completion (success, fallback,
/// timeout, stop, cancel) so a warm model never carries prompt state into the
/// next request. Performs genuine slot-KV erasure
/// (`POST /slots/{id}?action=erase`, weights stay warm); on erase failure the
/// process is shut down before reuse. When `no_history` is true the
/// coordinator should additionally keep prompt logging and cache-to-disk off
/// (both already disabled in our launch args). Returns the time spent.
/// For the actual confirmation, prefer `end_job_report` and its `private()`.
pub fn end_job(no_history: bool) -> Duration {
    let started = Instant::now();
    let _ = no_history;
    let _ = super::end_job_cache();
    started.elapsed()
}

/// End-of-job hook with actual confirmation. Same genuine erasure as
/// `end_job`; the returned report tells the coordinator whether slot KV was
/// erased (`erased`, with `erased_tokens` summed), nothing was running
/// (`already_clear`), the process was shut down instead
/// (`shutdown_fallback`), or another job owns the erase (`busy_deferred` —
/// do not force a shutdown under it). `report.private()` is the privacy
/// gate: no known retained prompt KV.
pub fn end_job_report(no_history: bool) -> JobEraseReport {
    let _ = no_history;
    super::end_job_cache()
}

/// Best-effort per-job cache clear without unloading the model. Genuinely
/// erases slot KV via the owning guard when idle; reports `busy_deferred`
/// while a job owns the guard (that job erases under-guard itself).
/// Never logs transcript content. Prefer `clear_job_cache_report` for the
/// confirmation.
pub fn clear_job_cache() -> Duration {
    let started = Instant::now();
    let _ = super::end_job_cache();
    // Yield once so a woken idle timer can observe the released lock.
    std::thread::yield_now();
    started.elapsed()
}

/// Per-job cache clear with actual confirmation. See `end_job_report`.
pub fn clear_job_cache_report() -> JobEraseReport {
    let report = super::end_job_cache();
    std::thread::yield_now();
    report
}

/// Last erasure confirmation, for Setup/diagnostics. Never carries
/// transcript text (counts, flags and transport errors only).
pub fn last_erase_report() -> JobEraseReport {
    super::last_erase_report()
}

/// Warm the named model without blocking the caller. No-op when already warm
/// or when the model is `off`/`local-rules`. Used by the coordinator's
/// prewarm path; dictation prewarm stays on the existing `prewarm` entry.
pub fn warm(root: std::path::PathBuf, model: &str) {
    if matches!(model, "off" | "local-rules" | "") {
        return;
    }
    super::prewarm_model_bg(root, model.to_owned());
}

/// Shut the cleanup worker down (e.g. app exit, role switch, memory pressure).
/// Never touches other workers. Safe to call when idle or busy: a busy job
/// keeps its guard until its HTTP future drops, then observes shutdown.
pub fn shutdown_worker() {
    super::shutdown();
}

/// Record that cleanup was asked to stop (Stop button). Sets the caller's
/// flag; the in-flight chunk loop observes it at the next checkpoint and
/// preserves accepted edits plus the complete tail. This helper only flips
/// the flag the caller owns.
pub fn request_stop(stop_flag: &std::sync::atomic::AtomicBool) {
    stop_flag.store(true, Ordering::Release);
}

/// Record cancellation. Same ownership as `request_stop`.
pub fn request_cancel(cancel_flag: &std::sync::atomic::AtomicBool) {
    cancel_flag.store(true, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn hooks_never_panic_without_models() {
        // No server is running in unit tests; all hooks must be safe no-ops.
        let _ = is_busy();
        let _ = worker_status();
        let _ = release_idle();
        let _ = end_job(true);
        let _ = end_job(false);
        let _ = clear_job_cache();
        // Without a live server there is no retained KV: already clear.
        for report in [
            end_job_report(true),
            end_job_report(false),
            clear_job_cache_report(),
            last_erase_report(),
        ] {
            assert!(report.private(), "{report:?}");
            assert!(report.already_clear, "{report:?}");
            assert!(!report.shutdown_fallback, "{report:?}");
            assert!(report.error.is_none(), "{report:?}");
        }
        request_stop(&AtomicBool::new(false));
        request_cancel(&AtomicBool::new(false));
        shutdown_worker();
    }

    #[test]
    fn erase_report_privacy_gate_semantics() {
        // The gate must hold exactly for erased / already-clear / shutdown,
        // and must not claim privacy for a busy-deferred report owned by an
        // active job.
        let erased = JobEraseReport {
            erased: true,
            erased_tokens: 1745,
            already_clear: false,
            shutdown_fallback: false,
            busy_deferred: false,
            error: None,
            elapsed_ms: 1.0,
        };
        assert!(erased.private());
        let shutdown = JobEraseReport {
            erased: false,
            erased_tokens: 0,
            already_clear: false,
            shutdown_fallback: true,
            busy_deferred: false,
            error: Some("Slot erase failed: connection refused".into()),
            elapsed_ms: 1.0,
        };
        assert!(shutdown.private());
        let busy = JobEraseReport {
            erased: false,
            already_clear: false,
            shutdown_fallback: false,
            busy_deferred: true,
            ..erased.clone()
        };
        assert!(!busy.private());
    }
}
