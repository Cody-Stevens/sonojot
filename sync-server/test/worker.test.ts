import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { restore } from "../src/restore";
import { store } from "../src/store";
import { rekeyPages } from "../src/rekey_pages";
import { meetings } from "../src/meetings";
import { backup, maintenance } from "../src/maintenance";
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
        ENVIRONMENT: "staging",
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
  await db.exec(readFileSync("migrations/0008_email_signin.sql", "utf8"));
  await db.exec(readFileSync("migrations/0009_meetings.sql", "utf8"));
  await db.exec(readFileSync("migrations/0010_meeting_lifecycle.sql", "utf8"));
  await db.exec(readFileSync("migrations/0011_meeting_restore.sql", "utf8"));
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
    "DELETE FROM restore_accounts; DELETE FROM restore_run; DELETE FROM trash; DELETE FROM operations; DELETE FROM pairings; DELETE FROM sessions; DELETE FROM wraps; DELETE FROM devices; DELETE FROM identities; DELETE FROM accounts; DELETE FROM auth_requests; DELETE FROM rate_limits; DELETE FROM workos_events; DELETE FROM workos_revocations;",
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
    expect(r.headers.get("cache-control")).toBe("no-store, no-transform");
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
    expect((await call("/v1/auth/email", "POST", {})).status).toBe(403);
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
    expect(b.location).toBe("https://sync.test/sign-in?request=" + a.id);
    const row = await db
      .prepare("SELECT browser_hash FROM auth_requests WHERE id=?")
      .bind(a.id)
      .first<{ browser_hash: string }>();
    expect(row?.browser_hash).toBe(hash(b.cookie.split(".")[1]));
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
  JSON.parse(
    readFileSync(
      "../rewrite/src-tauri/tests/fixtures/sync-vectors.json",
      "utf8",
    ),
  ).envelopeHex,
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
  const original = await mf.getBindings<Env>();
  const checkpointKey = a.account.id + "/synthetic-checkpoint";
  await original.CONTENT.put(checkpointKey, envelope);
  await db
    .prepare("INSERT INTO rekey_pages VALUES(?,2,'synthetic',0,?,4096,0)")
    .bind(a.account.id, checkpointKey)
    .run();
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
    Buffer.from(await (await env.CONTENT.get(checkpointKey))!.arrayBuffer()),
  ).toEqual(envelope);
  expect(
    (await db.prepare("SELECT expires_at FROM rekey_pages").first())
      ?.expires_at,
  ).toBe(0);
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
      (
        await call(
          "/v1/rekey",
          "POST",
          { confirmed: true, reset: true },
          a.token,
        )
      ).status,
    ).toBe(403);
  });
  it("resets unreadable data while preserving the account and revoking other devices", async () => {
    const a = await signin("a"),
      b = await signin("b");
    await put(a);
    expect(
      (
        await call(
          "/v1/meetings/reset-meeting",
          "POST",
          { createdAt: 1 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect((await put(a, "m/reset-meeting/raw/base")).status).toBe(200);
    expect((await put(a, "m/reset-meeting/head")).status).toBe(200);

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
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM meetings").first())?.n,
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
  it("web sign-out clears its own cookie without a third-party page", async () => {
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
    expect(((await out.json()) as { url: null }).url).toBeNull();
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

describe("branded email sign-in", () => {
  async function attempt(intent = "sign-up", device = "email-device") {
    const verifier = randomBytes(32).toString("base64url");
    const started = await call("/v1/auth/start", "POST", {
      intent,
      challenge: hash(verifier),
      device,
      publicKey: pk,
      platform: "Test",
    });
    const a = (await started.json()) as { id: string; url: string };
    const b = await browser(a.id);
    return { ...a, ...b, verifier };
  }
  async function email(
    a: { id: string; cookie: string },
    address = "person@example.invalid",
    extra: Record<string, string> = {},
  ) {
    return call(
      "/v1/auth/email",
      "POST",
      { request: a.id, email: address },
      undefined,
      { origin: "https://sync.test", cookie: a.cookie, ...extra },
    );
  }
  async function verify(
    a: { id: string; cookie: string },
    code = "123456",
    extra: Record<string, string> = {},
  ) {
    return call("/v1/auth/verify", "POST", { request: a.id, code }, undefined, {
      origin: "https://sync.test",
      cookie: a.cookie,
      ...extra,
    });
  }
  async function redeem(a: { id: string; cookie: string; verifier: string }) {
    const r = await verify(a);
    expect(r.status).toBe(200);
    const u = new URL(((await r.json()) as { url: string }).url);
    return call("/v1/auth/token", "POST", {
      request: a.id,
      code: u.searchParams.get("code"),
      verifier: a.verifier,
    });
  }
  it("creates a verified account and preserves the immutable id of an existing hosted account", async () => {
    const old = await signin();
    const a = await attempt("sign-in");
    expect((await email(a)).status).toBe(200);
    const t = await redeem(a);
    expect(t.status).toBe(200);
    expect(((await t.json()) as { account: { id: string } }).account.id).toBe(
      old.account.id,
    );
    const b = await attempt("sign-up", "new-device");
    expect((await email(b, "new@example.com")).status).toBe(200);
    expect((await redeem(b)).status).toBe(200);
    expect((await verify(b)).status).toBe(401);
  });
  it("requires email ownership before distinguishing a missing account", async () => {
    const a = await attempt("sign-in");
    expect((await email(a)).status).toBe(200);
    expect((await verify(a, "000000")).status).toBe(401);
    expect((await verify(a)).status).toBe(409);
  });
  it("gives identical responses for new, existing, rejected and email-limited addresses", async () => {
    await signin();
    const responses: unknown[] = [];
    for (const address of [
      "person@example.invalid",
      "new@example.com",
      "reject@example.com",
    ]) {
      responses.push(await (await email(await attempt(), address)).json());
    }
    await db
      .prepare("INSERT INTO rate_limits(key,count,expires_at) VALUES(?,5,?)")
      .bind(
        "email-address:" + hash("limited@example.com"),
        Math.floor(Date.now() / 1000) + 3600,
      )
      .run();
    responses.push(
      await (await email(await attempt(), "limited@example.com")).json(),
    );
    expect(
      responses.every(
        (r) => JSON.stringify(r) === JSON.stringify(responses[0]),
      ),
    ).toBe(true);
  });
  it("limits guesses, binds the browser and requires the exact origin and JSON", async () => {
    const a = await attempt();
    expect(
      (await email(a, "person@example.invalid", { origin: "" })).status,
    ).toBe(403);
    expect(
      (
        await email(a, "person@example.invalid", {
          origin: "https://evil.example.com",
        })
      ).status,
    ).toBe(403);
    expect(
      (
        await email(a, "person@example.invalid", {
          "content-type": "text/plain",
        })
      ).status,
    ).toBe(403);
    expect(
      (await email(a, "person@example.invalid", { cookie: "" })).status,
    ).toBe(401);
    expect((await email(a)).status).toBe(200);
    expect((await verify(a, "123456", { cookie: "" })).status).toBe(401);
    for (let n = 0; n < 5; n++)
      expect((await verify(a, "000000")).status).toBe(401);
    expect((await verify(a)).status).toBe(401);
    const row = await db
      .prepare(
        "SELECT code_tries,email,code_hash FROM auth_requests WHERE id=?",
      )
      .bind(a.id)
      .first<{ code_tries: number; code_hash: string }>();
    expect(row?.code_tries).toBe(5);
    expect(row?.code_hash).not.toBe("123456");
  });
  it("allows one concurrent send and one grant, and keeps the app PKCE check", async () => {
    const a = await attempt();
    expect(
      (await Promise.all([email(a), email(a)])).map((r) => r.status).sort(),
    ).toEqual([200, 409]);
    const results = await Promise.all([verify(a), verify(a)]);
    expect(results.map((r) => r.status).sort()).toEqual([200, 401]);
    const ok = results.find((r) => r.status === 200)!;
    const u = new URL(((await ok.json()) as { url: string }).url);
    expect(
      (
        await call("/v1/auth/token", "POST", {
          request: a.id,
          code: u.searchParams.get("code"),
          verifier: "wrong",
        })
      ).status,
    ).toBe(401);
    expect(
      (
        await call("/v1/auth/token", "POST", {
          request: a.id,
          code: u.searchParams.get("code"),
          verifier: a.verifier,
        })
      ).status,
    ).toBe(200);
  });
  it("expires attempts after five minutes and enforces the IP send limit", async () => {
    const a = await attempt();
    await db
      .prepare("UPDATE auth_requests SET expires_at=0 WHERE id=?")
      .bind(a.id)
      .run();
    expect((await email(a)).status).toBe(401);
    const b = await attempt();
    await db
      .prepare("INSERT INTO rate_limits(key,count,expires_at) VALUES(?,10,?)")
      .bind(
        "email-ip:" + hash("203.0.113.10"),
        Math.floor(Date.now() / 1000) + 3600,
      )
      .run();
    expect(
      (
        await email(b, "person@example.invalid", {
          "cf-connecting-ip": "203.0.113.10",
        })
      ).status,
    ).toBe(429);
  });
  it("serves both paper entry points without identity-provider names or third-party scripts", async () => {
    for (const path of ["/sign-up", "/sign-in", "/signin.js", "/signin.css"]) {
      const r = await call(path);
      expect(r.status).toBe(200);
      expect(r.headers.get("content-security-policy")).toContain(
        "script-src 'self'",
      );
      expect(await r.text()).not.toMatch(/workos|authkit/i);
    }
    expect(
      (await mf.dispatchFetch("https://wrong.example.com/sign-in")).status,
    ).toBe(400);
    expect(
      (
        await call("/v1/auth/start", "POST", {
          redirect: "http://localhost:8000/sync",
          challenge: pk,
          device: "x",
          publicKey: pk,
          platform: "Test",
        })
      ).status,
    ).toBe(400);
  });
});

describe("meeting layers and storage", () => {
  async function register(a: { token: string }, name: string, created: number) {
    expect(
      (
        await call(
          `/v1/meetings/${name}`,
          "POST",
          { createdAt: created * 86400 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect((await put(a, `m/${name}/raw/base`)).status).toBe(200);
    expect((await put(a, `m/${name}/head`)).status).toBe(200);
  }
  async function hold(a: { token: string }, name: string) {
    const inventory = (await (
      await call(`/v1/meetings/${name}`, "GET", undefined, a.token)
    ).json()) as { version: number };
    return call(
      "/v1/holds",
      "POST",
      {
        object: `m/${name}/head`,
        rev: 1,
        version: inventory.version,
        complete: true,
      },
      a.token,
    );
  }
  it("makes raw, clean, batches, snapshots and blobs immutable", async () => {
    const a = await signin();
    await register(a, "first", 1);
    for (const suffix of [
      "raw/base",
      "clean/one",
      "ops/base/one",
      "snap/base/one",
    ]) {
      if (suffix !== "raw/base")
        expect((await put(a, `m/first/${suffix}`)).status).toBe(200);
      expect((await put(a, `m/first/${suffix}`, 1)).status).toBe(412);
      expect(
        (
          await call(
            `/v1/objects/m/first/${suffix}`,
            "DELETE",
            undefined,
            a.token,
            { "if-match": '"1"', "x-sync-epoch": "1" },
          )
        ).status,
      ).toBe(400);
    }
    expect((await put(a, "m/first/head", 1)).status).toBe(200);
    const b = await signin("device-b");
    expect((await hold(b, "first")).status).toBe(412);
  });
  it("drops the oldest eligible meeting and keeps unheld or stale-held ones", async () => {
    const a = await signin();
    await register(a, "unheld", 1);
    await register(a, "old", 2);
    await register(a, "new", 3);
    expect((await hold(a, "old")).status).toBe(200);
    expect((await hold(a, "new")).status).toBe(200);
    await db.prepare("UPDATE accounts SET meeting_quota=24576").run();
    expect(
      (
        await call(
          "/v1/meetings/incoming",
          "POST",
          { createdAt: 4 * 86400 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect((await put(a, "m/incoming/raw/base")).status).toBe(200);
    const rows = (
      await db
        .prepare("SELECT id,state FROM meetings ORDER BY created_at")
        .all()
    ).results;
    expect(rows).toEqual([
      { id: "unheld", state: "active" },
      { id: "old", state: "dropped" },
      { id: "new", state: "active" },
      { id: "incoming", state: "active" },
    ]);
    expect((await put(a, "m/old/ops/base/late")).status).toBe(410);
    expect(
      (await call("/v1/meetings/old", "POST", { createdAt: 2 }, a.token))
        .status,
    ).toBe(410);
    const changes = (await (
      await call("/v1/changes?since=0", "GET", undefined, a.token)
    ).json()) as { changes: { id: string; deleted: number }[] };
    expect(
      changes.changes
        .filter((c) => c.deleted === 2)
        .map((c) => c.id)
        .sort(),
    ).toEqual(["m/old/head", "m/old/raw/base"]);
  });
  it("drops an incoming edit's own meeting when it is the oldest held copy", async () => {
    const a = await signin();
    await register(a, "old", 1);
    await register(a, "new", 2);
    expect((await hold(a, "old")).status).toBe(200);
    expect((await hold(a, "new")).status).toBe(200);
    await db.prepare("UPDATE accounts SET meeting_quota=16384").run();
    expect((await put(a, "m/old/ops/base/growth")).status).toBe(410);
    expect(
      (await db.prepare("SELECT state FROM meetings WHERE id='old'").first())
        ?.state,
    ).toBe("dropped");
    expect(
      (await db.prepare("SELECT state FROM meetings WHERE id='new'").first())
        ?.state,
    ).toBe("active");
  });
  it("requires a hold for every current layer and a signed-in retained device", async () => {
    const a = await signin();
    await register(a, "first", 1);
    expect((await hold(a, "first")).status).toBe(200);
    expect((await put(a, "m/first/ops/base/offline")).status).toBe(200);
    await db.prepare("UPDATE accounts SET meeting_quota=12288").run();
    expect(
      (await call("/v1/meetings/second", "POST", { createdAt: 2 }, a.token))
        .status,
    ).toBe(200);
    expect((await put(a, "m/second/head")).status).toBe(507);
    expect((await hold(a, "first")).status).toBe(200);
    await db.prepare("UPDATE devices SET revoked_at=1").run();
    const b = await signin("device-b");
    expect((await put(b, "m/second/head")).status).toBe(507);
  });
  it("deletes the entire meeting, retains trash and rejects offline resurrection", async () => {
    const a = await signin();
    await register(a, "first", 1);
    expect(
      (await call("/v1/meetings/first", "DELETE", { confirmed: true }, a.token))
        .status,
    ).toBe(400);
    expect((await put(a, "m/first/head", 1)).status).toBe(200);
    expect(
      (
        await call(
          "/v1/meetings/first",
          "DELETE",
          { confirmed: true, headRevision: 2 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT deleted,size FROM objects").all()).results,
    ).toEqual([
      { deleted: 1, size: 0 },
      { deleted: 0, size: 4096 },
    ]);
    expect(
      (await db.prepare("SELECT * FROM trash").all()).results,
    ).toHaveLength(2);
    expect((await put(a, "m/first/ops/base/late")).status).toBe(410);
    const head = await db
      .prepare("SELECT body_key FROM objects WHERE id='m/first/head'")
      .first<{ body_key: string }>();
    await db
      .prepare(
        "UPDATE meetings SET updated_at=unixepoch()-181*86400 WHERE id='first'",
      )
      .run();
    await call("/__test/cron");
    expect(
      await db
        .prepare("SELECT id FROM objects WHERE id='m/first/head'")
        .first(),
    ).toBeNull();
    expect(
      (await db.prepare("SELECT bytes_used FROM accounts").first())?.bytes_used,
    ).toBe(0);
    expect(
      await db
        .prepare("SELECT body_key FROM trash WHERE body_key=?")
        .bind(head!.body_key)
        .first(),
    ).not.toBeNull();
    await db.prepare("UPDATE trash SET purge_at=0").run();
    await call("/__test/cron");
    expect(
      await (await mf.getBindings<Env>()).CONTENT.head(head!.body_key),
    ).toBeNull();
  });
  it("retains folded batches for thirty days and keeps snapshots", async () => {
    const a = await signin();
    await register(a, "first", 1);
    expect((await put(a, "m/first/ops/base/batch")).status).toBe(200);
    expect((await put(a, "m/first/snap/base/snapshot")).status).toBe(200);
    expect(
      (
        await call(
          "/v1/meetings/first/compact",
          "POST",
          {
            snapshot: "m/first/snap/base/snapshot",
            folded: ["m/first/ops/base/batch"],
          },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT purge_at FROM compactions").first())!.purge_at,
    ).toBeGreaterThan(Date.now() / 1000 + 29 * 86400);
    await call("/__test/cron");
    expect(
      (
        await call(
          "/v1/objects/m/first/ops/base/batch",
          "GET",
          undefined,
          a.token,
        )
      ).status,
    ).toBe(200);
    await db.prepare("UPDATE compactions SET purge_at=0").run();
    await call("/__test/cron");
    expect(
      (
        await call(
          "/v1/objects/m/first/ops/base/batch",
          "GET",
          undefined,
          a.token,
        )
      ).status,
    ).toBe(404);
    expect(
      (
        await call(
          "/v1/objects/m/first/snap/base/snapshot",
          "GET",
          undefined,
          a.token,
        )
      ).status,
    ).toBe(200);
  });
  async function retain(
    a: { token: string },
    name: string,
    objects: string[],
    extra: Record<string, unknown> = {},
  ) {
    const inv = (await (
      await call(`/v1/meetings/${name}`, "GET", undefined, a.token)
    ).json()) as { version: number; objects: { id: string; rev: number }[] };
    const headRevision = inv.objects.find(
      (o) => o.id === `m/${name}/head`,
    )!.rev;
    return call(
      `/v1/meetings/${name}/references`,
      "POST",
      {
        token: randomBytes(16).toString("hex"),
        version: inv.version,
        headRevision,
        objects,
        complete: true,
        ...extra,
      },
      a.token,
    );
  }
  async function blob(a: { token: string }, name: string, chunk: string) {
    return mf.dispatchFetch(`https://sync.test/v1/blobs/${name}/${chunk}`, {
      method: "PUT",
      headers: {
        authorization: "Bearer " + a.token,
        "x-sync-auth": proof,
        "if-match": '\"0\"',
        "x-sync-epoch": "1",
      },
      body: envelope,
    });
  }
  it("M1 limits ordinary staging accounts and grants 100 GB only to test accounts", async () => {
    const a = await signin();
    await register(a, "first", 1);
    expect(
      (await db.prepare("SELECT meeting_quota FROM accounts").first())
        ?.meeting_quota,
    ).toBe(2_000_000_000);
    await db.prepare("UPDATE accounts SET is_test=1").run();
    await call("/v1/meetings/first", "POST", { createdAt: 86400 }, a.token);
    expect(
      (await db.prepare("SELECT meeting_quota FROM accounts").first())
        ?.meeting_quota,
    ).toBe(100_000_000_000);
    await db.prepare("UPDATE accounts SET is_test=0").run();
    await call("/v1/meetings/first", "POST", { createdAt: 86400 }, a.token);
    expect(
      (await db.prepare("SELECT meeting_quota FROM accounts").first())
        ?.meeting_quota,
    ).toBe(2_000_000_000);
  });
  it("M1 enforces a daily byte budget and rolls it over the next UTC day", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await db.prepare("UPDATE meeting_upload_bytes SET bytes=2000000000").run();
    expect((await put(a, "m/first/raw/blocked")).status).toBe(429);
    expect(
      await db
        .prepare("SELECT id FROM objects WHERE id='m/first/raw/blocked'")
        .first(),
    ).toBeNull();
    await db.prepare("UPDATE meeting_upload_bytes SET day=day-1").run();
    expect((await put(a, "m/first/raw/allowed")).status).toBe(200);
  });
  it("M1 reconciles a revoked allowance before direct object writes", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await db.prepare("UPDATE accounts SET meeting_quota=100000000000").run();
    expect((await put(a, "m/first/raw/direct")).status).toBe(200);
    expect(
      (await db.prepare("SELECT meeting_quota FROM accounts").first())
        ?.meeting_quota,
    ).toBe(2_000_000_000);
  });
  it("L4 rounds the meeting ordering timestamp to the UTC day", async () => {
    const a = await signin();
    expect(
      (
        await call(
          "/v1/meetings/day",
          "POST",
          { createdAt: 86400 + 12345 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (
        await db
          .prepare("SELECT created_at FROM meetings WHERE id='day'")
          .first()
      )?.created_at,
    ).toBe(86400);
  });
  it("M2 requires both a recent hold and a recently seen holding device", async () => {
    const a = await signin();
    await register(a, "old", 1);
    await hold(a, "old");
    const b = await signin("device-b");
    await call("/v1/meetings/new", "POST", { createdAt: 2 * 86400 }, b.token);
    await db.prepare("UPDATE accounts SET meeting_quota=8192").run();
    await db.prepare("UPDATE holds SET created_at=unixepoch()-2592001").run();
    expect((await put(b, "m/new/head")).status).toBe(507);
    await hold(a, "old");
    await db
      .prepare(
        "UPDATE devices SET last_seen_at=unixepoch()-2592001 WHERE id='device-a'",
      )
      .run();
    expect((await put(b, "m/new/head")).status).toBe(507);
    await hold(a, "old");
    expect((await put(b, "m/new/head")).status).toBe(200);
  });
  it("L1 never records missing or cross-meeting folded objects", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await put(a, "m/first/snap/base/snap");
    expect(
      (
        await call(
          "/v1/meetings/first/compact",
          "POST",
          {
            snapshot: "m/first/snap/base/snap",
            folded: ["m/first/ops/base/missing"],
          },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM compactions").first())?.n,
    ).toBe(0);
    expect(
      (
        await call(
          "/v1/meetings/first/compact",
          "POST",
          {
            snapshot: "m/first/snap/base/snap",
            folded: ["m/other/ops/base/batch"],
          },
          a.token,
        )
      ).status,
    ).toBe(400);
  });
  it("retains unreferenced bases for 30 days, cancels retirement on reuse, and rejects stale snapshots", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await put(a, "m/first/raw/old");
    const refs = ["m/first/head", "m/first/raw/base"];
    expect((await retain(a, "first", refs)).status).toBe(200);
    expect(
      (await db
        .prepare(
          "SELECT purge_at FROM meeting_gc WHERE object='m/first/raw/old'",
        )
        .first())!.purge_at,
    ).toBeGreaterThan(Date.now() / 1000 + 29 * 86400);
    expect(
      (await retain(a, "first", [...refs, "m/first/raw/old"])).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM meeting_gc").first())?.n,
    ).toBe(0);
    expect((await retain(a, "first", refs)).status).toBe(200);
    await db.prepare("UPDATE meeting_gc SET purge_at=0").run();
    await put(a, "m/first/ops/base/late");
    await call("/__test/cron");
    expect(
      (await call("/v1/objects/m/first/raw/old", "GET", undefined, a.token))
        .status,
    ).toBe(200);
    expect((await retain(a, "first", refs, { version: 1 })).status).toBe(412);
    expect(
      (await retain(a, "first", [...refs, "m/first/ops/base/late"])).status,
    ).toBe(200);
    await call("/__test/cron");
    expect(
      (await call("/v1/objects/m/first/raw/old", "GET", undefined, a.token))
        .status,
    ).toBe(404);
    expect(
      (await call("/v1/objects/m/first/raw/base", "GET", undefined, a.token))
        .status,
    ).toBe(200);
  });
  it("partial reference pages cannot collect anything, including after a prior successful snapshot", async () => {
    const a = await signin();
    await register(a, "first", 1);
    const refs = ["m/first/head", "m/first/raw/base"];
    expect((await retain(a, "first", refs)).status).toBe(200);
    expect(
      (await retain(a, "first", ["m/first/head", "m/first/raw/missing"]))
        .status,
    ).toBe(412);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM meeting_gc").first())?.n,
    ).toBe(0);
    expect(
      (await retain(a, "first", ["m/first/head"], { complete: false })).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM meeting_gc").first())?.n,
    ).toBe(0);
  });
  it("retains segmented document parts for the parent's full thirty days", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await put(a, "m/first/raw/large");
    await put(a, "m/first/part/large");
    const refs = ["m/first/head", "m/first/raw/base"];
    expect((await retain(a, "first", refs)).status).toBe(200);
    await db
      .prepare(
        "UPDATE objects SET updated_at=unixepoch()-86401 WHERE id='m/first/part/large'",
      )
      .run();
    await call("/__test/cron");
    expect(
      (await call("/v1/objects/m/first/part/large", "GET", undefined, a.token))
        .status,
    ).toBe(200);
    const deadlines = (
      await db
        .prepare(
          "SELECT purge_at FROM meeting_gc WHERE object IN('m/first/raw/large','m/first/part/large')",
        )
        .all<{ purge_at: number }>()
    ).results;
    expect(deadlines).toHaveLength(2);
    expect(deadlines[0].purge_at).toBe(deadlines[1].purge_at);
    expect((await retain(a, "first", refs)).status).toBe(200);
    await db.prepare("UPDATE meeting_gc SET purge_at=0").run();
    await call("/__test/cron");
    expect(
      (await call("/v1/objects/m/first/raw/large", "GET", undefined, a.token))
        .status,
    ).toBe(404);
    expect(
      (await call("/v1/objects/m/first/part/large", "GET", undefined, a.token))
        .status,
    ).toBe(404);
  });
  it("expires incomplete audio after a day, keeps committed chunks, and releases bytes", async () => {
    const a = await signin();
    await register(a, "first", 1);
    await blob(a, "first", "kept");
    expect(
      (
        await retain(a, "first", [
          "m/first/head",
          "m/first/raw/base",
          "blob/first/kept",
        ])
      ).status,
    ).toBe(200);
    await blob(a, "first", "abandoned");
    await db
      .prepare(
        "UPDATE objects SET updated_at=unixepoch()-86401 WHERE id LIKE 'blob/%'",
      )
      .run();
    const old = await db
      .prepare("SELECT body_key FROM objects WHERE id='blob/first/abandoned'")
      .first<{ body_key: string }>();
    await call("/__test/cron");
    expect(
      (await call("/v1/blobs/first/kept", "GET", undefined, a.token)).status,
    ).toBe(200);
    expect(
      (await call("/v1/blobs/first/abandoned", "GET", undefined, a.token))
        .status,
    ).toBe(404);
    expect(
      await db
        .prepare(
          "SELECT size,deleted FROM objects WHERE id='blob/first/abandoned'",
        )
        .first(),
    ).toEqual({ size: 0, deleted: 4 });
    expect(
      await (await mf.getBindings<Env>()).CONTENT.head(old!.body_key),
    ).toBeNull();
    expect((await blob(a, "first", "resumed-new-id")).status).toBe(200);
  });
  it("H2 pages encrypted cutovers immutably and inventories objects without a monolithic export", async () => {
    const a = await signin();
    const path = "/v1/rekey-pages/2/synthetic-checkpoint/0";
    const request = () =>
      mf.dispatchFetch("https://sync.test" + path, {
        method: "PUT",
        headers: { authorization: "Bearer " + a.token, "x-sync-auth": proof },
        body: envelope,
      });
    expect((await request()).status).toBe(200);
    expect((await request()).status).toBe(412);
    expect(
      Buffer.from(
        await (await call(path, "GET", undefined, a.token)).arrayBuffer(),
      ),
    ).toEqual(envelope);
    await db.batch(
      Array.from({ length: 501 }, (_, i) =>
        db
          .prepare("INSERT INTO objects VALUES(?,?,1,0,'',1,?,NULL)")
          .bind(
            a.account.id,
            `synthetic-${String(i).padStart(4, "0")}`,
            Math.floor(Date.now() / 1000),
          ),
      ),
    );
    const first = (await (
      await call("/v1/inventory", "GET", undefined, a.token)
    ).json()) as { objects: { id: string }[]; hasMore: boolean };
    expect(first.objects).toHaveLength(250);
    expect(first.hasMore).toBe(true);
    const second = (await (
      await call(
        "/v1/inventory?after=" + first.objects.at(-1)!.id,
        "GET",
        undefined,
        a.token,
      )
    ).json()) as { objects: { id: string }[] };
    expect(second.objects).toHaveLength(250);
    expect(second.objects[0].id > first.objects.at(-1)!.id).toBe(true);
  });
  it.each(["budget", "capacity", "rate", "epoch"])(
    "rejects checkpoint %s exhaustion before R2 upload",
    async (reason) => {
      const a = await signin();
      const env = await mf.getBindings<Env>();
      if (reason === "budget")
        await db
          .prepare(
            "INSERT INTO meeting_upload_bytes VALUES(?,floor(unixepoch()/86400),2000000000)",
          )
          .bind(a.account.id)
          .run();
      if (reason === "capacity")
        await db
          .prepare(
            "INSERT INTO rekey_pages VALUES(?,2,'full',0,'synthetic',134217728,0)",
          )
          .bind(a.account.id)
          .run();
      if (reason === "rate")
        await db
          .prepare("INSERT INTO rate_limits VALUES(?,300,unixepoch()+60)")
          .bind(`checkpoint:${a.account.id}`)
          .run();
      let writes = 0;
      const content = new Proxy(env.CONTENT, {
        get(target, property) {
          if (property === "put")
            return async () => {
              writes++;
              throw new Error("unexpected upload");
            };
          const member = Reflect.get(target, property);
          return typeof member === "function" ? member.bind(target) : member;
        },
      });
      await expect(
        rekeyPages(
          new Request(
            `https://sync.test/v1/rekey-pages/${reason === "epoch" ? 3 : 2}/synthetic/0`,
            {
              method: "PUT",
              headers: {
                authorization: "Bearer " + a.token,
                "x-sync-auth": proof,
              },
              body: envelope,
            },
          ),
          { ...env, CONTENT: content },
        ),
      ).rejects.toThrow();
      expect(writes).toBe(0);
    },
  );
  it("allows a deletion head after eviction without resurrecting other objects", async () => {
    const a = await signin();
    await register(a, "old", 1);
    await hold(a, "old");
    await call("/v1/meetings/new", "POST", { createdAt: 2 * 86400 }, a.token);
    await db.prepare("UPDATE accounts SET meeting_quota=8192").run();
    expect((await put(a, "m/new/head")).status).toBe(200);
    expect((await put(a, "m/old/raw/resurrection")).status).toBe(410);
    expect((await put(a, "m/old/head", 2)).status).toBe(200);
    expect(
      (
        await call(
          "/v1/meetings/old",
          "DELETE",
          { confirmed: true, headRevision: 3 },
          a.token,
        )
      ).status,
    ).toBe(200);
    expect(
      (await call("/v1/objects/m/old/head", "GET", undefined, a.token)).status,
    ).toBe(200);
    expect(
      (await db.prepare("SELECT state FROM meetings WHERE id='old'").first())
        ?.state,
    ).toBe("deleted");
  });
  it("L6 an R2 failure cannot evict a held meeting", async () => {
    const a = await signin();
    await register(a, "old", 1);
    await hold(a, "old");
    await call("/v1/meetings/new", "POST", { createdAt: 2 * 86400 }, a.token);
    await db.prepare("UPDATE accounts SET meeting_quota=8192").run();
    const env = await mf.getBindings<Env>();
    const content = new Proxy(env.CONTENT, {
      get(target, property) {
        if (property === "put")
          return async () => {
            throw new Error("synthetic R2 failure");
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    await expect(
      store(
        new Request("https://sync.test/v1/objects/m/new/head", {
          method: "PUT",
          headers: {
            authorization: "Bearer " + a.token,
            "x-sync-auth": proof,
            "if-match": '\"0\"',
            "x-sync-epoch": "1",
          },
          body: envelope,
        }),
        { ...env, CONTENT: content },
      ),
    ).rejects.toThrow();
    expect(
      (await db.prepare("SELECT state FROM meetings WHERE id='old'").first())
        ?.state,
    ).toBe("active");
    expect(
      (await call("/v1/objects/m/old/head", "GET", undefined, a.token)).status,
    ).toBe(200);
  });
  it("L5 a meeting cleanup failure does not stop account retention or backup", async () => {
    await signin();
    await db.exec("ALTER TABLE meeting_gc RENAME TO meeting_gc_hidden;");
    try {
      expect((await call("/__test/cron")).status).toBe(200);
      expect(
        (
          await db
            .prepare(
              "SELECT last_failure FROM operations WHERE name='meeting-gc'",
            )
            .first()
        )?.last_failure,
      ).toBeGreaterThan(0);
      expect(
        (
          await db
            .prepare("SELECT last_ok FROM operations WHERE name='backup'")
            .first()
        )?.last_ok,
      ).toBeGreaterThan(0);
    } finally {
      await db.exec("ALTER TABLE meeting_gc_hidden RENAME TO meeting_gc;");
    }
  });
  it("L5 a checkpoint cleanup failure does not stop unrelated maintenance", async () => {
    await signin();
    const env = await mf.getBindings<Env>();
    const database = new Proxy(env.DB, {
      get(target, property) {
        if (property === "prepare")
          return (sql: string) => {
            if (
              sql.startsWith(
                "SELECT account,epoch,nonce,page,body_key FROM rekey_pages",
              )
            )
              throw new Error("synthetic checkpoint cleanup failure");
            return target.prepare(sql);
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    await maintenance({ ...env, DB: database });
    expect(
      (
        await db
          .prepare(
            "SELECT last_failure FROM operations WHERE name='checkpoint-gc'",
          )
          .first()
      )?.last_failure,
    ).toBeGreaterThan(0);
    expect(
      (
        await db
          .prepare("SELECT last_ok FROM operations WHERE name='backup'")
          .first()
      )?.last_ok,
    ).toBeGreaterThan(0);
  });
  it.each(["stale-hold", "failed-insert"])(
    "L6 rolls back eviction after R2 success: %s",
    async (failure) => {
      const a = await signin();
      await register(a, "old", 1);
      await hold(a, "old");
      await call("/v1/meetings/new", "POST", { createdAt: 2 * 86400 }, a.token);
      await db.prepare("UPDATE accounts SET meeting_quota=8192").run();
      if (failure === "failed-insert")
        await db.exec(
          "CREATE TRIGGER synthetic_failed_write BEFORE INSERT ON objects WHEN NEW.id='m/new/head' BEGIN SELECT RAISE(ABORT,'synthetic_write_failure'); END;",
        );
      const env = await mf.getBindings<Env>();
      let uploaded: string | undefined;
      const content = new Proxy(env.CONTENT, {
        get(target, property) {
          if (property === "put")
            return async (...args: Parameters<R2Bucket["put"]>) => {
              const result = await target.put(...args);
              uploaded = args[0];
              if (failure === "stale-hold")
                await db.prepare("DELETE FROM holds").run();
              return result;
            };
          const member = Reflect.get(target, property);
          return typeof member === "function" ? member.bind(target) : member;
        },
      });
      try {
        await expect(
          store(
            new Request("https://sync.test/v1/objects/m/new/head", {
              method: "PUT",
              headers: {
                authorization: "Bearer " + a.token,
                "x-sync-auth": proof,
                "if-match": '"0"',
                "x-sync-epoch": "1",
              },
              body: envelope,
            }),
            { ...env, CONTENT: content },
          ),
        ).rejects.toThrow();
        expect(uploaded).toBeDefined();
        expect(await env.CONTENT.head(uploaded!)).toBeNull();
        expect(
          (
            await db
              .prepare("SELECT state FROM meetings WHERE id='old'")
              .first()
          )?.state,
        ).toBe("active");
        expect(
          (await call("/v1/objects/m/old/head", "GET", undefined, a.token))
            .status,
        ).toBe(200);
        expect(
          await db
            .prepare("SELECT id FROM objects WHERE id='m/new/head'")
            .first(),
        ).toBeNull();
      } finally {
        if (failure === "failed-insert")
          await db.exec("DROP TRIGGER synthetic_failed_write;");
      }
    },
  );
});

describe("meeting migration follow-up", () => {
  it("S2 restores deleted and dropped meetings and retries an interrupted restore", async () => {
    const a = await signin();
    for (const state of ["deleted", "dropped"] as const) {
      expect(
        (
          await call(
            `/v1/meetings/${state}`,
            "POST",
            { createdAt: 86400 },
            a.token,
          )
        ).status,
      ).toBe(200);
      expect((await put(a, `m/${state}/raw/base`)).status).toBe(200);
      expect((await put(a, `m/${state}/head`)).status).toBe(200);
      await db
        .prepare("UPDATE meetings SET state=? WHERE id=?")
        .bind(state, state)
        .run();
      await db
        .prepare(
          "UPDATE objects SET deleted=?,size=0,body_key='',rev=rev+1 WHERE meeting=? AND id NOT LIKE '%/head'",
        )
        .bind(state === "deleted" ? 1 : 2, state)
        .run();
    }
    const env = await mf.getBindings<Env>();
    const prefix = await backup(env);
    const key = (await db
      .prepare("SELECT body_key FROM objects WHERE id='m/dropped/head'")
      .first<{ body_key: string }>())!.body_key;
    const body = await (await env.BACKUPS.get(
      prefix + "content/" + key,
    ))!.arrayBuffer();
    await env.BACKUPS.delete(prefix + "content/" + key);
    await db.exec("DELETE FROM accounts; DELETE FROM trash;");
    const target = { ...env, ENVIRONMENT: "staging" };
    await expect(
      restore(target, env.BACKUPS, prefix, new Set()),
    ).rejects.toThrow("restore_missing_body");
    expect(await db.prepare("SELECT 1 FROM accounts").first()).not.toBeNull();
    await env.BACKUPS.put(prefix + "content/" + key, body);
    await restore(target, env.BACKUPS, prefix, new Set());
    expect(
      (await db.prepare("SELECT id,state FROM meetings ORDER BY id").all())
        .results,
    ).toEqual([
      { id: "deleted", state: "deleted" },
      { id: "dropped", state: "dropped" },
    ]);
    expect(
      (
        await db
          .prepare(
            "SELECT deleted FROM objects WHERE id LIKE '%/raw/base' ORDER BY id",
          )
          .all()
      ).results,
    ).toEqual([{ deleted: 1 }, { deleted: 2 }]);
    expect(await db.prepare("SELECT 1 FROM restore_run").first()).toBeNull();
    expect(await db.prepare("SELECT 1 FROM sessions").first()).toBeNull();
    await expect(
      restore(target, env.BACKUPS, prefix, new Set()),
    ).rejects.toThrow("restore_requires_empty_database");
    await expect(
      db
        .prepare(
          "INSERT INTO objects(account,id,rev,size,body_key,deleted,updated_at,meeting) VALUES(?,'m/deleted/raw/forbidden',1,4096,'synthetic',0,0,'deleted')",
        )
        .bind(a.account.id)
        .run(),
    ).rejects.toThrow("meeting_inactive");
  });
  it("L-d quota reads compute entitlement without reconciling D1", async () => {
    const a = await signin();
    await db
      .prepare("UPDATE accounts SET is_test=0,meeting_quota=100000000000")
      .run();
    const env = await mf.getBindings<Env>();
    const database = new Proxy(env.DB, {
      get(target, property) {
        if (property === "prepare")
          return (sql: string) => {
            if (sql.includes("UPDATE accounts SET meeting_quota"))
              throw Error("quota read wrote D1");
            return target.prepare(sql);
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    const response = await meetings(
      new Request("https://sync.test/v1/meetings", {
        headers: { authorization: "Bearer " + a.token },
      }),
      { ...env, DB: database },
    );
    expect(
      ((await response!.json()) as { storage: { quota: number } }).storage
        .quota,
    ).toBe(2000000000);
    expect(
      (await db.prepare("SELECT meeting_quota FROM accounts").first())
        ?.meeting_quota,
    ).toBe(100000000000);
  });
  it("S3 key-authorized rotation reads have an independent finite budget", async () => {
    const a = await signin();
    await put(a);
    await db
      .prepare(
        "INSERT INTO rate_limits(key,count,expires_at) VALUES(?,300,unixepoch()+60)",
      )
      .bind("read:" + a.account.id)
      .run();
    expect(
      (await call("/v1/inventory", "GET", undefined, a.token)).status,
    ).toBe(429);
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
          "X-Sync-Auth": pk,
        })
      ).status,
    ).toBe(403);
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(403);
    expect(
      (
        await call("/v1/rekey/prepare", "POST", { epoch: 1 }, a.token, {
          "X-Sync-Auth": pk,
        })
      ).status,
    ).toBe(403);
    expect(
      (await call("/v1/rekey/prepare", "POST", { epoch: 2 }, a.token)).status,
    ).toBe(409);
    expect(
      (await call("/v1/rekey/prepare", "POST", { epoch: 1 }, a.token)).status,
    ).toBe(200);
    const b = await signin("device-b");
    expect(
      (
        await call("/v1/inventory", "GET", undefined, b.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(403);
    expect(
      (await call("/v1/rekey/prepare", "POST", { epoch: 1 }, b.token)).status,
    ).toBe(409);
    for (let i = 0; i < 305; i++)
      expect(
        (
          await call(
            i % 2 ? "/v1/objects/vocabulary" : "/v1/inventory",
            "GET",
            undefined,
            a.token,
            { "X-Sync-Rotation": "1" },
          )
        ).status,
      ).toBe(200);
    expect(
      (
        await db
          .prepare("SELECT count FROM rate_limits WHERE key=?")
          .bind("read:" + a.account.id)
          .first()
      )?.count,
    ).toBe(301);
    for (const path of [
      "/v1/blobs/synthetic/chunk",
      "/v1/objects/blob/synthetic/chunk",
      "/v1/objects/%62lob/synthetic/chunk",
    ]) {
      expect(
        (
          await call(path, "GET", undefined, a.token, {
            "X-Sync-Rotation": "1",
          })
        ).status,
      ).toBe(429);
    }
    await db
      .prepare("UPDATE rate_limits SET count=2000 WHERE key=?")
      .bind("rotation-read:" + a.account.id)
      .run();
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(429);
    await db
      .prepare("UPDATE rate_limits SET count=0 WHERE key=?")
      .bind("rotation-read:" + a.account.id)
      .run();
    await db
      .prepare("UPDATE rate_limits SET count=20000 WHERE key=?")
      .bind("rotation-read-day:" + a.account.id)
      .run();
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(429);
    await db
      .prepare("DELETE FROM rate_limits WHERE key=?")
      .bind("rotation-read-day:" + a.account.id)
      .run();
    await db.prepare("UPDATE rotation_reads SET expires_at=0").run();
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(403);
    expect(
      (await call("/v1/rekey/prepare", "POST", { epoch: 1 }, a.token)).status,
    ).toBe(200);
    await db
      .prepare("UPDATE accounts SET epoch=2 WHERE id=?")
      .bind(a.account.id)
      .run();
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(403);
    await db
      .prepare("UPDATE accounts SET epoch=1 WHERE id=?")
      .bind(a.account.id)
      .run();
    expect(
      (await call("/v1/rekey/prepare", "DELETE", undefined, a.token)).status,
    ).toBe(200);
    expect(
      (
        await call("/v1/inventory", "GET", undefined, a.token, {
          "X-Sync-Rotation": "1",
        })
      ).status,
    ).toBe(403);
  }, 30000);
  it("S4 backup completes before a failed orphan sweep", async () => {
    await signin();
    const env = await mf.getBindings<Env>();
    const content = new Proxy(env.CONTENT, {
      get(target, property) {
        if (property === "list")
          return async () => {
            throw Error("synthetic orphan sweep failure");
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    await expect(maintenance({ ...env, CONTENT: content })).rejects.toThrow(
      "sync_maintenance_failed",
    );
    expect(
      (
        await db
          .prepare("SELECT last_ok FROM operations WHERE name='backup'")
          .first()
      )?.last_ok,
    ).toBeGreaterThan(0);
  });
  it("F5 reused object and checkpoint bodies leave trash without losing their content", async () => {
    const a = await signin();
    await put(a);
    const env = await mf.getBindings<Env>();
    const row = await db
      .prepare(
        "SELECT body_key FROM objects WHERE account=? AND id='vocabulary'",
      )
      .bind(a.account.id)
      .first<{ body_key: string }>();
    const checkpoint = a.account.id + "/synthetic-live-checkpoint";
    await env.CONTENT.put(checkpoint, "synthetic encrypted checkpoint");
    await db
      .prepare("INSERT INTO rekey_pages VALUES(?,2,'synthetic',0,?,32,0)")
      .bind(a.account.id, checkpoint)
      .run();
    await db
      .prepare("INSERT INTO trash VALUES(?,?,0)")
      .bind(row!.body_key, a.account.id)
      .run();
    await db
      .prepare("INSERT INTO trash VALUES(?,?,0)")
      .bind(checkpoint, a.account.id)
      .run();
    await maintenance(env);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM trash").first())?.n,
    ).toBe(0);
    expect(await env.CONTENT.head(row!.body_key)).not.toBeNull();
    expect(await env.CONTENT.head(checkpoint)).not.toBeNull();
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM rekey_pages").first())?.n,
    ).toBe(1);
  });
  it("S4 orphan checks are batched and the trash drain exceeds 500", async () => {
    const a = await signin();
    const env = await mf.getBindings<Env>();
    const keys = Array.from(
      { length: 750 },
      (_, i) => a.account.id + "/synthetic-retired-" + i,
    );
    await db.batch(
      keys.map((key) =>
        db.prepare("INSERT INTO trash VALUES(?,?,0)").bind(key, a.account.id),
      ),
    );
    let orphanQueries = 0,
      deleted = 0;
    const database = new Proxy(env.DB, {
      get(target, property) {
        if (property === "prepare")
          return (sql: string) => {
            if (
              sql.startsWith("SELECT body_key FROM objects WHERE body_key IN")
            )
              orphanQueries++;
            return target.prepare(sql);
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    const content = new Proxy(env.CONTENT, {
      get(target, property) {
        if (property === "list")
          return async () => ({
            objects: Array.from({ length: 500 }, (_, i) => ({
              key: "synthetic-orphan-" + i,
              uploaded: new Date(0),
            })),
            truncated: false,
          });
        if (property === "delete")
          return async (keys: string | string[]) => {
            deleted += Array.isArray(keys) ? keys.length : 1;
          };
        const member = Reflect.get(target, property);
        return typeof member === "function" ? member.bind(target) : member;
      },
    });
    // Missing retired bodies deliberately fail backup, but cannot stop cleanup.
    await expect(
      maintenance({ ...env, DB: database, CONTENT: content }),
    ).rejects.toThrow("sync_maintenance_failed");
    expect(orphanQueries).toBe(1);
    expect(deleted).toBe(1250);
    expect(
      (await db.prepare("SELECT COUNT(*) AS n FROM trash").first())?.n,
    ).toBe(0);
    expect(
      (
        await db
          .prepare("SELECT last_failure FROM operations WHERE name='backup'")
          .first()
      )?.last_failure,
    ).toBeGreaterThan(0);
  });
});
