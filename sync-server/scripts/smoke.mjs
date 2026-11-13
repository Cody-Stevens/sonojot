import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { createRequire } from "node:module";
import { readFile, mkdir } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
const { chromium } = createRequire(
  new URL("../../rewrite/package.json", import.meta.url),
)("playwright");
const out = await build({
  entryPoints: ["src/index.ts"],
  bundle: true,
  format: "esm",
  platform: "browser",
  write: false,
  target: "es2022",
});
const mf = new Miniflare(
  convertV4MiniflareOptions({
    modules: true,
    script: out.outputFiles[0].text,
    compatibilityDate: "2026-11-08",
    compatibilityFlags: ["nodejs_compat"],
    d1Databases: ["DB"],
    r2Buckets: ["CONTENT", "BACKUPS"],
    bindings: { ENVIRONMENT: "staging", PUBLIC_ORIGIN: "https://sync.test" },
  }),
);
let browser;
try {
  const db = await mf.getD1Database("DB");
  for (const f of [
    "0001_auth",
    "0002_store",
    "0003_rotation",
    "0004_security",
    "0005_account_safety",
    "0006_authkit",
    "0007_test_accounts",
  ])
    await db.exec(await readFile("migrations/" + f + ".sql", "utf8"));
  browser = await chromium.launch({
    channel: process.platform === "win32" ? "msedge" : undefined,
    headless: true,
  });
  const page = await browser.newPage({
    viewport: { width: 1180, height: 900 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.route("https://sync.test/**", async (route) => {
    const request = route.request();
    const r = await mf.dispatchFetch(request.url(), {
      method: request.method(),
      headers: request.headers(),
      body: request.postDataBuffer(),
      redirect: "manual",
    });
    await route.fulfill({
      status: r.status,
      headers: Object.fromEntries(r.headers),
      body: Buffer.from(await r.arrayBuffer()),
    });
  });
  await page.goto("https://sync.test/account");
  await page
    .getByRole("button", { name: "Continue with WorkOS AuthKit" })
    .waitFor();
  await page
    .getByRole("button", { name: "Continue with WorkOS AuthKit" })
    .click();
  await page
    .getByRole("status")
    .filter({ hasText: "not configured" })
    .waitFor();
  await mkdir("../artifacts/sync", { recursive: true });
  await page.screenshot({
    path: "../artifacts/sync/account.png",
    fullPage: true,
  });
  const root = resolve("../web/dist");
  const mime = {
    ".css": "text/css",
    ".js": "text/javascript",
    ".svg": "image/svg+xml",
    ".woff2": "font/woff2",
    ".png": "image/png",
    ".webp": "image/webp",
  };
  await page.route("https://site.test/**", async (route) => {
    const url = new URL(route.request().url());
    const path = resolve(
      root,
      "." +
        (url.pathname === "/sync"
          ? "/sync/index.html"
          : decodeURIComponent(url.pathname)),
    );
    if (!path.startsWith(root + sep)) return route.abort();
    try {
      await route.fulfill({
        body: await readFile(path),
        contentType: mime[extname(path)] ?? "text/html",
      });
    } catch {
      await route.fulfill({ status: 404, body: "" });
    }
  });
  await page.goto("https://site.test/sync");
  await page
    .getByRole("heading", { name: "Your words, on your devices." })
    .waitFor();
  await page.screenshot({ path: "../artifacts/sync/site.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "../artifacts/sync/site-mobile.png",
    fullPage: true,
  });
  if (
    await page.evaluate(() => document.documentElement.scrollWidth > innerWidth)
  )
    throw Error("Mobile page overflows");
  if (errors.length) throw Error(errors.join("; "));
  console.log(
    "Account sign-in failure and desktop/mobile sync page smoke passed; no browser errors.",
  );
} finally {
  await browser?.close();
  await mf.dispose();
}
