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
const PARAKEET_FT2: &str = crate::optional_asr::PARAKEET_FT2_ID;
/// SonoJot Parakeet (the fine-tuned `ft2`) defaults, approved by the owner on
/// November 12, 2026 (docs/research/ParakeetFineTuneDesktop.md and
/// docs/research/ParakeetMeetingsFp16.md):
/// - It replaces stock Parakeet wherever Parakeet is offered. Stock Parakeet
///   stays installable under All models and keeps working for anyone using it.
/// - When the system language is English, Balanced (the default profile)
///   recommends it on every supported computer: CPU-only Windows and Linux,
///   NVIDIA PCs and Apple Silicon Macs. Meetings follow it ("Same as
///   dictation"); on meetings the int8 build ties Qwen 1.7B and is about three
///   times faster. The int8 CPU build only; the fp16 GPU build is not wired.
/// - Other languages keep today's recommendation (Qwen where it runs), since
///   Parakeet is English only. Qwen stays one tap away as the multilingual
///   choice, and Accurate keeps Qwen everywhere.
/// Recommendations only apply when a profile is applied (first run or a preset);
/// saved choices never change.
const FT2_REASON: &str = "SonoJot Parakeet is tuned by SonoJot for English dictation and meetings. It is smaller and faster than Qwen and finishes moments after you stop speaking.";
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
    /// Primary subtag of the system's preferred language ("en", "de"), when
    /// known. Only English moves the Balanced speech default to SonoJot Parakeet.
    pub system_language: Option<String>,
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
    pub frontier: SpeechFrontier,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechFrontier {
    pub dictation: Vec<String>,
    pub meetings: Vec<String>,
    pub other_languages: String,
}

/// SonoJot Parakeet can be offered on this build: its weights are published
/// (or a developer folder supplies them) and the platform runs sherpa-onnx.
fn ft2_offered(hardware: &HardwareProfile) -> bool {
    let platform = match hardware.platform.as_str() {
        "windows" | "linux" => true,
        "macos" => hardware.apple_silicon,
        _ => false,
    };
    platform && crate::optional_asr::find(PARAKEET_FT2).is_some_and(crate::optional_asr::obtainable)
}

fn english(hardware: &HardwareProfile) -> bool {
    hardware.system_language.as_deref() == Some("en")
}

/// Balanced recommends SonoJot Parakeet instead of stock Parakeet here.
fn ft2_default(hardware: &HardwareProfile) -> bool {
    ft2_offered(hardware) && english(hardware)
}

/// November 2026 product-model tiers, constrained by the existing RAM guard.
/// This is display policy; it does not restrict installation through the CLI.
/// When SonoJot Parakeet is offered, the main choice is SonoJot Parakeet
/// against Qwen3-ASR (owner request, November 12, 2026; awaiting director
/// review). Stock Parakeet, Moonshine and Whisper Small stay under All models.
fn speech_frontier(hardware: &HardwareProfile) -> SpeechFrontier {
    speech_frontier_with(hardware, ft2_offered(hardware), ft2_default(hardware))
}

fn speech_frontier_with(hardware: &HardwareProfile, ft2: bool, ft2_default: bool) -> SpeechFrontier {
    let mac = hardware.platform == "macos" && hardware.apple_silicon;
    let cuda = hardware.platform == "windows" && hardware.cuda_ok
        && hardware.total_vram_bytes.is_some_and(|bytes| bytes >= CUDA_SPEECH_MIN_VRAM);
    let supported = mac || matches!(hardware.platform.as_str(), "windows" | "linux");
    let mut candidates = if mac {
        let mut ids = if ft2 { vec![PARAKEET_FT2, QWEN_Q4_METAL] } else { vec![WHISPER_SMALL_METAL, QWEN_Q4_METAL] };
        if hardware.total_ram_bytes.is_some_and(|bytes| bytes >= 16 * GIB) { ids.push(QWEN_Q8_METAL); }
        ids
    } else if cuda { vec![if ft2 { PARAKEET_FT2 } else { PARAKEET }, QWEN_CUDA] }
    else if supported && ft2 { vec![PARAKEET_FT2, QWEN_CPU] }
    else if supported { vec![MOONSHINE_MEDIUM, PARAKEET, QWEN_CPU] }
    else { Vec::new() };
    let fits = |id: &str| {
        hardware.total_ram_bytes.is_none_or(|ram| {
            let selection = Selection { asr_model: id.into(), cleanup_model: "local-rules".into(),
                notes_model: QWEN_NOTES.into(), reasons: Vec::new() };
            !dictation_exceeds(&selection, ram)
        })
    };
    candidates.retain(|id| fits(id));
    let recommended = recommend_with(hardware, SetupProfile::Balanced, ft2 && ft2_default).asr_model;
    if !recommended.is_empty() && !candidates.contains(&recommended.as_str()) { candidates.push(&recommended); }
    let other = if mac { QWEN_Q4_METAL } else if cuda { QWEN_CUDA }
        else if supported && hardware.total_ram_bytes.is_some_and(|ram| ram > 8 * GIB) { QWEN_CPU }
        else if supported { QWEN_SMALL_CPU } else { "" };
    let other_languages = if !other.is_empty() && fits(other) { other.into() } else { String::new() };
    let dictation: Vec<String> = candidates.into_iter().map(String::from).collect();
    let mut meetings = dictation.clone();
    if cuda { meetings.push(crate::granite_model::GRANITE_ID.into()); }
    SpeechFrontier { dictation, meetings, other_languages }
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
        system_language: crate::system_language::primary(),
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
    let frontier = speech_frontier(&hardware);
    HardwareProfiles { hardware, recommendations, frontier }
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
    recommend_with(hardware, profile, ft2_default(hardware))
}

fn recommend_with(hardware: &HardwareProfile, profile: SetupProfile, ft2_default: bool) -> Selection {
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
            _ if ft2_default => {
                reasons.push(FT2_REASON.into());
                PARAKEET_FT2
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
            } else if ft2_default && profile == SetupProfile::Balanced {
                reasons.push(FT2_REASON.into());
                PARAKEET_FT2
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
                (_, SetupProfile::Balanced) if hardware.total_ram_bytes.is_some() && ft2_default => {
                    reasons.push(FT2_REASON.into());
                    PARAKEET_FT2
                }
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
    apply_memory_guard(hardware, &mut selection, ft2_default);
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
fn lighter_speech(id: &str, ft2_default: bool, mac: bool) -> Option<(&'static str, &'static str)> {
    match id {
        // Moonshine does not run on a Mac; Whisper Small is its light model.
        PARAKEET | PARAKEET_FT2 if mac => Some((WHISPER_SMALL_METAL, "Whisper Small replaces Parakeet")),
        QWEN_CPU => Some((QWEN_SMALL_CPU, "Qwen 0.6B replaces Qwen 1.7B")),
        QWEN_SMALL_CPU if ft2_default => Some((PARAKEET_FT2, "SonoJot Parakeet replaces Qwen speech")),
        QWEN_SMALL_CPU => Some((PARAKEET, "Parakeet replaces Qwen speech")),
        PARAKEET | PARAKEET_FT2 => Some((MOONSHINE_MEDIUM, "Moonshine replaces Parakeet")),
        QWEN_Q8_METAL => Some((QWEN_Q4_METAL, "Qwen Q4 replaces Q8")),
        QWEN_Q4_METAL => Some((WHISPER_SMALL_METAL, "Whisper Small replaces Qwen speech")),
        _ => None,
    }
}

fn apply_memory_guard(hardware: &HardwareProfile, selection: &mut Selection, ft2_default: bool) {
    let Some(ram) = hardware.total_ram_bytes.filter(|ram| *ram > 0) else {
        selection.reasons.push("RAM capacity is unknown; the memory estimate could not be checked.".into());
        return;
    };
    let memory = if hardware.apple_silicon { "unified memory" } else { "RAM" };
    // Only the dictation set depends on the speech choice. A smaller speech
    // model cannot reduce the Notes peak, so Notes never forces a downgrade.
    while dictation_exceeds(selection, ram) {
        let Some((lighter, replacement)) = lighter_speech(&selection.asr_model, ft2_default, hardware.apple_silicon) else { break };
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
            system_language: None,
        }
    }

    fn english(hardware: HardwareProfile) -> HardwareProfile {
        HardwareProfile { system_language: Some("en".into()), ..hardware }
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

    #[test]
    fn speech_frontiers_follow_hardware_and_keep_granite_meeting_only() {
        // Without SonoJot Parakeet on offer (independent of a developer folder).
        let frontier = |hardware: &HardwareProfile| speech_frontier_with(hardware, false, false);
        let gpu = frontier(&nvidia());
        assert_eq!(gpu.dictation, [PARAKEET, QWEN_CUDA]);
        assert!(gpu.meetings.contains(&crate::granite_model::GRANITE_ID.into()));
        assert_eq!(gpu.other_languages, QWEN_CUDA);
        let cpu = frontier(&windows(16, Some(4), Some(8)));
        assert_eq!(cpu.dictation, [MOONSHINE_MEDIUM, PARAKEET, QWEN_CPU]);
        assert_eq!(cpu.meetings, cpu.dictation);
        assert_eq!(frontier(&windows(8, Some(4), Some(8))).other_languages, QWEN_SMALL_CPU);
        let small_gpu = HardwareProfile { total_vram_bytes: Some(4 * GIB), ..nvidia() };
        assert!(!frontier(&small_gpu).dictation.contains(&QWEN_CUDA.into()));
        for ram in [8, 16, 24, 32, 64] {
            assert_eq!(recommend_with(&apple(ram), SetupProfile::Balanced, false).asr_model, QWEN_Q4_METAL);
            let dictation = frontier(&apple(ram)).dictation;
            assert!(dictation.contains(&QWEN_Q4_METAL.into()));
            assert_eq!(dictation.contains(&QWEN_Q8_METAL.into()), ram >= 16);
        }
    }

    #[test]
    fn sonojot_parakeet_is_the_main_choice_against_qwen_only_when_offered() {
        // Not offered (no published weights, no developer folder): today's frontier.
        let gpu = speech_frontier_with(&nvidia(), false, false);
        assert_eq!(gpu.dictation, [PARAKEET, QWEN_CUDA]);
        // Offered: SonoJot Parakeet replaces stock Parakeet against Qwen.
        let gpu = speech_frontier_with(&nvidia(), true, false);
        assert_eq!(gpu.dictation, [PARAKEET_FT2, QWEN_CUDA]);
        let mac = speech_frontier_with(&apple(16), true, false);
        assert_eq!(mac.dictation, [PARAKEET_FT2, QWEN_Q4_METAL, QWEN_Q8_METAL]);
        // Not the default (another language): stock Parakeet stays recommended, so it stays visible.
        let cpu = speech_frontier_with(&windows(16, Some(4), Some(8)), true, false);
        assert_eq!(cpu.dictation, [PARAKEET_FT2, QWEN_CPU, PARAKEET]);
        let cpu = speech_frontier_with(&windows(16, Some(4), Some(8)), true, true);
        assert_eq!(cpu.dictation, [PARAKEET_FT2, QWEN_CPU]);
    }

    #[test]
    fn sonojot_parakeet_replaces_stock_parakeet_as_the_balanced_default() {
        // Rule (b): where Balanced recommended stock Parakeet (CPU-only), it now
        // recommends SonoJot Parakeet; Accurate and Light do not change.
        let cpu = windows(16, Some(4), Some(8));
        assert_eq!(recommend_with(&cpu, SetupProfile::Balanced, false).asr_model, PARAKEET);
        assert_eq!(recommend_with(&cpu, SetupProfile::Balanced, true).asr_model, PARAKEET_FT2);
        let linux = HardwareProfile { platform: "linux".into(), ..windows(32, Some(8), Some(16)) };
        assert_eq!(recommend_with(&linux, SetupProfile::Balanced, true).asr_model, PARAKEET_FT2);
        for profile in [SetupProfile::Accurate, SetupProfile::Light] {
            assert_eq!(recommend_with(&cpu, profile, true).asr_model, recommend_with(&cpu, profile, false).asr_model);
        }
        // Unknown RAM keeps the lighter streaming model, as before.
        let unknown = HardwareProfile { total_ram_bytes: None, ..windows(32, None, None) };
        assert_eq!(recommend_with(&unknown, SetupProfile::Balanced, true).asr_model, MOONSHINE_MEDIUM);
        // The memory guard steps Qwen 0.6B down to SonoJot Parakeet when it is the default.
        assert_eq!(lighter_speech(QWEN_SMALL_CPU, true, false).unwrap().0, PARAKEET_FT2);
        assert_eq!(lighter_speech(QWEN_SMALL_CPU, false, false).unwrap().0, PARAKEET);
        assert_eq!(lighter_speech(PARAKEET_FT2, true, false).unwrap().0, MOONSHINE_MEDIUM);
        assert_eq!(lighter_speech(PARAKEET_FT2, true, true).unwrap().0, WHISPER_SMALL_METAL);
    }

    #[test]
    fn sonojot_parakeet_is_the_balanced_default_on_every_computer_in_english() {
        // November 12, 2026: NVIDIA PCs and Apple Silicon Macs move from Qwen to
        // SonoJot Parakeet for Balanced too; meetings follow ("Same as dictation").
        assert_eq!(recommend_with(&nvidia(), SetupProfile::Balanced, true).asr_model, PARAKEET_FT2);
        assert!(recommend_with(&nvidia(), SetupProfile::Balanced, true).reasons.contains(&FT2_REASON.to_string()));
        for ram in [8, 16, 32] {
            assert_eq!(recommend_with(&apple(ram), SetupProfile::Balanced, true).asr_model, PARAKEET_FT2, "{ram} GiB");
        }
        let no_cuda = HardwareProfile { cuda_ok: false, ..nvidia() };
        assert_eq!(recommend_with(&no_cuda, SetupProfile::Balanced, true).asr_model, PARAKEET_FT2);
        // Custom suggests the Balanced choice.
        assert_eq!(recommend_with(&nvidia(), SetupProfile::Custom, true).asr_model, PARAKEET_FT2);
        // Accurate keeps Qwen and Light keeps the small models.
        assert_eq!(recommend_with(&nvidia(), SetupProfile::Accurate, true).asr_model, QWEN_CUDA);
        assert_eq!(recommend_with(&nvidia(), SetupProfile::Light, true).asr_model, MOONSHINE_MEDIUM);
        assert_eq!(recommend_with(&apple(16), SetupProfile::Accurate, true).asr_model, QWEN_Q8_METAL);
        assert_eq!(recommend_with(&apple(16), SetupProfile::Light, true).asr_model, WHISPER_SMALL_METAL);
        // Not the default (another language or no weights): today's Qwen defaults.
        assert_eq!(recommend_with(&nvidia(), SetupProfile::Balanced, false).asr_model, QWEN_CUDA);
        assert_eq!(recommend_with(&apple(16), SetupProfile::Balanced, false).asr_model, QWEN_Q4_METAL);
        assert_eq!(recommend_with(&no_cuda, SetupProfile::Balanced, false).asr_model, PARAKEET);
    }

    #[test]
    fn qwen_stays_one_tap_away_as_the_multilingual_choice() {
        // The frontier shows SonoJot Parakeet (recommended) against Qwen, and
        // Qwen is the other-languages model on every platform.
        let gpu = speech_frontier_with(&nvidia(), true, true);
        assert_eq!(gpu.dictation, [PARAKEET_FT2, QWEN_CUDA]);
        assert_eq!(gpu.other_languages, QWEN_CUDA);
        assert!(gpu.meetings.starts_with(&[PARAKEET_FT2.to_string(), QWEN_CUDA.to_string()]));
        assert!(gpu.meetings.contains(&crate::granite_model::GRANITE_ID.into()));
        let mac = speech_frontier_with(&apple(16), true, true);
        assert_eq!(mac.dictation, [PARAKEET_FT2, QWEN_Q4_METAL, QWEN_Q8_METAL]);
        assert_eq!(mac.other_languages, QWEN_Q4_METAL);
        let cpu = speech_frontier_with(&windows(16, Some(4), Some(8)), true, true);
        assert_eq!(cpu.dictation, [PARAKEET_FT2, QWEN_CPU]);
        assert_eq!(cpu.other_languages, QWEN_CPU);
    }

    #[test]
    fn only_an_english_system_gets_the_new_default() {
        // Rule (b): English only. ft2_default also needs the weights on offer,
        // so check the language half directly.
        assert!(super::english(&english(windows(16, Some(4), Some(8)))));
        for language in [None, Some("de"), Some("fr"), Some("pt")] {
            let hardware = HardwareProfile { system_language: language.map(String::from), ..windows(16, Some(4), Some(8)) };
            assert!(!super::english(&hardware), "{language:?}");
            assert!(!ft2_default(&hardware), "{language:?}");
            assert_eq!(recommend(&hardware, SetupProfile::Balanced).asr_model, PARAKEET, "{language:?}");
        }
        // Other languages keep Qwen on NVIDIA and Apple Silicon.
        for language in [None, Some("de"), Some("ja")] {
            let gpu = HardwareProfile { system_language: language.map(String::from), ..nvidia() };
            assert_eq!(recommend(&gpu, SetupProfile::Balanced).asr_model, QWEN_CUDA, "{language:?}");
            let mac = HardwareProfile { system_language: language.map(String::from), ..apple(16) };
            assert_eq!(recommend(&mac, SetupProfile::Balanced).asr_model, QWEN_Q4_METAL, "{language:?}");
        }
        // With the weights published, an English NVIDIA PC and Mac get SonoJot Parakeet.
        for hardware in [english(nvidia()), english(apple(16))] {
            assert!(ft2_offered(&hardware), "{hardware:?}");
            assert_eq!(recommend(&hardware, SetupProfile::Balanced).asr_model, PARAKEET_FT2, "{hardware:?}");
        }
        // With the weights published, an English CPU-only computer gets SonoJot Parakeet.
        let english_cpu = english(windows(16, Some(4), Some(8)));
        let expected = if ft2_offered(&english_cpu) { PARAKEET_FT2 } else { PARAKEET };
        assert_eq!(recommend(&english_cpu, SetupProfile::Balanced).asr_model, expected);
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
