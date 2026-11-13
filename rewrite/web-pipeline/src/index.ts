// Public API. Everything heavy runs in a Web Worker; this module decodes
// audio (Web Audio only exists on the main thread) and forwards jobs.
import { decodeAudio, resample } from "./audio.ts";
import { supportsLanguage, UnsupportedLanguageError } from "./language.ts";
import type { CacheInfo, LoadReport, ProgressEvent, Request, Response, TranscribeResult, WasmPaths, WorkerConfig } from "./protocol.ts";
import type { CleanupMode } from "./text/contract.ts";
import { HUGGING_FACE_MODEL_URLS } from "./models.ts";

export type { CacheInfo, CleanupMode, LoadReport, ProgressEvent, TranscribeResult, WasmPaths };
export { PARAKEET, PUNCTUATION, ALL_FILES, HUGGING_FACE_MODEL_URLS, type ModelFile } from "./models.ts";
export { supportsLanguage, SUPPORTED_LANGUAGES, UnsupportedLanguageError } from "./language.ts";
export { decodeAudio };

export interface TranscriberOptions {
  /**
   * Where the model files are downloaded from. Default: the pinned Hugging
   * Face commits (`HUGGING_FACE_MODEL_URLS`). Otherwise a URL template with
   * {model}, {name} and {sha256}, or a map from model folder
   * ("sonojot-parakeet-v1-web-q4", "sonojot-punct-v3") to a base URL.
   * Files are verified against pinned SHA-256 values whatever the host.
   */
  modelUrl?: WorkerConfig["modelUrl"];
  /** "auto" (default) uses WebGPU for recognition when the browser has it. */
  device?: WorkerConfig["device"];
  /**
   * SonoJot Punctuation (default true, 303 MB more to download). When false, the
   * text keeps the speech model's own punctuation and gets the cleanup rules only.
   */
  punctuation?: boolean;
  /** Delete cached models not used for this many days. Default 30; 0 leaves it to the browser. */
  maxAgeDays?: number;
  /** Default cleanup for transcribe(); "clean" matches SonoJot's default dictation. */
  cleanup?: CleanupMode;
  /**
   * Where onnxruntime-web loads its WebAssembly files from, when the bundled
   * copies should not be used. A string is a folder (prefix) that holds both
   * `ort-wasm-simd-threaded.asyncify.wasm` and `.mjs`; an object names each
   * file, and a field left out keeps the bundled file, e.g.
   * `{ wasm: "/ort/ort-wasm-simd-threaded.asyncify.wasm" }`. Relative values
   * resolve against the page's URL (`document.baseURI`), not the worker's.
   */
  wasmPaths?: string | { wasm?: string | URL; mjs?: string | URL };
  /** Supply your own worker (for bundlers that need a custom entry); defaults to the bundled one. */
  worker?: () => Worker;
}

export interface TranscribeOptions {
  /** BCP 47 tag. Only English is available; other languages reject with UnsupportedLanguageError. */
  language?: string;
  cleanup?: CleanupMode;
  /** Sample rate of a Float32Array input. Default 16000. */
  sampleRate?: number;
  /**
   * Cancels this call, including the model download or load it is waiting for.
   * It rejects with the signal's reason (a DOMException named "AbortError" for a
   * plain `abort()`), like fetch.
   */
  signal?: AbortSignal;
}

export interface PreloadOptions {
  /**
   * Stops waiting. Other callers waiting for the same load are not affected;
   * when none are left, downloads stop. Files already downloaded and verified
   * stay cached, and the next preload() or transcribe() continues from them.
   * A file that was partly downloaded keeps its bytes where the browser allows
   * it (OPFS) and resumes from there, even after a reload.
   */
  signal?: AbortSignal;
}

export interface Transcriber {
  /**
   * Download (or read from cache) and load every model. Safe to call more than
   * once: callers share one load. Resolves with what the load did and its timings.
   */
  preload(options?: PreloadOptions): Promise<LoadReport>;
  /** Recording (any format the browser can play) or 16 kHz mono samples -> transcript. */
  transcribe(audio: Blob | ArrayBuffer | Float32Array, options?: TranscribeOptions): Promise<TranscribeResult>;
  /** Download and loading progress. Returns an unsubscribe function. */
  onProgress(listener: (event: ProgressEvent) => void): () => void;
  /** Cached model files and their total size, plus interrupted downloads that will resume. */
  cacheInfo(): Promise<CacheInfo>;
  /** Stop the worker and free its memory. Cached files stay. */
  dispose(): void;
}

/** The model load that preload() and transcribe() callers share. */
interface SharedLoad {
  id: number;
  promise: Promise<LoadReport>;
  waiters: number;
  settled: boolean;
}

/** onnxruntime-web's `wasmPaths` with relative URLs made absolute, so they mean the same inside the worker. */
function resolveWasmPaths(paths: TranscriberOptions["wasmPaths"]): WasmPaths | undefined {
  if (paths === undefined) return undefined;
  const base = typeof document !== "undefined" ? document.baseURI : globalThis.location.href;
  const absolute = (value: string | URL) => new URL(value, base).href;
  if (typeof paths === "string") return absolute(paths);
  const resolved: { wasm?: string; mjs?: string } = {};
  if (paths.wasm !== undefined) resolved.wasm = absolute(paths.wasm);
  if (paths.mjs !== undefined) resolved.mjs = absolute(paths.mjs);
  return resolved;
}

function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("This operation was aborted.", "AbortError");
}

export function createTranscriber(options: TranscriberOptions = {}): Transcriber {
  const worker = options.worker?.() ?? new Worker(new URL("./worker.ts", import.meta.url), { type: "module", name: "sonojot-transcribe" });
  const listeners = new Set<(event: ProgressEvent) => void>();
  const pending = new Map<number, { resolve(value: unknown): void; reject(error: unknown): void }>();
  let nextId = 1;
  let shared: SharedLoad | undefined;
  let disposed = false;

  worker.onmessage = (event: MessageEvent<Response>) => {
    const message = event.data;
    if (message.type === "progress") {
      for (const listener of listeners) listener(message.event);
      return;
    }
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    if (message.type === "done") waiter.resolve(message.value);
    else waiter.reject(Object.assign(new Error(message.message), { name: message.name }));
  };
  worker.onerror = (event) => {
    const error = new Error(event.message || "The transcription worker failed to start.");
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };

  const send = (request: Request) => worker.postMessage(request, request.type === "transcribe" ? [request.job.samples.buffer] : []);
  const call = <T>(build: (id: number) => Request): Promise<T> => {
    if (disposed) return Promise.reject(new Error("This transcriber was disposed."));
    const id = nextId++;
    return new Promise<T>((resolve, reject) => {
      pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
      send(build(id));
    });
  };

  /** Wait for the shared model load; aborting stops only this caller unless it was the last one waiting. */
  const waitForLoad = (signal?: AbortSignal): Promise<LoadReport> => {
    if (disposed) return Promise.reject(new Error("This transcriber was disposed."));
    if (signal?.aborted) return Promise.reject(abortReason(signal));
    if (!shared) {
      const id = nextId++;
      const load: SharedLoad = { id, waiters: 0, settled: false, promise: undefined as never };
      load.promise = new Promise<LoadReport>((resolve, reject) => {
        pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
        send({ type: "preload", id });
      });
      load.promise.then(
        () => (load.settled = true),
        () => {
          load.settled = true;
          if (shared === load) shared = undefined; // a later call starts a new load
        },
      );
      shared = load;
    }
    const load = shared;
    load.waiters++;
    return new Promise<LoadReport>((resolve, reject) => {
      const onAbort = () => {
        load.waiters--;
        reject(abortReason(signal!));
        if (load.waiters === 0 && !load.settled) {
          // Nobody else is waiting: stop the downloads. Finished files stay cached.
          // A call made right after this, even in the same task, starts a new load.
          load.settled = true;
          if (shared === load) shared = undefined;
          send({ type: "cancel-load" });
          const waiter = pending.get(load.id);
          pending.delete(load.id);
          waiter?.reject(new DOMException("Loading the models was cancelled.", "AbortError"));
        }
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      load.promise.then(
        (report) => {
          signal?.removeEventListener("abort", onAbort);
          resolve(report);
        },
        (error: unknown) => {
          signal?.removeEventListener("abort", onAbort);
          if (!signal?.aborted) reject(error);
        },
      );
    });
  };

  send({
    type: "init",
    config: {
      modelUrl: options.modelUrl ?? HUGGING_FACE_MODEL_URLS,
      device: options.device ?? "auto",
      punctuation: options.punctuation ?? true,
      maxAgeDays: options.maxAgeDays ?? 30,
      wasmPaths: resolveWasmPaths(options.wasmPaths),
    },
  });

  return {
    preload(preloadOptions = {}) {
      return waitForLoad(preloadOptions.signal);
    },

    async transcribe(audio, transcribeOptions = {}) {
      const { language, signal } = transcribeOptions;
      if (!supportsLanguage(language)) throw new UnsupportedLanguageError(language!);
      signal?.throwIfAborted();
      const samples =
        audio instanceof Float32Array
          ? resample(audio, transcribeOptions.sampleRate ?? 16_000).slice()
          : await decodeAudio(audio);
      await waitForLoad(signal);
      const id = nextId;
      const onAbort = () => send({ type: "cancel", id });
      signal?.addEventListener("abort", onAbort, { once: true });
      try {
        return await call<TranscribeResult>((assigned) => ({
          type: "transcribe",
          id: assigned,
          job: { samples, cleanup: transcribeOptions.cleanup ?? options.cleanup ?? "clean" },
        }));
      } catch (error) {
        // The worker reports its own AbortError; surface the signal's reason, as fetch does.
        if (signal?.aborted) throw abortReason(signal);
        throw error;
      } finally {
        signal?.removeEventListener("abort", onAbort);
      }
    },

    onProgress(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    cacheInfo() {
      return call((id) => ({ type: "cache-info", id }));
    },

    dispose() {
      disposed = true;
      worker.terminate();
      const error = new Error("This transcriber was disposed.");
      for (const waiter of pending.values()) waiter.reject(error);
      pending.clear();
    },
  };
}

export { clearModelCache } from "./store.ts";
