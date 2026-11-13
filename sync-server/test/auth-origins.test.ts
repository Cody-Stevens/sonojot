import { it, expect } from "vitest";
import { build } from "esbuild";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { readFileSync, readdirSync } from "node:fs";
import { createHash } from "node:crypto";

it("keeps both origins, binds the browser to its host, and preserves web PKCE storage", async () => {
  const out = await build({
    entryPoints: ["test/entry.ts"],
    bundle: true,
    format: "esm",
    platform: "browser",
    write: false,
  });
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: out.outputFiles[0].text,
      compatibilityDate: "2026-11-08",
      compatibilityFlags: ["nodejs_compat"],
      d1Databases: ["DB"],
      bindings: {
        PUBLIC_ORIGIN: "https://api.example.com",
        AUTH_ORIGIN: "https://auth.example.com",
        WORKOS_CLIENT_ID: "client_test",
        WORKOS_API_KEY: "synthetic",
        WORKOS_WEBHOOK_SECRET: "synthetic",
      },
    }),
  );
  try {
    const db = await mf.getD1Database("DB");
    for (const f of readdirSync("migrations")
      .filter((f) => f.endsWith(".sql"))
      .sort())
      await db.exec(readFileSync("migrations/" + f, "utf8"));
    const post = (
      origin: string,
      path: string,
      data: unknown,
      cookie?: string,
    ) =>
      mf.dispatchFetch(origin + path, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          origin,
          ...(cookie ? { cookie } : {}),
        },
        body: JSON.stringify(data),
        redirect: "manual",
      });
    const payload = {
      device: "fixture",
      publicKey: Buffer.alloc(32, 1).toString("base64url"),
      challenge: createHash("sha256")
        .update("synthetic-verifier")
        .digest("base64url"),
      platform: "Test",
    };
    const old = (await (
      await post("https://api.example.com", "/v1/auth/start", payload)
    ).json()) as { id: string; url: string };
    expect(old.url).toMatch(/^https:\/\/api.example.com\//);
    const redirect = await mf.dispatchFetch(old.url, { redirect: "manual" });
    expect(redirect.status).toBe(303);
    expect(redirect.headers.get("set-cookie")).toBeNull();
    const claimed = await mf.dispatchFetch(redirect.headers.get("location")!, {
      redirect: "manual",
    });
    const cookie = claimed.headers.get("set-cookie")!.split(";")[0];
    expect(claimed.headers.get("set-cookie")).toMatch(
      /HttpOnly; Secure; SameSite=Lax; Max-Age=300/,
    );
    expect(claimed.headers.get("location")).toMatch(
      /^https:\/\/auth.example.com\/sign-in/,
    );
    expect(
      (
        await post(
          "https://api.example.com",
          "/v1/auth/email",
          { request: old.id, email: "person@example.com" },
          cookie,
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await post(
          "https://auth.example.com",
          "/v1/auth/email",
          { request: old.id, email: "person@example.com" },
          cookie,
        )
      ).status,
    ).toBe(200);
    const modern = (await (
      await post("https://api.example.com", "/v1/auth/start", {
        ...payload,
        browserOrigin: "https://auth.example.com",
        intent: "sign-up",
      })
    ).json()) as { url: string };
    expect(modern.url).toMatch(/^https:\/\/auth.example.com\//);
    const web = (await (
      await post("https://api.example.com", "/v1/auth/start", {
        ...payload,
        redirect: "web",
        intent: "sign-up",
      })
    ).json()) as { url: string };
    expect(web.url).toMatch(/^https:\/\/api.example.com\//);
    expect(
      (await mf.dispatchFetch("https://untrusted.example.com/sign-in")).status,
    ).toBe(400);
  } finally {
    await mf.dispose();
  }
}, 30000);
