import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash, createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import worker, { signedHeaders, signRequest, toIssueSubmission, validateReport } from "./worker/report.mjs";

const requestId = "9f81911b-2a56-4b85-9fc2-316aaef5cd4d";
const receiptId = "4e77aacb-3fa8-4f81-a056-aa24004dd90a";
const ip = "192.0.2.10";
const secret = "synthetic-test-signing-key";
const report = () => ({
  schema: 1, kind: "bug", requestId,
  app: { version: "0.16.0", platform: "windows", arch: "x86_64", os: "Windows 11" },
  notes: "The shortcut stopped responding.\nIt worked before opening Settings.",
  crash: null, diagnostics: null,
});
const crashReport = () => ({
  ...report(), kind: "crash", notes: "",
  crash: { summary: "Rust panic", location: "capture.rs:24", backtrace: "capture::start\napp::main" },
});
const environment = (overrides = {}) => ({
  PATCHBELL_BASE_URL: "https://patchbell.example.test",
  PATCHBELL_PROJECT: "sonojot",
  PATCHBELL_INGEST_KEY_ID: "synthetic-key-id",
  PATCHBELL_INGEST_SECRET: secret,
  REPORT_RATE_LIMITER: { limit: async () => ({ success: true }) },
  ASSETS: { fetch: async () => new Response("asset", { status: 200 }) },
  ...overrides,
});
function request(value = report(), headers = {}, options = {}) {
  return new Request("https://sonojot.example.test/api/report", {
    method: "POST", headers: { "content-type": "application/json", "cf-connecting-ip": ip, ...headers },
    body: typeof value === "string" ? value : JSON.stringify(value), ...options,
  });
}
function upstream(created = true) {
  return Response.json({ id: receiptId, stage: "triage", created }, { status: 201 });
}
function referenceSignature(value, signingSecret = secret) {
  // PatchBell packages/client/src/server.ts and canonical.ts: Node's independent HMAC implementation.
  const digest = createHash("sha256").update(value.body).digest("hex");
  const canonical = [value.method.toUpperCase(), value.path, value.timestamp, value.nonce, digest].join("\n");
  return createHmac("sha256", signingSecret).update(canonical).digest("hex");
}

test("bug and crash contracts accept the app's optional diagnostic object", () => {
  assert.equal(validateReport(report()), true);
  assert.equal(validateReport(crashReport()), true);
  assert.equal(validateReport({ ...report(), diagnostics: {
    setupProfile: "balanced", models: { speech: "moonshine-base", cleanup: "small", notes: "small", meetingSpeech: "moonshine-base" },
    hardware: { ramBytes: 17179869184, physicalCores: 8, logicalCores: 16, gpu: null },
    events: [{ v: 1, event: "capture.start", at_ms: 1234, pid: 10, session: 2, ms: 42 }],
  } }), true);
});

test("requires every contract field and rejects unknown envelope, app and crash fields", () => {
  for (const key of Object.keys(report())) {
    const value = report();
    delete value[key];
    assert.equal(validateReport(value), false, `missing ${key}`);
  }
  assert.equal(validateReport({ ...report(), unexpected: true }), false);
  assert.equal(validateReport({ ...report(), app: { ...report().app, user: "unexpected" } }), false);
  assert.equal(validateReport({ ...crashReport(), crash: { ...crashReport().crash, thread: "unexpected" } }), false);
  for (const value of [null, [], "text", 1, { ...report(), schema: "1" }, { ...report(), kind: "other" }]) {
    assert.equal(validateReport(value), false);
  }
});

test("validates UUID v4, platform and crash/bug invariants", () => {
  for (const invalid of ["", "x".repeat(36), requestId.replace("4b85", "1b85"), requestId.replace("9fc2", "7fc2")]) {
    assert.equal(validateReport({ ...report(), requestId: invalid }), false);
  }
  for (const platform of ["windows", "macos", "linux"]) {
    assert.equal(validateReport({ ...report(), app: { ...report().app, platform } }), true);
  }
  assert.equal(validateReport({ ...report(), app: { ...report().app, platform: "darwin" } }), false);
  assert.equal(validateReport({ ...report(), app: { ...report().app, version: "" } }), false);
  assert.equal(validateReport({ ...report(), app: { ...report().app, arch: "" } }), false);
  assert.equal(validateReport({ ...report(), notes: " \n\t\u2003" }), false);
  assert.equal(validateReport({ ...report(), crash: crashReport().crash }), false);
  assert.equal(validateReport({ ...crashReport(), crash: null }), false);
});

test("enforces field limits in Unicode characters and rejects unpaired surrogates", () => {
  const cases = [
    ["notes", 4000], ["app.version", 80], ["app.arch", 40], ["app.os", 200],
    ["crash.summary", 2000], ["crash.location", 300], ["crash.backtrace", 16384],
  ];
  for (const [path, limit] of cases) {
    const value = crashReport();
    const parts = path.split(".");
    const target = parts.length === 1 ? value : value[parts[0]];
    const key = parts.at(-1);
    target[key] = "😀".repeat(limit);
    assert.equal(validateReport(value), true, path);
    target[key] += "a";
    assert.equal(validateReport(value), false, path);
    target[key] = 42;
    assert.equal(validateReport(value), false, path);
  }
  assert.equal(validateReport({ ...report(), notes: "\ud800" }), false);
});

test("diagnostics accepts bounded extensible JSON objects and rejects pathological structure", () => {
  assert.equal(validateReport({ ...report(), diagnostics: { futureField: [1, true, null, "ok"] } }), true);
  for (const diagnostics of [[], "text", 1, { bad: Infinity }, { events: Array(1025).fill(null) }]) {
    assert.equal(validateReport({ ...report(), diagnostics }), false);
  }
  let nested = {};
  for (let i = 0; i < 18; i++) nested = { child: nested };
  assert.equal(validateReport({ ...report(), diagnostics: nested }), false);
});

test("issue mapping keeps notes first and distinguishes crash fingerprints", () => {
  const value = report();
  const issue = toIssueSubmission(value);
  assert.equal(issue.title, "Bug: SonoJot 0.16.0 on windows: The shortcut stopped responding.");
  assert.ok(issue.description.startsWith(value.notes + "\n\n"));
  assert.equal(issue.environment, "windows");
  assert.deepEqual(issue.context, { kind: "bug", version: "0.16.0", platform: "windows", arch: "x86_64" });
  assert.equal(Object.hasOwn(issue, "fingerprint"), false);
  const crash = toIssueSubmission(crashReport());
  assert.equal(crash.title, "Crash: SonoJot 0.16.0 on windows: Rust panic");
  assert.equal(crash.fingerprint, "crash:0.16.0:capture.rs:24");
});

test("large details stay within PatchBell limits with a closed fence and intact notes", () => {
  const value = crashReport();
  value.notes = "😀".repeat(4000);
  value.crash.summary = "😀".repeat(2000);
  value.crash.backtrace = "`".repeat(16384);
  const issue = toIssueSubmission(value);
  assert.ok(issue.title.length <= 200);
  assert.ok(issue.description.length <= 32000);
  assert.equal(issue.title.isWellFormed(), true);
  assert.equal(issue.description.isWellFormed(), true);
  assert.ok(issue.description.startsWith(value.notes));
  assert.ok(issue.description.endsWith("\n[Details truncated]\n```"));
  assert.equal((issue.description.match(/```/g) ?? []).length, 2);
});

test("Web Crypto signature matches PatchBell's Node client fixture, including UTF-8", async () => {
  const fixture = {
    method: "post", path: "/api/v1/projects/sonojot/ingest/issues",
    timestamp: "2026-10-15T12:00:00.000Z", nonce: "AAECAwQFBgcICQoLDA0ODxAREhMUFRYX",
    body: '{"title":"Crash: SonoJot café 😀","description":"line one\\nline two"}',
  };
  assert.equal(await signRequest(secret, fixture), referenceSignature(fixture));
  await assert.rejects(signRequest(secret, { ...fixture, path: "/bad\npath" }));
  const first = await signedHeaders(secret, "fixture-key", fixture.path, fixture.body, requestId);
  const second = await signedHeaders(secret, "fixture-key", fixture.path, fixture.body, requestId);
  assert.match(first["x-patchbell-nonce"], /^[A-Za-z0-9_-]{32}$/);
  assert.equal(Buffer.from(first["x-patchbell-nonce"], "base64url").length, 24);
  assert.notEqual(first["x-patchbell-nonce"], second["x-patchbell-nonce"]);
  assert.equal(first["idempotency-key"], requestId);
  assert.equal(first["x-patchbell-key-id"], "fixture-key");
  assert.equal(new Date(first["x-patchbell-timestamp"]).toISOString(), first["x-patchbell-timestamp"]);
});

test("forwards the signed issue and returns only a safe receipt", async t => {
  let observed;
  t.mock.method(globalThis, "fetch", async (target, init) => {
    observed = { target, init };
    return upstream();
  });
  const response = await worker.fetch(request(), environment());
  assert.equal(response.status, 202);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(await response.json(), { receiptId, replayed: false });
  assert.equal(observed.target.href, "https://patchbell.example.test/api/v1/projects/sonojot/ingest/issues");
  const { init } = observed;
  assert.equal(init.method, "POST");
  assert.equal(init.redirect, "manual");
  assert.ok(init.signal instanceof AbortSignal);
  assert.deepEqual(JSON.parse(init.body), toIssueSubmission(report()));
  assert.equal(init.headers["content-type"], "application/json");
  assert.equal(init.headers["idempotency-key"], requestId);
  assert.equal(init.headers["x-patchbell-signature"], referenceSignature({
    method: "POST", path: observed.target.pathname, body: init.body,
    timestamp: init.headers["x-patchbell-timestamp"], nonce: init.headers["x-patchbell-nonce"],
  }));
  assert.equal(JSON.stringify(observed).includes(ip), false);
});

test("returns the available duplicate signal from PatchBell", async t => {
  t.mock.method(globalThis, "fetch", async () => upstream(false));
  const response = await worker.fetch(request(crashReport()), environment());
  assert.deepEqual(await response.json(), { receiptId, replayed: true });
});

test("unconfigured or invalid targets fail closed without reading or sending reports", async t => {
  const outbound = t.mock.method(globalThis, "fetch", () => assert.fail("unexpected outbound request"));
  for (const patch of [
    { PATCHBELL_INGEST_KEY_ID: "" }, { PATCHBELL_INGEST_SECRET: undefined },
    { PATCHBELL_BASE_URL: "http://patchbell.example.test" }, { PATCHBELL_PROJECT: " " },
    { PATCHBELL_BASE_URL: "https://user:pass@patchbell.example.test" },
    { PATCHBELL_BASE_URL: "https://patchbell.example.test/wrong-path" },
  ]) {
    const incoming = request();
    const response = await worker.fetch(incoming, environment(patch));
    assert.equal(response.status, 503);
    assert.deepEqual(await response.json(), { error: "Reports are not enabled yet." });
    assert.equal(incoming.bodyUsed, false);
  }
  assert.equal(outbound.mock.callCount(), 0);
});

test("rate limiting uses a stable keyed IP digest and returns 429 before reading the body", async t => {
  t.mock.method(globalThis, "fetch", () => assert.fail("unexpected outbound request"));
  const keys = [];
  const env = environment({ REPORT_RATE_LIMITER: { limit: async ({ key }) => {
    keys.push(key); return { success: false };
  } } });
  for (let i = 0; i < 2; i++) {
    const incoming = request();
    const response = await worker.fetch(incoming, env);
    assert.equal(response.status, 429);
    assert.equal(response.headers.get("retry-after"), "60");
    assert.equal(incoming.bodyUsed, false);
  }
  assert.equal(keys[0], keys[1]);
  assert.equal(keys[0], `sonojot-report:${createHmac("sha256", secret).update(ip).digest("hex")}`);
  assert.ok(!keys[0].includes(ip));
  assert.equal((await worker.fetch(request(), environment({ REPORT_RATE_LIMITER: undefined }))).status, 503);
  const noIp = request(); noIp.headers.delete("cf-connecting-ip");
  assert.equal((await worker.fetch(noIp, env)).status, 400);
});

test("rejects method, media type, malformed JSON and schema violations locally", async t => {
  t.mock.method(globalThis, "fetch", () => assert.fail("unexpected outbound request"));
  const method = await worker.fetch(new Request("https://sonojot.example.test/api/report"), environment());
  assert.equal(method.status, 405);
  assert.equal(method.headers.get("allow"), "POST");
  assert.equal((await worker.fetch(request(report(), { "content-type": "text/plain" }), environment())).status, 415);
  for (const value of ["{", "null", JSON.stringify({ ...report(), surprise: true })]) {
    assert.equal((await worker.fetch(request(value), environment())).status, 400);
  }
});

test("caps actual stream bytes even with missing or dishonest Content-Length", async t => {
  t.mock.method(globalThis, "fetch", async () => upstream());
  const oversized = " ".repeat(65537);
  for (const headers of [{}, { "content-length": "1" }, { "content-length": "65537" }]) {
    assert.equal((await worker.fetch(request(oversized, headers), environment())).status, 413);
  }
  let cancelled = false;
  const stream = new ReadableStream({
    pull(controller) { controller.enqueue(new Uint8Array(40000)); },
    cancel() { cancelled = true; },
  });
  assert.equal((await worker.fetch(request(null, {}, { body: stream, duplex: "half" }), environment())).status, 413);
  assert.equal(cancelled, true);
  const body = JSON.stringify(report());
  const padded = body + " ".repeat(65536 - Buffer.byteLength(body));
  assert.equal((await worker.fetch(request(padded, { "content-type": "application/json; charset=utf-8" }), environment())).status, 202);
  const malformedUtf8 = new Uint8Array([123, 34, 255, 34, 58, 49, 125]);
  assert.equal((await worker.fetch(request(null, {}, { body: malformedUtf8 }), environment())).status, 400);
});

test("upstream errors and invalid receipts do not disclose their contents", async t => {
  const sensitive = "upstream-private-error";
  const fetchMock = t.mock.method(globalThis, "fetch", async () => { throw new Error(sensitive); });
  for (const handler of [
    async () => { throw new Error(sensitive); },
    async () => new Response(sensitive, { status: 500 }),
    async () => new Response("bad json", { status: 200 }),
    async () => Response.json({ id: receiptId, stage: "triage" }),
    async () => Response.json({ id: sensitive, stage: "triage", created: true }),
    async () => new Response(" ".repeat(8193)),
  ]) {
    fetchMock.mock.mockImplementation(handler);
    const response = await worker.fetch(request(), environment());
    assert.equal(response.status, 503);
    assert.deepEqual(await response.json(), { error: "Reports are temporarily unavailable. Try again." });
  }
  fetchMock.mock.mockImplementation(async () => new Response(sensitive, { status: 409 }));
  const conflict = await worker.fetch(request(), environment());
  assert.equal(conflict.status, 409);
  assert.ok(!(await conflict.text()).includes(sensitive));
});

test("other paths preserve the asset binding response", async () => {
  for (const path of ["/", "/privacy", "/missing", "/api/other", "/api/report/"]) {
    const expected = new Response("asset response", { status: 404, headers: { "x-asset": "preserved" } });
    const incoming = new Request(`https://sonojot.example.test${path}`);
    const response = await worker.fetch(incoming, environment({ ASSETS: { fetch: async received => {
      assert.equal(received, incoming); return expected;
    } } }));
    assert.equal(response, expected);
  }
});

test("Worker configuration retains asset routing, a well-formed ingest key id and five-per-minute binding", () => {
  const config = readFileSync(new URL("./wrangler.jsonc", import.meta.url), "utf8");
  assert.match(config, /"main":\s*"worker\/report\.mjs"/);
  assert.match(config, /"run_worker_first":\s*\["\/api\/\*"\]/);
  assert.match(config, /"binding":\s*"ASSETS"/);
  assert.match(config, /"html_handling":\s*"drop-trailing-slash"/);
  assert.match(config, /"not_found_handling":\s*"404-page"/);
  assert.match(config, /"PATCHBELL_INGEST_KEY_ID":\s*"ingest_[0-9a-f]{16}"/);
  assert.doesNotMatch(config, /"PATCHBELL_INGEST_SECRET"\s*:/);
  assert.match(config, /"name":\s*"REPORT_RATE_LIMITER"/);
  assert.match(config, /"simple":\s*\{\s*"limit":\s*5,\s*"period":\s*60\s*\}/);
});
