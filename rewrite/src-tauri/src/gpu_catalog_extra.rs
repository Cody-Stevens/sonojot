//! Additional CUDA catalog entries; include as a child of `gpu_asr`.
//! Source receipts: artifacts/reliability-setup/gpu-options-pins.json.
//! New entries remain preview until native adapter validation is recorded.
//! Parent verified Small/Parakeet API pins and downloaded both tokenizers.

use super::{GpuFile, GpuModel};

pub const WHISPER_TURBO_FP32_CUDA_ID: &str = "whisper-large-v3-turbo-fp32-cuda";
pub const WHISPER_TURBO_FEATURE_DIM: i32 = 128;
pub const WHISPER_SMALL_FP32_CUDA_ID: &str = "whisper-small-fp32-cuda";
pub const PARAKEET_FP16_CUDA_ID: &str = "parakeet-tdt-0.6b-v2-fp16-cuda";

pub const SMALL_FILES: &[GpuFile] = &[
    GpuFile {
        url_name: "small-encoder.onnx",
        local_name: "encoder.onnx",
        bytes: 409_528_992,
        sha256: "119bd1e8ba0524baee1687f6b22bf0abd2fe539549cd000734edbca81c66751e",
    },
    GpuFile {
        url_name: "small-decoder.onnx",
        local_name: "decoder.onnx",
        bytes: 559_127_829,
        sha256: "a4165cca5c77e381938c0e111032a384901b1e434ae2ad948859035392d21d2c",
    },
    GpuFile {
        url_name: "small-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
    },
];
pub const SMALL: GpuModel = GpuModel {
    id: WHISPER_SMALL_FP32_CUDA_ID, name: "Whisper Small (FP32 CUDA preview)", family: "whisper", languages: "Multilingual",
    repo: "csukuangfj/sherpa-onnx-whisper-small", revision: "8f3c18b358db4d1f2fc1eae49d75cd20989e4309", files: SMALL_FILES,
    license: "MIT (OpenAI Whisper weights; sherpa export/runtime Apache-2.0)", quantization: "fp32",
    backend: "sherpa-onnx-1.13.8-cuda13 (OfflineWhisper fp32, transcribe, CUDA provider; VAD CPU)", compatibility: super::CUDA_COMPATIBILITY,
    guidance: "Preview. Mid-size multilingual option; 969 MB weights. NVIDIA CUDA. Memory and speed are estimates until measured with this adapter.",
    evidence: "2026-09-17: all 969 MB files downloaded and SHA-256 verified in isolation. Actual SonoJot CUDA worker decoded the public 6.625s clip in 0.75s, cold load 36.8s including checks, private host peak 3.29 GB, clean acknowledged shutdown. Debug-build single-clip evidence; VRAM remains estimated, no comparative accuracy claim.",
};

pub const PARAKEET_FILES: &[GpuFile] = &[
    GpuFile {
        url_name: "encoder.fp16.onnx",
        local_name: "encoder.fp16.onnx",
        bytes: 1_239_245_548,
        sha256: "bb1cc16e8223472b36bd637e2f77725d62b9895eaf609fec6cc91183e2753e38",
    },
    GpuFile {
        url_name: "decoder.fp16.onnx",
        local_name: "decoder.fp16.onnx",
        bytes: 14_446_596,
        sha256: "45b04b71eb29dddeeb560e13a515291615bc541f8f5e9e336004956c0ce1a538",
    },
    GpuFile {
        url_name: "joiner.fp16.onnx",
        local_name: "joiner.fp16.onnx",
        bytes: 3_456_459,
        sha256: "5d9d840eff9a3f3724ebce6d84c52038ee4664a08955c5a69d277f2d9733f9b1",
    },
    GpuFile {
        url_name: "tokens.txt",
        local_name: "tokens.txt",
        bytes: 9_384,
        sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
    },
];
pub const PARAKEET: GpuModel = GpuModel {
    id: PARAKEET_FP16_CUDA_ID, name: "Parakeet TDT v2 (FP16 CUDA preview)", family: "parakeet", languages: "English",
    repo: "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16", revision: "a6cc4e24ed264a93a17575dad416dacc4238497a", files: PARAKEET_FILES,
    license: "CC-BY-4.0 (NVIDIA Parakeet TDT 0.6B v2; converted to ONNX FP16 by sherpa-onnx; https://creativecommons.org/licenses/by/4.0/). Runtime Apache-2.0.", quantization: "fp16",
    backend: "sherpa-onnx-1.13.8-cuda13 (OfflineNemoTransducer fp16, CUDA provider; VAD CPU)", compatibility: super::CUDA_COMPATIBILITY,
    guidance: "Preview. English dictation with punctuation and case; 1.26 GB weights. NVIDIA CUDA. Memory and speed need adapter measurement.",
    evidence: "2026-09-17: all 1.26 GB files downloaded and SHA-256 verified in isolation. Actual SonoJot CUDA worker decoded the public 6.625s clip in 0.85s, cold load 40.6s including checks, private host peak 4.29 GB, clean acknowledged shutdown. Debug-build single-clip evidence; VRAM remains estimated, no comparative accuracy claim.",
};

pub const TURBO_FILES: &[GpuFile] = &[
    GpuFile {
        url_name: "turbo-encoder.onnx",
        local_name: "encoder.onnx",
        bytes: 735_920,
        sha256: "1b960f278564fb8bbacd544d4f85f4dd6d8a64d3aa89543d8f2c4021c926f976",
    },
    GpuFile {
        url_name: "turbo-encoder.weights",
        // Referenced inside the ONNX graph. Renaming the graph is safe;
        // renaming this sidecar without rewriting the graph is not.
        local_name: "turbo-encoder.weights",
        bytes: 2_600_325_120,
        sha256: "746f879ecf066450ab0cdecc05383380b85157270ff6c0a9fb7cfdd917036e12",
    },
    GpuFile {
        url_name: "turbo-decoder.onnx",
        local_name: "decoder.onnx",
        bytes: 636_209_532,
        sha256: "b24db5d90fa230c5eaa6b823d74862ced9e0d1dc3e01ec46601968e8db0e09ec",
    },
    GpuFile {
        url_name: "turbo-tokens.txt",
        local_name: "tokens.txt",
        bytes: 816_730,
        sha256: "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
    },
];

pub const TURBO: GpuModel = GpuModel {
    id: WHISPER_TURBO_FP32_CUDA_ID,
    name: "Whisper Large v3 Turbo (FP32 CUDA preview)",
    family: "whisper",
    languages: "Multilingual",
    repo: "csukuangfj/sherpa-onnx-whisper-turbo",
    revision: "2ca6ff69fc878651b770880507669577ac41c2ff",
    files: TURBO_FILES,
    license: "MIT (OpenAI Whisper weights; sherpa ONNX export/runtime Apache-2.0)",
    quantization: "fp32",
    backend: "sherpa-onnx-1.13.8-cuda13 (OfflineWhisper fp32, transcribe, CUDA provider; VAD CPU)",
    compatibility: super::CUDA_COMPATIBILITY,
    guidance: "Preview. 3.24 GB model download plus the shared CUDA runtime. NVIDIA only. CUDA inference, load time, host RAM and VRAM requirements are unmeasured for this export.",
    evidence: "2026-09-17: graph sizes and SHA-256 pins verified against the upstream revision's LFS commit; cached same-revision tokens rehashed. FP32 graphs were not downloaded or executed. SonoJot adapter integration remains unverified.",
};
