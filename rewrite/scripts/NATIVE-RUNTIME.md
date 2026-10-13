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
