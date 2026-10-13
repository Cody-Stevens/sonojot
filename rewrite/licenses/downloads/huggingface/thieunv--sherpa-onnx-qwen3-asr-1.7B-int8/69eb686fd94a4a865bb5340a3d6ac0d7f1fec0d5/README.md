---
license: apache-2.0
language:
- multilingual
- zh
- en
- ja
- vi
- ko
- yue
library_name: sherpa-onnx
tags:
- automatic-speech-recognition
- qwen3-asr
- sherpa-onnx
- onnx
- int8
pipeline_tag: automatic-speech-recognition
---

# sherpa-onnx-qwen3-asr-1.7B-int8

INT8 ONNX build of **Qwen3-ASR-1.7B** in the [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)
`from_qwen3_asr` layout, for on-device multilingual speech recognition (no Python at runtime).

- Base model: [`Qwen/Qwen3-ASR-1.7B`](https://huggingface.co/Qwen/Qwen3-ASR-1.7B) (Apache-2.0)
- Exported to sherpa-onnx format with
  [Wasser1462/Qwen3-ASR-onnx](https://github.com/Wasser1462/Qwen3-ASR-onnx)
  (`export_qwen3_asr_onnx.py --device cpu --max-total-len 512`, INT8 dynamic quantization).
- Languages: 30+ incl. Chinese (+dialects), English, Japanese, Vietnamese, Korean, Cantonese, …
- Higher accuracy than the 0.6B build at the cost of size/speed.

## Files
```
conv_frontend.onnx        # log-mel / conv frontend
encoder.int8.onnx         # audio encoder (int8)
decoder.int8.onnx         # text decoder w/ KV cache (int8, ~2 GB)
tokenizer/vocab.json
tokenizer/merges.txt
tokenizer/tokenizer_config.json
```

## Usage (sherpa-onnx)
```python
import sherpa_onnx
rec = sherpa_onnx.OfflineRecognizer.from_qwen3_asr(
    conv_frontend="conv_frontend.onnx",
    encoder="encoder.int8.onnx",
    decoder="decoder.int8.onnx",
    tokenizer="tokenizer",
    num_threads=4,
)
s = rec.create_stream()
s.accept_waveform(16000, samples_f32_mono)  # 16 kHz mono float32
rec.decode_stream(s)
print(s.result.text)
```
Requires `sherpa-onnx >= 1.13.4` (adds `from_qwen3_asr`).

## License
Apache-2.0, inherited from Qwen/Qwen3-ASR-1.7B. Attribution to the Qwen team (Alibaba Cloud)
and the sherpa-onnx / export-script authors.
