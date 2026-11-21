# sonojot-mobile

SonoJot's offline speech recognition for Android and iOS, with no Tauri or desktop dependencies.
It builds as `libsonojot_mobile.so` (Android), a static library (iOS) and an rlib, and exposes a
small C ABI ([`include/sonojot_mobile.h`](include/sonojot_mobile.h)). Android apps use it to
transcribe on phones whose system recognizer can't.

For one recording of 16 kHz mono `f32` PCM, the engine:

1. **Gates.** `AudioGateFilter` runs SonoJot's RNNoise filter over a copy of the audio, shifts it
   back by the filter's fixed 352-sample delay, and marks the speech regions. Regions longer than
   the model window (28 s for Whisper) are cut at their quietest frame. The recognizer always
   decodes the original audio, never the denoised copy. If the gate hears no speech in audio that
   is clearly audible, it passes that span through anyway, so a gate miss never becomes "no speech".
2. **Decodes** each region with `SherpaRecognizer`, which loads the sherpa-onnx C API
   (`libsherpa-onnx-c-api.so`) at run time. Supported model families: Whisper, SenseVoice,
   transducers (Zipformer, NeMo Parakeet) and Moonshine v1/v2.
3. **Cleans up**: removes model control tokens and non-speech captions (`[BLANK_AUDIO]`, `(music)`),
   collapses decoder loops (a phrase repeated 8+ times), recases all-caps output (keeping Turkish
   dotted and dotless i apart), tidies spacing around Latin and Arabic punctuation and, for English,
   applies SonoJot's spoken-to-written rules (numbers, dates, money, product names).

## Rules

These keep the crate, the desktop app and the phone apps coherent. Change them only on purpose,
and update every place a rule names in the same change.

1. **One copy of shared code.** `asr_filter.rs` and `written_form.rs` (with `written_form/`) are
   compiled from `rewrite/src-tauri/src` by `#[path]`. Keep them free of `crate::` paths and
   desktop-only dependencies, and run this crate's tests after changing them.
2. **One set of model pins, and the same models as SonoJot.** English uses SonoJot Parakeet, the
   SonoJot apps' English model; every other language uses Whisper Small, because Parakeet is
   English only. `src/catalog.rs` reuses the desktop's pins (`optional_asr.rs` for SonoJot
   Parakeet's current release, `whisper_catalog.rs` for Whisper); the
   `pins_match_the_desktop_catalog` test compares every file and enforces it. When the desktop
   ships a new Parakeet release, re-pin both together. A pack whose files change gets a new
   catalog `id`.
3. **One sherpa-onnx release.** The `#[repr(C)]` structs in `src/sherpa.rs` mirror sherpa-onnx
   **1.13.8**, the desktop's release; `SherpaRuntime::open` refuses any other version. Moving to a
   new release means re-checking every struct against its `c-api.h` and changing, together:
   `SHERPA_ONNX_ABI`, the desktop runtime URLs, and the runtime each app bundles. Apps read the
   required version from `sonojot_mobile_abi_info()` instead of copying it.
4. **Packs are data; the runtime ships in the app.** Android and iOS don't let apps load native
   code they download, so `libsherpa-onnx-c-api.so` and `libonnxruntime.so` are bundled and packs
   carry only model files. On iOS, where the runtime is linked statically, pass
   `"runtime_library": "self"` so its symbols are resolved from the app itself.
5. **Nothing unverified reaches native code.** Downloads come only from the pinned catalog URLs,
   over HTTPS, from `catalog::DOWNLOAD_HOSTS`, with at most `catalog::MAX_REDIRECTS` redirects.
   Apps hash while downloading into a staging directory in their private internal storage, call
   `sonojot_pack_finalize` on it (it writes `manifest.json` only if every file's size and SHA-256
   match, and renames it into place so a planted link is replaced, not followed), and only then
   move the pack into place. `SonojotEngine::open` always hashes every file again and refuses
   `"verify": "sizes"`; only status checks (`sonojot_pack_inspect`) compare sizes alone. Symbolic links anywhere in a file's path, and paths outside a pack, are refused.
6. **Never translate, and never hand Whisper an unknown language.** Whisper's task is always
   `transcribe`. A multilingual Whisper model is told the requested language only when it has a
   token for it in `WHISPER_LANGUAGES` (the 99 of the tiny–large-v2 checkpoints); otherwise it
   detects the language itself. sherpa-onnx 1.13.8 ends the whole app process on an unknown
   token, so locales are mapped first: Android's legacy `iw`, `in` and `ji` become `he`, `id` and
   `yi`, and Whisper's own spellings apply (`nb` → `no`, `fil` → `tl`, `jv` → `jw`). A pack's
   fixed `options.language` must be one of the tokens.
7. **Never drop audible audio.** The gate may skip silence, never audible sound; the recognizer
   always hears original samples.
8. **Cleanup only removes what wasn't said or changes case and spacing.** The written-form rules
   apply to English only.
9. **The C ABI is versioned.** `sonojot_mobile_abi_version()` changes whenever a function is
   added, removed, or changes signature or meaning, and apps refuse other versions.
   `sonojot_mobile_abi_info()` reports it with the manifest schema and sherpa-onnx release. Error codes in
   `EngineError::code` never change meaning; add new ones instead.
10. **16 KB pages.** Every Android `.so` (this crate's and the runtime's) must have 16 KB-aligned
    LOAD segments; build scripts check it.
11. **The source of truth is the private SonoJot repository.** The public `sonojot` repository is
    exported from it, and apps that vendor this crate copy it with their sync script. Never edit a vendored copy.

## Model packs

A pack is a directory with a `manifest.json` that pins each file's size and SHA-256 (see
`src/manifest.rs` for the format). The catalog phones download from (`src/catalog.rs`):

| Language | Pack | Download |
|---|---|---|
| `en` | SonoJot Parakeet v1 (NeMo transducer), int8 | 662 MB |
| `ar`, `tr`, any other | Whisper Small multilingual, int8 | 375 MB |

SonoJot Parakeet is the same sherpa-onnx int8 export, at the same revision, as the desktop's.
Whisper Base is too weak for Arabic, so Small is the smallest checkpoint for the multilingual
pack. For packs outside the catalog, generate a manifest with:

```sh
cargo run --release --example make_pack -- <pack-dir> \
    --id whisper-base-multilingual-int8 --family whisper --languages '*' \
    encoder=base-encoder.int8.onnx decoder=base-decoder.int8.onnx tokens=base-tokens.txt
```

## Building

```sh
cargo test                                   # unit tests, no runtime or models needed
cargo check --target aarch64-linux-android   # pure Rust: no NDK needed to check
```

To link `libsonojot_mobile.so` for Android, use the NDK's clang as the linker and link with
16 KB pages (`-C link-arg=-Wl,-z,max-page-size=16384`), for `arm64-v8a` and `x86_64`, and bundle
the pinned sherpa-onnx 1.13.8 runtime beside it.

An end-to-end check against a real runtime and model is in `tests/real_runtime.rs` (ignored by
default; it lists the environment variables it needs).

## C ABI

`sonojot_engine_open(config_json)` → handle; `sonojot_engine_transcribe_f32le(handle, bytes)` →
result JSON; `sonojot_engine_close(handle)`; `sonojot_catalog()`;
`sonojot_pack_finalize(staging_dir, pack_id)`; `sonojot_pack_inspect(dir, full)`;
`sonojot_string_free(text)`. Errors come back as `{"code", "message"}` JSON with stable codes.
Panics never cross the boundary.

`sonojot_mobile_abi_info()` returns a static JSON string (do not free it) with what the loaded
library expects: `{"abi_version": 2, "manifest_schema": 1, "required_sherpa_version": "1.13.8"}`.
Apps should read these values from the library instead of copying them, so a library update
cannot leave the app checking packs or the bundled runtime against stale numbers.
