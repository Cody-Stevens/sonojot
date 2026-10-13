//! Local onboarding recommendations; inspecting never installs or saves models.
//! Call `inspect` from a blocking worker, not the UI thread.
use crate::domain::{MemoryMode, SetupProfile};
use crate::resource_manager;
use serde::Serialize;
use std::path::Path;

const GIB: u64 = 1 << 30;

// Policy thresholds, not measured guarantees. Evidence:
// rewrite/evaluation/BENCHMARK-WINDOWS-2026-09-28.md reports ~1.0 GB CUDA
// host RAM + 3.4 GB GPU, ~2.1 GB Qwen CPU, and ~0.56 GB Moonshine Medium.
// Six GiB VRAM leaves room beyond the CUDA speech estimate. Six physical
// cores (or twelve logical only when physical is unknown) and 16 GiB RAM
// are conservative CPU starting points from one 12-core workstation.
// Only Moonshine provides continuous live partials among these choices.
const CUDA_SPEECH_MIN_VRAM: u64 = 6 * GIB;
const STRONG_CPU_MIN_PHYSICAL: u32 = 6;
const STRONG_CPU_MIN_LOGICAL: u32 = 12;
const LARGE_MODEL_MIN_RAM: u64 = 16 * GIB;
// Leave 55% for Windows/macOS, other apps and uncertainty in loading peaks.
// This recommendation policy does not change runtime admission safeguards.
const MAX_PLANNED_RAM_PERCENT: u128 = 45;

const QWEN_CUDA: &str = "qwen3-asr-1.7b-q8-cuda";
const QWEN_CPU: &str = "qwen3-asr-1.7b-q4-cpu";
const QWEN_SMALL_CPU: &str = crate::llama_asr::SMALL_CPU_MODEL_ID;
const QWEN_Q8_METAL: &str = crate::llama_asr::METAL_Q8_MODEL_ID;
const QWEN_Q4_METAL: &str = crate::llama_asr::METAL_Q4_MODEL_ID;
const MOONSHINE_MEDIUM: &str = "moonshine-medium";
const MOONSHINE_SMALL: &str = "moonshine-small";
const WHISPER_SMALL_METAL: &str = "whisper-small-metal";
const QWEN_NOTES: &str = "qwen3-4b-instruct-2507";
// Qwen3-4B is the smallest Apache/MIT Notes model in the current catalog.
// Smaller Qwen instruct sizes would need new pins in the protected catalog.
const LIGHT_NOTES: &str = QWEN_NOTES;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub platform: String,
    pub apple_silicon: bool,
    pub total_ram_bytes: Option<u64>,
    pub physical_cores: Option<u32>,
    pub logical_cores: Option<u32>,
    pub gpu_name: Option<String>,
    pub gpu_driver_version: Option<String>,
    pub gpu_compute_capability: Option<String>,
    /// Dedicated memory only; Apple Silicon uses `total_ram_bytes` once.
    pub total_vram_bytes: Option<u64>,
    pub cuda_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub asr_model: String,
    pub cleanup_model: String,
    pub notes_model: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendations {
    pub accurate: Selection,
    pub balanced: Selection,
    pub light: Selection,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfiles {
    pub hardware: HardwareProfile,
    pub recommendations: Recommendations,
}

pub fn inspect(models_root: &Path) -> HardwareProfiles {
    let probe = resource_manager::setup_hardware();
    let mut hardware = HardwareProfile {
        platform: if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "unknown"
        }.into(),
        apple_silicon: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        total_ram_bytes: probe.total_ram_bytes,
        physical_cores: probe.physical_cores,
        logical_cores: probe.logical_cores,
        gpu_name: probe.gpu.as_ref().and_then(|gpu| gpu.name.clone()),
        gpu_driver_version: probe.gpu.as_ref().and_then(|gpu| gpu.driver_version.clone()),
        gpu_compute_capability: probe.gpu.as_ref().and_then(|gpu| gpu.compute_capability.clone()),
        total_vram_bytes: probe.gpu.as_ref().and_then(|gpu| gpu.total_bytes),
        cuda_ok: false,
    };
    // Reuse acceleration's hidden, cached --list-devices probe. The bundled
    // CUDA 13.3 runtime must actually accept the driver/device; driver branch
    // or compute-capability guesses must not replace the app's existing rule.
    if cfg!(all(target_os = "windows", target_arch = "x86_64"))
        && probe.gpu.is_some()
        && crate::cleanup::cuda_server_path(models_root).is_file()
    {
        let support = crate::acceleration::gpu_support(models_root);
        hardware.cuda_ok = support.available && support.backend == "cuda";
    }
    profiles_for(hardware)
}

fn profiles_for(hardware: HardwareProfile) -> HardwareProfiles {
    let recommendations = Recommendations {
        accurate: recommend(&hardware, SetupProfile::Accurate),
        balanced: recommend(&hardware, SetupProfile::Balanced),
        light: recommend(&hardware, SetupProfile::Light),
    };
    HardwareProfiles { hardware, recommendations }
}

fn strong_cpu(hardware: &HardwareProfile) -> bool {
    let enough_cores = match hardware.physical_cores.filter(|cores| *cores > 0) {
        Some(physical) => physical >= STRONG_CPU_MIN_PHYSICAL,
        None => hardware.logical_cores.is_some_and(|cores| cores >= STRONG_CPU_MIN_LOGICAL),
    };
    enough_cores && hardware.total_ram_bytes.is_some_and(|ram| ram >= LARGE_MODEL_MIN_RAM)
}

/// Pure recommendation only. Custom returns the Balanced suggestion with an
/// explanation; applying Custom must preserve the user's actual selections.
pub fn recommend(hardware: &HardwareProfile, profile: SetupProfile) -> Selection {
    let custom = profile == SetupProfile::Custom;
    let profile = if custom { SetupProfile::Balanced } else { profile };
    let mut reasons = Vec::new();
    let asr_model = if hardware.platform == "macos" && hardware.apple_silicon {
        reasons.push("Apple Silicon shares memory between speech and the rest of the app.".into());
        match profile {
            SetupProfile::Accurate => QWEN_Q8_METAL,
            SetupProfile::Light => {
                // Moonshine is unavailable on Mac; Whisper Small keeps the
                // fast-start option with the lowest speech planning range.
                reasons.push("Whisper Small starts quickly and uses the least speech memory on this Mac.".into());
                WHISPER_SMALL_METAL
            }
            _ => QWEN_Q4_METAL,
        }
    } else if hardware.platform == "windows" {
        let cuda_speech = hardware.cuda_ok
            && hardware.total_vram_bytes.is_some_and(|vram| vram >= CUDA_SPEECH_MIN_VRAM);
        let strong = strong_cpu(hardware);
        if cuda_speech {
            if profile == SetupProfile::Light {
                MOONSHINE_MEDIUM
            } else {
                reasons.push("The NVIDIA GPU meets the memory and compatibility checks for Qwen speech.".into());
                QWEN_CUDA
            }
        } else {
            if hardware.cuda_ok && hardware.total_vram_bytes.is_none() {
                reasons.push("GPU memory is unknown; speech will use the CPU.".into());
            } else if hardware.cuda_ok && hardware.total_vram_bytes.is_some_and(|vram| vram < CUDA_SPEECH_MIN_VRAM) {
                reasons.push("Qwen GPU speech needs at least 6 GiB of GPU memory.".into());
            } else if hardware.gpu_name.is_some() || hardware.gpu_driver_version.is_some() {
                reasons.push("CUDA 13.3 compatibility or its runtime could not be confirmed; speech will use the CPU.".into());
            }
            match (strong, profile) {
                (true, SetupProfile::Accurate | SetupProfile::Balanced) => {
                    reasons.push("The CPU and RAM meet the recommendation for Qwen speech.".into());
                    QWEN_CPU
                }
                (true, _) => MOONSHINE_MEDIUM,
                (false, SetupProfile::Accurate) => {
                    reasons.push("Qwen speech is slower on this computer.".into());
                    QWEN_CPU
                }
                // Without a known RAM size the memory guard cannot check Qwen,
                // so Balanced keeps the lighter streaming model.
                (false, SetupProfile::Balanced) if hardware.total_ram_bytes.is_some() => {
                    reasons.push("The smaller Qwen model uses less memory and suits fewer CPU cores.".into());
                    QWEN_SMALL_CPU
                }
                (false, SetupProfile::Balanced) => MOONSHINE_MEDIUM,
                _ => MOONSHINE_SMALL,
            }
        }
    } else {
        // No supported engine is established for Intel Macs or other targets.
        // An empty speech ID fails settings validation instead of guessing a
        // Windows package that cannot be installed on this platform.
        reasons.push("Automatic speech setup is unavailable on this platform. Choose Custom to review model support.".into());
        ""
    };
    let notes_model = if profile != SetupProfile::Light
        && hardware.total_ram_bytes.is_some_and(|ram| ram >= LARGE_MODEL_MIN_RAM)
    {
        QWEN_NOTES
    } else {
        LIGHT_NOTES
    };
    let mut selection = Selection {
        asr_model: asr_model.into(),
        cleanup_model: "local-rules".into(),
        notes_model: notes_model.into(),
        reasons,
    };
    apply_memory_guard(hardware, &mut selection);
    if matches!(selection.asr_model.as_str(), MOONSHINE_MEDIUM | MOONSHINE_SMALL) {
        selection.reasons.push("Moonshine uses less memory and shows words while you speak.".into());
    }
    if selection.notes_model == LIGHT_NOTES {
        selection.reasons.push("Qwen3 keeps meeting notes local under Apache-2.0.".into());
    }
    if custom {
        selection.reasons.push("Custom keeps your choices; this is only the Balanced suggestion.".into());
    }
    selection
}

fn planned_peak(selection: &Selection) -> Option<u64> {
    // Cleanup and Notes share one worker. Cleanup is local-rules in every
    // profile, so the larger text resident is always the Notes model.
    resource_manager::plan_range(&selection.asr_model, &selection.notes_model, MemoryMode::Overlap)
        .map(|(_, peak)| peak)
}

fn exceeds_memory_budget(peak: u64, ram: u64) -> bool {
    u128::from(peak) * 100 > u128::from(ram) * MAX_PLANNED_RAM_PERCENT
}

fn apply_memory_guard(hardware: &HardwareProfile, selection: &mut Selection) {
    let Some(ram) = hardware.total_ram_bytes.filter(|ram| *ram > 0) else {
        selection.reasons.push("RAM capacity is unknown; the memory estimate could not be checked.".into());
        return;
    };
    if !planned_peak(selection).is_some_and(|peak| exceeds_memory_budget(peak, ram)) {
        return;
    }
    if matches!(selection.asr_model.as_str(), QWEN_CPU | QWEN_SMALL_CPU) {
        selection.asr_model = MOONSHINE_MEDIUM.into();
        selection.reasons.push("Moonshine replaces Qwen speech because the memory plan exceeds 45% of RAM.".into());
    }
    // No smaller permissive Notes package is currently pinned. Keep Notes
    // on Qwen3 and use the existing speech fallback to reduce memory.
    if hardware.platform == "macos" && hardware.apple_silicon
        && crate::llama_asr::is_metal(&selection.asr_model)
        && planned_peak(selection).is_some_and(|peak| exceeds_memory_budget(peak, ram))
    {
        selection.asr_model = WHISPER_SMALL_METAL.into();
        selection.reasons.push("Whisper Small replaces Qwen speech because the memory plan with Qwen3 notes exceeds 45% of unified memory.".into());
    }
    if planned_peak(selection).is_some_and(|peak| exceeds_memory_budget(peak, ram)) {
        selection.reasons.push("Even this selection may use more than 45% of RAM; close other apps or run models one at a time.".into());
    }
}

#[cfg(test)]
mod profiles_tests {
    use super::*;

    fn windows(ram_gib: u64, physical: Option<u32>, logical: Option<u32>) -> HardwareProfile {
        HardwareProfile {
            platform: "windows".into(),
            apple_silicon: false,
            total_ram_bytes: Some(ram_gib * GIB),
            physical_cores: physical,
            logical_cores: logical,
            gpu_name: None,
            gpu_driver_version: None,
            gpu_compute_capability: None,
            total_vram_bytes: None,
            cuda_ok: false,
        }
    }

    fn nvidia() -> HardwareProfile {
        HardwareProfile {
            gpu_name: Some("NVIDIA GeForce RTX 5070".into()),
            gpu_driver_version: Some("610.88".into()),
            gpu_compute_capability: Some("12.0".into()),
            total_vram_bytes: Some(12 * GIB),
            cuda_ok: true,
            ..windows(32, Some(12), Some(24))
        }
    }

    fn apple(ram_gib: u64) -> HardwareProfile {
        HardwareProfile {
            platform: "macos".into(),
            apple_silicon: true,
            ..windows(ram_gib, Some(8), Some(8))
        }
    }

    fn assert_models(hardware: &HardwareProfile, speech: [&str; 3], notes: [&str; 3]) {
        for (index, profile) in [SetupProfile::Accurate, SetupProfile::Balanced, SetupProfile::Light].into_iter().enumerate() {
            let selection = recommend(hardware, profile);
            assert_eq!(selection.asr_model, speech[index], "{profile:?}: {hardware:?}");
            assert_eq!(selection.cleanup_model, "local-rules");
            assert_eq!(selection.notes_model, notes[index], "{profile:?}: {hardware:?}");
            assert!(!selection.notes_model.starts_with("lfm"));
            assert!(!selection.reasons.is_empty());
            assert!(![&selection.asr_model, &selection.cleanup_model, &selection.notes_model].iter().any(|id| id.contains("openrouter")));
        }
        let balanced = recommend(hardware, SetupProfile::Balanced);
        let custom = recommend(hardware, SetupProfile::Custom);
        assert_eq!(custom.asr_model, balanced.asr_model);
        assert_eq!(custom.cleanup_model, balanced.cleanup_model);
        assert_eq!(custom.notes_model, balanced.notes_model);
        assert!(custom.reasons.last().unwrap().contains("Custom keeps your choices"));
    }

    #[test]
    fn profiles_tier_table_covers_every_profile() {
        let mut unknown_cores = windows(32, None, None);
        unknown_cores.total_ram_bytes = None;
        let cases = [
            (nvidia(), [QWEN_CUDA, QWEN_CUDA, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, Some(6), Some(6)), [QWEN_CPU, QWEN_CPU, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, Some(12)), [QWEN_CPU, QWEN_CPU, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, Some(4), Some(16)), [QWEN_CPU, QWEN_SMALL_CPU, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, Some(11)), [QWEN_CPU, QWEN_SMALL_CPU, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, None), [QWEN_CPU, QWEN_SMALL_CPU, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(12, Some(4), Some(8)), [MOONSHINE_MEDIUM, MOONSHINE_MEDIUM, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
            (apple(32), [QWEN_Q8_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (apple(16), [WHISPER_SMALL_METAL; 3], [LIGHT_NOTES; 3]),
            (apple(12), [WHISPER_SMALL_METAL; 3], [LIGHT_NOTES; 3]),
            (apple(8), [WHISPER_SMALL_METAL; 3], [LIGHT_NOTES; 3]),
            (unknown_cores, [QWEN_CPU, MOONSHINE_MEDIUM, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
        ];
        for (hardware, speech, notes) in cases {
            assert_models(&hardware, speech, notes);
        }
        assert!(recommend(&windows(12, Some(4), Some(8)), SetupProfile::Accurate)
            .reasons.iter().any(|reason| reason.contains("slower on this computer")));
    }

    #[test]
    fn profiles_memory_guard_uses_existing_overlap_peak() {
        let hardware = windows(16, Some(6), Some(12));
        let peak = resource_manager::plan_range(QWEN_CPU, QWEN_NOTES, MemoryMode::Overlap).unwrap().1;
        assert!(exceeds_memory_budget(peak, hardware.total_ram_bytes.unwrap()));
        assert_models(&hardware, [MOONSHINE_MEDIUM; 3], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]);
        let selection = recommend(&hardware, SetupProfile::Accurate);
        assert!(selection.reasons.iter().any(|reason| reason.contains("replaces Qwen speech")));
        assert!(!selection.reasons.iter().any(|reason| reason.contains("replaces Qwen notes")));
        assert!(!exceeds_memory_budget(planned_peak(&selection).unwrap(), 16 * GIB));
        assert_models(&windows(8, Some(4), Some(8)), [MOONSHINE_MEDIUM, MOONSHINE_MEDIUM, MOONSHINE_SMALL], [LIGHT_NOTES; 3]);
        let tiny = recommend(&windows(2, Some(2), Some(4)), SetupProfile::Light);
        assert!(tiny.reasons.iter().any(|reason| reason.contains("Even this selection")));
        assert!(!exceeds_memory_budget(45, 100));
        assert!(exceeds_memory_budget(46, 100));
        assert!(exceeds_memory_budget(u64::MAX, u64::MAX));
    }

    #[test]
    fn profiles_apple_memory_is_counted_once_and_moonshine_is_unavailable() {
        for id in [QWEN_Q8_METAL, QWEN_Q4_METAL] {
            assert!(crate::llama_asr::find(id).is_some());
            assert!(crate::llama_asr::is_metal(id));
            assert_eq!(resource_manager::estimated_vram_interval(id), Some((0, 0)));
        }
        assert!(crate::mac_speech::find(WHISPER_SMALL_METAL).is_some());
        assert!(crate::mac_speech::find(MOONSHINE_MEDIUM).is_none());
        assert!(apple(16).total_vram_bytes.is_none());
    }

    #[test]
    fn profiles_apple_guard_keeps_notes_permissive_and_uses_existing_speech_fallback() {
        for (profile, speech) in [(SetupProfile::Accurate, QWEN_Q8_METAL), (SetupProfile::Balanced, QWEN_Q4_METAL)] {
            let initial_peak = resource_manager::plan_range(speech, QWEN_NOTES, MemoryMode::Overlap).unwrap().1;
            assert!(exceeds_memory_budget(initial_peak, 16 * GIB));
            let selection = recommend(&apple(16), profile);
            assert_eq!(selection.asr_model, WHISPER_SMALL_METAL);
            assert_eq!(selection.notes_model, LIGHT_NOTES);
            assert!(!selection.notes_model.starts_with("lfm"));
            assert!(selection.reasons.iter().any(|reason| reason.contains("replaces Qwen speech")));
            assert!(!exceeds_memory_budget(planned_peak(&selection).unwrap(), 16 * GIB));

            let small = recommend(&apple(8), profile);
            assert_eq!(small.asr_model, WHISPER_SMALL_METAL);
            assert_eq!(small.notes_model, LIGHT_NOTES);
            assert!(small.reasons.iter().any(|reason| reason.contains("Whisper Small replaces Qwen speech")));
            // No lighter supported Mac package: keep the residual warning.
            assert!(exceeds_memory_budget(planned_peak(&small).unwrap(), 8 * GIB));
            assert!(small.reasons.iter().any(|reason| reason.contains("Even this selection")));
        }
    }

    #[test]
    fn profiles_apple_guard_uses_actual_plan_boundary_and_keeps_unknown_ram_unknown() {
        for (profile, speech) in [(SetupProfile::Accurate, QWEN_Q8_METAL), (SetupProfile::Balanced, QWEN_Q4_METAL)] {
            let peak = resource_manager::plan_range(speech, LIGHT_NOTES, MemoryMode::Overlap).unwrap().1;
            let minimum_ram = (u128::from(peak) * 100).div_ceil(MAX_PLANNED_RAM_PERCENT) as u64;
            assert!(minimum_ram > LARGE_MODEL_MIN_RAM);
            for (ram, expected) in [(minimum_ram - 1, WHISPER_SMALL_METAL), (minimum_ram, speech)] {
                let hardware = HardwareProfile { total_ram_bytes: Some(ram), ..apple(16) };
                assert_eq!(recommend(&hardware, profile).asr_model, expected);
            }
            for ram in [None, Some(0)] {
                let hardware = HardwareProfile { total_ram_bytes: ram, ..apple(16) };
                let selection = recommend(&hardware, profile);
                assert_eq!(selection.asr_model, speech);
                assert_eq!(selection.notes_model, LIGHT_NOTES);
                assert!(selection.reasons.iter().any(|reason| reason.contains("RAM capacity is unknown")));
            }
        }
    }

    #[test]
    fn profiles_unknown_or_small_vram_never_selects_cuda() {
        for vram in [None, Some(0), Some(6 * GIB - 1)] {
            let hardware = HardwareProfile { total_vram_bytes: vram, ..nvidia() };
            assert_models(&hardware, [QWEN_CPU, QWEN_CPU, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]);
        }
        let boundary = HardwareProfile { total_vram_bytes: Some(6 * GIB), ..nvidia() };
        assert_eq!(recommend(&boundary, SetupProfile::Accurate).asr_model, QWEN_CUDA);
    }

    #[test]
    fn profiles_cuda_tier_requires_runtime_compatibility() {
        for hardware in [
            HardwareProfile { cuda_ok: false, ..nvidia() },
            HardwareProfile { gpu_name: Some("AMD Radeon".into()), ..windows(32, Some(6), Some(12)) },
        ] {
            assert_eq!(recommend(&hardware, SetupProfile::Balanced).asr_model, QWEN_CPU);
        }
        // Metadata alone never proves compatibility. Conversely, a runtime
        // that confirms CUDA support does not need a guessed driver threshold.
        let confirmed = HardwareProfile {
            gpu_driver_version: None,
            gpu_compute_capability: None,
            ..nvidia()
        };
        assert_eq!(recommend(&confirmed, SetupProfile::Balanced).asr_model, QWEN_CUDA);
    }

    #[test]
    fn profiles_unknown_ram_and_cores_are_not_guessed() {
        let unknown = HardwareProfile { total_ram_bytes: None, ..windows(32, None, None) };
        let selection = recommend(&unknown, SetupProfile::Balanced);
        assert_eq!(selection.asr_model, MOONSHINE_MEDIUM);
        assert_eq!(selection.notes_model, LIGHT_NOTES);
        assert!(selection.reasons.iter().any(|reason| reason.contains("RAM capacity is unknown")));
        assert!(!strong_cpu(&windows(32, Some(5), Some(24))));
        assert!(!strong_cpu(&windows(15, Some(12), Some(24))));
        assert!(strong_cpu(&windows(16, Some(6), None)));
        for platform in ["unknown", "macos"] {
            let unsupported = HardwareProfile { platform: platform.into(), ..windows(32, Some(8), Some(16)) };
            assert_models(&unsupported, [""; 3], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]);
            assert!(recommend(&unsupported, SetupProfile::Accurate).reasons.iter().any(|reason| reason.contains("unavailable on this platform")));
        }
    }

    #[test]
    fn profiles_contract_serializes_camel_case_and_null_unknowns() {
        let value = serde_json::to_value(profiles_for(windows(32, None, None))).unwrap();
        assert_eq!(value["hardware"]["platform"], "windows");
        assert_eq!(value["hardware"]["totalRamBytes"], 32 * GIB);
        assert!(value["hardware"]["physicalCores"].is_null());
        assert!(value["hardware"]["totalVramBytes"].is_null());
        assert_eq!(value["hardware"]["cudaOk"], false);
        assert_eq!(value["recommendations"]["balanced"]["cleanupModel"], "local-rules");
        assert!(value["recommendations"]["accurate"]["reasons"].is_array());
        assert_eq!(QWEN_CUDA, crate::llama_asr::MODEL_ID);
        assert_eq!(QWEN_CPU, crate::llama_asr::CPU_MODEL_ID);
    }
}
