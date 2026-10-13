import assert from "node:assert/strict";
import test from "node:test";
import {
  isGpuAsrModel,
  modelAcceleration,
  textGpuName,
  type AccelerationStatus,
} from "./acceleration";

const available: AccelerationStatus = {
  textGpu: {
    available: true,
    backend: "cuda",
    devices: ["NVIDIA GPU"],
    detail: "Compatible GPU",
  },
  textRuntime: null,
  asrRuntime: {
    modelId: null,
    backend: null,
    compatibility: null,
    providerRequested: "cpu",
    providerInitialized: "cpu",
    deviceIndex: null,
    executionProof: null,
  },
  cpuOnly: false,
  speechDetail: "CPU",
};
test("GPU capability and a requested backend do not claim execution", () => {
  assert.equal(
    modelAcceleration("qwen", "cleanup", available, false).gpu,
    false,
  );
  const requested = {
    ...available,
    textRuntime: {
      pid: 1,
      modelId: "qwen",
      backend: "cuda",
      loading: false,
      gpuLayers: null,
      totalLayers: null,
      fallbackReason: null,
    },
  };
  assert.match(
    modelAcceleration("qwen", "cleanup", requested, false).label,
    /unverified/,
  );
  const loaded = {
    ...requested,
    textRuntime: { ...requested.textRuntime, gpuLayers: 37, totalLayers: 37 },
  };
  assert.equal(modelAcceleration("qwen", "cleanup", loaded, false).gpu, true);
  assert.equal(
    modelAcceleration("different-model", "cleanup", loaded, false).gpu,
    false,
  );
  assert.match(
    modelAcceleration("qwen", "cleanup", loaded, true).detail,
    /next model load/,
  );
});
test("CPU fallback, CPU selection and speech packages remain explicit", () => {
  const cpu = {
    ...available,
    textRuntime: {
      pid: 1,
      modelId: "qwen",
      backend: "cpu",
      loading: false,
      gpuLayers: 0,
      totalLayers: 37,
      fallbackReason: "CUDA could not start; using CPU.",
    },
  };
  assert.match(modelAcceleration("qwen", "cleanup", cpu, false).label, /CPU/);
  assert.match(
    modelAcceleration("qwen", "cleanup", cpu, false).detail,
    /CUDA could not start/,
  );
  assert.equal(modelAcceleration("qwen", "cleanup", cpu, false).gpu, false);
  assert.match(
    modelAcceleration("qwen", "cleanup", available, true).label,
    /CPU selected/,
  );
  assert.match(
    modelAcceleration("qwen-asr", "asr", available, false).label,
    /CPU only/,
  );
});
test("ASR is artifact-aware: CUDA packages never claim running, CPU Qwen stays CPU", () => {
  assert.equal(isGpuAsrModel("qwen3-asr-1.7b-fp16-cuda"), true);
  assert.equal(isGpuAsrModel("qwen3-asr-1.7b-q8-cuda"), true);
  assert.equal(isGpuAsrModel("whisper-tiny-fp32-cuda"), true);
  assert.equal(isGpuAsrModel("qwen3-asr-1.7b"), false);
  assert.equal(isGpuAsrModel("moonshine-medium"), false);
  for (const id of [
    "qwen3-asr-1.7b-fp16-cuda",
    "qwen3-asr-1.7b-q8-cuda",
    "whisper-tiny-fp32-cuda",
  ]) {
    const info = modelAcceleration(id, "asr", available, false, "windows");
    assert.equal(info.gpu, false);
    assert.match(info.label, /NVIDIA CUDA/);
    assert.match(info.label, /not loaded/);
    assert.match(info.detail, /Installed, selected and running are separate/);
    assert.match(info.detail, /CPU Qwen int8 remains CPU only/);
    assert.match(info.detail, /AMD is unsupported/);
  }
  const cpuQwen = modelAcceleration(
    "qwen3-asr-1.7b",
    "asr",
    available,
    false,
    "windows",
  );
  assert.match(cpuQwen.label, /CPU only/);
  assert.doesNotMatch(cpuQwen.label, /CUDA package/);
  assert.match(cpuQwen.detail, /AMD is unsupported/);
});
test("resident CUDA runtime reports initialized truthfully, never measured execution", () => {
  const resident = {
    ...available,
    asrRuntime: {
      modelId: "qwen3-asr-1.7b-fp16-cuda",
      backend:
        "sherpa-onnx-1.13.8-cuda13 (OfflineQwen3ASR fp16, transcribe, CUDA provider; VAD CPU)",
      compatibility: "Windows x64 NVIDIA CUDA 13.x + cuDNN 9.x",
      providerRequested: "cuda",
      providerInitialized: "cuda",
      deviceIndex: 0,
      executionProof: null,
    },
  };
  const info = modelAcceleration(
    "qwen3-asr-1.7b-fp16-cuda",
    "asr",
    resident,
    false,
    "windows",
  );
  assert.equal(info.label, "NVIDIA CUDA loaded");
  assert.equal(info.gpu, false);
  assert.match(info.detail, /initialized cuda/);
  assert.match(info.detail, /not measured/);
  assert.match(info.detail, /CPU Qwen int8 remains CPU only/);
  assert.match(info.detail, /AMD is unsupported/);
  // A different resident model leaves this row on the static package label.
  const other = modelAcceleration(
    "whisper-tiny-fp32-cuda",
    "asr",
    resident,
    false,
    "windows",
  );
  assert.match(other.label, /not loaded/);
  // Missing asrRuntime (older backend) still parses and stays static.
  const legacy = { ...available, asrRuntime: undefined };
  const legacyInfo = modelAcceleration(
    "qwen3-asr-1.7b-fp16-cuda",
    "asr",
    legacy,
    false,
    "windows",
  );
  assert.match(legacyInfo.label, /selected/);
});
test("text GPU wording preserves Metal on macOS instead of all-platform CUDA", () => {
  assert.equal(textGpuName("macos", "metal"), "Metal");
  assert.equal(textGpuName("macos", "cuda"), "Metal");
  assert.equal(textGpuName("windows", "cuda"), "NVIDIA CUDA");
  assert.equal(textGpuName("Isolated UI test host", "cuda"), "NVIDIA CUDA");
  const macAvailable: AccelerationStatus = {
    ...available,
    textGpu: {
      available: true,
      backend: "metal",
      devices: ["Apple GPU"],
      detail: "Metal GPU",
    },
  };
  assert.match(
    modelAcceleration("qwen", "cleanup", macAvailable, false, "macos").label,
    /Metal available/,
  );
  assert.match(
    modelAcceleration("qwen", "cleanup", available, false, "windows").label,
    /NVIDIA CUDA available/,
  );
});

test("Qwen Metal and CUDA rows are labelled by their GPU backend, CPU Qwen stays CPU", () => {
  for (const id of ["qwen3-asr-1.7b-q8-metal", "qwen3-asr-1.7b-q4-metal", "whisper-small-metal"]) {
    const result = modelAcceleration(id, "asr", null, false, "macos");
    assert.equal(result.label, "Apple Metal selected", id);
  }
  const loaded = modelAcceleration("qwen3-asr-1.7b-q4-metal", "asr", {
    asrRuntime: { modelId: "qwen3-asr-1.7b-q4-metal", providerInitialized: "metal" },
  } as unknown as AccelerationStatus, false, "macos");
  assert.equal(loaded.label, "Apple Metal initialized");
  assert.equal(modelAcceleration("qwen3-asr-1.7b-q8-cuda", "asr", null, false, "windows").label, "NVIDIA CUDA selected");
  assert.equal(modelAcceleration("qwen3-asr-1.7b-q4-cpu", "asr", null, false, "windows").label, "CPU only");
  assert.equal(modelAcceleration("qwen3-asr-1.7b", "asr", null, false, "windows").label, "CPU only");
});
