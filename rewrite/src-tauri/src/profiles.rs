//! Local onboarding recommendations; inspecting never installs or saves models.
//! Call `inspect` from a blocking worker, not the UI thread.
use crate::domain::SetupProfile;
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
const PARAKEET: &str = "parakeet-tdt-0.6b-v2";
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
        } else if cfg!(target_os = "linux") {
            "linux"
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
    } else if matches!(hardware.platform.as_str(), "windows" | "linux") {
        let cuda_speech = hardware.platform == "windows" && hardware.cuda_ok
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
            } else if hardware.platform == "windows" && (hardware.gpu_name.is_some() || hardware.gpu_driver_version.is_some()) {
                reasons.push("CUDA 13.3 compatibility or its runtime could not be confirmed; speech will use the CPU.".into());
            }
            // CPU-only Balanced uses Parakeet: on a 4-core CPU (October 15, 2026)
            // it scored 3.3% word errors on one-minute TED clips against 2.9%
            // for Qwen 1.7B and 3.7% for Qwen 0.6B, and finished 0.15 s after
            // speech ended against 2.0 s and 1.5 s. Accurate keeps Qwen 1.7B.
            match (strong, profile) {
                (true, SetupProfile::Accurate) => {
                    reasons.push("The CPU and RAM meet the recommendation for Qwen speech.".into());
                    QWEN_CPU
                }
                (false, SetupProfile::Accurate) => {
                    reasons.push("Qwen speech is slower on this computer.".into());
                    QWEN_CPU
                }
                // Without a known RAM size the memory guard cannot check
                // Parakeet, so Balanced keeps the lighter streaming model.
                (_, SetupProfile::Balanced) if hardware.total_ram_bytes.is_some() => {
                    reasons.push("Parakeet is nearly as accurate as Qwen on the CPU and finishes moments after you stop speaking.".into());
                    PARAKEET
                }
                (_, SetupProfile::Balanced) => MOONSHINE_MEDIUM,
                (true, _) => MOONSHINE_MEDIUM,
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

/// Upper-bound memory of the models that are resident together.
///
/// Dictation keeps speech, the Cleanup model (none for local rules) and the
/// warm punctuation helper loaded at once; the helper prewarms while speech
/// records. Notes loads only while notes are processed. Cleanup and Notes
/// share one text worker, and runtime admission releases idle speech before
/// loading Notes under memory pressure. The two sets are therefore planned
/// separately rather than summed. The overlap sum (`plan_range` Overlap)
/// overstated a 16 GiB Mac's peak by the whole speech model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemoryPlan {
    dictation: u64,
    notes: u64,
}

fn memory_plan(selection: &Selection) -> Option<MemoryPlan> {
    let peak = |id: &str| resource_manager::estimated_ram_interval(id).map(|(_, max)| max);
    let overhead = resource_manager::WORKING_OVERHEAD_BYTES;
    Some(MemoryPlan {
        dictation: peak(&selection.asr_model)?
            .saturating_add(peak(&selection.cleanup_model)?)
            .saturating_add(crate::punctuation_model::RAM_ESTIMATE_BYTES)
            .saturating_add(overhead),
        notes: peak(&selection.notes_model)?.saturating_add(overhead),
    })
}

fn exceeds_memory_budget(peak: u64, ram: u64) -> bool {
    u128::from(peak) * 100 > u128::from(ram) * MAX_PLANNED_RAM_PERCENT
}

fn dictation_exceeds(selection: &Selection, ram: u64) -> bool {
    memory_plan(selection).is_some_and(|plan| exceeds_memory_budget(plan.dictation, ram))
}

/// The next lighter speech model, in measured accuracy order. Qwen steps
/// down within its family before leaving it.
fn lighter_speech(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        QWEN_CPU => Some((QWEN_SMALL_CPU, "Qwen 0.6B replaces Qwen 1.7B")),
        QWEN_SMALL_CPU => Some((PARAKEET, "Parakeet replaces Qwen speech")),
        PARAKEET => Some((MOONSHINE_MEDIUM, "Moonshine replaces Parakeet")),
        QWEN_Q8_METAL => Some((QWEN_Q4_METAL, "Qwen Q4 replaces Q8")),
        QWEN_Q4_METAL => Some((WHISPER_SMALL_METAL, "Whisper Small replaces Qwen speech")),
        _ => None,
    }
}

fn apply_memory_guard(hardware: &HardwareProfile, selection: &mut Selection) {
    let Some(ram) = hardware.total_ram_bytes.filter(|ram| *ram > 0) else {
        selection.reasons.push("RAM capacity is unknown; the memory estimate could not be checked.".into());
        return;
    };
    let memory = if hardware.apple_silicon { "unified memory" } else { "RAM" };
    // Only the dictation set depends on the speech choice. A smaller speech
    // model cannot reduce the Notes peak, so Notes never forces a downgrade.
    while dictation_exceeds(selection, ram) {
        let Some((lighter, replacement)) = lighter_speech(&selection.asr_model) else { break };
        selection.asr_model = lighter.into();
        selection.reasons.push(format!("{replacement} because dictation memory would exceed 45% of {memory}."));
    }
    let Some(plan) = memory_plan(selection) else { return };
    if exceeds_memory_budget(plan.dictation, ram) {
        selection.reasons.push(format!("Even this selection may use more than 45% of {memory} while dictating; close other apps."));
    }
    // No smaller permissive Notes package is currently pinned.
    if exceeds_memory_budget(plan.notes, ram) {
        selection.reasons.push(format!("Notes may use more than 45% of {memory} while it runs; close other apps before processing notes."));
    }
}

#[cfg(test)]
mod profiles_tests {
    use super::*;
    use crate::domain::MemoryMode;

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
            (windows(32, Some(6), Some(6)), [QWEN_CPU, PARAKEET, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, Some(12)), [QWEN_CPU, PARAKEET, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, Some(4), Some(16)), [QWEN_CPU, PARAKEET, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, Some(11)), [QWEN_CPU, PARAKEET, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(32, None, None), [QWEN_CPU, PARAKEET, MOONSHINE_SMALL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (HardwareProfile { total_ram_bytes: Some(16 * GIB), ..nvidia() }, [QWEN_CUDA, QWEN_CUDA, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(16, Some(6), Some(12)), [QWEN_CPU, PARAKEET, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (windows(12, Some(4), Some(8)), [QWEN_CPU, PARAKEET, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
            (windows(8, Some(4), Some(8)), [QWEN_CPU, PARAKEET, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
            (windows(7, Some(4), Some(8)), [QWEN_SMALL_CPU, PARAKEET, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
            (windows(6, Some(4), Some(8)), [PARAKEET, PARAKEET, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
            (apple(32), [QWEN_Q8_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (apple(24), [QWEN_Q8_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (apple(16), [QWEN_Q8_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]),
            (apple(12), [QWEN_Q8_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [LIGHT_NOTES; 3]),
            (apple(8), [QWEN_Q4_METAL, QWEN_Q4_METAL, WHISPER_SMALL_METAL], [LIGHT_NOTES; 3]),
            (unknown_cores, [QWEN_CPU, MOONSHINE_MEDIUM, MOONSHINE_SMALL], [LIGHT_NOTES; 3]),
        ];
        for (hardware, speech, notes) in cases {
            assert_models(&hardware, speech, notes);
        }
        assert!(recommend(&windows(12, Some(4), Some(8)), SetupProfile::Accurate)
            .reasons.iter().any(|reason| reason.contains("slower on this computer")));
    }

    #[test]
    fn profiles_memory_plan_separates_dictation_from_notes() {
        // The old overlap sum counted Notes as resident during dictation.
        for (speech, ram) in [(QWEN_CPU, 16 * GIB), (QWEN_Q8_METAL, 16 * GIB), (QWEN_Q4_METAL, 16 * GIB)] {
            let overlap = resource_manager::plan_range(speech, QWEN_NOTES, MemoryMode::Overlap).unwrap().1;
            assert!(exceeds_memory_budget(overlap, ram), "{speech}");
            let selection = Selection {
                asr_model: speech.into(),
                cleanup_model: "local-rules".into(),
                notes_model: QWEN_NOTES.into(),
                reasons: Vec::new(),
            };
            let plan = memory_plan(&selection).unwrap();
            let speech_max = resource_manager::estimated_ram_interval(speech).unwrap().1;
            let notes_max = resource_manager::estimated_ram_interval(QWEN_NOTES).unwrap().1;
            let overhead = resource_manager::WORKING_OVERHEAD_BYTES;
            assert_eq!(plan.dictation, speech_max + crate::punctuation_model::RAM_ESTIMATE_BYTES + overhead);
            assert_eq!(plan.notes, notes_max + overhead);
            assert!(!exceeds_memory_budget(plan.dictation, ram), "{speech}");
            assert!(!exceeds_memory_budget(plan.notes, ram), "{speech}");
        }
        // A generative Cleanup model is resident during dictation; it counts.
        let with_cleanup = Selection {
            asr_model: QWEN_CPU.into(),
            cleanup_model: "lfm2.5-1.2b".into(),
            notes_model: QWEN_NOTES.into(),
            reasons: Vec::new(),
        };
        let rules = Selection { cleanup_model: "local-rules".into(), ..with_cleanup.clone() };
        assert_eq!(
            memory_plan(&with_cleanup).unwrap().dictation - memory_plan(&rules).unwrap().dictation,
            resource_manager::estimated_ram_interval("lfm2.5-1.2b").unwrap().1,
        );
        assert!(!exceeds_memory_budget(45, 100));
        assert!(exceeds_memory_budget(46, 100));
        assert!(exceeds_memory_budget(u64::MAX, u64::MAX));
    }

    #[test]
    fn profiles_memory_guard_steps_speech_down_only_for_dictation() {
        let qwen = recommend(&windows(16, Some(6), Some(12)), SetupProfile::Accurate);
        assert_eq!(qwen.asr_model, QWEN_CPU);
        assert!(!qwen.reasons.iter().any(|reason| reason.contains("replaces") || reason.contains("45%")));

        let smaller = recommend(&windows(7, Some(4), Some(8)), SetupProfile::Accurate);
        assert_eq!(smaller.asr_model, QWEN_SMALL_CPU);
        assert!(smaller.reasons.iter().any(|reason| reason == "Qwen 0.6B replaces Qwen 1.7B because dictation memory would exceed 45% of RAM."));
        let parakeet = recommend(&windows(6, Some(4), Some(8)), SetupProfile::Accurate);
        assert_eq!(parakeet.asr_model, PARAKEET);
        assert!(parakeet.reasons.iter().any(|reason| reason.starts_with("Parakeet replaces Qwen speech")));
        let moonshine = recommend(&windows(4, Some(4), Some(8)), SetupProfile::Balanced);
        assert_eq!(moonshine.asr_model, MOONSHINE_MEDIUM);
        assert!(moonshine.reasons.iter().any(|reason| reason.starts_with("Moonshine replaces Parakeet")));

        // 8 GiB fits Qwen dictation but not Notes. Notes is reported, never
        // traded for a worse speech model.
        let notes_heavy = recommend(&windows(8, Some(4), Some(8)), SetupProfile::Accurate);
        assert_eq!(notes_heavy.asr_model, QWEN_CPU);
        assert!(notes_heavy.reasons.iter().any(|reason| reason.starts_with("Notes may use more than 45% of RAM")));
        assert!(!notes_heavy.reasons.iter().any(|reason| reason.contains("Even this selection")));

        let tiny = recommend(&windows(2, Some(2), Some(4)), SetupProfile::Light);
        assert!(tiny.reasons.iter().any(|reason| reason.contains("Even this selection")));
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
    fn profiles_apple_16_gib_keeps_qwen_and_8_gib_keeps_qwen_q4() {
        for (profile, speech) in [(SetupProfile::Accurate, QWEN_Q8_METAL), (SetupProfile::Balanced, QWEN_Q4_METAL)] {
            for ram in [16, 24] {
                let selection = recommend(&apple(ram), profile);
                assert_eq!(selection.asr_model, speech, "{ram} GiB");
                assert_eq!(selection.cleanup_model, "local-rules");
                assert_eq!(selection.notes_model, LIGHT_NOTES);
                assert!(!selection.reasons.iter().any(|reason| reason.contains("replaces") || reason.contains("45%")));
            }

            // Measured Q4 use (2.16-2.36 GB) fits 8 GiB; it beats Whisper Small.
            let small = recommend(&apple(8), profile);
            assert_eq!(small.asr_model, QWEN_Q4_METAL);
            assert_eq!(small.notes_model, LIGHT_NOTES);
            assert!(!small.reasons.iter().any(|reason| reason.starts_with("Whisper Small replaces")));
            // Q4 dictation fits; Notes alone still exceeds the budget.
            let plan = memory_plan(&small).unwrap();
            assert!(!exceeds_memory_budget(plan.dictation, 8 * GIB));
            assert!(exceeds_memory_budget(plan.notes, 8 * GIB));
            assert!(small.reasons.iter().any(|reason| reason.starts_with("Notes may use more than 45% of unified memory")));
            assert!(!small.reasons.iter().any(|reason| reason.contains("Even this selection")));
        }
        let accurate = recommend(&apple(8), SetupProfile::Accurate);
        assert!(accurate.reasons.iter().any(|reason| reason.starts_with("Qwen Q4 replaces Q8")));
    }

    #[test]
    fn profiles_apple_guard_uses_actual_plan_boundary_and_keeps_unknown_ram_unknown() {
        for (profile, speech, lighter) in [
            (SetupProfile::Accurate, QWEN_Q8_METAL, QWEN_Q4_METAL),
            (SetupProfile::Balanced, QWEN_Q4_METAL, WHISPER_SMALL_METAL),
        ] {
            let selection = Selection {
                asr_model: speech.into(),
                cleanup_model: "local-rules".into(),
                notes_model: LIGHT_NOTES.into(),
                reasons: Vec::new(),
            };
            let peak = memory_plan(&selection).unwrap().dictation;
            let minimum_ram = (u128::from(peak) * 100).div_ceil(MAX_PLANNED_RAM_PERCENT) as u64;
            // Both Qwen tiers fit well below 16 GiB; Q8 needs about 9.2 GiB and
            // Q4 about 7.9 GiB, so 8 GiB Macs keep Q4.
            assert!(minimum_ram < 10 * GIB && minimum_ram > 7 * GIB);
            for (ram, expected) in [(minimum_ram - 1, lighter), (minimum_ram, speech)] {
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
            assert_models(&hardware, [QWEN_CPU, PARAKEET, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]);
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
            assert_eq!(recommend(&hardware, SetupProfile::Balanced).asr_model, PARAKEET);
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
    fn profiles_linux_uses_cpu_choices_even_with_nvidia_metadata() {
        let hardware = HardwareProfile { platform: "linux".into(), cuda_ok: true,
            total_vram_bytes: Some(12 * GIB), ..windows(32, Some(8), Some(16)) };
        assert_models(&hardware, [QWEN_CPU, PARAKEET, MOONSHINE_MEDIUM], [QWEN_NOTES, QWEN_NOTES, LIGHT_NOTES]);
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
