// Model storage in the browser's Cache Storage, keyed by SHA-256 so a file is
// downloaded once per origin whatever host served it. A small ledger records
// when each file was last used; files idle longer than `maxAgeDays` are
// deleted on the next start. The browser may also evict everything under
// storage pressure (we never request persistent storage), and then files are
// simply downloaded again.
//
// While a file downloads, its bytes go to the Origin Private File System
// (OPFS), one file per SHA-256, through a synchronous access handle. An
// interrupted download (cancelled, network error, tab closed) leaves that
// partial file behind, and the next download of the same file continues with an
// HTTP Range request. The access handle is exclusive, so a second tab or worker
// that finds the file in use downloads into memory instead. Where OPFS is
// missing or fails (private windows, a full disk), downloads go to memory and an
// interruption starts the file again, as before.
import { ALL_FILES, type ModelFile, type ModelUrl } from "./models.ts";
import type { CacheInfo } from "./protocol.ts";

const CACHE_NAME = "sonojot-transcribe-v1";
const LEDGER_KEY = "ledger.json";
/** OPFS folder for downloads in progress. */
const PARTIAL_DIR = "sonojot-transcribe-v1-partial";
const DAY_MS = 86_400_000;

export interface DownloadProgress {
  file: ModelFile;
  loaded: number;
  total: number;
  /** True when the bytes came from the cache rather than the network. */
  cached: boolean;
  /** Bytes already stored by an earlier, interrupted download that this one continues from. */
  resumedFrom: number;
  /** Bytes received from the network so far by this call. */
  fetched: number;
}

export interface StoreOptions {
  url: ModelUrl;
  /** Delete files not used for this many days. Default 30; 0 keeps files until the browser evicts them. */
  maxAgeDays?: number;
  fetch?: typeof fetch;
  /** For tests: the current time. */
  now?: () => number;
}

interface Ledger {
  [sha256: string]: { lastUsed: number; bytes: number; name: string };
}

export class IntegrityError extends Error {
  constructor(file: ModelFile, detail: string) {
    super(`${file.model}/${file.name} failed verification (${detail}).`);
    this.name = "IntegrityError";
  }
}

export class ModelStore {
  readonly #options: StoreOptions;
  #cache?: Promise<Cache | undefined>;
  /** Ledger writes are read-modify-write; run them one at a time. */
  #ledgerQueue: Promise<unknown> = Promise.resolve();

  constructor(options: StoreOptions) {
    this.#options = options;
  }

  /**
   * Bytes of a pinned file, from the cache or the network. Downloads are checked
   * against the pinned SHA-256 before they are cached or returned. Cached files are
   * checked by size only: they were verified when stored, under a key that is their
   * hash, and Cache Storage can only be written by this origin's own scripts, which
   * could change the code that verifies just as easily. Hashing the 678 MB again
   * would add 0.4 s to every start on the Ryzen 9 9900X used for the measurements
   * (more on a slower laptop) and would only catch on-disk corruption that keeps
   * the size intact, which onnxruntime would most likely reject when parsing anyway.
   *
   * A cache hit does not update the file's last-used date; call `touch` once the
   * files are in use. Aborting `signal` stops a download; the bytes received so far
   * stay in OPFS (when available) and the next call continues from them.
   */
  async get(file: ModelFile, onProgress?: (progress: DownloadProgress) => void, signal?: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
    const cache = await this.#open();
    const key = keyFor(file);
    const hit = await cache?.match(key);
    if (hit) {
      const bytes = new Uint8Array(await hit.arrayBuffer());
      if (bytes.byteLength === file.bytes) {
        onProgress?.({ file, loaded: file.bytes, total: file.bytes, cached: true, resumedFrom: 0, fetched: 0 });
        return bytes;
      }
      await cache!.delete(key);
    }
    signal?.throwIfAborted();
    const sink = await openSink(file);
    let kept = false;
    try {
      const bytes = await this.#download(file, sink, onProgress, signal);
      if (cache) {
        try {
          // A finished partial is put from its file, so the bytes are not copied out of the worker again.
          await cache.put(key, new Response((await sink.finish()) ?? bytes, { headers: { "content-type": "application/octet-stream" } }));
          await this.touch([file]);
        } catch {
          // Quota exceeded or storage blocked: keep working without the cache. A finished
          // partial stays, so the next start verifies it instead of downloading it again.
          kept = true;
        }
      } else kept = true;
      return bytes;
    } catch (error) {
      // Keep what arrived for the next attempt, unless it failed verification.
      kept = !(error instanceof IntegrityError);
      throw error;
    } finally {
      await (kept ? sink.close() : sink.discard()).catch(() => undefined);
    }
  }

  /**
   * Fetch the rest of the file into `sink` and verify it. When the verified bytes were
   * joined from stored data or several responses and do not match, the stored data is
   * discarded and the file downloaded once more from the start before giving up.
   */
  async #download(file: ModelFile, sink: Sink, onProgress?: (progress: DownloadProgress) => void, signal?: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
    const fetcher = this.#options.fetch ?? fetch;
    const url = this.#options.url(file);
    const state: FetchState = { resumedFrom: sink.length, fetched: 0, joined: false };
    for (let pass = 0; ; pass++) {
      try {
        await fetchInto(fetcher, url, file, sink, state, onProgress, signal);
        const bytes = await sink.read();
        await verify(file, bytes);
        return bytes;
      } catch (error) {
        if (!(error instanceof IntegrityError) || !state.joined || pass > 0) throw error;
        await sink.reset();
        state.resumedFrom = 0;
        state.joined = false;
      }
    }
  }

  /** Delete files idle for longer than maxAgeDays, including partial downloads. Returns how many were removed. */
  sweep(): Promise<number> {
    const run = this.#ledgerQueue.then(() => this.#sweep());
    this.#ledgerQueue = run.catch(() => undefined);
    return run;
  }

  async #sweep(): Promise<number> {
    const days = this.#options.maxAgeDays ?? 30;
    if (days <= 0) return 0;
    const cache = await this.#open();
    const ledger = cache ? await readLedger(cache) : {};
    const cutoff = this.#now() - days * DAY_MS;
    let removed = 0;
    if (cache) {
      for (const [sha, entry] of Object.entries(ledger)) {
        if (entry.lastUsed < cutoff) {
          await cache.delete(keyFor({ sha256: sha }));
          delete ledger[sha];
          removed++;
        }
      }
      // A file cached without a ledger entry (another tab raced the write) starts its clock now.
      let adopted = false;
      for (const request of await cache.keys()) {
        const sha = request.url.split("/").pop()!;
        if (sha !== LEDGER_KEY && !ledger[sha]) {
          ledger[sha] = { lastUsed: this.#now(), bytes: 0, name: "unknown" };
          adopted = true;
        }
      }
      if (removed || adopted) await writeLedger(cache, ledger);
    }
    // Partial downloads age by their last write. One whose file is already cached is left over
    // from a tab that closed after another finished the same file, and goes too.
    for (const partial of await listPartials()) {
      if (partial.lastModified < cutoff || ledger[partial.sha256]) {
        if (await removePartial(partial.sha256)) removed++;
      }
    }
    return removed;
  }

  /** What is stored, for a settings or storage screen. */
  async info(): Promise<CacheInfo> {
    const cache = await this.#open();
    const ledger = cache ? await readLedger(cache) : {};
    const files = Object.values(ledger);
    const partialFiles = (await listPartials()).map(({ sha256, bytes, lastModified }) => {
      const known = ALL_FILES.find((file) => file.sha256 === sha256);
      return { name: known ? `${known.model}/${known.name}` : "unknown", bytes, total: known?.bytes, lastUsed: lastModified };
    });
    return {
      files,
      bytes: files.reduce((sum, file) => sum + file.bytes, 0),
      partialFiles,
      partialBytes: partialFiles.reduce((sum, file) => sum + file.bytes, 0),
    };
  }

  /** Record that these files were just used, in one ledger write. */
  touch(files: readonly ModelFile[]): Promise<void> {
    const run = this.#ledgerQueue.then(async () => {
      const cache = await this.#open();
      if (!cache) return;
      const ledger = await readLedger(cache);
      for (const file of files) ledger[file.sha256] = { lastUsed: this.#now(), bytes: file.bytes, name: `${file.model}/${file.name}` };
      await writeLedger(cache, ledger);
    });
    this.#ledgerQueue = run.catch(() => undefined);
    return run;
  }

  #now(): number {
    return this.#options.now?.() ?? Date.now();
  }

  #open(): Promise<Cache | undefined> {
    // Cache Storage is missing on insecure origins and can throw in private modes.
    this.#cache ??= (typeof caches === "undefined" ? Promise.resolve(undefined) : caches.open(CACHE_NAME)).catch(() => undefined);
    return this.#cache;
  }
}

/**
 * Remove every cached model file and partial download for this origin. A partial
 * download that another tab is writing right now is left alone.
 */
export async function clearModelCache(): Promise<boolean> {
  let removed = false;
  for (const partial of await listPartials()) removed = (await removePartial(partial.sha256)) || removed;
  const root = await storageRoot();
  await root?.removeEntry(PARTIAL_DIR).catch(() => undefined); // fails while a partial is in use
  if (typeof caches === "undefined") return removed;
  return (await caches.delete(CACHE_NAME)) || removed;
}

function keyFor(file: Pick<ModelFile, "sha256">): string {
  // Cache keys must be http(s) URLs; this path is never fetched.
  return new URL(`/__sonojot-transcribe__/${file.sha256}`, self.location.href).href;
}

async function readLedger(cache: Cache): Promise<Ledger> {
  const response = await cache.match(new URL(`/__sonojot-transcribe__/${LEDGER_KEY}`, self.location.href).href);
  if (!response) return {};
  try {
    return (await response.json()) as Ledger;
  } catch {
    return {};
  }
}

async function writeLedger(cache: Cache, ledger: Ledger): Promise<void> {
  await cache.put(
    new URL(`/__sonojot-transcribe__/${LEDGER_KEY}`, self.location.href).href,
    new Response(JSON.stringify(ledger), { headers: { "content-type": "application/json" } }),
  );
}

interface FetchState {
  /** Stored bytes this download continued from; reset to 0 when they are discarded. */
  resumedFrom: number;
  /** Bytes received from the network. */
  fetched: number;
  /** The bytes came from storage or from more than one response. */
  joined: boolean;
}

/**
 * Fill `sink` up to the pinned size, continuing from what it holds with a Range
 * request. Network errors and early ends retry up to three times, each continuing
 * where the last stopped. A server that ignores Range (200) restarts the file; a 416
 * means the stored bytes cannot belong to this file, so they are dropped.
 */
async function fetchInto(
  fetcher: typeof fetch,
  url: string,
  file: ModelFile,
  sink: Sink,
  state: FetchState,
  onProgress?: (progress: DownloadProgress) => void,
  signal?: AbortSignal,
): Promise<void> {
  const report = () => onProgress?.({ file, loaded: sink.length, total: file.bytes, cached: false, resumedFrom: state.resumedFrom, fetched: state.fetched });
  let lastError: unknown;
  for (let attempt = 0; attempt < 3; attempt++) {
    const offset = sink.length;
    if (offset > 0) state.joined = true;
    if (offset === file.bytes) {
      // Every byte arrived before an earlier run was interrupted, but it was never verified.
      report();
      return;
    }
    try {
      const init: RequestInit = { signal, cache: "no-store", credentials: "omit" };
      // "bytes=N-" is a simple range, which CORS lets through without a preflight.
      if (offset > 0) init.headers = { range: `bytes=${offset}-` };
      const response = await fetcher(url, init);
      if (offset > 0 && response.status === 416) {
        await response.body?.cancel().catch(() => undefined);
        throw await restart(sink, state, `HTTP 416 for ${url} at byte ${offset}`);
      }
      if (!response.ok || !response.body) throw new Error(`HTTP ${response.status} for ${url}`);
      if (offset > 0 && response.status !== 206) await restart(sink, state); // Range ignored: the whole file follows
      else if (response.status === 206) {
        // Content-Range is only readable when the host exposes it to CORS; the hash catches the rest.
        const start = /^bytes (\d+)-/.exec(response.headers.get("content-range") ?? "")?.[1];
        if (start !== undefined && Number(start) !== offset) {
          await response.body.cancel().catch(() => undefined);
          throw await restart(sink, state, `range starting at ${start} instead of ${offset} from ${url}`);
        }
      }
      report();
      const reader = response.body.getReader();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        if (sink.length + value.byteLength > file.bytes) throw new IntegrityError(file, "larger than expected");
        await sink.write(value);
        state.fetched += value.byteLength;
        report();
      }
      if (sink.length !== file.bytes) throw new Error(`${url} ended after ${sink.length} of ${file.bytes} bytes`);
      return;
    } catch (error) {
      if (signal?.aborted || error instanceof IntegrityError) throw error;
      lastError = error;
    }
  }
  throw lastError;
}

/** Drop the stored bytes so the next request starts from zero. Returns an error for the caller to throw when given a message. */
async function restart(sink: Sink, state: FetchState, message?: string): Promise<Error> {
  await sink.reset();
  state.resumedFrom = 0;
  state.joined = false;
  return new Error(message ?? "restarting from zero");
}

async function verify(file: ModelFile, bytes: Uint8Array<ArrayBuffer>): Promise<void> {
  if (bytes.byteLength !== file.bytes) throw new IntegrityError(file, `${bytes.byteLength} of ${file.bytes} bytes`);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  const hex = Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
  if (hex !== file.sha256) throw new IntegrityError(file, "SHA-256 mismatch");
}

// Where download bytes go: an OPFS file that outlives the page, or memory.

interface Sink {
  /** Bytes stored so far; the next write goes here. */
  readonly length: number;
  write(chunk: Uint8Array): Promise<void>;
  /** Drop everything stored. */
  reset(): Promise<void>;
  /** Everything stored, in one buffer. */
  read(): Promise<Uint8Array<ArrayBuffer>>;
  /** After a verified read: the stored file as a Blob, or undefined when the bytes only live in memory. */
  finish(): Promise<Blob | undefined>;
  /** Stop writing and keep what is stored, for a later download to continue from. */
  close(): Promise<void>;
  /** Stop writing and delete what is stored. */
  discard(): Promise<void>;
}

class MemorySink implements Sink {
  length = 0;
  readonly #buffer: Uint8Array<ArrayBuffer>;

  constructor(total: number) {
    this.#buffer = new Uint8Array(total);
  }

  async write(chunk: Uint8Array): Promise<void> {
    this.#buffer.set(chunk, this.length);
    this.length += chunk.byteLength;
  }

  async reset(): Promise<void> {
    this.length = 0;
  }

  async read(): Promise<Uint8Array<ArrayBuffer>> {
    return this.length === this.#buffer.byteLength ? this.#buffer : this.#buffer.slice(0, this.length);
  }

  async finish(): Promise<undefined> {
    return undefined;
  }

  async close(): Promise<void> {}

  async discard(): Promise<void> {}
}

/**
 * The synchronous OPFS handle, declared here because TypeScript's `DOM` lib leaves
 * it to the `WebWorker` lib, which a host type-checking this source may not include.
 */
interface SyncAccessHandle {
  read(buffer: Uint8Array, options?: { at?: number }): number;
  write(buffer: Uint8Array, options?: { at?: number }): number;
  truncate(size: number): void;
  getSize(): number;
  flush(): void;
  close(): void;
}

/**
 * An OPFS file written through a synchronous access handle, which only dedicated
 * workers have. The handle locks the file, so no other tab can write it meanwhile.
 * If a write fails (usually quota), the bytes move to memory and the download goes on.
 */
class FileSink implements Sink {
  length: number;
  readonly #sha256: string;
  readonly #total: number;
  readonly #fileHandle: FileSystemFileHandle;
  #handle?: SyncAccessHandle;
  #memory?: MemorySink;

  constructor(file: ModelFile, fileHandle: FileSystemFileHandle, handle: SyncAccessHandle, length: number) {
    this.#sha256 = file.sha256;
    this.#total = file.bytes;
    this.#fileHandle = fileHandle;
    this.#handle = handle;
    this.length = length;
  }

  async write(chunk: Uint8Array): Promise<void> {
    if (this.#memory) return this.#writeMemory(chunk);
    try {
      // Older Safari returned promises from these methods; awaiting covers both.
      const written = await this.#handle!.write(chunk, { at: this.length });
      if (written !== chunk.byteLength) throw new Error("short write");
      this.length += written;
    } catch {
      const stored = await this.read().catch(() => undefined);
      this.#memory = new MemorySink(this.#total);
      await this.discard();
      if (!stored) {
        // Unreadable too: start the file again, in memory.
        this.length = 0;
        throw new Error("the partial download could not be read back");
      }
      await this.#writeMemory(stored);
      await this.#writeMemory(chunk);
    }
  }

  async #writeMemory(chunk: Uint8Array): Promise<void> {
    await this.#memory!.write(chunk);
    this.length = this.#memory!.length;
  }

  async reset(): Promise<void> {
    if (this.#memory) await this.#memory.reset();
    else await this.#handle!.truncate(0);
    this.length = 0;
  }

  async read(): Promise<Uint8Array<ArrayBuffer>> {
    if (this.#memory) return this.#memory.read();
    const out = new Uint8Array(this.length);
    let at = 0;
    while (at < out.byteLength) {
      const count = await this.#handle!.read(out.subarray(at), { at });
      if (!count) throw new Error(`could read only ${at} of ${out.byteLength} stored bytes`);
      at += count;
    }
    return out;
  }

  async finish(): Promise<Blob | undefined> {
    if (this.#memory || !this.#handle) return undefined;
    await this.#close();
    try {
      return await this.#fileHandle.getFile();
    } catch {
      return undefined;
    }
  }

  async close(): Promise<void> {
    await this.#close();
  }

  async discard(): Promise<void> {
    await this.#close().catch(() => undefined);
    await removePartial(this.#sha256);
  }

  async #close(): Promise<void> {
    const handle = this.#handle;
    this.#handle = undefined;
    if (!handle) return;
    try {
      await handle.flush();
    } finally {
      await handle.close();
    }
  }
}

/** An OPFS-backed sink for `file` when possible, otherwise memory. */
async function openSink(file: ModelFile): Promise<Sink> {
  const dir = await partialDir(true);
  let fileHandle: FileSystemFileHandle | undefined;
  try {
    if (dir) {
      fileHandle = await dir.getFileHandle(file.sha256, { create: true });
      // Throws NoModificationAllowedError while another tab or worker holds the file,
      // and NotSupportedError in builds without the backend (Playwright's WebKit on Windows).
      const handle = await (fileHandle as unknown as { createSyncAccessHandle(): Promise<SyncAccessHandle> }).createSyncAccessHandle();
      try {
        let length = await handle.getSize();
        if (length > file.bytes) {
          // More than the pinned size: it cannot be this file.
          await handle.truncate(0);
          length = 0;
        }
        return new FileSink(file, fileHandle, handle, length);
      } catch (error) {
        await handle.close();
        throw error;
      }
    }
  } catch {
    // Remove the empty file just created, unless it belongs to another writer.
    if ((await fileHandle?.getFile().catch(() => undefined))?.size === 0) await removePartial(file.sha256);
  }
  return new MemorySink(file.bytes);
}

async function storageRoot(): Promise<FileSystemDirectoryHandle | undefined> {
  try {
    return (await navigator.storage?.getDirectory?.()) ?? undefined;
  } catch {
    return undefined; // private windows and insecure origins
  }
}

async function partialDir(create: boolean): Promise<FileSystemDirectoryHandle | undefined> {
  const root = await storageRoot();
  try {
    return await root?.getDirectoryHandle(PARTIAL_DIR, { create });
  } catch {
    return undefined;
  }
}

async function listPartials(): Promise<{ sha256: string; bytes: number; lastModified: number }[]> {
  const dir = await partialDir(false);
  if (!dir) return [];
  const partials = [];
  try {
    // Directory iteration is in TypeScript's `DOM.AsyncIterable` lib, which a host may not include.
    const entries = (dir as unknown as { entries(): AsyncIterable<[string, FileSystemHandle]> }).entries();
    for await (const [name, handle] of entries) {
      if (handle.kind !== "file") continue;
      try {
        // Fails in some browsers while another tab holds the file's access handle.
        const stored = await (handle as FileSystemFileHandle).getFile();
        partials.push({ sha256: name, bytes: stored.size, lastModified: stored.lastModified });
      } catch {
        // in use; leave it out
      }
    }
  } catch {
    return partials;
  }
  return partials;
}

/** Delete one partial download. False when it is missing or in use. */
async function removePartial(sha256: string): Promise<boolean> {
  const dir = await partialDir(false);
  try {
    await dir?.removeEntry(sha256);
    return !!dir;
  } catch {
    return false;
  }
}
