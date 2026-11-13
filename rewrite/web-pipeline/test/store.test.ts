// ModelStore against an in-memory Cache Storage, an in-memory OPFS and fake fetch.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { beforeEach, test } from "node:test";
import type { ModelFile } from "../src/models.ts";

class MemoryCache {
  entries = new Map<string, { body: ArrayBuffer; type: string }>();
  async match(key: string) {
    const entry = this.entries.get(key);
    return entry ? new Response(entry.body.slice(0), { headers: { "content-type": entry.type } }) : undefined;
  }
  async put(key: string, response: Response) {
    this.entries.set(key, { body: await response.arrayBuffer(), type: response.headers.get("content-type") ?? "" });
  }
  async delete(key: string | Request) {
    return this.entries.delete(typeof key === "string" ? key : key.url);
  }
  async keys() {
    return [...this.entries.keys()].map((url) => new Request(url));
  }
}

/** Just enough of OPFS: one folder level, files with an exclusive synchronous access handle. */
class MemoryFile {
  bytes = new Uint8Array(0);
  lastModified = 0;
  locked = false;
  /** Writes after this many bytes in total throw, like a full disk. */
  quota = Infinity;
}

class MemoryFileHandle {
  readonly kind = "file";
  readonly entry: MemoryFile;
  readonly clock: () => number;
  constructor(entry: MemoryFile, clock: () => number) {
    this.entry = entry;
    this.clock = clock;
  }
  async getFile() {
    return Object.assign(new Blob([this.entry.bytes.slice()]), { lastModified: this.entry.lastModified });
  }
  async createSyncAccessHandle() {
    const entry = this.entry;
    if (entry.locked) throw new DOMException("in use", "NoModificationAllowedError");
    entry.locked = true;
    const touch = () => (entry.lastModified = this.clock());
    return {
      getSize: () => entry.bytes.byteLength,
      read(buffer: Uint8Array, { at = 0 } = {}) {
        const part = entry.bytes.subarray(at, at + buffer.byteLength);
        buffer.set(part);
        return part.byteLength;
      },
      write(buffer: Uint8Array, { at = 0 } = {}) {
        if (at + buffer.byteLength > entry.quota) throw new DOMException("full", "QuotaExceededError");
        if (at + buffer.byteLength > entry.bytes.byteLength) {
          const grown = new Uint8Array(at + buffer.byteLength);
          grown.set(entry.bytes);
          entry.bytes = grown;
        }
        entry.bytes.set(buffer, at);
        touch();
        return buffer.byteLength;
      },
      truncate(size: number) {
        entry.bytes = entry.bytes.slice(0, size);
        touch();
      },
      flush() {},
      close() {
        entry.locked = false;
      },
    };
  }
}

class MemoryDirectory {
  readonly kind = "directory";
  files = new Map<string, MemoryFile>();
  dirs = new Map<string, MemoryDirectory>();
  readonly clock: () => number;
  constructor(clock: () => number) {
    this.clock = clock;
  }
  async getDirectoryHandle(name: string, { create = false } = {}) {
    if (!this.dirs.has(name)) {
      if (!create) throw new DOMException("missing", "NotFoundError");
      this.dirs.set(name, new MemoryDirectory(this.clock));
    }
    return this.dirs.get(name)!;
  }
  async getFileHandle(name: string, { create = false } = {}) {
    if (!this.files.has(name)) {
      if (!create) throw new DOMException("missing", "NotFoundError");
      const file = new MemoryFile();
      file.lastModified = this.clock();
      this.files.set(name, file);
    }
    return new MemoryFileHandle(this.files.get(name)!, this.clock);
  }
  async removeEntry(name: string) {
    const file = this.files.get(name);
    if (file?.locked) throw new DOMException("in use", "NoModificationAllowedError");
    if (!this.files.delete(name) && !this.dirs.delete(name)) throw new DOMException("missing", "NotFoundError");
  }
  async *entries(): AsyncGenerator<[string, MemoryFileHandle | MemoryDirectory]> {
    for (const [name, file] of this.files) yield [name, new MemoryFileHandle(file, this.clock)];
    for (const [name, dir] of this.dirs) yield [name, dir];
  }
}

let cache: MemoryCache;
let opfs: MemoryDirectory | undefined;
let now = 0;
beforeEach(() => {
  cache = new MemoryCache();
  now = 0;
  opfs = new MemoryDirectory(() => now);
  Object.assign(globalThis, {
    caches: { open: async () => cache, delete: async () => cache.entries.size > 0 && (cache.entries.clear(), true) },
    self: { location: { href: "https://app.example/" } },
  });
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { storage: { getDirectory: async () => opfs ?? Promise.reject(new DOMException("no OPFS", "SecurityError")) } },
  });
});

/** The partial downloads folder, once the store has created it. */
function partials(): Map<string, MemoryFile> {
  return opfs?.dirs.get("sonojot-transcribe-v1-partial")?.files ?? new Map();
}

function fileFor(bytes: Uint8Array, name = "a.onnx"): ModelFile {
  return { model: "m", name, bytes: bytes.byteLength, sha256: createHash("sha256").update(bytes).digest("hex") };
}

function fakeFetch(bodies: Record<string, Uint8Array>, calls: string[]): typeof fetch {
  return (async (url: string) => {
    calls.push(url);
    const body = bodies[url];
    return body ? new Response(body as Uint8Array<ArrayBuffer>) : new Response(null, { status: 404 });
  }) as unknown as typeof fetch;
}

interface Call {
  range: string | null;
}

/**
 * A host serving `body`. With `ranges` it answers "bytes=N-" with 206; otherwise it ignores
 * Range. `stallAt` makes the first response stop after that many bytes until the request is aborted.
 */
function rangeHost(body: Uint8Array, options: { ranges?: boolean; stallAt?: number; status?: (call: Call, index: number) => number | undefined } = {}) {
  const calls: Call[] = [];
  const fetcher = (async (_url: string, init?: RequestInit) => {
    const range = new Headers(init?.headers).get("range");
    const call = { range };
    calls.push(call);
    const forced = options.status?.(call, calls.length - 1);
    if (forced) return new Response(null, { status: forced });
    let start = 0;
    let status = 200;
    const headers: Record<string, string> = {};
    const match = range && options.ranges !== false ? /^bytes=(\d+)-$/.exec(range) : null;
    if (match) {
      start = Number(match[1]);
      if (start >= body.byteLength) return new Response(null, { status: 416 });
      status = 206;
      headers["content-range"] = `bytes ${start}-${body.byteLength - 1}/${body.byteLength}`;
    }
    const rest = body.slice(start);
    if (calls.length === 1 && options.stallAt !== undefined) {
      const signal = init!.signal!;
      return new Response(
        new ReadableStream<Uint8Array>({
          start(stream) {
            stream.enqueue(rest.slice(0, options.stallAt));
            signal.addEventListener("abort", () => stream.error(signal.reason));
          },
        }),
        { status, headers },
      );
    }
    return new Response(rest, { status, headers });
  }) as unknown as typeof fetch;
  return { calls, fetcher };
}

const BODY = Uint8Array.from({ length: 200 }, (_, i) => (i * 7) % 251);

/** Abort a first download of BODY once 80 bytes have arrived; the partial stays behind. */
async function interrupted(store: import("../src/store.ts").ModelStore, file: ModelFile) {
  const controller = new AbortController();
  await assert.rejects(
    store.get(file, (p) => p.loaded >= 80 && controller.abort(), controller.signal),
    { name: "AbortError" },
  );
}

test("downloads, verifies, caches and then serves from the cache", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const bytes = new TextEncoder().encode("model bytes");
  const file = fileFor(bytes);
  const calls: string[] = [];
  const store = new ModelStore({ url: (f) => `https://host/${f.model}/${f.name}`, fetch: fakeFetch({ "https://host/m/a.onnx": bytes }, calls) });
  const progress: boolean[] = [];
  assert.deepEqual(await store.get(file, (p) => progress.push(p.cached)), bytes);
  assert.deepEqual(await store.get(file, (p) => progress.push(p.cached)), bytes);
  assert.equal(calls.length, 1);
  assert.equal(progress.at(-1), true);
  assert.equal((await store.info()).files.length, 1);
  assert.equal(partials().size, 0, "the partial is removed once the file is cached");
});

test("rejects bytes that do not match the pinned hash", async () => {
  const { ModelStore, IntegrityError } = await import("../src/store.ts");
  const file = fileFor(new TextEncoder().encode("expected"));
  const calls: string[] = [];
  const store = new ModelStore({ url: () => "https://host/x", fetch: fakeFetch({ "https://host/x": new TextEncoder().encode("tampered") }, calls) });
  await assert.rejects(store.get(file), IntegrityError);
  assert.equal(cache.entries.size, 0);
  assert.equal(partials().size, 0);
  assert.equal(calls.length, 1, "a download from zero that fails verification is not repeated");
});

test("files unused for longer than maxAgeDays are swept", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const old = new TextEncoder().encode("old");
  const fresh = new TextEncoder().encode("fresh");
  const bodies = { "https://host/m/old": old, "https://host/m/fresh": fresh };
  const store = new ModelStore({ url: (f) => `https://host/m/${f.name}`, fetch: fakeFetch(bodies, []), maxAgeDays: 30, now: () => now });
  await store.get(fileFor(old, "old"));
  now = 20 * 86_400_000;
  await store.get(fileFor(fresh, "fresh"));
  now = 40 * 86_400_000;
  assert.equal(await store.sweep(), 1);
  const names = (await store.info()).files.map((f) => f.name);
  assert.deepEqual(names, ["m/fresh"]);
});

test("maxAgeDays 0 never sweeps", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const bytes = new TextEncoder().encode("x");
  const store = new ModelStore({ url: () => "https://host/x", fetch: fakeFetch({ "https://host/x": bytes }, []), maxAgeDays: 0, now: () => now });
  await store.get(fileFor(bytes));
  now = 1e12;
  assert.equal(await store.sweep(), 0);
});

test("cache hits leave the ledger alone until touch records them in one write", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const a = new TextEncoder().encode("a");
  const b = new TextEncoder().encode("bb");
  now = 1;
  const bodies = { "https://host/m/a": a, "https://host/m/b": b };
  const store = new ModelStore({ url: (f) => `https://host/m/${f.name}`, fetch: fakeFetch(bodies, []), now: () => now });
  const files = [fileFor(a, "a"), fileFor(b, "b")];
  for (const file of files) await store.get(file);
  now = 5;
  let writes = 0;
  const put = cache.put.bind(cache);
  cache.put = async (key, response) => {
    writes++;
    return put(key, response);
  };
  for (const file of files) await store.get(file);
  assert.equal(writes, 0);
  assert.deepEqual((await store.info()).files.map((f) => f.lastUsed), [1, 1]);
  await store.touch(files);
  assert.equal(writes, 1);
  assert.deepEqual((await store.info()).files.map((f) => f.lastUsed), [5, 5]);
});

test("an aborted download keeps its bytes and the next get resumes with a Range request", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80 });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  assert.equal(cache.entries.size, 0);
  assert.equal(partials().get(file.sha256)?.bytes.byteLength, 80);
  assert.deepEqual((await store.info()).partialFiles, [{ name: "unknown", bytes: 80, total: undefined, lastUsed: 0 }]);
  assert.equal((await store.info()).partialBytes, 80);

  const events: { loaded: number; resumedFrom: number; fetched: number }[] = [];
  assert.deepEqual(await store.get(file, ({ loaded, resumedFrom, fetched }) => events.push({ loaded, resumedFrom, fetched })), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null, "bytes=80-"]);
  assert.deepEqual(events[0], { loaded: 80, resumedFrom: 80, fetched: 0 }, "progress starts at the resumed offset");
  assert.deepEqual(events.at(-1), { loaded: 200, resumedFrom: 80, fetched: 120 });
  assert.equal(partials().size, 0);
  assert.equal(cache.entries.size, 2); // the file and the ledger
});

test("a new store (a reload, or another session) resumes from the same partial", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80 });
  await interrupted(new ModelStore({ url: () => "https://old-host/x", fetch: host.fetcher }), file);
  // Partials are keyed by hash, so a different host continues the same file.
  const urls: string[] = [];
  const fetcher = ((url: string, init?: RequestInit) => {
    urls.push(url);
    return host.fetcher(url, init);
  }) as typeof fetch;
  assert.deepEqual(await new ModelStore({ url: () => "https://new-host/x", fetch: fetcher }).get(file), BODY);
  assert.deepEqual(urls, ["https://new-host/x"]);
  assert.equal(host.calls.at(-1)!.range, "bytes=80-");
});

test("a server that ignores Range sends the whole file, which replaces the partial", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80, ranges: false });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  const events: { loaded: number; resumedFrom: number }[] = [];
  assert.deepEqual(await store.get(file, ({ loaded, resumedFrom }) => events.push({ loaded, resumedFrom })), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null, "bytes=80-"]);
  assert.deepEqual(events[0], { loaded: 0, resumedFrom: 0 }, "progress restarts from zero");
});

test("416 drops the partial and downloads from zero", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80, status: (call) => (call.range ? 416 : undefined) });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  assert.deepEqual(await store.get(file), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null, "bytes=80-", null]);
});

test("a partial longer than the pinned size is dropped before anything is fetched", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const dir = await opfs!.getDirectoryHandle("sonojot-transcribe-v1-partial", { create: true });
  (await dir.getFileHandle(file.sha256, { create: true })) && (dir.files.get(file.sha256)!.bytes = new Uint8Array(300));
  const host = rangeHost(BODY);
  assert.deepEqual(await new ModelStore({ url: () => "https://host/x", fetch: host.fetcher }).get(file), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null]);
});

test("a complete but unverified partial is verified without fetching", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const dir = await opfs!.getDirectoryHandle("sonojot-transcribe-v1-partial", { create: true });
  await dir.getFileHandle(file.sha256, { create: true });
  dir.files.get(file.sha256)!.bytes = BODY.slice();
  const host = rangeHost(BODY);
  assert.deepEqual(await new ModelStore({ url: () => "https://host/x", fetch: host.fetcher }).get(file), BODY);
  assert.equal(host.calls.length, 0);
});

test("a corrupt partial fails verification once, then the file is downloaded again from zero", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80 });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  partials().get(file.sha256)!.bytes[10]! ^= 0xff;
  assert.deepEqual(await store.get(file), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null, "bytes=80-", null]);
  assert.equal(partials().size, 0);
});

test("when the clean retry fails too, IntegrityError is reported and nothing is kept", async () => {
  const { ModelStore, IntegrityError } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const tampered = BODY.slice();
  tampered[150]! ^= 0xff;
  const host = rangeHost(tampered, { stallAt: 80 });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  await assert.rejects(store.get(file), IntegrityError);
  assert.deepEqual(host.calls.map((c) => c.range), [null, "bytes=80-", null]);
  assert.equal(partials().size, 0);
  assert.equal(cache.entries.size, 0);
});

test("a network error mid-download continues with a Range request in the same call", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  let first = true;
  const host = rangeHost(BODY);
  const fetcher = (async (url: string, init?: RequestInit) => {
    if (!first) return host.fetcher(url, init);
    first = false;
    let sent = false;
    return new Response(
      new ReadableStream<Uint8Array>({
        pull(stream) {
          if (sent) return stream.error(new TypeError("network error"));
          stream.enqueue(BODY.slice(0, 50));
          sent = true;
        },
      }),
    );
  }) as unknown as typeof fetch;
  assert.deepEqual(await new ModelStore({ url: () => "https://host/x", fetch: fetcher }).get(file), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), ["bytes=50-"]);
});

test("a write failure (quota) moves the download to memory without failing it", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const dir = await opfs!.getDirectoryHandle("sonojot-transcribe-v1-partial", { create: true });
  await dir.getFileHandle(file.sha256, { create: true });
  dir.files.get(file.sha256)!.quota = 100;
  const host = rangeHost(BODY);
  // Deliver in 40-byte chunks so the failing write comes mid-stream.
  const chunked = (async (url: string, init?: RequestInit) => {
    const response = await host.fetcher(url, init);
    const bytes = new Uint8Array(await response.arrayBuffer());
    return new Response(
      new ReadableStream<Uint8Array>({
        start(stream) {
          for (let at = 0; at < bytes.byteLength; at += 40) stream.enqueue(bytes.slice(at, at + 40));
          stream.close();
        },
      }),
      { status: response.status, headers: response.headers },
    );
  }) as unknown as typeof fetch;
  assert.deepEqual(await new ModelStore({ url: () => "https://host/x", fetch: chunked }).get(file), BODY);
  assert.equal(host.calls.length, 1, "the download carried on in memory");
  assert.equal(partials().size, 0, "the partial was given up to free space");
  assert.equal(cache.entries.size, 2);
});

test("without OPFS, downloads work in memory and an abort starts again from zero", async () => {
  const { ModelStore } = await import("../src/store.ts");
  opfs = undefined;
  const file = fileFor(BODY);
  const host = rangeHost(BODY, { stallAt: 80 });
  const store = new ModelStore({ url: () => "https://host/x", fetch: host.fetcher });
  await interrupted(store, file);
  assert.deepEqual(await store.get(file), BODY);
  assert.deepEqual(host.calls.map((c) => c.range), [null, null]);
  assert.deepEqual((await store.info()).partialFiles, []);
});

test("a second writer finds the partial locked and downloads into memory without touching it", async () => {
  const { ModelStore } = await import("../src/store.ts");
  const file = fileFor(BODY);
  const controller = new AbortController();
  const slow = rangeHost(BODY, { stallAt: 80 });
  const first = new ModelStore({ url: () => "https://host/x", fetch: slow.fetcher });
  let reached!: () => void;
  const atEighty = new Promise<void>((resolve) => (reached = resolve));
  const pending = first.get(file, (p) => p.loaded >= 80 && reached(), controller.signal);
  await atEighty;
  // The other tab: same file, while the first still holds the partial's access handle.
  const other = rangeHost(BODY);
  assert.deepEqual(await new ModelStore({ url: () => "https://host/x", fetch: other.fetcher }).get(file), BODY);
  assert.deepEqual(other.calls.map((c) => c.range), [null]);
  assert.equal(partials().get(file.sha256)?.bytes.byteLength, 80, "the first writer's partial is untouched");
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  // The leftover partial belongs to a file that is now cached: the sweep removes it.
  assert.equal(partials().size, 1);
  await first.sweep();
  assert.equal(partials().size, 0);
});

test("partials expire with maxAgeDays and clearModelCache removes them", async () => {
  const { ModelStore, clearModelCache } = await import("../src/store.ts");
  const a = Uint8Array.from(BODY);
  const b = Uint8Array.from(BODY, (x) => x ^ 1);
  const store = new ModelStore({ url: (f) => `https://host/${f.name}`, fetch: rangeHost(a, { stallAt: 80 }).fetcher, maxAgeDays: 30, now: () => now });
  await interrupted(store, fileFor(a, "a"));
  now = 20 * 86_400_000;
  await interrupted(new ModelStore({ url: () => "https://host/b", fetch: rangeHost(b, { stallAt: 80 }).fetcher }), fileFor(b, "b"));
  now = 40 * 86_400_000;
  assert.equal(await store.sweep(), 1);
  assert.deepEqual([...partials().keys()], [fileFor(b).sha256]);
  assert.equal(await clearModelCache(), true);
  assert.equal(partials().size, 0);
  assert.equal(opfs!.dirs.size, 0, "the partials folder goes too");
});
