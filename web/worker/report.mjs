const MAX_REPORT_BYTES = 64 * 1024;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const PLATFORMS = new Set(["windows", "macos", "linux"]);
const ISSUE_STAGES = new Set([
  "triage", "deferred", "rejected", "queued", "working", "blocked", "review",
  "merge_authorized", "merged", "resolved",
]);
const encoder = new TextEncoder();

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function fields(value, names) {
  return object(value) && Object.keys(value).length === names.length
    && names.every(name => Object.hasOwn(value, name));
}

function boundedString(value, limit) {
  return typeof value === "string" && !/[\uD800-\uDFFF]/u.test(value)
    && [...value].length <= limit;
}

// Diagnostics are app-projected JSON, not a second schema for app internals.
// Bound structure as well as wire bytes before serializing it for PatchBell.
function diagnosticsValid(value) {
  if (value === null) return true;
  if (!object(value)) return false;
  const pending = [[value, 0]];
  let nodes = 0;
  while (pending.length) {
    const [item, depth] = pending.pop();
    if (++nodes > 4096 || depth > 16) return false;
    if (item === null || typeof item === "boolean") continue;
    if (typeof item === "number") {
      if (!Number.isFinite(item)) return false;
    } else if (typeof item === "string") {
      if (!boundedString(item, MAX_REPORT_BYTES)) return false;
    } else if (object(item) || Array.isArray(item)) {
      const entries = Object.entries(item);
      if (entries.length > 1024) return false;
      for (const [key, child] of entries) {
        if (!boundedString(key, 200)) return false;
        pending.push([child, depth + 1]);
      }
    } else return false;
  }
  return true;
}

export function validateReport(value) {
  if (!fields(value, ["schema", "kind", "requestId", "app", "notes", "crash", "diagnostics"])) return false;
  if (value.schema !== 1 || !["bug", "crash"].includes(value.kind)
    || typeof value.requestId !== "string" || !UUID_V4.test(value.requestId)) return false;
  if (!fields(value.app, ["version", "platform", "arch", "os"])
    || !boundedString(value.app.version, 80) || !PLATFORMS.has(value.app.platform)
    || !boundedString(value.app.arch, 40) || !boundedString(value.app.os, 200)) return false;
  if (!value.app.version.length || !value.app.arch.length) return false;
  if (!boundedString(value.notes, 4000) || !diagnosticsValid(value.diagnostics)) return false;
  if (value.kind === "bug") return value.notes.trim().length > 0 && value.crash === null;
  return fields(value.crash, ["summary", "location", "backtrace"])
    && boundedString(value.crash.summary, 2000) && boundedString(value.crash.location, 300)
    && boundedString(value.crash.backtrace, 16384);
}

// Use the stricter UTF-16 bound for PatchBell, without cutting a surrogate pair.
function truncate(value, limit) {
  let end = Math.min(value.length, limit);
  if (end > 0 && /[\uD800-\uDBFF]/.test(value[end - 1])) end--;
  return value.slice(0, end);
}

export function toIssueSubmission(report) {
  const summary = report.kind === "crash" ? report.crash.summary
    : report.notes.trim().split(/\r?\n/, 1)[0];
  const label = report.kind === "crash" ? "Crash" : "Bug";
  const title = truncate(`${label}: SonoJot ${report.app.version} on ${report.app.platform}: ${summary}`
    .replace(/\s+/g, " ").trim(), 200);
  const start = `${report.notes}${report.notes ? "\n\n" : ""}Report details\n\n\`\`\`json\n`;
  const end = "\n\`\`\`";
  const details = JSON.stringify({ app: report.app, crash: report.crash, diagnostics: report.diagnostics }, null, 2)
    .replaceAll("`", "\\u0060");
  const budget = 32000 - start.length - end.length;
  const shortened = details.length > budget;
  const marker = "\n[Details truncated]";
  const description = start + (shortened ? truncate(details, budget - marker.length) + marker : details) + end;
  return {
    title,
    description,
    environment: report.app.platform,
    ...(report.kind === "crash" ? {
      fingerprint: truncate(`crash:${report.app.version}:${report.crash.location}`, 500),
    } : {}),
    context: {
      kind: report.kind, version: report.app.version,
      platform: report.app.platform, arch: report.app.arch,
    },
  };
}

function hex(buffer) {
  return [...new Uint8Array(buffer)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

async function hmac(secret, value) {
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return hex(await crypto.subtle.sign("HMAC", key, encoder.encode(value)));
}

export async function signRequest(secret, { method, path, timestamp, nonce, body }) {
  const digest = hex(await crypto.subtle.digest("SHA-256", encoder.encode(body)));
  const parts = [method.toUpperCase(), path, timestamp, nonce, digest];
  if (parts.some(part => !part || /[\r\n]/.test(part))) throw new Error("Invalid signing input");
  return hmac(secret, parts.join("\n"));
}

export async function signedHeaders(secret, keyId, path, body, requestId) {
  const timestamp = new Date().toISOString();
  const nonce = btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(24))))
    .replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
  return {
    "content-type": "application/json",
    "idempotency-key": requestId,
    "x-patchbell-key-id": keyId,
    "x-patchbell-nonce": nonce,
    "x-patchbell-timestamp": timestamp,
    "x-patchbell-signature": await signRequest(secret, { method: "POST", path, timestamp, nonce, body }),
  };
}

class BodyTooLarge extends Error {}

async function boundedJson(message, limit) {
  const length = message.headers.get("content-length");
  if (length !== null && (!/^\d+$/.test(length) || Number(length) > limit)) throw new BodyTooLarge();
  if (!message.body) throw new SyntaxError("Missing JSON body");
  const reader = message.body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let bytes = 0;
  let text = "";
  let complete = false;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) { complete = true; break; }
      bytes += value.byteLength;
      if (bytes > limit) throw new BodyTooLarge();
      text += decoder.decode(value, { stream: true });
    }
    return JSON.parse(text + decoder.decode());
  } finally {
    if (!complete) await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

function json(status, body, headers = {}) {
  return Response.json(body, { status, headers: { "cache-control": "no-store", ...headers } });
}

function unavailable() {
  return json(503, { error: "Reports are temporarily unavailable. Try again." });
}

function configuration(env) {
  if (![env.PATCHBELL_BASE_URL, env.PATCHBELL_PROJECT, env.PATCHBELL_INGEST_KEY_ID, env.PATCHBELL_INGEST_SECRET]
    .every(value => typeof value === "string" && value.trim().length > 0)) return null;
  try {
    const base = new URL(env.PATCHBELL_BASE_URL);
    if (base.protocol !== "https:" || base.username || base.password || base.search || base.hash
      || base.pathname !== "/" || /[\r\n]/.test(env.PATCHBELL_INGEST_KEY_ID)) return null;
    return new URL(`/api/v1/projects/${encodeURIComponent(env.PATCHBELL_PROJECT)}/ingest/issues`, base);
  } catch { return null; }
}

async function reportRequest(request, env) {
  if (request.method !== "POST") return json(405, { error: "Use POST to send a report." }, { allow: "POST" });
  if (request.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase() !== "application/json") {
    return json(415, { error: "Reports must use application/json." });
  }
  const target = configuration(env);
  if (!target) return json(503, { error: "Reports are not enabled yet." });
  const ip = request.headers.get("cf-connecting-ip");
  if (!ip) return json(400, { error: "Could not identify the request." });
  // Only a keyed digest reaches the counter; never store, forward or log the IP.
  const key = `sonojot-report:${await hmac(env.PATCHBELL_INGEST_SECRET, ip)}`;
  if (!(await env.REPORT_RATE_LIMITER.limit({ key })).success) {
    return json(429, { error: "Too many reports. Please try again in a minute." }, { "retry-after": "60" });
  }
  let report;
  try { report = await boundedJson(request, MAX_REPORT_BYTES); }
  catch (error) {
    return error instanceof BodyTooLarge
      ? json(413, { error: "Reports must be no larger than 64 KiB." })
      : json(400, { error: "The report must contain valid JSON." });
  }
  if (!validateReport(report)) return json(400, { error: "The report has invalid or unsupported fields." });
  const body = JSON.stringify(toIssueSubmission(report));
  const response = await fetch(target, {
    // Workers support only follow/manual; a redirect is a non-OK response and fails closed.
    method: "POST", body, redirect: "manual", signal: AbortSignal.timeout(15000),
    headers: await signedHeaders(env.PATCHBELL_INGEST_SECRET, env.PATCHBELL_INGEST_KEY_ID,
      target.pathname, body, report.requestId),
  });
  if (!response.ok) {
    await response.body?.cancel();
    if (response.status === 409) return json(409, { error: "This report ID was already used for different content." });
    return unavailable();
  }
  const receipt = await boundedJson(response, 8192);
  if (!fields(receipt, ["id", "stage", "created"]) || typeof receipt.id !== "string"
    || !UUID.test(receipt.id) || !ISSUE_STAGES.has(receipt.stage) || typeof receipt.created !== "boolean") {
    return unavailable();
  }
  // Trusted ingest exposes creation/deduplication, not exact retry replay status.
  return json(202, { receiptId: receipt.id, replayed: !receipt.created });
}

export default {
  async fetch(request, env) {
    if (new URL(request.url).pathname !== "/api/report") return env.ASSETS.fetch(request);
    try { return await reportRequest(request, env); }
    catch { return unavailable(); } // Never include report data or upstream error text.
  },
};
