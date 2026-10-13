// This host exists only in the test bundle. It does not record audio or write files.
import { readFileSync } from "node:fs";
const downloadConsentCatalog = JSON.parse(readFileSync(new URL("../../legal/download-consent.json", import.meta.url), "utf8"));
export const structuredMeetingTranscript =
  "[0:12] You: Keep 4.6. Do not replace the backup.\n\n[0:31] Other speakers: I will check the release notes.\n\n[1:02:03] You: We can ship after the review.";

// Fixed Mac scenarios from profiles.rs; this fixture does not detect hardware
// or evaluate recommendation policy. Availability and free memory are simulated.
export function macProfileFixture(ramGiB = 16) {
  if (![8, 16, 32].includes(ramGiB)) throw new Error("Choose an 8, 16 or 32 GiB Mac fixture.");
  const MiB = 2 ** 20;
  const GiB = 2 ** 30;
  const small = "whisper-small-metal";
  const q8 = "qwen3-asr-1.7b-q8-metal";
  const q4 = "qwen3-asr-1.7b-q4-metal";
  const notesModel = "qwen3-4b-instruct-2507";
  const reasons = ["Apple Silicon shares memory between speech and the rest of the app."];
  if (ramGiB === 16) reasons.push("Whisper Small replaces Qwen speech because the memory plan with Qwen3 notes exceeds 45% of unified memory.");
  if (ramGiB === 8) reasons.push(
    "Whisper Small replaces Qwen speech because the memory plan with Qwen3 notes exceeds 45% of unified memory.",
    "Even this selection may use more than 45% of RAM; close other apps or run models one at a time.",
  );
  const lightNotesReason = "Qwen3 keeps meeting notes local under Apache-2.0.";
  const selection = (asrModel) => ({ asrModel, cleanupModel: "local-rules", notesModel,
    reasons: [...reasons, lightNotesReason] });
  const libraryModels = [
    { id: q8, name: "Qwen3-ASR 1.7B (Q8 Metal)", diskBytes: 2520744288,
      ramMinBytes: 2_800_000_000, ramMaxBytes: 3600 * MiB, quantization: "Q8_0", rss: "2.8", workerRss: "2.91" },
    { id: q4, name: "Qwen3-ASR 1.7B (Q4 Metal)", diskBytes: 1638144896,
      ramMinBytes: 2_360_000_000, ramMaxBytes: 3100 * MiB, quantization: "Q4_K_M", rss: "2.36", workerRss: "2.35" },
  ].map(({ rss, workerRss, ...model }) => ({
    ...model, kind: "asr", roles: ["asr"], family: "Qwen3-ASR", languages: ["Multilingual"], older: false,
    vramMinBytes: 0, vramMaxBytes: 0,
    memoryBasis: `Observed on one M1 Pro 16 GiB: ${rss} GB direct-server RSS; later evaluator + worker + server peak RSS ${workerRss} GB. Planning adds headroom. Unified memory is counted once.`,
    evidence: "2026-09-28, bundled b10964 Metal, ctx 2048, load-mode none, -ngl 999, explicit English, 12 clips. See evaluation/BENCHMARK-MAC-METAL-2026-09-28.md. Fixture availability.",
    guidance: "Local speech on Apple Silicon.", backend: "llama.cpp b10964 Metal", compatibility: "Apple Silicon macOS",
    state: "available", downloadedBytes: 0, totalBytes: model.diskBytes, error: null,
    installable: true, removable: false, usedBy: [], revision: "fixture", contextTokens: 2048,
  }));
  libraryModels.push({
    id: small, name: "Whisper Small · Metal", kind: "asr", roles: ["asr"], family: "Whisper",
    languages: ["Multilingual"], older: false, diskBytes: 487601967,
    ramMinBytes: 1_000_000_000, ramMaxBytes: 2_000_000_000, vramMinBytes: 0, vramMaxBytes: 0,
    memoryBasis: "Provisional unified-memory estimate; no separate VRAM allocation.",
    evidence: "Planning estimate; fixture availability.", guidance: "Fast start, least speech memory.",
    backend: "whisper.cpp Metal", compatibility: "Apple Silicon macOS", state: "installed",
    downloadedBytes: 487601967, totalBytes: 487601967, error: null, installable: true,
    removable: false, usedBy: [], revision: "fixture", quantization: "F16", contextTokens: null,
  });
  return {
    settings: { setupProfile: null, asrModel: small, cleanupModel: "local-rules", notesModel },
    capabilities: { platform: "macos", autoPaste: false, systemAudio: false },
    hardwareProfile: {
      hardware: { platform: "macos", appleSilicon: true, totalRamBytes: ramGiB * GiB,
        physicalCores: 8, logicalCores: 8, gpuName: null, gpuDriverVersion: null,
        gpuComputeCapability: null, totalVramBytes: null, cudaOk: false },
      recommendations: {
        accurate: selection(ramGiB < 32 ? small : q8),
        balanced: selection(ramGiB < 32 ? small : q4),
        light: { asrModel: small, cleanupModel: "local-rules", notesModel,
          reasons: [reasons[0], "Whisper Small starts quickly and uses the least speech memory on this Mac.",
            ...(ramGiB === 8 ? ["Even this selection may use more than 45% of RAM; close other apps or run models one at a time."] : []),
            lightNotesReason] },
      },
    },
    libraryHardware: { unifiedMemory: true, totalRamBytes: ramGiB * GiB, availableRamBytes: ramGiB * GiB * 0.75,
      dedicatedVramBytes: null, availableVramBytes: null },
    libraryModels,
    modelPatches: {
      "lfm2.5-1.2b": { backend: "llama.cpp Metal", compatibility: "Apple Silicon macOS",
        ramMinBytes: 1350 * MiB, ramMaxBytes: 2150 * MiB, vramMinBytes: 0, vramMaxBytes: 0 },
      "qwen3-4b-instruct-2507": { backend: "llama.cpp Metal", compatibility: "Apple Silicon macOS",
        ramMinBytes: 3500 * MiB, ramMaxBytes: 4900 * MiB, vramMinBytes: 0, vramMaxBytes: 0 },
    },
    acceleration: { textGpu: { available: true, backend: "metal", devices: ["Apple test GPU"],
      detail: "Fixture: Apple Silicon uses unified memory." }, speechDetail: "Fixture: local Metal speech." },
  };
}

export async function installDesktopFixture(page, options = {}) {
  await page.addInitScript((options) => {
    window.isTauri = true;
    const handlers = new Map();
    let maximized = false;
    let onboardingRequired = options.firstRun ?? options.settings?.setupProfile === null;
    let onboardingProfile = null;
    let microphoneActive = false;
    let nextCallback = 1;
    const callbacks = new Map();
    // Deterministic slow native responses, released explicitly by the spec.
    const heldCommands = new Map((options.holdCommands ?? []).map((command) => {
      let release;
      const promise = new Promise((resolve) => { release = resolve; });
      return [command, { promise, release }];
    }));
    window.__testReleaseCommand = (command) => {
      heldCommands.get(command)?.release();
      heldCommands.delete(command);
    };
    // Mock-only theme wiring: the real app keeps the saved theme, but the
    // isolated fixture applies ?theme=fog|slate so slate captures cannot
    // silently render Fog. Labelled mock behavior, not production logic.
    let initialTheme = "fog";
    try {
      const requested = new URLSearchParams(window.location.search).get(
        "theme",
      );
      if (requested === "fog" || requested === "slate")
        initialTheme = requested;
    } catch {
      initialTheme = "fog";
    }
    const state = {
      revision: 1,
      meetingsRevision: 1,
      sessionId: null,
      tabVisible: true,
      tabExpanded: false,
      tabTemporary: false,
      canCopyLast: true,
      deliveryNotice: null,
      resultSessionId: 1,
      recognitionCompleteness: "complete",
      recognitionWarnings: [],
      settings: {
        termsAcceptedVersion: options.firstRun ? 0 : 1,
        termsAcceptedAt: options.firstRun ? null : "2026-10-12T12:00:00Z",
        meetingConsentAcknowledged: true,
        componentAcknowledgments: {},
        setupProfile: "custom",
        punctuationRecommendationAcknowledged: false,
        punctuationV2RecommendationAcknowledged: false,
        theme: initialTheme,
        material: true,
        cleanupMode: "clean",
        cleanupTrial: false,
        smartLayout: false,
        writtenForm: true,
        launchAtLogin: false,
        hotkey: "Ctrl+Shift+D",
        hotkeyMode: "toggle",
        autoPaste: false,
        audioCues: true,
        autoStopSeconds: 0,
        microphone: null,
        asrModel: "moonshine-medium",
        meetingAsrModel: "",
        cleanupModel: "lfm2.5-1.2b",
        notesModel: "qwen3-4b-instruct-2507",
        textAcceleration: "auto",
        doNotSaveDictations: false,
        learnFromCorrections: false,
        memoryMode: "overlap",
        modelRetention: "10m",
        startTabHidden: false,
        cursorIndicator: true,
        meetingSource: "both",
        meetingSourceMigrated: true,
        autoMeetingTitles: true,
        meetingSpeakerLabels: true,
        ...options.settings,
        ...(options.firstRun ? { setupProfile: null, hotkey: "Ctrl+Space", microphone: null } : {}),
      },
      state: "idle",
      originalText: "first keep grock 4.6",
      outputText: "First, keep Grok 4.6.",
      interimText: "",
      audioLevel: 0,
      error: null,
      models: [
        {
          id: "moonshine-medium",
          name: "Moonshine Medium",
          kind: "asr",
          ready: true,
          description: "Test model",
          sizeBytes: 1000000,
        },
        {
          id: "lfm2.5-1.2b",
          name: "LFM 2.5",
          kind: "cleanup",
          ready: true,
          description: "Test cleanup",
          sizeBytes: 1000000,
        },
      ],
      devices: [{ id: "mic", name: "Test microphone", isDefault: true }],
      meetingRecording: false,
      activeMeetingId: null,
      capabilities: {
        autoPaste: true,
        systemAudio: true,
        globalShortcut: true,
        platform: "Isolated UI test host",
        ...options.capabilities,
      },
      version: "0.22.0",
      update: {
        currentVersion: "0.22.0",
        status: { state: "idle" },
        restartBlockedReason: null,
        installing: false,
        ...options.update,
      },
    };
    let vocabulary = [];
    const seed = {
      id: "meeting-one",
      title: "Design review",
      createdAt: "2026-09-10T20:00:00Z",
      durationSeconds: 300,
      status: "ready",
      hasAudio: true,
      hasTranscript: true,
      audioOutages: [],
      missingSeconds: 0,
      cleanedTranscript: null,
      transcriptionStoppedAtSeconds: null,
      transcript:
        "Keep 4.6. Do not replace the backup. <img src=x onerror=alert(1)>",
      notes: "",
      source: "microphone",
      audioPath: "C:/test.wav",
      error: null,
      ...options.meeting,
    };
    const outageMeeting = {
      ...structuredClone(seed),
      id: "meeting-outage",
      title: "Client call",
      createdAt: "2026-09-09T20:00:00Z",
      durationSeconds: 2892,
      status: "transcribed",
      source: "both",
      speakerCount: 2,
      speakerLabels: true,
      transcript: "[12:31] Speaker 1: The main thing for us is the timeline in, in March.\n\n[12:55] You: Sorry, I lost you for a second. You were saying March?",
      cleanedTranscript: "[12:31] Speaker 1: The main thing for us is the timeline in March.\n\n[12:55] You: Sorry, I lost you for a second. You were saying March?",
      notes: "Discuss the March timeline.",
      audioOutages: [{ source: "microphone", startSeconds: 760, endSeconds: 774, reason: "Microphone disconnected" }],
      missingSeconds: 14,
      recordingWarnings: [],
      recognitionWarnings: [],
      error: null,
    };
    let meetings = options.meetings ? structuredClone(options.meetings) : [structuredClone(seed), outageMeeting];
    // Mock Round 6 model library for UI tests only. Sizes and memory ranges
    // are illustrative placeholders, not measurements from this machine.
    const GB = 1024 * 1024 * 1024;
    const hardwareProfile = options.hardwareProfile ?? {
      hardware: {
        platform: "windows", appleSilicon: false, totalRamBytes: 32 * GB,
        physicalCores: 8, logicalCores: 16, gpuName: "NVIDIA test GPU",
        gpuDriverVersion: "fixture", gpuComputeCapability: "8.6",
        totalVramBytes: 8 * GB, cudaOk: true,
      },
      recommendations: {
        accurate: { asrModel: "qwen3-asr-1.7b-fp16-cuda", cleanupModel: "local-rules", notesModel: "mock-cleanup-2b-cuda", reasons: ["Fictional accuracy profile"] },
        balanced: { asrModel: "mock-whisper-small-cuda", cleanupModel: "local-rules", notesModel: "mock-cleanup-2b-cuda", reasons: ["Fictional balanced profile"] },
        light: { asrModel: "moonshine-medium", cleanupModel: "local-rules", notesModel: "qwen3-4b-instruct-2507", reasons: ["Fictional light profile"] },
      },
    };
    let library = {
      hardware: {
        totalRamBytes: 32 * GB,
        availableRamBytes: 20 * GB,
        dedicatedVramBytes: 8 * GB,
        availableVramBytes: 6 * GB,
        ...options.libraryHardware,
      },
      guidance:
        "Mock estimates for UI tests only. Text weights support CPU or GPU; CUDA speech is a separate package.",
      models: [
        {
          id: "moonshine-medium",
          name: "Moonshine Medium",
          kind: "asr",
          roles: ["asr"],
          family: "Moonshine medium",
          languages: ["Multilingual"],
          older: false,
          diskBytes: 1 * GB,
          ramMinBytes: 1 * GB,
          ramMaxBytes: 2 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock speech entry. Runs on CPU.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU. AMD GPU acceleration is not supported.",
          state: "installed",
          downloadedBytes: 1 * GB,
          totalBytes: 1 * GB,
          error: null,
          installable: false,
          removable: false,
          usedBy: ["asr"],
          revision: "mock-r1",
          quantization: "mock-int8",
          contextTokens: null,
        },
        {
          id: "lfm2.5-1.2b",
          name: "LFM 2.5",
          kind: "cleanup",
          roles: ["cleanup", "notes"],
          family: "Mock text 2B",
          languages: ["English"],
          older: false,
          diskBytes: 1 * GB,
          ramMinBytes: 2 * GB,
          ramMaxBytes: 3 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock text entry shared by cleanup and notes.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU. AMD GPU acceleration is not supported.",
          state: "installed",
          downloadedBytes: 1 * GB,
          totalBytes: 1 * GB,
          error: null,
          installable: true,
          removable: true,
          usedBy: ["cleanup"],
          revision: "mock-r1",
          quantization: "mock-q4",
          contextTokens: 4096,
        },
        {
          id: "qwen3-4b-instruct-2507",
          name: "Qwen 3 4B",
          kind: "cleanup",
          roles: ["cleanup", "notes"],
          family: "Mock text 4B",
          languages: ["English"],
          older: false,
          diskBytes: 3 * GB,
          ramMinBytes: 4 * GB,
          ramMaxBytes: 5 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock notes entry.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU. AMD GPU acceleration is not supported.",
          state: "installed",
          downloadedBytes: 3 * GB,
          totalBytes: 3 * GB,
          error: null,
          installable: true,
          removable: true,
          usedBy: ["notes"],
          revision: "mock-r1",
          quantization: "mock-q4",
          contextTokens: 8192,
        },
        {
          id: "mock-whisper-small-cpu",
          name: "Mock Whisper Small (CPU)",
          kind: "asr",
          roles: ["asr"],
          family: "Whisper small",
          languages: ["Multilingual"],
          older: false,
          diskBytes: 1 * GB,
          ramMinBytes: 2 * GB,
          ramMaxBytes: 3 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock speech entry. Runs on CPU.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU. AMD GPU acceleration is not supported.",
          state: "installed",
          downloadedBytes: 1 * GB,
          totalBytes: 1 * GB,
          error: null,
          installable: true,
          removable: false,
          usedBy: ["asr"],
          revision: "mock-r1",
          quantization: "mock-int8",
          contextTokens: null,
        },
        {
          id: "mock-whisper-small-cuda",
          name: "Mock Whisper Small (NVIDIA CUDA)",
          kind: "asr",
          roles: ["asr"],
          family: "Whisper small",
          languages: ["Multilingual"],
          older: false,
          diskBytes: 1 * GB,
          ramMinBytes: 1 * GB,
          ramMaxBytes: 2 * GB,
          vramMinBytes: 2 * GB,
          vramMaxBytes: 3 * GB,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock GPU variant, listed separately from the CPU package.",
          evidence: "Unrated",
          backend: "cuda",
          compatibility:
            "Requires an NVIDIA GPU with CUDA. AMD GPUs are not supported.",
          state: "available",
          downloadedBytes: 0,
          totalBytes: 1 * GB,
          error: null,
          installable: true,
          removable: true,
          usedBy: [],
          revision: "mock-r1",
          quantization: "mock-int8",
          contextTokens: null,
        },
        {
          id: "mock-cleanup-2b-cpu",
          name: "Mock Cleanup 2B (CPU)",
          kind: "cleanup",
          roles: ["cleanup", "notes"],
          family: "Mock text 2B",
          languages: ["English"],
          older: false,
          diskBytes: 2 * GB,
          ramMinBytes: 3 * GB,
          ramMaxBytes: 4 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock text entry shared by cleanup and notes.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU. AMD GPU acceleration is not supported.",
          state: "installed",
          downloadedBytes: 2 * GB,
          totalBytes: 2 * GB,
          error: null,
          installable: true,
          removable: true,
          usedBy: ["cleanup", "notes"],
          revision: "mock-r1",
          quantization: "mock-q4",
          contextTokens: 4096,
        },
        {
          id: "mock-cleanup-2b-cuda",
          name: "Mock Cleanup 2B (NVIDIA CUDA)",
          kind: "cleanup",
          roles: ["cleanup", "notes"],
          family: "Mock text 2B",
          languages: ["English"],
          older: false,
          diskBytes: 2 * GB,
          ramMinBytes: 1 * GB,
          ramMaxBytes: 1 * GB,
          vramMinBytes: 3 * GB,
          vramMaxBytes: 4 * GB,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock GPU variant, listed separately from the CPU package.",
          evidence: "Unrated",
          backend: "cuda",
          compatibility:
            "Requires an NVIDIA GPU with CUDA. AMD GPUs are not supported.",
          state: "available",
          downloadedBytes: 0,
          totalBytes: 2 * GB,
          error: null,
          installable: true,
          removable: true,
          usedBy: [],
          revision: "mock-r1",
          quantization: "mock-q4",
          contextTokens: 4096,
        },
        {
          id: "mock-cleanup-old-1b",
          name: "Mock Cleanup 1B (older)",
          kind: "cleanup",
          roles: ["cleanup"],
          family: "Mock text 1B",
          languages: ["English"],
          older: true,
          diskBytes: 1 * GB,
          ramMinBytes: null,
          ramMaxBytes: null,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Unrated in this mock.",
          guidance: "Older mock entry kept for disclosure, not recommended.",
          evidence: "Unrated",
          backend: "cpu",
          compatibility: "Windows CPU.",
          state: "failed",
          downloadedBytes: 0,
          totalBytes: 1 * GB,
          error: "Mock download failure for UI retry tests.",
          installable: true,
          removable: false,
          usedBy: [],
          revision: "mock-r0",
          quantization: "mock-q4",
          contextTokens: 2048,
        },
        {
          id: "local-rules",
          name: "Formatting and vocabulary",
          kind: "cleanup",
          roles: ["cleanup"],
          family: "Rules",
          languages: ["en"],
          older: false,
          diskBytes: 0,
          ramMinBytes: null,
          ramMaxBytes: null,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock: host-only rules; no model weights.",
          guidance:
            "Mock rules-only entry. Immediate local rules; no language model.",
          evidence: "Host rules; no model rating applies.",
          backend: "local-rules",
          compatibility: "All platforms, CPU only, no download.",
          state: "installed",
          downloadedBytes: 0,
          totalBytes: 0,
          error: null,
          installable: false,
          removable: false,
          usedBy: [],
          revision: "host-rules-v4",
          quantization: "none",
          contextTokens: null,
        },
        {
          id: "qwen3-asr-1.7b",
          name: "Qwen3-ASR 1.7B (CPU int8)",
          kind: "asr",
          roles: ["asr"],
          family: "Qwen3-ASR",
          languages: ["multilingual"],
          older: false,
          diskBytes: 2 * GB,
          ramMinBytes: 3 * GB,
          ramMaxBytes: 4 * GB,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock estimate for UI tests, not a measurement.",
          guidance: "Mock CPU Qwen entry. Runs on CPU only.",
          evidence: "Unrated",
          backend: "sherpa-onnx-1.13.8-cpu (Offline, transcribe, CPU provider)",
          compatibility:
            "Windows x64 CPU via bundled sherpa-onnx v1.13.8; NVIDIA CUDA not used by this adapter; AMD has no CUDA path",
          state: "available",
          downloadedBytes: 0,
          totalBytes: 2 * GB,
          error: null,
          installable: true,
          removable: false,
          usedBy: [],
          revision: "mock-r1",
          quantization: "int8",
          contextTokens: null,
        },
        {
          id: "qwen3-asr-1.7b-fp16-cuda",
          name: "Qwen3-ASR 1.7B (FP16 CUDA)",
          kind: "asr",
          roles: ["asr"],
          family: "qwen",
          languages: ["Multilingual"],
          older: false,
          diskBytes: 5 * GB,
          ramMinBytes: null,
          ramMaxBytes: null,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock: Unmeasured on this machine.",
          guidance:
            "Mock NVIDIA-only entry. Needs CUDA runtime; third-party export, disclosed provenance.",
          evidence: "Unrated",
          backend:
            "sherpa-onnx-1.13.8-cuda13 (OfflineQwen3ASR fp16, transcribe, CUDA provider; VAD CPU)",
          compatibility:
            "Windows x64 NVIDIA CUDA 13.x + cuDNN 9.x via bundled sherpa-onnx v1.13.8 CUDA runtime; AMD has no CUDA path",
          state: "available",
          downloadedBytes: 0,
          totalBytes: 5 * GB,
          error: null,
          installable: true,
          removable: false,
          usedBy: [],
          revision: "mock-gpu-r1",
          quantization: "fp16",
          contextTokens: null,
        },
        {
          id: "whisper-tiny-fp32-cuda",
          name: "Whisper Tiny (FP32 CUDA)",
          kind: "asr",
          roles: ["asr"],
          family: "whisper",
          languages: ["Multilingual"],
          older: false,
          diskBytes: 153 * 1024 * 1024,
          ramMinBytes: null,
          ramMaxBytes: null,
          vramMinBytes: null,
          vramMaxBytes: null,
          memoryBasis: "Mock: Unmeasured on this machine.",
          guidance:
            "Mock small CUDA option; same CUDA runtime as the Qwen FP16 path.",
          evidence: "Unrated",
          backend:
            "sherpa-onnx-1.13.8-cuda13 (OfflineWhisper fp32, transcribe, CUDA provider; VAD CPU)",
          compatibility:
            "Windows x64 NVIDIA CUDA 13.x + cuDNN 9.x via bundled sherpa-onnx v1.13.8 CUDA runtime; AMD has no CUDA path",
          state: "available",
          downloadedBytes: 0,
          totalBytes: 153 * 1024 * 1024,
          error: null,
          installable: true,
          removable: false,
          usedBy: [],
          revision: "mock-gpu-r1",
          quantization: "fp32",
          contextTokens: null,
        },
      ],
    };
    if (options.diarization !== null) {
      library.models.push({
        id: "nemotron-3-diarization",
        name: "Nemotron 3 Diarization",
        kind: "diarization",
        roles: ["speakers"],
        family: "Nemotron 3 Diarization · CPU ONNX Runtime",
        languages: [],
        older: false,
        diskBytes: 103871648,
        ramMinBytes: null,
        ramMaxBytes: null,
        vramMinBytes: 0,
        vramMaxBytes: 0,
        memoryBasis: "Mock: Unmeasured on this machine.",
        guidance: "Separates voices on this computer after transcription.",
        evidence: "UI fixture only",
        backend: "CPU · ONNX Runtime",
        compatibility: "Windows CPU",
        state: "available",
        downloadedBytes: 0,
        totalBytes: 103871648,
        error: null,
        installable: true,
        removable: false,
        usedBy: [],
        revision: "mock-diarization-r1",
        quantization: "int8",
        contextTokens: null,
        ...options.diarization,
      });
    }
    // Installed by default so unrelated profile cases isolate their own downloads.
    // Punctuation tests explicitly request available/failed/checking states.
    if (options.punctuation !== null) library.models.push({
      id: "sonojot-punct-v2", name: "SonoJot Punctuation v2", kind: "punctuation", roles: ["punctuation"],
      family: "SonoJot Punctuation", languages: ["English"], older: false,
      diskBytes: 302728954, totalBytes: 302728954, downloadedBytes: 302728954,
      ramMinBytes: null, ramMaxBytes: null, vramMinBytes: 0, vramMaxBytes: 0,
      memoryBasis: "Unmeasured; fixture only", guidance: "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU",
      evidence: "CC BY-SA 4.0; https://huggingface.co/SonoJot/sonojot-punct-v2",
      backend: "CPU · ONNX Runtime", compatibility: "Windows x64 and Apple Silicon CPU",
      state: "installed", error: null, installable: true, removable: true,
      usedBy: [], revision: "fixture", quantization: "int8", contextTokens: null,
      ...options.punctuation,
    });
    // Text uses the same weights on CPU/GPU. These are fictional estimates.
    for (const model of library.models) {
      if (
        model.kind === "cleanup" &&
        model.id !== "local-rules" &&
        !model.older
      ) {
        model.vramMinBytes ??= model.diskBytes;
        model.vramMaxBytes ??= model.diskBytes + GB;
      }
    }
    // Optional catalog entries for focused visual fixtures; never native data.
    for (const model of options.libraryModels ?? []) {
      library.models = library.models.filter((entry) => entry.id !== model.id);
      library.models.push(structuredClone(model));
    }
    for (const model of library.models) {
      Object.assign(model, options.modelPatches?.[model.id] ?? {});
    }
    const syncUsedBy = () => {
      for (const model of library.models) {
        model.usedBy = ["asr", "cleanup", "notes"].filter(
          (role) => state.settings[`${role}Model`] === model.id,
        );
        if (state.settings.meetingAsrModel === model.id)
          model.usedBy.push("meetingSpeech");
      }
    };
    syncUsedBy();
    // Mock learned entries for UI tests only.
    let learned = [
      {
        id: "mock-learned-one",
        kind: "correction",
        text: "Grok",
        heard: "grock",
        scope: "AI model discussion",
        sourceIds: ["dictation-one"],
        sourceType: "dictation-correction",
        enabled: true,
        userPinned: false,
      },
    ];
    let learnedSeq = 2;
    // AI-only fixture options cover connected lists, explicit empty lists and
    // pre-migration responses (aiLegacySettings: true omits quickModels).
    let aiSettings = {
      backend: "local",
      model: "",
      speechModel: "",
      connected: false,
      supported: true,
      help: "Mock AI settings for UI tests.",
      ...options.aiSettings,
    };
    if (!Object.hasOwn(aiSettings, "quickModels") && !options.aiLegacySettings) {
      aiSettings.quickModels = [
        { id: aiSettings.model, role: "text" },
        { id: aiSettings.speechModel, role: "speech" },
      ].filter((entry) => entry.id);
    }
    const aiCatalog = options.aiCatalog ?? [
      {
        id: "mock/text", name: "Mock text model", contextLength: 32000,
        promptPrice: "0.000001", completionPrice: "0.000002",
        inputModalities: ["text"],
      },
      {
        id: "mock/audio", name: "Mock speech model", contextLength: 32000,
        promptPrice: "0.000001", completionPrice: "0.000002",
        inputModalities: ["text", "audio"],
      },
      {
        id: "mock/unknown", name: "Legacy catalog model", contextLength: 32000,
        promptPrice: null, completionPrice: null,
      },
    ];
    window.__testAiSettings = () => clone(aiSettings);
    window.__testPatchAiSettings = (patch) => Object.assign(aiSettings, clone(patch));
    const aiErrors = new Map(Object.entries(options.aiErrors ?? {}));
    window.__testFailAiCommand = (command, message) => aiErrors.set(command, message);
    function checkAiError(command) {
      if (!aiErrors.has(command)) return;
      const message = aiErrors.get(command);
      aiErrors.delete(command);
      throw new Error(message);
    }
    let aiConnectPending = null;
    window.__testCompleteAiConnect = () => {
      if (!aiConnectPending) return;
      aiSettings = { ...aiSettings, connected: true };
      aiConnectPending.resolve(clone(aiSettings));
      aiConnectPending = null;
    };
    window.__testLibrary = () => clone(library);
    const onboardingQueue = [];
    window.__testOnboarding = () => ({ required: onboardingRequired, profile: onboardingProfile, queue: [...onboardingQueue], microphoneActive });
    function queueOnboarding(profile, defaults = false) {
      const choice = profile === "custom" ? state.settings : hardwareProfile.recommendations[profile];
      Object.assign(state.settings, { asrModel: choice.asrModel, cleanupModel: choice.cleanupModel, notesModel: choice.notesModel,
        setupProfile: onboardingRequired && !defaults ? null : profile,
        punctuationRecommendationAcknowledged: true, punctuationV2RecommendationAcknowledged: true });
      if (defaults) Object.assign(state.settings, { microphone: null, hotkey: "Ctrl+Space", hotkeyMode: "toggle" });
      onboardingProfile = profile;
      onboardingQueue.splice(0, onboardingQueue.length, ...new Set([choice.asrModel, "sonojot-punct-v2", choice.notesModel].filter((id) => id !== "openrouter")));
      let downloading = false;
      for (const id of onboardingQueue) {
        const model = library.models.find((entry) => entry.id === id);
        if (model && !["installed", "ready", "unloaded"].includes(model.state)) {
          model.state = downloading ? "queued" : "downloading";
          downloading = true;
        }
      }
      if (profile !== "custom") aiSettings.backend = "local";
      syncUsedBy(); emit();
    }
    let profileApplyError = null;
    let setupCompletionError = null;
    let modelInstallError = null;
    window.__testFailProfileApply = (message) => { profileApplyError = message; };
    window.__testFailSetupCompletion = (message) => { setupCompletionError = message; };
    window.__testFailModelInstall = (id, message) => { modelInstallError = { id, message }; };
    window.__testPatchModel = (id, patch) => {
      const model = library.models.find((item) => item.id === id);
      if (!model) throw new Error(`Unknown mock model: ${id}`);
      Object.assign(model, clone(patch));
    };
    window.__testLibraryProgress = (id, downloadedBytes, state) => {
      const entry = library.models.find((item) => item.id === id);
      if (entry) {
        entry.downloadedBytes = downloadedBytes;
        entry.state = state;
        if (state === "installed") entry.removable = true;
      }
      emit();
    };
    let history = options.history ?? [
      {
        id: "dictation-one",
        createdAt: "2026-09-10T20:00:00Z",
        raw: "first keep grock 4.6",
        published: "First, keep Grok 4.6.",
        recognitionCompleteness: "complete",
        recognitionWarnings: [],
      },
      {
        id: "dictation-incomplete",
        createdAt: "2026-09-10T20:01:00Z",
        raw: "Keep the first instruction. <thö Do not remove the backup.",
        published: "Keep the first instruction. <thö Do not remove the backup.",
        recognitionCompleteness: "incomplete",
        recognitionWarnings: ["segment two remained ambiguous after one retry"],
      },
    ];
    let pending;
    let completeTranscription;
    window.__testCompleteTranscription = (patch = {}) => completeTranscription?.(patch);
    let nextSession = 2;
    window.__testCalls = [];
    // Audio-export test controls (mock only): queued save-dialog results,
    // one-shot export failure, and extra meetings for selection-change cover.
    let dialogSaveQueue = [];
    let heldDialogResolve = null;
    let audioExportError = null;
    // Match Tauri's JSON boundary, including nested Svelte state proxies.
    const clone = (value) => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
    window.__testDialogSave = (value) => {
      dialogSaveQueue.push(value);
    };
    window.__testReleaseDialogSave = (value) => {
      if (heldDialogResolve) {
        const resolve = heldDialogResolve;
        heldDialogResolve = null;
        resolve(value);
      } else {
        dialogSaveQueue.push(value);
      }
    };
    window.__testFailAudioExport = (message) => {
      audioExportError = message;
    };
    // Structured ASR runtime override (mock only): null by default (nothing
    // resident, cpu/cpu). Tests set a resident CUDA object to cover the
    // truthful "NVIDIA CUDA loaded" label without claiming measured execution.
    let asrRuntimeOverride = null;
    window.__testAsrRuntime = (value) => {
      asrRuntimeOverride = value;
    };
    window.__testAddMeeting = (patch = {}) => {
      const meeting = { ...clone(seed), ...clone(patch) };
      meetings.push(meeting);
      state.meetingsRevision += 1;
      return clone(meeting);
    };
    window.__testUpdateMeetingById = (id, patch, notify = true) => {
      const meeting = meetings.find((item) => item.id === id);
      if (meeting) {
        Object.assign(meeting, clone(patch));
        state.meetingsRevision += 1;
      }
      if (notify) emit();
    };
    const emit = () => {
      state.revision += 1;
      const handler = handlers.get("app-state");
      if (handler)
        callbacks.get(handler)?.({
          event: "app-state",
          id: 1,
          payload: clone(state),
        });
    };
    window.__testPushSnapshot = (patch) => {
      Object.assign(state, clone(patch));
      emit();
    };
    window.__testSnapshot = () => clone(state);
    // Drive checks, unknown/known download progress, ready and errors without timers.
    // Initial scenarios still use options.update; `installing` simulates a restart.
    window.__testUpdateStatus = (status, patch = {}) => {
      Object.assign(state.update, clone(patch), { status: clone(status) });
      emit();
    };
    window.__testUpdateMeeting = (patch, notify = true) => {
      Object.assign(meetings[0], clone(patch));
      state.meetingsRevision += 1;
      if (notify) emit();
    };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener: (_, id) => callbacks.delete(id),
    };
    window.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main", windowLabel: "main" },
      },
      transformCallback: (callback) => {
        const id = nextCallback++;
        callbacks.set(id, callback);
        return id;
      },
      invoke: async (command, args = {}) => {
        window.__testCalls.push({ command, args: clone(args) });
        if (heldCommands.has(command)) await heldCommands.get(command).promise;
        switch (command) {
          case "plugin:event|listen":
            handlers.set(args.event, args.handler);
            return args.handler;
          case "plugin:window|is_maximized":
            return maximized;
          case "plugin:window|toggle_maximize":
            maximized = !maximized;
            callbacks.get(handlers.get("tauri://resize"))?.({
              event: "tauri://resize",
              id: 2,
              payload: { width: 1220, height: 860 },
            });
            return null;
          case "plugin:window|minimize":
          case "plugin:window|start_dragging":
            return null;
          case "plugin:window|close":
            microphoneActive = false;
            if (onboardingRequired) { queueOnboarding("balanced", true); onboardingRequired = false; }
            return null;
          case "plugin:event|unlisten":
            return null;
          case "get_setup":
          case "install_media_support":
          case "install_asr_model":
          case "uninstall_asr_model":
          case "install_text_model":
          case "uninstall_text_model":
            return {
              microphone: { state: "installed", detail: null },
              speech: { state: "installed", detail: null },
              shortcut: { state: "ready", detail: null },
              cleanup: { state: "installed", detail: null },
              mediaImport: { state: "installed", detail: null },
              modelsPath: "C:/Models",
              ...clone(options.setup ?? {}),
            };
          case "get_snapshot":
            return clone(state);
          case "check_for_updates":
            if (["checking", "available", "downloading", "ready"].includes(state.update.status.state) || state.update.installing)
              return clone(state);
            state.update.status = { state: "checking" };
            emit();
            return clone(state);
          case "download_update":
            if (["downloading", "ready"].includes(state.update.status.state) || state.update.installing)
              return clone(state);
            if (state.update.status.state !== "available") throw new Error("Check for updates before downloading.");
            state.update.status = { state: "downloading", percent: null };
            emit();
            return clone(state);
          case "restart_to_update":
            if (state.update.restartBlockedReason) throw new Error(state.update.restartBlockedReason);
            if (state.update.status.state !== "ready") throw new Error("No update is ready.");
            state.update.installing = true;
            emit();
            return clone(state);
          case "get_hardware_profile":
            return clone(hardwareProfile);
          case "begin_onboarding":
            return onboardingRequired;
          case "confirm_onboarding_profile":
            if (profileApplyError) { const message = profileApplyError; profileApplyError = null; throw new Error(message); }
            queueOnboarding(args.profile);
            return clone(state);
          case "finish_onboarding":
            if (setupCompletionError) { const message = setupCompletionError; setupCompletionError = null; throw new Error(message); }
            if (onboardingRequired && !onboardingProfile) throw new Error("Confirm a model profile before finishing setup.");
            if (onboardingRequired) { state.settings.setupProfile = onboardingProfile; onboardingRequired = false; emit(); }
            return clone(state);
          case "onboarding_microphone":
            if (args.action === "start" && options.microphoneError) throw new Error(options.microphoneError);
            if (args.action === "start") microphoneActive = true;
            if (args.action === "stop") microphoneActive = false;
            return { level: microphoneActive ? options.microphoneLevel ?? 0.12 : 0, active: microphoneActive };
          case "open_permission_settings":
            return null;
          case "onboarding_paste_permission":
            return state.capabilities.platform === "macos" ? "Accessibility permission is not granted." : null;
          case "apply_setup_profile": {
            if (profileApplyError) {
              const message = profileApplyError;
              profileApplyError = null;
              throw new Error(message);
            }
            const choice = args.profile === "custom" ? state.settings : hardwareProfile.recommendations[args.profile];
            const speech = library.models.find((model) => model.id === choice?.asrModel);
            if ((args.profile !== "custom" || state.settings.setupProfile === null) &&
                (!speech || !["installed", "ready", "unloaded"].includes(speech.state))) {
              throw new Error("Install the selected speech model before applying a setup profile.");
            }
            if (args.profile !== "custom") {
              if (state.settings.setupProfile === null) {
                state.settings.punctuationRecommendationAcknowledged = true;
                state.settings.punctuationV2RecommendationAcknowledged = true;
              }
              state.settings.asrModel = choice.asrModel;
              state.settings.cleanupModel = choice.cleanupModel;
              state.settings.notesModel = choice.notesModel;
            }
            state.settings.setupProfile = args.profile;
            syncUsedBy();
            emit();
            return clone(state);
          }
          case "get_model_acceleration":
            return {
              textGpu: {
                available: true,
                backend: "cuda",
                devices: ["NVIDIA test GPU"],
                detail:
                  "Compatible GPU detected. The same downloaded text model can run on CPU or GPU; no second model download is needed.",
              },
              textRuntime: null,
              cpuOnly: state.settings.textAcceleration === "cpu",
              asrRuntime: asrRuntimeOverride ?? {
                modelId: null,
                backend: null,
                compatibility: null,
                providerRequested: "cpu",
                providerInitialized: "cpu",
                deviceIndex: null,
                executionProof: null,
              },
              speechDetail:
                "Mock speech detail for UI tests. Installed CPU packages run on CPU. NVIDIA CUDA speech packages need NVIDIA hardware and runtime; runtime use is unverified until measured. AMD is unsupported.",
              ...clone(options.acceleration ?? {}),
            };
          case "set_screen_tab":
            state.tabVisible = args.visible;
            state.tabExpanded =
              args.visible && (args.expanded ?? state.tabExpanded);
            state.tabTemporary =
              state.tabTemporary && args.visible && !args.focus && !args.pinned;
            emit();
            return clone(state);
          case "show_page":
          case "copy_last_dictation":
            return null;
          case "copy_smart_original":
            if (state.resultSessionId !== args.resultSessionId || state.sessionId !== null ||
                !["idle", "error"].includes(state.state) || state.meetingRecording ||
                !["smart_cleanup", "smart_layout"].includes(state.publicationStage) || !state.preModelText ||
                state.preModelText === state.outputText || state.recognitionCompleteness !== "complete") {
              throw new Error("This cleaned dictation is no longer current. Review it in Recent dictations.");
            }
            window.__testClipboard = state.preModelText;
            return null;
          case "copy_history_pre_model": {
            const entry = history.find((entry) => entry.id === args.id);
            if (!entry?.preModelText) throw new Error("There is no original cleanup text to copy.");
            window.__testClipboard = entry.preModelText;
            return null;
          }
          case "complete_setup":
            if (setupCompletionError) {
              const message = setupCompletionError;
              setupCompletionError = null;
              throw new Error(message);
            }
            return null;
          case "get_download_requirements": {
            const platform = state.capabilities.platform === "macos" ? "macos" : "windows";
            return clone(options.downloadConsentGroups.filter(group =>
              (group.platform === "all" || group.platform === platform)
              && group.models.some(id => args.ids.includes(id))
              && state.settings.componentAcknowledgments[group.id] !== group.version));
          }
          case "acknowledge_download_terms":
            for (const { id, version } of args.acknowledgments) {
              const group = options.downloadConsentGroups.find(group => group.id === id && group.version === version);
              if (!group) throw new Error("Review the changed terms before downloading.");
              state.settings.componentAcknowledgments[id] = version;
            }
            emit();
            return clone(state);
          case "save_settings":
            state.settings = clone(args.settings);
            syncUsedBy();
            emit();
            return clone(state);
          case "start_dictation":
          case "test_microphone":
            if (command === "start_dictation" && state.settings.setupProfile === null) {
              state.error = "SETUP_INCOMPLETE: Finish setup to start dictating.";
              emit();
              throw new Error(state.error);
            }
            state.sessionId = nextSession++;
            state.resultSessionId = null;
            state.state = "listening";
            state.audioLevel = 0.45;
            state.interimText = "first keep grock 4.6";
            emit();
            return clone(state);
          case "screen_tab_record":
            if (state.settings.setupProfile === null) {
              state.error = "SETUP_INCOMPLETE: Finish setup to start dictating.";
              state.state = "error";
              emit();
              throw new Error(state.error);
            }
            state.state = "listening";
            state.sessionId = nextSession++;
            emit();
            return clone(state);
          case "stop_dictation":
            state.resultSessionId = state.sessionId;
            state.sessionId = null;
            state.recognitionCompleteness = "complete";
            state.recognitionWarnings = [];
            state.state = "idle";
            state.audioLevel = 0;
            state.interimText = "";
            emit();
            return clone(state);
          case "cancel_dictation":
            state.sessionId = null;
            state.state = "idle";
            state.audioLevel = 0;
            state.interimText = "";
            emit();
            return clone(state);
          case "copy_text":
          case "copy_history":
          case "open_path":
          case "open_punctuation_model_page":
          case "open_model_license":
          case "open_legal_link":
          case "open_download_license":
          case "export_meeting":
            return null;
          case "export_meeting_audio": {
            if (audioExportError) {
              const message = audioExportError;
              audioExportError = null;
              throw new Error(message);
            }
            return null;
          }
          case "get_ai_settings":
            checkAiError(command);
            return clone(aiSettings);
          case "save_ai_settings":
            checkAiError(command);
            if (args.settings.backend === "local" && state.settings.notesModel === "openrouter")
              throw new Error("Choose a local Notes model before switching AI processing back to Local.");
            aiSettings = {
              ...aiSettings,
              ...clone(args.settings),
              connected: aiSettings.connected,
              supported: aiSettings.supported,
              help: aiSettings.help,
            };
            return clone(aiSettings);
          case "connect_openrouter":
            checkAiError(command);
            if (!aiSettings.supported) throw new Error(aiSettings.help);
            if (options.aiConnectPending) {
              return new Promise((resolve, reject) => {
                aiConnectPending = { resolve, reject };
              });
            }
            aiSettings = { ...aiSettings, connected: true };
            return clone(aiSettings);
          case "cancel_ai_connect":
            aiConnectPending?.reject(new Error("OpenRouter sign-in cancelled."));
            aiConnectPending = null;
            return null;
          case "disconnect_openrouter":
            checkAiError(command);
            if (state.settings.notesModel === "openrouter")
              throw new Error("Choose a local Notes model before disconnecting OpenRouter.");
            aiSettings = { ...aiSettings, backend: "local", connected: false };
            return clone(aiSettings);
          case "list_ai_models":
            checkAiError(command);
            return clone(options.aiRecommendedModels ?? aiCatalog.filter((model) => model.id === "mock/text"));
          case "list_ai_catalog":
            checkAiError(command);
            return clone(aiCatalog);
          case "check_ai_model":
            checkAiError(command);
            return clone(options.aiModelChecks?.[args.model] ?? {
              availability: "unknown",
              privacy: "unknown",
              variant: null,
              routes: 0,
              activeRoutes: 0,
            });
          case "suggest_vocabulary":
            return [];
          case "cancel_vocabulary_suggestions":
            return null;
          case "get_model_library":
            return clone(library);
          case "start_model_install": {
            if (modelInstallError?.id === args.id) {
              const message = modelInstallError.message;
              modelInstallError = null;
              throw new Error(message);
            }
            const entry = library.models.find((item) => item.id === args.id);
            if (!entry) throw new Error(`Unknown mock model: ${args.id}`);
            if (entry.state === "checking") throw new Error("Mock guard: model check is still pending.");
            entry.error = null;
            entry.state = "downloading";
            entry.downloadedBytes = Math.floor(entry.totalBytes / 2);
            return clone(library);
          }
          case "cancel_model_install": {
            const index = onboardingQueue.indexOf(args.id);
            if (index >= 0) onboardingQueue.splice(index, 1);
            const entry = library.models.find((item) => item.id === args.id);
            if (!entry) throw new Error(`Unknown mock model: ${args.id}`);
            entry.state = "available";
            entry.downloadedBytes = 0;
            entry.error = null;
            return clone(library);
          }
          case "remove_model": {
            if (!args.confirmed)
              throw new Error("Removal needs a confirmed choice.");
            const entry = library.models.find((item) => item.id === args.id);
            if (!entry) throw new Error(`Unknown mock model: ${args.id}`);
            const selectedRoles = ["asrModel", "cleanupModel", "notesModel"]
              .filter((key) => state.settings[key] === entry.id)
              .join(", ");
            if (selectedRoles)
              throw new Error(
                `Mock guard: ${entry.name} is still selected for ${selectedRoles}. Choose a replacement first.`,
              );
            entry.state = "available";
            entry.downloadedBytes = 0;
            entry.usedBy = [];
            return clone(library);
          }
          case "delete_dictation": {
            if (!args.confirmed)
              throw new Error("Deletion needs a confirmed choice.");
            history = history.filter((item) => item.id !== args.id);
            learned = learned.filter(
              (item) => !item.sourceIds.includes(args.id),
            );
            return clone(state);
          }
          case "delete_all_dictations": {
            if (!args.confirmed)
              throw new Error("Deletion needs a confirmed choice.");
            const deletedCount = history.length;
            history = [];
            learned = learned.filter((item) => !item.userPinned);
            return { deletedCount, snapshot: clone(state) };
          }
          case "count_dictations":
            return history.length;
          case "save_dictation_correction": {
            const record = history.find((item) => item.id === args.id);
            if (!record)
              throw new Error("That dictation is no longer in history.");
            record.published = args.text;
            let learnedCount = 0;
            if (state.settings.learnFromCorrections) {
              learned.push({
                id: `mock-learned-${learnedSeq++}`,
                kind: "correction",
                text: args.text.slice(0, 120),
                heard: null,
                scope: args.scope,
                sourceIds: [args.id],
                sourceType: "dictation-correction",
                enabled: true,
                userPinned: false,
              });
              learnedCount = 1;
            }
            return { record: clone(record), learnedCount };
          }
          case "get_learning":
            return clone(learned);
          case "save_learning":
            learned = clone(args.entries);
            return clone(learned);
          case "reset_learning": {
            if (!args.confirmed)
              throw new Error("Reset needs a confirmed choice.");
            learned = [];
            return null;
          }
          case "list_dictations":
            return clone(history);
          case "get_vocabulary":
            return clone(vocabulary);
          case "save_vocabulary":
            vocabulary = clone(args.entries);
            return clone(vocabulary);
          case "learn_correction":
            vocabulary.push({
              id: "learned",
              heard: args.heard,
              written: args.written,
              enabled: true,
              scope: null,
            });
            return clone(vocabulary);
          case "list_meetings":
            return clone(meetings);
          case "get_meeting":
            return clone(meetings.find((meeting) => meeting.id === args.id));
          case "save_meeting": {
            const meeting = meetings.find((meeting) => meeting.id === args.id);
            Object.assign(meeting, args);
            state.meetingsRevision += 1;
            emit();
            return clone(meeting);
          }
          case "set_meeting_speaker_labels": {
            const meeting = meetings.find((meeting) => meeting.id === args.id);
            if (!args.confirmed && (meeting.cleanedTranscript != null || meeting.transcript.includes("Edited.")))
              throw new Error(
                "Changing speaker labels rebuilds the transcript and replaces your edits.",
              );
            meeting.speakerLabels = args.enabled;
            meeting.cleanedTranscript = null;
            meeting.transcript = args.enabled
              ? "[0:01] Speaker 1: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Speaker 2: Reply."
              : "[0:01] Other speakers: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Other speakers: Reply.";
            state.meetingsRevision += 1;
            emit();
            return clone(meeting);
          }
          case "clean_meeting": {
            const meeting = meetings.find((meeting) => meeting.id === args.id);
            // Fixture output, not an implementation of the native Clean pipeline.
            meeting.cleanedTranscript = meeting.transcript.replace("in, in March", "in March");
            state.meetingsRevision += 1;
            emit();
            return clone(meeting);
          }
          case "generate_notes": {
            const meeting = meetings.find((meeting) => meeting.id === args.id);
            meeting.notes = "- Keep 4.6.\n- Preserve the backup.";
            state.meetingsRevision += 1;
            emit();
            return clone(meeting);
          }
          case "transcribe_meeting": {
            const meeting = meetings.find((meeting) => meeting.id === args.id);
            if (
              args.model === "openrouter" &&
              (!aiSettings.connected || !aiSettings.speechModel)
            )
              throw new Error("Choose an OpenRouter speech model in Setup.");
            const model = args.model === "openrouter"
              ? `openrouter:${aiSettings.speechModel}`
              : args.model ?? (state.settings.meetingAsrModel || state.settings.asrModel);
            meeting.status = "transcribing";
            state.meetingsRevision += 1;
            emit();
            return new Promise((resolve) => {
              completeTranscription = (patch) => {
                Object.assign(meeting, clone(patch), { status: "transcribed", transcriptionModel: model, cleanedTranscript: null });
                pending = undefined;
                completeTranscription = undefined;
                state.meetingsRevision += 1;
                emit();
                resolve(clone(meeting));
              };
              pending = () => {
                completeTranscription = undefined;
                meeting.status = "cancelled";
                state.meetingsRevision += 1;
                emit();
                resolve(clone(meeting));
              };
            });
          }
          case "cancel_meeting":
            pending?.();
            pending = undefined;
            return null;
          case "delete_meeting":
            meetings = meetings.filter((meeting) => meeting.id !== args.id);
            state.meetingsRevision += 1;
            emit();
            return null;
          case "screen_tab_meeting":
          case "start_meeting": {
            if (state.settings.termsAcceptedVersion < 1 || !state.settings.termsAcceptedAt) {
              throw new Error("TERMS_REQUIRED: Open SonoJot and review the Terms of Use to continue.");
            }
            if (state.settings.setupProfile === null) {
              state.error = "SETUP_INCOMPLETE: Finish setup to record meetings.";
              emit();
              throw new Error(state.error);
            }
            if (!state.settings.meetingConsentAcknowledged) {
              throw new Error("MEETING_CONSENT_REQUIRED: Review the recording reminder before starting a meeting.");
            }
            const meeting = {
              ...clone(seed),
              id: "new-meeting",
              title: args.title || "Untitled meeting",
              source: state.settings.meetingSource,
              status: "recording",
              durationSeconds: 0,
              notes: "",
              transcript: "",
              hasTranscript: false,
              cleanedTranscript: null,
              audioOutages: [],
              missingSeconds: 0,
              transcriptionStoppedAtSeconds: null,
            };
            meetings.push(meeting);
            state.meetingsRevision += 1;
            state.meetingRecording = true;
            state.activeMeetingId = meeting.id;
            emit();
            return clone(meeting);
          }
          case "stop_meeting": {
            const meeting = meetings.find(
              (meeting) => meeting.id === state.activeMeetingId,
            );
            meeting.status = "ready";
            meeting.durationSeconds = 31;
            state.meetingsRevision += 1;
            state.meetingRecording = false;
            state.activeMeetingId = null;
            emit();
            return clone(meeting);
          }
          case "import_media": {
            const meeting = {
              ...clone(seed),
              id: "imported",
              title: "Imported recording",
              source: "import",
            };
            meetings.push(meeting);
            state.meetingsRevision += 1;
            emit();
            return clone(meeting);
          }
          case "plugin:dialog|open":
            return "C:/fixture.wav";
          case "plugin:dialog|save": {
            if (dialogSaveQueue.length > 0) {
              const next = dialogSaveQueue.shift();
              if (next === "__HOLD__") {
                return new Promise((resolve) => {
                  heldDialogResolve = resolve;
                });
              }
              if (next instanceof Error) throw next;
              return next;
            }
            return "C:/fixture-export.pdf";
          }
          case "get_diagnostics":
            return "Isolated UI fixture. No native service was contacted.";
          default:
            throw new Error(`Unhandled test command: ${command}`);
        }
      },
    };
  }, { ...options, downloadConsentGroups: downloadConsentCatalog.groups });
}
