//! Backend-owned OTA. Only restart_to_update can consume a verified package.
//! The webview has no updater/process plugin permissions or update resource IDs.
use crate::{app::Runtime, domain::Snapshot, resource_manager};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tauri::AppHandle;
use tauri_plugin_updater::{RemoteRelease, Update, UpdaterExt};

const CHECK_FAILED: &str = "Updates are unavailable. Try again later.";
const DOWNLOAD_FAILED: &str = "The update could not be downloaded. Check for updates to try again.";
const INSTALL_FAILED: &str = "The update could not be installed. Check for updates to try again.";
const FIRST_CHECK: Duration = Duration::from_secs(30);
/// Opening or returning to the main window checks again once the last check
/// is this old. Privacy notice section 3 describes this cadence.
const FOCUS_CHECK_AFTER: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    Available {
        version: String,
        notes: Option<String>,
        size: Option<u64>,
    },
    Downloading {
        percent: Option<u8>,
    },
    Ready {
        version: String,
        notes: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub current_version: String,
    pub status: UpdateStatus,
    pub restart_blocked_reason: Option<String>,
    pub installing: bool,
    /// The user chose Later on the main-window notice for this offered
    /// version. Held in memory, so it lasts until SonoJot next starts.
    pub notice_dismissed: bool,
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").into(),
            status: UpdateStatus::Idle,
            restart_blocked_reason: None,
            installing: false,
            notice_dismissed: false,
        }
    }
}

/// What prompted an automatic check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    Timer,
    Focus,
}

#[derive(Clone, Copy, Debug)]
struct Moment {
    mono: Instant,
    wall: SystemTime,
}

impl Moment {
    fn now() -> Self {
        Self {
            mono: Instant::now(),
            wall: SystemTime::now(),
        }
    }
}

/// When automatic checks are due: once about 30 seconds after launch, then
/// about six hours after the most recent check, and on window focus once the
/// most recent check is an hour old. Any check, including Check now, restarts
/// both clocks, so focus and the timer never stack requests.
#[derive(Debug)]
struct Schedule {
    launched: Instant,
    last: Option<Moment>,
    interval: Duration,
}

impl Schedule {
    fn new(launched: Instant) -> Self {
        Self {
            launched,
            last: None,
            // Unused until the first check records a jittered interval.
            interval: Duration::MAX,
        }
    }

    fn since_last(&self, now: Moment) -> Option<Duration> {
        let last = self.last?;
        // Monotonic time can pause while the computer sleeps and the wall
        // clock can move backwards; whichever has advanced further counts.
        Some(
            now.mono
                .saturating_duration_since(last.mono)
                .max(now.wall.duration_since(last.wall).unwrap_or_default()),
        )
    }

    fn due(&self, trigger: Trigger, now: Moment) -> bool {
        match (self.since_last(now), trigger) {
            (None, Trigger::Timer) => {
                now.mono.saturating_duration_since(self.launched) >= FIRST_CHECK
            }
            // The launch check comes first; opening the window never preempts it.
            (None, Trigger::Focus) => false,
            (Some(elapsed), Trigger::Timer) => elapsed >= self.interval,
            (Some(elapsed), Trigger::Focus) => elapsed >= FOCUS_CHECK_AFTER,
        }
    }

    fn record(&mut self, now: Moment, jitter: u64) {
        self.last = Some(now);
        self.interval = next_check_delay(jitter);
    }
}

fn jitter() -> u64 {
    uuid::Uuid::new_v4().as_u128() as u64
}

struct Package<R> {
    release: R,
    bytes: Vec<u8>,
}

struct State<R> {
    status: UpdateStatus,
    available: Option<R>,
    package: Option<Package<R>>,
    installing: bool,
    schedule: Schedule,
    /// Version most recently found by a check, kept through download.
    offered: Option<String>,
    dismissed: Option<String>,
}

impl<R> Default for State<R> {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            available: None,
            package: None,
            installing: false,
            schedule: Schedule::new(Instant::now()),
            offered: None,
            dismissed: None,
        }
    }
}

pub(crate) struct Service<R> {
    state: Mutex<State<R>>,
}
pub(crate) type UpdateService = Service<Update>;

impl<R> Default for Service<R> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
        }
    }
}

impl<R> Service<R> {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State<R>>, String> {
        self.state.lock().map_err(|_| CHECK_FAILED.into())
    }

    pub(crate) fn snapshot(&self) -> Result<UpdateSnapshot, String> {
        let state = self.lock()?;
        Ok(UpdateSnapshot {
            status: state.status.clone(),
            installing: state.installing,
            restart_blocked_reason: resource_manager::update_blocked_reason(),
            notice_dismissed: state.offered.is_some() && state.dismissed == state.offered,
            ..UpdateSnapshot::default()
        })
    }

    fn begin_check(&self) -> Result<bool, String> {
        let mut state = self.lock()?;
        if state.installing
            || matches!(
                state.status,
                UpdateStatus::Checking
                    | UpdateStatus::Available { .. }
                    | UpdateStatus::Downloading { .. }
                    | UpdateStatus::Ready { .. }
            )
        {
            return Ok(false);
        }
        state.package = None;
        state.available = None;
        state.status = UpdateStatus::Checking;
        state.schedule.record(Moment::now(), jitter());
        Ok(true)
    }

    /// Claims an automatic check if one is due. A claim restarts the clocks
    /// even when the check itself is skipped (an update is already offered).
    fn claim_automatic(&self, trigger: Trigger, now: Moment, jitter: u64) -> bool {
        let Ok(mut state) = self.lock() else {
            return false;
        };
        if !state.schedule.due(trigger, now) {
            return false;
        }
        state.schedule.record(now, jitter);
        true
    }

    fn dismiss_notice(&self) -> Result<(), String> {
        let mut state = self.lock()?;
        state.dismissed = state.offered.clone();
        Ok(())
    }

    fn set_status(&self, status: UpdateStatus) -> bool {
        let Ok(mut state) = self.lock() else {
            return false;
        };
        if state.status == status {
            return false;
        }
        state.status = status;
        true
    }

    fn begin_download(&self) -> Result<Option<R>, String> {
        let mut state = self.lock()?;
        if state.installing
            || matches!(
                state.status,
                UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. }
            )
        {
            return Ok(None);
        }
        if !matches!(state.status, UpdateStatus::Available { .. }) {
            return Err("Check for updates before downloading.".into());
        }
        let release = state
            .available
            .take()
            .ok_or("No update is available to download.")?;
        state.status = UpdateStatus::Downloading { percent: None };
        Ok(Some(release))
    }

    fn claim_install(&self) -> Result<(Package<R>, resource_manager::UpdateInstallLease), String> {
        let mut state = self.lock()?;
        if state.installing || !matches!(state.status, UpdateStatus::Ready { .. }) {
            return Err("No update is ready to install.".into());
        }
        // Atomic with every whole-job start and inference acquire. No locks
        // are held during installation; the lease keeps admission closed.
        let lease = resource_manager::begin_update_install()?;
        let package = state
            .package
            .take()
            .ok_or("No update is ready to install.")?;
        state.installing = true;
        Ok((package, lease))
    }

    fn install_failed(&self) {
        if let Ok(mut state) = self.lock() {
            state.installing = false;
            state.status = UpdateStatus::Error {
                message: INSTALL_FAILED.into(),
            };
        }
    }
}

type Task<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type Progress<'a> = Box<dyn FnMut(usize, Option<u64>) + Send + 'a>;

/// The production adapter delegates signature verification to the plugin.
/// Tests inject a local backend; they cannot reach the network or installer.
trait Backend {
    type Release: Send + Sync;
    fn check(&self) -> Task<'_, Result<Option<Self::Release>, ()>>;
    fn describe(release: &Self::Release) -> (String, Option<String>, Option<u64>);
    fn download<'a>(
        &'a self,
        release: &'a Self::Release,
        progress: Progress<'a>,
    ) -> Task<'a, Result<Vec<u8>, ()>>;
}

struct PluginBackend(AppHandle);

impl Backend for PluginBackend {
    type Release = Update;

    fn check(&self) -> Task<'_, Result<Option<Update>, ()>> {
        Box::pin(async {
            let Some(target) = platform_key(std::env::consts::OS, std::env::consts::ARCH) else {
                return Ok(None);
            };
            let updater = self
                .0
                .updater_builder()
                .target(target)
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| ())?;
            let mut release = updater.check().await.map_err(|_| ())?;
            if let Some(update) = release.as_mut() {
                validate_manifest(&update.raw_json, target)?;
                // Plugin check and download use separate clients/timeouts.
                update.timeout = Some(Duration::from_secs(15 * 60));
            }
            Ok(release)
        })
    }

    fn describe(release: &Update) -> (String, Option<String>, Option<u64>) {
        (
            release.version.clone(),
            release
                .body
                .as_ref()
                .map(|notes| notes.chars().take(32_768).collect()),
            platform_key(std::env::consts::OS, std::env::consts::ARCH).and_then(|target| {
                release
                    .raw_json
                    .get("platforms")?
                    .get(target)?
                    .get("size")?
                    .as_u64()
            }),
        )
    }

    fn download<'a>(
        &'a self,
        release: &'a Update,
        mut progress: Progress<'a>,
    ) -> Task<'a, Result<Vec<u8>, ()>> {
        // on_download_finish runs BEFORE verification in the plugin. Only a
        // successful download return may publish Ready or retain these bytes.
        Box::pin(async move { release.download(&mut progress, || {}).await.map_err(|_| ()) })
    }
}

fn platform_key(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("macos", "aarch64") => Some("darwin-aarch64"),
        _ => None,
    }
}

fn validate_manifest(value: &serde_json::Value, target: &str) -> Result<(), ()> {
    // Require the static per-platform contract; a dynamic single URL must not
    // accidentally offer a Windows package to a Mac (or the converse).
    if !value
        .get("platforms")
        .is_some_and(|p| p.get(target).is_some())
    {
        return Err(());
    }
    let release: RemoteRelease = serde_json::from_value(value.clone()).map_err(|_| ())?;
    let url = release.download_url(target).map_err(|_| ())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || release.signature(target).map_err(|_| ())?.trim().is_empty()
    {
        return Err(());
    }
    Ok(())
}

fn percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    total
        .filter(|total| *total > 0)
        .map(|total| ((u128::from(downloaded) * 100 / u128::from(total)).min(99)) as u8)
}

async fn check<B: Backend>(service: &Service<B::Release>, backend: &B, emit: impl Fn() + Sync) {
    let result = async {
        let Some(release) = backend.check().await? else {
            service.set_status(UpdateStatus::Idle);
            return Ok::<(), ()>(());
        };
        let (version, notes, size) = B::describe(&release);
        let mut state = service.lock().map_err(|_| ())?;
        state.available = Some(release);
        state.offered = Some(version.clone());
        state.status = UpdateStatus::Available {
            version,
            notes,
            size,
        };
        Ok(())
    }
    .await;
    if result.is_err() {
        service.set_status(UpdateStatus::Error {
            message: CHECK_FAILED.into(),
        });
        // Never format plugin/HTTP errors: they can contain URLs or credentials.
        crate::recognition_diagnostics::event(
            "update-check-failed",
            "",
            std::process::id(),
            0,
            0,
            0,
        );
    }
    emit();
}

async fn download<B: Backend>(
    service: &Service<B::Release>,
    backend: &B,
    release: B::Release,
    emit: impl Fn() + Sync,
) {
    let result = async {
        let (version, notes, _) = B::describe(&release);
        let mut downloaded = 0u64;
        let bytes = backend
            .download(
                &release,
                Box::new(|chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    if service.set_status(UpdateStatus::Downloading {
                        percent: percent(downloaded, total),
                    }) {
                        emit();
                    }
                }),
            )
            .await?;
        let mut state = service.lock().map_err(|_| ())?;
        state.package = Some(Package { release, bytes });
        state.status = UpdateStatus::Ready { version, notes };
        Ok::<(), ()>(())
    }
    .await;
    if result.is_err() {
        service.set_status(UpdateStatus::Error {
            message: DOWNLOAD_FAILED.into(),
        });
        // Never format plugin/HTTP errors: they can contain URLs or credentials.
        crate::recognition_diagnostics::event(
            "update-download-failed",
            "",
            std::process::id(),
            0,
            0,
            0,
        );
    }
    emit();
}

fn request_check(runtime: &Arc<Runtime>, app: &AppHandle) -> Result<(), String> {
    if !runtime.updater.begin_check()? {
        return Ok(());
    }
    runtime.emit(app);
    let runtime = runtime.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        check(&runtime.updater, &PluginBackend(app.clone()), || {
            runtime.emit(&app)
        })
        .await;
    });
    Ok(())
}

#[tauri::command]
pub(crate) fn check_for_updates(
    app: AppHandle,
    state: tauri::State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    request_check(state.inner(), &app)?;
    state.snapshot()
}

#[tauri::command]
pub(crate) fn download_update(
    app: AppHandle,
    state: tauri::State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    if let Some(release) = state.updater.begin_download()? {
        state.emit(&app);
        let runtime = state.inner().clone();
        tauri::async_runtime::spawn(async move {
            download(
                &runtime.updater,
                &PluginBackend(app.clone()),
                release,
                || runtime.emit(&app),
            )
            .await;
        });
    }
    state.snapshot()
}

/// Later on the main-window notice: hide it for this version until next launch.
#[tauri::command]
pub(crate) fn dismiss_update_notice(
    app: AppHandle,
    state: tauri::State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    state.updater.dismiss_notice()?;
    state.emit(&app);
    state.snapshot()
}

/// The main window was opened or brought to the front.
pub(crate) fn window_focused(app: &AppHandle) {
    use tauri::Manager;
    let Some(runtime) = app.try_state::<Arc<Runtime>>().map(|s| s.inner().clone()) else {
        return;
    };
    if runtime
        .updater
        .claim_automatic(Trigger::Focus, Moment::now(), jitter())
    {
        // Window events arrive on the UI thread; emit from a worker as the
        // scheduler does.
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _ = request_check(&runtime, &app);
        });
    }
}

#[tauri::command]
pub(crate) fn restart_to_update(
    app: AppHandle,
    state: tauri::State<'_, Arc<Runtime>>,
) -> Result<Snapshot, String> {
    let (package, lease) = state.updater.claim_install()?;
    let snapshot = match state.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            state.updater.install_failed();
            drop(lease);
            return Err(error);
        }
    };
    state.emit(&app);
    let runtime = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Idle resident workers may hold DLLs in the installation directory.
        // Releasing them does not cancel any job: admission is already closed.
        crate::cleanup::shutdown();
        resource_manager::request_asr_release();
        let idle = resource_manager::await_asr_release(
            &std::sync::atomic::AtomicBool::new(false),
            Duration::from_secs(10),
        );
        if idle.is_ok() && package.release.install(&package.bytes).is_ok() {
            // Windows normally exits inside install after /P /R /UPDATE.
            // macOS returns after replacement, then Tauri restarts the app.
            std::mem::forget(lease);
            app.restart();
        }
        runtime.updater.install_failed();
        drop(lease);
        runtime.emit(&app);
    });
    Ok(snapshot)
}

fn next_check_delay(jitter: u64) -> Duration {
    // Six hours +/- fifteen minutes, newly sampled for each interval.
    Duration::from_secs(6 * 60 * 60 - 15 * 60 + jitter % (30 * 60 + 1))
}

pub(crate) fn start_scheduler(runtime: &Arc<Runtime>, app: &AppHandle) {
    let weak = Arc::downgrade(runtime);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last_reason = resource_manager::update_blocked_reason();
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(runtime) = weak.upgrade() else { break };
            let reason = resource_manager::update_blocked_reason();
            if reason != last_reason {
                last_reason = reason;
                runtime.emit(&app);
            }
            if runtime
                .updater
                .claim_automatic(Trigger::Timer, Moment::now(), jitter())
            {
                let _ = request_check(&runtime, &app);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use resource_manager::{begin_update_activity, begin_update_install, UpdateActivity};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockBackend {
        available: bool,
        check_fails: bool,
        signature_valid: bool,
        checks: AtomicUsize,
        downloads: AtomicUsize,
    }

    impl Default for MockBackend {
        fn default() -> Self {
            Self {
                available: true,
                check_fails: false,
                signature_valid: true,
                checks: AtomicUsize::new(0),
                downloads: AtomicUsize::new(0),
            }
        }
    }

    impl Backend for MockBackend {
        type Release = String;
        fn check(&self) -> Task<'_, Result<Option<String>, ()>> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            Box::pin(std::future::ready(if self.check_fails {
                Err(())
            } else {
                Ok(self.available.then(|| "0.23.0".into()))
            }))
        }
        fn describe(release: &String) -> (String, Option<String>, Option<u64>) {
            (release.clone(), Some("Release notes".into()), Some(100))
        }
        fn download<'a>(
            &'a self,
            _: &'a String,
            mut progress: Progress<'a>,
        ) -> Task<'a, Result<Vec<u8>, ()>> {
            Box::pin(async move {
                self.downloads.fetch_add(1, Ordering::SeqCst);
                progress(20, None);
                progress(30, Some(100));
                progress(50, Some(100));
                // Mimics plugin verification after the last byte arrives.
                if self.signature_valid {
                    Ok(vec![1, 2, 3])
                } else {
                    Err(())
                }
            })
        }
    }

    fn run<T>(task: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(task)
    }

    #[test]
    fn mock_download_only_becomes_ready_after_verification_and_requires_explicit_install() {
        let _gate = resource_manager::tests::test_gate();
        resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            crate::domain::ModelRetention::OnExit,
        );
        let service = Service::default();
        let backend = MockBackend::default();
        let seen = Mutex::new(Vec::new());
        assert!(service.begin_check().unwrap());
        assert!(!service.begin_check().unwrap());
        assert!(service.claim_install().is_err());
        assert!(service.begin_download().is_err());
        run(check(&service, &backend, || {
            seen.lock().unwrap().push(service.snapshot().unwrap())
        }));
        assert_eq!(backend.downloads.load(Ordering::SeqCst), 0);
        assert_eq!(
            service.snapshot().unwrap().status,
            UpdateStatus::Available {
                version: "0.23.0".into(),
                notes: Some("Release notes".into()),
                size: Some(100),
            }
        );
        assert!(service.lock().unwrap().package.is_none());
        assert!(service.claim_install().is_err());
        assert!(!service.begin_check().unwrap());
        let release = service.begin_download().unwrap().unwrap();
        seen.lock().unwrap().push(service.snapshot().unwrap());
        assert!(service.begin_download().unwrap().is_none());
        assert!(!service.begin_check().unwrap());
        run(download(&service, &backend, release, || {
            seen.lock().unwrap().push(service.snapshot().unwrap())
        }));
        assert_eq!(backend.checks.load(Ordering::SeqCst), 1);
        assert_eq!(backend.downloads.load(Ordering::SeqCst), 1);
        assert!(!service.begin_check().unwrap());
        assert!(service.begin_download().unwrap().is_none());
        let seen = seen.into_inner().unwrap();
        assert!(seen
            .iter()
            .any(|s| s.status == UpdateStatus::Downloading { percent: None }));
        assert!(seen
            .iter()
            .any(|s| s.status == UpdateStatus::Downloading { percent: Some(50) }));
        assert!(seen
            .iter()
            .any(|s| s.status == UpdateStatus::Downloading { percent: Some(99) }));
        assert!(seen.iter().all(|s| !s.installing));
        assert!(matches!(
            seen.last().unwrap().status,
            UpdateStatus::Ready { .. }
        ));
        let active = begin_update_activity(UpdateActivity::Meeting).unwrap();
        assert!(service.claim_install().is_err());
        assert!(service.snapshot().unwrap().restart_blocked_reason.is_some());
        drop(active);
        assert!(service.snapshot().unwrap().restart_blocked_reason.is_none());
        let (package, lease) = service.claim_install().unwrap();
        assert_eq!(package.release, "0.23.0");
        assert_eq!(package.bytes, vec![1, 2, 3]);
        assert!(service.snapshot().unwrap().installing);
        assert!(service.claim_install().is_err());
        assert!(begin_update_activity(UpdateActivity::Dictation).is_err());
        // Installer failure drops the gate and permits a fresh signed download.
        service.install_failed();
        drop(lease);
        assert!(!service.snapshot().unwrap().installing);
        assert!(service.begin_check().unwrap());
    }

    #[test]
    fn failed_signature_and_http_checks_never_retain_installable_bytes_and_can_retry() {
        let _gate = resource_manager::tests::test_gate();
        resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            crate::domain::ModelRetention::OnExit,
        );
        let service = Service::default();
        for backend in [
            MockBackend {
                signature_valid: false,
                ..MockBackend::default()
            },
            MockBackend {
                check_fails: true,
                ..MockBackend::default()
            },
        ] {
            assert!(service.begin_check().unwrap());
            run(check(&service, &backend, || {}));
            if !backend.check_fails {
                let release = service.begin_download().unwrap().unwrap();
                run(download(&service, &backend, release, || {}));
            }
            assert!(service.lock().unwrap().package.is_none());
            assert!(service.lock().unwrap().available.is_none());
            assert_eq!(
                service.snapshot().unwrap().status,
                UpdateStatus::Error {
                    message: if backend.check_fails {
                        CHECK_FAILED
                    } else {
                        DOWNLOAD_FAILED
                    }
                    .into()
                }
            );
            assert!(service.claim_install().is_err());
        }
        assert!(service.begin_check().unwrap());
        run(check(
            &service,
            &MockBackend {
                available: false,
                ..MockBackend::default()
            },
            || {},
        ));
        assert_eq!(service.snapshot().unwrap().status, UpdateStatus::Idle);
        assert!(service.begin_download().is_err());
    }

    #[test]
    fn all_job_classes_and_inference_leases_exclude_installation_in_both_directions() {
        let _gate = resource_manager::tests::test_gate();
        resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            crate::domain::ModelRetention::OnExit,
        );
        for kind in [
            UpdateActivity::Dictation,
            UpdateActivity::Meeting,
            UpdateActivity::Ai,
            UpdateActivity::Models,
            UpdateActivity::Media,
        ] {
            let activity = begin_update_activity(kind).unwrap();
            assert!(begin_update_install().is_err());
            drop(activity);
            let install = begin_update_install().unwrap();
            assert!(begin_update_activity(kind).is_err());
            assert!(resource_manager::acquire_asr("mock").is_err());
            assert!(resource_manager::acquire_text("notes", "mock").is_err());
            assert!(resource_manager::acquire_asr_with_admission("mock", "test").is_err());
            assert!(
                resource_manager::acquire_text_with_admission("notes", "mock", "test").is_err()
            );
            drop(install);
        }
        resource_manager::acquire_asr("mock").unwrap();
        assert!(begin_update_install().is_err());
        resource_manager::notify_asr_finished();
        resource_manager::acquire_text("notes", "mock").unwrap();
        assert!(begin_update_install().is_err());
        resource_manager::notify_text_finished();
    }

    #[test]
    fn simultaneous_job_and_restart_have_exactly_one_owner() {
        let _gate = resource_manager::tests::test_gate();
        resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            crate::domain::ModelRetention::OnExit,
        );
        // Hold the winner until both attempts finish, rather than letting a
        // short-lived winning lease make the second attempt legitimately idle.
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let b = barrier.clone();
        let worker = std::thread::spawn(move || {
            b.wait();
            let result = begin_update_activity(UpdateActivity::Models);
            b.wait();
            result
        });
        let b = barrier.clone();
        let installer = std::thread::spawn(move || {
            b.wait();
            let result = begin_update_install();
            b.wait();
            result
        });
        barrier.wait();
        barrier.wait();
        let work = worker.join().unwrap();
        let install = installer.join().unwrap();
        assert_ne!(work.is_ok(), install.is_ok());
    }

    #[test]
    fn static_manifest_uses_exact_supported_platform_and_rejects_bad_metadata() {
        let manifest = serde_json::json!({
            "version": "0.23.0", "notes": "Test", "platforms": {
                "windows-x86_64": { "url": "https://releases.sonojot.com/test-setup.exe", "signature": "signature" },
                "darwin-aarch64": { "url": "https://releases.sonojot.com/test.app.tar.gz", "signature": "signature" }
            }
        });
        assert_eq!(platform_key("windows", "x86_64"), Some("windows-x86_64"));
        assert_eq!(platform_key("macos", "aarch64"), Some("darwin-aarch64"));
        assert_eq!(platform_key("macos", "x86_64"), None);
        assert_eq!(platform_key("linux", "x86_64"), None);
        for target in ["windows-x86_64", "darwin-aarch64"] {
            assert!(validate_manifest(&manifest, target).is_ok());
            let mut missing = manifest.clone();
            missing["platforms"].as_object_mut().unwrap().remove(target);
            assert!(validate_manifest(&missing, target).is_err());
            for url in [
                "http://example.com/file",
                "https://user:pass@example.com/file",
                "invalid",
            ] {
                let mut bad = manifest.clone();
                bad["platforms"][target]["url"] = url.into();
                assert!(validate_manifest(&bad, target).is_err());
            }
            let mut bad = manifest.clone();
            bad["platforms"][target]["signature"] = "".into();
            assert!(validate_manifest(&bad, target).is_err());
        }
        assert!(validate_manifest(
            &serde_json::json!({ "version": "0.23.0",
            "url": "https://example.com/file", "signature": "x" }),
            "windows-x86_64"
        )
        .is_err());
        let mut bad = manifest;
        bad["version"] = "bad-version".into();
        assert!(validate_manifest(&bad, "windows-x86_64").is_err());
    }

    /// A moment `mono` after launch on the monotonic clock and `wall` after
    /// launch on the wall clock. They differ only across sleep or clock changes.
    fn at(launch: Instant, mono: u64, wall: u64) -> Moment {
        Moment {
            mono: launch + Duration::from_secs(mono),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + wall),
        }
    }

    const HOUR: u64 = 60 * 60;

    #[test]
    fn launch_check_waits_thirty_seconds_and_focus_never_preempts_it() {
        let launch = Instant::now();
        let schedule = Schedule::new(launch);
        assert!(!schedule.due(Trigger::Timer, at(launch, 0, 0)));
        assert!(!schedule.due(Trigger::Timer, at(launch, 29, 29)));
        assert!(schedule.due(Trigger::Timer, at(launch, 30, 30)));
        // Opening the window at launch (or before the first check) waits for it.
        for seconds in [0, 10, 31, 2 * HOUR] {
            assert!(!schedule.due(Trigger::Focus, at(launch, seconds, seconds)));
        }
    }

    #[test]
    fn focus_checks_only_once_the_last_check_is_an_hour_old() {
        let launch = Instant::now();
        let mut schedule = Schedule::new(launch);
        schedule.record(at(launch, 30, 30), 900);
        for fresh in [31, 30 + 10 * 60, 30 + HOUR - 1] {
            assert!(!schedule.due(Trigger::Focus, at(launch, fresh, fresh)));
        }
        assert!(schedule.due(Trigger::Focus, at(launch, 30 + HOUR, 30 + HOUR)));
        // A focus check restarts both clocks: neither focus nor the timer
        // repeats it straight away.
        let focus = 30 + 2 * HOUR;
        schedule.record(at(launch, focus, focus), 900);
        assert!(!schedule.due(Trigger::Focus, at(launch, focus + 60, focus + 60)));
        assert!(!schedule.due(
            Trigger::Timer,
            at(launch, focus + 5 * HOUR, focus + 5 * HOUR)
        ));
        assert!(schedule.due(
            Trigger::Timer,
            at(launch, focus + 6 * HOUR, focus + 6 * HOUR)
        ));
    }

    #[test]
    fn timer_keeps_the_six_hour_cadence_and_catches_up_after_sleep() {
        let launch = Instant::now();
        let mut schedule = Schedule::new(launch);
        // Jitter 0 and 1800 give the 5 h 45 min and 6 h 15 min bounds.
        schedule.record(at(launch, 30, 30), 0);
        assert!(!schedule.due(Trigger::Timer, at(launch, 30 + 20_699, 30 + 20_699)));
        assert!(schedule.due(Trigger::Timer, at(launch, 30 + 20_700, 30 + 20_700)));
        schedule.record(at(launch, 30, 30), 1800);
        assert!(!schedule.due(Trigger::Timer, at(launch, 30 + 22_499, 30 + 22_499)));
        assert!(schedule.due(Trigger::Timer, at(launch, 30 + 22_500, 30 + 22_500)));
        // Eight hours asleep: the monotonic clock may pause, the wall clock
        // does not. Both triggers are due as soon as the computer wakes.
        let asleep = at(launch, 30 + 60, 30 + 8 * HOUR);
        assert!(schedule.due(Trigger::Timer, asleep));
        assert!(schedule.due(Trigger::Focus, asleep));
        // A wall clock set back a day does not stall checks either.
        let set_back = Moment {
            mono: launch + Duration::from_secs(30 + 7 * HOUR),
            wall: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 - 24 * HOUR),
        };
        assert!(schedule.due(Trigger::Timer, set_back));
    }

    #[test]
    fn claims_restart_the_clocks_and_dismissal_follows_the_offered_version() {
        let _gate = resource_manager::tests::test_gate();
        resource_manager::reset_for_tests(
            crate::domain::MemoryMode::Overlap,
            crate::domain::ModelRetention::OnExit,
        );
        let service = Service::<String>::default();
        let launch = service.lock().unwrap().schedule.launched;
        assert!(!service.claim_automatic(Trigger::Timer, at(launch, 10, 10), 900));
        assert!(service.claim_automatic(Trigger::Timer, at(launch, 30, 30), 900));
        assert!(!service.claim_automatic(Trigger::Timer, at(launch, 31, 31), 900));
        assert!(!service.claim_automatic(Trigger::Focus, at(launch, 60, 60), 900));
        // Check now counts as the most recent check.
        assert!(service.begin_check().unwrap());
        assert!(service.lock().unwrap().schedule.last.is_some());
        assert!(!service.snapshot().unwrap().notice_dismissed);
        run(check(&service, &MockBackend::default(), || {}));
        assert!(!service.snapshot().unwrap().notice_dismissed);
        service.dismiss_notice().unwrap();
        assert!(service.snapshot().unwrap().notice_dismissed);
        // The dismissal holds through download and ready for that version.
        let release = service.begin_download().unwrap().unwrap();
        assert!(service.snapshot().unwrap().notice_dismissed);
        run(download(&service, &MockBackend::default(), release, || {}));
        assert!(matches!(
            service.snapshot().unwrap().status,
            UpdateStatus::Ready { .. }
        ));
        assert!(service.snapshot().unwrap().notice_dismissed);
        // A newer version is offered afresh.
        service.lock().unwrap().offered = Some("0.24.0".into());
        assert!(!service.snapshot().unwrap().notice_dismissed);
        let value = serde_json::to_value(service.snapshot().unwrap()).unwrap();
        assert_eq!(value["noticeDismissed"], false);
    }

    #[test]
    fn wire_contract_progress_and_schedule_are_bounded() {
        let snapshot = UpdateSnapshot::default();
        let value = serde_json::to_value(snapshot).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(value["currentVersion"], config["version"]);
        assert_eq!(value["status"]["state"], "idle");
        assert!(value["restartBlockedReason"].is_null());
        assert_eq!(value["installing"], false);
        assert_eq!(percent(u64::MAX, Some(1)), Some(99));
        assert_eq!(percent(10, Some(0)), None);
        assert_eq!(percent(10, None), None);
        assert_eq!(value["noticeDismissed"], false);
        assert_eq!(FIRST_CHECK, Duration::from_secs(30));
        assert_eq!(FOCUS_CHECK_AFTER, Duration::from_secs(3600));
        assert_eq!(next_check_delay(0), Duration::from_secs(20_700));
        assert_eq!(next_check_delay(1800), Duration::from_secs(22_500));
    }
}
