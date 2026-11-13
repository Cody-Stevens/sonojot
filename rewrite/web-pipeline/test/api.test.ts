// createTranscriber against a fake worker: shared loads, cancellation and options.
import assert from "node:assert/strict";
import { test } from "node:test";
import { createTranscriber, supportsLanguage as fromIndex, type LoadReport } from "../src/index.ts";
import { SUPPORTED_LANGUAGES, supportsLanguage } from "../src/language.ts";
import type { Request, Response, WorkerConfig } from "../src/protocol.ts";

class FakeWorker {
  sent: Request[] = [];
  onmessage: ((event: MessageEvent<Response>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  postMessage(message: Request) {
    this.sent.push(message);
  }
  terminate() {}
  reply(message: Response) {
    this.onmessage?.({ data: message } as MessageEvent<Response>);
  }
  of<T extends Request["type"]>(type: T): Extract<Request, { type: T }>[] {
    return this.sent.filter((m): m is Extract<Request, { type: T }> => m.type === type);
  }
}

const REPORT: LoadReport = {
  device: "wasm",
  cachedFiles: 7,
  downloadedFiles: 0,
  downloadedBytes: 0,
  timings: { totalMs: 1, filesMs: 1, recognitionSessionsMs: 1 },
};

function setup(options: Parameters<typeof createTranscriber>[0] = {}) {
  const worker = new FakeWorker();
  const transcriber = createTranscriber({ ...options, worker: () => worker as unknown as Worker });
  return { worker, transcriber };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("preload callers share one load and resolve with its report", async () => {
  const { worker, transcriber } = setup();
  const a = transcriber.preload();
  const b = transcriber.preload();
  assert.equal(worker.of("preload").length, 1);
  worker.reply({ type: "done", id: worker.of("preload")[0]!.id, value: REPORT });
  assert.deepEqual(await a, REPORT);
  assert.deepEqual(await b, REPORT);
  assert.deepEqual(await transcriber.preload(), REPORT);
  assert.equal(worker.of("preload").length, 1);
});

test("aborting one caller leaves the others waiting and the download running", async () => {
  const { worker, transcriber } = setup();
  const controller = new AbortController();
  const aborted = transcriber.preload({ signal: controller.signal });
  const other = transcriber.preload();
  controller.abort();
  await assert.rejects(aborted, (error: Error) => error instanceof DOMException && error.name === "AbortError");
  assert.equal(worker.of("cancel-load").length, 0);
  worker.reply({ type: "done", id: worker.of("preload")[0]!.id, value: REPORT });
  assert.deepEqual(await other, REPORT);
});

test("when the last caller aborts the downloads stop, and a later preload starts again", async () => {
  const { worker, transcriber } = setup();
  const first = new AbortController();
  const second = new AbortController();
  const a = transcriber.preload({ signal: first.signal });
  const b = transcriber.preload({ signal: second.signal });
  first.abort();
  await assert.rejects(a, { name: "AbortError" });
  assert.equal(worker.of("cancel-load").length, 0);
  second.abort(new DOMException("Gone", "AbortError"));
  await assert.rejects(b, { message: "Gone" });
  assert.equal(worker.of("cancel-load").length, 1);
  // The worker's late answer to the cancelled load is ignored.
  worker.reply({ type: "error", id: worker.of("preload")[0]!.id, name: "AbortError", message: "cancelled" });
  await tick();

  const again = transcriber.preload();
  assert.equal(worker.of("preload").length, 2);
  worker.reply({ type: "done", id: worker.of("preload")[1]!.id, value: REPORT });
  assert.deepEqual(await again, REPORT);
});

test("a preload in the same task as the last abort starts a fresh load", async () => {
  const { worker, transcriber } = setup();
  const controller = new AbortController();
  const aborted = transcriber.preload({ signal: controller.signal });
  controller.abort();
  const fresh = transcriber.preload();
  await assert.rejects(aborted, { name: "AbortError" });
  assert.equal(worker.of("preload").length, 2);
  worker.reply({ type: "done", id: worker.of("preload")[1]!.id, value: REPORT });
  assert.deepEqual(await fresh, REPORT);
});

test("an already aborted signal rejects without starting a load", async () => {
  const { worker, transcriber } = setup();
  await assert.rejects(transcriber.preload({ signal: AbortSignal.abort() }), { name: "AbortError" });
  assert.equal(worker.of("preload").length, 0);
});

test("transcribe's signal cancels the model load it waits for", async () => {
  const { worker, transcriber } = setup();
  const controller = new AbortController();
  const result = transcriber.transcribe(new Float32Array(16_000), { signal: controller.signal });
  await tick();
  assert.equal(worker.of("preload").length, 1);
  controller.abort();
  await assert.rejects(result, (error: Error) => error instanceof DOMException && error.name === "AbortError");
  assert.equal(worker.of("cancel-load").length, 1);
  assert.equal(worker.of("transcribe").length, 0);
});

test("transcribe sends its job once the models are loaded", async () => {
  const { worker, transcriber } = setup({ cleanup: "format" });
  const result = transcriber.transcribe(new Float32Array(16_000));
  await tick();
  worker.reply({ type: "done", id: worker.of("preload")[0]!.id, value: REPORT });
  await tick();
  const [job] = worker.of("transcribe");
  assert.equal(job!.job.cleanup, "format");
  const value = { raw: "a", text: "A.", language: "en", durationSeconds: 1, device: "wasm", timings: { recognitionMs: 1, cleanupMs: 1, totalMs: 2 } };
  worker.reply({ type: "done", id: job!.id, value });
  assert.deepEqual(await result, value);
});

test("a failed load rejects every caller and the next call retries", async () => {
  const { worker, transcriber } = setup();
  const a = transcriber.preload();
  worker.reply({ type: "error", id: worker.of("preload")[0]!.id, name: "Error", message: "HTTP 503" });
  await assert.rejects(a, { message: "HTTP 503" });
  void transcriber.preload();
  assert.equal(worker.of("preload").length, 2);
});

test("wasmPaths reaches the worker with relative URLs resolved against the page", () => {
  Object.assign(globalThis, { document: { baseURI: "https://app.example/dashboard/" } });
  try {
    const config = (options: Parameters<typeof createTranscriber>[0]) =>
      (setup(options).worker.of("init")[0]!.config as WorkerConfig).wasmPaths;
    assert.equal(config({}), undefined);
    assert.equal(config({ wasmPaths: "/ort/" }), "https://app.example/ort/");
    assert.equal(config({ wasmPaths: "ort/" }), "https://app.example/dashboard/ort/");
    assert.deepEqual(config({ wasmPaths: { wasm: "/wasm/ort.wasm" } }), { wasm: "https://app.example/wasm/ort.wasm" });
    assert.deepEqual(
      config({ wasmPaths: { wasm: new URL("https://cdn.example/ort.wasm"), mjs: "./ort.mjs" } }),
      { wasm: "https://cdn.example/ort.wasm", mjs: "https://app.example/dashboard/ort.mjs" },
    );
  } finally {
    delete (globalThis as { document?: unknown }).document;
  }
});

test("supportsLanguage accepts English tags only, from either entry point", () => {
  assert.equal(fromIndex, supportsLanguage);
  assert.deepEqual(SUPPORTED_LANGUAGES, ["en"]);
  for (const tag of [undefined, "", "en", "EN", "en-US", "en-gb"]) assert.equal(supportsLanguage(tag), true, String(tag));
  for (const tag of ["fr", "de-DE", "eng", "english", "en_US", "-en"]) assert.equal(supportsLanguage(tag), false, tag);
});

test("a non-English language rejects before anything is loaded", async () => {
  const { worker, transcriber } = setup();
  await assert.rejects(transcriber.transcribe(new Float32Array(1), { language: "fr" }), { name: "UnsupportedLanguageError" });
  assert.equal(worker.of("preload").length, 0);
});
