import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { readFileSync } from "node:fs";
import { randomBytes, createHash, generateKeyPairSync } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
const out = await build({
  stdin: {
    contents: `import original from './test/entry';
    let objectDownloads=0;
    export default {...original,async fetch(request,env){
      const path=new URL(request.url).pathname;
      if(path==='/__test/requests') return Response.json({objectDownloads});
      if(path.startsWith('/v1/objects/')&&request.method==='GET') objectDownloads++;
      return original.fetch(request,env);
    }};`,
    resolveDir: process.cwd(),
    sourcefile: "local-e2e-entry.ts",
    loader: "ts",
  },
  bundle: true,
  format: "esm",
  platform: "browser",
  write: false,
  target: "es2022",
});
const mf = new Miniflare(
  convertV4MiniflareOptions({
    host: "127.0.0.1",
    port: 0,
    modules: true,
    script: out.outputFiles[0].text,
    compatibilityDate: "2026-11-08",
    compatibilityFlags: ["nodejs_compat"],
    d1Databases: ["DB"],
    r2Buckets: ["CONTENT", "BACKUPS"],
    bindings: {
      PUBLIC_ORIGIN: "",
      ENVIRONMENT: "staging",
      WORKOS_CLIENT_ID: "client_synthetic",
      WORKOS_API_KEY: "synthetic-test-key",
      WORKOS_WEBHOOK_SECRET: "synthetic-test-signature",
    },
  }),
);
try {
  const db = await mf.getD1Database("DB");
  await db.exec(readFileSync("migrations/0001_auth.sql", "utf8"));
  await db.exec(readFileSync("migrations/0002_store.sql", "utf8"));
  await db.exec(readFileSync("migrations/0003_rotation.sql", "utf8"));
  await db.exec(readFileSync("migrations/0004_security.sql", "utf8"));
  await db.exec(readFileSync("migrations/0005_account_safety.sql", "utf8"));
  await db.exec(readFileSync("migrations/0006_authkit.sql", "utf8"));
  await db.exec(readFileSync("migrations/0007_test_accounts.sql", "utf8"));
  let account;
  const sessions = {};
  const identities = {};
  for (const id of ["a", "b"]) {
    const keys = generateKeyPairSync("x25519");
    const secret = keys.privateKey
      .export({ format: "der", type: "pkcs8" })
      .subarray(-32)
      .toString("base64url");
    const publicKey = keys.publicKey
      .export({ format: "der", type: "spki" })
      .subarray(-32)
      .toString("base64url");
    identities[id] = { secret, publicKey };
    const verifier = randomBytes(32).toString("base64url");
    const post = async (path, data) => {
      const r = await mf.dispatchFetch("https://sync.test" + path, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(data),
        redirect: "manual",
      });
      if (!r.ok) throw Error("AuthKit mock exchange failed: " + r.status);
      return r.json();
    };
    const attempt = await post("/v1/auth/start", {
      provider: "authkit",
      challenge: createHash("sha256").update(verifier).digest("base64url"),
      device: id,
      publicKey,
      platform: "Test",
    });
    const browser = await mf.dispatchFetch(attempt.url, { redirect: "manual" });
    const callback = await mf.dispatchFetch(
      "https://sync.test/v1/auth/callback?state=" +
        attempt.id +
        "&code=synthetic-provider-code",
      {
        headers: { cookie: browser.headers.get("set-cookie").split(";")[0] },
        redirect: "manual",
      },
    );
    if (callback.status !== 303) throw Error("AuthKit mock callback failed");
    const returned = new URL(callback.headers.get("location"));
    const session = await post("/v1/auth/token", {
      request: attempt.id,
      code: returned.searchParams.get("code"),
      verifier,
    });
    if (account && account !== session.account.id)
      throw Error("WorkOS identity did not converge");
    account = session.account.id;
    sessions[id] = session.token;
  }
  const { a, b } = sessions;
  const url = (await mf.ready).origin;
  const child = spawn(
    "cargo",
    [
      "test",
      "--manifest-path",
      "../rewrite/src-tauri/Cargo.toml",
      "--lib",
      "sync_engine::tests::two_device_miniflare",
      "--",
      "--ignored",
      "--exact",
    ],
    {
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
      env: {
        ...process.env,
        SONOJOT_SYNC_E2E_URL: url,
        SONOJOT_SYNC_E2E_ACCOUNT: account,
        SONOJOT_SYNC_E2E_A: a,
        SONOJOT_SYNC_E2E_B: b,
        SONOJOT_SYNC_E2E_KEYS: JSON.stringify(identities),
      },
    },
  );
  // Only test summaries reach output; compiler diagnostics remain unrelated to credentials.
  child.stdout.on("data", (b) => process.stdout.write(b));
  let diagnostics = "";
  child.stderr.on("data", (b) => {
    diagnostics = (diagnostics + b).slice(-16000);
  });
  const timer = setTimeout(() => {
    if (process.platform === "win32")
      spawnSync("taskkill", ["/T", "/F", "/PID", String(child.pid)], {
        windowsHide: true,
        stdio: "ignore",
      });
    else child.kill("SIGKILL");
  }, 300000);
  const exit = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", resolve);
  }).finally(() => clearTimeout(timer));
  if (exit !== 0) {
    process.stderr.write(diagnostics);
    throw Error("two-device Rust test failed");
  }
  const env = await mf.getBindings();
  const bodies = await env.CONTENT.list();
  if (!bodies.objects.length) throw Error("no ciphertext stored");
  for (const object of bodies.objects) {
    const body = Buffer.from(
      await (await env.CONTENT.get(object.key)).arrayBuffer(),
    );
    if (
      body.includes(Buffer.from("Asterlume")) ||
      body.subarray(0, 4).toString() !== "SJV1"
    )
      throw Error("server plaintext boundary failed");
  }
  const metadata = await db.prepare("SELECT * FROM objects").all();
  if (JSON.stringify(metadata).includes("Asterlume"))
    throw Error("database plaintext boundary failed");
  console.log(
    "Two Rust devices signed in through mocked WorkOS AuthKit, converged, paired with two confirmations, preserved a conflict and explicitly re-paired after re-key through Miniflare; R2 held only encrypted SJV1 envelopes.",
  );
} finally {
  await mf.dispose();
}
