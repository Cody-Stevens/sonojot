//! In-place model updates. A model keeps its id (the family the user chose,
//! which settings and assignments point to) while the files underneath move to
//! a newer release. Nothing is re-chosen or re-assigned.
//!
//! - Speech models (`optional_asr`) list their earlier releases. Files from an
//!   earlier release stay "installed" and keep working; the update downloads
//!   only the files that changed into a staging folder (resuming after a
//!   failed connection), verifies them by hash, waits until SonoJot is idle,
//!   test-loads them in the speech runtime, and then swaps folders. The old
//!   files are deleted after the swap. Large downloads ask first.
//! - Cleanup (`punctuation_model`) packages are versioned folders and the
//!   newest verified one is used automatically. The newest published package
//!   downloads in the background when an older one is installed, is test-loaded
//!   in the cleanup helper when idle, and only then are the older published
//!   packages removed. If it cannot load, it is removed and the old one stays.
//!
//! Models the user added themselves, local test packages and catalogs without
//! an earlier release are never touched.

use crate::model_verification::Check;
use crate::optional_asr::{self, InstalledRelease, OptionalModel, PinnedFile, Release};
use crate::punctuation_model::Package;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    time::Duration,
};

const STATE_FILE: &str = ".sonojot-model-updates.json";
const FIRST_CHECK: Duration = Duration::from_secs(45);
const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
const IDLE_POLL: Duration = Duration::from_secs(5);

pub(crate) const DOWNLOAD_STOPPED: &str = "The download stopped. SonoJot will try again later.";
pub(crate) const NOT_LOADED: &str = "The update did not load on this computer. SonoJot kept the current version.";

/// One update the user can see (or that runs by itself).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUpdate {
    /// The model id after the update: the same id for speech models.
    pub id: String,
    /// Installed model ids this update replaces (rows that show it).
    pub replaces: Vec<String>,
    pub name: String,
    pub note: String,
    pub download_bytes: u64,
    /// Runs without asking (small built-in models).
    pub automatic: bool,
    /// available | queued | downloading | waiting | installing | failed
    pub state: String,
    pub downloaded_bytes: u64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) enum Plan {
    Speech { model: &'static OptionalModel, from: &'static Release },
    Cleanup { to: Package, from: Vec<Package> },
}

impl Plan {
    pub(crate) fn id(&self) -> &'static str {
        match self {
            Plan::Speech { model, .. } => model.id,
            Plan::Cleanup { to, .. } => to.id(),
        }
    }

    fn automatic(&self) -> bool {
        matches!(self, Plan::Cleanup { .. })
    }

    /// What a failure is remembered against: the target files.
    fn key(&self) -> String {
        match self {
            Plan::Speech { model, .. } => format!(
                "{}@{}",
                model.id,
                model.files.first().map_or("", |file| &file.sha256[..16])
            ),
            Plan::Cleanup { to, .. } => to.id().into(),
        }
    }

    fn replaces(&self) -> Vec<String> {
        match self {
            Plan::Speech { model, .. } => vec![model.id.into()],
            Plan::Cleanup { from, .. } => from.iter().map(|package| package.id().into()).collect(),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Plan::Speech { model, .. } => model.name,
            Plan::Cleanup { to, .. } => to.name(),
        }
    }

    fn note(&self) -> &'static str {
        match self {
            Plan::Speech { model, .. } => model.update_note,
            Plan::Cleanup { to, .. } => to.update_note(),
        }
    }

    fn download_bytes(&self, root: &Path, check: Check) -> u64 {
        match self {
            Plan::Speech { model, from } => changed_files(model, from).map(|file| file.bytes).sum(),
            Plan::Cleanup { to, .. } => {
                if to.model_status(root, check).ready() { 0 } else { to.download_bytes(root, check) }
            }
        }
    }
}

/// Files of the current release that the installed one does not already have.
fn changed_files<'a>(model: &'a OptionalModel, from: &'a Release) -> impl Iterator<Item = &'a PinnedFile> + 'a {
    model.files.iter().filter(move |file| reusable(from, file).is_none())
}

fn reusable<'a>(from: &'a Release, file: &PinnedFile) -> Option<&'a PinnedFile> {
    from.files
        .iter()
        .find(|old| old.relative == file.relative && old.sha256.eq_ignore_ascii_case(file.sha256) && old.bytes == file.bytes)
}

/// Every update that applies to what is installed under `root` right now.
pub(crate) fn plans(root: &Path, check: Check) -> Vec<Plan> {
    let mut out = Vec::new();
    for model in optional_asr::catalog() {
        if model.previous.is_empty() || !optional_asr::obtainable(model) {
            continue;
        }
        if let InstalledRelease::Previous(from) = optional_asr::installed_release(root, model, check) {
            out.push(Plan::Speech { model, from });
        }
    }
    if let Some(plan) = cleanup_plan(root, check) {
        out.push(plan);
    }
    out
}

fn cleanup_plan(root: &Path, check: Check) -> Option<Plan> {
    if !crate::diarization_model::AVAILABLE {
        return None;
    }
    cleanup_plan_with(&|package| package.model_status(root, check).ready())
}

fn cleanup_plan_with(ready: &dyn Fn(Package) -> bool) -> Option<Plan> {
    let to = Package::offered();
    if !to.released() {
        return None;
    }
    // Only SonoJot's own published packages older than the offered one; local
    // test packages and anything unpublished are left alone.
    let older = Package::NEWEST_FIRST.into_iter().skip_while(|package| *package != to).skip(1);
    let from: Vec<Package> = older.filter(|package| package.released() && ready(*package)).collect();
    (!from.is_empty()).then_some(Plan::Cleanup { to, from })
}

// ---------------------------------------------------------------------------
// Running an update. The hooks are the outside world, so tests can drive it.
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Connection or disk trouble: keep the old version, try again later.
    Retry(String),
    /// The new files did not verify or load: keep the old version for good.
    Rejected(String),
    Cancelled,
}

pub(crate) struct Hooks<'a> {
    /// SonoJot is not recording, processing or installing, and this model is not running.
    pub idle: &'a dyn Fn(&str) -> bool,
    /// Make sure no recognizer holds this model's files. False: try again shortly.
    pub release_speech: &'a dyn Fn(&str) -> bool,
    /// Load and run the staged model. The path is a models root of its own
    /// (`optional/<id>/` holds the verified new files), so the real recognition
    /// worker can open it exactly as it opens an installed model.
    pub probe_speech: &'a dyn Fn(&str, &Path) -> Result<(), String>,
    pub probe_cleanup: &'a dyn Fn(Package) -> Result<(), String>,
    pub fetch: &'a dyn Fn(&str, &Path, &PinnedFile, &AtomicBool, &dyn Fn(u64, u64)) -> Result<(), String>,
    pub install_cleanup: &'a dyn Fn(Package, &AtomicBool, &dyn Fn(u64, u64)) -> Result<(), String>,
    pub cleanup_ready: &'a dyn Fn(Package) -> bool,
    /// Pause between idle checks. False when cancelled.
    pub pause: &'a dyn Fn(&AtomicBool) -> bool,
    pub phase: &'a dyn Fn(&'static str),
}

/// A throwaway models root for one update; the new files sit in
/// `optional/<id>/` inside it.
fn staging_dir(root: &Path, model: &OptionalModel) -> PathBuf {
    let tag = if model.revision.is_empty() {
        model.files.first().map_or("dev", |file| &file.sha256[..12])
    } else {
        &model.revision[..model.revision.len().min(12)]
    };
    optional_asr::optional_root(root).join(format!(".update-{}-{tag}", model.id))
}

fn remove_stale_staging(root: &Path, keep: &[PathBuf]) {
    let Ok(entries) = fs::read_dir(optional_asr::optional_root(root)) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".update-") && !keep.contains(&path) && path.is_dir() {
            if optional_asr::ensure_within(root, &path).is_ok() {
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
}

pub(crate) fn update_speech(
    root: &Path,
    model: &'static OptionalModel,
    from: &'static Release,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    hooks: &Hooks<'_>,
) -> Result<(), Failure> {
    let dest = optional_asr::model_dir(root, model.id);
    let probe_root = staging_dir(root, model);
    let staging = optional_asr::model_dir(&probe_root, model.id);
    let retry = |e: String| Failure::Retry(e);
    fs::create_dir_all(&staging).map_err(|e| retry(format!("Cannot prepare the update folder: {e}")))?;
    optional_asr::ensure_within(root, &staging).map_err(retry)?;
    let total: u64 = changed_files(model, from).map(|file| file.bytes).sum();
    let mut done = 0u64;
    progress(0, total);
    (hooks.phase)("downloading");
    for file in model.files {
        if cancel.load(Ordering::Acquire) {
            return Err(Failure::Cancelled);
        }
        let target = staging.join(file.relative);
        if optional_asr::file_matches(&target, file).unwrap_or(false) {
            if reusable(from, file).is_none() {
                done += file.bytes;
            }
            continue;
        }
        if reusable(from, file).is_some() {
            // Unchanged between releases: copy it from the installed folder.
            fs::copy(dest.join(file.relative), &target)
                .map_err(|e| retry(format!("Cannot copy {}: {e}", file.relative)))?;
            if !optional_asr::file_matches(&target, file).unwrap_or(false) {
                let _ = fs::remove_file(&target);
                return Err(retry(format!("The installed {} changed. Try again later.", file.relative)));
            }
            continue;
        }
        let url = optional_asr::download_url(model, file).ok_or_else(|| retry(optional_asr::UNPUBLISHED.into()))?;
        let part = staging.join(format!("{}.part", file.relative));
        let base = done;
        (hooks.fetch)(&url, &part, file, cancel, &|got, _| progress(base + got.min(file.bytes), total)).map_err(
            |e| {
                if cancel.load(Ordering::Acquire) {
                    Failure::Cancelled
                } else if e.contains("checksum") {
                    Failure::Rejected(e)
                } else {
                    Failure::Retry(e)
                }
            },
        )?;
        fs::rename(&part, &target).map_err(|e| retry(format!("Cannot stage {}: {e}", file.relative)))?;
        done += file.bytes;
        progress(done, total);
    }
    // Every staged file is checked against the new pins before anything else.
    for file in model.files {
        if !optional_asr::file_matches(&staging.join(file.relative), file).unwrap_or(false) {
            let _ = fs::remove_dir_all(&probe_root);
            return Err(Failure::Rejected(format!("The downloaded {} did not match its pinned checksum.", file.relative)));
        }
    }
    loop {
        (hooks.phase)("waiting");
        if !dest.is_dir() {
            // Removed meanwhile: nothing to update.
            let _ = fs::remove_dir_all(&probe_root);
            return Err(Failure::Cancelled);
        }
        if (hooks.idle)(model.id) && (hooks.release_speech)(model.id) {
            (hooks.phase)("installing");
            if let Err(error) = (hooks.probe_speech)(model.id, &probe_root) {
                let _ = fs::remove_dir_all(&probe_root);
                return Err(Failure::Rejected(format!("{NOT_LOADED} ({error})")));
            }
            // Same guard as installs and removals; busy means try again shortly.
            if let Ok(_gate) = optional_asr::try_install_gate() {
                if (hooks.idle)(model.id) && (hooks.release_speech)(model.id) {
                    let _activity = crate::resource_manager::begin_update_activity(
                        crate::resource_manager::UpdateActivity::Models,
                    )
                    .map_err(retry)?;
                    optional_asr::transactional_activate(root, &staging, &dest, "speech model").map_err(retry)?;
                    let _ = fs::remove_dir_all(&probe_root);
                    // Verify at the final path now, so the next dictation does not hash.
                    Check::Blocking.files(root, &dest, model.files);
                    crate::model_manager::invalidate_sizes();
                    return Ok(());
                }
            }
        }
        if !(hooks.pause)(cancel) {
            return Err(Failure::Cancelled);
        }
    }
}

pub(crate) fn update_cleanup(
    root: &Path,
    to: Package,
    from: &[Package],
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
    hooks: &Hooks<'_>,
) -> Result<(), Failure> {
    let fresh = !(hooks.cleanup_ready)(to);
    if fresh {
        (hooks.phase)("downloading");
        (hooks.install_cleanup)(to, cancel, progress).map_err(|e| {
            if cancel.load(Ordering::Acquire) {
                Failure::Cancelled
            } else if e.contains("checksum") {
                Failure::Rejected(e)
            } else {
                Failure::Retry(e)
            }
        })?;
    }
    loop {
        (hooks.phase)("waiting");
        if (hooks.idle)(to.id()) {
            (hooks.phase)("installing");
            if let Err(error) = (hooks.probe_cleanup)(to) {
                // A package this update brought in goes; the old one keeps working.
                if fresh {
                    remove_package(root, to);
                }
                return Err(Failure::Rejected(format!("{NOT_LOADED} ({error})")));
            }
            if let Ok(_gate) = optional_asr::try_install_gate() {
                if (hooks.idle)(to.id()) {
                    let _activity = crate::resource_manager::begin_update_activity(
                        crate::resource_manager::UpdateActivity::Models,
                    )
                    .map_err(|e| Failure::Retry(e))?;
                    for package in from {
                        remove_package(root, *package);
                    }
                    crate::model_manager::invalidate_sizes();
                    return Ok(());
                }
            }
        }
        if !(hooks.pause)(cancel) {
            return Err(Failure::Cancelled);
        }
    }
}

fn remove_package(root: &Path, package: Package) {
    let dest = package.path(root);
    let parent = root.join("punctuation");
    if dest.is_dir()
        && optional_asr::ensure_within(root, &parent).is_ok()
        && optional_asr::ensure_within(&parent, &dest).is_ok()
    {
        let _ = fs::remove_dir_all(&dest);
    }
    crate::resource_manager::note_model_removed(package.id());
}

// ---------------------------------------------------------------------------
// Remembered rejections, so a build that cannot load here is not fetched again
// on every start. A new app version tries again.
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct Remembered {
    #[serde(default)]
    rejected: Vec<String>,
}

fn remembered_key(plan: &Plan) -> String {
    format!("{}@{}", plan.key(), env!("CARGO_PKG_VERSION"))
}

fn load_remembered(root: &Path) -> Remembered {
    fs::read(root.join(STATE_FILE))
        .ok()
        .filter(|bytes| bytes.len() < 64 * 1024)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn remember_rejection(root: &Path, plan: &Plan) {
    let mut state = load_remembered(root);
    let key = remembered_key(plan);
    if !state.rejected.contains(&key) {
        state.rejected.push(key);
        state.rejected.retain(|item| item.ends_with(env!("CARGO_PKG_VERSION")));
        if let Ok(bytes) = serde_json::to_vec(&state) {
            let _ = fs::write(root.join(STATE_FILE), bytes);
        }
    }
}

fn rejected(root: &Path, plan: &Plan) -> bool {
    load_remembered(root).rejected.contains(&remembered_key(plan))
}

// ---------------------------------------------------------------------------
// Background worker and the job table the UI reads.
// ---------------------------------------------------------------------------

struct Job {
    state: &'static str,
    downloaded: u64,
    total: u64,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Job>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

fn approved() -> &'static Mutex<HashSet<String>> {
    static APPROVED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    APPROVED.get_or_init(Default::default)
}

fn wake() -> &'static (Mutex<bool>, Condvar) {
    static WAKE: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    WAKE.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

fn signal() {
    let (flag, condvar) = wake();
    if let Ok(mut flag) = flag.lock() {
        *flag = true;
        condvar.notify_all();
    }
}

/// Sleep up to `limit`, returning early when an update was requested.
fn sleep_or_wake(limit: Duration) {
    let (flag, condvar) = wake();
    let Ok(guard) = flag.lock() else { return };
    let (mut guard, _) = condvar
        .wait_timeout_while(guard, limit, |woken| !*woken)
        .unwrap_or_else(|e| e.into_inner());
    *guard = false;
}

fn set_job(id: &str, update: impl FnOnce(&mut Job)) {
    if let Ok(mut jobs) = jobs().lock() {
        if let Some(job) = jobs.get_mut(id) {
            update(job);
        }
    }
}

/// What the UI shows: every applicable update with its job state. Plans that
/// only tidy up (nothing to download) are not listed.
pub(crate) fn list(root: &Path) -> Vec<ModelUpdate> {
    let jobs = jobs().lock().ok();
    let approved = approved().lock().ok();
    plans(root, Check::Background)
        .into_iter()
        .filter_map(|plan| {
            let bytes = plan.download_bytes(root, Check::Background);
            let job = jobs.as_ref().and_then(|jobs| jobs.get(plan.id()));
            if bytes == 0 && job.is_none() {
                return None;
            }
            let queued = approved.as_ref().is_some_and(|set| set.contains(plan.id()));
            let (state, downloaded, error) = match job {
                Some(job) => (job.state, job.downloaded, job.error.clone()),
                None if queued => ("queued", 0, None),
                None => ("available", 0, None),
            };
            Some(ModelUpdate {
                id: plan.id().into(),
                replaces: plan.replaces(),
                name: plan.name().into(),
                note: plan.note().into(),
                download_bytes: bytes,
                automatic: plan.automatic(),
                state: state.into(),
                downloaded_bytes: downloaded,
                error,
            })
        })
        .collect()
}

pub(crate) fn approve(root: &Path, id: Option<&str>) {
    if let Ok(mut set) = approved().lock() {
        for plan in plans(root, Check::Background) {
            if id.is_none_or(|id| id == plan.id()) {
                set.insert(plan.id().into());
            }
        }
    }
    if let Ok(mut jobs) = jobs().lock() {
        // "Try again" clears a shown failure.
        jobs.retain(|key, job| !(job.state == "failed" && id.is_none_or(|id| id == key)));
    }
    signal();
}

pub(crate) fn cancel(id: &str) {
    if let Ok(mut set) = approved().lock() {
        set.remove(id);
    }
    if let Ok(jobs) = jobs().lock() {
        if let Some(job) = jobs.get(id) {
            job.cancel.store(true, Ordering::Release);
        }
    }
    signal();
}

pub(crate) fn start(runtime: &Arc<crate::app::Runtime>, app: &tauri::AppHandle) {
    let weak = Arc::downgrade(runtime);
    let app = app.clone();
    let _ = std::thread::Builder::new().name("model-updates".into()).spawn(move || {
        sleep_or_wake(FIRST_CHECK);
        loop {
            let Some(runtime) = weak.upgrade() else { break };
            let ready = !crate::onboarding::needs_setup(&runtime)
                && runtime.snapshot().is_ok_and(|s| crate::onboarding::terms_accepted(&s.settings));
            if ready {
                tick(&runtime, &app);
            }
            drop(runtime);
            sleep_or_wake(CHECK_EVERY);
        }
    });
}

fn tick(runtime: &Arc<crate::app::Runtime>, app: &tauri::AppHandle) {
    let root = runtime.models_root.clone();
    let all = plans(&root, Check::Blocking);
    let keep: Vec<PathBuf> = all
        .iter()
        .filter_map(|plan| match plan {
            Plan::Speech { model, .. } => Some(staging_dir(&root, model)),
            Plan::Cleanup { .. } => None,
        })
        .collect();
    remove_stale_staging(&root, &keep);
    for plan in all {
        let id = plan.id();
        let asked = approved().lock().is_ok_and(|set| set.contains(id));
        let silent_tidy = plan.automatic() && plan.download_bytes(&root, Check::Blocking) == 0;
        if !(asked || (plan.automatic() && (silent_tidy || !rejected(&root, &plan)))) {
            continue;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if let Ok(mut jobs) = jobs().lock() {
            jobs.insert(
                id.into(),
                Job { state: "downloading", downloaded: 0, total: 0, error: None, cancel: cancel.clone() },
            );
        }
        let progress_id = id.to_owned();
        let progress = move |done: u64, total: u64| {
            set_job(&progress_id, |job| {
                job.downloaded = done;
                job.total = total;
            });
        };
        let phase_id = id.to_owned();
        let phase = move |state: &'static str| set_job(&phase_id, |job| job.state = state);
        let models_root = root.clone();
        let snapshot_runtime = runtime.clone();
        let idle = move |id: &str| {
            snapshot_runtime.snapshot().is_ok_and(|s| crate::model_manager::require_idle_snapshot(&s).is_ok())
                && !crate::model_manager::install_in_progress()
                && !crate::resource_manager::is_model_active(id)
        };
        let release_speech = |id: &str| {
            let (active, loaded) = crate::resource_manager::model_state_blocking(id);
            if active {
                return false;
            }
            if !loaded || !crate::resource_manager::request_asr_release() {
                return true;
            }
            crate::resource_manager::await_asr_release(&AtomicBool::new(false), Duration::from_secs(10)).is_ok()
        };
        let runtime_root = root.clone();
        let probe_speech = move |id: &str, staged: &Path| probe_speech_in_worker(&runtime_root, id, staged);
        let cleanup_root = root.clone();
        let probe_cleanup = move |to: Package| probe_cleanup(&cleanup_root, to);
        let fetch = |url: &str, dest: &Path, file: &PinnedFile, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)| {
            optional_asr::download_resumable(url, dest, file.bytes, file.sha256, "speech model", cancel, progress)
        };
        let install_root = root.clone();
        let install_cleanup = move |to: Package, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)| {
            crate::punctuation_model::install_package(&install_root, to, cancel, progress, &|| {})
        };
        let ready_root = root.clone();
        let cleanup_ready = move |package: Package| package.model_status(&ready_root, Check::Blocking).ready();
        let pause = |cancel: &AtomicBool| {
            sleep_or_wake(IDLE_POLL);
            !cancel.load(Ordering::Acquire)
        };
        let hooks = Hooks {
            idle: &idle,
            release_speech: &release_speech,
            probe_speech: &probe_speech,
            probe_cleanup: &probe_cleanup,
            fetch: &fetch,
            install_cleanup: &install_cleanup,
            cleanup_ready: &cleanup_ready,
            pause: &pause,
            phase: &phase,
        };
        let result = match &plan {
            Plan::Speech { model, from } => update_speech(&models_root, model, from, &cancel, &progress, &hooks),
            Plan::Cleanup { to, from } => update_cleanup(&models_root, *to, from, &cancel, &progress, &hooks),
        };
        let mut drop_job = true;
        match result {
            Ok(()) => {
                if let Ok(mut set) = approved().lock() {
                    set.remove(id);
                }
                crate::recognition_diagnostics::event("model-update-installed", id, 0, 0, 0, 0);
            }
            Err(Failure::Cancelled) => {
                if let Plan::Speech { model, .. } = &plan {
                    let _ = fs::remove_dir_all(staging_dir(&root, model));
                }
            }
            Err(Failure::Retry(error)) => {
                // Automatic updates stay quiet offline; asked-for ones say so and retry later.
                crate::recognition_diagnostics::event("model-update-retry", id, 0, 0, 0, 0);
                if asked {
                    drop_job = false;
                    let _ = error;
                    set_job(id, |job| {
                        job.state = "failed";
                        job.error = Some(DOWNLOAD_STOPPED.into());
                    });
                }
            }
            Err(Failure::Rejected(error)) => {
                crate::recognition_diagnostics::event("model-update-rejected", id, 0, 0, 0, 0);
                remember_rejection(&root, &plan);
                if let Ok(mut set) = approved().lock() {
                    set.remove(id);
                }
                drop_job = false;
                let message = error.split(" (").next().unwrap_or(NOT_LOADED).to_owned();
                set_job(id, |job| {
                    job.state = "failed";
                    job.error = Some(if message.contains("checksum") {
                        "The update did not download correctly. SonoJot kept the current version.".into()
                    } else {
                        message
                    });
                });
            }
        }
        if drop_job {
            if let Ok(mut jobs) = jobs().lock() {
                jobs.remove(id);
            }
        }
        let _ = optional_asr::refresh_snapshot(app, runtime);
    }
}

/// Load the staged speech model in the real recognition worker (its own
/// process, as for dictation) and transcribe a second of quiet audio. The
/// staged root borrows the installed speech runtime by link or copy.
pub(crate) fn probe_speech_in_worker(root: &Path, id: &str, staged: &Path) -> Result<(), String> {
    let from = optional_asr::sherpa_runtime_dir(root);
    let to = optional_asr::sherpa_runtime_dir(staged);
    fs::create_dir_all(&to).map_err(|e| format!("Cannot prepare the test runtime: {e}"))?;
    for file in optional_asr::RUNTIME_FILES {
        let (source, target) = (from.join(file.relative), to.join(file.relative));
        if !target.exists() && fs::hard_link(&source, &target).is_err() {
            fs::copy(&source, &target).map_err(|e| format!("Cannot prepare the test runtime: {e}"))?;
        }
    }
    let wav = staged.join("probe.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&wav, spec).map_err(|e| e.to_string())?;
    for i in 0..16_000u32 {
        writer.write_sample(((i as f32 * 0.05).sin() * 30.0) as i16).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    let cancel = AtomicBool::new(false);
    let mut recognizer = crate::asr::Recognizer::load_interruptible(staged, id, &cancel)?;
    let result = crate::asr::transcribe_file_with(&mut recognizer, &wav, &cancel).map(drop);
    drop(recognizer);
    result
}

/// Load the newly installed cleanup package in the real helper process.
fn probe_cleanup(root: &Path, to: Package) -> Result<(), String> {
    let selection = crate::punctuation::Selection::new(root, None);
    if selection.package_directory() != Some(to.path(root).as_path())
        && selection.label() != to.id()
    {
        return Err(format!("{} is not the package SonoJot would use", to.id()));
    }
    crate::punctuation_worker::wait_ready(&selection, &AtomicBool::new(false))
        .map(drop)
        .map_err(|failure| format!("{failure:?}"))
}

#[tauri::command]
pub async fn get_model_updates(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
) -> Result<Vec<ModelUpdate>, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list(&runtime.models_root))
        .await
        .map_err(|e| e.to_string())
}

/// Start one update (`id`) or every waiting one (no `id`).
#[tauri::command]
pub async fn start_model_update(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
    id: Option<String>,
) -> Result<Vec<ModelUpdate>, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        approve(&runtime.models_root, id.as_deref());
        list(&runtime.models_root)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_model_update(
    state: tauri::State<'_, Arc<crate::app::Runtime>>,
    id: String,
) -> Result<Vec<ModelUpdate>, String> {
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        cancel(&id);
        list(&runtime.models_root)
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "model_updates_tests.rs"]
mod tests;
