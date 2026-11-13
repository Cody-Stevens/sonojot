# SonoJot in the browser

`@sonojot/transcribe` turns a voice recording into text inside the user's browser: speech recognition, then SonoJot punctuation, then the same conservative cleanup as SonoJot's desktop dictation. Audio never leaves the page and nothing runs on a server.

```ts
import { createTranscriber } from "@sonojot/transcribe";

const transcriber = createTranscriber();
transcriber.onProgress((e) => console.log(e.stage, e.overallLoaded, e.overallTotal));
await transcriber.preload(); // optional: download while the user is still recording

const { raw, text, language } = await transcriber.transcribe(recordingBlob, { language: "en" });
```

`raw` is what the speech model heard. `text` is what SonoJot would publish: punctuated, with fillers, repeats and restarts removed, and numbers, money, dates and times written as figures (`cleanup: "format"` keeps every word and only adds punctuation; `"original"` returns the raw text).

## What runs

| Stage | Model | Download | License |
| --- | --- | ---: | --- |
| Speech recognition | SonoJot Parakeet v1 (SonoJot's fine-tune of NVIDIA Parakeet TDT 0.6B v2), encoder weights in 4-bit | 375 MB | CC BY 4.0 |
| Punctuation and written form | SonoJot Punctuation v3 | 303 MB | CC BY-SA 4.0 |
| Cleanup rules | ported from the desktop app | in the package | Apache-2.0 |

Recognition uses WebGPU when the browser has it and WebAssembly otherwise. Punctuation runs on WebAssembly. Everything runs in a Web Worker, so the page stays responsive. Only English is supported; any other `language` rejects with `UnsupportedLanguageError`, so an app can say so instead of showing a wrong transcript.

Downloads are checked against pinned SHA-256 values and kept in Cache Storage under their hash. Later starts read them back and check only their size: they were verified when stored, and hashing 678 MB on every start would cost 0.4 s on a fast desktop and more on a laptop. A file that has not been used for `maxAgeDays` (default 30) is deleted the next time the transcriber starts; the browser may also evict it earlier when storage runs low, and it is then downloaded again. `clearModelCache()` removes everything.

### Resumed downloads

While a file downloads, the worker writes its bytes to the Origin Private File System (OPFS), one file per SHA-256. If the download stops (an abort, a network error, a reload or a closed tab), the next `preload()` or `transcribe()` continues it with an HTTP `Range` request. A file is keyed by its hash, not its URL, so a download can continue from another host.

- The finished file is checked against the pinned size and SHA-256 before it is cached or used. If a resumed file does not match, its stored bytes are dropped and it is downloaded once more from the start before the load fails with `IntegrityError`.
- A server that ignores `Range` (status 200) sends the whole file, which replaces the stored part. A `416`, or stored bytes longer than the pinned size, also start the file again.
- Partial files count toward `maxAgeDays` from their last write and are removed by `clearModelCache()`. `cacheInfo()` lists them in `partialFiles` (`name`, `bytes` received, `total`, `lastUsed`) and `partialBytes`, next to the existing `files` and `bytes`.
- Progress events for a resumed file start at the stored offset, and `resumedFrom` says how many bytes were kept, so a host can show "Resuming" when it is above 0. `LoadReport.downloadedBytes` counts the bytes fetched by this load; `resumedFiles` and `resumedBytes` count what was reused.
- Two tabs never write the same partial: the OPFS access handle is exclusive, so a tab that finds a file in use downloads it into memory instead, as before.
- Where OPFS is unavailable or fails (Firefox private windows, a full disk, Playwright's WebKit build for Windows), downloads go to memory and an interrupted file starts again from the beginning. A failed write moves the download to memory without failing it.

During a download the worker holds only the chunk in flight. The whole file is read back from OPFS once, after the download, for hashing and loading.

`Range: bytes=N-` is a CORS-safelisted header, so resuming from Hugging Face or another CORS host needs no preflight and no CSP or header change.

## API

- `createTranscriber(options)`: `modelUrl`, `device` (`"auto"`, `"webgpu"`, `"wasm"`), `punctuation` (default true), `maxAgeDays`, `cleanup`, `wasmPaths`, `worker`.
- `preload({ signal })` downloads (or reads from the cache) and loads every model. Callers share one load. It resolves with a `LoadReport`: the device, how many files came from the cache or the network, and per-step timings (`filesMs`, `recognitionSessionsMs`, `punctuationSessionMs`, `tokenizerMs`, `totalMs`). The steps overlap, so they do not add up to the total.
- `transcribe(audio, { language, cleanup, sampleRate, signal })` takes a recording (any format the browser can decode) or 16 kHz mono `Float32Array` samples.
- `onProgress(listener)`, `cacheInfo()`, `dispose()`, `clearModelCache()`, `decodeAudio()`. `cacheInfo()` resolves with `{ files, bytes, partialFiles, partialBytes }` (type `CacheInfo`).
- `supportsLanguage(tag)`, `SUPPORTED_LANGUAGES` and `UnsupportedLanguageError` are also exported from `@sonojot/transcribe/language`, a module with no dependencies. Importing it does not bundle the worker, onnxruntime or the text data (0.5 kB), so an app can decide whether to offer transcription before loading anything.

### Cancellation

`preload({ signal })` and `transcribe(audio, { signal })` reject with the signal's reason, like fetch: a `DOMException` named `AbortError` for a plain `abort()`.

- Aborting stops only that caller. Other callers waiting for the same model load keep waiting.
- When the last waiting caller aborts, the downloads stop. Files already downloaded and verified stay cached, and a file that was partway through its download keeps the bytes it has (see Resumed downloads).
- A later `preload()` or `transcribe()` starts a new load that reads the finished files from the cache and downloads only the rest.
- Once every file is in hand, an abort no longer stops the load: model sessions finish loading, so the next call finds them ready.
- During recognition, `transcribe`'s signal stops the job between steps.
- `dispose()` still stops everything at once.

## Host requirements

- By default the models are downloaded from Hugging Face at pinned commits: the `web/` folder of `SonoJot/sonojot-parakeet-v1` and the root of `SonoJot/sonojot-punct-v3`. Downloads redirect to CDN hosts, so `connect-src` needs `https://huggingface.co https://*.hf.co`.
- To serve them yourself, lay the files out as `<model>/<file>` (`sonojot-parakeet-v1-web-q4/…`, `sonojot-punct-v3/…`) on a host listed in `connect-src`, with CORS allowing your origin, and pass `modelUrl: "https://models.example.com/{model}/{name}"`. Files are cached by hash, so changing hosts later does not cause a new download.
- `script-src` needs `'wasm-unsafe-eval'` to compile WebAssembly. Workers and `.wasm` files load from your own origin, so `worker-src 'self'` (or the `script-src` fallback) is enough. No third-party scripts are loaded.
- Recording needs `Permissions-Policy: microphone=(self)`.
- Optional: `Cross-Origin-Opener-Policy: same-origin` with `Cross-Origin-Embedder-Policy: require-corp` lets WebAssembly use several threads, which speeds up the WebAssembly path.

The package ships TypeScript source. `src` type-checks with `lib: ["ES2022", "DOM", "DOM.Iterable"]` (checked with TypeScript 5.9 and 7), so a host does not need the `WebWorker` lib; `npm run check` covers that through `tsconfig.host.json`.

With Vite, add `worker: { format: "es" }` and `optimizeDeps: { exclude: ["onnxruntime-web"] }` to the config so the worker and onnxruntime-web's files are bundled from your origin.

### `wasmPaths`

onnxruntime-web needs two files: `ort-wasm-simd-threaded.asyncify.mjs` (53 kB, loaded with `import()`) and `ort-wasm-simd-threaded.asyncify.wasm` (28.7 MB, loaded with `fetch()`). Your bundler emits both next to the worker. `wasmPaths` overrides where they come from:

- an object names each file: `{ wasm: "/ort/ort-wasm-simd-threaded.asyncify.wasm" }` moves only the `.wasm` and keeps the bundled `.mjs`;
- a string is a folder that holds both files: `"/ort/"`.

Strings and `URL`s are accepted. Relative values resolve against the page's URL (`document.baseURI`), not the worker's, and reach onnxruntime as absolute URLs.

## Cloudflare and the 25 MiB file limit

Cloudflare Pages and Workers static assets refuse files over 25 MiB (26,214,400 bytes). onnxruntime-web 1.31's WebGPU build, `ort-wasm-simd-threaded.asyncify.wasm`, is 28,747,738 bytes (7.3 MB gzipped). Bundlers copy it next to the worker even when `wasmPaths` points elsewhere, because `wasmPaths` is read at run time and the bundle refers to the file at build time.

| Option | Setup | CSP |
| --- | --- | --- |
| Same origin, served by something other than static assets | A Worker route on your origin serves the `.wasm` from R2 or KV. Pass `wasmPaths: { wasm: "/ort/ort-wasm-simd-threaded.asyncify.wasm" }`. | No change: `connect-src 'self'` covers the fetch. |
| Split into parts, joined by a Worker (what Saidframe does) | Store the `.wasm` in parts under 25 MiB; a Worker streams them back as one response with `Content-Type: application/wasm`, then pass that path as `wasm`. | No change. `application/wasm` lets the browser compile while downloading; with another type onnxruntime logs a warning, fetches the file again and compiles it after. |
| Another origin: R2 public bucket, CDN | Serve the `.wasm` with `Access-Control-Allow-Origin` for your origin and pass its full URL as `wasm`. | Add that origin to `connect-src`. With COEP `require-corp`, it also needs `Cross-Origin-Resource-Policy: cross-origin` (or CORS, as above). Moving the `.mjs` too (`mjs`) would also need the origin in `script-src`. |
| Stop the bundler emitting the file | Add onnxruntime-web's own export condition to Vite: `resolve: { conditions: ["onnxruntime-web-use-extern-wasm", ...defaultClientConditions] }` (`defaultClientConditions` from `vite`). It picks a build that refers to neither file, so nothing is emitted; serve both files yourself (copy them from `node_modules/onnxruntime-web/dist/`) and pass `wasmPaths` with both: `"/ort/"` or `{ mjs, wasm }`. Combine with one of the rows above for the `.wasm`. | The `.mjs` loads with `import()`, so its origin must be in `script-src` (`'self'` when it is a static asset of your site); the `.wasm` follows the row you combine it with. |

Without `wasmPaths` the export condition breaks the default setup, since onnxruntime then looks for both files next to the worker. Smaller onnxruntime builds exist, but they are not drop-in: the WebAssembly-only build (`ort-wasm-simd-threaded.wasm`, 14.8 MB) has no WebGPU, and the JSPI build (17.9 MB) needs JavaScript Promise Integration, which not every browser has.

## Speed

Measured in Chrome 157 on Windows, on a Ryzen 9 9900X with an RTX 5070, with the models served from a local folder:

- First run, models from a local server: 5 to 8 s to download, verify, cache and load the 678 MB, depending on machine load.
- Later starts, from the cache: 3.0 to 3.3 s when the machine was lightly loaded, 4.6 to 4.8 s while other jobs kept its CPU about 90% busy. With the browser held to half a CPU core, as a stand-in for a slower laptop, starts took 13 to 17 s (21 to 24 s with the CPU also busy). Expect a laptop to fall between these. Most of the time is onnxruntime building the speech model's session (about 60%) and the punctuation model and tokenizer (about 40%), all on one thread.
- Recognition on WebGPU runs at about 0.01 of real time (a 60 s note in under a second); single-threaded WebAssembly at about 0.4.

## Browsers

Chrome 157 (WebGPU and WebAssembly), Playwright's Firefox 155 (WebGPU in a visible window, WebAssembly) and Playwright's WebKit 26.6 on Windows (WebAssembly) load the models, cancel downloads, and produce the same transcripts as Chrome. Chrome and Firefox continue interrupted downloads with `Range` requests, also after the browser was closed and reopened; Playwright's WebKit lacks OPFS access handles and restarts them from the beginning. Playwright's WebKit build for Windows is not Safari: it has no WebGPU and no Web Audio, so it was tested with WAV files, which the package reads itself when Web Audio is missing. Safari itself is untested.

## Develop

```sh
npm install
npm run check
npm test                      # unit tests; model tests run when SONOJOT_MODEL_DIR is set
SONOJOT_MODEL_DIR=/path/to/models npm run demo
```

`SONOJOT_MODEL_DIR` is a folder laid out like the model host (`sonojot-parakeet-v1-web-q4/…`, `sonojot-punct-v3/…`). The demo records from the microphone or opens a file and shows both texts and timings. The Parakeet reference test also needs `SONOJOT_TEST_WAV` (a 16 kHz 16-bit WAV file) and, to compare the text, `SONOJOT_TEST_TEXT`.

`scripts/measure.ts` drives the built demo (`npm run build:demo`) in Chrome, Firefox or WebKit through Playwright: cold and cached starts, transcription speed, and a cancellation check. Its options are listed at the top of the file.
