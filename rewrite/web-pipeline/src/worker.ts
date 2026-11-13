/// <reference lib="webworker" />
// The transcription worker: downloads and caches the models, runs speech
// recognition (WebGPU when available, otherwise WASM), then punctuation and
// cleanup. One job runs at a time; the page queues the rest.
import * as ort from "onnxruntime-web/webgpu";
import { Parakeet, AbortError } from "./asr/parakeet.ts";
import { splitForRecognition, SAMPLE_RATE } from "./audio.ts";
import { ALL_FILES, PARAKEET, PUNCTUATION, type ModelFile, type ModelUrl } from "./models.ts";
import type { Device, LoadReport, ProgressEvent, Request, Response, TranscribeJob, TranscribeResult, WorkerConfig } from "./protocol.ts";
import { clearModelCache, ModelStore, type DownloadProgress } from "./store.ts";
import { loadPunctuationPackage, lowerSegmentStartCapitals, postprocess } from "./text/index.ts";
import type { PunctuationPackage } from "./text/contract.ts";

declare const self: DedicatedWorkerGlobalScope;

interface Engine {
  asr: Parakeet;
  punctuation?: PunctuationPackage;
  device: Device;
  report: LoadReport;
}

/** A model load in progress. It can be cancelled only while files are still being fetched. */
interface Loading {
  promise: Promise<Engine>;
  controller: AbortController;
  fetching: boolean;
}

// Warnings about shape ops placed on the CPU are expected; keep the console quiet.
const WASM: ort.InferenceSession.SessionOptions = { executionProviders: ["wasm"], graphOptimizationLevel: "all", logSeverityLevel: 3 };

let config: WorkerConfig | undefined;
let store: ModelStore | undefined;
let current: Loading | undefined;
/** Settles once the downloads of a cancelled or failed load have stopped and their files are cached. */
let settling: Promise<unknown> = Promise.resolve();
const cancels = new Map<number, AbortController>();
let queue: Promise<unknown> = Promise.resolve();

// With cross-origin isolation, onnxruntime-web starts its WebAssembly threads by
// loading this same bundle as workers named "em-pthread". Those workers belong
// to onnxruntime; installing our handler there would replace its own.
if (!self.name.startsWith("em-pthread")) self.onmessage = handle;

function handle(event: MessageEvent<Request>): void {
  const message = event.data;
  switch (message.type) {
    case "init":
      config = message.config;
      if (config.wasmPaths) ort.env.wasm.wasmPaths = config.wasmPaths;
      ort.env.logLevel = "error";
      store = new ModelStore({ url: urlFor(config.modelUrl), maxAgeDays: config.maxAgeDays });
      return;
    case "preload":
      return reply(message.id, async () => (await load()).report);
    case "cancel-load":
      return cancelLoad();
    case "transcribe": {
      const controller = new AbortController();
      cancels.set(message.id, controller);
      const run = queue.then(() => transcribe(message.job, controller.signal));
      queue = run.catch(() => undefined);
      return reply(message.id, () => run, () => cancels.delete(message.id));
    }
    case "cancel":
      cancels.get(message.id)?.abort();
      return;
    case "cache-info":
      return reply(message.id, () => requireStore().info());
    case "clear-cache":
      return reply(message.id, async () => {
        if (current) throw new Error("Models are in use; reload the page before clearing the cache.");
        return clearModelCache();
      });
  }
}

function reply(id: number, work: () => Promise<unknown>, done?: () => void): void {
  work().then(
    (value) => post({ type: "done", id, value }),
    (error: unknown) => {
      const e = error instanceof Error ? error : new Error(String(error));
      post({ type: "error", id, name: e.name, message: e.message });
    },
  ).finally(done);
}

function post(message: Response): void {
  self.postMessage(message);
}

function requireStore(): ModelStore {
  if (!store) throw new Error("The transcription worker was not initialised.");
  return store;
}

function urlFor(source: WorkerConfig["modelUrl"]): ModelUrl {
  if (typeof source === "string") {
    return (file) => source.replaceAll("{model}", file.model).replaceAll("{name}", file.name).replaceAll("{sha256}", file.sha256);
  }
  return (file) => {
    const base = source[file.model];
    if (!base) throw new Error(`No download URL configured for ${file.model}.`);
    return new URL(file.name, base.endsWith("/") ? base : `${base}/`).href;
  };
}

function load(): Promise<Engine> {
  if (current) return current.promise;
  const loading = { controller: new AbortController(), fetching: true } as Loading;
  // A cancelled load may still be writing finished files to the cache; wait so
  // they are read back from there instead of being downloaded again.
  loading.promise = settling.then(() => loadEngine(loading)).catch((error: unknown) => {
    if (current === loading) current = undefined; // let a later call retry, e.g. after a network error
    throw error;
  });
  current = loading;
  return loading.promise;
}

/**
 * The page sends this when every caller waiting for the load has aborted. Downloads
 * stop; files already verified stay cached, and the next load continues from them.
 * Once every file is in hand the load finishes anyway, so a later call finds it ready.
 */
function cancelLoad(): void {
  if (!current?.fetching) return;
  current.controller.abort();
  // The aborted load rejects only after its downloads have stopped.
  settling = current.promise.catch(() => undefined);
  current = undefined;
}

async function loadEngine(loading: Loading): Promise<Engine> {
  const started = performance.now();
  const elapsed = () => Math.round(performance.now() - started);
  const { signal } = loading.controller;
  signal.throwIfAborted();
  const models = requireStore();
  const wanted = config?.device ?? "auto";
  const gpu = wanted === "wasm" ? Promise.resolve(false) : hasWebGpu();
  await models.sweep().catch(() => 0);
  const withPunctuation = config?.punctuation !== false;
  const files = withPunctuation ? ALL_FILES : Object.values(PARAKEET.files);
  const total = files.reduce((sum, file) => sum + file.bytes, 0);
  const loaded = new Map<string, number>();
  const progress = (event: Omit<ProgressEvent, "overallLoaded" | "overallTotal">) => {
    const overallLoaded = [...loaded.values()].reduce((sum, value) => sum + value, 0);
    post({ type: "progress", event: { ...event, overallLoaded, overallTotal: total } });
  };
  const report: LoadReport = {
    device: "wasm",
    cachedFiles: 0,
    downloadedFiles: 0,
    downloadedBytes: 0,
    resumedFiles: 0,
    resumedBytes: 0,
    timings: { totalMs: 0, filesMs: 0, recognitionSessionsMs: 0 },
  };

  // Fetch every file at once, the encoder first because its session takes longest.
  // Each file's bytes are handed out once by `take` and dropped as soon as its
  // session exists, so large buffers are not held for the whole load.
  const order = [PARAKEET.files.encoder, ...files.filter((file) => file !== PARAKEET.files.encoder)];
  const pending = new Map<ModelFile, Promise<Uint8Array<ArrayBuffer>>>();
  for (const file of order) {
    let last: DownloadProgress | undefined;
    const bytes = models.get(
      file,
      (p) => {
        last = p;
        loaded.set(file.sha256, p.loaded);
        progress({ stage: "download", file: `${file.model}/${file.name}`, loaded: p.loaded, total: p.total, cached: p.cached, resumedFrom: p.resumedFrom });
      },
      signal,
    );
    // Counted once the file is verified; a download can restart, so progress alone could count it twice.
    bytes.then(() => {
      if (last?.cached) report.cachedFiles++;
      else if (last) {
        report.downloadedFiles++;
        report.downloadedBytes += last.fetched;
        if (last.resumedFrom > 0) {
          report.resumedFiles! += 1;
          report.resumedBytes! += last.resumedFrom;
        }
      }
    }, () => undefined); // failures are observed below; this also avoids unhandled rejections for files never taken
    pending.set(file, bytes);
  }
  const fetched = Promise.allSettled(pending.values());
  const allFetched = Promise.all(pending.values()).then(() => {
    signal.throwIfAborted(); // cancelled just as the last file arrived
    loading.fetching = false;
    report.timings.filesMs = elapsed();
    progress({ stage: "load" });
  });
  allFetched.catch(() => undefined);
  const take = (file: ModelFile) => {
    const bytes = pending.get(file)!;
    pending.delete(file);
    return bytes;
  };

  const sessions: ort.InferenceSession[] = [];
  const create = async (file: ModelFile, options = WASM) => {
    const session = await ort.InferenceSession.create(await take(file), options);
    sessions.push(session);
    return session;
  };
  try {
    // The tokenizer is about a second of parsing; doing it first overlaps it with the large reads.
    let punctSession: ort.InferenceSession | undefined;
    let punctuation: PunctuationPackage | undefined;
    if (withPunctuation) {
      const [tokenizer, punctConfig] = await Promise.all([take(PUNCTUATION.files.tokenizer), take(PUNCTUATION.files.config)]);
      const tokenizerStarted = performance.now();
      punctuation = loadPunctuation(tokenizer, punctConfig, () => punctSession!);
      report.timings.tokenizerMs = Math.round(performance.now() - tokenizerStarted);
    }

    const recognitionStarted = performance.now();
    const encoder = await createEncoder(await take(PARAKEET.files.encoder), await gpu, wanted);
    sessions.push(encoder.session);
    report.device = encoder.device;
    const [preprocessor, decoder] = await Promise.all([create(PARAKEET.files.preprocessor), create(PARAKEET.files.decoder)]);
    const vocab = new TextDecoder().decode(await take(PARAKEET.files.vocab));
    const asr = new Parakeet(ort, { preprocessor, encoder: encoder.session, decoder }, vocab);
    report.timings.recognitionSessionsMs = Math.round(performance.now() - recognitionStarted);

    if (withPunctuation) {
      const punctStarted = performance.now();
      punctSession = await create(PUNCTUATION.files.model);
      report.timings.punctuationSessionMs = Math.round(performance.now() - punctStarted);
    }
    await allFetched;
    void models.touch(files).catch(() => undefined); // last-used dates for expiry; not needed to start
    report.timings.totalMs = elapsed();
    progress({ stage: "ready" });
    return { asr, punctuation, device: encoder.device, report };
  } catch (error) {
    for (const session of sessions) void session.release().catch(() => undefined);
    if (signal.aborted) {
      await fetched; // every download has stopped and finished files are cached
      throw new DOMException("Loading the models was cancelled.", "AbortError");
    }
    // After a network error the other downloads carry on; the next load waits for
    // them and reads their files from the cache instead of fetching them again.
    settling = fetched;
    throw error;
  }
}

function loadPunctuation(tokenizer: Uint8Array, punctConfig: Uint8Array, session: () => ort.InferenceSession): PunctuationPackage {
  const text = new TextDecoder();
  const pkg = loadPunctuationPackage(JSON.parse(text.decode(punctConfig)), JSON.parse(text.decode(tokenizer)), {
    async run({ inputIds }: { inputIds: number[] }) {
      const ids = BigInt64Array.from(inputIds, (id) => BigInt(id));
      const mask = new BigInt64Array(ids.length).fill(1n);
      const out = await session().run({
        input_ids: new ort.Tensor("int64", ids, [1, ids.length]),
        attention_mask: new ort.Tensor("int64", mask, [1, ids.length]),
      });
      return {
        logits: out.logits!.data as Float32Array,
        spanLogits: out.span_logits?.data as Float32Array | undefined,
      };
    },
  });
  return pkg;
}

async function createEncoder(bytes: Uint8Array, webgpu: boolean, wanted: WorkerConfig["device"]): Promise<{ session: ort.InferenceSession; device: Device }> {
  if (wanted !== "wasm" && webgpu) {
    try {
      const session = await ort.InferenceSession.create(bytes, { ...WASM, executionProviders: ["webgpu"] });
      return { session, device: "webgpu" };
    } catch (error) {
      if (wanted === "webgpu") throw error;
    }
  }
  const session = await ort.InferenceSession.create(bytes, WASM);
  return { session, device: "wasm" };
}

async function hasWebGpu(): Promise<boolean> {
  const gpu = (navigator as Navigator & { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
  if (!gpu) return false;
  try {
    return (await gpu.requestAdapter()) != null;
  } catch {
    return false;
  }
}

async function transcribe(job: TranscribeJob, signal: AbortSignal): Promise<TranscribeResult> {
  const started = performance.now();
  const { asr, punctuation, device } = await load();
  if (signal.aborted) throw new AbortError();
  const pieces: string[] = [];
  for (const [start, end] of splitForRecognition(job.samples, { maxSeconds: Math.min(120, PARAKEET.maxSeconds) })) {
    const text = await asr.transcribe(job.samples.subarray(start, end), signal);
    if (text) pieces.push(text);
  }
  const raw = pieces.join(" ");
  const recognized = performance.now();
  // Each piece is decoded on its own; cleanup reads the join without the
  // capital the recognizer gives a piece's first word mid-sentence.
  const input = job.cleanup === "original" ? raw : (lowerSegmentStartCapitals(raw, pieces) ?? raw);
  const cleaned = await postprocess(input, { mode: job.cleanup, punctuation, signal });
  const finished = performance.now();
  return {
    raw,
    text: cleaned.text,
    language: "en",
    durationSeconds: job.samples.length / SAMPLE_RATE,
    device,
    timings: {
      recognitionMs: Math.round(recognized - started),
      cleanupMs: Math.round(finished - recognized),
      totalMs: Math.round(finished - started),
    },
  };
}
