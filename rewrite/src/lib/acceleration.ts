import { z } from "zod";

export const asrRuntimeSchema = z.object({
  modelId: z.string().nullable(),
  backend: z.string().nullable(),
  compatibility: z.string().nullable(),
  providerRequested: z.string(),
  providerInitialized: z.string(),
  deviceIndex: z.number().int().nonnegative().nullable(),
  executionProof: z.string().nullable(),
});
export type AsrRuntime = z.infer<typeof asrRuntimeSchema>;

export const accelerationSchema = z.object({
  textGpu: z.object({
    available: z.boolean(),
    backend: z.string(),
    devices: z.array(z.string()),
    detail: z.string(),
  }),
  textRuntime: z
    .object({
      pid: z.number().int(),
      modelId: z.string(),
      backend: z.string(),
      loading: z.boolean(),
      gpuLayers: z.number().int().nonnegative().nullable(),
      totalLayers: z.number().int().positive().nullable(),
      fallbackReason: z.string().nullable(),
    })
    .nullable(),
  asrRuntime: asrRuntimeSchema.nullable().optional(),
  cpuOnly: z.boolean(),
  speechDetail: z.string(),
});
export type AccelerationStatus = z.infer<typeof accelerationSchema>;

/** NVIDIA CUDA speech IDs from `gpu_asr.rs` and `llama_asr.rs` (never CPU). */
export const GPU_ASR_IDS = [
  "qwen3-asr-1.7b-fp16-cuda",
  "qwen3-asr-1.7b-q8-cuda",
  "whisper-tiny-fp32-cuda",
  "whisper-small-fp32-cuda",
  "whisper-large-v3-turbo-fp32-cuda",
  "parakeet-tdt-0.6b-v2-fp16-cuda",
] as const;
/** CPU Qwen int8 package; must never be labelled GPU. */
export const CPU_QWEN_ASR_ID = "qwen3-asr-1.7b";

export function isGpuAsrModel(modelId: string): boolean {
  // Every NVIDIA speech row ends in -cuda (gpu_asr.rs, llama_asr.rs).
  return (
    (GPU_ASR_IDS as readonly string[]).includes(modelId) ||
    modelId.endsWith("-cuda")
  );
}

/** Apple Metal speech rows (mac_speech.rs Whisper, llama_asr.rs Qwen) end in -metal. */
export function isMetalAsrModel(modelId: string): boolean {
  return modelId.endsWith("-metal");
}

/**
 * Display name for the text GPU path: Windows CUDA, macOS Metal, Linux Vulkan.
 * Unknown hosts keep the
 * Windows primary wording so existing checks stay explicit.
 */
export function textGpuName(
  platform?: string | null,
  backend?: string | null,
): string {
  const p = (platform ?? "").toLowerCase();
  const b = (backend ?? "").toLowerCase();
  if (p === "macos" || p.startsWith("mac") || b === "metal") return "Metal";
  if (p === "linux" || b === "vulkan") return "Vulkan";
  return "NVIDIA CUDA";
}

export function modelAcceleration(
  modelId: string,
  kind: "asr" | "cleanup",
  status: AccelerationStatus | null,
  cpuSelected: boolean,
  platform?: string | null,
): { label: string; detail: string; gpu: boolean } {
  if (kind === "asr") {
    if (isMetalAsrModel(modelId)) {
      const initialized =
        status?.asrRuntime?.modelId === modelId &&
        status.asrRuntime.providerInitialized === "metal";
      return {
        label: initialized ? "Apple Metal initialized" : "Apple Metal selected",
        detail: initialized
          ? "The speech model initialized on the Apple GPU through Metal. Per-decode GPU activity is not measured."
          : "This model uses the Apple Metal backend when loaded.",
        gpu: initialized,
      };
    }
    if (isGpuAsrModel(modelId)) {
      if (!status?.asrRuntime)
        return {
          label: "NVIDIA CUDA selected",
          gpu: false,
          detail:
            "The GPU model is selected. Its current loading state is not available yet.",
        };
      // Structured runtime identity (backend asrRuntime): a resident CUDA
      // model reports its initialized backend truthfully instead of hiding
      // behind the static unverified label. Requested provider is selection
      // only; executionProof is null in this build, so nothing here claims
      // measured per-decode GPU execution.
      const resident =
        status?.asrRuntime?.modelId === modelId ? status.asrRuntime : null;
      if (resident && resident.providerInitialized === "cuda") {
        const device =
          resident.deviceIndex === null || resident.deviceIndex === undefined
            ? ""
            : ` (CUDA device ${resident.deviceIndex})`;
        return {
          label: "NVIDIA CUDA loaded",
          gpu: false,
          detail:
            `Speech model ${modelId} is loaded in the NVIDIA CUDA runtime` +
            `${device} (${resident.backend ?? "CUDA backend"}). Requested ` +
            `${resident.providerRequested}, initialized ${resident.providerInitialized}; ` +
            `per-decode GPU execution is not measured in this build. ` +
            `CPU Qwen int8 remains CPU only. AMD is unsupported.`,
        };
      }
      return {
        label: "NVIDIA CUDA · not loaded",
        gpu: false,
        detail:
          "Loads when recording starts; microphone audio is buffered while it loads. Installed, selected and running are separate states. CPU Qwen int8 remains CPU only. AMD is unsupported.",
      };
    }
    return {
      label: "CPU only",
      gpu: false,
      detail:
        "This speech package uses the CPU. GPU acceleration is not included in its installed runtime. AMD is unsupported.",
    };
  }
  if (modelId === "local-rules")
    return {
      label: "CPU",
      gpu: false,
      detail: "SonoJot Punctuation and your vocabulary run on the CPU.",
    };
  if (!status)
    return { label: "Checking acceleration…", detail: "", gpu: false };
  const gpuName = textGpuName(
    platform,
    status.textRuntime?.backend ?? status.textGpu.backend,
  );
  const runtime =
    status.textRuntime?.modelId === modelId ? status.textRuntime : null;
  if (runtime) {
    const gpu = runtime.backend !== "cpu" && (runtime.gpuLayers ?? 0) > 0;
    const count = gpu
      ? `${runtime.gpuLayers}/${runtime.totalLayers ?? "?"} layers on GPU.`
      : "";
    const next =
      (cpuSelected || status.cpuOnly) && runtime.backend !== "cpu"
        ? "CPU only takes effect on the next model load."
        : "";
    const runtimeGpuName = textGpuName(platform, runtime.backend);
    return {
      label: runtime.loading
        ? runtime.backend === "cpu"
          ? "Loading on CPU…"
          : `Loading for ${runtimeGpuName}…`
        : gpu
          ? `GPU accelerated · ${runtimeGpuName}`
          : runtime.backend === "cpu" || runtime.gpuLayers === 0
            ? "CPU · model loaded"
            : `${runtimeGpuName} requested · offload unverified`,
      detail:
        [count, runtime.fallbackReason, next].filter(Boolean).join(" ") ||
        (runtime.backend === "cpu"
          ? "The loaded model is using system memory and CPU processing."
          : "GPU use is confirmed only after the runtime reports offloaded model layers."),
      gpu,
    };
  }
  if (cpuSelected || status.cpuOnly)
    return {
      label: "CPU selected · model not loaded",
      gpu: false,
      detail:
        "The next load will use the CPU. The downloaded model files stay the same.",
    };
  return {
    label: status.textGpu.available
      ? `${gpuName} available · model not loaded`
      : `CPU · ${gpuName} unavailable`,
    detail: status.textGpu.detail,
    gpu: false,
  };
}
