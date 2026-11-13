/// <reference types="vite/client" />
import { createTranscriber, type WasmPaths, clearModelCache, ALL_FILES, type Transcriber, type CleanupMode, type ProgressEvent } from "../src/index.ts";

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

function current(): Transcriber {
  if (!transcriber) {
    const wasmPaths = import.meta.env.VITE_WASM_PATHS as WasmPaths | undefined; // see vite.config.ts
    transcriber = createTranscriber({ modelUrl: MODEL_URL, device: device.value as "auto" | "webgpu" | "wasm", wasmPaths });
    transcriber.onProgress(show);
  }
  return transcriber;
}

function megabytes(bytes: number): string {
  return `${Math.round(bytes / 1e6)} MB`;
}

function show(event: ProgressEvent): void {
  progress.hidden = event.stage === "ready";
  bar.style.width = `${(100 * event.overallLoaded) / event.overallTotal}%`;
  status.textContent =
    event.stage === "download"
      ? `Downloading models: ${megabytes(event.overallLoaded)} of ${megabytes(event.overallTotal)}`
      : event.stage === "load"
        ? "Loading models…"
        : "Models are ready.";
}

async function preload(): Promise<void> {
  const started = performance.now();
  status.textContent = `Preparing ${megabytes(TOTAL)} of models…`;
  await current().preload();
  status.textContent = `Models are ready (${((performance.now() - started) / 1000).toFixed(1)} s).`;
}

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
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
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
  $("cache").textContent = `${info.files.length} files, ${megabytes(info.bytes)}`;
});
$("clear").addEventListener("click", async () => {
  transcriber?.dispose();
  transcriber = undefined;
  await clearModelCache();
  $("cache").textContent = "Removed.";
  status.textContent = "Models are not loaded.";
});
