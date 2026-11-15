use super::*;
use crate::optional_asr::{OptionalModel, PinnedFile, Release};
use sha2::{Digest, Sha256};
use std::cell::{Cell, RefCell};

// Fixture files: an "encoder" that changes between releases and a "tokens"
// file that does not. Hashes are of the literal contents below.
const OLD_ENC: &[u8] = b"old encoder";
const NEW_ENC: &[u8] = b"new encoder!";
const TOKENS: &[u8] = b"tokens";

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn fixture_model(id: &'static str) -> (&'static OptionalModel, &'static Release) {
    let tokens = PinnedFile { relative: "tokens.txt", bytes: TOKENS.len() as u64, sha256: leak(hex(TOKENS)) };
    let old = PinnedFile { relative: "encoder.int8.onnx", bytes: OLD_ENC.len() as u64, sha256: leak(hex(OLD_ENC)) };
    let new = PinnedFile { relative: "encoder.int8.onnx", bytes: NEW_ENC.len() as u64, sha256: leak(hex(NEW_ENC)) };
    let release: &'static Release = Box::leak(Box::new(Release {
        revision: "1111111111111111111111111111111111111111",
        files: Box::leak(vec![old, tokens].into_boxed_slice()),
    }));
    let model: &'static OptionalModel = Box::leak(Box::new(OptionalModel {
        id,
        name: "Fixture Parakeet",
        description: "",
        repo: "SonoJot/fixture",
        revision: "2222222222222222222222222222222222222222",
        files: Box::leak(vec![new, tokens].into_boxed_slice()),
        previous: std::slice::from_ref(release),
        update_note: "Fewer mistakes.",
    }));
    (model, release)
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("model-updates-{name}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn install_old(root: &Path, model: &OptionalModel) -> PathBuf {
    let dir = optional_asr::model_dir(root, model.id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("encoder.int8.onnx"), OLD_ENC).unwrap();
    fs::write(dir.join("tokens.txt"), TOKENS).unwrap();
    dir
}

struct Fake {
    idle_after: Cell<u32>,
    idle_checks: Cell<u32>,
    pauses: Cell<u32>,
    probed: RefCell<Vec<PathBuf>>,
    probe_error: Option<&'static str>,
    fetch_body: Option<&'static [u8]>,
    fetches: Cell<u32>,
    phases: RefCell<Vec<&'static str>>,
}

impl Fake {
    fn new() -> Self {
        Self {
            idle_after: Cell::new(0),
            idle_checks: Cell::new(0),
            pauses: Cell::new(0),
            probed: RefCell::new(Vec::new()),
            probe_error: None,
            fetch_body: Some(NEW_ENC),
            fetches: Cell::new(0),
            phases: RefCell::new(Vec::new()),
        }
    }
}

fn run_speech(root: &Path, model: &'static OptionalModel, from: &'static Release, fake: &Fake) -> Result<(), Failure> {
    let idle = |_: &str| {
        let n = fake.idle_checks.get() + 1;
        fake.idle_checks.set(n);
        n > fake.idle_after.get()
    };
    let release = |_: &str| true;
    let probe_speech = |id: &str, staged: &Path| {
        // The staged files must be complete and verified before the probe,
        // laid out as a models root the worker can open.
        assert_eq!(fs::read(optional_asr::model_dir(staged, id).join("encoder.int8.onnx")).unwrap(), NEW_ENC);
        fake.probed.borrow_mut().push(staged.to_path_buf());
        fake.probe_error.map_or(Ok(()), |e| Err(e.to_string()))
    };
    let probe_cleanup = |_: Package| Ok(());
    let fetch = |_: &str, dest: &Path, file: &PinnedFile, _: &AtomicBool, progress: &dyn Fn(u64, u64)| {
        fake.fetches.set(fake.fetches.get() + 1);
        let body = fake.fetch_body.ok_or("The speech model download was interrupted: offline")?;
        fs::write(dest, body).map_err(|e| e.to_string())?;
        progress(body.len() as u64, file.bytes);
        if hex(body) != file.sha256 {
            let _ = fs::remove_file(dest);
            return Err("The downloaded speech model file did not match its pinned checksum.".into());
        }
        Ok(())
    };
    let install_cleanup = |_: Package, _: &AtomicBool, _: &dyn Fn(u64, u64)| Ok(());
    let cleanup_ready = |_: Package| true;
    let pause = |_: &AtomicBool| {
        fake.pauses.set(fake.pauses.get() + 1);
        fake.pauses.get() < 500
    };
    let phase = |state: &'static str| fake.phases.borrow_mut().push(state);
    let hooks = Hooks {
        idle: &idle,
        release_speech: &release,
        probe_speech: &probe_speech,
        probe_cleanup: &probe_cleanup,
        fetch: &fetch,
        install_cleanup: &install_cleanup,
        cleanup_ready: &cleanup_ready,
        pause: &pause,
        phase: &phase,
    };
    update_speech(root, model, from, &AtomicBool::new(false), &|_, _| {}, &hooks)
}

fn only_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn an_older_release_is_detected_as_installed_and_offered_as_an_update() {
    let root = temp_root("detect");
    let (model, release) = fixture_model("fixture-detect");
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Missing));
    install_old(&root, model);
    match optional_asr::installed_release(&root, model, Check::Blocking) {
        InstalledRelease::Previous(found) => assert_eq!(found.revision, release.revision),
        _ => panic!("old files should read as the previous release"),
    }
    // Only the changed encoder is downloaded; tokens are reused.
    let plan = Plan::Speech { model, from: release };
    assert_eq!(plan.download_bytes(&root, Check::Blocking), NEW_ENC.len() as u64);
    assert_eq!(plan.id(), model.id, "the update keeps the model's id");
    assert!(!plan.automatic(), "speech models ask first");
    fs::write(optional_asr::model_dir(&root, model.id).join("encoder.int8.onnx"), NEW_ENC).unwrap();
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Current));
    // A damaged file is neither release.
    let encoder = optional_asr::model_dir(&root, model.id).join("encoder.int8.onnx");
    fs::remove_file(&encoder).unwrap();
    fs::write(&encoder, b"damaged!!!!!").unwrap();
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Missing));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_shipped_sonojot_parakeet_lists_its_first_release_and_downloads_only_the_encoder() {
    let ft2 = optional_asr::find(optional_asr::PARAKEET_FT2_ID).unwrap();
    assert_eq!(ft2.previous.len(), 1);
    let first = &ft2.previous[0];
    assert_eq!(first.revision, "ff2638d470935675096725da78c4ad691fbde46a");
    let changed: Vec<_> = changed_files(ft2, first).map(|file| file.relative).collect();
    assert_eq!(changed, ["encoder.int8.onnx"]);
    assert!(!ft2.update_note.is_empty() && !ft2.update_note.contains('\u{2014}'));
}

#[test]
fn verified_files_are_swapped_in_and_the_old_ones_removed() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("swap");
    let (model, release) = fixture_model("fixture-swap");
    let dir = install_old(&root, model);
    let fake = Fake::new();
    run_speech(&root, model, release, &fake).unwrap();
    assert_eq!(fake.fetches.get(), 1, "tokens were copied, not downloaded");
    assert_eq!(fake.probed.borrow().len(), 1, "loaded before the swap");
    assert_eq!(fs::read(dir.join("encoder.int8.onnx")).unwrap(), NEW_ENC);
    assert_eq!(fs::read(dir.join("tokens.txt")).unwrap(), TOKENS);
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Current));
    // No backup, staging or partial files are left behind.
    assert_eq!(only_entries(&dir), ["encoder.int8.onnx", "tokens.txt"]);
    assert_eq!(only_entries(&optional_asr::optional_root(&root)), [model.id.to_string()]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn assignments_survive_because_the_model_id_never_changes() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("assign");
    let (model, release) = fixture_model("fixture-assign");
    install_old(&root, model);
    let mut settings = crate::domain::Settings::default();
    settings.asr_model = model.id.into();
    settings.meeting_asr_model = model.id.into();
    let before = serde_json::to_value(&settings).unwrap();
    run_speech(&root, model, release, &Fake::new()).unwrap();
    // Nothing in settings refers to a version, so nothing needs rewriting.
    assert_eq!(serde_json::to_value(&settings).unwrap(), before);
    assert_eq!(settings.asr_model, model.id);
    assert!(optional_asr::model_dir(&root, model.id).join("encoder.int8.onnx").is_file());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_failed_download_keeps_the_old_version_and_retries_later() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("offline");
    let (model, release) = fixture_model("fixture-offline");
    let dir = install_old(&root, model);
    let mut fake = Fake::new();
    fake.fetch_body = None;
    let result = run_speech(&root, model, release, &fake);
    assert!(matches!(result, Err(Failure::Retry(_))), "{result:?}");
    assert_eq!(fs::read(dir.join("encoder.int8.onnx")).unwrap(), OLD_ENC);
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Previous(_)));
    assert!(fake.probed.borrow().is_empty());
    // The staging folder stays so the next try resumes.
    assert!(optional_asr::model_dir(&staging_dir(&root, model), model.id).is_dir());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_bad_checksum_or_a_failed_load_keeps_the_old_version() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("reject");
    let (model, release) = fixture_model("fixture-reject");
    let dir = install_old(&root, model);
    let mut fake = Fake::new();
    fake.fetch_body = Some(b"wrong bytes!");
    let result = run_speech(&root, model, release, &fake);
    assert!(matches!(result, Err(Failure::Rejected(_))), "{result:?}");
    assert_eq!(fs::read(dir.join("encoder.int8.onnx")).unwrap(), OLD_ENC);

    let mut fake = Fake::new();
    fake.probe_error = Some("Cannot initialize speech recognition");
    let result = run_speech(&root, model, release, &fake);
    match result {
        Err(Failure::Rejected(message)) => assert!(message.starts_with(NOT_LOADED)),
        other => panic!("{other:?}"),
    }
    assert_eq!(fs::read(dir.join("encoder.int8.onnx")).unwrap(), OLD_ENC);
    assert!(!staging_dir(&root, model).exists(), "a build that cannot load is discarded");
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Previous(_)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn the_swap_waits_until_sonojot_is_idle() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("idle");
    let (model, release) = fixture_model("fixture-idle");
    let dir = install_old(&root, model);
    let fake = Fake::new();
    fake.idle_after.set(3);
    run_speech(&root, model, release, &fake).unwrap();
    assert_eq!(fake.pauses.get(), 3, "waited through three busy checks");
    assert_eq!(fs::read(dir.join("encoder.int8.onnx")).unwrap(), NEW_ENC);
    let phases = fake.phases.borrow();
    let waiting = phases.iter().position(|p| *p == "waiting").unwrap();
    let installing = phases.iter().position(|p| *p == "installing").unwrap();
    assert!(waiting < installing);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_removed_model_cancels_its_waiting_update() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("removed");
    let (model, release) = fixture_model("fixture-removed");
    let dir = install_old(&root, model);
    let fake = Fake::new();
    fake.idle_after.set(u32::MAX);
    // Busy forever; the user removes the model while the update waits.
    let idle = |_: &str| {
        let _ = fs::remove_dir_all(&dir);
        false
    };
    let release_speech = |_: &str| true;
    let probe = |_: &str, _: &Path| Ok(());
    let probe_cleanup = |_: Package| Ok(());
    let fetch = |_: &str, dest: &Path, _: &PinnedFile, _: &AtomicBool, _: &dyn Fn(u64, u64)| {
        fs::write(dest, NEW_ENC).map_err(|e| e.to_string())
    };
    let install_cleanup = |_: Package, _: &AtomicBool, _: &dyn Fn(u64, u64)| Ok(());
    let cleanup_ready = |_: Package| true;
    let pause = |_: &AtomicBool| true;
    let phase = |_: &'static str| {};
    let hooks = Hooks {
        idle: &idle,
        release_speech: &release_speech,
        probe_speech: &probe,
        probe_cleanup: &probe_cleanup,
        fetch: &fetch,
        install_cleanup: &install_cleanup,
        cleanup_ready: &cleanup_ready,
        pause: &pause,
        phase: &phase,
    };
    let result = update_speech(&root, model, release, &AtomicBool::new(false), &|_, _| {}, &hooks);
    assert_eq!(result, Err(Failure::Cancelled));
    assert!(!dir.exists(), "never reinstalled behind the user's back");
    assert!(!staging_dir(&root, model).exists());
    let _ = fs::remove_dir_all(&root);
}

fn package_dir(root: &Path, package: Package) -> PathBuf {
    let dir = package.path(root);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("model.int8.onnx"), b"x").unwrap();
    dir
}

fn run_cleanup(root: &Path, from: &[Package], probe_error: Option<&'static str>, idle_after: u32) -> (Result<(), Failure>, u32) {
    let to = Package::offered();
    let checks = Cell::new(0u32);
    let pauses = Cell::new(0u32);
    let idle = |_: &str| {
        checks.set(checks.get() + 1);
        checks.get() > idle_after
    };
    let release_speech = |_: &str| true;
    let probe_speech = |_: &str, _: &Path| Ok(());
    let probe_cleanup = |package: Package| {
        assert_eq!(package, to);
        assert!(package.path(root).is_dir(), "the new package is in place before it is loaded");
        probe_error.map_or(Ok(()), |e| Err(e.to_string()))
    };
    let fetch = |_: &str, _: &Path, _: &PinnedFile, _: &AtomicBool, _: &dyn Fn(u64, u64)| Ok(());
    let install_cleanup = |package: Package, _: &AtomicBool, _: &dyn Fn(u64, u64)| {
        package_dir(root, package);
        Ok(())
    };
    let cleanup_ready = |package: Package| package.path(root).is_dir();
    let pause = |_: &AtomicBool| {
        pauses.set(pauses.get() + 1);
        pauses.get() < 500
    };
    let phase = |_: &'static str| {};
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
    (update_cleanup(root, to, from, &AtomicBool::new(false), &|_, _| {}, &hooks), pauses.get())
}

#[test]
fn cleanup_updates_by_itself_and_only_from_published_older_packages() {
    let to = Package::offered();
    assert_eq!(to, Package::P4);
    let installed = [Package::V3, Package::V4];
    let plan = cleanup_plan_with(&|package| installed.contains(&package)).unwrap();
    match &plan {
        Plan::Cleanup { to: target, from } => {
            assert_eq!(*target, Package::P4);
            assert_eq!(from, &[Package::V3], "the local test package is left alone");
        }
        _ => unreachable!(),
    }
    assert!(plan.automatic());
    assert!(!plan.note().is_empty());
    assert!(cleanup_plan_with(&|package| package == Package::V4).is_none());
    assert!(cleanup_plan_with(&|_| false).is_none());
}

#[test]
fn cleanup_swap_removes_the_old_package_only_after_the_new_one_loads() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("cleanup");
    let old = package_dir(&root, Package::V3);
    let local = package_dir(&root, Package::V4);
    let (result, pauses) = run_cleanup(&root, &[Package::V3], None, 2);
    result.unwrap();
    assert_eq!(pauses, 2, "removal waited for idle");
    assert!(!old.exists(), "old version removed");
    assert!(local.exists(), "local test package untouched");
    assert!(Package::P4.path(&root).is_dir());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn cleanup_that_cannot_load_is_removed_and_the_old_package_stays() {
    let _gate = crate::resource_manager::tests::test_gate();
    let root = temp_root("cleanup-fail");
    let old = package_dir(&root, Package::V3);
    let (result, _) = run_cleanup(&root, &[Package::V3], Some("Load"), 0);
    assert!(matches!(result, Err(Failure::Rejected(_))));
    assert!(old.exists());
    assert!(!Package::P4.path(&root).exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_rejected_update_is_not_retried_automatically_until_the_next_app_version() {
    let root = temp_root("remember");
    let plan = Plan::Cleanup { to: Package::P4, from: vec![Package::V3] };
    assert!(!rejected(&root, &plan));
    remember_rejection(&root, &plan);
    assert!(rejected(&root, &plan));
    let text = fs::read_to_string(root.join(STATE_FILE)).unwrap();
    assert!(text.contains(env!("CARGO_PKG_VERSION")));
    let _ = fs::remove_dir_all(&root);
}

/// Opt-in: SONOJOT_LIVE_UPDATE_ROOT is a models root (a copy, it is changed)
/// holding `optional/sherpa-runtime` and the first SonoJot Parakeet release.
/// Runs the real update: download from the pinned revision (or
/// SONOJOT_DEV_MODEL_DIR before publication), hash, load and decode in the
/// sherpa-onnx runtime from staging, swap, and verify the result.
#[test]
fn live_sonojot_parakeet_updates_in_place_when_staged() {
    let Some(root) = std::env::var_os("SONOJOT_LIVE_UPDATE_ROOT").map(PathBuf::from) else {
        eprintln!("Skipping live update: set SONOJOT_LIVE_UPDATE_ROOT");
        return;
    };
    let _gate = crate::resource_manager::tests::test_gate();
    let model = optional_asr::find(optional_asr::PARAKEET_FT2_ID).unwrap();
    let Some(Plan::Speech { from, .. }) = plans(&root, Check::Blocking).into_iter().find(|p| p.id() == model.id)
    else {
        panic!("the staged root should hold the first release");
    };
    let started = std::time::Instant::now();
    let idle = |_: &str| true;
    let release_speech = |_: &str| true;
    let probe_root = root.clone();
    let probe_speech = |id: &str, staged: &Path| probe_speech_in_worker(&probe_root, id, staged);
    let probe_cleanup = |_: Package| Ok(());
    let fetch = |url: &str, dest: &Path, file: &PinnedFile, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)| {
        optional_asr::download_resumable(url, dest, file.bytes, file.sha256, "speech model", cancel, progress)
    };
    let install_cleanup = |_: Package, _: &AtomicBool, _: &dyn Fn(u64, u64)| Ok(());
    let cleanup_ready = |_: Package| true;
    let pause = |_: &AtomicBool| false;
    let phase = |state: &'static str| eprintln!("phase {state} at {:.1}s", started.elapsed().as_secs_f64());
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
    update_speech(&root, model, from, &AtomicBool::new(false), &|_, _| {}, &hooks).expect("update");
    assert!(matches!(optional_asr::installed_release(&root, model, Check::Blocking), InstalledRelease::Current));
    assert!(plans(&root, Check::Blocking).iter().all(|p| p.id() != model.id));
    let names = only_entries(&optional_asr::model_dir(&root, model.id));
    assert_eq!(names, ["decoder.int8.onnx", "encoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]);
    println!("{}", serde_json::json!({"updated_s": started.elapsed().as_secs_f64()}));
}
