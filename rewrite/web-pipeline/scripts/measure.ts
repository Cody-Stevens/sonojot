// Measures the built demo in a real browser: cold first run (download, verify,
// cache, load), warm start from the cache, and transcription speed on public
// benchmark clips with WebGPU and WASM.
//
// usage: tsx scripts/measure.ts --models DIR --audio CORPUS_DIR --clips INFERENCE.jsonl --out RESULTS.json
//        [--limit N] [--isolated] [--hf] [--browser chrome|chromium|firefox|webkit] [--headed] [--devices webgpu,wasm]
//        [--cpu-cap N] [--load-only] [--repeat N] [--abort]
//   --isolated   adds COOP/COEP so WASM can use threads
//   --hf         downloads the models from the pinned Hugging Face commits instead of --models
//   --browser    Chrome (default), Playwright's Chromium, Firefox or WebKit
//   --headed     show the browser window (Playwright's Firefox has no WebGPU adapter when headless)
//   --devices    warm sessions to run (default webgpu,wasm)
//   --cpu-cap N  Windows only: hold the browser's whole process tree to N cores' worth of CPU time
//                (a job object hard cap), as a stand-in for a slower laptop. DevTools CPU throttling is
//                no use here: Chrome applies it to the page only and rejects it for workers.
//   --load-only  time cache starts only (no clips); --repeat N warm starts per device (default 3)
//   --abort      cancellation check: abort a cold download, then preload again and transcribe one clip
//   --resume     resumable downloads: abort a cold load after --abort-bytes (default 150e6), load again
//                on the same page, then abort a second cold load, close the browser, reopen the same
//                profile and load again; reports the bytes the server sent in each phase and transcribes
//                one clip. --resume-limit N aborts the second load after N more bytes instead of
//                downloading everything (for --hf). --two-tabs: a second tab loads while the first resumes.
// Build first: npm run build:demo. To measure the other ways of serving onnxruntime's
// files, set SONOJOT_EXTERN_WASM or SONOJOT_WASM_PATHS for both the build and this script
// (see vite.config.ts).
import { execFile, execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { chromium, firefox, webkit, type BrowserContext, type Page } from "@playwright/test";

const args = new Map<string, string>();
for (let i = 2; i < process.argv.length; i++) {
  const key = process.argv[i]!.replace(/^--/, "");
  const next = process.argv[i + 1];
  if (next && !next.startsWith("--")) {
    args.set(key, next);
    i++;
  } else args.set(key, "true");
}
const models = args.get("models");
const audio = args.get("audio");
const clipsFile = args.get("clips");
const out = args.get("out");
if (!models || !audio || !clipsFile || !out) throw new Error("--models, --audio, --clips and --out are required");
const limit = Number(args.get("limit") ?? Infinity);
const browserName = args.get("browser") ?? "chrome";
const devices = (args.get("devices") ?? "webgpu,wasm").split(",");
const cpuCap = Number(args.get("cpu-cap") ?? 0);
const headless = !args.has("headed");
const repeat = Number(args.get("repeat") ?? 3);
const port = 5318;
/** The local server appends one line per model or audio request here (see vite.config.ts). */
const requestLog = `${out}.requests.jsonl`;
writeFileSync(requestLog, "");
const base = `http://127.0.0.1:${port}`;

const clips = readFileSync(clipsFile, "utf8")
  .split("\n")
  .filter(Boolean)
  .map((line) => JSON.parse(line) as { id: string; audio_path: string })
  .slice(0, limit)
  .map((row) => ({ id: row.id, path: relative(audio, row.audio_path).replaceAll("\\", "/") }));

function startServer(isolated: boolean): Promise<ChildProcess> {
  const env = { ...process.env, SONOJOT_MODEL_DIR: models, SONOJOT_BENCH_AUDIO_DIR: audio, SONOJOT_ISOLATED: isolated ? "1" : "", SONOJOT_REQUEST_LOG: requestLog };
  const child = spawn(process.execPath, ["node_modules/vite/bin/vite.js", "preview", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], { env, stdio: ["ignore", "pipe", "inherit"] });
  return new Promise((resolve, reject) => {
    child.stdout!.on("data", (chunk: Buffer) => {
      if (chunk.toString().includes(String(port))) resolve(child);
    });
    child.on("exit", (code) => reject(new Error(`preview exited ${code}`)));
  });
}

function chromeMemoryScript(profile: string): string {
  const name = profile.split(/[\\/]/).pop()!;
  return (
    `(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | Where-Object { $_.CommandLine -like '*${name}*' } | ` +
    `ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum`
  );
}

/** Private bytes of every Chrome process using this profile (Windows and Chrome only; 0 otherwise). */
function chromeMemory(profile: string): number {
  if (process.platform !== "win32" || browserName !== "chrome") return 0;
  return Number(execFileSync("powershell", ["-NoProfile", "-Command", chromeMemoryScript(profile)], { encoding: "utf8" }).trim()) || 0;
}

/**
 * Samples chromeMemory back to back (each sample takes a few hundred ms) until stopped and
 * resolves with the highest value seen. Short spikes can be missed; it estimates the peak.
 */
function memoryPeak(profile: string): () => Promise<number> {
  if (process.platform !== "win32" || browserName !== "chrome") return async () => 0;
  let peak = 0;
  let running = true;
  const loop = (async () => {
    while (running) {
      const value = await new Promise<number>((resolve) =>
        execFile("powershell", ["-NoProfile", "-Command", chromeMemoryScript(profile)], { encoding: "utf8" }, (_error, stdout) => resolve(Number(stdout.trim()) || 0)),
      );
      peak = Math.max(peak, value);
    }
  })();
  return async () => {
    running = false;
    await loop;
    return peak;
  };
}

/**
 * Put every process of this browser profile into a job object with a hard CPU cap of
 * `cores` cores' worth of time. Processes the browser starts later inherit the job.
 */
function capCpu(profile: string, cores: number): void {
  if (process.platform !== "win32") throw new Error("--cpu-cap is Windows only");
  const name = profile.split(/[\\/]/).pop()!;
  const image = { chrome: "chrome.exe", chromium: "chrome.exe", firefox: "firefox.exe", webkit: "Playwright.exe" }[browserName] ?? "chrome.exe";
  const rate = Math.max(1, Math.round((cores / Number(process.env.NUMBER_OF_PROCESSORS ?? 1)) * 10_000));
  const script = `
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class CpuCap {
  [StructLayout(LayoutKind.Sequential)] struct Rate { public uint Flags; public uint CpuRate; }
  [DllImport("kernel32.dll")] static extern IntPtr CreateJobObject(IntPtr a, string n);
  [DllImport("kernel32.dll")] static extern bool SetInformationJobObject(IntPtr j, int c, ref Rate r, uint l);
  [DllImport("kernel32.dll")] static extern bool AssignProcessToJobObject(IntPtr j, IntPtr p);
  [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint a, bool i, int id);
  public static int Apply(int[] ids, uint rate) {
    IntPtr job = CreateJobObject(IntPtr.Zero, null);
    Rate r = new Rate(); r.Flags = 0x1 | 0x4; r.CpuRate = rate; // enable + hard cap
    if (!SetInformationJobObject(job, 15, ref r, 8)) return -1;
    int n = 0;
    foreach (int id in ids) { IntPtr p = OpenProcess(0x1F0FFF, false, id); if (p != IntPtr.Zero && AssignProcessToJobObject(job, p)) n++; }
    return n;
  }
}
"@
$ids = @(Get-CimInstance Win32_Process -Filter "Name='${image}'" | Where-Object { $_.CommandLine -like '*${name}*' } | ForEach-Object { [int]$_.ProcessId })
[CpuCap]::Apply($ids, ${rate})`;
  const assigned = Number(execFileSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" }).trim());
  if (!(assigned > 0)) throw new Error(`--cpu-cap could not limit the browser (${assigned})`);
}

async function open(profile: string): Promise<BrowserContext> {
  if (browserName === "firefox") {
    return firefox.launchPersistentContext(profile, { headless, firefoxUserPrefs: { "dom.webgpu.enabled": true } });
  }
  if (browserName === "webkit") return webkit.launchPersistentContext(profile, { headless });
  return chromium.launchPersistentContext(profile, {
    channel: browserName === "chrome" ? "chrome" : undefined,
    headless,
    args: ["--enable-unsafe-webgpu", "--enable-features=WebGPU"],
  });
}

type LoadReport = { device: string; cachedFiles: number; downloadedBytes: number; timings: Record<string, number> };
type Bench = {
  load(device: string): Promise<{ ms: number; cachedFiles: number; downloadedBytes: number; downloadEndMs: number; report: LoadReport }>;
  run(paths: string[]): Promise<{ path: string; wallMs: number; raw: string; text: string; device: string; durationSeconds: number; timings: { recognitionMs: number; cleanupMs: number; totalMs: number } }[]>;
  abortCheck(device: string, abortAfterBytes: number, clip: string): Promise<Record<string, unknown>>;
  resumeStart(device: string, bytes: number): Promise<Record<string, unknown>>;
  resumeFinish(device: string, limit?: number): Promise<Record<string, unknown> & { cacheInfoAfter: unknown }>;
  clear(): Promise<boolean>;
};
type BenchWindow = { bench: Bench };

/** page.evaluate has no timeout of its own; a hung worker must fail the run, not stall it. */
function bounded<T>(work: Promise<T>, minutes: number, label: string): Promise<T> {
  let timer: NodeJS.Timeout;
  const limit = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} took longer than ${minutes} min`)), minutes * 60_000);
  });
  return Promise.race([work, limit]).finally(() => clearTimeout(timer));
}

async function openBench(profile: string): Promise<{ context: BrowserContext; page: Page; errors: string[] }> {
  const context = await open(profile);
  const errors: string[] = [];
  if (cpuCap > 0) capCpu(profile, cpuCap);
  const page = await openPage(context, errors);
  return { context, page, errors };
}

async function openPage(context: BrowserContext, errors: string[]): Promise<Page> {
  const page = await context.newPage();
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${base}/bench.html${args.has("hf") ? "?models=hf" : ""}`);
  await page.waitForFunction(() => document.getElementById("state")?.textContent === "ready", undefined, { timeout: 60_000 });
  return page;
}

type NetworkEntry = { url: string; range: string | null; status: number | string };

/** Model requests as the browser reports them (the worker's too, and redirects), for the resume check. */
function recordNetwork(context: BrowserContext): NetworkEntry[] {
  const log: NetworkEntry[] = [];
  const model = (url: string) => /\/model-files\/|huggingface\.co|\.hf\.co/.test(url);
  const short = (url: string) => url.replace(/\?.*$/, "").replace(/^https?:\/\//, "");
  context.on("response", async (response) => {
    const request = response.request();
    if (!model(request.url())) return;
    const headers = await request.allHeaders().catch(() => request.headers());
    log.push({ url: short(request.url()), range: headers.range ?? null, status: response.status() });
  });
  context.on("requestfailed", async (request) => {
    if (!model(request.url())) return;
    const headers = await request.allHeaders().catch(() => request.headers());
    log.push({ url: short(request.url()), range: headers.range ?? null, status: `failed: ${request.failure()?.errorText}` });
  });
  return log;
}

type Served = { range: string | null; status: number; bytes: number };

/** What the local server sent since line `from` of the request log, per model file. */
function served(from: number): { lines: number; total: number; files: Record<string, Served[]> } {
  const lines = readFileSync(requestLog, "utf8").split("\n").filter(Boolean);
  const files: Record<string, Served[]> = {};
  let total = 0;
  for (const line of lines.slice(from)) {
    const entry = JSON.parse(line) as Served & { url: string };
    if (!entry.url.startsWith("/model-files/")) continue;
    (files[entry.url.slice("/model-files/".length)] ??= []).push({ range: entry.range, status: entry.status, bytes: entry.bytes });
    total += entry.bytes;
  }
  return { lines: lines.length, total, files };
}

/** Wait until the server has logged every request; it logs when a response closes. */
async function settledLog(): Promise<void> {
  let last = -1;
  for (let i = 0; i < 20; i++) {
    const size = readFileSync(requestLog, "utf8").length;
    if (size === last) return;
    last = size;
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
}

async function resumeCheck(profile: string) {
  const device = devices[0]!;
  const abortBytes = Number(args.get("abort-bytes") ?? 150e6);
  const limit = args.has("resume-limit") ? Number(args.get("resume-limit")) : undefined;
  const minutes = args.has("hf") ? 20 : 10;
  const result: Record<string, unknown> = { device, abortBytes, limit };
  const phase = async (name: string, work: () => Promise<unknown>) => {
    await settledLog();
    const from = served(0).lines;
    const value = await bounded(work(), minutes, name);
    await settledLog();
    const sent = served(from);
    result[name] = { value, served: sent };
    console.log(`${name}: local server sent ${(sent.total / 1e6).toFixed(1)} MB`);
  };
  const bench = (page: Page) => ({
    start: () => page.evaluate(([d, b]) => (window as unknown as BenchWindow).bench.resumeStart(d as string, b as number), [device, abortBytes] as const),
    finish: (l?: number) => page.evaluate(([d, n]) => (window as unknown as BenchWindow).bench.resumeFinish(d as string, n as number | undefined), [device, l] as const),
    clear: () => page.evaluate(() => (window as unknown as BenchWindow).bench.clear()),
  });

  // 1. Same page: abort a cold load, then load again.
  let { context, page, errors } = await openBench(profile);
  let network = recordNetwork(context);
  try {
    await phase("abortSamePage", () => bench(page).start());
    if (args.has("two-tabs")) {
      // A second tab starts the same load while this one resumes; it finds the partials locked.
      const other = await openPage(context, errors);
      await phase("resumeWithSecondTab", async () => {
        const [first, second] = await Promise.all([bench(page).finish(), bench(other).finish()]);
        return { first, second };
      });
      await other.close();
    } else {
      await phase("resumeSamePage", () => bench(page).finish(limit));
    }
    result.networkSamePage = network;
    result.cleared = await bench(page).clear();
    if (limit || args.has("two-tabs")) return { ...result, errors };
    // 2. Abort another cold load, close the browser, reopen the profile and load again.
    await phase("abortBeforeClose", () => bench(page).start());
  } finally {
    await context.close();
  }
  ({ context, page, errors } = await openBench(profile));
  network = recordNetwork(context);
  try {
    await phase("resumeAfterReopen", () => bench(page).finish());
    result.networkAfterReopen = network;
    const [clip] = await bounded(page.evaluate((p) => (window as unknown as BenchWindow).bench.run(p), [clips[0]!.path]), 10, "clip");
    result.clip = clip;
    return { ...result, errors };
  } finally {
    await context.close();
  }
}

async function browserInfo(page: Page) {
  return page.evaluate(async () => {
    const gpu = (navigator as unknown as { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
    let webgpu = false;
    try {
      webgpu = !!gpu && (await gpu.requestAdapter()) != null;
    } catch {
      webgpu = false;
    }
    return { crossOriginIsolated: self.crossOriginIsolated, webgpu, cores: navigator.hardwareConcurrency, userAgent: navigator.userAgent };
  });
}

async function session(profile: string, device: string, transcribe: boolean) {
  const { context, page, errors } = await openBench(profile);
  try {
    const info = await browserInfo(page);
    const minutes = (args.has("hf") ? 30 : 5) * (cpuCap > 0 ? 4 : 1);
    const stopPeak = memoryPeak(profile);
    const load = await bounded(page.evaluate((d) => (window as unknown as { bench: Bench }).bench.load(d), device), minutes, `${device} load`).finally(stopPeak);
    const memoryPeakDuringLoad = await stopPeak();
    const memoryAfterLoad = chromeMemory(profile);
    let results: Awaited<ReturnType<Bench["run"]>> = [];
    if (transcribe) {
      // One warm-up clip so first-inference shader compilation is reported separately.
      const warmStart = Date.now();
      await bounded(page.evaluate((p) => (window as unknown as { bench: Bench }).bench.run(p), [clips[0]!.path]), 10, "first clip");
      const firstClipMs = Date.now() - warmStart;
      results = await bounded(
        page.evaluate((p) => (window as unknown as { bench: Bench }).bench.run(p), clips.map((c) => c.path)),
        60,
        `${device} transcription`,
      );
      Object.assign(load, { firstClipMs });
    }
    const memoryAfterRun = chromeMemory(profile);
    return { device, info, load, memory: { peakDuringLoad: memoryPeakDuringLoad, afterLoad: memoryAfterLoad, afterRun: memoryAfterRun }, errors, results };
  } catch (error) {
    return { device, error: error instanceof Error ? error.message : String(error), errors, results: [] };
  } finally {
    await context.close();
  }
}

async function abortCheck(profile: string) {
  const { context, page, errors } = await openBench(profile);
  try {
    const result = await bounded(
      page.evaluate(([d, c]) => (window as unknown as { bench: Bench }).bench.abortCheck(d!, 150e6, c!), [devices[0]!, clips[0]!.path]),
      10,
      "abort check",
    );
    return { ...result, errors };
  } finally {
    await context.close();
  }
}

type Session = Awaited<ReturnType<typeof session>>;

function summary(name: string, run: Session): string {
  if ("error" in run && run.error) return `${name}: FAILED ${run.error}${run.errors.length ? `; errors: ${run.errors.slice(0, 3).join(" | ")}` : ""}`;
  const ok = run as Exclude<Session, { error: string }>;
  const audioSeconds = ok.results.reduce((sum, r) => sum + r.durationSeconds, 0);
  const recognition = ok.results.reduce((sum, r) => sum + r.timings.recognitionMs, 0) / 1000;
  const cleanup = ok.results.reduce((sum, r) => sum + r.timings.cleanupMs, 0) / 1000;
  const wall = ok.results.reduce((sum, r) => sum + r.wallMs, 0) / 1000;
  const stages = ok.load.report?.timings ?? {};
  return (
    `${name}: device ${ok.results[0]?.device ?? ok.load.report?.device ?? ok.device}, load ${(ok.load.ms / 1000).toFixed(1)} s ` +
    `(downloaded ${(ok.load.downloadedBytes / 1e6).toFixed(0)} MB, cached files ${ok.load.cachedFiles})` +
    ` [${Object.entries(stages).map(([k, v]) => `${k} ${v}`).join(", ")}]` +
    (ok.results.length
      ? `, ${ok.results.length} clips ${audioSeconds.toFixed(0)} s audio: recognition RTF ${(recognition / audioSeconds).toFixed(4)}, ` +
        `cleanup ${((cleanup / ok.results.length) * 1000).toFixed(0)} ms/clip, end-to-end RTF ${(wall / audioSeconds).toFixed(4)}`
      : "") +
    (ok.memory.afterLoad
      ? `; Chrome private memory ${(ok.memory.peakDuringLoad / 1e9).toFixed(2)} GB peak while loading, ${(ok.memory.afterLoad / 1e9).toFixed(2)} GB loaded, ${(ok.memory.afterRun / 1e9).toFixed(2)} GB after`
      : "") +
    `; isolated=${ok.info.crossOriginIsolated} webgpu=${ok.info.webgpu}` +
    (ok.errors.length ? `; errors: ${ok.errors.slice(0, 3).join(" | ")}` : "")
  );
}

const isolated = args.has("isolated");
const server = await startServer(isolated);
const profile = mkdtempSync(join(tmpdir(), "sonojot-web-bench-"));
try {
  const report: Record<string, unknown> = { browser: browserName, cpuCap, isolated, clips: clips.length };
  const sessions: [string, Session][] = [];
  if (args.has("abort")) {
    report.abort = await abortCheck(profile);
    console.log(`abort: ${JSON.stringify(report.abort)}`);
  } else if (args.has("resume")) {
    report.resume = await resumeCheck(profile);
  } else {
    sessions.push(["cold", await session(profile, "auto", false)]);
    for (const device of devices) {
      if (args.has("load-only")) {
        for (let i = 1; i <= repeat; i++) sessions.push([`warm-${device}-${i}`, await session(profile, device, false)]);
      } else {
        // Key names match what score_browser.py reads.
        sessions.push([`warm${device === "webgpu" ? "Webgpu" : "Wasm"}`, await session(profile, device, true)]);
      }
    }
  }
  for (const [name, run] of sessions) report[name] = run;
  writeFileSync(out, JSON.stringify(report, null, 2));
  for (const [name, run] of sessions) console.log(summary(name, run));
} finally {
  server.kill();
  rmSync(profile, { recursive: true, force: true });
}
