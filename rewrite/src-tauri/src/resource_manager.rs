//! Shared resource coordinator for Round 6.
//!
//! Owns hardware probing, stage leases, memory-mode policy and idle retention.
//! It does not own capture, inference or settings persistence. Those stay with
//! their existing owners; this module only coordinates them.
//!
//! Design notes (MODELS_AND_MEMORY.md sections 4-6):
//! - Overlap keeps speech and cleanup resident when budget permits.
//! - Sequential releases ASR inference resources before cleanup loads.
//! - Active jobs are never evicted. Idle eviction is retention-gated and
//!   pressure-triggered, and a new request cancels pending eviction.
//! - No idle timer is driven by UI polling. Polling calls only read state.

use crate::domain::{MemoryMode, ModelRetention, Settings};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// Conservative safety reserve. Provisional values chosen for implementation,
/// not a Microsoft recommendation (see SOURCE_MAP W5):
/// 15% of total RAM plus a 1 GiB floor. Calibrated against a small native
/// smoke check on this machine; parent should re-tune after wider testing.
pub const RESERVE_FRACTION: f64 = 0.15;
pub const RESERVE_FLOOR_BYTES: u64 = 1 << 30;
/// Working overhead added to a plan (engine buffers, audio backlog, HTTP).
/// Provisional 256 MiB; unmeasured per-model, stated here so admission math
/// is reviewable instead of hidden.
pub const WORKING_OVERHEAD_BYTES: u64 = 256 << 20;

/// Selected CUDA device index. The GPU backend (`gpu_asr::Engine::load`)
/// requests provider `"cuda"` without a device id, which resolves to the
/// default device 0. Admission MUST use device 0's matching total/free pair,
/// never the largest single device across a multi-GPU machine.
pub const SELECTED_CUDA_DEVICE_INDEX: u32 = 0;

/// Shared mutation/selection guard (short critical sections only).
///
/// - Install/remove entry points (`model_manager`, legacy `optional_asr` /
///   `optional_llm`) hold this across their start-marker + busy-check sections
///   and across the whole legacy download (which fails fast for selection).
/// - Selection commit (`commands::save_settings` via
///   `with_model_selection`) holds this across check + file commit.
/// - Load acquires (`acquire_*_with_admission`) do NOT take this guard; they
///   take only the coordinator mutex briefly, so downloads never block
///   dictation. Selection vs load is serialized via the coordinator mutex
///   held across the selection commit closure (see `with_model_selection`).
/// Lock order wherever both are needed: selection guard -> coordinator.
/// No path takes them in the opposite order.
fn selection_guard() -> &'static Mutex<()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(()))
}

/// Try-lock the shared mutation/selection guard. Fail-fast with an
/// install-busy message so selection never commits mid-install.
pub fn try_lock_selection_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    selection_guard().try_lock().map_err(|_| {
        "A model is being installed. Wait for it to finish before changing models.".to_string()
    })
}

/// Blocking lock for short selection critical sections (parent commit path).
pub fn lock_selection_guard() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    selection_guard()
        .lock()
        .map_err(|_| "Selection state lock failed.".to_string())
}

/// Owned closure API for the parent `commands::save_settings` commit.
/// Holds the selection guard + coordinator across `commit`, checks busy
/// atomically (fail-closed), then runs the file write. `commit` itself must
/// NOT re-lock either guard (it runs inside the critical section).
/// `model_ids_changed` gates the busy/install checks; pure visual changes
/// skip them. See `runtime-last-fix-interface.md` for the exact parent hook.
pub fn with_model_selection<R>(
    model_ids_changed: bool,
    commit: impl FnOnce() -> Result<R, String>,
) -> Result<R, String> {
    let _sel = lock_selection_guard()?;
    if model_ids_changed {
        // Fail fast while any install/remove holds the registry marker.
        if crate::model_manager::install_in_progress() {
            return Err(
                "A model is being installed. Wait for it to finish before changing models.".into(),
            );
        }
        // Atomic busy check under the coordinator while selection is held,
        // so a concurrent load acquire cannot slip between check and commit.
        // `shared()` is private to this module; hold it across `commit`.
        let coord = shared()
            .lock()
            .map_err(|_| "Resource state lock failed.".to_string())?;
        if coord.asr_active.is_some() || coord.text_active.is_some() {
            return Err(
                "Local inference is busy. Wait for dictation or notes to finish before changing models.".into(),
            );
        }
        // Hold both guards across the file commit. Commits are short
        // (settings JSON write); loads block for milliseconds, never minutes.
        // `commit` must not call back into this module's locking APIs.
        let result = commit();
        drop(coord);
        result
    } else {
        commit()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Asr,
    Cleanup,
    Notes,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Asr => "asr",
            Stage::Cleanup => "cleanup",
            Stage::Notes => "notes",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HardwareSnapshot {
    pub total_ram_bytes: Option<u64>,
    pub available_ram_bytes: Option<u64>,
    pub dedicated_vram_bytes: Option<u64>,
}

impl HardwareSnapshot {
    pub fn unknown() -> Self {
        Self {
            total_ram_bytes: None,
            available_ram_bytes: None,
            dedicated_vram_bytes: None,
        }
    }
}

/// Probe current volatile memory. Total/available come from
/// GlobalMemoryStatusEx on Windows. Dedicated VRAM needs a DXGI enumeration
/// (parent dep); until then it stays unknown rather than invented.
pub fn probe_hardware() -> HardwareSnapshot {
    HardwareSnapshot {
        total_ram_bytes: total_ram_bytes(),
        available_ram_bytes: available_ram_bytes(),
        dedicated_vram_bytes: dedicated_vram_bytes(),
    }
}

/// Stable onboarding facts. The caller runs this on a blocking worker; no
/// inference process or permission prompt is needed. Volatile admission
/// readings continue to use `probe_hardware` and fresh GPU memory queries.
#[derive(Clone, Debug)]
pub(crate) struct SetupHardware {
    pub total_ram_bytes: Option<u64>,
    pub physical_cores: Option<u32>,
    pub logical_cores: Option<u32>,
    pub gpu: Option<GpuHardwareRow>,
}

pub(crate) fn setup_hardware() -> SetupHardware {
    static CPU: OnceLock<(Option<u64>, Option<u32>, Option<u32>)> = OnceLock::new();
    let (ram, physical, logical) = *CPU.get_or_init(|| {
        (
            total_ram_bytes(),
            physical_core_count(),
            std::thread::available_parallelism()
                .ok()
                .and_then(|n| u32::try_from(n.get()).ok()),
        )
    });
    SetupHardware {
        total_ram_bytes: ram,
        physical_cores: physical,
        logical_cores: logical,
        gpu: if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            cached_gpu_hardware()
        } else {
            // Apple GPUs use unified RAM, not a separate VRAM allowance.
            // There is no cheap existing AMD/Intel name probe in this module.
            None
        },
    }
}

#[cfg(windows)]
fn physical_core_count() -> Option<u32> {
    let mut bytes = 0u32;
    // RelationProcessorCore = 0. The first call reports the required size.
    unsafe { GetLogicalProcessorInformationEx(0, std::ptr::null_mut(), &mut bytes) };
    if bytes < 8 || bytes > 1024 * 1024 {
        return None;
    }
    // The variable-sized Windows records require pointer alignment.
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
    let capacity = buffer.len() * 8;
    let ok = unsafe {
        GetLogicalProcessorInformationEx(0, buffer.as_mut_ptr().cast(), &mut bytes)
    };
    if ok == 0 || bytes as usize > capacity {
        return None;
    }
    // SAFETY: the kernel initialized `bytes` bytes within the aligned buffer.
    let records = unsafe {
        std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), bytes as usize)
    };
    count_processor_core_records(records)
}

#[cfg(any(windows, test))]
fn count_processor_core_records(mut records: &[u8]) -> Option<u32> {
    let mut count = 0u32;
    while !records.is_empty() {
        let header = records.get(..8)?;
        let relation = u32::from_ne_bytes(header[..4].try_into().ok()?);
        let size = u32::from_ne_bytes(header[4..8].try_into().ok()?) as usize;
        if relation != 0 || size < 8 || size > records.len() {
            return None;
        }
        count = count.checked_add(1)?;
        records = &records[size..];
    }
    (count > 0).then_some(count)
}

#[cfg(target_os = "macos")]
fn physical_core_count() -> Option<u32> {
    let mut cores = 0u32;
    let mut size = std::mem::size_of::<u32>();
    let status = unsafe {
        libc::sysctlbyname(
            c"hw.physicalcpu".as_ptr(),
            (&mut cores as *mut u32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && size == 4 && cores > 0).then_some(cores)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn physical_core_count() -> Option<u32> {
    None
}

#[cfg(windows)]
#[repr(C)]
struct MemoryStatusEx {
    dw_length: u32,
    dw_memory_load: u32,
    ull_total_phys: u64,
    ull_avail_phys: u64,
    ull_total_page_file: u64,
    ull_avail_page_file: u64,
    ull_total_virtual: u64,
    ull_avail_virtual: u64,
    ull_avail_extended_virtual: u64,
}

#[cfg(windows)]
extern "system" {
    fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    fn GetLogicalProcessorInformationEx(
        relationship: i32,
        buffer: *mut u8,
        returned_length: *mut u32,
    ) -> i32;
}

#[cfg(windows)]
fn memory_status() -> Option<MemoryStatusEx> {
    let mut status = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    // SAFETY: fixed-size struct with dw_length set, no borrowed memory.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status)
}

#[cfg(windows)]
fn total_ram_bytes() -> Option<u64> {
    #[cfg(test)]
    {
        let injected = TEST_RAM_TOTAL.load(std::sync::atomic::Ordering::Acquire);
        if injected != u64::MAX {
            return (injected > 0).then_some(injected);
        }
    }
    memory_status().map(|s| s.ull_total_phys).filter(|v| *v > 0)
}

#[cfg(windows)]
fn available_ram_bytes() -> Option<u64> {
    #[cfg(test)]
    {
        let injected = TEST_RAM_AVAILABLE.load(std::sync::atomic::Ordering::Acquire);
        if injected != u64::MAX {
            return Some(injected);
        }
    }
    memory_status().map(|s| s.ull_avail_phys).filter(|_| true)
}

#[cfg(not(windows))]
fn total_ram_bytes() -> Option<u64> {
    #[cfg(test)]
    {
        let injected = TEST_RAM_TOTAL.load(std::sync::atomic::Ordering::Acquire);
        if injected != u64::MAX {
            return (injected > 0).then_some(injected);
        }
    }
    #[cfg(target_os = "macos")]
    {
        mac_memory::total()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(not(windows))]
fn available_ram_bytes() -> Option<u64> {
    #[cfg(test)]
    {
        let injected = TEST_RAM_AVAILABLE.load(std::sync::atomic::Ordering::Acquire);
        if injected != u64::MAX {
            return Some(injected);
        }
    }
    #[cfg(target_os = "macos")]
    {
        mac_memory::available()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

// These kernel statistics are readable without an app permission. Available
// is a conservative estimate: free + inactive pages. Speculative pages are
// already part of free; purgeable pages overlap other counters.
#[cfg(target_os = "macos")]
mod mac_memory {
    #[test]
    fn native_memory_is_bounded_without_permissions() {
        let total = total().expect("macOS physical memory");
        let available = available().expect("macOS available memory");
        assert!(total >= 1 << 30);
        assert!(available <= total);
    }

    pub(super) fn total() -> Option<u64> {
        let mut bytes = 0u64;
        let mut size = std::mem::size_of::<u64>();
        let status = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == 8 && bytes > 0).then_some(bytes)
    }

    pub(super) fn available() -> Option<u64> {
        // Oversized aligned storage accommodates VM statistics revisions.
        // Only the stable first four natural_t fields are read below.
        let mut stats = [0u64; 128];
        let mut count = (std::mem::size_of_val(&stats) / 4) as u32;
        let host = unsafe { libc::mach_host_self() };
        let status = unsafe {
            libc::host_statistics64(
                host,
                libc::HOST_VM_INFO64,
                stats.as_mut_ptr().cast(),
                &mut count,
            )
        };
        unsafe extern "C" {
            fn mach_port_deallocate(task: u32, name: u32) -> i32;
        }
        unsafe {
            mach_port_deallocate(libc::mach_task_self(), host);
        }
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if status != 0 || count < 4 || page_size <= 0 {
            return None;
        }
        let fields = stats.as_ptr().cast::<u32>();
        let pages = unsafe { u64::from(*fields) + u64::from(*fields.add(2)) };
        let total = total()?;
        let counted = pages.saturating_mul(page_size as u64).min(total);
        // Free + inactive leaves out compressible and purgeable memory, so it
        // can read near zero while macOS still has room. The kernel's own
        // memory-pressure figure (percent available, as used by jetsam) does
        // count them; take whichever is larger.
        Some(counted.max(pressure_level().map_or(0, |percent| total / 100 * percent)))
    }

    fn pressure_level() -> Option<u64> {
        let mut percent = 0i32;
        let mut size = std::mem::size_of::<i32>();
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.memorystatus_level".as_ptr(),
                (&mut percent as *mut i32).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && size == 4 && (0..=100).contains(&percent)).then_some(percent as u64)
    }
}

/// Controlled test overrides for deterministic admission/barrier tests.
/// Production always reads the OS; tests inject small volatile numbers so two
/// individually-fitting but jointly-oversized loads can be proven to serialize
/// without allocating gigabytes or OOMing the host.
#[cfg(test)]
static TEST_RAM_TOTAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
#[cfg(test)]
static TEST_RAM_AVAILABLE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(u64::MAX);
#[cfg(test)]
static TEST_GPU_TABLE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

#[cfg(test)]
fn test_gpu_table() -> &'static Mutex<Option<String>> {
    TEST_GPU_TABLE.get_or_init(|| Mutex::new(None))
}

/// Inject RAM numbers for tests. `u64::MAX` means "no override, probe the OS".
#[cfg(test)]
pub fn set_test_memory(total: u64, available: u64) {
    TEST_RAM_TOTAL.store(total, std::sync::atomic::Ordering::Release);
    TEST_RAM_AVAILABLE.store(available, std::sync::atomic::Ordering::Release);
}

#[cfg(test)]
pub fn clear_test_memory() {
    TEST_RAM_TOTAL.store(u64::MAX, std::sync::atomic::Ordering::Release);
    TEST_RAM_AVAILABLE.store(u64::MAX, std::sync::atomic::Ordering::Release);
}

/// Inject an `nvidia-smi` CSV body for tests. The body must use the
/// `index, memory.total, memory.free` CSV shape (MiB, no header), e.g.
/// `"0, 12226, 9000\n1, 6144, 5000\n"`. `None` clears back to live probing.
#[cfg(test)]
pub fn set_test_gpu_table(csv: Option<String>) {
    if let Ok(mut slot) = test_gpu_table().lock() {
        *slot = csv;
    }
}

/// One probed CUDA device row: index + matching total/free pair (bytes).
/// Total and free always come from the SAME row; they are never mixed across
/// devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuDeviceRow {
    pub index: u32,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// Parse `nvidia-smi --query-gpu=index,memory.total,memory.free` CSV output.
/// Strict: 1-16 rows, `index, total_mib, free_mib`, 0 < total <= 256 GiB,
/// 0 <= free <= total. Any violation yields None (honest unknown), never a
/// partial max.
pub fn parse_gpu_table(text: &str) -> Option<Vec<GpuDeviceRow>> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() != 3 {
            return None;
        }
        let index: u32 = parts[0].parse().ok()?;
        let total_mib: u64 = parts[1].split_whitespace().next()?.parse().ok()?;
        let free_mib: u64 = parts[2].split_whitespace().next()?.parse().ok()?;
        if total_mib == 0 || total_mib > 256 * 1024 {
            return None;
        }
        if free_mib > total_mib {
            return None;
        }
        rows.push(GpuDeviceRow {
            index,
            total_bytes: total_mib.saturating_mul(1024 * 1024),
            free_bytes: free_mib.saturating_mul(1024 * 1024),
        });
        if rows.len() > 16 {
            return None;
        }
    }
    if rows.is_empty() {
        return None;
    }
    rows.sort_by_key(|r| r.index);
    rows.dedup_by_key(|r| r.index);
    Some(rows)
}

/// Live single-shot probe returning ALL device rows with matched pairs.
/// Test override (see `set_test_gpu_table`) takes precedence under `cfg(test)`.
fn probe_gpu_table() -> Option<Vec<GpuDeviceRow>> {
    #[cfg(test)]
    {
        if let Ok(slot) = test_gpu_table().lock() {
            if let Some(csv) = slot.as_ref() {
                return parse_gpu_table(csv);
            }
        }
        // When test RAM is injected but no GPU CSV is injected, report
        // unknown VRAM rather than probing the live host (keeps barrier tests
        // hermetic and never blocks on a 2 s child).
        if TEST_RAM_AVAILABLE.load(std::sync::atomic::Ordering::Acquire) != u64::MAX {
            return None;
        }
    }
    parse_gpu_table(&query_nvidia_smi("index,memory.total,memory.free")?)
}

/// Shared hidden, bounded query for both admission and cached setup metadata.
fn query_nvidia_smi(fields: &str) -> Option<String> {
    let mut command = std::process::Command::new("nvidia-smi");
    command
        .arg(format!("--query-gpu={fields}"))
        .arg("--format=csv,noheader,nounits")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().ok()?;
    let start = Instant::now();
    let deadline = Duration::from_millis(2000);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if output.stdout.len() > 64 * 1024 {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuHardwareRow {
    pub index: u32,
    pub total_bytes: Option<u64>,
    pub name: Option<String>,
    pub driver_version: Option<String>,
    pub compute_capability: Option<String>,
}

/// The same indexed NVIDIA table as admission, with optional identity fields.
/// An unavailable memory counter must not turn into zero or borrow another GPU.
fn parse_gpu_hardware_table(text: &str) -> Option<Vec<GpuHardwareRow>> {
    fn known(value: &str) -> Option<String> {
        let value = value.trim_matches('"').trim();
        (!value.is_empty()
            && value.len() <= 160
            && !value.chars().any(char::is_control)
            && !matches!(value.to_ascii_lowercase().as_str(), "n/a" | "[n/a]" | "unknown" | "not supported" | "[not supported]"))
            .then(|| value.to_owned())
    }
    let mut rows: Vec<GpuHardwareRow> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if !matches!(parts.len(), 5 | 6) || rows.len() >= 16 {
            return None;
        }
        let index = parts[0].parse().ok()?;
        if rows.iter().any(|row| row.index == index) {
            return None;
        }
        let total_bytes = parts[1].parse::<u64>().ok()
            .filter(|mib| *mib > 0 && *mib <= 256 * 1024)
            .map(|mib| mib * 1024 * 1024);
        rows.push(GpuHardwareRow {
            index,
            total_bytes,
            name: known(parts[3]),
            driver_version: known(parts[4]),
            compute_capability: parts.get(5).and_then(|value| known(value)),
        });
    }
    (!rows.is_empty()).then_some(rows)
}

fn selected_gpu_hardware(text: &str) -> Option<GpuHardwareRow> {
    parse_gpu_hardware_table(text)?
        .into_iter()
        .find(|row| row.index == SELECTED_CUDA_DEVICE_INDEX)
}

fn cached_gpu_hardware() -> Option<GpuHardwareRow> {
    // Cache successes and failures; retries allow a driver/device change to
    // become visible without spawning a probe for every profile card.
    const CACHE_LIFETIME: Duration = Duration::from_secs(30);
    static SAMPLE: OnceLock<Mutex<Option<(Instant, Option<GpuHardwareRow>)>>> = OnceLock::new();
    let mut sample = SAMPLE.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if let Some((at, gpu)) = sample.as_ref() {
        if at.elapsed() < CACHE_LIFETIME {
            return gpu.clone();
        }
    }
    let gpu = query_nvidia_smi("index,memory.total,memory.free,name,driver_version,compute_cap")
        // Older nvidia-smi versions lack compute_cap; retain known facts but
        // leave capability unknown, so they cannot qualify for CUDA setup.
        .or_else(|| query_nvidia_smi("index,memory.total,memory.free,name,driver_version"))
        .and_then(|text| selected_gpu_hardware(&text));
    *sample = Some((Instant::now(), gpu.clone()));
    gpu
}

/// Matching total/free pair for the ACTUAL selected CUDA device (index
/// `SELECTED_CUDA_DEVICE_INDEX`, normally 0). This is the only VRAM pair
/// admission may use. It is never the largest device, never a sum, and never
/// a total from one card mixed with free from another.
pub fn selected_gpu_pair() -> Option<(u64, u64)> {
    let rows = probe_gpu_table()?;
    rows.iter()
        .find(|r| r.index == SELECTED_CUDA_DEVICE_INDEX)
        .map(|r| (r.total_bytes, r.free_bytes))
}

/// UI-only sample. Admission always probes afresh; opening several role
/// controls must not spawn a hardware probe per control.
pub fn displayed_gpu_pair() -> Option<(u64, u64)> {
    static SAMPLE: OnceLock<Mutex<Option<(Instant, Option<(u64, u64)>)>>> = OnceLock::new();
    let mut sample = SAMPLE.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if let Some((at, pair)) = *sample {
        if at.elapsed() < Duration::from_secs(5) {
            return pair;
        }
    }
    let pair = selected_gpu_pair();
    *sample = Some((Instant::now(), pair));
    pair
}

/// Total VRAM on the selected device (normally 0). Honest unknown when the
/// probe fails; never a multi-GPU max/sum presented as the selected device.
pub fn selected_gpu_total_vram_bytes() -> Option<u64> {
    selected_gpu_pair().map(|(total, _)| total)
}

/// Free VRAM on the selected device (normally 0). Honest unknown when the
/// probe fails; never another device's free bytes.
pub fn selected_gpu_free_vram_bytes() -> Option<u64> {
    selected_gpu_pair().map(|(_, free)| free)
}

/// Dedicated VRAM via a bounded `nvidia-smi` query when the tool exists.
///
/// Returns the SELECTED device's `memory.total` (MiB -> bytes), never the
/// largest and never the sum across GPUs. Returns None when nvidia-smi is
/// missing, times out, or fails to parse — never a fabricated number, never
/// the truncated 32-bit Win32 AdapterRAM value.
fn dedicated_vram_bytes() -> Option<u64> {
    #[cfg(test)]
    {
        selected_gpu_total_vram_bytes()
    }
    #[cfg(not(test))]
    {
        static DEDICATED_VRAM_BYTES: OnceLock<Option<u64>> = OnceLock::new();
        *DEDICATED_VRAM_BYTES.get_or_init(selected_gpu_total_vram_bytes)
    }
}

/// Legacy name kept for back-compat. Now returns the SELECTED device total
/// (normally device 0), NOT the largest. New code must call
/// `selected_gpu_total_vram_bytes`. No caller may claim another device's
/// memory proves fit on the selected device.
pub fn largest_gpu_total_vram_bytes() -> Option<u64> {
    selected_gpu_total_vram_bytes()
}

/// Legacy name kept for back-compat. Now returns the SELECTED device free
/// (normally device 0), NOT the largest. New code must call
/// `selected_gpu_free_vram_bytes`.
pub fn largest_gpu_free_vram_bytes() -> Option<u64> {
    selected_gpu_free_vram_bytes()
}

/// Back-compat probe name. Now returns the SELECTED device free (normally
/// device 0) via the matched-pair probe, never a max across devices.
/// Hardware availability is distinct from observed execution.
pub fn nvidia_smi_free_vram_bytes() -> Option<u64> {
    selected_gpu_free_vram_bytes()
}

/// Free disk bytes for install admission. Windows uses GetDiskFreeSpaceExW
/// without new deps; other platforms return None (caller treats as unknown).
pub fn disk_free_bytes(path: &std::path::Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free: u64 = 0;
        let mut _total: u64 = 0;
        let mut _avail: u64 = 0;
        // SAFETY: pointers to live u64 out-params, path NUL-terminated.
        extern "system" {
            fn GetDiskFreeSpaceExW(
                dir: *const u16,
                free_to_caller: *mut u64,
                total: *mut u64,
                free_total: *mut u64,
            ) -> i32;
        }
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut _total, &mut _avail) };
        (ok != 0).then_some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

pub fn reserve_bytes(total_ram: Option<u64>) -> u64 {
    let fraction = total_ram.map(|t| ((t as f64) * RESERVE_FRACTION) as u64);
    fraction
        .map(|f| f.max(RESERVE_FLOOR_BYTES))
        .unwrap_or(RESERVE_FLOOR_BYTES)
}

pub fn format_bytes(bytes: Option<u64>) -> String {
    match bytes {
        None => "unknown".to_owned(),
        Some(v) => {
            const GB: f64 = 1024.0 * 1024.0 * 1024.0;
            const MB: f64 = 1024.0 * 1024.0;
            if v as f64 >= GB {
                format!("{:.1} GiB", v as f64 / GB)
            } else if v as f64 >= MB {
                format!("{:.0} MiB", v as f64 / MB)
            } else {
                format!("{v} bytes")
            }
        }
    }
}

/// Setup callout with real values substituted. Unknown stays unknown.
pub fn guidance_text(hardware: &HardwareSnapshot, plan_range: Option<(u64, u64)>) -> String {
    let plan = match plan_range {
        Some((lo, hi)) if lo == hi => format_bytes(Some(lo)),
        Some((lo, hi)) => format!("{}–{}", format_bytes(Some(lo)), format_bytes(Some(hi))),
        None => "an unmeasured amount".to_owned(),
    };
    format!(
        "This computer has {} RAM; about {} is available now. Your selected dictation setup is estimated to use {} while active. Leave room for your other apps.",
        format_bytes(hardware.total_ram_bytes),
        format_bytes(hardware.available_ram_bytes),
        plan
    )
}

// ---------------------------------------------------------------------------
// Loaded-memory planning estimates (observed where cited, otherwise provisional).
//
// Observations name the machine and process scope. Upper bounds add planning
// headroom; a direct-server observation is not an app-worker measurement.
// Disk sizes are pinned HF LFS bytes (2026-09-16); loaded RAM adds engine
// overhead, working buffers, context/KV and runtime copies on top, so disk
// bytes are never relabelled as measured RAM.
//
// Assumption sources (see SOURCE_MAP_AND_RESEARCH.md):
// - W1 OpenAI Whisper README: upstream GPU/speed figures are
//   deployment-specific; they do not transfer to SonoJot RAM.
// - W3 whisper.cpp registry: file size and runtime memory are separate;
//   quantized artifact size seeds but does not equal resident memory.
// - W4 llama.cpp server README: context (`--ctx-size 4096` in cleanup.rs
//   launch args), model/KV cache and idle-sleep controls exist upstream;
//   SonoJot's bundled server is launched with ctx 4096, 4 threads, parallel 1.
// - W5 GlobalMemoryStatusEx: availability is volatile; admission re-reads.
// - Pinned disk bytes: optional_asr/optional_llm/whisper_catalog package_bytes
//   (HF LFS sizes + SHA-256, 2026-09-16).
//
// Per-model assumptions:
// - ASR ONNX CPU (sherpa-onnx 1.13.8 CPU provider): weights (disk) + ORT
//   arena/activations (~30-50% of weights for int8 conformer/transducer or
//   whisper encoder-decoder) + silero VAD (~1 MB weights + ~50 MB runtime) +
//   audio backlog/capture buffers (bounded by WORKING_OVERHEAD share).
// - Text GGUF Q4_K_M (llama-server, ctx 4096): weights (disk) + F16 KV cache
//   for 4096 tokens (scales with layers*KV-heads; ~130 MB for 1.2B class,
//   ~400-600 MB for 4B class) + compute buffers (~200-400 MB) + server/HTTP
//   overhead (in WORKING_OVERHEAD).
// - Whisper int8 scales with checkpoint disk size; turbo has fewer decoder
//   layers than large-v3 so its interval sits below large-v3 despite similar
//   family (upstream design, not a quality claim).
// - Unknown stays unknown: GPU display-only rows, unsupported backends and
//   unlisted IDs return None.
// ---------------------------------------------------------------------------

const MIB: u64 = 1024 * 1024;

/// Host-RAM planning interval (observed or provisional, with upper headroom).
/// None means unknown/unsupported — never zero-as-known, never fabricated.
///
/// CUDA ASR rows report HOST RAM including loading copies; device
/// memory lives in `estimated_vram_interval`. Host + device are never summed
/// into one number.
pub fn estimated_ram_interval(id: &str) -> Option<(u64, u64)> {
    if id == crate::granite_model::GRANITE_ID {
        // Windows native Q8 benchmark: 2.19 GiB process-tree RSS. Loading
        // allocations are not measured; leave headroom. Metal is provisional
        // unified memory, counted here once rather than also as device VRAM.
        return if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some((4_000 * MIB, 6_000 * MIB))
        } else {
            Some((2_250 * MIB, 3_500 * MIB))
        };
    }
    if let Some(model) = crate::mac_speech::find(id) {
        return Some((model.ram_min_bytes, model.ram_max_bytes));
    }
    let interval = match id {
        // App-worker peak RAM, holdout file mode, one Windows workstation:
        // evaluation/BENCHMARK-WINDOWS-2026-09-28.md, "App-worker peak RAM".
        // Reported GB are decimal. Keep headroom above each observation;
        // the separate plan overhead is not part of the speech measurement.
        "moonshine-medium" => (560_000_000, 800 * MIB),
        // Small was not measured in that report; retain its provisional range.
        "moonshine-small" => (400 * MIB, 800 * MIB),
        "parakeet-tdt-0.6b-v2" => (1200 * MIB, 2000 * MIB),
        "qwen3-asr-1.7b" => (3000 * MIB, 4600 * MIB),
        crate::llama_asr::MODEL_ID => (900 * MIB, 1600 * MIB),
        crate::llama_asr::CPU_MODEL_ID => (2100 * MIB, 2800 * MIB),
        // Provisional: 1.02 GB weights plus CPU decoder, KV and audio buffers.
        crate::llama_asr::SMALL_CPU_MODEL_ID => (1400 * MIB, 2300 * MIB),
        // M1 Pro 16 GiB, bundled b10964 Metal, ctx 2048, load-mode none,
        // -ngl 999, explicit English, full 12-clip corpus (2026-09-28).
        // Lower endpoints preserve decimal-GB server RSS observations.
        // Upper bounds cover the later worker observations and load/decode
        // variance; the planner's shared working allowance remains separate.
        // See evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.
        crate::llama_asr::METAL_Q8_MODEL_ID => (2_800_000_000, 3600 * MIB),
        crate::llama_asr::METAL_Q4_MODEL_ID => (2_360_000_000, 3000 * MIB),
        "whisper-tiny" => (300 * MIB, 600 * MIB),
        "whisper-tiny.en" => (530_000_000, 700 * MIB),
        "whisper-base" | "whisper-base.en" => (450 * MIB, 850 * MIB),
        "whisper-small" => (1_280_000_000, 1600 * MIB),
        "whisper-small.en" => (850 * MIB, 1450 * MIB),
        "whisper-medium" | "whisper-medium.en" => (1500 * MIB, 2600 * MIB),
        "whisper-large-v3" => (2600 * MIB, 4100 * MIB),
        "whisper-large-v3-turbo" => (1700 * MIB, 2700 * MIB),
        "whisper-large-v1" | "whisper-large-v2" => (2600 * MIB, 4100 * MIB),
        // Include host weight deserialization/copies during load, even though
        // inference weights also occupy VRAM. These are provisional peaks.
        "whisper-tiny-fp32-cuda" => (500 * MIB, 1200 * MIB),
        "qwen3-asr-1.7b-fp16-cuda" => (9000 * MIB, 12000 * MIB),
        "whisper-small-fp32-cuda" => (1500 * MIB, 3500 * MIB),
        "whisper-large-v3-turbo-fp32-cuda" => (4000 * MIB, 8500 * MIB),
        "parakeet-tdt-0.6b-v2-fp16-cuda" => (4000 * MIB, 6000 * MIB),
        "lfm2.5-1.2b" => (1350 * MIB, 2150 * MIB),
        crate::optional_llm::SMART_CLEANUP_ID => (1000 * MIB, 1500 * MIB),
        "lfm2.5-2.6b" => (2450 * MIB, 3500 * MIB),
        "qwen3-4b-instruct-2507" => (3500 * MIB, 4900 * MIB),
        "ternary-bonsai-2-27b" => (7000 * MIB, 10000 * MIB),
        "local-rules" | "off" | "" => (0, 0),
        _ => return None,
    };
    Some(interval)
}

/// Provisional VRAM interval. CPU-only ASR is known-zero (never CUDA);
/// Text intervals assume full CUDA offload; CPU preference reserves zero VRAM.
/// CUDA ASR rows carry provisional device intervals with explicit method in
/// `estimate_basis`; unknown stays unknown for unsupported ids.
pub fn estimated_vram_interval(id: &str) -> Option<(u64, u64)> {
    if id == crate::granite_model::GRANITE_ID {
        return if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some((0, 0))
        } else {
            // 3941 MiB sampled CUDA device-use increase, plus planning headroom.
            Some((3_941 * MIB, 4_800 * MIB))
        };
    }
    match id {
        "moonshine-medium"
        | "moonshine-small"
        | "parakeet-tdt-0.6b-v2"
        | "qwen3-asr-1.7b"
        | "whisper-small-metal"
        | "whisper-turbo-metal"
        // Metal shares system RAM; never add a separate device allocation.
        | crate::llama_asr::METAL_Q8_MODEL_ID
        | crate::llama_asr::METAL_Q4_MODEL_ID
        | "whisper-tiny"
        | "whisper-tiny.en"
        | "whisper-base"
        | "whisper-base.en"
        | "whisper-small"
        | "whisper-small.en"
        | "whisper-medium"
        | "whisper-medium.en"
        | "whisper-large-v3"
        | "whisper-large-v3-turbo"
        | "whisper-large-v1"
        | "whisper-large-v2"
        | "local-rules"
        | "off"
        | "" => Some((0, 0)),
        // Whisper tiny FP32 CUDA: isolated SonoJot run on RTX 5070 observed
        // VRAM 1822 MiB idle -> 2527 MiB peak (+705 MiB) for the model decode
        // (gpu-asr-evidence.json). Interval widens that single observation with
        // CUDA context + VAD/CPU-side variance; provisional, not a fleet rating.
        "whisper-tiny-fp32-cuda" => Some((600 * MIB, 1100 * MIB)),
        // Qwen FP16 CUDA: MEASURED native peak on RTX 5070 (gpu-native
        // evidence 2026-09-16): 1791 -> 8919 MiB (+~7.1 GiB incremental,
        // +7.2 GiB in the switch run; ~9 GB total device with the ~1.8 GiB
        // baseline). The interval starts at the measured peak and adds
        // cautious allocator headroom; it is NOT the old 4800-6500 MiB
        // provisional estimate, which underestimated the observed delta.
        // Host RAM for this row stays a separate unmeasured interval above.
        "qwen3-asr-1.7b-fp16-cuda" => Some((7100 * MIB, 8400 * MIB)),
        crate::llama_asr::MODEL_ID => Some((3200 * MIB, 4200 * MIB)),
        crate::llama_asr::CPU_MODEL_ID => Some((0, 0)),
        crate::llama_asr::SMALL_CPU_MODEL_ID => Some((0, 0)),
        "whisper-small-fp32-cuda" => Some((1800 * MIB, 3500 * MIB)),
        "whisper-large-v3-turbo-fp32-cuda" => Some((4500 * MIB, 7500 * MIB)),
        "parakeet-tdt-0.6b-v2-fp16-cuda" => Some((2200 * MIB, 4500 * MIB)),
        // Full CUDA offload, ctx 4096: weights + KV + compute buffers. These
        // are conservative estimates, not measurements of the user's machine.
        "lfm2.5-1.2b" => Some((1200 * MIB, 2200 * MIB)),
        crate::optional_llm::SMART_CLEANUP_ID => Some((850 * MIB, 1200 * MIB)),
        "lfm2.5-2.6b" => Some((2200 * MIB, 3600 * MIB)),
        "qwen3-4b-instruct-2507" => Some((3200 * MIB, 5000 * MIB)),
        "ternary-bonsai-2-27b" => Some((7000 * MIB, 9500 * MIB)),
        _ => None,
    }
}

/// Human-readable basis for an estimate, naming weights + overhead parts.
pub fn estimate_basis(id: &str) -> String {
    if id == crate::llama_asr::SMALL_CPU_MODEL_ID {
        return "Provisional CPU planning range: 1.02 GB Q8 decoder/projector plus native buffers and a 2,048-token context. No separate GPU memory. Not a measured peak.".into();
    }
    match id {
        crate::granite_model::GRANITE_ID => "Observed 2026-10-10 native Q8_0 benchmark on RTX 5070: 2.19 GiB process-tree RSS and 3941 MiB device-use increase. Windows bounds add loading/allocator headroom. Apple Metal unified memory is provisional, counted once in host RAM. Meetings hold one ASR lease and drop the worker before speaker labeling.".into(),
        crate::optional_llm::SMART_CLEANUP_ID => "Observed 2026-09-28 direct-server bake-off on Windows: SpeakoFlow Q4_K_M peak RSS 1084.5 MiB CUDA / 994.7 MiB CPU (four threads); whole-device VRAM delta 863 MiB. Weights are 541,903,616 bytes. Planning bounds add headroom; these are not app-wide peaks. CPU-only uses no VRAM. Mac unified-memory use is unmeasured and uses the provisional host range.".into(),
        crate::llama_asr::CPU_MODEL_ID => "Observed 2026-09-28 on a Ryzen 9 9900X through the app worker with llama-server b10900 CPU: Q4_K_M decoder plus Q8_0 audio encoder, 2.14 GB peak memory for worker plus server with a 2,048-token context and no memory mapping. Upper bound adds headroom; no GPU memory.".into(),
        crate::llama_asr::MODEL_ID => "Observed 2026-09-28 on RTX 5070 through the app worker with the pinned llama-server b10900 CUDA 13.3, a 2,048-token context and no memory mapping: about 1.0 GB peak host memory for worker plus server, and +3.4 GB whole-device memory (CUDA context included, measured with memory mapping). Upper bounds add headroom; not a fleet rating.".into(),
        crate::llama_asr::METAL_Q8_MODEL_ID | crate::llama_asr::METAL_Q4_MODEL_ID => {
            let (quantization, rss, worker_rss, upper_mib) = if id == crate::llama_asr::METAL_Q8_MODEL_ID {
                ("Q8_0", "2.8", "2.91", 3600)
            } else {
                ("Q4_K_M", "2.36", "2.35", 3000)
            };
            format!("Observed 2026-09-28 on one M1 Pro with 16 GiB unified memory: {quantization} decoder plus Q8_0 audio projector, {rss} GB server RSS (decimal). App-bundled llama-server b10964 mac-arm64 Metal, context 2048, load-mode none, -ngl 999, full 12-clip corpus with explicit English. Later file and real-time app-worker runs peaked at {worker_rss} GB combined evaluator, worker and server RSS, sampled every 100 ms. The {upper_mib} MiB planning upper bound adds load/decode headroom. Notes and the planner's 256 MiB working allowance are separate. CPU and GPU share RAM; no separate VRAM is added. These are single-machine observations, not guarantees for other Macs. See evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.")
        },
        "ternary-bonsai-2-27b" => "Preview estimate, not measured in SonoJot. PrismML Ternary Bonsai 2 PQ2_0 weights are 7.2 GB; short-context CUDA planning includes the Prism fork runtime, CUDA workspaces and KV cache. The bundled stock llama-server cannot load this format.".to_owned(),
        "whisper-small-fp32-cuda" | "whisper-large-v3-turbo-fp32-cuda" | "parakeet-tdt-0.6b-v2-fp16-cuda" => "Preview estimate, not measured: full floating-point model weights plus native sessions, CUDA workspaces and activation headroom for short VAD chunks. Host range includes loading copies; GPU range is separate. Validate on your hardware.".to_owned(),
        "moonshine-medium" => "Observed 2026-09-28 app-worker peak RAM in holdout file mode: 0.56 GB on Windows. See evaluation/BENCHMARK-WINDOWS-2026-09-28.md, App-worker peak RAM. Upper bound adds headroom; Notes and the app's working allowance are separate. One computer, not a guarantee.".into(),
        "whisper-tiny.en" => "Observed 2026-09-28 app-worker peak RAM in holdout file mode: 0.53 GB on Windows. See evaluation/BENCHMARK-WINDOWS-2026-09-28.md, App-worker peak RAM. Upper bound adds headroom; this observation does not measure multilingual Tiny or CUDA.".into(),
        "whisper-small" => "Observed 2026-09-28 app-worker peak RAM in holdout file mode: 1.28 GB on Windows. See evaluation/BENCHMARK-WINDOWS-2026-09-28.md, App-worker peak RAM. Upper bound adds headroom; this observation does not measure Small.en, CUDA or Metal.".into(),
        "moonshine-small" => format!(
            "Provisional estimate, not measured in SonoJot. Streaming ONNX bundle ({id}) + sherpa/Moonshine ORT CPU arena/activations (~30-50% of weights) + silero VAD + bounded audio backlog. Disk size is real and separate; it is not relabelled as RAM."
        ),
        "parakeet-tdt-0.6b-v2" | "qwen3-asr-1.7b" => format!(
            "Provisional estimate, not measured in SonoJot. Pinned int8 weights ({id}, HF LFS 2026-09-16) + ORT CPU arena/activations + conformer/transducer working buffers + VAD + audio backlog. See estimated interval; disk bytes shown separately."
        ),
        "whisper-tiny-fp32-cuda" => "Provisional estimate, partly observed. Host RAM covers VAD (CPU) + audio backlog + ORT host side. VRAM interval widens the isolated RTX 5070 observation (+705 MiB model decode, 1822->2527 MiB) with CUDA context variance. Same CUDA 13 stack proven end-to-end; not a fleet rating.".to_owned(),
        "qwen3-asr-1.7b-fp16-cuda" => "Observed 2026-09-17 on RTX 5070 with the isolated app worker: sampled private host peak 10.51 GB, working set 2.60 GB, device-use increase 7.56 GB. Host planning range covers private loading allocations, not resident physical RAM; upper bounds add headroom. One 6.625s clip, not a fleet rating.".to_owned(),
        "whisper-small-metal" | "whisper-turbo-metal" => "Provisional unified-memory estimate: F16 Whisper weights, Metal compute buffers, mel and audio buffers. CPU and GPU share one memory pool; no separate VRAM allocation is added.".into(),
        w if w.starts_with("whisper-") => format!(
            "Provisional estimate, not measured in SonoJot. Pinned int8 encoder/decoder + tokens ({id}, HF LFS 2026-09-16) + sherpa OfflineWhisper CPU runtime (ORT arena, mel/VAD buffers). Turbo interval sits below large-v3 (fewer decoder layers, upstream design)."
        ),
        "lfm2.5-1.2b" | "lfm2.5-2.6b" | "qwen3-4b-instruct-2507" => format!(
            "Provisional loading estimate, not measured: Q4_K_M GGUF ({id}), 4096-token F16 KV and compute buffers. GPU range assumes full CUDA offload; CPU-only uses no VRAM. Cleanup and notes share one text worker, so their peak is the larger model rather than the sum."
        ),
        "local-rules" | "off" | "" => "Host-only rules; no model weights, no measurable inference RAM.".to_owned(),
        _ => "Unmeasured in SonoJot with this backend and context. Disk size is real; loaded RAM/VRAM is not inferred from disk bytes. Quantization, backend and context affect it.".to_owned(),
    }
}

/// Evidence label: observed peaks inform some intervals; upper bounds remain
/// planning estimates, not measured guarantees for other hardware.
pub fn estimate_evidence(id: &str) -> &'static str {
    match id {
        crate::granite_model::GRANITE_ID => "Observed Windows native Q8_0, evaluation/asr-bench-2026-10/NAR-TRANSCRIBECPP.md. Upper bounds are planning estimates; Apple Metal execution and memory unmeasured.",
        crate::optional_llm::SMART_CLEANUP_ID => "Observed Windows direct-server peaks, evaluation/CLEANUP-BAKEOFF-2026-09-28.md and runs/cleanup-bakeoff-2026/measurements. Upper bounds are planning estimates; Mac unmeasured.",
        crate::llama_asr::CPU_MODEL_ID => "Observed 2026-09-28 app worker (runs/windows-2026/integrated2): 2.14 GB host peak. Bounds widened from one machine.",
        crate::llama_asr::MODEL_ID => "Observed 2026-09-28 app worker (runs/windows-2026/integrated2): 1.0 GB host peak; +3.43 GB device-use delta offline. Bounds widened from one machine.",
        crate::llama_asr::METAL_Q8_MODEL_ID => "Observed 2026-09-28 M1 Pro 16 GiB, b10964 Metal: 2.8 GB direct-server RSS; later evaluator + worker + server peak RSS 2.91 GB. Upper bound adds headroom; one machine. See evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.",
        crate::llama_asr::METAL_Q4_MODEL_ID => "Observed 2026-09-28 M1 Pro 16 GiB, b10964 Metal: 2.36 GB direct-server RSS; later evaluator + worker + server peak RSS 2.35 GB; 2026-10-13 M4 16 GiB, app-pinned Metal llama-server: 2.16 GB peak RSS. Upper bound adds about 30% headroom. See evaluation/BENCHMARK-MAC-METAL-2026-09-28.md.",
        "local-rules" | "off" | "" => "Host rules; no model rating applies.",
        "whisper-tiny-fp32-cuda" => "Provisional 2026-09-16; VRAM widens isolated RTX 5070 observation (+705 MiB decode, 1822->2527 MiB, public audio, gpu-asr-evidence.json); host RAM provisional",
        "qwen3-asr-1.7b-fp16-cuda" => "Observed 2026-09-17 app worker (native-qwen.json): 10.51 GB sampled private host peak, 2.60 GB working set, +7.56 GB device-use delta. Bounds are estimates widened from this single run.",
        "moonshine-medium" => "Observed 2026-09-28 app-worker peak: 0.56 GB (BENCHMARK-WINDOWS-2026-09-28.md, holdout file mode). Upper bound adds headroom.",
        "whisper-tiny.en" => "Observed 2026-09-28 app-worker peak: 0.53 GB (BENCHMARK-WINDOWS-2026-09-28.md, holdout file mode). Upper bound adds headroom.",
        "whisper-small" => "Observed 2026-09-28 app-worker peak: 1.28 GB (BENCHMARK-WINDOWS-2026-09-28.md, holdout file mode). Upper bound adds headroom.",
        "moonshine-small" | "parakeet-tdt-0.6b-v2" | "qwen3-asr-1.7b"
        | "whisper-tiny" | "whisper-base" | "whisper-base.en"
        | "whisper-small.en" | "whisper-medium" | "whisper-medium.en"
        | "whisper-large-v3" | "whisper-large-v3-turbo" | "whisper-large-v1"
        | "whisper-large-v2" | "lfm2.5-1.2b" | "lfm2.5-2.6b" | "qwen3-4b-instruct-2507" => {
            "Provisional estimate 2026-09-16 (weights pinned; overhead/context assumptions documented; not measured in SonoJot)"
        }
        _ => "Unrated",
    }
}

/// Planned resident range for a dictation setup (asr + text) under a mode:
/// overlap sums distinct residents + working overhead; sequential takes the
/// max + working overhead (confirmed unload, not A+C). Returns None when any
/// selected id is unknown.
pub fn plan_range(asr_id: &str, text_id: &str, mode: MemoryMode) -> Option<(u64, u64)> {
    let (a_min, a_max) = estimated_ram_interval(asr_id)?;
    let (t_min, t_max) = estimated_ram_interval(text_id)?;
    match mode {
        MemoryMode::Overlap => Some((
            a_min
                .saturating_add(t_min)
                .saturating_add(WORKING_OVERHEAD_BYTES),
            a_max
                .saturating_add(t_max)
                .saturating_add(WORKING_OVERHEAD_BYTES),
        )),
        MemoryMode::Sequential => Some((
            a_min.max(t_min).saturating_add(WORKING_OVERHEAD_BYTES),
            a_max.max(t_max).saturating_add(WORKING_OVERHEAD_BYTES),
        )),
    }
}

#[derive(Clone, Copy, Debug)]
struct Config {
    memory_mode: MemoryMode,
    retention: ModelRetention,
    no_history: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            memory_mode: MemoryMode::Overlap,
            retention: ModelRetention::Minutes10,
            no_history: false,
        }
    }
}

struct State {
    update_installing: bool,
    update_activities: [std::sync::Arc<std::sync::atomic::AtomicUsize>; 5],
    config: Config,
    asr_active: Option<String>,
    text_active: Option<String>,
    asr_loaded: Option<String>,
    text_loaded: Option<String>,
    asr_idle_since: Option<Instant>,
    text_idle_since: Option<Instant>,
    asr_release_pending: bool,
    /// Pending (reserved but not yet loaded) host/VRAM bytes tied to the
    /// active lease. Recorded at atomic acquire time so a concurrent load
    /// observes the reservation even though the OS free counter has not yet
    /// dropped. Cleared on `note_*_loaded` (now resident via `loaded`), on
    /// load failure/cancel/finish, and on removal. Already-resident reuses
    /// record zero (accounted once via `loaded`).
    asr_pending_host: u64,
    asr_pending_vram: u64,
    asr_pending_id: Option<String>,
    text_pending_host: u64,
    text_pending_vram: u64,
    text_pending_id: Option<String>,
    /// Auto processing may place the text worker on CPU when GPU speech and
    /// cleanup would exceed the device budget. Scoped to this text lease.
    text_budget_cpu: bool,
    /// Monotonic lease generation. Every acquire/touch bumps it. Idle timers
    /// capture it at decision time and only clear state when it is unchanged,
    /// so a job that started between the check and the shutdown is never
    /// evicted underneath itself.
    generation: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            update_installing: false,
            update_activities: std::array::from_fn(|_| std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))),
            config: Config::default(),
            asr_active: None,
            text_active: None,
            asr_loaded: None,
            text_loaded: None,
            asr_idle_since: None,
            text_idle_since: None,
            asr_release_pending: false,
            asr_pending_host: 0,
            asr_pending_vram: 0,
            asr_pending_id: None,
            text_pending_host: 0,
            text_pending_vram: 0,
            text_pending_id: None,
            text_budget_cpu: false,
            generation: 0,
        }
    }
}

/// Host/VRAM estimate pair (max bytes) for reservation accounting.
/// Unknown ids yield 0 (honest unknown proceeds; admission never invents a
/// denial for unrated ids). Rules-only yields 0.
fn estimate_pair_max(id: &str) -> (u64, u64) {
    let host = estimated_ram_interval(id).map(|(_, hi)| hi).unwrap_or(0);
    let text_cpu = crate::optional_llm::find(id).is_some()
        && crate::acceleration::cpu_only();
    let vram = if text_cpu {
        0
    } else {
        estimated_vram_interval(id).map(|(_, hi)| hi).unwrap_or(0)
    };
    (host, vram)
}

fn clear_asr_pending(state: &mut State) {
    state.asr_pending_host = 0;
    state.asr_pending_vram = 0;
    state.asr_pending_id = None;
}

fn clear_text_pending(state: &mut State) {
    state.text_pending_host = 0;
    state.text_pending_vram = 0;
    state.text_pending_id = None;
}

pub fn text_cpu_required(model_id: &str) -> bool {
    shared()
        .lock()
        .is_ok_and(|state| state.text_active.as_deref() == Some(model_id) && state.text_budget_cpu)
}

fn shared() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

fn lock_state() -> Result<std::sync::MutexGuard<'static, State>, String> {
    shared()
        .lock()
        .map_err(|_| "Resource state lock failed.".into())
}

/// Whole-operation ownership, including preparation, queues, remote work and
/// cancellation drain. The same coordinator serializes installer admission and
/// inference admission. No mutex is held while a worker runs.
#[derive(Clone, Copy, Debug)]
pub(crate) enum UpdateActivity { Dictation, Meeting, Ai, Models, Media }

#[derive(Debug)]
pub(crate) struct UpdateActivityLease(std::sync::Arc<std::sync::atomic::AtomicUsize>);

pub(crate) fn begin_update_activity(kind: UpdateActivity) -> Result<UpdateActivityLease, String> {
    let state = lock_state()?;
    if state.update_installing { return Err(UPDATE_INSTALLING.into()); }
    let count = state.update_activities[kind as usize].clone();
    count.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    Ok(UpdateActivityLease(count))
}

impl Drop for UpdateActivityLease {
    fn drop(&mut self) {
        // Completion can run under capture/snapshot/store locks. It must not
        // take the coordinator (selection commits use the opposite order).
        // Starts still increment under the coordinator, so installation can
        // never admit while a job begins. A concurrent drop only makes it idle.
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

const UPDATE_INSTALLING: &str = "SonoJot is restarting to install an update.";

fn update_blocked(state: &State) -> Option<&'static str> {
    if state.update_installing { return Some(UPDATE_INSTALLING); }
    let reasons = [
        "Wait for dictation to finish.",
        "Wait for meeting recording or processing to finish.",
        "Wait for AI work to finish.",
        "Wait for model installation or removal to finish.",
        "Wait for media support or import work to finish.",
    ];
    for (count, reason) in state.update_activities.iter().zip(reasons) {
        if count.load(std::sync::atomic::Ordering::Acquire) > 0 { return Some(reason); }
    }
    if state.asr_active.is_some() || state.text_active.is_some() {
        return Some("Wait for local model work to finish.");
    }
    None
}

pub(crate) fn update_blocked_reason() -> Option<String> {
    // A snapshot must never wait behind admission or model-selection work.
    // Uncertainty disables the UI action; installation still checks atomically.
    match shared().try_lock() {
        Ok(state) => update_blocked(&state).map(str::to_owned),
        Err(_) => Some("SonoJot cannot confirm that background work has finished.".into()),
    }
}

/// Held through installer handoff/restart. Dropping after an error reopens work.
pub(crate) struct UpdateInstallLease;

pub(crate) fn begin_update_install() -> Result<UpdateInstallLease, String> {
    let mut state = lock_state()?;
    if let Some(reason) = update_blocked(&state) { return Err(reason.into()); }
    state.update_installing = true;
    Ok(UpdateInstallLease)
}

impl Drop for UpdateInstallLease {
    fn drop(&mut self) {
        if let Ok(mut state) = lock_state() { state.update_installing = false; }
    }
}

/// Non-blocking read for UI polling. Never waits on inference.
fn try_state<F, T>(f: F) -> Option<T>
where
    F: FnOnce(&State) -> T,
{
    shared().try_lock().ok().map(|s| f(&s))
}

/// Apply Settings lifecycle choices. Existing installs keep selections;
/// this only changes overlap/retention/no-history going forward.
/// Also ensures the singleton idle worker runs (startup/configure path; the
/// speech 500 ms poll only confirms ASR drops, it never drives text idle).
pub fn configure(settings: &Settings) {
    crate::recognition_diagnostics::set_privacy(settings.do_not_save_dictations);
    if let Ok(mut state) = shared().lock() {
        state.config.memory_mode = settings.memory_mode;
        state.config.retention = settings.model_retention;
        store_retention(settings.model_retention);
        state.config.no_history = settings.do_not_save_dictations;
        // A mode switch never reuses an incompatible worker mid-job.
        // Active jobs keep their leases; the next acquire sees the new mode.
    }
    ensure_idle_worker();
}

/// Singleton idle worker: bounded 15 s poll calling `release_idle()`.
/// Retention-gated, generation-guarded, confirmed-only inside. Spawned once
/// from `configure()` (startup + settings saves + dictation starts), so no
/// `lib.rs` edit is needed. No large probes here; only retention timestamps
/// and a confirmed child shutdown. `#[cfg(test)]` never spawns (tests drive
/// `release_idle()` explicitly with barriers).
#[cfg(not(test))]
fn ensure_idle_worker() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("resource-idle".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(15));
                let _ = release_idle();
            })
            .ok();
    });
}

#[cfg(test)]
fn ensure_idle_worker() {}

/// Test-only delay (ms) held INSIDE the idle critical section to prove
/// serialization with a barrier. Production always 0.
#[cfg(test)]
static IDLE_TEST_DELAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
static IDLE_IN_CRITICAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub fn set_idle_test_delay_ms(ms: u64) {
    IDLE_TEST_DELAY_MS.store(ms, std::sync::atomic::Ordering::Release);
    IDLE_IN_CRITICAL.store(false, std::sync::atomic::Ordering::Release);
}

/// Whether no-history is on. Fail-closed (true on contention) so a privacy
/// decision never defaults to retaining when the coordinator is contended.
pub fn no_history_enabled() -> bool {
    try_state(|s| s.config.no_history).unwrap_or(true)
}

pub fn no_history_blocking() -> bool {
    shared()
        .lock()
        .ok()
        .map(|s| s.config.no_history)
        .unwrap_or(true)
}

pub fn current_memory_mode() -> MemoryMode {
    try_state(|s| s.config.memory_mode).unwrap_or(MemoryMode::Overlap)
}

// Updated inside the configuration lock, but readable while idle/lease work
// holds that lock. Contention must never turn Immediate into ten minutes.
static RETENTION: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
fn store_retention(retention: ModelRetention) {
    RETENTION.store(match retention {
        ModelRetention::Immediately => 0,
        ModelRetention::Minutes10 => 1,
        ModelRetention::Minutes30 => 2,
        ModelRetention::OnExit => 3,
    }, std::sync::atomic::Ordering::Release);
}
pub fn current_retention() -> ModelRetention {
    match RETENTION.load(std::sync::atomic::Ordering::Acquire) {
        0 => ModelRetention::Immediately,
        1 => ModelRetention::Minutes10,
        2 => ModelRetention::Minutes30,
        _ => ModelRetention::OnExit,
    }
}

pub fn is_sequential() -> bool {
    matches!(current_memory_mode(), MemoryMode::Sequential)
}

/// True while any inference holds a lease. UI polling uses this; it never
/// blocks on the cleanup server mutex.
pub fn is_busy() -> bool {
    try_state(|s| s.asr_active.is_some() || s.text_active.is_some()).unwrap_or(false)
}

pub fn active_stages() -> Vec<String> {
    try_state(|s| {
        let mut out = Vec::new();
        if s.asr_active.is_some() {
            out.push("asr".to_owned());
        }
        if s.text_active.is_some() {
            out.push("cleanup/notes".to_owned());
        }
        out
    })
    .unwrap_or_default()
}

pub fn loaded_models() -> Vec<String> {
    try_state(|s| {
        let mut out = Vec::new();
        if let Some(m) = s.asr_loaded.as_ref() {
            out.push(m.clone());
        }
        if let Some(m) = s.text_loaded.as_ref() {
            out.push(m.clone());
        }
        out
    })
    .unwrap_or_default()
}

pub fn is_model_active(id: &str) -> bool {
    // Polling-only observation. try_lock failure defaults false (fail-open for
    // scheduling hints). Safety decisions must use `model_state_blocking`.
    try_state(|s| s.asr_active.as_deref() == Some(id) || s.text_active.as_deref() == Some(id))
        .unwrap_or(false)
}

pub fn is_model_loaded(id: &str) -> bool {
    // Polling-only observation, same fail-open caveat as `is_model_active`.
    try_state(|s| s.asr_loaded.as_deref() == Some(id) || s.text_loaded.as_deref() == Some(id))
        .unwrap_or(false)
}

/// Blocking ownership check for destructive/safety decisions (removal,
/// admission, eviction). Holds the coordinator lock; never defaults on
/// contention. Returns `(active, loaded)` for the id.
pub fn model_state_blocking(id: &str) -> (bool, bool) {
    shared()
        .lock()
        .ok()
        .map(|s| {
            let active =
                s.asr_active.as_deref() == Some(id) || s.text_active.as_deref() == Some(id);
            let loaded =
                s.asr_loaded.as_deref() == Some(id) || s.text_loaded.as_deref() == Some(id);
            (active, loaded)
        })
        .unwrap_or((true, true))
}

/// Blocking busy check for safety paths (install/remove/admission). True on
/// lock failure (fail-closed) so contention never authorizes destruction.
pub fn busy_blocking() -> bool {
    shared()
        .lock()
        .ok()
        .map(|s| s.asr_active.is_some() || s.text_active.is_some())
        .unwrap_or(true)
}

/// Current lease generation (blocking). Idle timers capture it and only
/// complete a release when it is unchanged.
pub fn generation_blocking() -> u64 {
    shared().lock().ok().map(|s| s.generation).unwrap_or(0)
}

/// Acquire the ASR stage for one dictation job. The single capture owner
/// guarantees at most one ASR job; a second acquire is a logic error.
///
/// A lease is NOT proof the weights loaded: this sets `active` + a pending
/// reservation only. The caller must `note_asr_loaded(id)` after
/// `Recognizer::load` succeeds (reservation converts to resident) and must
/// release on failure via Drop/`note_asr_load_failed` (reservation cleared).
/// This keeps a failed load from stranding a stale resident flag AND lets a
/// concurrent text admission observe the pending bytes. Legacy lease-only
/// entry kept for the non-owned `meetings.rs` caller; owned load paths must
/// use `acquire_asr_with_admission` (atomic reservation + fit).
pub fn acquire_asr(model_id: &str) -> Result<(), String> {
    let mut state = lock_state()?;
    if state.update_installing { return Err(UPDATE_INSTALLING.into()); }
    if state.asr_active.is_some() {
        return Err("Speech recognition is already running.".into());
    }
    // Sequential mode forbids starting ASR while a text job is active, and
    // vice versa. Overlap permits both when admission allows.
    if is_sequential_locked(&state) && state.text_active.is_some() {
        return Err(
            "A text job is using local inference. Wait for it to finish before dictating in one-stage mode."
                .into(),
        );
    }
    state.asr_active = Some(model_id.to_owned());
    // Record a pending reservation so concurrent admission observes it even
    // before the OS free counter drops. Zero when reusing the resident.
    let already_resident = state.asr_loaded.as_deref() == Some(model_id);
    if already_resident {
        clear_asr_pending(&mut state);
    } else {
        let (host, vram) = estimate_pair_max(model_id);
        state.asr_pending_host = host;
        state.asr_pending_vram = vram;
        state.asr_pending_id = Some(model_id.to_owned());
    }
    // `loaded` untouched: previous resident stays until the caller drops it
    // and confirms, or overwrites it via `note_asr_loaded` on success.
    state.asr_idle_since = None;
    state.asr_release_pending = false;
    state.generation = state.generation.wrapping_add(1);
    Ok(())
}

/// Mark ASR readiness on actual successful native load. Call while still
/// holding the lease (before `notify_asr_finished`), after `Recognizer::load`
/// returns Ok. Converts the pending reservation into a resident: pending is
/// cleared because `loaded` now accounts the bytes once. Failure paths must
/// NOT call this; they leave `loaded` as-is (None after a switch-drop, or the
/// previous resident when reusing) and clear pending via
/// `note_asr_load_failed`/Drop.
pub fn note_asr_loaded(model_id: &str) {
    if let Ok(mut state) = shared().lock() {
        // Only the active job may mark its own readiness.
        if state.asr_active.as_deref() == Some(model_id) {
            state.asr_loaded = Some(model_id.to_owned());
            state.asr_idle_since = None;
            state.asr_release_pending = false;
            clear_asr_pending(&mut state);
            // Pending converted to resident: the OS free counter dropped by
            // the same bytes, so in-flight admission probes sampled against
            // the old pending must re-probe (see the generation check in the
            // atomic acquire paths).
            state.generation = state.generation.wrapping_add(1);
        }
    }
}

/// Failure contract for ASR loads: clears the pending reservation so a
/// concurrent admission no longer accounts bytes that will never load. Switch
/// paths drop the stale recognizer + `confirm_asr_released()` BEFORE
/// attempting the new load, so a failed load leaves `loaded=None` without
/// inventing a resident. Call on every ASR load error/cancel before Drop.
pub fn note_asr_load_failed() {
    if let Ok(mut state) = shared().lock() {
        clear_asr_pending(&mut state);
        // A reservation others may have observed is gone; move the generation
        // so stale admission probes re-sample instead of trusting it.
        state.generation = state.generation.wrapping_add(1);
    }
}

/// Mark ASR inference done but keep the loaded flag until the caller actually
/// drops the recognizer. Clears any unconverted pending reservation (cancel
/// before load) so it never leaks into the next admission. Sequential callers
/// must drop first, then call `confirm_asr_released`.
pub fn notify_asr_finished() {
    if let Ok(mut state) = shared().lock() {
        state.asr_active = None;
        clear_asr_pending(&mut state);
        state.asr_idle_since = Some(Instant::now());
        state.generation = state.generation.wrapping_add(1);
    }
}

/// Request the speech worker drop its cached native recognizer (idle
/// resident) so a meeting file job can load without duplicating same-GPU
/// weights or colliding same-basename CPU/GPU DLLs in one process.
///
/// Sets the pending flag only; the physical drop stays owned by the speech
/// worker's poll, which confirms via `confirm_asr_released`. Returns true
/// when a release is still owed (caller must await `asr_resident_blocking()
/// == false`), false when nothing is resident. Never fakes `loaded=None`:
/// only the owning worker clears it after the physical drop.
pub fn request_asr_release() -> bool {
    if let Ok(mut state) = shared().lock() {
        if state.asr_loaded.is_some() {
            state.asr_release_pending = true;
            return true;
        }
        return false;
    }
    // Fail-closed on contention: the waiter treats this as resident and its
    // timeout (not a concurrent load) resolves the ambiguity.
    true
}

/// Blocking check: true while a native ASR recognizer is still resident.
/// Fail-closed on contention, so waiters never load under a live worker.
/// A file job's own atomic lease does not set `loaded` (its Engine is
/// ephemeral), so this observes only the cached speech worker.
pub fn asr_resident_blocking() -> bool {
    shared()
        .lock()
        .ok()
        .map(|s| s.asr_loaded.is_some())
        .unwrap_or(true)
}

/// Await the speech worker's confirmed cached-recognizer drop after
/// `request_asr_release`. Returns Ok once nothing is resident. A set
/// `cancel` aborts the wait promptly (lease cleanup stays with the caller);
/// `timeout` errors rather than authorizing a concurrent load, so same-GPU
/// weights can never duplicate and same-basename DLLs never collide.
/// Polls every 20 ms; blocking.
pub fn await_asr_release(
    cancel: &std::sync::atomic::AtomicBool,
    timeout: Duration,
) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Meeting operation cancelled.".into());
        }
        if !asr_resident_blocking() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(
                "The speech model is still loaded. Wait for idle release, then try again.".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Confirm the owning worker physically dropped its cached native
/// recognizer, so admission sees reclaimed memory.
///
/// Pending accounting is tied to active replacement ownership: when an ASR
/// lease is active (a dictation switch or a meeting file job awaiting its
/// reload), the coming load keeps — or, for a same-ID cached reuse that
/// reserved zero, is newly armed — its pending reservation across the
/// confirmation. Clearing it here would open a drop-to-reload gap where an
/// overlap text admission could take the just-freed bytes before the
/// replacement engine loads (generation bumps cannot fix an absent
/// reservation). With no active lease (idle eviction, model-root refresh),
/// pending clears so nothing strands. Re-arming an already-reserved lease
/// rewrites the same values (idempotent, never double-counted); the
/// reservation converts via `note_asr_loaded` or clears via
/// `note_asr_load_failed` / `notify_asr_finished` / Drop.
pub fn confirm_asr_released() {
    if let Ok(mut state) = shared().lock() {
        state.asr_loaded = None;
        state.asr_release_pending = false;
        if let Some(active) = state.asr_active.clone() {
            let (host, vram) = estimate_pair_max(&active);
            state.asr_pending_host = host;
            state.asr_pending_vram = vram;
            state.asr_pending_id = Some(active);
        } else {
            clear_asr_pending(&mut state);
        }
        if state.asr_active.is_none() && state.asr_idle_since.is_none() {
            state.asr_idle_since = Some(Instant::now());
        }
        // Residency changed (bytes returned to the OS, reservation
        // armed/preserved); stale probes must re-sample.
        state.generation = state.generation.wrapping_add(1);
    }
}
/// Whether speech.rs should drop its recognizer before cleanup/prewarm.
/// Blocking ownership check (fail-closed): true on contention so sequential
/// callers drop before loading text rather than racing. Polling hint
/// `asr_release_due_for_cleanup_try` stays for UI only.
pub fn asr_release_due_for_cleanup() -> bool {
    shared()
        .lock()
        .ok()
        .map(|s| matches!(s.config.memory_mode, MemoryMode::Sequential) && s.asr_loaded.is_some())
        .unwrap_or(true)
}

/// Polling-only hint for UI. Never drives prewarm/destruction.
pub fn asr_release_due_for_cleanup_try() -> bool {
    try_state(|s| matches!(s.config.memory_mode, MemoryMode::Sequential) && s.asr_loaded.is_some())
        .unwrap_or(false)
}

/// Cleanup/text prewarm gate for actual load paths. Blocking, fail-closed:
/// false on contention (never prewarm when ownership is unclear). Try-lock
/// `should_prewarm_cleanup` below is polling-only and must never authorize
/// prewarm or destruction.
pub fn should_prewarm_cleanup_blocking() -> bool {
    match shared().lock() {
        Ok(s) => {
            !(matches!(s.config.memory_mode, MemoryMode::Sequential)
                && (s.asr_active.is_some() || s.asr_loaded.is_some()))
        }
        Err(_) => false,
    }
}

/// Cleanup/text prewarm is forbidden while ASR holds the exclusive lease in
/// sequential mode. POLLING HINT ONLY (fail-open try-lock): safety decisions
/// must use `should_prewarm_cleanup_blocking`.
pub fn should_prewarm_cleanup() -> bool {
    try_state(|s| {
        !(matches!(s.config.memory_mode, MemoryMode::Sequential)
            && (s.asr_active.is_some() || s.asr_loaded.is_some()))
    })
    .unwrap_or(true)
}

fn is_sequential_locked(state: &State) -> bool {
    matches!(state.config.memory_mode, MemoryMode::Sequential)
}

/// Acquire the shared text worker for cleanup or notes. `role` is
/// "cleanup" or "notes" and only names the waiter for error text.
///
/// A lease is NOT proof the weights loaded: this sets `active` + pending
/// reservation only. The caller must `note_text_loaded(id)` after the first
/// successful `generate_local`/`refine` under this lease (pending converts to
/// resident) and must release on failure via Drop/`note_text_load_failed`.
/// Shared text weights are not duplicated: one artifact serving cleanup+notes
/// keeps one `text_loaded` entry with independent per-request contexts
/// (cleanup owner erases KV per request); admission accounts the resident
/// once. Legacy lease-only entry; owned load paths must use
/// `acquire_text_with_admission` / `acquire_text_waiting_with_admission`.
pub fn acquire_text(role: &str, model_id: &str) -> Result<(), String> {
    let mut state = lock_state()?;
    if state.update_installing { return Err(UPDATE_INSTALLING.into()); }
    if state.text_active.is_some() {
        return Err(format!(
            "Local text inference is busy. Wait for the current {role} job or stop it first."
        ));
    }
    if is_sequential_locked(&state) && (state.asr_active.is_some() || state.asr_loaded.is_some()) {
        return Err(
            "Speech recognition still holds local inference in one-stage mode. Release it before loading cleanup."
                .into(),
        );
    }
    // A role/model switch never reuses an incompatible worker. If a different
    // text model is resident, the caller loads the new one after this
    // acquire; `loaded` is overwritten only via `note_text_loaded` on actual
    // success, never here.
    state.text_active = Some(model_id.to_owned());
    let already_resident = state.text_loaded.as_deref() == Some(model_id);
    if already_resident {
        clear_text_pending(&mut state);
    } else {
        let (host, vram) = estimate_pair_max(model_id);
        state.text_pending_host = host;
        state.text_pending_vram = vram;
        state.text_pending_id = Some(model_id.to_owned());
    }
    state.text_idle_since = None;
    state.generation = state.generation.wrapping_add(1);
    let _ = role;
    Ok(())
}

/// Mark text readiness on actual successful load/inference under the held
/// lease. Converts the pending reservation into a resident (pending cleared,
/// `loaded` now accounts once). Call after the first successful
/// `generate_local`/`refine` with this `model_id`. Inference errors that are
/// not load failures must still call this when the server is resident (the
/// worker is warm); load failures that dropped the server must call
/// `note_text_load_failed` instead.
pub fn note_text_loaded(model_id: &str) {
    if let Ok(mut state) = shared().lock() {
        if state.text_active.as_deref() == Some(model_id) {
            state.text_loaded = Some(model_id.to_owned());
            state.text_idle_since = None;
            clear_text_pending(&mut state);
            // Pending converted to resident (see the ASR-side note): move the
            // generation so stale admission probes re-sample.
            state.generation = state.generation.wrapping_add(1);
        }
    }
}

/// Clear text resident state after a confirmed load failure that dropped the
/// server (model switch where the old worker was replaced and the new launch
/// failed). Also clears the pending reservation so concurrent admission stops
/// accounting bytes that will never load. Never touches the active lease;
/// Drop clears that. Callers that failed WITHOUT replacing (e.g. busy) must
/// not call this.
pub fn note_text_load_failed() {
    if let Ok(mut state) = shared().lock() {
        // Conservative: the server slot was replaced-then-failed, so any
        // previous resident is gone. Clear to avoid pointing at a dead child.
        // Callers that failed WITHOUT replacing (e.g. busy) must not call this.
        state.text_loaded = None;
        state.text_idle_since = None;
        clear_text_pending(&mut state);
        state.generation = state.generation.wrapping_add(1);
    }
}

pub fn notify_text_finished() {
    if let Ok(mut state) = shared().lock() {
        state.text_active = None;
        clear_text_pending(&mut state);
        state.text_idle_since = Some(Instant::now());
        state.generation = state.generation.wrapping_add(1);
    }
}

/// Clear resident flags for a model whose files were just deleted.
/// Called by model_manager after a confirmed removal so a stale loaded flag
/// never points at a missing directory. Never touches active leases (the
/// caller blocks those first). Also clears matching pending reservations.
pub fn note_model_removed(id: &str) {
    if let Ok(mut state) = shared().lock() {
        if state.asr_loaded.as_deref() == Some(id) {
            state.asr_loaded = None;
            state.asr_release_pending = false;
            state.asr_idle_since = None;
        }
        if state.asr_pending_id.as_deref() == Some(id) {
            clear_asr_pending(&mut state);
        }
        if state.text_loaded.as_deref() == Some(id) {
            state.text_loaded = None;
            state.text_idle_since = None;
        }
        if state.text_pending_id.as_deref() == Some(id) {
            clear_text_pending(&mut state);
        }
        state.generation = state.generation.wrapping_add(1);
    }
}

/// A new request cancels pending eviction safely and bumps the generation
/// so in-flight idle timers observe the new activity and abort.
pub fn touch_activity(stage: Stage) {
    if let Ok(mut state) = shared().lock() {
        state.generation = state.generation.wrapping_add(1);
        match stage {
            Stage::Asr => {
                state.asr_idle_since = None;
                state.asr_release_pending = false;
            }
            Stage::Cleanup | Stage::Notes => {
                state.text_idle_since = None;
            }
        }
    }
}

fn retention_elapsed(since: Option<Instant>, retention: ModelRetention) -> bool {
    match retention {
        ModelRetention::Immediately => true,
        ModelRetention::Minutes10 => {
            since.is_some_and(|t| t.elapsed() >= Duration::from_secs(10 * 60))
        }
        ModelRetention::Minutes30 => {
            since.is_some_and(|t| t.elapsed() >= Duration::from_secs(30 * 60))
        }
        ModelRetention::OnExit => false,
    }
}

/// Release idle workers per retention with lease ownership held across the
/// physical shutdown. The coordinator mutex is held for the whole
/// decision→shutdown→flag-clear transition, so a new `acquire_text`/
/// `acquire_asr` (same mutex) cannot start between the idle decision and the
/// child kill. Checking generation AFTER shutdown alone would only protect
/// flags, not the new job's process — this serializes the process itself.
///
/// - Text: confirmed via `cleanup_resource::release_idle()` (true only when a
///   live child was actually stopped) while holding the lease mutex. A new job
///   blocks on the mutex until the shutdown finishes, then loads fresh; it is
///   never killed underneath itself.
/// - ASR: native memory is owned by speech.rs. This sets a pending request
///   but returns 0 for ASR until speech actually drops the recognizer and
///   calls `confirm_asr_released`. Pending is never counted as freed bytes.
/// Returns confirmed releases of the text worker and punctuation helper;
/// ASR confirms separately.
pub fn release_idle() -> usize {
    // The helper serializes inference with its own cache lock. Run outside the
    // lease mutex because inference reads the current retention setting.
    let helpers = usize::from(crate::punctuation::release_idle(current_retention()));
    let mut state = match shared().lock() {
        Ok(s) => s,
        Err(_) => return helpers,
    };
    if state.asr_active.is_some() || state.text_active.is_some() {
        return helpers;
    }
    let retention = state.config.retention;
    let release_text =
        state.text_loaded.is_some() && retention_elapsed(state.text_idle_since, retention);
    let release_asr = state.asr_loaded.is_some()
        && !state.asr_release_pending
        && retention_elapsed(state.asr_idle_since, retention);
    if release_asr {
        // Request only. Speech's idle poll confirms by dropping the
        // recognizer; until then no bytes are accounted as freed.
        state.asr_release_pending = true;
    }
    if !release_text {
        return helpers;
    }
    #[cfg(test)]
    {
        let delay = IDLE_TEST_DELAY_MS.load(std::sync::atomic::Ordering::Acquire);
        if delay > 0 {
            // Hold the lease mutex across the delay so barrier tests prove a
            // concurrent acquire blocks instead of interleaving.
            IDLE_IN_CRITICAL.store(true, std::sync::atomic::Ordering::Release);
            std::thread::sleep(Duration::from_millis(delay));
            IDLE_IN_CRITICAL.store(false, std::sync::atomic::Ordering::Release);
            // Re-check under the same guard: a waiter cannot have acquired
            // (it blocks), but active could only change if this guard were
            // dropped, which it was not. Fail-closed anyway.
            if state.asr_active.is_some() || state.text_active.is_some() {
                return helpers;
            }
        }
    }
    // Physical shutdown WHILE HOLDING the lease mutex. New acquires block on
    // this same mutex, so they cannot launch a server that we then kill.
    // `cleanup_resource::release_idle` try_locks the SERVER mutex (different
    // mutex, resource->server order); an active job holding SERVER makes this
    // return false without evicting. No callback into resource_manager occurs.
    let stopped = crate::cleanup_resource::release_idle();
    if !stopped {
        // Nothing confirmed freed. A stale loaded flag with no live child
        // clears without counting as a byte release. Still holding the guard,
        // so no concurrent acquire could have slipped in.
        if state.text_active.is_none()
            && retention_elapsed(state.text_idle_since, state.config.retention)
        {
            state.text_loaded = None;
        }
        return helpers;
    }
    // Shutdown happened while new jobs were blocked. They will acquire after
    // we release and load fresh. Clear the old resident now.
    if state.text_active.is_some() {
        // Impossible while holding the guard; fail-closed.
        return helpers;
    }
    state.text_loaded = None;
    helpers + 1
}

/// Whether speech.rs still owes an ASR drop after `release_idle`.
/// Polling hint only; admission/accounting never treats it as freed.
pub fn asr_release_pending() -> bool {
    try_state(|s| s.asr_release_pending).unwrap_or(false)
}

/// Blocking pending check for safety paths (fail-closed on contention).
pub fn asr_release_pending_blocking() -> bool {
    shared()
        .lock()
        .ok()
        .map(|s| s.asr_release_pending)
        .unwrap_or(true)
}

/// Memory-pressure path: evict idle workers immediately, never active.
/// Serialized like `release_idle` (lease mutex held across shutdown). ASR
/// pending is requested but never counted until `confirm_asr_released`.
/// Returns number of workers with confirmed release.
pub fn evict_idle_on_pressure() -> usize {
    let helpers = usize::from(crate::punctuation::release_cached());
    let mut state = match shared().lock() {
        Ok(s) => s,
        Err(_) => return helpers,
    };
    if state.asr_active.is_some() || state.text_active.is_some() {
        return helpers;
    }
    let text = state.text_loaded.is_some();
    let asr = state.asr_loaded.is_some() && !state.asr_release_pending;
    if asr {
        state.asr_release_pending = true;
    }
    if !text {
        return helpers;
    }
    let stopped = crate::cleanup_resource::release_idle();
    if !stopped {
        return helpers;
    }
    if state.text_active.is_some() {
        return helpers;
    }
    state.text_loaded = None;
    helpers + 1
}

/// Admission for a new load: compare *additional* allocation plus reserve
/// against current availability. Currently resident SonoJot allocations are
/// accounted once: when the requested model is already resident, additional
/// is zero. Otherwise the caller passes the model's estimated max. Idle
/// workers are evicted via confirmed release first, then availability is
/// re-read before the final decision. Never claims pending ASR memory as
/// freed. A low-memory estimate is advisory after one idle-eviction attempt.
pub fn admission_check(additional_bytes: u64, what: &str) -> Result<(), String> {
    admission_check_inner(None, additional_bytes, what)
}

/// Resident-aware admission for a named model: when `model_id` is already
/// resident, additional is zero (accounted once); otherwise uses
/// `estimate_max_bytes`. Confirmed-evicts idle, then admits with a warning.
pub fn admission_check_model(
    model_id: &str,
    estimate_max_bytes: u64,
    what: &str,
) -> Result<(), String> {
    admission_check_inner(Some(model_id), estimate_max_bytes, what)
}

/// Source-backed admission for a named model id. Uses the row's
/// `ramMaxBytes` (host) plus VRAM fit on the SELECTED single device
/// (normally 0, matched total/free pair) when the id is a CUDA model.
/// Resident-once: an already-resident id needs zero additional host bytes.
/// Pending reservations from concurrent atomic acquires are subtracted, so
/// concurrent requests remain visible to the estimate. Unknown availability,
/// unknown estimates and insufficient estimates all proceed; a known shortfall
/// evicts confirmed-idle work once and writes an advisory warning. LEGACY
/// helper: owned load paths must use the atomic
/// `acquire_*_with_admission` below (admission tied to lease ownership), not
/// this check-then-acquire pair.
pub fn admission_for_model(model_id: &str, what: &str) -> Result<(), String> {
    // Host RAM gate from the source-backed interval.
    let ram_max = estimated_ram_interval(model_id)
        .map(|(_, hi)| hi)
        .unwrap_or(0);
    // Unknown ids have no interval (ram_max 0 here would look like rules);
    // distinguish: None estimate with non-rules id proceeds as unknown, not
    // as zero. The RAM check below treats 0-additional as resident/rules.
    let known = estimated_ram_interval(model_id).is_some();
    if known {
        admission_check_model(model_id, ram_max, what)?;
    } else if !matches!(model_id, "local-rules" | "off" | "") {
        // Truly unknown model: no honest bytes to check; proceed (callers
        // already validate installability) rather than inventing a denial.
    }
    // Device VRAM advisory for CUDA ids only. Uses the SELECTED single-device
    // free (normally device 0, matched pair, never a max/sum). Unknown free
    // stays unknown (proceed); proven pressure evicts idle work once, warns and
    // still proceeds.
    // Pending VRAM from the other stage is subtracted.
    if let Some((_, vram_max)) = estimated_vram_interval(model_id) {
        if vram_max > 0 {
            if let Some((total, free)) = selected_gpu_pair() {
                // Reserve 10% of the selected device for the OS/driver; honest
                // floor so a 12 GiB card does not promise 12 GiB to the model.
                let pending_other = shared()
                    .lock()
                    .ok()
                    .map(|s| {
                        // The requester's own pending is not yet recorded for this
                        // legacy path; count both stages' pending conservatively.
                        // Over-counting here only makes the legacy path stricter,
                        // never looser; the atomic path below is exact.
                        s.asr_pending_vram.saturating_add(s.text_pending_vram)
                    })
                    .unwrap_or(0);
                let usable = free.saturating_sub(pending_other);
                // Resident-once for VRAM: CPU rows are 0; CUDA reuse needs 0.
                let already_resident = model_state_blocking(model_id).1;
                let need = if already_resident { 0 } else { vram_max };
                if usable < need {
                    let warning = format!(
                        "Selected CUDA device {} reports {} free ({} total) for {what}; the estimate is about {} on one device.",
                        SELECTED_CUDA_DEVICE_INDEX,
                        format_bytes(Some(free)),
                        format_bytes(Some(total)),
                        format_bytes(Some(need)),
                    );
                    evict_idle_on_pressure();
                    std::thread::sleep(Duration::from_millis(50));
                    admission_warning(&warning);
                }
            }
        }
    }
    Ok(())
}

/// Non-owning fit warning for installs/Setup. Returns `Some(warning)` when
/// the SELECTED device provably cannot fit `model_id`, else None. Never
/// rejects a download; downloads do not load weights. Loads still enforce
/// admission atomically.
pub fn vram_fit_warning(model_id: &str, what: &str) -> Option<String> {
    let (_, vram_max) = estimated_vram_interval(model_id)?;
    if vram_max == 0 {
        return None;
    }
    let (total, free) = selected_gpu_pair()?;
    let usable = free;
    (usable < vram_max).then(|| {
        format!(
            "Heads-up for {what}: the selected CUDA device (index {}) reports {} free ({} total); this model wants about {} on one device. The download can proceed, but loading may need freed VRAM or a CPU model.",
            SELECTED_CUDA_DEVICE_INDEX,
            format_bytes(Some(free)),
            format_bytes(Some(total)),
            format_bytes(Some(vram_max)),
        )
    })
}

fn admission_check_inner(
    model_id: Option<&str>,
    estimate_max_bytes: u64,
    what: &str,
) -> Result<(), String> {
    let already_resident = match model_id {
        Some(id) => model_state_blocking(id).1,
        None => false,
    };
    let additional = if already_resident {
        0
    } else {
        estimate_max_bytes
    };
    // Account pending reservations from concurrent atomic acquires: they have
    // not yet reduced the OS counter but will. Without this, two loads can
    // both observe enough free RAM then load together.
    let pending_other = shared()
        .lock()
        .ok()
        .map(|s| {
            match model_id {
                // Legacy helper cannot know which stage the caller is; count both
                // conservatively (stricter, never looser).
                Some(_) => s.asr_pending_host.saturating_add(s.text_pending_host),
                None => s.asr_pending_host.saturating_add(s.text_pending_host),
            }
        })
        .unwrap_or(0);
    let hardware = probe_hardware();
    let Some(available) = hardware.available_ram_bytes else {
        // Unknown availability is not a denial; proceed with a warning path.
        return Ok(());
    };
    let reserve = reserve_bytes(hardware.total_ram_bytes);
    let need = additional.saturating_add(WORKING_OVERHEAD_BYTES);
    let effective = available
        .saturating_sub(reserve)
        .saturating_sub(pending_other);
    if effective >= need {
        return Ok(());
    }
    let warning = format!(
        "Available RAM is below the estimate for {what}: about {} plus working overhead, with {} reserved.",
        format_bytes(Some(additional)),
        format_bytes(Some(reserve)),
    );
    // Reusing a resident model frees nothing by evicting; it could only evict
    // the model about to be reused. Warn and admit.
    if already_resident {
        admission_warning(&warning);
        return Ok(());
    }
    // Evict confirmed-idle safely, then admit with an advisory warning.
    evict_idle_on_pressure();
    // Give the OS a moment to reclaim after a confirmed shutdown; do not spin.
    std::thread::sleep(Duration::from_millis(50));
    admission_warning(&warning);
    Ok(())
}

fn admission_warning(reason: &str) {
    eprintln!("Resource admission advisory: {reason} Loading will still be attempted.");
}

// ---------------------------------------------------------------------------
// Atomic reservation/admission tied to lease ownership (Bug 1 fix).
//
// The bug: admission ran BEFORE acquiring the lease, using only the OS free
// counter. Two loads could both observe enough free RAM/VRAM, both pass, then
// both acquire and load together (joint OOM). The fix: probe outside the
// lock (slow), then check fit + mark active + record pending IN ONE
// coordinator critical section. Pending bytes make the second contender's
// advisory estimate include the first reservation. Reservations clear on
// `note_*_loaded` (resident), failure, cancel, finish, or removal.
// ---------------------------------------------------------------------------

/// Fit check + active marking + pending reservation for ASR in ONE critical
/// section. Probes happen before the lock; the decision and the mark share
/// the lock, so two contenders cannot miss each other's reservations. Nothing
/// is marked for busy/stale outcomes. After one idle-release attempt, ordinary
/// physical-RAM estimates remain advisory, but GPU shortfalls and critical RAM
/// exhaustion reject the load. Auto text can explicitly fall back to CPU.
///
/// Probe/sample consistency: the hardware/VRAM sample is taken outside the
/// lock (probing may spawn nvidia-smi, up to ~2 s). Every accounting
/// transition (acquire, pending↔resident conversion, finish, release,
/// removal) bumps `State::generation`, so if another load's pending
/// reservation converts to resident between our sample and our lock — the
/// pending clears while the OS free counter drops by the same bytes — the
/// generation check below fails and we re-sample instead of admitting
/// against a stale free count plus a cleared reservation (joint OOM).
pub fn acquire_asr_with_admission(model_id: &str, what: &str) -> Result<(), String> {
    // Bounded probe/lock loop: a stale sample re-probes (allocation moved
    // under us); estimated pressure evicts confirmed-idle once, then admits;
    // busy fails fast. The bound keeps constant churn from livelocking.
    let mut evicted = false;
    let mut pressure_warning = None;
    for _ in 0..4 {
        let probe_gen = generation_blocking();
        let hw = probe_hardware();
        let gpu = selected_gpu_pair();
        match try_acquire_asr_locked(model_id, what, &hw, gpu, probe_gen, evicted) {
            LockedAdmission::Admitted(advisory) => {
                if let Some(warning) = advisory.as_deref().or(pressure_warning.as_deref()) {
                    admission_warning(warning);
                }
                return Ok(());
            }
            LockedAdmission::Busy(error) | LockedAdmission::Denied(error) => return Err(error),
            LockedAdmission::Stale => continue,
            LockedAdmission::Insufficient(error) => {
                evicted = true;
                pressure_warning = Some(error);
                // Try to reclaim confirmed-idle once, then recheck with a
                // fresh sample (the eviction itself moves the generation).
                evict_idle_on_pressure();
                wait_for_pressure_release();
            }
        }
    }
    Err(
        "Local inference stayed busy while validating admission. Wait a moment, then try again."
            .into(),
    )
}

/// One locked admission attempt. `probe_gen` is the coordinator generation
/// captured BEFORE the hardware sample; a mismatch means an accounting
/// transition landed between sample and lock, so the sample and the
/// in-lock pending figures describe different worlds and must not be mixed.
enum LockedAdmission {
    /// Admitted; carries an advisory when a resident reuse was admitted
    /// despite a low-memory estimate.
    Admitted(Option<String>),
    Busy(String),
    Denied(String),
    Insufficient(String),
    Stale,
}

pub(crate) const MEMORY_UNAVAILABLE: &str = "Memory unavailable:";

fn memory_denied(detail: &str) -> LockedAdmission {
    LockedAdmission::Denied(format!("{MEMORY_UNAVAILABLE} {detail} Close another GPU application, choose CPU text processing, or use one model at a time."))
}

/// Wait for an idle owner's acknowledgement, never assume a 50ms sleep freed
/// a model. Bounded: callers fail safely if the owner cannot release yet.
fn wait_for_pressure_release() {
    let started = Instant::now();
    while asr_release_pending() && started.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn try_acquire_asr_locked(
    model_id: &str,
    what: &str,
    hw: &HardwareSnapshot,
    gpu: Option<(u64, u64)>,
    probe_gen: u64,
    admit_on_pressure: bool,
) -> LockedAdmission {
    let mut state = match lock_state() {
        Ok(state) => state,
        Err(error) => return LockedAdmission::Busy(error),
    };
    if state.update_installing {
        return LockedAdmission::Busy(UPDATE_INSTALLING.into());
    }
    if state.generation != probe_gen {
        return LockedAdmission::Stale;
    }
    if state.asr_active.is_some() {
        return LockedAdmission::Busy("Speech recognition is already running.".into());
    }
    if is_sequential_locked(&state) && state.text_active.is_some() {
        return LockedAdmission::Busy(
            "A text job is using local inference. Wait for it to finish before dictating in one-stage mode."
                .into(),
        );
    }
    // Resident-once: reusing the resident needs no new bytes.
    let already_resident = state.asr_loaded.as_deref() == Some(model_id);
    let (host_max, vram_max) = estimate_pair_max(model_id);
    let known = estimated_ram_interval(model_id).is_some();
    // RAM fit including the OTHER stage's pending (not yet visible to the OS).
    let mut advisory = None;
    if known {
        let additional = if already_resident { 0 } else { host_max };
        let need = additional.saturating_add(WORKING_OVERHEAD_BYTES);
        if let Some(available) = hw.available_ram_bytes {
            if admit_on_pressure && !already_resident && available < WORKING_OVERHEAD_BYTES {
                return memory_denied("System RAM is critically low.");
            }
            let reserve = reserve_bytes(hw.total_ram_bytes);
            let effective = available
                .saturating_sub(reserve)
                .saturating_sub(state.text_pending_host);
            if effective < need {
                let warning = format!(
                    "Available RAM is below the estimate for {what}: about {} plus working overhead, with {} reserved.",
                    format_bytes(Some(additional)),
                    format_bytes(Some(reserve)),
                );
                if already_resident {
                    // Reuse allocates no model bytes. Pressure eviction would
                    // only mark this very model for release (and the caller,
                    // often its owner, would wait on itself), so never evict
                    // or deny a resident reuse: warn and admit.
                    advisory = Some(warning);
                } else if !admit_on_pressure {
                    return LockedAdmission::Insufficient(warning);
                }
            }
        }
    }
    // VRAM fit on the SELECTED device including the other stage's pending.
    if vram_max > 0 {
        if let Some((total, free)) = gpu {
            let already = already_resident;
            let need = if already { 0 } else { vram_max };
            // Free already excludes resident OS/other-app allocations. The
            // model upper bound includes allocator headroom; don't reserve a
            // second 10% of the entire device on top of both.
            let usable = free.saturating_sub(state.text_pending_vram);
            if usable < need {
                let warning = format!(
                    "Selected CUDA device {} reports {} free ({} total) for {what}; the estimate is about {} on one device.",
                    SELECTED_CUDA_DEVICE_INDEX,
                    format_bytes(Some(free)),
                    format_bytes(Some(total)),
                    format_bytes(Some(need)),
                );
                if !admit_on_pressure {
                    return LockedAdmission::Insufficient(warning);
                }
                return memory_denied(&warning);
            }
        }
    }
    // Fit proven (or unknown → proceed). Mark active + reservation atomically.
    state.asr_active = Some(model_id.to_owned());
    if already_resident {
        clear_asr_pending(&mut state);
    } else {
        state.asr_pending_host = host_max;
        state.asr_pending_vram = vram_max;
        state.asr_pending_id = Some(model_id.to_owned());
    }
    state.asr_idle_since = None;
    state.asr_release_pending = false;
    state.generation = state.generation.wrapping_add(1);
    LockedAdmission::Admitted(advisory)
}

/// Fit check + active marking + pending reservation for text in ONE critical
/// section. Same atomicity contract as the ASR variant (including the
/// probe-generation consistency check). `role` only names the waiter for busy
/// errors.
pub fn acquire_text_with_admission(role: &str, model_id: &str, what: &str) -> Result<(), String> {
    let mut evicted = false;
    let mut pressure_warning = None;
    for _ in 0..4 {
        let probe_gen = generation_blocking();
        let hw = probe_hardware();
        let gpu = selected_gpu_pair();
        match try_acquire_text_locked(role, model_id, what, &hw, gpu, probe_gen, evicted) {
            LockedAdmission::Admitted(advisory) => {
                if let Some(warning) = advisory.as_deref().or(pressure_warning.as_deref()) {
                    admission_warning(warning);
                }
                return Ok(());
            }
            LockedAdmission::Busy(error) | LockedAdmission::Denied(error) => return Err(error),
            LockedAdmission::Stale => continue,
            LockedAdmission::Insufficient(error) => {
                evicted = true;
                pressure_warning = Some(error);
                evict_idle_on_pressure();
                wait_for_pressure_release();
            }
        }
    }
    Err(
        "Local inference stayed busy while validating admission. Wait a moment, then try again."
            .into(),
    )
}

fn try_acquire_text_locked(
    role: &str,
    model_id: &str,
    what: &str,
    hw: &HardwareSnapshot,
    gpu: Option<(u64, u64)>,
    probe_gen: u64,
    admit_on_pressure: bool,
) -> LockedAdmission {
    let mut state = match lock_state() {
        Ok(state) => state,
        Err(error) => return LockedAdmission::Busy(error),
    };
    if state.update_installing {
        return LockedAdmission::Busy(UPDATE_INSTALLING.into());
    }
    if state.generation != probe_gen {
        return LockedAdmission::Stale;
    }
    if state.text_active.is_some() {
        return LockedAdmission::Busy(format!(
            "Local text inference is busy. Wait for the current {role} job or stop it first."
        ));
    }
    if is_sequential_locked(&state) && (state.asr_active.is_some() || state.asr_loaded.is_some()) {
        return LockedAdmission::Busy(
            "Speech recognition still holds local inference in one-stage mode. Release it before loading cleanup."
                .into(),
        );
    }
    let already_resident = state.text_loaded.as_deref() == Some(model_id);
    let (host_max, vram_max) = estimate_pair_max(model_id);
    let mut budget_cpu = false;
    let mut advisory = None;
    let known = estimated_ram_interval(model_id).is_some();
    if known {
        let additional = if already_resident { 0 } else { host_max };
        let need = additional.saturating_add(WORKING_OVERHEAD_BYTES);
        if let Some(available) = hw.available_ram_bytes {
            if admit_on_pressure && !already_resident && available < WORKING_OVERHEAD_BYTES {
                return memory_denied("System RAM is critically low.");
            }
            let reserve = reserve_bytes(hw.total_ram_bytes);
            // Overlap: ASR may hold active + pending; sequential already
            // rejected above. Count ASR pending (and, conservatively, nothing
            // for loaded — the OS counter already reflects it).
            let effective = available
                .saturating_sub(reserve)
                .saturating_sub(state.asr_pending_host);
            if effective < need {
                let warning = format!(
                    "Available RAM is below the estimate for {what}: about {} plus working overhead, with {} reserved.",
                    format_bytes(Some(additional)),
                    format_bytes(Some(reserve)),
                );
                if already_resident {
                    // Same rule as ASR: pressure eviction would stop the text
                    // worker this caller is about to reuse.
                    advisory = Some(warning);
                } else if !admit_on_pressure {
                    return LockedAdmission::Insufficient(warning);
                }
            }
        }
    }
    if vram_max > 0 {
        if let Some((total, free)) = gpu {
            let need = if already_resident { 0 } else { vram_max };
            let usable = free.saturating_sub(state.asr_pending_vram);
            if usable < need {
                let warning = format!(
                    "Selected CUDA device {} reports {} free ({} total) for {what}; the estimate is about {} on one device.",
                    SELECTED_CUDA_DEVICE_INDEX,
                    format_bytes(Some(free)),
                    format_bytes(Some(total)),
                    format_bytes(Some(need)),
                );
                if !admit_on_pressure {
                    return LockedAdmission::Insufficient(warning);
                }
                // Auto already promises CPU fallback when CUDA cannot serve
                // the selected text model. Reserve CPU explicitly before
                // starting the child, rather than first provoking CUDA OOM.
                budget_cpu = true;
            }
        }
    }
    state.text_active = Some(model_id.to_owned());
    state.text_budget_cpu = budget_cpu;
    if already_resident {
        clear_text_pending(&mut state);
    } else {
        state.text_pending_host = host_max;
        state.text_pending_vram = if budget_cpu { 0 } else { vram_max };
        state.text_pending_id = Some(model_id.to_owned());
    }
    state.text_idle_since = None;
    state.generation = state.generation.wrapping_add(1);
    LockedAdmission::Admitted(advisory)
}

/// Release a pending reservation WITHOUT touching active/loaded.
/// Used when an atomic acquire succeeded but the later native load was
/// cancelled before `note_*_loaded`/`note_*_load_failed` ran.
pub fn release_asr_reservation() {
    if let Ok(mut state) = shared().lock() {
        // Only clear pending when no successful load converted it. `loaded`
        // accounting stays untouched.
        if state.asr_pending_id.is_some() {
            clear_asr_pending(&mut state);
            state.generation = state.generation.wrapping_add(1);
        }
    }
}

/// Release a text pending reservation without touching active/loaded.
pub fn release_text_reservation() {
    if let Ok(mut state) = shared().lock() {
        if state.text_pending_id.is_some() {
            clear_text_pending(&mut state);
            state.generation = state.generation.wrapping_add(1);
        }
    }
}

/// Blocking wait/queue for the shared text worker at a task boundary.
/// Tries `acquire_text` immediately, then polls every 50 ms until `timeout`
/// while observing `cancel`/`stop`. Returns Ok once the lease is held.
/// - cancel set -> Err(cancelled), no lease.
/// - stop set -> Err(stopped), caller should use fast local rules.
/// - timeout -> Err(busy waiting timeout), caller must surface it explicitly
///   (never a silent rules downgrade).
/// Legacy lease-only waiter kept for back-compat; owned paths must use
/// `acquire_text_waiting_with_admission` (atomic reservation with advisory
/// memory estimates).
pub fn acquire_text_waiting(
    role: &str,
    model_id: &str,
    timeout: Duration,
    cancel: &std::sync::atomic::AtomicBool,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    // Delegate to the atomic variant with a generic task label. Admission
    // pressure remains advisory inside; busy retries until timeout.
    acquire_text_waiting_with_admission(role, model_id, "text", timeout, cancel, stop)
}

/// Atomic waiter: reservation + advisory fit tied to lease ownership. Retries
/// only on busy/sequential contention. Cancel/stop is observed every 50 ms;
/// on cancel/stop nothing is marked.
pub fn acquire_text_waiting_with_admission(
    role: &str,
    model_id: &str,
    what: &str,
    timeout: Duration,
    cancel: &std::sync::atomic::AtomicBool,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Local text job cancelled while waiting for the model.".into());
        }
        if stop.load(Ordering::Acquire) {
            return Err("Local text job stopped while waiting for the model.".into());
        }
        match acquire_text_with_admission(role, model_id, what) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if e.starts_with(MEMORY_UNAVAILABLE) {
                    return Err(e);
                }
                if start.elapsed() >= timeout {
                    return Err(format!(
                        "Local text inference stayed busy while waiting ({role}/{model_id}). {e}"
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// App exit: shut down text workers and mark ASR for release. Child
/// processes are owned by cleanup.rs job objects; shutdown drops them.
pub fn release_all_for_exit() {
    crate::punctuation::release_cached();
    crate::cleanup::shutdown();
    if let Ok(mut state) = shared().lock() {
        state.text_active = None;
        state.text_loaded = None;
        clear_text_pending(&mut state);
        state.asr_active = None;
        state.asr_loaded = None;
        clear_asr_pending(&mut state);
        state.asr_release_pending = false;
        state.asr_idle_since = None;
        state.text_idle_since = None;
    }
}

/// Test-only reset. Production never calls this; retention timers are real.
/// Injects ample hermetic RAM (32 GiB total / 24 GiB free) so lease tests do
/// not depend on live host pressure (the review host reported ~3.7 GiB free).
/// VRAM stays unknown (proceeds) unless a test injects a GPU table.
#[cfg(test)]
pub(crate) fn reset_for_tests(mode: MemoryMode, retention: ModelRetention) {
    if let Ok(mut state) = shared().lock() {
        *state = State::default();
        state.config.memory_mode = mode;
        state.config.retention = retention;
        store_retention(retention);
    }
    set_test_memory(32 << 30, 24 << 30);
    set_test_gpu_table(None);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn profiles_gpu_metadata_stays_on_the_selected_device() {
        let row = selected_gpu_hardware(
            "1, 24576, 20000, NVIDIA Other GPU, 610.88, 12.0\n0, 6144, 1000, NVIDIA Selected GPU, 610.88, 7.5\n",
        ).unwrap();
        assert_eq!(row.index, SELECTED_CUDA_DEVICE_INDEX);
        assert_eq!(row.total_bytes, Some(6 << 30));
        assert_eq!(row.name.as_deref(), Some("NVIDIA Selected GPU"));
        assert_eq!(row.driver_version.as_deref(), Some("610.88"));
        assert_eq!(row.compute_capability.as_deref(), Some("7.5"));
        assert!(selected_gpu_hardware("1, 24576, 20000, NVIDIA Other GPU, 610.88, 12.0").is_none());
    }

    #[test]
    fn profiles_gpu_metadata_retains_unknown_fields() {
        for total in ["[N/A]", "0", "-1", "262145", "18446744073709551615"] {
            let row = selected_gpu_hardware(&format!(
                "0, {total}, [N/A], NVIDIA GPU, 610.88, [Not Supported]"
            )).unwrap();
            assert_eq!(row.total_bytes, None);
            assert_eq!(row.name.as_deref(), Some("NVIDIA GPU"));
            assert_eq!(row.compute_capability, None);
        }
        let old_query = selected_gpu_hardware("0, 12288, [N/A], NVIDIA GPU, 580.88").unwrap();
        assert_eq!(old_query.total_bytes, Some(12 << 30));
        assert_eq!(old_query.compute_capability, None);
        let unknown = selected_gpu_hardware("0, [N/A], [N/A], [N/A], [N/A], [N/A]").unwrap();
        assert!(unknown.name.is_none());
        assert!(unknown.driver_version.is_none());
    }

    #[test]
    fn profiles_gpu_metadata_rejects_malformed_or_ambiguous_tables() {
        for text in [
            "", "0, 6144, 5000", "bad, 6144, 5000, NVIDIA GPU, 610.88, 7.5",
            "0, 6144, 5000, NVIDIA GPU, 610.88, 7.5, extra",
            "0, 6144, 5000, NVIDIA GPU, 610.88, 7.5\n0, 12288, 9000, NVIDIA GPU, 610.88, 12.0",
        ] {
            assert!(parse_gpu_hardware_table(text).is_none(), "{text}");
        }
        let too_many = (0..17).map(|index| format!(
            "{index}, 6144, 5000, NVIDIA GPU, 610.88, 7.5\n"
        )).collect::<String>();
        assert!(parse_gpu_hardware_table(&too_many).is_none());
    }

    #[test]
    fn profiles_physical_cores_count_variable_sized_windows_records() {
        let mut bytes = Vec::new();
        for size in [48u32, 64, 48] {
            bytes.extend_from_slice(&0u32.to_ne_bytes());
            bytes.extend_from_slice(&size.to_ne_bytes());
            bytes.resize(bytes.len() + size as usize - 8, 0);
        }
        assert_eq!(count_processor_core_records(&bytes), Some(3));
        assert_eq!(count_processor_core_records(&[]), None);
        assert_eq!(count_processor_core_records(&bytes[..bytes.len() - 1]), None);
        let mut bad = bytes.clone();
        bad[4..8].copy_from_slice(&0u32.to_ne_bytes());
        assert_eq!(count_processor_core_records(&bad), None);
        bad[4..8].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert_eq!(count_processor_core_records(&bad), None);
        bytes[..4].copy_from_slice(&1u32.to_ne_bytes());
        assert_eq!(count_processor_core_records(&bytes), None);
    }

    /// Global lease state is process-wide; serialize the tests that mutate
    /// it so parallel runners cannot interleave modes. Other modules' tests
    /// that take ASR leases must hold this gate too.
    pub(crate) fn test_gate() -> std::sync::MutexGuard<'static, ()> {
        static GATE: OnceLock<Mutex<()>> = OnceLock::new();
        GATE.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn update_status_stays_responsive_when_admission_is_locked() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::OnExit);
        let held = lock_state().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender.send(update_blocked_reason()).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(1));
        drop(held);
        worker.join().unwrap();
        assert!(result.expect("snapshot waited on admission").is_some());
        assert!(update_blocked_reason().is_none());
    }

    #[test]
    fn hardware_probe_never_invents_values() {
        let _gate = test_gate();
        // Test hermetic RAM by default; clear to probe the live host honestly.
        clear_test_memory();
        set_test_gpu_table(None);
        // Live probe must never OOM or disturb the host; it only reads.
        let hardware = probe_hardware();
        // Restore hermetic defaults for the next test.
        set_test_memory(32 << 30, 24 << 30);
        // Unknown stays None, never zero. On Windows CI this usually has RAM;
        // elsewhere it must be None, not a fabricated number.
        for value in [
            hardware.total_ram_bytes,
            hardware.available_ram_bytes,
            hardware.dedicated_vram_bytes,
        ] {
            assert!(value.is_none() || value.unwrap() > 0);
        }
        // Dedicated VRAM is Some only when bounded nvidia-smi parsing
        // succeeded; otherwise honest None (never AdapterRAM truncation).
        if let Some(vram) = hardware.dedicated_vram_bytes {
            assert!(vram > 0);
        }
    }

    #[test]
    fn reserve_policy_is_conservative_and_reviewable() {
        let eight_gb = Some(8 << 30);
        // 15% of 8 GiB = 1.2 GiB, above the 1 GiB floor.
        assert_eq!(reserve_bytes(eight_gb), (8u64 << 30) * 15 / 100);
        // Small machines still reserve the full 1 GiB floor.
        assert_eq!(reserve_bytes(Some(2 << 30)), RESERVE_FLOOR_BYTES);
        assert_eq!(reserve_bytes(None), RESERVE_FLOOR_BYTES);
    }

    #[test]
    fn guidance_callout_uses_real_values_or_unknown() {
        let text = guidance_text(&HardwareSnapshot::unknown(), None);
        assert!(text.contains("unknown"));
        assert!(text.contains("Leave room for your other apps."));
        let hardware = HardwareSnapshot {
            total_ram_bytes: Some(16 << 30),
            available_ram_bytes: Some(8 << 30),
            dedicated_vram_bytes: None,
        };
        let text = guidance_text(&hardware, Some((1 << 30, 2 << 30)));
        assert!(text.contains("16.0 GiB"));
        assert!(text.contains("8.0 GiB"));
    }

    #[test]
    fn sequential_blocks_prewarm_and_text_while_asr_resident() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Sequential, ModelRetention::Minutes10);
        assert!(should_prewarm_cleanup_blocking());
        assert!(should_prewarm_cleanup());
        acquire_asr("moonshine-medium").unwrap();
        // Lease alone (before successful load) already blocks sequential text;
        // readiness only strengthens it.
        assert!(!should_prewarm_cleanup_blocking());
        assert!(!should_prewarm_cleanup());
        note_asr_loaded("moonshine-medium");
        assert!(asr_release_due_for_cleanup());
        assert!(acquire_text("cleanup", "lfm2.5-1.2b").is_err());
        notify_asr_finished();
        // Still resident until the caller actually drops the recognizer.
        assert!(!should_prewarm_cleanup_blocking());
        assert!(acquire_text("cleanup", "lfm2.5-1.2b").is_err());
        confirm_asr_released();
        assert!(should_prewarm_cleanup_blocking());
        // Blocking gate is fail-closed; polling hint agrees when uncontended.
        assert!(should_prewarm_cleanup());
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        assert!(acquire_asr("moonshine-medium").is_err());
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn lease_is_not_proof_of_load_until_noted() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        acquire_asr("moonshine-medium").unwrap();
        // Active but not yet loaded: readiness requires actual load success.
        assert!(model_state_blocking("moonshine-medium").0);
        assert!(!model_state_blocking("moonshine-medium").1);
        note_asr_loaded("moonshine-medium");
        assert!(model_state_blocking("moonshine-medium").1);
        notify_asr_finished();
        // Finished but still resident until confirmed drop.
        assert!(model_state_blocking("moonshine-medium").1);
        confirm_asr_released();
        assert!(!model_state_blocking("moonshine-medium").1);
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        assert!(!model_state_blocking("lfm2.5-1.2b").1);
        note_text_loaded("lfm2.5-1.2b");
        assert!(model_state_blocking("lfm2.5-1.2b").1);
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn overlap_permits_both_stages_but_busy_is_visible() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        assert!(is_busy());
        assert_eq!(active_stages(), vec!["asr".to_owned()]);
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        assert!(is_busy());
        notify_asr_finished();
        assert!(is_busy());
        notify_text_finished();
        assert!(!is_busy());
        assert!(is_model_loaded("moonshine-medium"));
        assert!(is_model_loaded("lfm2.5-1.2b"));
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn active_jobs_are_never_evicted_by_idle_or_pressure() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Immediately);
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        assert_eq!(release_idle(), 0);
        assert_eq!(evict_idle_on_pressure(), 0);
        assert!(is_model_loaded("moonshine-medium"));
        assert!(is_model_loaded("lfm2.5-1.2b"));
        notify_asr_finished();
        notify_text_finished();
        // Immediately retention requests ASR release (pending, not counted)
        // and confirmed-clears a stale text flag with no live child (0 counted
        // because no child process actually stopped in unit tests).
        let released = release_idle();
        assert_eq!(released, 0);
        assert!(asr_release_pending());
        // Pending is never accounted as freed: blocking check still sees the
        // ASR allocation resident until speech confirms the drop.
        assert!(model_state_blocking("moonshine-medium").1);
        confirm_asr_released();
        assert!(!asr_release_pending());
        assert!(!model_state_blocking("moonshine-medium").1);
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn retention_gates_idle_release_and_activity_cancels_eviction() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished();
        // 10-minute retention: not yet elapsed, nothing released.
        assert_eq!(release_idle(), 0);
        assert!(!asr_release_pending());
        // Simulate an old idle timestamp.
        if let Ok(mut state) = shared().lock() {
            state.asr_idle_since = Some(Instant::now() - Duration::from_secs(11 * 60));
        }
        // A new request cancels pending eviction.
        touch_activity(Stage::Asr);
        assert_eq!(release_idle(), 0);
        if let Ok(mut state) = shared().lock() {
            state.asr_idle_since = Some(Instant::now() - Duration::from_secs(11 * 60));
        }
        // Retention elapsed: pending is requested but NOT counted as freed.
        assert_eq!(release_idle(), 0);
        assert!(asr_release_pending());
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn new_activity_between_decision_and_shutdown_survives_timer() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Immediately);
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        notify_text_finished();
        // Capture generation at decision time, then simulate a new job
        // arriving before the shutdown completes.
        let gen_before = generation_blocking();
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        assert!(generation_blocking() != gen_before);
        // Timer path with a stale generation must not clear the new lease.
        // Active jobs short-circuit to 0; the new lease survives.
        assert_eq!(release_idle(), 0);
        assert!(model_state_blocking("lfm2.5-1.2b").0);
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn idle_shutdown_serializes_with_new_acquire_behind_barrier() {
        use std::sync::{Arc, Barrier};
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Immediately);
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        notify_text_finished();
        // Hold the lease mutex inside release_idle so a concurrent acquire
        // must block instead of slipping between decision and shutdown.
        // Barrier synchronizes start; the entered-flag guarantees the idle
        // thread holds the mutex before the waiter attempts its acquire.
        set_idle_test_delay_ms(200);
        let barrier = Arc::new(Barrier::new(2));
        let start_barrier = barrier.clone();
        let idle = std::thread::spawn(move || {
            start_barrier.wait();
            release_idle()
        });
        barrier.wait();
        // Wait until the idle thread is inside its critical section.
        let entered = Instant::now();
        while !IDLE_IN_CRITICAL.load(std::sync::atomic::Ordering::Acquire) {
            assert!(
                entered.elapsed() < Duration::from_secs(5),
                "idle thread never entered its critical section"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // This acquire must block on the idle mutex (not slip between
        // decision and shutdown), then load fresh once idle releases.
        let start = Instant::now();
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        let waited = start.elapsed();
        assert!(
            waited >= Duration::from_millis(100),
            "acquire should have blocked on the idle critical section, waited {waited:?}"
        );
        let _ = idle.join().unwrap();
        // New lease survived with active flag intact; idle never killed it.
        assert!(model_state_blocking("lfm2.5-1.2b").0);
        set_idle_test_delay_ms(0);
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn text_waiting_queue_observes_cancel_and_stop_without_drops() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        // Cancelled waiter gets no lease and the holder keeps its lease.
        let cancel = AtomicBool::new(true);
        let stop = AtomicBool::new(false);
        let err = acquire_text_waiting(
            "notes",
            "lfm2.5-1.2b",
            Duration::from_secs(5),
            &cancel,
            &stop,
        )
        .unwrap_err();
        assert!(err.contains("cancelled"), "{err}");
        assert!(model_state_blocking("lfm2.5-1.2b").0);
        // Stopped waiter also gets no lease (fast safe path upstream).
        cancel.store(false, Ordering::Release);
        stop.store(true, Ordering::Release);
        let err = acquire_text_waiting(
            "notes",
            "lfm2.5-1.2b",
            Duration::from_secs(5),
            &cancel,
            &stop,
        )
        .unwrap_err();
        assert!(err.contains("stopped"), "{err}");
        // Short timeout surfaces busy explicitly (never silent).
        stop.store(false, Ordering::Release);
        let err = acquire_text_waiting(
            "notes",
            "lfm2.5-1.2b",
            Duration::from_millis(120),
            &cancel,
            &stop,
        )
        .unwrap_err();
        assert!(err.contains("busy"), "{err}");
        notify_text_finished();
        // Worker now free: waiting acquires promptly.
        acquire_text_waiting(
            "notes",
            "lfm2.5-1.2b",
            Duration::from_secs(5),
            &cancel,
            &stop,
        )
        .unwrap();
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn estimates_cover_asr_text_and_whisper_with_explicit_assumptions() {
        // Qwen ASR, text, Whisper and CUDA families have provisional intervals;
        // unsupported stays unknown.
        for id in [
            "qwen3-asr-1.7b",
            "qwen3-4b-instruct-2507",
            "lfm2.5-1.2b",
            "whisper-tiny",
            "whisper-large-v3-turbo",
            "moonshine-small",
            "whisper-tiny-fp32-cuda",
        ] {
            let (lo, hi) = estimated_ram_interval(id).expect(id);
            assert!(lo > 0 && hi >= lo, "{id}");
            assert!(estimate_evidence(id).contains("Provisional"), "{id}");
            assert!(estimate_basis(id).contains("Provisional"), "{id}");
        }
        // Qwen FP16 CUDA is the measured exception: native VRAM peak
        // 2026-09-16 (+~7.1 GiB incremental), host RAM unmeasured. The old
        // 4800-6500 MiB provisional VRAM band underestimated the delta.
        let (qlo, qhi) =
            estimated_vram_interval("qwen3-asr-1.7b-fp16-cuda").expect("qwen cuda vram");
        assert!(qlo >= 7100 * MIB && qhi >= qlo, "measured band");
        assert!(estimate_evidence("qwen3-asr-1.7b-fp16-cuda").contains("Observed"));
        assert!(estimate_basis("qwen3-asr-1.7b-fp16-cuda").contains("Observed"));
        // The llama.cpp Q8 row is measured too (2026-09-28 offline benchmark).
        assert!(estimate_evidence(crate::llama_asr::MODEL_ID).contains("Observed"));
        assert!(estimate_basis(crate::llama_asr::MODEL_ID).contains("Observed"));
        let smart = crate::optional_llm::SMART_CLEANUP_ID;
        assert_eq!(estimated_ram_interval(smart), Some((1000 * MIB, 1500 * MIB)));
        assert_eq!(estimated_vram_interval(smart), Some((850 * MIB, 1200 * MIB)));
        assert!(estimate_basis(smart).contains("Mac unified-memory use is unmeasured"));
        assert!(estimate_evidence(smart).contains("Observed Windows direct-server"));
        assert_eq!(estimated_ram_interval("not-a-model"), None);
        // CPU ASR never consumes dedicated VRAM; text depends on offload.
        assert_eq!(estimated_vram_interval("whisper-tiny"), Some((0, 0)));
        assert_eq!(
            estimated_vram_interval("lfm2.5-1.2b"),
            Some((1200 * MIB, 2200 * MIB))
        );
        // CUDA VRAM is provisional with method, never a multi-GPU sum.
        let (lo, hi) = estimated_vram_interval("whisper-tiny-fp32-cuda").expect("tiny cuda vram");
        assert!(lo > 0 && hi >= lo);
        let (lo, hi) = estimated_vram_interval("qwen3-asr-1.7b-fp16-cuda").expect("qwen cuda vram");
        assert!(lo > 0 && hi >= lo);
        // Overlap sums + working overhead; sequential takes max + overhead.
        let overlap = plan_range("whisper-tiny", "lfm2.5-1.2b", MemoryMode::Overlap).unwrap();
        let seq = plan_range("whisper-tiny", "lfm2.5-1.2b", MemoryMode::Sequential).unwrap();
        assert!(overlap.0 <= overlap.1 && seq.0 <= seq.1);
        assert!(overlap.0 >= seq.0);
        assert_eq!(
            plan_range("whisper-tiny", "nope", MemoryMode::Overlap),
            None
        );
    }

    #[test]
    fn measured_cpu_speech_peaks_keep_headroom_and_unmeasured_variants() {
        for (id, peak) in [
            ("moonshine-medium", 560_000_000),
            ("whisper-tiny.en", 530_000_000),
            ("whisper-small", 1_280_000_000),
        ] {
            let (low, high) = estimated_ram_interval(id).unwrap();
            assert_eq!(low, peak, "{id}: preserve decimal GB measurement");
            assert!(high > peak, "{id}: keep planning headroom");
            assert_eq!(estimated_vram_interval(id), Some((0, 0)));
            assert!(estimate_basis(id).contains("BENCHMARK-WINDOWS-2026-09-28.md"));
            assert!(estimate_evidence(id).contains("Observed"));
        }
        for id in ["moonshine-small", "whisper-tiny", "whisper-small.en"] {
            assert!(estimate_evidence(id).contains("Provisional"), "{id}");
        }
        assert_eq!(estimated_ram_interval("moonshine-small"), Some((400 * MIB, 800 * MIB)));
        let speech = estimated_ram_interval("moonshine-medium").unwrap();
        let notes = estimated_ram_interval("lfm2.5-1.2b").unwrap();
        assert_eq!(plan_range("moonshine-medium", "lfm2.5-1.2b", MemoryMode::Overlap),
            Some((speech.0 + notes.0 + WORKING_OVERHEAD_BYTES,
                  speech.1 + notes.1 + WORKING_OVERHEAD_BYTES)));
    }

    #[test]
    fn metal_qwen_estimates_keep_server_evidence_and_one_unified_memory_pool() {
        for (id, rss, upper_mib) in [
            (crate::llama_asr::METAL_Q8_MODEL_ID, 2_800_000_000, 3600),
            (crate::llama_asr::METAL_Q4_MODEL_ID, 2_360_000_000, 3000),
        ] {
            let speech = estimated_ram_interval(id).unwrap();
            assert_eq!(speech, (rss, upper_mib * MIB));
            assert!(speech.1 > rss + WORKING_OVERHEAD_BYTES, "{id}: allow for worker memory and headroom");
            assert_eq!(estimated_vram_interval(id), Some((0, 0)));
            let basis = estimate_basis(id);
            assert!(basis.contains("server RSS (decimal)"));
            assert!(basis.contains("combined evaluator, worker and server RSS"));
            assert!(basis.contains("b10964 mac-arm64 Metal, context 2048, load-mode none, -ngl 999"));
            assert!(basis.contains("no separate VRAM"));
            let evidence = estimate_evidence(id);
            assert!(evidence.contains("evaluator + worker + server peak RSS"));
            assert!(evidence.contains("BENCHMARK-MAC-METAL-2026-09-28.md"));
            let notes = estimated_ram_interval("lfm2.5-1.2b").unwrap();
            assert_eq!(plan_range(id, "lfm2.5-1.2b", MemoryMode::Overlap),
                Some((speech.0 + notes.0 + WORKING_OVERHEAD_BYTES,
                      speech.1 + notes.1 + WORKING_OVERHEAD_BYTES)));
            assert_eq!(plan_range(id, "lfm2.5-1.2b", MemoryMode::Sequential),
                Some((speech.0.max(notes.0) + WORKING_OVERHEAD_BYTES,
                      speech.1.max(notes.1) + WORKING_OVERHEAD_BYTES)));
        }
        assert!(estimate_basis("whisper-small-metal").starts_with("Provisional"));
        assert_eq!(estimated_ram_interval("qwen3-asr-1.7b-unknown-metal"), None);
        assert_eq!(estimated_vram_interval("qwen3-asr-1.7b-unknown-metal"), None);
    }

    #[test]
    fn admission_accounts_resident_once_and_never_ooms_on_unknown() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        // Resident-once: a resident model needs no additional estimate bytes,
        // so a huge estimate for the SAME resident id behaves like a zero
        // additional request (both need working overhead + reserve only).
        // This holds regardless of live machine pressure (volatile available).
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        assert!(model_state_blocking("lfm2.5-1.2b").1);
        let resident_huge = admission_check_model("lfm2.5-1.2b", u64::MAX, "notes").is_ok();
        let zero_additional = admission_check(0, "notes").is_ok();
        assert_eq!(resident_huge, zero_additional);
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn legacy_memory_pressure_is_advisory() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        set_test_memory(8 << 30, 512 << 20);
        assert!(admission_check_model("lfm2.5-1.2b", 3 << 30, "legacy text").is_ok());
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn configure_tracks_settings_without_touching_disk() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        let mut settings = Settings::default();
        settings.memory_mode = MemoryMode::Sequential;
        settings.model_retention = ModelRetention::OnExit;
        configure(&settings);
        assert!(is_sequential());
        assert_eq!(current_retention(), ModelRetention::OnExit);
        // OnExit never auto-evicts.
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished();
        if let Ok(mut state) = shared().lock() {
            state.asr_idle_since = Some(Instant::now() - Duration::from_secs(3600 * 24));
        }
        assert_eq!(release_idle(), 0);
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn retention_snapshot_survives_coordinator_lock_contention() {
        let _gate = test_gate();
        for retention in [ModelRetention::Immediately, ModelRetention::Minutes30, ModelRetention::OnExit] {
            let mut settings = Settings::default();
            settings.model_retention = retention;
            configure(&settings);
            let _held = shared().lock().unwrap();
            assert_eq!(current_retention(), retention);
        }
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn await_asr_release_times_out_rather_than_loading_concurrently() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        // Idle resident with no worker coming: the waiter must fail, never
        // load under it, and never fake the resident flag clear.
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished();
        assert!(asr_resident_blocking());
        let cancel = AtomicBool::new(false);
        let err = await_asr_release(&cancel, Duration::from_millis(80)).unwrap_err();
        assert!(err.contains("still loaded"), "{err}");
        assert!(asr_resident_blocking());
        // Confirmed physical drop opens the gate promptly.
        confirm_asr_released();
        assert!(!asr_resident_blocking());
        assert!(await_asr_release(&cancel, Duration::from_millis(80)).is_ok());
        // Cancellation aborts even a long wait without touching residency.
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished();
        let cancel = AtomicBool::new(true);
        let err = await_asr_release(&cancel, Duration::from_secs(5)).unwrap_err();
        assert!(err.contains("cancelled"), "{err}");
        assert!(asr_resident_blocking());
        confirm_asr_released();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        let _ = Ordering::Acquire;
    }

    #[test]
    fn stale_probe_generation_rejects_mixed_sample_and_reservation() {
        // The reviewed race is real: a pending load converting to resident
        // between another contender's hardware sample and its locked
        // admission clears the pending reservation while the OS free counter
        // drops by the same bytes. Mixing the stale free sample with the
        // fresh (cleared) pending would over-admit into a joint OOM.
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        let hw = probe_hardware();
        let gpu = selected_gpu_pair();
        // Any generation mismatch — even without other activity — refuses to
        // mix worlds and marks no lease.
        let stale = generation_blocking().wrapping_add(1);
        assert!(matches!(
            try_acquire_asr_locked("whisper-tiny", "test", &hw, gpu, stale, false),
            LockedAdmission::Stale
        ));
        assert!(!model_state_blocking("whisper-tiny").0);
        assert!(matches!(
            try_acquire_text_locked("cleanup", "lfm2.5-1.2b", "test", &hw, gpu, stale, false),
            LockedAdmission::Stale
        ));
        assert!(!model_state_blocking("lfm2.5-1.2b").0);
        // The exact interleaving: sample first, then the other stage's
        // pending converts to resident (pending clears, generation moves).
        // The contender's locked attempt against its pre-conversion sample
        // must report Stale rather than admit.
        let gen_before = generation_blocking();
        let sample_before = probe_hardware();
        let gpu_before = selected_gpu_pair();
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        assert!(generation_blocking() != gen_before);
        assert!(matches!(
            try_acquire_asr_locked(
                "whisper-tiny",
                "test",
                &sample_before,
                gpu_before,
                gen_before,
                false
            ),
            LockedAdmission::Stale
        ));
        assert!(!model_state_blocking("whisper-tiny").0);
        // The public path re-samples and then decides honestly on fresh
        // numbers (ample injected RAM here, so it admits exactly once).
        acquire_asr_with_admission("whisper-tiny", "test").unwrap();
        assert!(model_state_blocking("whisper-tiny").0);
        note_asr_load_failed();
        notify_asr_finished();
        notify_text_finished();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn confirm_keeps_replacement_reservation_and_arms_same_id_reuse() {
        // Drop-to-reload gap, closed deterministically. Tight injected
        // memory (8 GiB total, 4096 MiB free; reserve ~1228 MiB) fits an
        // lfm2.5-1.2b text load (2150 + 256 MiB overhead) fits with zero ASR
        // pending. ASR pending makes the estimate exceed the advisory budget,
        // but the request is still admitted after one eviction attempt.
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        set_test_memory(8 << 30, 4096 << 20);
        set_test_gpu_table(None);
        // Baseline: nothing pending, the text load fits.
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "probe").unwrap();
        notify_text_finished();

        // Case 1 — different-ID replacement: the file job's pending
        // reservation survives the worker's confirmation of the old drop.
        acquire_asr_with_admission("whisper-tiny", "dictation").unwrap();
        confirm_asr_released();
        assert!(!asr_resident_blocking());
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "competing").unwrap();
        assert!(model_state_blocking("lfm2.5-1.2b").0);
        notify_text_finished();
        // The replacement converts cleanly and leaves no strand.
        note_asr_loaded("whisper-tiny");
        notify_asr_finished();
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "after").unwrap();
        notify_text_finished();

        // Case 2 — same-ID cached reuse: zero pending at acquire (resident),
        // armed at the deliberate confirm; cancel/finish releases it.
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished(); // idle resident, pending 0
        acquire_asr_with_admission("moonshine-medium", "dictation").unwrap();
        // Pre-fix this text load slipped the gap (pending 0, stale trust).
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "reuse-gap").unwrap();
        notify_text_finished();
        confirm_asr_released(); // deliberate cached release for replacement
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "competing2").unwrap();
        notify_text_finished();
        notify_asr_finished(); // cancel/finish without load
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "cancelled").unwrap();
        notify_text_finished();

        // Case 3 — idle confirm clears: no active lease, nothing strands,
        // residency truthfully None.
        acquire_asr("moonshine-medium").unwrap();
        note_asr_loaded("moonshine-medium");
        notify_asr_finished();
        confirm_asr_released();
        assert!(!asr_resident_blocking());
        acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "idle").unwrap();
        notify_text_finished();

        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn atomic_reservation_tracks_joint_pressure_without_denial() {
        use std::sync::{Arc, Barrier};
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        // Controlled volatile memory: 8 GiB total, 4 GiB available.
        // Reserve = 1.2 GiB, usable ~= 2.8 GiB. whisper-tiny (~600 MiB max +
        // 256 MiB overhead) fits alone; lfm2.5-1.2b (~2150 MiB + 256 MiB)
        // fits alone; together they exceed usable once pending is counted.
        // The estimate is advisory, so both leases are recorded.
        set_test_memory(8 << 30, 4 << 30);
        set_test_gpu_table(None);
        let barrier = Arc::new(Barrier::new(2));
        let b1 = barrier.clone();
        let asr = std::thread::spawn(move || {
            b1.wait();
            acquire_asr_with_admission("whisper-tiny", "dictation")
        });
        let b2 = barrier.clone();
        let text = std::thread::spawn(move || {
            b2.wait();
            acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "dictation cleanup")
        });
        let asr_res = asr.join().unwrap();
        let text_res = text.join().unwrap();
        assert!(asr_res.is_ok(), "asr admission failed: {asr_res:?}");
        assert!(text_res.is_ok(), "text admission failed: {text_res:?}");
        assert!(model_state_blocking("whisper-tiny").0);
        assert!(model_state_blocking("lfm2.5-1.2b").0);
        note_asr_load_failed();
        notify_asr_finished();
        note_text_load_failed();
        notify_text_finished();
        // Pending never leaks: both stages clear after finish/failure.
        if let Ok(state) = shared().lock() {
            assert_eq!(state.asr_pending_host, 0);
            assert_eq!(state.text_pending_host, 0);
        }
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn gpu_admission_enforces_selected_device_not_largest() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        // Unequal two-device machine: device 0 is the SMALL card (4 GiB
        // total, 3 GiB free), device 1 is the LARGE card (12 GiB total,
        // 11 GiB free). The backend uses default device 0, so a ~4.8-6.5 GiB
        // Qwen CUDA load raises pressure against device 0 even though the
        // largest device could fit. A load must not use that other device's budget.
        set_test_memory(32 << 30, 24 << 30);
        set_test_gpu_table(Some("0, 4096, 3072\n1, 12288, 11264\n".to_owned()));
        let pair = selected_gpu_pair().expect("selected pair");
        assert_eq!(pair.0, 4096 * 1024 * 1024);
        assert_eq!(pair.1, 3072 * 1024 * 1024);
        // Legacy max-based names now return the selected device, never max.
        assert_eq!(largest_gpu_total_vram_bytes(), Some(4096 * 1024 * 1024));
        assert_eq!(largest_gpu_free_vram_bytes(), Some(3072 * 1024 * 1024));
        // Legacy advisory is kept for compatibility; the owned load is refused.
        assert!(admission_for_model("qwen3-asr-1.7b-fp16-cuda", "test").is_ok());
        let error = acquire_asr_with_admission("qwen3-asr-1.7b-fp16-cuda", "test").unwrap_err();
        assert!(error.starts_with(MEMORY_UNAVAILABLE));
        assert!(!model_state_blocking("qwen3-asr-1.7b-fp16-cuda").0);
        note_asr_load_failed();
        notify_asr_finished();
        // Tiny FP32 CUDA (~600-1100 MiB) fits device 0 and passes.
        assert!(admission_for_model("whisper-tiny-fp32-cuda", "test").is_ok());
        // Fit warning still reports the pressure without rejecting downloads.
        assert!(vram_fit_warning("qwen3-asr-1.7b-fp16-cuda", "test").is_some());
        assert!(vram_fit_warning("whisper-tiny-fp32-cuda", "test").is_none());
        // Swapped table: device 0 large now fits Qwen.
        set_test_gpu_table(Some("0, 12288, 11264\n1, 4096, 3072\n".to_owned()));
        assert!(admission_for_model("qwen3-asr-1.7b-fp16-cuda", "test").is_ok());
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn selected_pair_never_mixes_devices() {
        // Total from device 0 + free from device 1 must never combine.
        let rows = parse_gpu_table("0, 8192, 1000\n1, 8192, 7000\n").unwrap();
        assert_eq!(rows.len(), 2);
        // Malformed rows yield honest unknown, never a partial max.
        assert!(parse_gpu_table("0, 8192\n1, 8192, 7000\n").is_none());
        assert!(parse_gpu_table("0, 0, 0\n").is_none());
        assert!(parse_gpu_table("").is_none());
    }

    #[test]
    fn auto_text_reserves_cpu_before_an_oversized_gpu_overlap() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        // Leave preferences untouched: test the locked accounting function.
        let hardware = probe_hardware();
        {
            let mut state = shared().lock().unwrap();
            state.asr_active = Some("qwen3-asr-1.7b-fp16-cuda".into());
            state.asr_pending_vram = 8400 * MIB;
        }
        let gpu = Some((12288 * MIB, 10240 * MIB));
        let result = try_acquire_text_locked(
            "cleanup",
            "qwen3-4b-instruct-2507",
            "test",
            &hardware,
            gpu,
            generation_blocking(),
            true,
        );
        assert!(matches!(result, LockedAdmission::Admitted(_)));
        if !crate::acceleration::cpu_only() {
            assert!(text_cpu_required("qwen3-4b-instruct-2507"));
        }
        assert_eq!(shared().lock().unwrap().text_pending_vram, 0);
        notify_text_finished();
        assert!(!text_cpu_required("qwen3-4b-instruct-2507"));
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn unknown_selected_device_does_not_borrow_another_cards_capacity() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        set_test_gpu_table(Some("1, 12288, 11264\n".into()));
        assert_eq!(selected_gpu_pair(), None);
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn critical_host_pressure_refuses_without_leaving_a_lease() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        set_test_memory(32 << 30, 64 * MIB);
        let error = acquire_asr_with_admission("whisper-tiny", "test").unwrap_err();
        assert!(error.starts_with(MEMORY_UNAVAILABLE));
        assert!(!model_state_blocking("whisper-tiny").0);
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    /// Field failure 2026-10-14 (Mac mini, 16 GB, whisper-small-metal):
    /// reusing the resident speech model under host pressure must neither
    /// mark it for release nor wait on the owner that is asking, and even
    /// critically low RAM only warns because reuse allocates no model bytes.
    #[test]
    fn resident_models_under_pressure_are_admitted_and_not_evicted() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        acquire_asr("whisper-small-metal").unwrap();
        note_asr_loaded("whisper-small-metal");
        notify_asr_finished();
        acquire_text("cleanup", "lfm2.5-1.2b").unwrap();
        note_text_loaded("lfm2.5-1.2b");
        notify_text_finished();
        for available in [1 << 30, 64 * MIB] {
            set_test_memory(16 << 30, available);
            let started = Instant::now();
            acquire_asr_with_admission("whisper-small-metal", "dictation").unwrap();
            // No pressure wait: the old path slept up to 1 s for its own release.
            assert!(started.elapsed() < Duration::from_millis(500));
            assert!(!asr_release_pending_blocking());
            assert!(model_state_blocking("whisper-small-metal").1);
            assert_eq!(shared().lock().unwrap().asr_pending_host, 0);
            notify_asr_finished();
            acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "dictation cleanup").unwrap();
            // A resident text reuse never evicts the idle speech model either.
            assert!(!asr_release_pending_blocking());
            assert!(model_state_blocking("lfm2.5-1.2b").1);
            notify_text_finished();
            assert!(admission_check_model("lfm2.5-1.2b", 3 << 30, "legacy").is_ok());
            assert!(!asr_release_pending_blocking());
        }
        // A model that is not resident still faces the critical-RAM refusal.
        set_test_memory(16 << 30, 64 * MIB);
        assert!(acquire_asr_with_admission("whisper-turbo-metal", "dictation")
            .unwrap_err()
            .starts_with(MEMORY_UNAVAILABLE));
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn granite_meeting_registration_reserves_and_releases_the_shared_asr_stage() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        set_test_memory(32 << 30, 24 << 30);
        let id = crate::granite_model::GRANITE_ID;
        assert!(estimated_ram_interval(id).unwrap().1 >= 3500 * MIB);
        assert!(estimate_evidence(id).contains("NAR-TRANSCRIBECPP"));
        set_test_gpu_table(Some("0, 12288, 10000\n".into()));
        acquire_asr_with_admission(id, "meeting").unwrap();
        assert!(acquire_asr_with_admission(crate::llama_asr::MODEL_ID, "dictation").is_err());
        {
            let state = shared().lock().unwrap();
            assert_eq!(state.asr_pending_id.as_deref(), Some(id));
            assert_eq!(state.asr_pending_host, estimated_ram_interval(id).unwrap().1);
            assert_eq!(state.asr_pending_vram, estimated_vram_interval(id).unwrap().1);
        }
        notify_asr_finished();
        assert_eq!(model_state_blocking(id), (false, false));
        assert_eq!(shared().lock().unwrap().asr_pending_vram, 0);
        if cfg!(windows) {
            set_test_gpu_table(Some("0, 8192, 4799\n".into()));
            assert!(acquire_asr_with_admission(id, "meeting").unwrap_err().starts_with(MEMORY_UNAVAILABLE));
        }
        clear_test_memory();
        set_test_gpu_table(None);
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn llama_asr_estimates_keep_host_and_device_memory_separate() {
        let id = crate::llama_asr::MODEL_ID;
        assert_eq!(estimated_ram_interval(id), Some((900 * MIB, 1600 * MIB)));
        assert_eq!(estimated_vram_interval(id), Some((3200 * MIB, 4200 * MIB)));
        assert!(estimate_basis(id).starts_with("Observed 2026-09-28"));
        assert!(estimate_evidence(id).starts_with("Observed 2026-09-28"));
        assert_eq!(
            plan_range(id, "lfm2.5-1.2b", MemoryMode::Overlap),
            Some((2250 * MIB + WORKING_OVERHEAD_BYTES, 3750 * MIB + WORKING_OVERHEAD_BYTES))
        );
        assert_eq!(
            plan_range(id, "lfm2.5-1.2b", MemoryMode::Sequential),
            Some((1350 * MIB + WORKING_OVERHEAD_BYTES, 2150 * MIB + WORKING_OVERHEAD_BYTES))
        );
    }

    #[test]
    fn llama_asr_admission_reserves_once_and_rejects_selected_device_pressure() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
        let id = crate::llama_asr::MODEL_ID;
        // Device 1 cannot compensate for insufficient free VRAM on device 0.
        set_test_gpu_table(Some("0, 8192, 4199\n1, 12288, 11264\n".into()));
        assert!(vram_fit_warning(id, "test").is_some());
        assert!(acquire_asr_with_admission(id, "test").unwrap_err().starts_with(MEMORY_UNAVAILABLE));
        assert_eq!(model_state_blocking(id), (false, false));
        assert_eq!(shared().lock().unwrap().asr_pending_vram, 0);

        set_test_gpu_table(Some("0, 8192, 4200\n".into()));
        acquire_asr_with_admission(id, "test").unwrap();
        {
            let state = shared().lock().unwrap();
            assert_eq!(state.asr_pending_id.as_deref(), Some(id));
            assert_eq!(state.asr_pending_host, 1600 * MIB);
            assert_eq!(state.asr_pending_vram, 4200 * MIB);
        }
        assert!(acquire_asr_with_admission(id, "test").unwrap_err().contains("already running"));
        note_asr_loaded(id);
        notify_asr_finished();
        set_test_gpu_table(Some("0, 8192, 0\n".into()));
        acquire_asr_with_admission(id, "test").unwrap();
        {
            let state = shared().lock().unwrap();
            assert_eq!(state.asr_pending_host, 0);
            assert_eq!(state.asr_pending_vram, 0);
        }
        notify_asr_finished();
        confirm_asr_released();
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }

    #[test]
    fn qwen_fits_observed_free_vram_without_a_second_headroom_reserve() {
        let _gate = test_gate();
        reset_for_tests(MemoryMode::Sequential, ModelRetention::Immediately);
        set_test_gpu_table(Some("0, 12227, 9488\n".into()));
        acquire_asr_with_admission("qwen3-asr-1.7b-fp16-cuda", "test").unwrap();
        assert!(model_state_blocking("qwen3-asr-1.7b-fp16-cuda").0);
        notify_asr_finished();
        set_test_gpu_table(Some("0, 12227, 8000\n".into()));
        assert!(
            acquire_asr_with_admission("qwen3-asr-1.7b-fp16-cuda", "test")
                .unwrap_err()
                .starts_with(MEMORY_UNAVAILABLE)
        );
        reset_for_tests(MemoryMode::Overlap, ModelRetention::Minutes10);
    }
}
