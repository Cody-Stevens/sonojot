/// <reference types="vite/client" />
import {
  createTranscriber,
  type WasmPaths,
  clearModelCache,
  ALL_FILES,
  type Transcriber,
  type CleanupMode,
  type LoadReport,
  type ProgressEvent,
} from "../src/index.ts";
import { isHardwareAdapter, type AdapterLike } from "../src/gpu.ts";

// The demo server maps /model-files/ to a local folder laid out like the model host.
// ?models=hf uses the package default, the pinned Hugging Face commits.
const MODEL_URL =
  new URLSearchParams(location.search).get("models") === "hf"
    ? undefined
    : ((import.meta.env.VITE_MODEL_URL as string | undefined) ?? "/model-files/{model}/{name}");
const TOTAL = ALL_FILES.reduce((sum, file) => sum + file.bytes, 0);

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const status = $("status");
const bar = document.querySelector<HTMLElement>(".bar")!;
const progress = document.querySelector<HTMLElement>(".progress")!;
const device = $<HTMLSelectElement>("device");
const cleanup = $<HTMLSelectElement>("cleanup");
const record = $<HTMLButtonElement>("record");

let transcriber: Transcriber | undefined;
/** Set once the current transcriber's load report is on the page; a new transcriber clears it. */
let reported = false;
/** Set when any file in this load continues an interrupted download. */
let resuming = false;

function current(): Transcriber {
  if (!transcriber) {
    const wasmPaths = import.meta.env.VITE_WASM_PATHS as WasmPaths | undefined; // see vite.config.ts
    transcriber = createTranscriber({ modelUrl: MODEL_URL, device: device.value as "auto" | "webgpu" | "wasm", wasmPaths });
    transcriber.onProgress(show);
    reported = false;
    resuming = false;
  }
  return transcriber;
}

function megabytes(bytes: number): string {
  return `${Math.round(bytes / 1e6)} MB`;
}

function seconds(ms: number | undefined): string {
  return `${((ms ?? 0) / 1000).toFixed(1)} s`;
}

function show(event: ProgressEvent): void {
  progress.hidden = event.stage === "ready";
  bar.style.width = `${(100 * event.overallLoaded) / event.overallTotal}%`;
  if ((event.resumedFrom ?? 0) > 0) resuming = true;
  status.textContent =
    event.stage === "download"
      ? `${resuming ? "Resuming" : "Downloading"} models: ${megabytes(event.overallLoaded)} of ${megabytes(event.overallTotal)}`
      : event.stage === "load"
        ? "Loading models…"
        : "Models are ready.";
}

/** What a hand test needs to see: the device, where the files came from, and where the time went. */
function describe(report: LoadReport): string {
  const t = report.timings;
  const resumed = report.resumedFiles ? `, ${report.resumedFiles} resumed (${megabytes(report.resumedBytes ?? 0)} kept from before)` : "";
  return (
    `Loaded on ${report.device} in ${seconds(t.totalMs)}: ${report.cachedFiles} files from the browser's cache, ` +
    `${report.downloadedFiles} downloaded (${megabytes(report.downloadedBytes)} fetched${resumed}). ` +
    `Files ${seconds(t.filesMs)}, speech model ${seconds(t.recognitionSessionsMs)}, punctuation ${seconds(t.punctuationSessionMs)}, ` +
    `tokenizer ${seconds(t.tokenizerMs)}.`
  );
}

async function preload(): Promise<void> {
  const started = performance.now();
  status.textContent = `Preparing ${megabytes(TOTAL)} of models…`;
  const report = await current().preload();
  status.textContent = `Models are ready (${((performance.now() - started) / 1000).toFixed(1)} s).`;
  if (!reported) {
    $("load").textContent = describe(report);
    reported = true;
  }
}

async function describeBrowser(): Promise<void> {
  const gpu = (navigator as Navigator & { gpu?: { requestAdapter(): Promise<AdapterLike | null> } }).gpu;
  let webgpu = "not available";
  try {
    const adapter = gpu ? await gpu.requestAdapter() : null;
    if (adapter) webgpu = isHardwareAdapter(adapter) ? "available" : "software only (Automatic uses WASM)";
  } catch {
    webgpu = "not available";
  }
  const opfs = typeof navigator.storage?.getDirectory === "function" ? "yes" : "no";
  $("browser").textContent =
    `This browser: WebGPU ${webgpu}; origin private file system ${opfs}; ` +
    `secure context ${self.isSecureContext ? "yes" : "no (the microphone needs localhost or HTTPS)"}. ` +
    `Models from ${MODEL_URL ? "this server" : "Hugging Face"}.`;
}
void describeBrowser();

async function run(audio: Blob): Promise<void> {
  try {
    await preload();
    status.textContent = "Transcribing…";
    const result = await current().transcribe(audio, { language: "en", cleanup: cleanup.value as CleanupMode });
    $("output").hidden = false;
    $("text").textContent = result.text || "(no speech)";
    $("raw").textContent = result.raw || "(no speech)";
    const rtf = result.timings.totalMs / 1000 / Math.max(result.durationSeconds, 0.001);
    $("timings").textContent =
      `${result.durationSeconds.toFixed(1)} s of audio · recognition ${result.timings.recognitionMs} ms on ${result.device} · ` +
      `punctuation and cleanup ${result.timings.cleanupMs} ms · real-time factor ${rtf.toFixed(3)}`;
    status.textContent = "Done.";
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  }
}

$("preload").addEventListener("click", () => preload().catch((e: Error) => (status.textContent = e.message)));
device.addEventListener("change", () => {
  transcriber?.dispose();
  transcriber = undefined;
  $("load").textContent = "";
});
$<HTMLInputElement>("file").addEventListener("change", (event) => {
  const file = (event.target as HTMLInputElement).files?.[0];
  if (file) void run(file);
});

let recorder: MediaRecorder | undefined;
record.addEventListener("click", async () => {
  if (recorder?.state === "recording") {
    recorder.stop();
    return;
  }
  void current().preload().catch(() => undefined); // download while the user speaks
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
  } catch (error) {
    status.textContent = `The microphone is not available: ${error instanceof Error ? error.message : String(error)}`;
    return;
  }
  const chunks: Blob[] = [];
  recorder = new MediaRecorder(stream);
  recorder.ondataavailable = (event) => chunks.push(event.data);
  recorder.onstop = () => {
    stream.getTracks().forEach((track) => track.stop());
    record.textContent = "Record";
    void run(new Blob(chunks, { type: recorder!.mimeType }));
  };
  recorder.start();
  record.textContent = "Stop";
});

$("cache-info").addEventListener("click", async () => {
  const info = await current().cacheInfo();
  // A finished download the cache refused (Chrome's incognito) stays in OPFS and is reused.
  const partials = info.partialFiles ?? [];
  const finished = partials.filter((p) => p.total !== undefined && p.bytes >= p.total);
  const unfinished = partials.filter((p) => !finished.includes(p));
  const sum = (files: typeof partials) => megabytes(files.reduce((total, file) => total + file.bytes, 0));
  const parts = [`${info.files.length} files, ${megabytes(info.bytes)}`];
  if (finished.length) parts.push(`${finished.length} finished downloads kept outside the cache, ${sum(finished)}`);
  if (unfinished.length) parts.push(`${unfinished.length} unfinished downloads, ${sum(unfinished)}`);
  $("cache").textContent = parts.join("; ");
});
$("clear").addEventListener("click", async () => {
  transcriber?.dispose();
  transcriber = undefined;
  await clearModelCache();
  $("cache").textContent = "Removed.";
  $("load").textContent = "";
  status.textContent = "Models are not loaded.";
});
