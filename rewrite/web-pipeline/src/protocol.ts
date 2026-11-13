// Messages between the page and the transcription worker. Everything here
// must survive structured cloning.
import type { CleanupMode } from "./text/contract.ts";

export type Device = "webgpu" | "wasm";

export interface WorkerConfig {
  /**
   * Where model files come from: a template with {model}, {name} and {sha256}
   * (e.g. "https://models.example.com/{model}/{name}"), or a map from model
   * folder to a base URL that the file name is appended to.
   */
  modelUrl: string | Record<string, string>;
  device: "auto" | Device;
  /** Download and run SonoJot Punctuation (303 MB). Off: rules only, on the speech model's own punctuation. */
  punctuation: boolean;
  maxAgeDays: number;
  /** Where onnxruntime-web finds its .wasm/.mjs files; absolute URLs (the page resolves relative ones). */
  wasmPaths?: WasmPaths;
}

/**
 * onnxruntime-web's `env.wasm.wasmPaths`: a prefix that both file names are
 * appended to, or the URL of each file. Leave a field out to keep the bundled file.
 */
export type WasmPaths = string | { wasm?: string; mjs?: string };

/** What a model load did and where the time went. */
export interface LoadReport {
  device: Device;
  /** Files read from the browser's cache, and files downloaded and verified. */
  cachedFiles: number;
  downloadedFiles: number;
  /** Bytes received from the network by this load. A resumed file counts only the part fetched now. */
  downloadedBytes: number;
  /** Downloaded files that continued from an earlier, interrupted download, and the bytes they reused. */
  resumedFiles?: number;
  resumedBytes?: number;
  /**
   * Milliseconds. The steps overlap (files are read while sessions are created),
   * so they do not add up to `totalMs`.
   */
  timings: {
    /** From the load starting in the worker to ready. */
    totalMs: number;
    /** Until every file was read from the cache, or downloaded and verified. */
    filesMs: number;
    /** Speech recognition sessions: encoder, feature extractor and decoder. */
    recognitionSessionsMs: number;
    /** Punctuation session; absent with `punctuation: false`. */
    punctuationSessionMs?: number;
    /** Parsing tokenizer.json and building the tokenizer; absent with `punctuation: false`. */
    tokenizerMs?: number;
  };
}

/** What is stored for this origin; `cacheInfo()` returns it. */
export interface CacheInfo {
  /** Verified model files in Cache Storage. */
  files: { name: string; bytes: number; lastUsed: number }[];
  /** Total size of `files`. */
  bytes: number;
  /**
   * Downloads that were interrupted and will continue where they stopped: `bytes`
   * received so far of `total` (absent for a file this version does not know).
   * `lastUsed` is the last write. Always present since resumable downloads.
   */
  partialFiles?: { name: string; bytes: number; total?: number; lastUsed: number }[];
  /** Total size of `partialFiles`. */
  partialBytes?: number;
}

export interface TranscribeJob {
  samples: Float32Array;
  cleanup: CleanupMode;
}

export interface TranscribeResult {
  /** What the speech model heard, before punctuation and cleanup. */
  raw: string;
  /** Punctuated, cleaned text, as SonoJot's desktop app would publish it. */
  text: string;
  /** BCP 47 language of the transcript. */
  language: string;
  durationSeconds: number;
  device: Device;
  timings: { recognitionMs: number; cleanupMs: number; totalMs: number };
}

export type Stage = "download" | "load" | "ready";

export interface ProgressEvent {
  stage: Stage;
  /** Model file being fetched or loaded, when there is one. */
  file?: string;
  /** Bytes for this file. */
  loaded?: number;
  total?: number;
  /** Bytes across every model file, for one overall progress bar. */
  overallLoaded: number;
  overallTotal: number;
  /** True when this file came from the browser's cache. */
  cached?: boolean;
  /**
   * Bytes of this file kept from an earlier, interrupted download, which this one
   * continues from (`loaded` starts there). 0 for a fresh download; absent for other stages.
   */
  resumedFrom?: number;
}

export type Request =
  | { type: "init"; config: WorkerConfig }
  | { type: "preload"; id: number }
  | { type: "transcribe"; id: number; job: TranscribeJob }
  | { type: "cancel"; id: number }
  /** Nobody is waiting for the model load any more: stop its downloads. */
  | { type: "cancel-load" }
  | { type: "cache-info"; id: number }
  | { type: "clear-cache"; id: number };

export type Response =
  | { type: "progress"; event: ProgressEvent }
  | { type: "done"; id: number; value: unknown }
  | { type: "error"; id: number; name: string; message: string };
