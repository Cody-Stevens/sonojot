import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { readFileSync, readdirSync } from "node:fs";
import { createHash, randomBytes } from "node:crypto";
import {
  codeMail,
  mailOrigin,
  noticeKinds,
  noticeMail,
  platformName,
  when,
  type Mail,
} from "../src/mail";
import { NOTICE_CRON } from "../src/notices";

const origin = "https://auth.sonojot.com";
const at = Date.UTC(2026, 10, 14, 15, 42) / 1000;
function samples(o: string | null): Mail[] {
  return [
    codeMail(
      {
        to: "person@example.com",
        code: "123456",
        expiresAt: at * 1000 + 600000,
        intent: "sign-in",
        now: at * 1000,
      },
      o,
    ),
    codeMail(
      {
        to: "person@example.com",
        code: "654321",
        expiresAt: at * 1000 + 600000,
        intent: "sign-up",
        now: at * 1000,
      },
      o,
    ),
    ...noticeKinds.map((kind) =>
      noticeMail(kind, { to: "person@example.com", at, detail: "macos" }, o),
    ),
  ];
}

describe("email templates", () => {
  it("always have html and text, reply to support, and never mention WorkOS", () => {
    for (const o of [origin, null])
      for (const m of samples(o)) {
        expect(m.html).toMatch(/^<!doctype html>/);
        expect(m.html.length).toBeGreaterThan(500);
        expect(m.text.length).toBeGreaterThan(50);
        expect(m.replyTo).toBe("support@sonojot.com");
        expect(m.text).toContain("Questions? Reply to this email.");
        expect(m.html).toContain("Questions? Reply to this email.");
        expect(JSON.stringify(m)).not.toMatch(/workos|authkit/i);
        // No tracking, no web fonts and no links but the site and our own logo.
        const urls = [...m.html.matchAll(/(?:href|src)="([^"]*)"/g)].map(
          (x) => x[1],
        );
        for (const url of urls)
          expect(url).toMatch(
            /^(https:\/\/sonojot\.com|https:\/\/auth\.sonojot\.com\/assets\/sonojot-mail-(light|dark)\.png)$/,
          );
        expect(m.html).not.toMatch(/@import|<link|<script|url\(/i);
      }
  });

  it("puts the code in the subject and body and says when it expires", () => {
    const [signin, signup] = samples(origin);
    expect(signin.from).toEqual({ name: "SonoJot", email: "signin@sonojot.com" });
    expect(signin.subject).toBe("123456 is your SonoJot code");
    expect(signin.text).toContain("\n123456\n");
    expect(signin.html).toContain(">123456</td>");
    expect(signin.text).toContain(
      "Enter it on the SonoJot sign-in page. It expires in 10 minutes.",
    );
    expect(signin.text).toContain("If you didn't ask for this, you can ignore it.");
    expect(signup.text).toContain("Create your sync account");
    expect(signup.subject).toBe("654321 is your SonoJot code");
    const short = codeMail(
      {
        to: "a@example.com",
        code: "111111",
        expiresAt: 1000 + 50000,
        intent: "sign-in",
        now: 1000,
      },
      null,
    );
    expect(short.text).toContain("It expires in 1 minute.");
    expect(() =>
      codeMail({ to: "a@example.com", code: "<b>", expiresAt: 0, intent: "sign-in" }, null),
    ).toThrow();
  });

  it("names the computer that joined and when", () => {
    const m = noticeMail(
      "new-device",
      { to: "person@example.com", at, detail: "macos" },
      origin,
    );
    expect(m.from).toEqual({ name: "SonoJot", email: "notices@sonojot.com" });
    expect(m.subject).toBe("Mac joined your SonoJot sync");
    expect(m.text).toContain(
      "Mac signed in to your sync account on 14 November 2026 at 15:42 UTC. Your other computers hand it your key on their own.",
    );
    expect(m.text).toContain("If this was you, there's nothing to do.");
    expect(m.text).toContain(
      "If it wasn't, open SonoJot on one of your computers, go to Account and sync, and choose Not me. That removes it and changes your key.",
    );
    expect(m.html).toContain("Mac joined sync");
    expect(platformName("windows")).toBe("Windows PC");
    expect(platformName("Linux")).toBe("Linux computer");
    expect(platformName("Test")).toBe("A new computer");
    expect(platformName(null)).toBe("A new computer");
    expect(when(Date.UTC(2026, 0, 2, 3, 4) / 1000)).toBe(
      "2 January 2026 at 03:04 UTC",
    );
  });

  it("keeps the facts of the other security notices", () => {
    const [, , , identity, email, reset] = samples(origin);
    expect(identity.subject).toBe("Your SonoJot sign-in email changed");
    expect(identity.text).toMatch(/notices keep coming to this address until you confirm/);
    expect(identity.text).toMatch(/Confirm contact email/);
    expect(identity.text).toMatch(/secure your sign-in/);
    expect(email.subject).toBe("Your SonoJot contact email changed");
    expect(email.text).toMatch(/previous address/);
    expect(email.text).toMatch(/changes your key, then secure your email accounts/);
    expect(reset.subject).toBe("Someone asked to reset your SonoJot sync");
    expect(reset.text).toMatch(/can't finish for 72 hours/);
    expect(reset.text).toMatch(/expires 7 days after that/);
    expect(reset.text).toMatch(/cancels the reset on its own/);
    expect(() => noticeMail("other", { to: "a@example.com", at }, null)).toThrow();
  });

  it("uses the hosted logo only on sonojot.com and escapes what it prints", () => {
    expect(mailOrigin({ AUTH_ORIGIN: origin })).toBe(origin);
    for (const AUTH_ORIGIN of [
      "",
      "https://sonojot-sync-staging.fixture.workers.dev",
      "http://auth.sonojot.com",
      "https://auth.sonojot.com.evil.com",
      "https://auth.sonojot.com/",
    ])
      expect(mailOrigin({ AUTH_ORIGIN })).toBeNull();
    const plain = samples(null)[0];
    expect(plain.html).not.toContain("<img");
    expect(plain.html).toContain(">SonoJot</p>");
    const hosted = samples(origin)[0];
    expect(hosted.html).toContain(
      'src="https://auth.sonojot.com/assets/sonojot-mail-light.png"',
    );
    expect(hosted.html).toContain('alt="SonoJot"');
    const odd = noticeMail(
      "new-device",
      { to: "a@example.com", at, detail: "<script>" },
      null,
    );
    expect(odd.html).not.toContain("<script>");
  });

  it("ships the logo files and a sender-restricted binding with the notice trigger", () => {
    for (const variant of ["light", "dark"])
      expect(
        readFileSync(`public/assets/sonojot-mail-${variant}.png`).subarray(1, 4).toString(),
      ).toBe("PNG");
    const staging = JSON.parse(readFileSync("wrangler.jsonc", "utf8")).env.staging;
    expect(staging.triggers.crons).toContain(NOTICE_CRON);
    expect(staging.send_email[0].allowed_sender_addresses.sort()).toEqual([
      "notices@sonojot.com",
      "signin@sonojot.com",
    ]);
  });
});

// The worker with MAIL_ENABLED on and a recording mail stub (test/entry.ts).
let mf: Miniflare;
let db: Awaited<ReturnType<Miniflare["getD1Database"]>>;
const hash = (x: string) => createHash("sha256").update(x).digest("base64url");
const pk = Buffer.alloc(32, 3).toString("base64url");
function call(
  path: string,
  method = "GET",
  data?: unknown,
  extra: Record<string, string> = {},
) {
  return mf.dispatchFetch("https://sync.test" + path, {
    method,
    headers: { ...(data ? { "content-type": "application/json" } : {}), ...extra },
    body: data ? JSON.stringify(data) : undefined,
    redirect: "manual",
  });
}
async function start(device: string, platform: string, intent = "sign-in") {
  const verifier = randomBytes(32).toString("base64url");
  const r = await call("/v1/auth/start", "POST", {
    intent,
    challenge: hash(verifier),
    device,
    publicKey: pk,
    platform,
  });
  expect(r.status).toBe(200);
  const a = (await r.json()) as { id: string };
  const b = await call("/v1/auth/browser?request=" + a.id);
  expect(b.status).toBe(303);
  return { id: a.id, verifier, cookie: b.headers.get("set-cookie")!.split(";")[0] };
}
async function join(device: string, platform: string) {
  const a = await start(device, platform);
  const cb = await call(
    "/v1/auth/callback?state=" + a.id + "&code=synthetic-provider-code",
    "GET",
    undefined,
    { cookie: a.cookie },
  );
  expect(cb.status).toBe(303);
  const t = await call("/v1/auth/token", "POST", {
    request: a.id,
    code: new URL(cb.headers.get("location")!).searchParams.get("code"),
    verifier: a.verifier,
  });
  expect(t.status).toBe(200);
}
function email(a: { id: string; cookie: string }, address: string) {
  return call(
    "/v1/auth/email",
    "POST",
    { request: a.id, email: address },
    { origin: "https://sync.test", cookie: a.cookie },
  );
}
/** Waits for background sends (ctx.waitUntil) to reach the stub. */
async function outbox(expected: number, wait = 2000): Promise<Mail[]> {
  const sent: Mail[] = [];
  const end = Date.now() + wait;
  for (;;) {
    sent.push(...((await (await call("/__test/mail")).json()) as Mail[]));
    if ((expected > 0 && sent.length >= expected) || Date.now() > end)
      return sent;
    await new Promise((r) => setTimeout(r, 25));
  }
}
const queued = async () =>
  (
    await db
      .prepare("SELECT kind,detail,attempts,tried_at FROM notifications")
      .all()
  ).results;

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
        WORKOS_WEBHOOK_SECRET: "synthetic-test-only",
        MAIL_ENABLED: "true",
      },
    }),
  );
  db = await mf.getD1Database("DB");
  for (const f of readdirSync("migrations").filter((f) => f.endsWith(".sql")).sort())
    await db.exec(readFileSync("migrations/" + f, "utf8"));
}, 30000);
afterAll(async () => {
  await mf?.dispose();
});
beforeEach(async () => {
  await db.exec(
    "DELETE FROM notifications; DELETE FROM operations; DELETE FROM sessions; DELETE FROM devices; DELETE FROM identities; DELETE FROM accounts; DELETE FROM auth_requests; DELETE FROM rate_limits;",
  );
  await call("/__test/mail-fail?count=0");
  await outbox(0, 0);
});

describe("SonoJot's own email with MAIL_ENABLED", () => {
  it("sends the sign-in code itself, with the code in the subject and body", async () => {
    const a = await start("email-device", "windows", "sign-up");
    const r = await email(a, "new@example.com");
    expect(r.status).toBe(200);
    const [m, ...rest] = await outbox(1);
    expect(rest).toEqual([]);
    expect(m.from).toEqual({ name: "SonoJot", email: "signin@sonojot.com" });
    expect(m.to).toBe("new@example.com");
    expect(m.replyTo).toBe("support@sonojot.com");
    expect(m.subject).toBe("123456 is your SonoJot code");
    expect(m.text).toContain("123456");
    expect(m.html).toContain("123456");
    expect(m.text).toContain("Create your sync account");
    expect(m.text).toMatch(/It expires in (9|10) minutes\./);
    // The code itself is never stored, only its keyed hash.
    const row = await db
      .prepare("SELECT * FROM auth_requests WHERE id=?")
      .bind(a.id)
      .first();
    expect(JSON.stringify(row)).not.toContain("123456");
    const b = await start("other-device", "macos", "sign-in");
    expect((await email(b, "person@example.invalid")).status).toBe(200);
    expect((await outbox(1))[0].text).toContain("Your sign-in code");
  });

  it("returns the sign-in fault when the send fails, and the same attempt can retry", async () => {
    const a = await start("email-device", "windows", "sign-in");
    await call("/__test/mail-fail?count=1");
    const r = await email(a, "person@example.invalid");
    expect(r.status).toBe(503);
    expect(await r.json()).toEqual({
      error: "Sign-in is temporarily unavailable. Try again.",
    });
    expect(
      await db
        .prepare("SELECT code_started,code_hash FROM auth_requests WHERE id=?")
        .bind(a.id)
        .first(),
    ).toEqual({ code_started: 0, code_hash: null });
    expect((await email(a, "person@example.invalid")).status).toBe(200);
    expect((await outbox(1)).map((m) => m.subject)).toEqual([
      "123456 is your SonoJot code",
    ]);
  });

  it("sends no alert for the first computer and alerts at once when another joins", async () => {
    await join("first", "windows");
    expect(await outbox(0, 300)).toEqual([]);
    expect(await queued()).toEqual([]);
    await join("second", "macos");
    const [m, ...rest] = await outbox(1);
    expect(rest).toEqual([]);
    expect(m.from).toEqual({ name: "SonoJot", email: "notices@sonojot.com" });
    expect(m.to).toBe("person@example.invalid");
    expect(m.subject).toBe("Mac joined your SonoJot sync");
    expect(m.text).toMatch(
      /Mac signed in to your sync account on \d{1,2} \w+ \d{4} at \d\d:\d\d UTC. Your other computers hand it your key on their own./,
    );
    expect(m.html).toContain("choose Not me");
    expect(await queued()).toEqual([]);
    // Signing in again on a known computer is not a join.
    await join("second", "macos");
    expect(await outbox(0, 300)).toEqual([]);
  });

  it("keeps a failed alert queued and the notice run retries it later", async () => {
    await join("first", "windows");
    await call("/__test/mail-fail?count=1");
    await join("second", "linux");
    await new Promise((r) => setTimeout(r, 300));
    expect(await queued()).toMatchObject([
      { kind: "new-device", detail: "linux", attempts: 1 },
    ]);
    expect(
      (await db.prepare("SELECT last_failure FROM operations WHERE name='notices'").first())
        ?.last_failure,
    ).toBeGreaterThan(0);
    // Too soon: the first retry waits 15 minutes.
    expect((await call("/__test/notices")).status).toBe(200);
    expect(await outbox(0, 0)).toEqual([]);
    await db.prepare("UPDATE notifications SET tried_at=tried_at-900").run();
    expect((await call("/__test/notices")).status).toBe(200);
    expect((await outbox(1)).map((m) => m.subject)).toEqual([
      "Linux computer joined your SonoJot sync",
    ]);
    expect(await queued()).toEqual([]);
  });

  it("sends queued security notices from daily maintenance in the branded layout", async () => {
    await join("first", "windows");
    const account = (await db.prepare("SELECT id FROM accounts").first())!.id;
    for (const kind of ["identity-updated", "email-changed", "reset-requested"])
      await db
        .prepare(
          "INSERT INTO notifications(id,account,kind,created_at,recipient) VALUES(?,?,?,?,?)",
        )
        .bind(kind, account, kind, at, kind === "reset-requested" ? null : "old@example.com")
        .run();
    expect((await call("/__test/cron")).status).toBe(200);
    const sent = await outbox(3);
    expect(sent.map((m) => [m.to, m.subject]).sort()).toEqual([
      ["old@example.com", "Your SonoJot contact email changed"],
      ["old@example.com", "Your SonoJot sign-in email changed"],
      ["person@example.invalid", "Someone asked to reset your SonoJot sync"],
    ]);
    for (const m of sent) {
      expect(m.from.email).toBe("notices@sonojot.com");
      expect(m.html).toContain("14 November 2026 at 15:42 UTC");
      expect(JSON.stringify(m)).not.toMatch(/workos/i);
    }
    expect(await queued()).toEqual([]);
  });
});
