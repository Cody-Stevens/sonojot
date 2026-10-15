# Build the Moonshine Windows DLL

The Windows speech bridge uses the official [Moonshine v0.1.5 release](https://github.com/moonshine-ai/moonshine/releases/tag/v0.1.5), asset `moonshine-voice-windows-x86_64.tar.gz`. That asset contains static C++ libraries and `onnxruntime.dll`. `build-moonshine-dll.cmd` links the existing native objects into a DLL exposing the ten C functions used by `asr.rs`; it does not compile or ship a Python or .NET runtime.

Extract the release into `models/rewrite/moonshine/runtime/win-x64`, preserving its `include` and `lib` directories, then run:

```powershell
rewrite/scripts/build-moonshine-dll.cmd
```

The script accepts an alternative model root as its first argument. It locates Visual Studio with `vswhere`, targets x64, and writes `moonshine.dll` beside `onnxruntime.dll`. The application loads that exact directory without changing the process-wide DLL search path. Build machines require Visual Studio C++ build tools; installed apps only require the DLLs and the Visual C++ runtime.

The ABI comes from the release's [moonshine-c-api.h](https://github.com/moonshine-ai/moonshine/blob/v0.1.5/core/moonshine-c-api.h), header version `30000`. Rust checks the major ABI before calling the library. The 64-bit transcript-line structure is 88 bytes, with speaker-span pointers beginning at byte 48. Keep the header and Rust layout synchronized when changing runtime versions.

Moonshine's code and English models use the [upstream MIT license](https://github.com/moonshine-ai/moonshine/blob/v0.1.5/LICENSE); ONNX Runtime uses its own [MIT license and third-party notices](https://github.com/microsoft/onnxruntime/blob/main/LICENSE). Retain the upstream notices when distributing binaries. The model provision manifest records download URLs, revisions, sizes, and SHA-256 hashes. The cleanup model has separate Liquid AI license terms supplied with its model bundle.

The Rust runtime recognizes `paths.json` when present and otherwise resolves the documented bundle layout. `MOONSHINE_ORT_SINGLE_THREAD=1` must be set before application threads start; this uses Moonshine's supported ONNX Runtime thread restriction. Medium Streaming is the default, with Small Streaming used only if Medium cannot initialize. The model remains loaded across dictation sessions.

Cleanup starts the native llama.cpp server on a random loopback port with a per-process API key and no web UI. Windows prefers the CUDA 13.3 bundle when it loads successfully, then falls back to the CPU bundle. `SONOJOT_CLEANUP_CPU=1` forces CPU for diagnostics (`EAVESDROPPER_CLEANUP_CPU` remains a fallback alias). macOS attempts GPU offload using the native server and falls back to CPU; that path has not been tested on the user's Mac. The model's response can only select host-owned boundary edits; unsupported or late output cannot modify published text.

On Windows the server belongs to a job object with `KILL_ON_JOB_CLOSE`, so it exits when the app's process exits, including a crash. Normal shutdown also stops the owned child directly. Meeting notes use bounded source chunks and selected source sentence IDs; the host renders literal evidence instead of accepting invented summary prose.

## Linux x86_64

Setup downloads native runtimes with their models. Each archive has a pinned URL,
byte count and SHA-256. Installers stage files under the models root, verify the
extracted libraries, check cancellation and activate with an atomic rename.
The existing download consent and model-license checks remain in front of Setup
installs. Source notices are retained beside the runtimes.

- Moonshine v0.1.5: `moonshine/runtime/linux-x64/lib/libmoonshine.so` and
  `libonnxruntime.so.1`. The upstream shared library has `$ORIGIN` RUNPATH and
  passes the existing ABI-major check (`30000`). No local relink is needed.
  Medium, Small and the live-preview helper use their existing CDN weights.
- sherpa-onnx v1.13.8 CPU, ASR-only archive: `optional/sherpa-runtime/` contains
  `libsherpa-onnx-c-api.so`, `libonnxruntime.so` and `silero_vad.onnx`.
  Whisper, optional sherpa models and Qwen's speech detector share this runtime.
  Punctuation and speaker labels load its pinned ONNX library.
- llama.cpp b10964 CPU: `llama.cpp/runtime/linux-x64/llama-server`.
  Vulkan: `llama.cpp/runtime/linux-x64-vulkan/llama-server`. Both packages are
  fetched on the first GGUF model install (46,991,558 archive bytes total),
  including on CPU-only machines. Each has a `sonojot-runtime-release` marker
  containing `b10964`; extracted file identities live in
  `src-tauri/assets/linux-llama-identities.json`.

`paths.json` may override `moonshine_runtime_dir`, `moonshine_dll`,
`llama_server_exe` and `llama_server_vulkan_exe`. Linux runtime overrides must
retain the pinned files; llama.cpp also needs its release marker. Installations
always write the managed paths above, never an override destination.
Locate models accepts a verified Linux Moonshine runtime with bundled or managed
Medium/Small weights. It does not import arbitrary Linux GGUF packages.

Text cleanup tries Vulkan with `-ngl 99` only when the verified Vulkan runtime's
bounded `--list-devices` probe reports a hardware device. Software Vulkan devices
are excluded. Startup failure falls back to CPU; `SONOJOT_CLEANUP_CPU=1` or the
CPU preference bypasses Vulkan. Qwen3-ASR CPU variants stay on CPU: the current
speech-worker protocol reports selected IDs, not a runtime fallback result, so
Vulkan speech would require a separate IPC change to report execution honestly.
Linux sherpa CUDA remains disabled; the existing download hold requires an
ASR-only GPU package without the TTS libraries.

On Arch, the downloaded llama.cpp builds resolve system libraries from `glibc`,
`gcc-libs` (including `libgomp`), `openssl`, `zlib`, `brotli` and `zstd`. Vulkan
also needs `vulkan-icd-loader` and the graphics vendor's Vulkan driver. The VM
checked on 2026-10-15 had the loader and no GPU; `--list-devices` returned
`Available devices: (none)`. NVIDIA/AMD GPU inference remains untested.

## Linux CPU verification, 2026-10-15

The isolated Omarchy VM checkout was `~/src/sonojot-runtimes`; model files were
installed through the app installers into `/tmp/sj-rt/models`. The sample was
whisper.cpp's public-domain JFK `samples/jfk.wav`, 11.0 seconds, 16 kHz mono.
These are cold, single-sample measurements including worker/model startup,
not latency recommendations.

```bash
cd ~/src/sonojot-runtimes/rewrite/src-tauri
cargo check -j 6
cargo build -j 6 --bin sonojot
SONOJOT_MODELS=/tmp/sj-rt/models SONOJOT_NATIVE_WAV=/tmp/sj-rt/sample.wav \
  cargo test -j 6 --lib linux_runtime_native_smoke -- --ignored --nocapture
SONOJOT_MODELS=/tmp/sj-rt/models \
  cargo test -j 6 --lib linux_punctuation_native_smoke -- --ignored --nocapture
```

Observed output:

```text
INSTALL moonshine-medium: 35.769s
TRANSCRIBE moonshine-medium: 7.098s: And so, my fellow Americans, Ask not. What your country can do for you. Ask what you can do for your country.
INSTALL qwen3-asr-0.6b-q8-cpu: 88.837s
TRANSCRIBE qwen3-asr-0.6b-q8-cpu: 18.232s: And so, my fellow Americans, ask not what your country can do for you; ask what you can do for your country.
INSTALL whisper-tiny: 15.959s
TRANSCRIBE whisper-tiny: 2.372s: and so my fellow Americans! ask not what your country can do for you ask what you can do for your country
INSTALL cleanup: 49.237s
CLEANUP: 12.813s: The small garden is quiet today.
INSTALL punctuation: 22.018s
PUNCTUATION: 2.639s [punctuation_model]: Hello. My name is sam. This is a test of local punctuation.
```

The cleanup test confirmed the reported backend was `cpu`; Locate models
accepted the installed Moonshine folder. Hardware detection reported 8,315,555,840
bytes of RAM, 4 physical cores, 8 logical cores and no GPU. Diarization compiles
and uses the same verified ONNX runtime; native speaker-label inference was not
run. Live-preview installation uses the same runtime, but live audio was not
tested in this VM.

For unit tests, use an on-disk temporary directory when the model downloads fill
the VM's 3.9 GiB `/tmp` tmpfs. The audio spool correctly refuses to write below
its temporary-disk safety floor:

```bash
mkdir -p /home/digory/src/sonojot-runtimes/test-tmp
TMPDIR=/home/digory/src/sonojot-runtimes/test-tmp cargo test -j 6 --lib
```

The full library suites passed: Windows 1,001 passed / 15 ignored; Linux 960
passed / 12 ignored. `cargo check -j 6` passed on both platforms. The acceleration
TypeScript tests passed (6 tests), and `npm run check` reported no Svelte errors
or warnings. `python rewrite/scripts/generate_notices.py --offline --check`
passed; the inventory still carries 121 review issues and is not a strict
release-license audit.

Runtime archives are retained at `/tmp/sj-rt/cache`; test models, extracted
inspection copies and native helpers are removed/stopped after verification.
