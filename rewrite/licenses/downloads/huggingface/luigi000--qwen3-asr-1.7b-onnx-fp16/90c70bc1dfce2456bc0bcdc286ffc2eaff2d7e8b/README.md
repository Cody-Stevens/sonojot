---
license: apache-2.0
library_name: sherpa-onnx
pipeline_tag: automatic-speech-recognition
tags:
  - sherpa-onnx
  - onnx
  - fp16
  - qwen3-asr
  - speech-recognition
---

# Qwen3-ASR-1.7B — sherpa-onnx fp16 export

An **fp16 ONNX** export of [Qwen/Qwen3-ASR-1.7B](https://huggingface.co/Qwen/Qwen3-ASR-1.7B),
packaged for [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)
(`OfflineRecognizer.from_qwen3_asr`). Intended for **GPU** inference (CUDA execution provider):
~4.5 GB on disk / ~4–5 GB VRAM, roughly half the fp32 export, with full-precision accuracy
(fp16 is numerically lossless for this model — validated equal to fp32 to the decimal).

> **Why fp16, not int8?** Dynamic INT8 quantization severely degrades this 1.7B model
> (e.g. FLEURS Vietnamese WER 5.9 → 18.3). fp16 preserves accuracy; int8 is not offered here.

## Files

```
conv_frontend.onnx        # audio frontend (fp32)
encoder.onnx  (+ .data)   # fp16
decoder.onnx  (+ .data)   # fp16
tokenizer/                # copied unchanged from the base checkpoint
```

## Usage (sherpa-onnx, Python)

```python
import sherpa_onnx
rec = sherpa_onnx.OfflineRecognizer.from_qwen3_asr(
    conv_frontend="conv_frontend.onnx",
    encoder="encoder.onnx",
    decoder="decoder.onnx",
    tokenizer="tokenizer",
    provider="cuda",            # fp16 targets GPU
    num_threads=2,
    sample_rate=16000,
    max_total_len=4096,         # defaults (512/128) silently truncate long turns
    max_new_tokens=1024,
)
s = rec.create_stream()
s.accept_waveform(16000, audio_f32)   # 16 kHz mono float32
rec.decode_stream(s)
print(s.result.text)
```

## How this export was produced

1. Official `Qwen/Qwen3-ASR-1.7B` safetensors → ONNX (fp32) via the pipeline at
   [Wasser1462/Qwen3-ASR-onnx](https://github.com/Wasser1462/Qwen3-ASR-onnx)
   (conv_frontend, encoder, decoder graphs).
2. fp32 → fp16 with **`onnxruntime.transformers.optimizer.optimize_model(model_type="qwen3")`**,
   which fuses the RMSNorm/attention primitives into fused ops with correct fp16 kernels, then
   `OnnxModel.convert_float_to_float16(keep_io_types=True)`. (Fusing before converting is required —
   the standalone float16 converters produce an unloadable graph on the raw RMSNorm primitives.)

No fine-tuning or weight modification beyond the fp32→fp16 cast.

License: Apache-2.0 (inherited from the base model). See `LICENSE` and `NOTICE`.
