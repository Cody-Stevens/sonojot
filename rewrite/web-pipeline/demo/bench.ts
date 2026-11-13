// Driven by scripts/measure.ts through Playwright. Times a cold or warm model
// load, transcribes clips served from /bench-audio/, and checks cancellation.
/// <reference types="vite/client" />
import { clearModelCache, createTranscriber, type WasmPaths, type Transcriber, type ProgressEvent } from "../src/index.ts";

let transcriber: Transcriber | undefined;
const events: { at: number; event: ProgressEvent }[] = [];

function create(device: "auto" | "webgpu" | "wasm"): Transcriber {
  // ?models=hf downloads from the pinned Hugging Face commits instead of the local folder.
  const hosted = new URLSearchParams(location.search).get("models") === "hf";
  const wasmPaths = import.meta.env.VITE_WASM_PATHS as WasmPaths | undefined; // see vite.config.ts
  return createTranscriber({ modelUrl: hosted ? undefined : "/model-files/{model}/{name}", device, wasmPaths });
}

async function load(device: "auto" | "webgpu" | "wasm") {
  transcriber = create(device);
  const started = performance.now();
  transcriber.onProgress((event) => events.push({ at: performance.now() - started, event }));
  const report = await transcriber.preload();
  const done = events.filter((e) => e.event.stage === "download" && e.event.loaded === e.event.total);
  const downloadEnd = Math.max(0, ...done.map((e) => e.at));
  return { ms: performance.now() - started, cachedFiles: report.cachedFiles, downloadedBytes: report.downloadedBytes, downloadEndMs: downloadEnd, report };
}

async function run(paths: string[]) {
  const results = [];
  for (const path of paths) {
    const blob = await (await fetch(`/bench-audio/${path}`)).blob();
    const started = performance.now();
    const result = await transcriber!.transcribe(blob, { language: "en" });
    results.push({ path, wallMs: performance.now() - started, ...result });
  }
  return results;
}

/** Abort `work` once `bytes` more have arrived; report how it ended. */
async function abortAfter(t: Transcriber, bytes: number, work: (signal: AbortSignal) => Promise<unknown>) {
  const controller = new AbortController();
  let start: number | undefined;
  const files = new Set<string>();
  const stop = t.onProgress((event) => {
    if (event.stage === "download" && event.loaded === event.total && !event.cached && event.file) files.add(event.file);
    start ??= event.overallLoaded;
    if (event.overallLoaded - start >= bytes) controller.abort();
  });
  const started = performance.now();
  try {
    await work(controller.signal);
    return { outcome: "finished", ms: performance.now() - started };
  } catch (error) {
    const e = error as Error;
    return { outcome: e.name, isDOMException: e instanceof DOMException, message: e.message, ms: performance.now() - started, filesDownloadedBeforeAbort: [...files] };
  } finally {
    stop();
  }
}

/** Cancel a cold download twice (preload, then transcribe), then load and transcribe for real. */
async function abortCheck(device: "auto" | "webgpu" | "wasm", bytes: number, clip: string) {
  const t = create(device);
  const blob = await (await fetch(`/bench-audio/${clip}`)).blob();
  const preloadAbort = await abortAfter(t, bytes, (signal) => t.preload({ signal }));
  const cachedAfterFirst = (await t.cacheInfo()).files.length;
  const transcribeAbort = await abortAfter(t, bytes, (signal) => t.transcribe(blob, { language: "en", signal }));
  const cachedAfterSecond = (await t.cacheInfo()).files.length;
  const report = await t.preload();
  const result = await t.transcribe(blob, { language: "en" });
  t.dispose();
  return { preloadAbort, cachedAfterFirst, transcribeAbort, cachedAfterSecond, report, text: result.text, device: result.device };
}

/** First and last download event per file, to show where each one started and ended. */
function downloadSummary(t: Transcriber) {
  const files: Record<string, { firstLoaded: number; resumedFrom: number; lastLoaded: number; total: number; cached: boolean }> = {};
  const stop = t.onProgress((event) => {
    if (event.stage !== "download" || !event.file) return;
    const entry = (files[event.file] ??= { firstLoaded: event.loaded!, resumedFrom: event.resumedFrom ?? 0, lastLoaded: 0, total: event.total!, cached: !!event.cached });
    entry.lastLoaded = event.loaded!;
  });
  return { files, stop };
}

/**
 * Resume check, step 1: start a cold load and abort it once `bytes` have arrived.
 * The transcriber stays, so step 2 can continue on the same page.
 */
async function resumeStart(device: "auto" | "webgpu" | "wasm", bytes: number) {
  transcriber = create(device);
  const summary = downloadSummary(transcriber);
  const t = transcriber;
  const aborted = await abortAfter(t, bytes, (signal) => t.preload({ signal }));
  summary.stop();
  return { aborted, files: summary.files, cacheInfo: await t.cacheInfo() };
}

/**
 * Resume check, step 2: load again (on this page, or a new one after a reload) and report
 * where each file's download started. With `limit`, abort after that many more bytes instead.
 */
async function resumeFinish(device: "auto" | "webgpu" | "wasm", limit?: number) {
  transcriber ??= create(device);
  const t = transcriber;
  const cacheInfoBefore = await t.cacheInfo();
  const summary = downloadSummary(t);
  const started = performance.now();
  let outcome: unknown;
  let report: unknown;
  if (limit) outcome = await abortAfter(t, limit, (signal) => t.preload({ signal }));
  else report = await t.preload();
  summary.stop();
  return { ms: performance.now() - started, cacheInfoBefore, report, outcome, files: summary.files, cacheInfoAfter: await t.cacheInfo() };
}

/** Dispose the transcriber and remove every cached and partial file. */
async function clear() {
  transcriber?.dispose();
  transcriber = undefined;
  return clearModelCache();
}

Object.assign(window, { bench: { load, run, abortCheck, resumeStart, resumeFinish, clear, events } });
document.getElementById("state")!.textContent = "ready";
