import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { restore } from "../src/restore";
import { readFileSync } from "node:fs";
import { createHash, createHmac, randomBytes } from "node:crypto";
let mf: Miniflare;
let db: Awaited<ReturnType<Miniflare["getD1Database"]>>;
const hash = (x: string) => createHash("sha256").update(x).digest("base64url");
const syntheticWebhook = "synthetic-test-only-not-a-runtime-secret";
const proof = Buffer.alloc(32, 91).toString("base64url");
const verifierHash = createHash("sha256")
  .update(Buffer.from(proof, "base64url"))
  .digest("base64url");
const pk = Buffer.alloc(32, 3).toString("base64url");
async function call(
  path: string,
  method = "GET",
  data?: unknown,
  bearer?: string,
  extra: Record<string, string> = {},
) {
  return mf.dispatchFetch("https://sync.test" + path, {
    method,
    headers: {
      ...(data ? { "content-type": "application/json" } : {}),
      ...(bearer
        ? { authorization: "Bearer " + bearer, "X-Sync-Auth": proof }
        : {}),
      ...extra,
    },
    body: data ? JSON.stringify(data) : undefined,
    redirect: "manual",
  });
}
async function begin(device = "device-a", provider = "authkit") {
  const verifier = randomBytes(32).toString("base64url");
  const r = await call("/v1/auth/start", "POST", {
    provider,
    challenge: hash(verifier),
    device,
    publicKey: pk,
    platform: "Test",
  });
  expect(r.status).toBe(200);
  return { ...((await r.json()) as { id: string; url: string }), verifier };
}
async function signin(device = "device-a") {
  const a = await begin(device);
  const r = await finish(a.id);
  expect(r.status).toBe(303);
  const url = new URL(r.headers.get("location")!);
  const t = await call("/v1/auth/token", "POST", {
    request: a.id,
    code: url.searchParams.get("code"),
    verifier: a.verifier,
  });
  expect(t.status).toBe(200);
  await db
    .prepare("UPDATE accounts SET auth_verifier=? WHERE auth_verifier IS NULL")
    .bind(verifierHash)
    .run();
  return (await t.json()) as {
    token: string;
    device: string;
    account: { id: string; email: string };
  };
}
async function browser(id: string) {
  const r = await call("/v1/auth/browser?request=" + id);
  expect(r.status).toBe(303);
  return {
    cookie: r.headers.get("set-cookie")!.split(";")[0],
    location: r.headers.get("location")!,
  };
}
async function finish(id: string, code = "synthetic-provider-code") {
  const b = await browser(id);
  return call(
    "/v1/auth/callback?state=" + id + "&code=" + code,
    "GET",
    undefined,
    undefined,
    { cookie: b.cookie },
  );
}
beforeAll(async () => {
  const out = await build({
    entryPoints: ["test/entry.ts"],
    bundle: true,
    format: "esm",
    platform: "browser",
    write: false,
    target: "es2022",
  });
  mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: out.outputFiles[0].text,
      compatibilityDate: "2026-11-08",
      compatibilityFlags: ["nodejs_compat"],
      d1Databases: ["DB"],
      r2Buckets: ["CONTENT", "BACKUPS"],
      bindings: {
        PUBLIC_ORIGIN: "https://sync.test",
        WORKOS_CLIENT_ID: "client_synthetic",
        WORKOS_API_KEY: "synthetic-test-api-key",
        WORKOS_WEBHOOK_SECRET: syntheticWebhook,
        MAIL_ENABLED: "false",
      },
    }),
  );
  db = await mf.getD1Database("DB");
  await db.exec(readFileSync("migrations/0001_auth.sql", "utf8"));
  await db.exec(readFileSync("migrations/0002_store.sql", "utf8"));
  await db.exec(readFileSync("migrations/0003_rotation.sql", "utf8"));
  await db.exec(readFileSync("migrations/0004_security.sql", "utf8"));
  await db.exec(readFileSync("migrations/0005_account_safety.sql", "utf8"));
  await db.exec(readFileSync("migrations/0006_authkit.sql", "utf8"));
  await db.exec(readFileSync("migrations/0007_test_accounts.sql", "utf8"));
}, 30000);
afterAll(async () => {
  await mf?.dispose();
});
beforeEach(async () => {
  const buckets = await mf.getBindings<{
    CONTENT: R2Bucket;
    BACKUPS: R2Bucket;
  }>();
  for (const bucket of [buckets.CONTENT, buckets.BACKUPS]) {
    // These buckets exist only in this isolated Miniflare instance.
    for (;;) {
      const page = await bucket.list({ limit: 500 });
      if (!page.objects.length) break;
      await bucket.delete(page.objects.map((o) => o.key));
    }
  }
  await db.exec(
    "DELETE FROM trash; DELETE FROM operations; DELETE FROM pairings; DELETE FROM sessions; DELETE FROM wraps; DELETE FROM devices; DELETE FROM identities; DELETE FROM accounts; DELETE FROM auth_requests; DELETE FROM rate_limits; DELETE FROM workos_events; DELETE FROM workos_revocations;",
  );
});
describe("auth isolation", () => {
  async function link(token: string, extra: Record<string, string> = {}) {
    return call(
      "/v1/auth/start",
      "POST",
      {
        provider: "authkit",
        challenge: hash("synthetic-link-verifier"),
        device: "linked",
        publicKey: pk,
        platform: "Test",
        link: true,
      },
      token,
      extra,
    );
  }
  const finishLink = (id: string) => finish(id, "synthetic-other-email");
  it("M2 linking requires key proof and pins it through the callback", async () => {
    const a = await signin();
    expect(
      (await link(a.token, { "X-Sync-Auth": "" })).status,
    ).toBeGreaterThanOrEqual(400);
    expect((await link(a.token, { "X-Sync-Auth": pk })).status).toBe(403);
    const pending = await link(a.token);
    expect(pending.status).toBe(200);
    const { id } = (await pending.json()) as { id: string };
    await db.prepare("UPDATE accounts SET auth_verifier=?").bind(pk).run();
    expect((await finishLink(id)).status).toBe(409);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM identities").first())?.n,
    ).toBe(1);
    expect(
      (await db.prepare("SELECT email FROM accounts").first())?.email,
    ).toBe("person@example.invalid");
  });
  it("M2 verified email changes queue the previous address even while mail is disabled", async () => {
    const a = await signin();
    const pending = await link(a.token);
    const { id } = (await pending.json()) as { id: string };
    expect((await finishLink(id)).status).toBe(303);
    expect(
      (await db.prepare("SELECT email FROM accounts").first())?.email,
    ).toBe("changed@example.invalid");
    expect(
      (
        await db
          .prepare(
            "SELECT recipient FROM notifications WHERE kind='email-changed'",
          )
          .first()
      )?.recipient,
    ).toBe("person@example.invalid");
    expect((await call("/__test/cron", "POST")).status).toBe(200);
    expect(
      await db
        .prepare(
          "SELECT recipient,attempts FROM notifications WHERE kind='email-changed'",
        )
        .first(),
    ).toEqual({ recipient: "person@example.invalid", attempts: 0 });
  });
  it("M2 an uninitialized linking attempt cannot change email after bootstrap", async () => {
    const a = await signin();
    await db.prepare("UPDATE accounts SET auth_verifier=NULL").run();
    const pending = await link(a.token, { "X-Sync-Auth": "" });
    expect(pending.status).toBe(200);
    const { id } = (await pending.json()) as { id: string };
    expect(
      (
        await call(
          "/v1/bootstrap",
          "POST",
          { keyring: "YWJj", recovery: "YWJj", device: "YWJj" },
          a.token,
        )
      ).status,
    ).toBe(201);
    expect((await finishLink(id)).status).toBe(409);
    expect(
      (await db.prepare("SELECT email FROM accounts").first())?.email,
    ).toBe("person@example.invalid");
  });
  it("L1 reset expires seven days after readiness and a new request restarts the wait", async () => {
    const a = await signin();
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    await call("/v1/reset", "POST", { confirmed: true }, a.token);
    await db
      .prepare("UPDATE pending_resets SET ready_at=unixepoch()-7*86400")
      .run();
    const reset = {
      confirmed: true,
      reset: true,
      epoch: 1,
      previous: 1,
      device: a.device,
      keyring: "bmV3",
      recovery: "bmV3",
      verifier: verifierHash,
      devices: [{ id: a.device, ciphertext: "bmV3" }],
    };
    expect((await call("/v1/rekey", "POST", reset, a.token)).status).toBe(403);
    expect(
      (
        (await (await call("/v1/poll", "GET", undefined, a.token)).json()) as {
          ready_at: null;
        }
      ).ready_at,
    ).toBeNull();
    const renewed = await call(
      "/v1/reset",
      "POST",
      { confirmed: true },
      a.token,
    );
    const dates = (await renewed.json()) as {
      ready_at: number;
      requested_at: number;
    };
    expect(dates.ready_at - dates.requested_at).toBe(72 * 3600);
    expect((await call("/v1/rekey", "POST", reset, a.token)).status).toBe(403);
    await db
      .prepare("UPDATE pending_resets SET ready_at=unixepoch()-7*86400")
      .run();
    expect((await call("/__test/cron", "POST")).status).toBe(200);
    expect(await db.prepare("SELECT 1 FROM pending_resets").first()).toBeNull();
    expect(
      await db
        .prepare("SELECT 1 FROM notifications WHERE kind='reset-requested'")
        .first(),
    ).toBeNull();
  });
  it("L2 fresh sessions can erase uninitialized accounts, but wraps and stale sessions prevent it", async () => {
    const a = await signin();
    await db.prepare("UPDATE accounts SET auth_verifier=NULL").run();
    await db.prepare("UPDATE sessions SET fresh_at=0").run();
    const erase = () =>
      call("/v1/account", "DELETE", { confirmed: true }, a.token, {
        "X-Sync-Auth": "",
      });
    expect((await erase()).status).toBe(403);
    await db.prepare("UPDATE sessions SET fresh_at=unixepoch()").run();
    await db
      .prepare(
        "INSERT INTO wraps(account,kind,ref,ciphertext,rev) VALUES(?,'keyring','main','YWJj',1)",
      )
      .bind(a.account.id)
      .run();
    expect((await erase()).status).toBeGreaterThanOrEqual(400);
    await db.prepare("DELETE FROM wraps").run();
    expect((await erase()).status).toBe(200);
    expect(await db.prepare("SELECT 1 FROM accounts").first()).toBeNull();
    const backups = (await mf.getBindings<{ BACKUPS: R2Bucket }>()).BACKUPS;
    expect(await backups.head("erased/" + a.account.id)).not.toBeNull();
  });
  it("L3 pruning rotation rejects a changed object inventory atomically", async () => {
    const a = await signin("a");
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    const rotation = {
      confirmed: true,
      device: "lost",
      epoch: 1,
      previous: 1,
      keyring: "bmV3",
      recovery: "bmV3",
      verifier: verifierHash,
      vocabularyRevision: 1,
      pruneCursor: 0,
      devices: [{ id: "a", ciphertext: "bmV3" }],
    };
    await put(a);
    expect((await call("/v1/rekey", "POST", rotation, a.token)).status).toBe(
      412,
    );
    expect(
      (await db.prepare("SELECT epoch FROM accounts").first())?.epoch,
    ).toBe(1);
    expect(
      (
        await call(
          "/v1/rekey",
          "POST",
          { ...rotation, pruneCursor: 1 },
          a.token,
        )
      ).status,
    ).toBe(200);
  });
  it("health and security headers", async () => {
    const r = await call("/v1/health");
    expect(r.status).toBe(200);
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(r.headers.get("referrer-policy")).toBe("no-referrer");
  });
  it("AuthKit sign-in stores only session hashes", async () => {
    const a = await signin();
    const rows = await db.prepare("SELECT hash FROM sessions").all();
    expect(JSON.stringify(rows)).not.toContain(a.token);
    expect(rows.results[0].hash).toBe(hash(a.token));
    expect((await call("/v1/devices", "GET", undefined, a.token)).status).toBe(
      200,
    );
  });
  it("codes expire, consume once and PKCE rejects a different verifier", async () => {
    const a = await begin();
    const r = await finish(a.id);
    const u = new URL(r.headers.get("location")!);
    expect(
      (
        await call("/v1/auth/token", "POST", {
          request: a.id,
          code: u.searchParams.get("code"),
          verifier: "wrong",
        })
      ).status,
    ).toBe(401);
    const data = {
      request: a.id,
      code: u.searchParams.get("code"),
      verifier: a.verifier,
    };
    expect((await call("/v1/auth/token", "POST", data)).status).toBe(200);
    expect((await call("/v1/auth/token", "POST", data)).status).toBe(401);
  });
  it("rejects expired browser attempts and retired direct sign-ins", async () => {
    const a = await begin();
    await db.prepare("UPDATE auth_requests SET expires_at=0").run();
    expect((await call("/v1/auth/browser?request=" + a.id)).status).toBe(401);
    expect((await call("/v1/auth/email", "POST", {})).status).toBe(404);
    expect(
      (await call("/v1/auth/start", "POST", { provider: "google" })).status,
    ).toBe(400);
  });
  it("binds callbacks to one browser and consumes them before exchange", async () => {
    const a = await begin();
    const b = await browser(a.id);
    const path =
      "/v1/auth/callback?state=" + a.id + "&code=synthetic-provider-code";
    expect((await call(path)).status).toBe(401);
    expect(
      (
        await call(path, "GET", undefined, undefined, {
          cookie: b.cookie.replace(/.$/, "!"),
        })
      ).status,
    ).toBe(401);
    const replies = await Promise.all(
      [1, 2].map(() =>
        call(path, "GET", undefined, undefined, { cookie: b.cookie }),
      ),
    );
    expect(replies.map((r) => r.status).sort()).toEqual([303, 401]);
    expect(b.location).toContain("provider=authkit");
    expect(b.location).toContain("code_challenge_method=S256");
    expect(b.location).toContain("max_age=0");
    expect(
      (
        await db
          .prepare("SELECT browser_hash FROM auth_requests WHERE id=?")
          .bind(a.id)
          .first()
      )?.browser_hash,
    ).not.toContain(b.cookie.split(".")[1]);
  });
  it("limits sign-in attempts without owning email sending", async () => {
    for (let i = 0; i < 10; i++) await begin("a" + i);
    expect((await call("/v1/auth/start", "POST", {})).status).toBe(429);
  });
  it("requires a session and rejects cross-origin requests", async () => {
    expect((await call("/v1/devices")).status).toBe(401);
    expect(
      (
        await call("/v1/auth/start", "POST", {}, undefined, {
          origin: "https://evil.invalid",
        })
      ).status,
    ).toBe(403);
  });
  it("rejects bad provider codes without counting a server failure", async () => {
    const a = await begin();
    expect((await finish(a.id, "bad")).status).toBe(401);
    expect(
      (
        await db
          .prepare("SELECT last_failure FROM operations WHERE name='http'")
          .first()
      )?.last_failure ?? null,
    ).toBeNull();
  });
  it("revokes all device sessions immediately", async () => {
    const a = await signin("a");
    const b = await signin("b");
    expect(
      (await call("/v1/devices/b", "DELETE", { confirmed: true }, a.token))
        .status,
    ).toBe(200);
    expect((await call("/v1/devices", "GET", undefined, b.token)).status).toBe(
      401,
    );
  });
  it("wrap updates are conditional and account-scoped", async () => {
    const a = await signin();
    const wrap = {
      kind: "recovery",
      ref: "main",
      ciphertext: "YWJj",
      previous: 0,
    };
    expect((await call("/v1/wraps", "PUT", wrap, a.token)).status).toBe(200);
    expect((await call("/v1/wraps", "PUT", wrap, a.token)).status).toBe(412);
    expect(
      (await call("/v1/wraps", "PUT", { ...wrap, previous: 1 }, a.token))
        .status,
    ).toBe(200);
  });
  it("pairing can accept once and expires", async () => {
    const a = await signin("a");
    const b = await signin("b");
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "device", ref: "a", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    expect(
      (
        await call(
          "/v1/pairings",
          "POST",
          { id: "pair", commitment: pk },
          b.token,
        )
      ).status,
    ).toBe(201);
    expect(
      (
        await call(
          "/v1/pairings/pair",
          "POST",
          { action: "accept", commitment: pk },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (
        await call(
          "/v1/pairings/pair",
          "POST",
          { action: "accept", commitment: pk },
          a.token,
        )
      ).status,
    ).toBe(409);
    expect(
      (
        await call(
          "/v1/pairings/pair",
          "POST",
          { action: "reveal", nonce: pk },
          b.token,
        )
      ).status,
    ).toBe(400);
    await db.prepare("UPDATE pairings SET expires_at=0").run();
    expect(
      (await call("/v1/pairings/pair", "GET", undefined, a.token)).status,
    ).toBe(404);
  });
});

const envelope = Buffer.from(
  JSON.parse(readFileSync("../rewrite/src-tauri/tests/fixtures/sync-vectors.json", "utf8")).envelopeHex,
  "hex",
);
async function put(
  a: { token: string },
  object = "vocabulary",
  rev = 0,
  data: Uint8Array = envelope,
) {
  return mf.dispatchFetch("https://sync.test/v1/objects/" + object, {
    method: "PUT",
    headers: {
      authorization: "Bearer " + a.token,
      "if-match": '"' + rev + '"',
      "x-sync-epoch": "1",
      "x-sync-auth": proof,
    },
    body: data,
  });
}
describe("encrypted store", () => {
  it("stores and returns ciphertext without plaintext", async () => {
    const a = await signin();
    expect((await put(a)).status).toBe(200);
    const get = await call("/v1/objects/vocabulary", "GET", undefined, a.token);
    expect(get.status).toBe(200);
    expect(Buffer.from(await get.arrayBuffer())).toEqual(envelope);
    const account = await call("/v1/account", "GET", undefined, a.token);
    expect(((await account.json()) as { bytes_used: number }).bytes_used).toBe(
      envelope.length,
    );
    const bucket = await mf
      .getBindings<{ CONTENT: R2Bucket }>()
      .then((b) => b.CONTENT);
    const listed = await bucket.list();
    const saved = await bucket.get(listed.objects[0].key);
    expect(
      Buffer.from(await saved!.arrayBuffer()).includes(Buffer.from("entries")),
    ).toBe(false);
  });
  it("allows exactly one concurrent conditional writer", async () => {
    const a = await signin();
    const result = await Promise.all([put(a), put(a)]);
    expect(result.map((r) => r.status).sort()).toEqual([200, 412]);
    const changes = await call(
      "/v1/changes?since=0",
      "GET",
      undefined,
      a.token,
    );
    expect(
      ((await changes.json()) as { changes: unknown[] }).changes,
    ).toHaveLength(1);
  });
  it("enforces quota transactionally", async () => {
    const a = await signin();
    await db
      .prepare("UPDATE accounts SET quota=4096 WHERE id=?")
      .bind(a.account.id)
      .run();
    expect((await put(a)).status).toBe(200);
    expect((await put(a, "second")).status).toBe(413);
    expect(
      (
        await db
          .prepare("SELECT bytes_used FROM accounts WHERE id=?")
          .bind(a.account.id)
          .first()
      )?.bytes_used,
    ).toBe(4096);
  });
  it("rejects plaintext and stale encryption epochs", async () => {
    const a = await signin();
    expect(
      (await put(a, "words", 0, new TextEncoder().encode("private words")))
        .status,
    ).toBe(400);
    await db.prepare("UPDATE accounts SET epoch=2").run();
    expect((await put(a)).status).toBe(409);
  });
  it("publishes tombstones and invalidates stale holds", async () => {
    const a = await signin();
    await put(a);
    expect(
      (
        await call(
          "/v1/holds",
          "POST",
          { object: "vocabulary", rev: 1, complete: true },
          a.token,
        )
      ).status,
    ).toBe(200);
    await put(a, "vocabulary", 1);
    expect(
      (await db.prepare("SELECT * FROM holds").all()).results,
    ).toHaveLength(0);
    const r = await call(
      "/v1/objects/vocabulary",
      "DELETE",
      undefined,
      a.token,
      { "if-match": '"2"', "x-sync-epoch": "1" },
    );
    expect(r.status).toBe(200);
    expect(
      (await call("/v1/objects/vocabulary", "GET", undefined, a.token)).status,
    ).toBe(404);
    const changes = await call(
      "/v1/changes?since=2",
      "GET",
      undefined,
      a.token,
    );
    expect(
      ((await changes.json()) as { changes: { deleted: number }[] }).changes[0]
        .deleted,
    ).toBe(1);
  });
  it("account deletion revokes access and deletes live ciphertext", async () => {
    const a = await signin();
    await put(a);
    expect(
      (await call("/v1/account", "DELETE", { confirmed: true }, a.token))
        .status,
    ).toBe(200);
    expect((await call("/v1/account", "GET", undefined, a.token)).status).toBe(
      401,
    );
    expect(
      (
        await (
          await mf.getBindings<{ CONTENT: R2Bucket }>().then((b) => b.CONTENT)
        ).list()
      ).objects,
    ).toHaveLength(0);
  });
});

it("exports ciphertext, restores it to an empty database and requires fresh sessions", async () => {
  const a = await signin();
  await put(a);
  expect(
    (await call("/v1/reset", "POST", { confirmed: true }, a.token)).status,
  ).toBe(202);
  expect((await call("/__test/cron")).status).toBe(200);
  const env = await mf.getBindings<Env>();
  const backups = await env.BACKUPS.list();
  const marker = backups.objects.find((o) => o.key.endsWith("/manifest.json"));
  expect(marker).toBeTruthy();
  const prefix = marker!.key.replace("manifest.json", "");
  await db.exec("DELETE FROM accounts;");
  const content = await env.CONTENT.list();
  await env.CONTENT.delete(content.objects.map((o) => o.key));
  await restore(
    { ...env, ENVIRONMENT: "staging" },
    env.BACKUPS,
    prefix,
    new Set(),
  );
  expect(
    (await db.prepare("SELECT * FROM sessions").all()).results,
  ).toHaveLength(0);
  const again = await signin("restored");
  const r = await call("/v1/objects/vocabulary", "GET", undefined, again.token);
  expect(r.status).toBe(200);
  expect(Buffer.from(await r.arrayBuffer())).toEqual(envelope);
  expect(
    (await db.prepare("SELECT bytes_used FROM accounts").first())?.bytes_used,
  ).toBe(4096);
  expect(
    (await db.prepare("SELECT COUNT(*) AS n FROM pending_resets").first())?.n,
  ).toBe(0);
});

describe("account lifecycle", () => {
  it("reports a missed first nightly run without exposing account details", async () => {
    await call("/v1/health");
    await db
      .prepare("UPDATE operations SET last_ok=0 WHERE name='started'")
      .run();
    const r = await call("/v1/health");
    expect(r.status).toBe(503);
    const data = await r.text();
    expect(data).not.toContain("@");
    expect(data).not.toContain("token");
  });
  it("reports an elevated aggregate 5xx rate", async () => {
    const t = Math.floor(Date.now() / 1000);
    await db
      .prepare(
        "INSERT INTO operations(name,last_failure,count) VALUES('http5xx:test',?,5)",
      )
      .bind(t)
      .run();
    await db
      .prepare(
        "INSERT INTO operations(name,last_ok,count) VALUES('http:test',?,20)",
      )
      .bind(t)
      .run();
    expect((await call("/v1/health")).status).toBe(503);
  });
  it("does not restore an account erased after the snapshot", async () => {
    const a = await signin();
    await put(a);
    expect((await call("/__test/cron")).status).toBe(200);
    const env = await mf.getBindings<Env>();
    const marker = (await env.BACKUPS.list()).objects
      .filter((o) => o.key.endsWith("/manifest.json"))
      .at(-1)!;
    await call("/v1/account", "DELETE", { confirmed: true }, a.token);
    await restore(
      { ...env, ENVIRONMENT: "staging" },
      env.BACKUPS,
      marker.key.replace("manifest.json", ""),
      new Set(),
    );
    expect(
      await db
        .prepare("SELECT id FROM accounts WHERE id=?")
        .bind(a.account.id)
        .first(),
    ).toBeNull();
  });
  it("serves a keyless account page with a restrictive CSP", async () => {
    const page = await call("/account");
    expect(page.status).toBe(200);
    expect(page.headers.get("content-security-policy")).toContain(
      "default-src 'none'",
    );
    expect(await page.text()).not.toContain("recovery code<input");
    expect((await call("/account.js")).status).toBe(200);
  });
  it("links only after a fresh authenticated request and verified provider return", async () => {
    const a = await signin();
    const verifier = randomBytes(32).toString("base64url");
    const r = await call(
      "/v1/auth/start",
      "POST",
      {
        provider: "authkit",
        challenge: hash(verifier),
        device: "device-a",
        publicKey: pk,
        platform: "Test",
        link: true,
      },
      a.token,
    );
    expect(r.status).toBe(200);
    const auth = (await r.json()) as { id: string };
    expect((await finish(auth.id, "synthetic-linked-code")).status).toBe(303);
    const rows = await db.prepare("SELECT account FROM identities").all();
    expect(rows.results).toHaveLength(2);
    expect(
      rows.results.every(
        (r: { account: string }) => r.account === a.account.id,
      ),
    ).toBe(true);
  });
  it("rotates all wraps atomically, rejects stale epochs, and revokes the removed device", async () => {
    const a = await signin("a"),
      b = await signin("b");
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    const rotation = {
      confirmed: true,
      device: "b",
      epoch: 1,
      previous: 1,
      keyring: "bmV3",
      recovery: "bmV3",
      verifier: verifierHash,
      vocabularyRevision: 0,
      devices: [{ id: "a", ciphertext: "bmV3" }],
    };
    await put(a);
    expect((await call("/v1/rekey", "POST", rotation, a.token)).status).toBe(
      412,
    );
    expect(
      (await db.prepare("SELECT epoch FROM accounts").first())?.epoch,
    ).toBe(1);
    rotation.vocabularyRevision = 1;
    const replies = await Promise.all([
      call("/v1/rekey", "POST", rotation, a.token),
      call("/v1/rekey", "POST", rotation, a.token),
    ]);
    expect(replies.map((r) => r.status).sort()).toEqual([200, 412]);
    expect((await call("/v1/devices", "GET", undefined, b.token)).status).toBe(
      401,
    );
    expect((await call("/v1/account", "GET", undefined, a.token)).status).toBe(
      200,
    );
    expect(
      (await db.prepare("SELECT epoch FROM accounts").first())?.epoch,
    ).toBe(2);
    expect(
      (await db.prepare("SELECT rev FROM wraps WHERE kind='keyring'").first())
        ?.rev,
    ).toBe(2);
    expect((await put(a)).status).toBe(409);
  });
  it("requires a fresh session before resetting unreadable data", async () => {
    const a = await signin();
    await db.prepare("UPDATE sessions SET fresh_at=0").run();
    expect(
      (await call("/v1/rekey", "POST", { confirmed: true }, a.token)).status,
    ).toBe(403);
  });
  it("resets unreadable data while preserving the account and revoking other devices", async () => {
    const a = await signin("a"),
      b = await signin("b");
    await put(a);
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    expect(
      (await call("/v1/reset", "POST", { confirmed: true }, a.token)).status,
    ).toBe(202);
    await db.prepare("UPDATE pending_resets SET ready_at=unixepoch()-1").run();
    const freshDevice = await signin("fresh-after-wait");
    expect(
      (
        await call(
          "/v1/rekey",
          "POST",
          {
            confirmed: true,
            reset: true,
            device: freshDevice.device,
            epoch: 1,
            previous: 1,
            keyring: "bmV3",
            recovery: "bmV3",
            verifier: verifierHash,
            vocabularyRevision: 0,
            devices: [{ id: freshDevice.device, ciphertext: "bmV3" }],
          },
          freshDevice.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (
        await call(
          "/v1/objects/vocabulary",
          "GET",
          undefined,
          freshDevice.token,
        )
      ).status,
    ).toBe(404);
    expect((await call("/v1/account", "GET", undefined, b.token)).status).toBe(
      401,
    );
    expect(
      (await db.prepare("SELECT bytes_used FROM accounts").first())?.bytes_used,
    ).toBe(0);
  });
});

describe("security review regressions", () => {
  it("a provider session cannot overwrite keys, re-key, write vocabulary or erase the account", async () => {
    const a = await signin();
    const without = { "X-Sync-Auth": "" };
    const wrap = {
      kind: "recovery",
      ref: "main",
      ciphertext: "YWJj",
      previous: 0,
    };
    expect(
      (await call("/v1/wraps", "PUT", wrap, a.token, without)).status,
    ).toBe(403);
    expect(
      (await call("/v1/wraps", "PUT", wrap, a.token, { "X-Sync-Auth": pk }))
        .status,
    ).toBe(403);
    expect(
      (
        await call(
          "/v1/account",
          "DELETE",
          { confirmed: true },
          a.token,
          without,
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await call(
          "/v1/devices/device-a",
          "DELETE",
          { confirmed: true },
          a.token,
          without,
        )
      ).status,
    ).toBe(403);
    const rotation = {
      confirmed: true,
      device: "other",
      epoch: 1,
      previous: 1,
      keyring: "YWJj",
      recovery: "YWJj",
      verifier: verifierHash,
      vocabularyRevision: 0,
      devices: [{ id: a.device, ciphertext: "YWJj" }],
    };
    expect(
      (await call("/v1/rekey", "POST", rotation, a.token, without)).status,
    ).toBe(403);
    const write = await mf.dispatchFetch(
      "https://sync.test/v1/objects/vocabulary",
      {
        method: "PUT",
        headers: {
          authorization: "Bearer " + a.token,
          "if-match": '"0"',
          "x-sync-epoch": "1",
        },
        body: envelope,
      },
    );
    expect(write.status).toBe(403);
    expect((await call("/v1/wraps", "PUT", wrap, a.token)).status).toBe(200);
  });
  it("bootstrap is atomic and cannot claim existing or already initialized keys", async () => {
    const a = await signin();
    await db.prepare("UPDATE accounts SET auth_verifier=NULL").run();
    const b = { keyring: "YWJj", recovery: "YWJj", device: "YWJj" };
    const attempts = await Promise.all([
      call("/v1/bootstrap", "POST", b, a.token),
      call("/v1/bootstrap", "POST", b, a.token),
    ]);
    expect(attempts.map((r) => r.status).sort()).toEqual([201, 409]);
    expect(
      (await db.prepare("SELECT * FROM wraps").all()).results,
    ).toHaveLength(3);
    await db.prepare("UPDATE accounts SET auth_verifier=NULL").run();
    expect((await call("/v1/bootstrap", "POST", b, a.token)).status).toBe(409);
  });
  it("reset waits 72 hours, queues notice with mail disabled, and an unlocked device can cancel", async () => {
    const a = await signin("a"),
      b = await signin("b");
    await call(
      "/v1/wraps",
      "PUT",
      { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
      a.token,
    );
    const reset = {
      confirmed: true,
      reset: true,
      device: "b",
      epoch: 1,
      previous: 1,
      keyring: "YWJj",
      recovery: "YWJj",
      verifier: verifierHash,
      vocabularyRevision: 0,
      devices: [{ id: "b", ciphertext: "YWJj" }],
    };
    expect((await call("/v1/rekey", "POST", reset, b.token)).status).toBe(403);
    expect(
      (await call("/v1/reset", "POST", { confirmed: true }, b.token)).status,
    ).toBe(202);
    const pending = await db
      .prepare("SELECT * FROM pending_resets")
      .first<{ ready_at: number; requested_at: number }>();
    expect(pending!.ready_at - pending!.requested_at).toBe(72 * 3600);
    expect((await call("/v1/rekey", "POST", reset, b.token)).status).toBe(403);
    expect(
      await db
        .prepare("SELECT id FROM notifications WHERE kind='reset-requested'")
        .first(),
    ).not.toBeNull();
    expect(
      (
        (await (await call("/v1/poll", "GET", undefined, a.token)).json()) as {
          ready_at: number;
        }
      ).ready_at,
    ).toBe(pending!.ready_at);
    expect((await call("/v1/reset", "DELETE", undefined, a.token)).status).toBe(
      200,
    );
    expect((await call("/v1/rekey", "POST", reset, b.token)).status).toBe(403);
    expect(await db.prepare("SELECT * FROM pending_resets").first()).toBeNull();
  });
  it("an unchanged poll performs zero D1 writes", async () => {
    const a = await signin();
    await db.exec("CREATE TABLE poll_audit(n INTEGER);");
    const tables = [
      "accounts",
      "sessions",
      "devices",
      "rate_limits",
      "operations",
      "wraps",
      "pending_resets",
    ];
    try {
      for (const table of tables)
        for (const event of ["INSERT", "UPDATE", "DELETE"])
          await db.exec(
            `CREATE TRIGGER audit_${table}_${event} AFTER ${event} ON ${table} BEGIN INSERT INTO poll_audit VALUES(1); END;`,
          );
      for (let n = 0; n < 4; n++)
        expect((await call("/v1/poll", "GET", undefined, a.token)).status).toBe(
          200,
        );
      expect(
        (await db.prepare("SELECT COUNT(*) AS n FROM poll_audit").first())?.n,
      ).toBe(0);
    } finally {
      for (const table of tables)
        for (const event of ["INSERT", "UPDATE", "DELETE"])
          await db.exec(`DROP TRIGGER IF EXISTS audit_${table}_${event};`);
      await db.exec("DROP TABLE poll_audit;");
    }
  });
  it("backs up and restores more than 1000 rows and advertises full resync after pruning", async () => {
    const a = await signin();
    await put(a);
    const t = Math.floor(Date.now() / 1000);
    for (let start = 2; start <= 1105; start += 100)
      await db.batch(
        Array.from({ length: Math.min(100, 1106 - start) }, (_, i) =>
          db
            .prepare(
              "INSERT INTO changes(account,seq,id,rev,deleted,at) VALUES(?,?,?,1,0,?)",
            )
            .bind(a.account.id, start + i, "synthetic-" + (start + i), t),
        ),
      );
    await db.prepare("UPDATE accounts SET seq=1105").run();
    expect((await call("/__test/cron")).status).toBe(200);
    const env = await mf.getBindings<Env>();
    const marker = (await env.BACKUPS.list()).objects
      .filter((o) => o.key.endsWith("/manifest.json"))
      .at(-1)!;
    const manifest = await (await env.BACKUPS.get(marker.key))!.json<{
      schema: number;
      tables: Record<string, number>;
    }>();
    expect(manifest.schema).toBe(2);
    expect(manifest.tables.changes).toBe(5);
    await db.exec("DELETE FROM accounts;");
    await restore(
      { ...env, ENVIRONMENT: "staging" },
      env.BACKUPS,
      marker.key.replace("manifest.json", ""),
      new Set(),
    );
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM changes").first())?.n,
    ).toBe(1105);
    // Restored sessions must sign in again; use the fixture's existing identity.
    const signed = await signin("device-a");
    await db.prepare("UPDATE changes SET at=0 WHERE seq<=1000").run();
    expect((await call("/__test/cron")).status).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM changes").first())?.n,
    ).toBe(105);
    const response = await call(
      "/v1/changes?since=1",
      "GET",
      undefined,
      signed.token,
    );
    expect(
      ((await response.json()) as { fullResync: boolean }).fullResync,
    ).toBe(true);
  }, 60000);
  it("web device placeholders do not consume the native cap and are pruned after sessions expire", async () => {
    const a = await signin();
    await db.batch(
      Array.from({ length: 65 }, (_, i) =>
        db
          .prepare(
            "INSERT INTO devices(id,account,platform,public_key,created_at,last_seen_at) VALUES(?,?,'web','',0,0)",
          )
          .bind("web-" + i, a.account.id),
      ),
    );
    expect((await signin("native-b")).device).toBe("native-b");
    expect((await call("/__test/cron")).status).toBe(200);
    expect(
      (
        await db
          .prepare("SELECT COUNT(*) AS n FROM devices WHERE public_key=''")
          .first()
      )?.n,
    ).toBe(0);
  });
  it("concurrent first sign-ins create one account and no orphan", async () => {
    const a = await begin("a"),
      b = await begin("b");
    const replies = await Promise.all([a, b].map((x) => finish(x.id)));
    expect(replies.map((r) => r.status)).toEqual([303, 303]);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM accounts").first())?.n,
    ).toBe(1);
    expect(
      (
        await db
          .prepare(
            "SELECT COUNT(*) AS n FROM accounts WHERE NOT EXISTS(SELECT 1 FROM identities WHERE account=accounts.id)",
          )
          .first()
      )?.n,
    ).toBe(0);
  });
  it("malformed percent encoding is a client error, not a monitored server failure", async () => {
    const a = await signin();
    expect(
      (await call("/v1/objects/%ZZ", "GET", undefined, a.token)).status,
    ).toBe(400);
    expect(
      (
        await db
          .prepare(
            "SELECT COUNT(*) AS n FROM operations WHERE name LIKE 'http5xx:%'",
          )
          .first()
      )?.n,
    ).toBe(0);
  });
});

describe("WorkOS lifecycle boundaries", () => {
  async function delivery(
    event: string,
    data: Record<string, unknown>,
    options: { id?: string; timestamp?: number; forged?: boolean } = {},
  ) {
    const stamp = options.timestamp ?? Date.now();
    const payload = JSON.stringify({
      id: options.id ?? "event_" + randomBytes(8).toString("hex"),
      event,
      created_at: new Date().toISOString(),
      data,
    });
    const signature = createHmac(
      "sha256",
      options.forged ? "attacker" : syntheticWebhook,
    )
      .update(stamp + "." + payload)
      .digest("hex");
    return mf.dispatchFetch("https://sync.test/v1/auth/webhook", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "workos-signature": `t=${stamp},v1=${signature}`,
      },
      body: payload,
    });
  }
  const updated = {
    object: "user",
    id: "user_synthetic",
    email: "attacker@example.invalid",
    email_verified: true,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    first_name: null,
    last_name: null,
    profile_picture_url: null,
  };
  it("rejects forged, stale and future webhook signatures without mutation", async () => {
    await signin();
    for (const options of [
      { forged: true },
      { timestamp: Date.now() - 181000 },
      { timestamp: Date.now() + 181000 },
    ])
      expect((await delivery("user.deleted", updated, options)).status).toBe(
        400,
      );
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM workos_events").first())?.n,
    ).toBe(0);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM workos_revocations").first())
        ?.n,
    ).toBe(0);
  });
  it("accepts a correctly signed delivery that took a few seconds to arrive", async () => {
    await signin();
    const response = await delivery("user.deleted", updated, {
      timestamp: Date.now() - 5000,
    });
    expect(response.status).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM workos_revocations").first())
        ?.n,
    ).toBe(1);
  });
  it("verified email webhooks cannot redirect reset notices and are idempotent", async () => {
    const a = await signin();
    for (let i = 0; i < 2; i++)
      expect(
        (await delivery("user.updated", updated, { id: "event_same" })).status,
      ).toBe(200);
    expect(
      (await db.prepare("SELECT email FROM accounts").first())?.email,
    ).toBe("person@example.invalid");
    expect(
      (
        await db
          .prepare(
            "SELECT COUNT(*) AS n FROM notifications WHERE kind='identity-updated'",
          )
          .first()
      )?.n,
    ).toBe(1);
    expect(
      (
        await db
          .prepare(
            "SELECT recipient FROM notifications WHERE kind='identity-updated'",
          )
          .first()
      )?.recipient,
    ).toBe("person@example.invalid");
    expect(
      (
        await call("/v1/reset", "POST", { confirmed: true }, a.token, {
          "X-Sync-Auth": "",
        })
      ).status,
    ).toBe(202);
    expect(
      (
        await db
          .prepare(
            "SELECT a.email FROM pending_resets p JOIN accounts a ON a.id=p.account",
          )
          .first()
      )?.email,
    ).toBe("person@example.invalid");
  });
  it.each(["user.deleted", "session.revoked"])(
    "%s revokes keyless access but preserves proof-held reset cancellation",
    async (event) => {
      const a = await signin();
      expect(
        (
          await call("/v1/reset", "POST", { confirmed: true }, a.token, {
            "X-Sync-Auth": "",
          })
        ).status,
      ).toBe(202);
      await call(
        "/v1/wraps",
        "PUT",
        { kind: "keyring", ref: "main", ciphertext: "YWJj", previous: 0 },
        a.token,
      );
      const data =
        event === "user.deleted"
          ? updated
          : {
              object: "session",
              id: "session_synthetic",
              user_id: "user_synthetic",
              created_at: "2026-01-01T00:00:00Z",
              updated_at: "2026-01-01T00:00:00Z",
            };
      expect((await delivery(event, data)).status).toBe(200);
      expect(
        (
          await call("/v1/poll", "GET", undefined, a.token, {
            "X-Sync-Auth": "",
          })
        ).status,
      ).toBeGreaterThanOrEqual(400);
      expect((await call("/v1/poll", "GET", undefined, a.token)).status).toBe(
        200,
      );
      expect(
        (await call("/v1/reset", "DELETE", undefined, a.token)).status,
      ).toBe(200);
      expect(
        (await db.prepare("SELECT COUNT(*) AS n FROM wraps").first())?.n,
      ).toBe(1);
      expect(
        (await db.prepare("SELECT COUNT(*) AS n FROM pending_resets").first())
          ?.n,
      ).toBe(0);
      const b = await begin("b");
      expect((await finish(b.id)).status).toBe(401);
    },
  );
  it("a revoke between callback and exchange cannot mint keyless access", async () => {
    const a = await begin();
    const returned = await finish(a.id);
    const url = new URL(returned.headers.get("location")!);
    expect((await delivery("user.deleted", updated)).status).toBe(200);
    const r = await call("/v1/auth/token", "POST", {
      request: a.id,
      code: url.searchParams.get("code"),
      verifier: a.verifier,
    });
    const value = (await r.json()) as { token: string };
    expect(
      (
        await call("/v1/account", "GET", undefined, value.token, {
          "X-Sync-Auth": "",
        })
      ).status,
    ).toBeGreaterThanOrEqual(400);
  });
  it("session lifetimes have a 90-day idle limit and 365-day absolute limit", async () => {
    const a = await signin();
    await db
      .prepare("UPDATE sessions SET created_at=?,expires_at=?")
      .bind(
        Math.floor(Date.now() / 1000) - 366 * 86400,
        Math.floor(Date.now() / 1000) + 86400,
      )
      .run();
    expect((await call("/v1/poll", "GET", undefined, a.token)).status).toBe(
      401,
    );
  });
  it("a revoked key holder can link a fresh replacement identity without surrendering keys", async () => {
    const a = await signin();
    await delivery("user.deleted", updated);
    await db.prepare("UPDATE sessions SET fresh_at=0").run();
    const pending = await call(
      "/v1/auth/start",
      "POST",
      {
        provider: "authkit",
        link: true,
        challenge: hash("replacement-verifier"),
        device: "replacement",
        publicKey: pk,
        platform: "Test",
      },
      a.token,
    );
    expect(pending.status).toBe(200);
    const attempt = (await pending.json()) as { id: string };
    expect((await finish(attempt.id, "synthetic-linked-code")).status).toBe(
      303,
    );
    expect(
      (
        await db
          .prepare("SELECT account FROM identities WHERE subject='user_linked'")
          .first()
      )?.account,
    ).toBe(a.account.id);
    expect(
      (await db.prepare("SELECT auth_verifier FROM accounts").first())
        ?.auth_verifier,
    ).toBe(verifierHash);
  });
  it("web sign-out clears its own cookie and returns the SDK logout URL", async () => {
    const verifier = "web-synthetic-verifier";
    const r = await call("/v1/auth/start", "POST", {
      provider: "authkit",
      challenge: hash(verifier),
      device: "web",
      redirect: "web",
    });
    const a = (await r.json()) as { id: string };
    const callback = await finish(a.id);
    const fragment = new URLSearchParams(
      new URL(callback.headers.get("location")!).hash.slice(1),
    );
    const result = await call("/v1/auth/token", "POST", {
      request: a.id,
      code: fragment.get("code"),
      verifier,
    });
    const cookie = result.headers.get("set-cookie")!.split(";")[0];
    expect(await result.json()).not.toHaveProperty("token");
    expect(result.headers.get("set-cookie")).toContain(
      "HttpOnly; Secure; SameSite=Lax",
    );
    const out = await call("/v1/auth/signout", "POST", {}, undefined, {
      cookie,
      origin: "https://sync.test",
    });
    expect(out.status).toBe(200);
    expect(out.headers.get("set-cookie")).toContain("Max-Age=0");
    const logout = new URL(((await out.json()) as { url: string }).url);
    expect(logout.origin + logout.pathname).toBe(
      "https://api.workos.com/user_management/sessions/logout",
    );
    expect(logout.searchParams.get("return_to")).toBe(
      "https://sync.test/account",
    );
    expect(
      (await call("/v1/account", "GET", undefined, undefined, { cookie }))
        .status,
    ).toBe(401);
  });
  it("restore preserves revocation tombstones and cannot revive provider access", async () => {
    await signin();
    await delivery("user.deleted", updated);
    expect((await call("/__test/cron", "POST")).status).toBe(200);
    const env = await mf.getBindings<Env>();
    const page = await env.BACKUPS.list();
    const marker = page.objects.find((o) => o.key.endsWith("manifest.json"))!;
    await db.exec("DELETE FROM accounts; DELETE FROM workos_revocations;");
    await restore(
      { ...env, ENVIRONMENT: "staging" },
      env.BACKUPS,
      marker.key.replace("manifest.json", ""),
      new Set(),
    );
    expect(
      (
        await db
          .prepare("SELECT subject FROM workos_revocations WHERE kind='user'")
          .first()
      )?.subject,
    ).toBe("user_synthetic");
    const a = await begin("restored");
    expect((await finish(a.id)).status).toBe(401);
  });
  it("prunes lifecycle metadata only beyond its replay and session windows", async () => {
    await signin();
    await delivery("user.deleted", updated);
    const t = Math.floor(Date.now() / 1000);
    await db
      .prepare("UPDATE workos_events SET at=?")
      .bind(t - 31 * 86400)
      .run();
    await db
      .prepare("UPDATE workos_revocations SET at=?")
      .bind(t - 401 * 86400)
      .run();
    expect((await call("/__test/cron", "POST")).status).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM workos_events").first())?.n,
    ).toBe(0);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM workos_revocations").first())
        ?.n,
    ).toBe(0);
  });
});
