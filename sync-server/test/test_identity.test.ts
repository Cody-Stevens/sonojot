import { afterAll, beforeAll, beforeEach, expect, it } from "vitest";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { readFileSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";

const secret = "ab".repeat(32); // Synthetic fixture, never a deployed credential.
const pk = Buffer.alloc(32, 7).toString("base64url");
const payload = {
  subject: "synctest-fixture",
  device: "device-a",
  publicKey: pk,
  platform: "Test",
};
const instances: Miniflare[] = [];
let staging: Miniflare;
let absent: Miniflare;
let production: Miniflare;
let missingSecret: Miniflare;
let db: Awaited<ReturnType<Miniflare["getD1Database"]>>;
beforeAll(async () => {
  const out = await build({
    entryPoints: ["test/entry.ts"],
    bundle: true,
    format: "esm",
    platform: "browser",
    write: false,
    target: "es2022",
  });
  const make = (bindings: Record<string, string>) => {
    const mf = new Miniflare(
      convertV4MiniflareOptions({
        modules: true,
        script: out.outputFiles[0].text,
        compatibilityDate: "2026-11-08",
        compatibilityFlags: ["nodejs_compat"],
        d1Databases: ["DB"],
        r2Buckets: ["CONTENT", "BACKUPS"],
        bindings: { PUBLIC_ORIGIN: "https://sync.test", ...bindings },
      }),
    );
    instances.push(mf);
    return mf;
  };
  staging = make({
    ENVIRONMENT: "staging",
    SYNC_TEST_IDENTITY: "true",
    SYNC_TEST_TOKEN: secret,
  });
  absent = make({ ENVIRONMENT: "staging", SYNC_TEST_TOKEN: secret });
  production = make({
    ENVIRONMENT: "production",
    SYNC_TEST_IDENTITY: "true",
    SYNC_TEST_TOKEN: secret,
  });
  missingSecret = make({ ENVIRONMENT: "staging", SYNC_TEST_IDENTITY: "true" });
  for (const mf of instances) {
    const database = await mf.getD1Database("DB");
    for (const file of readdirSync("migrations")
      .filter((n) => n.endsWith(".sql"))
      .sort()) {
      await database.exec(readFileSync("migrations/" + file, "utf8"));
    }
  }
  db = await staging.getD1Database("DB");
}, 30000);
afterAll(async () => {
  await Promise.all(instances.map((mf) => mf.dispose()));
});
beforeEach(async () => {
  await db.exec(
    "DELETE FROM sessions; DELETE FROM devices; DELETE FROM identities; DELETE FROM accounts; DELETE FROM rate_limits;",
  );
});
function signin(
  mf = staging,
  data = payload,
  supplied = secret,
  method = "POST",
) {
  return mf.dispatchFetch("https://sync.test/v1/auth/test-session", {
    method,
    headers: {
      "content-type": "application/json",
      "X-Sync-Test-Token": supplied,
    },
    body: method === "POST" ? JSON.stringify(data) : undefined,
  });
}
it("is absent without the staging opt-in and in production even with the token", async () => {
  expect((await signin(absent)).status).toBe(404);
  expect((await signin(production)).status).toBe(404);
  expect((await signin(staging, payload, secret, "GET")).status).toBe(404);
});
it("rejects wrong, short, missing and unconfigured tokens", async () => {
  for (const value of ["cd".repeat(32), "x", ""])
    expect((await signin(staging, payload, value)).status).toBe(401);
  expect((await signin(missingSecret)).status).toBe(401);
  expect(await db.prepare("SELECT COUNT(*) n FROM accounts").first("n")).toBe(
    0,
  );
});
it("requires a synthetic namespace and valid device keys", async () => {
  for (const subject of [
    "user_real",
    "synctest-",
    "synctest-a@example.com",
    "synctest-UPPER",
  ]) {
    expect((await signin(staging, { ...payload, subject })).status).toBe(400);
  }
  expect((await signin(staging, { ...payload, publicKey: "bad" })).status).toBe(
    400,
  );
});
it("issues hashed sessions only for marked test accounts and keeps WorkOS identities separate", async () => {
  await db
    .prepare(
      "INSERT INTO accounts(id,email,created_at,last_seen_at) VALUES('real',?,1,1)",
    )
    .bind(payload.subject + "@example.invalid")
    .run();
  await db
    .prepare(
      "INSERT INTO identities(provider,subject,account) VALUES('workos',?,'real')",
    )
    .bind(payload.subject)
    .run();
  const response = await signin();
  expect(response.status).toBe(200);
  const a = (await response.json()) as {
    token: string;
    account: { id: string };
    device: string;
  };
  expect(a.account.id).not.toBe("real");
  expect(
    await db
      .prepare("SELECT is_test FROM accounts WHERE id=?")
      .bind(a.account.id)
      .first("is_test"),
  ).toBe(1);
  expect(
    await db
      .prepare("SELECT is_test FROM accounts WHERE id='real'")
      .first("is_test"),
  ).toBe(0);
  const session = await db
    .prepare("SELECT hash,workos_user FROM sessions WHERE account=?")
    .bind(a.account.id)
    .first();
  expect(session?.hash).toBe(
    createHash("sha256").update(a.token).digest("base64url"),
  );
  expect(session?.workos_user).toBeNull();
  const b = (await (
    await signin(staging, { ...payload, device: "device-b" })
  ).json()) as { account: { id: string } };
  expect(b.account.id).toBe(a.account.id);
  const proofless = await staging.dispatchFetch("https://sync.test/v1/wraps", {
    method: "PUT",
    headers: {
      authorization: "Bearer " + a.token,
      "content-type": "application/json",
    },
    body: "{}",
  });
  expect(proofless.status).toBe(403);
});
it("cannot claim an unmarked account or replace a revoked device", async () => {
  const a = (await (await signin()).json()) as { account: { id: string } };
  await db.prepare("UPDATE devices SET revoked_at=1").run();
  expect((await signin()).status).toBe(403);
  await db
    .prepare("UPDATE accounts SET is_test=0 WHERE id=?")
    .bind(a.account.id)
    .run();
  expect(
    (await signin(staging, { ...payload, device: "device-b" })).status,
  ).toBe(403);
});
it("rejects changed public keys and bounds attempts by subject and IP", async () => {
  expect((await signin()).status).toBe(200);
  expect(
    (
      await signin(staging, {
        ...payload,
        publicKey: Buffer.alloc(32, 8).toString("base64url"),
      })
    ).status,
  ).toBe(403);
  await db
    .prepare(
      "UPDATE rate_limits SET count=10 WHERE key LIKE 'test-signin-subject:%'",
    )
    .run();
  expect((await signin()).status).toBe(429);
  await db
    .prepare(
      "UPDATE rate_limits SET count=30 WHERE key LIKE 'test-signin-ip:%'",
    )
    .run();
  expect(
    (await signin(staging, { ...payload, subject: "synctest-different" }))
      .status,
  ).toBe(429);
});
it("keeps the opt-in out of the base and any production configuration", () => {
  const config = JSON.parse(readFileSync("wrangler.jsonc", "utf8"));
  expect(config.vars?.SYNC_TEST_IDENTITY).toBeUndefined();
  expect(config.env.production).toBeUndefined();
  expect(config.env.staging.vars.ENVIRONMENT).toBe("staging");
  expect(config.env.staging.vars.SYNC_TEST_IDENTITY).toBe("true");
});
