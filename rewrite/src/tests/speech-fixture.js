export const qwen = "qwen3-asr-1.7b-q8-cuda",
  parakeet = "parakeet-tdt-0.6b-v2",
  cpu = "qwen3-asr-1.7b-q4-cpu",
  granite = "granite-speech-4.1-2b-nar-q8";
export function model(id, name, ram, vram = 0) {
  return {
    id,
    name,
    kind: "asr",
    roles: id === granite ? ["meetingSpeech"] : ["asr"],
    family: name,
    languages: ["English"],
    older: false,
    diskBytes: 1.7e9,
    ramMinBytes: ram * 0.8,
    ramMaxBytes: ram,
    vramMinBytes: vram * 0.8,
    vramMaxBytes: vram,
    memoryBasis: "Fixture planning estimate",
    guidance: "Fixture",
    evidence: "Fixture",
    backend: vram ? "CUDA" : "CPU",
    compatibility: "Fixture",
    state: "installed",
    downloadedBytes: 1.7e9,
    totalBytes: 1.7e9,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "fixture",
    quantization: "Q8",
    contextTokens: null,
  };
}
export function pc(gpu = true) {
  const recommendation = {
    asrModel: gpu ? qwen : parakeet,
    cleanupModel: "local-rules",
    notesModel: "qwen3-4b-instruct-2507",
    reasons: [],
  };
  return {
    firstRun: false,
    capabilities: { platform: "windows" },
    settings: {
      ...recommendation,
      setupProfile: "balanced",
      localProofreader: false,
    },
    hardwareProfile: {
      hardware: {
        platform: "windows",
        appleSilicon: false,
        totalRamBytes: 32e9,
        physicalCores: 8,
        logicalCores: 16,
        gpuName: gpu ? "NVIDIA RTX 5070" : null,
        gpuDriverVersion: null,
        gpuComputeCapability: gpu ? "12.0" : null,
        totalVramBytes: gpu ? 12e9 : null,
        cudaOk: gpu,
      },
      recommendations: {
        accurate: recommendation,
        balanced: recommendation,
        light: recommendation,
      },
      frontier: {
        dictation: gpu ? [parakeet, qwen] : ["moonshine-medium", parakeet, cpu],
        meetings: gpu
          ? [parakeet, qwen, granite]
          : ["moonshine-medium", parakeet, cpu],
        otherLanguages: gpu ? qwen : cpu,
      },
    },
    libraryHardware: {
      totalRamBytes: 32e9,
      dedicatedVramBytes: gpu ? 12e9 : null,
      availableVramBytes: gpu ? 9e9 : null,
    },
    libraryModels: [
      model(qwen, "Qwen3-ASR 1.7B · GPU", 1.68e9, 4.4e9),
      model(parakeet, "Parakeet TDT 0.6B v2", 2.1e9),
      model(cpu, "Qwen3-ASR 1.7B · CPU", 2.52e9),
      model(granite, "Granite Speech 4.1", 2.94e9, 5.2e9),
      {
        ...model("moonshine-tiny-streaming", "Moonshine Tiny Streaming", 80e6),
        roles: ["livePreview"],
      },
    ],
    modelPatches: {
      [qwen]: { supported: gpu },
      [granite]: {
        supported: gpu,
        error: gpu
          ? null
          : "Needs a compatible NVIDIA GPU; not measured on this computer",
      },
      "qwen3-4b-instruct-2507": { state: "installed" },
    },
  };
}
