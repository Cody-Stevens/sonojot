//! Stamp-bound verification receipts and a single, bounded background queue.
//! Status readers never hash or wait for a hasher. Loading verifies only its
//! requested files; installers use `uncached` before activating downloads.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::SystemTime,
};

pub(crate) const CACHE_FILE: &str = ".sonojot-verification-v1.json";
const MAX_ENTRIES: usize = 512;
const MAX_ROOTS: usize = 8;
const MAX_CACHE_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Ready,
    Missing,
    Checking,
}

impl Status {
    pub fn ready(self) -> bool {
        self == Self::Ready
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "installed",
            Self::Missing => "not-installed",
            Self::Checking => "checking",
        }
    }
    pub fn present(value: bool) -> Self {
        if value {
            Self::Ready
        } else {
            Self::Missing
        }
    }

    // Consume every dependency, including shared runtimes. Iterator::all/any
    // would leave later files unscheduled after the first pending dependency.
    pub fn all(items: impl IntoIterator<Item = Self>) -> Self {
        items.into_iter().fold(Self::Ready, |a, b| match (a, b) {
            (Self::Checking, _) | (_, Self::Checking) => Self::Checking,
            (Self::Missing, _) | (_, Self::Missing) => Self::Missing,
            _ => Self::Ready,
        })
    }
    pub fn any(items: impl IntoIterator<Item = Self>) -> Self {
        items.into_iter().fold(Self::Missing, |a, b| match (a, b) {
            (Self::Ready, _) | (_, Self::Ready) => Self::Ready,
            (Self::Checking, _) | (_, Self::Checking) => Self::Checking,
            _ => Self::Missing,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Check {
    Background,
    Blocking,
}

impl Check {
    pub fn file(self, root: &Path, path: &Path, bytes: u64, sha256: &str) -> Status {
        verifier().check(root, path, bytes, sha256, self)
    }
    pub fn files(
        self,
        root: &Path,
        directory: &Path,
        files: &[crate::optional_asr::PinnedFile],
    ) -> Status {
        Status::all(files.iter().map(|file| {
            self.file(
                root,
                &directory.join(file.relative),
                file.bytes,
                file.sha256,
            )
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    identity: [u64; 2],
    // Unix ctime also detects in-place edits which restore mtime.
    changed: Option<(i64, i64)>,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(windows)]
    let (identity, changed) = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return None;
        }
        (
            [
                info.dwVolumeSerialNumber as u64,
                ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
            ],
            None,
        )
    };
    #[cfg(unix)]
    let (identity, changed) = {
        use std::os::unix::fs::MetadataExt;
        (
            [metadata.dev(), metadata.ino()],
            Some((metadata.ctime(), metadata.ctime_nsec())),
        )
    };
    #[cfg(not(any(windows, unix)))]
    return None;
    #[cfg(any(windows, unix))]
    Some(Stamp {
        bytes: metadata.len(),
        modified: metadata.modified().ok()?,
        created: metadata.created().ok(),
        identity,
        changed,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Key {
    root: PathBuf,
    path: PathBuf,
    sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Receipt {
    key: Key,
    stamp: Stamp,
}

#[derive(Serialize, Deserialize)]
struct DiskCache {
    version: u32,
    root: PathBuf,
    receipts: Vec<Receipt>,
}

#[derive(Clone)]
struct Job {
    key: Key,
    stamp: Stamp,
}

#[derive(Default)]
struct State {
    roots: HashSet<PathBuf>,
    // Failed hashes are memoized only in memory, to avoid perpetual requeueing
    // of a corrupt file. Only successful receipts survive a process restart.
    results: HashMap<Key, (Stamp, bool)>,
    queued: VecDeque<Job>,
    // Value records whether a status reader has observed this pending hash.
    running: HashMap<Key, bool>,
    worker: bool,
}

type Hasher = dyn Fn(&Path, u64, Check) -> Result<String, String> + Send + Sync;
type Completion = dyn Fn() + Send + Sync;

struct Verifier {
    state: Mutex<State>,
    changed: Condvar,
    // Persistence uses a separate lock: status reads and hash execution never
    // wait behind receipt writes.
    persist: Mutex<()>,
    hasher: Arc<Hasher>,
    completion: Mutex<Option<Arc<Completion>>>,
}

impl Verifier {
    fn new(hasher: Arc<Hasher>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            persist: Mutex::new(()),
            hasher,
            completion: Mutex::new(None),
        })
    }

    fn load(state: &mut State, root: &Path) {
        if state.roots.contains(root) {
            return;
        }
        if state.roots.len() >= MAX_ROOTS {
            state.roots.clear();
            state.results.clear();
        }
        state.roots.insert(root.to_owned());
        let path = root.join(CACHE_FILE);
        let disk = (|| {
            let file = File::open(path).ok()?;
            if file.metadata().ok()?.len() > MAX_CACHE_BYTES {
                return None;
            }
            let disk: DiskCache =
                serde_json::from_reader(std::io::BufReader::new(file.take(MAX_CACHE_BYTES + 1)))
                    .ok()?;
            (disk.version == 1 && disk.root == root && disk.receipts.len() <= MAX_ENTRIES)
                .then_some(disk)
        })();
        if let Some(disk) = disk {
            for receipt in disk.receipts {
                if receipt.key.root == root
                    && receipt.key.path.is_absolute()
                    && receipt.key.sha256.len() == 64
                {
                    if state.results.len() >= MAX_ENTRIES {
                        break;
                    }
                    state.results.insert(receipt.key, (receipt.stamp, true));
                }
            }
        }
    }

    fn check(
        self: &Arc<Self>,
        root: &Path,
        path: &Path,
        bytes: u64,
        sha256: &str,
        mode: Check,
    ) -> Status {
        loop {
            let Ok(root) = fs::canonicalize(root) else {
                return Status::Missing;
            };
            let Ok(path) = fs::canonicalize(path) else {
                return Status::Missing;
            };
            let key = Key {
                root,
                path,
                sha256: sha256.to_ascii_lowercase(),
            };
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            Self::load(&mut state, &key.root);
            // Cache loading/lock contention must not leave us trusting a
            // stamp sampled before another tool replaced the file.
            let Some(current) = stamp(&key.path).filter(|s| s.bytes == bytes) else {
                return Status::Missing;
            };
            if let Some((saved, result)) = state.results.get(&key) {
                if *saved == current {
                    return Status::present(*result);
                }
            }
            if matches!(mode, Check::Background) {
                if let Some(observed) = state.running.get_mut(&key) {
                    *observed = true;
                }
                if !state.running.contains_key(&key)
                    && !state.queued.iter().any(|j| j.key == key)
                    && state.queued.len() < MAX_ENTRIES
                {
                    state.queued.push_back(Job {
                        key,
                        stamp: current,
                    });
                }
                if !state.worker && !state.queued.is_empty() {
                    state.worker = true;
                    let owner = self.clone();
                    if std::thread::Builder::new()
                        .name("model-verification".into())
                        .spawn(move || owner.drain())
                        .is_err()
                    {
                        state.worker = false;
                    }
                }
                return Status::Checking;
            }
            if state.running.contains_key(&key) {
                // Only the requested file can delay a load. Re-sample its identity
                // after waking; never trust metadata from before the wait.
                state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
                drop(state);
                continue;
            }
            let observed = state.queued.iter().any(|j| j.key == key);
            state.queued.retain(|j| j.key != key);
            state.running.insert(key.clone(), observed);
            drop(state);
            return self.run(
                Job {
                    key,
                    stamp: current,
                },
                mode,
            );
        }
    }

    fn drain(self: Arc<Self>) {
        lower_priority();
        loop {
            let job = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let Some(job) = state.queued.pop_front() else {
                    state.worker = false;
                    self.changed.notify_all();
                    return;
                };
                state.running.insert(job.key.clone(), true);
                job
            };
            self.run(job, Check::Background);
        }
    }

    fn run(&self, job: Job, mode: Check) -> Status {
        // A queued file may have been replaced, removed or moved. Do not hash
        // its stale location or publish a receipt for a different identity.
        let before = stamp(&job.key.path);
        let matched = before.as_ref() == Some(&job.stamp)
            && (self.hasher)(&job.key.path, job.stamp.bytes, mode)
                .is_ok_and(|hash| hash.eq_ignore_ascii_case(&job.key.sha256));
        let unchanged = stamp(&job.key.path).as_ref() == Some(&job.stamp);
        let observed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if unchanged {
                if state.results.len() >= MAX_ENTRIES && !state.results.contains_key(&job.key) {
                    if let Some(evicted) = state.results.keys().next().cloned() {
                        state.results.remove(&evicted);
                    }
                }
                state.results.insert(job.key.clone(), (job.stamp, matched));
            } else {
                state.results.remove(&job.key);
            }
            let observed = state.running.remove(&job.key).unwrap_or(false);
            self.changed.notify_all();
            observed
        };
        self.save(&job.key.root);
        if observed {
            let callback = self
                .completion
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(callback) = callback {
                callback();
            }
        }
        Status::present(matched && unchanged)
    }

    fn save(&self, root: &Path) {
        // Replay may read the installed model store without changing receipts.
        #[cfg(feature = "fidelity-eval")]
        if std::env::var("SONOJOT_EVAL_READ_ONLY_MODELS").as_deref() == Ok("1") {
            return;
        }
        let _writer = self.persist.lock().unwrap_or_else(|e| e.into_inner());
        let receipts = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .results
                .iter()
                .filter(|(key, (_, ok))| key.root == root && *ok)
                .map(|(key, (stamp, _))| Receipt {
                    key: key.clone(),
                    stamp: stamp.clone(),
                })
                .collect()
        };
        let disk = DiskCache {
            version: 1,
            root: root.to_owned(),
            receipts,
        };
        if let Ok(bytes) = serde_json::to_vec(&disk) {
            if bytes.len() as u64 <= MAX_CACHE_BYTES {
                // A moved root does not inherit receipts, even when the JSON
                // and weights travel together. Unwritable roots still work.
                if fs::canonicalize(root).ok().as_deref() == Some(root) {
                    let _ = crate::store::atomic_write(&root.join(CACHE_FILE), &bytes);
                }
            }
        }
    }
}

fn lower_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
            THREAD_PRIORITY_LOWEST,
        };
        if SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) == 0 {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    unsafe {
        libc::nice(10);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
        }
        pthread_set_qos_class_self_np(0x09, 0); // QOS_CLASS_BACKGROUND, this thread only.
    }
}

fn verifier() -> &'static Arc<Verifier> {
    static VERIFIER: OnceLock<Arc<Verifier>> = OnceLock::new();
    VERIFIER.get_or_init(|| Verifier::new(Arc::new(hash_file)))
}

pub(crate) fn on_completion(callback: impl Fn() + Send + Sync + 'static) {
    *verifier()
        .completion
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(callback));
}

/// Installation/activation boundary: intentionally bypasses every receipt.
pub(crate) fn uncached(path: &Path, bytes: u64, sha256: &str) -> Result<bool, String> {
    let Some(before) = stamp(path).filter(|s| s.bytes == bytes) else {
        return Ok(false);
    };
    let matches = hash_file(path, bytes, Check::Blocking)?.eq_ignore_ascii_case(sha256);
    Ok(matches && stamp(path).as_ref() == Some(&before))
}

#[derive(Default)]
struct HashAccess {
    active: bool,
    foreground_waiters: usize,
}

#[derive(Default)]
struct HashScheduler {
    access: Mutex<HashAccess>,
    changed: Condvar,
}

struct HashPermit<'a>(&'a HashScheduler);

impl HashScheduler {
    fn acquire(&self, mode: Check) -> HashPermit<'_> {
        let foreground = matches!(mode, Check::Blocking);
        let mut access = self.access.lock().unwrap_or_else(|e| e.into_inner());
        if foreground {
            access.foreground_waiters += 1;
            self.changed.notify_all();
        }
        while access.active || (!foreground && access.foreground_waiters > 0) {
            access = self.changed.wait(access).unwrap_or_else(|e| e.into_inner());
        }
        if foreground {
            access.foreground_waiters -= 1;
        }
        access.active = true;
        HashPermit(self)
    }
}

impl Drop for HashPermit<'_> {
    fn drop(&mut self) {
        let mut access = self.0.access.lock().unwrap_or_else(|e| e.into_inner());
        access.active = false;
        self.0.changed.notify_all();
    }
}

fn hash_file(path: &Path, expected_bytes: u64, mode: Check) -> Result<String, String> {
    // One read/hash operation at a time, including requested-model checks.
    // Background work yields every 64 KiB to a foreground request, which then
    // owns the hasher until its file finishes. Loading a small model need not
    // wait for an unrelated multi-GB background file to finish.
    static SCHEDULER: OnceLock<HashScheduler> = OnceLock::new();
    let scheduler = SCHEDULER.get_or_init(HashScheduler::default);
    let background = matches!(mode, Check::Background);
    let _foreground = (!background).then(|| scheduler.acquire(Check::Blocking));
    let mut file = File::open(path).map_err(|e| format!("Cannot open model file: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let _chunk = background.then(|| scheduler.acquire(Check::Background));
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read model file: {e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > expected_bytes {
            return Err("Model file changed during verification.".into());
        }
        hash.update(&buffer[..read]);
    }
    if total != expected_bytes {
        return Err("Model file changed during verification.".into());
    }
    let _final = background.then(|| scheduler.acquire(Check::Background));
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const XYZ: &str = "3608bca1e44ea6c4d268eb6db02260269892c0b42b86bbf1e77a6fa16c3c9282";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("sonojot-verification-{}", crate::store::new_id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"abc").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn counted(counter: &Arc<AtomicUsize>) -> Arc<Verifier> {
        let counter = counter.clone();
        Verifier::new(Arc::new(move |path, bytes, mode| {
            counter.fetch_add(1, Ordering::SeqCst);
            hash_file(path, bytes, mode)
        }))
    }

    #[test]
    fn persisted_hit_uses_canonical_path_without_rehashing() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let count = Arc::new(AtomicUsize::new(0));
        let first = counted(&count);
        assert_eq!(
            first.check(&fixture.0, &path, 3, ABC, Check::Blocking),
            Status::Ready
        );
        let restarted = counted(&count);
        let alias = fixture.0.join(".").join("model.bin");
        assert_eq!(
            restarted.check(&fixture.0, &alias, 3, ABC, Check::Background),
            Status::Ready
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stamp_changes_rehash_and_missing_files_never_hit() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let count = Arc::new(AtomicUsize::new(0));
        let verifier = counted(&count);
        assert!(verifier
            .check(&fixture.0, &path, 3, ABC, Check::Blocking)
            .ready());
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"xyz").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified + Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, ABC, Check::Blocking),
            Status::Missing
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, ABC, Check::Background),
            Status::Missing
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn missing_corrupt_and_oversized_cache_rehash_on_restart() {
        for corrupt in [
            None,
            Some(b"not json".to_vec()),
            Some(vec![b' '; MAX_CACHE_BYTES as usize + 1]),
        ] {
            let fixture = Fixture::new();
            let path = fixture.file("model.bin");
            let count = Arc::new(AtomicUsize::new(0));
            assert!(counted(&count)
                .check(&fixture.0, &path, 3, ABC, Check::Blocking)
                .ready());
            match corrupt {
                Some(bytes) => fs::write(fixture.0.join(CACHE_FILE), bytes).unwrap(),
                None => fs::remove_file(fixture.0.join(CACHE_FILE)).unwrap(),
            }
            assert!(counted(&count)
                .check(&fixture.0, &path, 3, ABC, Check::Blocking)
                .ready());
            assert_eq!(count.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn moved_root_and_moved_model_do_not_inherit_receipts() {
        let fixture = Fixture::new();
        let original_root = fixture.0.join("original");
        fs::create_dir(&original_root).unwrap();
        let path = original_root.join("model.bin");
        fs::write(&path, b"abc").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let verifier = counted(&count);
        assert!(verifier
            .check(&original_root, &path, 3, ABC, Check::Blocking)
            .ready());
        let moved = fixture.0.join("moved");
        fs::rename(&original_root, &moved).unwrap();
        assert!(counted(&count)
            .check(&moved, &moved.join("model.bin"), 3, ABC, Check::Blocking)
            .ready());
        fs::rename(moved.join("model.bin"), moved.join("renamed.bin")).unwrap();
        assert!(counted(&count)
            .check(&moved, &moved.join("renamed.bin"), 3, ABC, Check::Blocking)
            .ready());
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn replacement_identity_rehashes_with_same_size_and_mtime() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let replacement = fixture.file("replacement.bin");
        let count = Arc::new(AtomicUsize::new(0));
        let verifier = counted(&count);
        assert!(verifier
            .check(&fixture.0, &path, 3, ABC, Check::Blocking)
            .ready());
        let original = stamp(&path).unwrap();
        fs::write(&replacement, b"xyz").unwrap();
        File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_modified(original.modified)
            .unwrap();
        fs::remove_file(&path).unwrap();
        fs::rename(&replacement, &path).unwrap();
        let replaced = stamp(&path).unwrap();
        assert_eq!(original.bytes, replaced.bytes);
        assert_eq!(original.modified, replaced.modified);
        assert_ne!(original.identity, replaced.identity);
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, ABC, Check::Blocking),
            Status::Missing
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn expected_hash_and_size_are_part_of_cache_validation() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let count = Arc::new(AtomicUsize::new(0));
        let verifier = counted(&count);
        assert!(verifier
            .check(&fixture.0, &path, 3, ABC, Check::Blocking)
            .ready());
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, XYZ, Check::Blocking),
            Status::Missing
        );
        assert_eq!(
            verifier.check(&fixture.0, &path, 4, ABC, Check::Blocking),
            Status::Missing
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn activation_checks_bypass_a_matching_receipt() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        // A deliberately dishonest injected hasher proves that an activation
        // check cannot inherit a receipt, even when every stamp field matches.
        let verifier = Verifier::new(Arc::new(|_, _, _| Ok(XYZ.into())));
        assert!(verifier
            .check(&fixture.0, &path, 3, XYZ, Check::Blocking)
            .ready());
        assert!(!uncached(&path, 3, XYZ).unwrap());
        assert!(uncached(&path, 3, ABC).unwrap());
    }

    #[test]
    fn file_changed_during_hash_does_not_publish_a_receipt() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let verifier = Verifier::new(Arc::new(|path, _, _| {
            fs::write(path, b"changed").unwrap();
            Ok(ABC.into())
        }));
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, ABC, Check::Blocking),
            Status::Missing
        );
        assert!(verifier.state.lock().unwrap().results.is_empty());
    }

    fn wait_idle(verifier: &Verifier) {
        let state = verifier.state.lock().unwrap();
        let (state, timeout) = verifier
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |s| s.worker)
            .unwrap();
        assert!(
            !timeout.timed_out() && !state.worker,
            "verification worker did not drain"
        );
    }

    #[test]
    fn hashing_is_serial_and_foreground_preempts_the_next_background_chunk() {
        let scheduler = Arc::new(HashScheduler::default());
        let background_chunk = scheduler.acquire(Check::Background);
        let (order_tx, order_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let foreground_scheduler = scheduler.clone();
        let foreground_tx = order_tx.clone();
        let foreground = std::thread::spawn(move || {
            let _permit = foreground_scheduler.acquire(Check::Blocking);
            foreground_tx.send("foreground").unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let access = scheduler.access.lock().unwrap();
        let (access, timeout) = scheduler
            .changed
            .wait_timeout_while(access, Duration::from_secs(5), |s| {
                s.foreground_waiters == 0
            })
            .unwrap();
        assert!(!timeout.timed_out());
        drop(access);
        let background_scheduler = scheduler.clone();
        let background = std::thread::spawn(move || {
            let _permit = background_scheduler.acquire(Check::Background);
            order_tx.send("background").unwrap();
        });
        assert!(
            order_rx.try_recv().is_err(),
            "neither may hash while the chunk owns the slot"
        );
        drop(background_chunk);
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "foreground"
        );
        assert!(
            order_rx.try_recv().is_err(),
            "background cannot hash alongside foreground"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            order_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            "background"
        );
        foreground.join().unwrap();
        background.join().unwrap();
    }

    #[test]
    fn background_is_deduplicated_and_never_blocks_unrelated_checks() {
        let fixture = Fixture::new();
        let slow = fixture.file("slow.bin");
        let fast = fixture.file("fast.bin");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let count = Arc::new(AtomicUsize::new(0));
        let hashes = count.clone();
        let verifier = Verifier::new(Arc::new(move |path, bytes, mode| {
            hashes.fetch_add(1, Ordering::SeqCst);
            if path.file_name().unwrap() == "slow.bin" {
                entered_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
            }
            hash_file(path, bytes, mode)
        }));
        let completions = Arc::new(AtomicUsize::new(0));
        let completed = completions.clone();
        *verifier.completion.lock().unwrap() = Some(Arc::new(move || {
            completed.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(
            verifier.check(&fixture.0, &slow, 3, ABC, Check::Background),
            Status::Checking
        );
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..100 {
            assert_eq!(
                verifier.check(&fixture.0, &slow, 3, ABC, Check::Background),
                Status::Checking
            );
            assert_eq!(
                verifier.check(&fixture.0, &fast, 3, ABC, Check::Background),
                Status::Checking
            );
        }
        {
            let state = verifier.state.lock().unwrap();
            assert_eq!(state.running.len(), 1);
            assert_eq!(state.queued.len(), 1);
        }
        // A requested model takes its queued file directly; it does not wait
        // behind the blocked background hash of another model.
        assert!(verifier
            .check(&fixture.0, &fast, 3, ABC, Check::Blocking)
            .ready());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        release_tx.send(()).unwrap();
        wait_idle(&verifier);
        assert_eq!(completions.load(Ordering::SeqCst), 2);
        assert!(verifier
            .check(&fixture.0, &slow, 3, ABC, Check::Background)
            .ready());
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn queue_is_bounded_under_repeated_catalog_reads() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let verifier = counted(&Arc::new(AtomicUsize::new(0)));
        // Freeze dispatch to inspect scheduling without hashing hundreds of
        // files. Distinct expected pins exercise the same production bound.
        verifier.state.lock().unwrap().worker = true;
        for i in 0..MAX_ENTRIES + 32 {
            assert_eq!(
                verifier.check(
                    &fixture.0,
                    &path,
                    3,
                    &format!("{i:064x}"),
                    Check::Background
                ),
                Status::Checking
            );
        }
        assert_eq!(verifier.state.lock().unwrap().queued.len(), MAX_ENTRIES);
    }

    #[test]
    fn status_reader_observing_foreground_hash_receives_completion() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let verifier = Verifier::new(Arc::new(move |path, bytes, mode| {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            hash_file(path, bytes, mode)
        }));
        let completions = Arc::new(AtomicUsize::new(0));
        let completed = completions.clone();
        *verifier.completion.lock().unwrap() = Some(Arc::new(move || {
            completed.fetch_add(1, Ordering::SeqCst);
        }));
        let owner = verifier.clone();
        let root = fixture.0.clone();
        let model = path.clone();
        let loading =
            std::thread::spawn(move || owner.check(&root, &model, 3, ABC, Check::Blocking));
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            verifier.check(&fixture.0, &path, 3, ABC, Check::Background),
            Status::Checking
        );
        assert!(
            !verifier.state.lock().unwrap().worker,
            "no duplicate background hash"
        );
        release_tx.send(()).unwrap();
        assert_eq!(loading.join().unwrap(), Status::Ready);
        assert_eq!(completions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn foreground_only_verification_does_not_schedule_catalog_work() {
        let fixture = Fixture::new();
        let path = fixture.file("model.bin");
        let counter = Arc::new(AtomicUsize::new(0));
        let verifier = counted(&counter);
        *verifier.completion.lock().unwrap() = Some(Arc::new(|| {
            panic!("unobserved load must not refresh the catalog")
        }));
        assert!(verifier
            .check(&fixture.0, &path, 3, ABC, Check::Blocking)
            .ready());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(verifier.state.lock().unwrap().queued.is_empty());
    }

    #[test]
    fn all_and_any_consume_pending_dependencies_without_short_circuiting() {
        let count = std::cell::Cell::new(0);
        let items = || {
            [Status::Ready, Status::Checking, Status::Missing]
                .into_iter()
                .map(|s| {
                    count.set(count.get() + 1);
                    s
                })
        };
        assert_eq!(Status::any(items()), Status::Ready);
        assert_eq!(Status::all(items()), Status::Checking);
        assert_eq!(count.get(), 6);
        assert_eq!(
            Status::any([Status::Missing, Status::Checking]),
            Status::Checking
        );
    }
}
