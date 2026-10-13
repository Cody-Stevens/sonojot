---
license: openmdw-1.1
license_link: https://openmdw.ai/license/1-1/
base_model: nvidia/Nemotron-3-Diarization
tags:
  - onnx
  - speaker-diarization
pipeline_tag: voice-activity-detection
---

# Nemotron-3-Diarization-ONNX

An ONNX export of [nvidia/Nemotron-3-Diarization](https://huggingface.co/nvidia/Nemotron-3-Diarization)
that runs on CPU with `numpy` + `onnxruntime` + `soundfile` only -- no PyTorch. Offline
(whole-recording) mode: up to 8 concurrently tracked speakers, ordered by first arrival,
10 ms frame resolution.

This is an independent conversion, not produced or endorsed by NVIDIA. See
[NOTICE](NOTICE) and [LICENSE](LICENSE).

## Quick start

```bash
pip install -r requirements.txt
python diarize.py meeting.wav --out meeting.rttm
```

```python
import soundfile as sf
from diarize import OnnxDiarizer, extract_speaker_dict, segments_to_rttm

audio, sr = sf.read("meeting.wav", dtype="float32")

diarizer = OnnxDiarizer(threads=8)                     # default graph: model.int8.onnx
probs = diarizer.diarize(audio, sample_rate=sr)         # (1, num_frames, 8), one row per 10 ms
segments = extract_speaker_dict(probs)                  # [{"Start":.., "End":.., "Speaker":..}, ...]

open("meeting.rttm", "w").write(segments_to_rttm(segments, uri="meeting"))
```

`diarize(audio, sample_rate=sr)` raises `ValueError` if `sr != 16000` (resample first -- every
frame-rate constant downstream is hard-coded to 16 kHz, so a wrong rate would otherwise
produce plausible-looking garbage, not an error) and downmixes multi-channel audio
(`soundfile`'s `(frames, channels)` shape) to mono automatically.

Pass `--model model.onnx` (CLI) or `model_filename="model.onnx"` (`OnnxDiarizer`) for the
fp32 graph instead of the int8 default.

## Files

| File | Size | Purpose |
|---|---|---|
| `preprocessor_core.onnx` | 0.14 MB | waveform -> log-mel spectrogram (bounded-memory windowed calls, see `diarize.py`) |
| `model.onnx` | 397.4 MB | fp32 chunk-step graph: one 30.4 s step of the encoder + head |
| `model.int8.onnx` | 103.7 MB | int8 (dynamic, per-channel) chunk-step graph -- the default |
| `constants.npz` | 0.01 MB | speaker-cache config and the `silence_embeds` weight `diarize.py` needs |
| `diarize.py` | -- | the offline loop, the speaker cache (numpy port), and the CLI |
| `requirements.txt` | -- | `numpy`, `onnxruntime`, `soundfile`, pinned minimums |
| `scripts/` | -- | the export/quantize/verification pipeline that produced the three graphs above -- see `scripts/README.md` |
| `docs/RESULTS.md` | -- | every number in this README, with tolerances and full per-check detail |

Sizes are decimal MB (10<sup>6</sup> bytes) throughout this document. The three `.onnx`
files have had exporter debug metadata (stack traces containing local file paths from the
machine they were built on) stripped -- `scripts/strip_metadata.py`, verified to leave
`onnxruntime`'s outputs bit-identical.

## Accuracy

Benchmarked under NVIDIA's own model-card protocol so these numbers compare directly: all
16 AMI `test`-split meetings, Mix-Headset, forced-alignment ground truth
([nttcslab-sp/diar-forced-alignment](https://github.com/nttcslab-sp/diar-forced-alignment),
CC BY 4.0), collar 0, overlap scored, 0.5 threshold.

| System | DER | Miss | FA | Confusion |
|---|---|---|---|---|
| PyTorch (reference) | 9.226% | 4.68% | 3.67% | 0.88% |
| This export, fp32 | 9.225% | 4.68% | 3.67% | 0.88% |
| This export, int8 | 9.248% | 4.80% | 3.58% | 0.87% |

The model card reports 9.25% DER under this protocol; the PyTorch reference reproduced here
gets 9.226% -- this export's fp32 and int8 graphs stay within 0.02 and 0.03 points of that.
Per-meeting, int8 vs. PyTorch ranges from -0.14 to +0.28 points DER across the 16 meetings
(worst cases: IS1009b -0.14, IS1009c +0.28).

**int8 accuracy criterion, and why it has two parts.** The original bar for shipping int8
was DER(int8, fp32) ≤ 1.0% absolute per file (±0.25s collar, overlap scored) on every test
file. That held on every file within the model's 8-speaker capacity, but failed on both
test files with more real speakers than the model has output slots for: 13 speakers scored
3.70%, 14 speakers scored 11.97% (see "Limitations" below for the full per-file table).
Investigating why found that past capacity, a single int8 rounding difference can flip
which frame a genuine, exact score tie in the speaker cache's compression step keeps --
after that point fp32 and int8 take different but equally valid paths, not one more
correct than the other (the same tie-break mechanism the "Parity with PyTorch" section
below describes for fp32 ONNX vs. PyTorch itself). Comparing int8 to fp32 directly is the
wrong test once that's true, so the criterion was revised, after seeing this result, to:
**for files within capacity, DER(int8, fp32) ≤ 1.0% still applies; for files with more
speakers than the model's 8 slots, int8 must be no worse than fp32 against real ground
truth, +0.5 points tolerance.** See "Limitations" below for the measurements this produced.

## Parity with PyTorch

Two kinds of comparison, on ES2004a (17.5 min) and TS3006d (49.5 min, a longer meeting used
as a cache-compression stress test):

- **Teacher-forced** (the ONNX cache replays PyTorch's exact tie-break choices, isolating
  everything else in the port): max |Δ probability| 1.4e-05 / 1.3e-05, 100.000% frame
  agreement at the 0.5 threshold, on both files.
- **Free-running** (each side breaks its own ties): DER(this export vs. PyTorch) 0.000% /
  0.035%, both well under the 0.5% tolerance this project holds itself to.

The small free-running gap exists because of one specific, understood mechanism: this
model's streaming speaker cache keeps only its `speaker_cache_length` most important frames
per compression step, chosen by a `top-k` score. When two frames score exactly equal, the
PyTorch CPU implementation and this export's tie-break can legitimately pick different
frames (verified: PyTorch's tie order is not simply "lowest index" or any other simple rule
we could find). From that point the two runs follow different but equally correct
trajectories -- not a numeric error, a genuine tie.

## Limitations

- **8-speaker cap.** The model tracks at most 8 concurrent speakers (an architectural
  limit, not specific to this export). Audio with more real speakers than that forces the
  cache to keep evicting/reassigning slots under contention, and diarization quality drops
  accordingly for the excess speakers -- this is true of the original PyTorch model too.
- **int8 vs. fp32 divergence past 8 speakers.** Under that same contention, a single int8
  rounding difference can flip a genuine cache-compression tie, so the fp32 and int8 graphs
  can diverge more than on lower-speaker-count audio (see "Accuracy" above for why that's a
  different question from accuracy). Measured on 5 VoxConverse clips spanning the boundary
  (collar ±0.25s, overlap scored):

  | File | Speakers | DER(int8, fp32) | DER vs. ground truth: fp32 / int8 |
  |---|---|---|---|
  | eoyaz | 8 | 0.28% | 3.83% / 3.89% |
  | hcyak | 8 | 0.08% | 3.16% / 2.86% |
  | clfcg | 9 | 0.26% | 26.32% / 26.18% |
  | aggyz | 13 | 3.70% | 18.35% / 16.66% |
  | qeejz | 14 | 11.97% | 33.56% / 31.41% |

  At or near the 8-speaker cap, int8 and fp32 agree to well under 1%. Well past it, they
  can differ substantially in which specific frames they attribute to which speaker. This
  is stated neutrally: on the two over-capacity files measured here int8 was not found to
  be less accurate against real ground truth than fp32, but two data points is not enough
  to claim int8 is *more* accurate in general either -- treat "possibly different, not
  proven worse" as the honest summary, not "int8 is better." A 31-layer mixed-precision
  sensitivity sweep found no smaller-than-fp32 variant that removes the divergence on one
  over-capacity file without introducing a new one on a different file, so it isn't fixed
  in this release.
- **Offline mode only.** This export targets the 30.4 s-chunk offline path (a whole
  recording, or a long segment of one, processed at once). The model's low-latency
  streaming mode (sub-second chunks) is architecturally the same step graph at different
  chunk sizes, but is not exported here.
- **RTF excludes model load.** The speed numbers below are wall time / audio duration for
  the diarization call only, not `OnnxDiarizer.__init__`'s model load, which the Speed
  table reports separately (measured with the OS file cache warm -- there is no way to
  measure a genuinely cold load without root, so "warm" is stated honestly rather than
  claiming "cold" or dropping the number).
- **Memory scales with file length -- not flat, and this is expected.** A whole file's mel
  spectrogram and frame-level logits are inherently arrays sized to the recording's
  duration, held in memory for the whole call; the preprocessing step (which used to scale
  much worse than that inherent minimum, from an unbounded STFT computation over the whole
  waveform at once) now processes bounded ~60 s windows instead (`OnnxDiarizer.log_mel`).
  Measured slope (fit between the 17.5-minute and 49.5-minute files in the Speed table, both
  at 4 threads):

  | Graph | Slope | Fixed baseline | 2-hour-file estimate |
  |---|---|---|---|
  | fp32 | 8.8 MB/min | 844 MB | ~1.9 GB |
  | int8 | 8.0 MB/min | 428 MB | ~1.4 GB |

- One file in the wider test set used to validate this export, TS3006d, is in AMI's `train`
  split (which NVIDIA trained on) and is used only as a long-file/cache-compression stress
  test, not as a held-out accuracy figure the way the 16 AMI-test meetings above are.

## Speed

CPU: i9-13900HX (24 cores / 32 threads), Linux, run alone with no other load. RTF excludes
model load; "Load" is `OnnxDiarizer.__init__`'s wall time with the OS file cache warm (see
"Limitations" for why this isn't labelled "cold").

| File | Graph | Threads | RTF | Peak RSS | Load (warm cache) |
|---|---|---|---|---|---|
| ES2004a (17.5 min) | fp32 | 1 | 0.049 | 1025 MB | 0.42 s |
| ES2004a | fp32 | 4 | 0.018 | 999 MB | 0.47 s |
| ES2004a | fp32 | 8 | 0.014 | 1020 MB | 0.57 s |
| ES2004a | int8 | 1 | 0.026 | 570 MB | 0.19 s |
| ES2004a | int8 | 4 | 0.011 | 568 MB | 0.25 s |
| ES2004a | int8 | 8 | 0.008 | 571 MB | 0.28 s |
| TS3006d (49.5 min) | fp32 | 4 | 0.022 | 1282 MB | 0.56 s |
| TS3006d | int8 | 4 | 0.012 | 824 MB | 0.26 s |

PyTorch reference: RTF 0.054 at 8 threads. int8 is **1.68-1.94x faster** than fp32 across
these four file/thread-count pairs and uses noticeably less memory; fp32 ONNX at a single
thread (RTF 0.049) already beats the PyTorch reference's own 8-thread number (0.054), and
int8 at a single thread (0.026) beats it by roughly 2x.

## How it was made

- The reference PyTorch model's `forward()` is one Python loop over 30.4 s chunks with a
  speaker cache mutated between iterations -- not exportable as one static graph, since its
  trip count depends on audio length and the cache's compression branches on Python ints.
- Split at that loop boundary: `preprocessor_core.onnx` (waveform -> log-mel) and
  `model.onnx`/`model.int8.onnx` (one chunk step: mel + speaker-cache context in, logits +
  new cache embeddings out) are static graphs; the outer chunking loop and the speaker
  cache (FIFO push/pop, score, boost, compress) are a plain numpy port in `diarize.py`.
- Exported with `torch.onnx.export(dynamo=True)`, opset 18, dynamic axes on chunk length,
  context length and waveform length.
- `attn_implementation="eager"` and an explicitly materialized attention mask -- the
  reference's own mask-building helper skips masking entirely when it sees no padding at
  trace time, which would silently bake "no mask" into a static graph.
- int8: `onnxruntime.quantization.quantize_dynamic`, MatMul/Gemm nodes, per-channel;
  `preprocessor_core.onnx` stays fp32 (no meaningful MatMul/Gemm to quantize, at 0.14 MB).
- Reference implementation: `transformers` git commit `4b28d51d0d5f17ec20c23a187d0475a8e68810c8`.
- The exact scripts, and full design notes, source-code citations and every number in this
  card: `scripts/` in this package (`scripts/README.md`).

## Licence and attribution

[LICENSE](LICENSE): the OpenMDW License Agreement, version 1.1 -- the official plain-text
file from [github.com/OpenMDW/OpenMDW](https://github.com/OpenMDW/OpenMDW/blob/main/1.1/LICENSE.OpenMDW-1.1)
(see [NOTICE](NOTICE) for the exact commit and SHA-256). This applies to the whole package,
not only the converted graphs: the Model Materials carry it because the upstream model does,
and `diarize.py`, `scripts/` and every other original file here carry it as this project's
own choice of licence, one licence for the package rather than two. Each shipped `.py` file
says so in its own header (`SPDX-License-Identifier: LicenseRef-OpenMDW-1.1`).
[NOTICE](NOTICE): origin of the Model Materials this conversion is derived from, and how the
conversion was made.

This is an independent, community-produced conversion. It is not affiliated with, or
endorsed by, NVIDIA Corporation, and makes no claim to NVIDIA's trademarks.
