import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { build } from "esbuild";
import { readFileSync } from "node:fs";
import { randomBytes, createHash, generateKeyPairSync } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
const out = await build({
  stdin: {
    contents: `import original from './test/entry';
    import {retainMaintenance} from './src/meeting_retention';
    let rejectObject=null;
    let objectDownloads=0,blobDownloads=0,checkpointDownloads=0,referencePosts=0,holdPosts=0;
    export default {...original,async fetch(request,env){
      const path=new URL(request.url).pathname;
      if(path==='/__test/reject-write') { rejectObject=(await request.json()).object; return Response.json({ready:true}); }
      if(rejectObject && path==='/v1/objects/'+rejectObject && request.method==='PUT') {rejectObject=null;return new Response('synthetic CAS race',{status:412});}
      if(path==='/__test/requests') return Response.json({objectDownloads,blobDownloads,checkpointDownloads,referencePosts,holdPosts});
      if(path==='/__test/meeting-state') {
        const value=await request.json();
        await env.DB.prepare('UPDATE meetings SET state=? WHERE id=?').bind(value.state,value.id).run();
        return Response.json({changed:true});
      }
      if(path==='/__test/clear-limits') {await env.DB.prepare('DELETE FROM rate_limits').run(); return Response.json({cleared:true});}
      if(path==='/__test/expire-upload') {
        const value=await request.json();
        await env.DB.prepare('UPDATE objects SET updated_at=unixepoch()-86401 WHERE id=?').bind(value.id).run();
        await retainMaintenance(env);
        return Response.json({expired:true});
      }
      if(path==='/__test/drop-meeting') {
        const value=await request.json();
        await env.DB.prepare("UPDATE meetings SET state='dropped' WHERE id=?").bind(value.id).run();
        await env.DB.prepare("UPDATE objects SET deleted=2,size=0,body_key='',rev=rev+1 WHERE meeting=?").bind(value.id).run();
        return Response.json({dropped:true});
      }
      if(path==='/__test/pad-inventory') {
        const value=await request.json();
        const count=await env.DB.prepare('SELECT COUNT(*) AS n FROM objects WHERE account=?').bind(value.account).first();
        const missing=500-count.n;
        if(missing<0) throw Error('synthetic inventory already exceeds target');
        if(missing) await env.DB.batch(Array.from({length:missing},(_,i)=>env.DB.prepare("INSERT INTO objects(account,id,rev,size,body_key,deleted,updated_at) VALUES(?,?,1,0,'',1,unixepoch())").bind(value.account,'synthetic-pad-'+i)));
        return Response.json({count:500});
      }
      if(path.startsWith('/v1/rekey-pages/')&&request.method==='GET') checkpointDownloads++;
      if(path.endsWith('/references')&&request.method==='POST') referencePosts++;
      if(path==='/v1/holds'&&request.method==='POST') holdPosts++;
      if(path.startsWith('/v1/objects/')&&request.method==='GET') objectDownloads++;
      if(path.startsWith('/v1/blobs/')&&request.method==='GET') blobDownloads++;
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
  await db.exec(readFileSync("migrations/0008_email_signin.sql", "utf8"));
  await db.exec(readFileSync("migrations/0009_meetings.sql", "utf8"));
  await db.exec(readFileSync("migrations/0010_meeting_lifecycle.sql", "utf8"));
  await db.exec(readFileSync("migrations/0011_meeting_restore.sql", "utf8"));
  let account;
  const sessions = {};
  const identities = {};
  for (const id of ["a", "b", "lost-device"]) {
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
      "--features",
      "synctest",
      "-j",
      "3",
      process.argv.includes("--meeting-library")
        ? "meeting_sync::tests::large_meeting_library_miniflare"
        : process.argv.includes("--meetings")
          ? "meeting_sync::tests::two_device_meeting_miniflare"
          : "sync_engine::tests::two_device_miniflare",
      "--",
      "--ignored",
      "--exact",
      "--nocapture",
    ],
    {
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
      env: {
        ...process.env,
        SONOJOT_SYNC_ORIGIN: "https://sonojot-sync-staging.example.workers.dev",
        SONOJOT_SYNC_E2E_URL: url,
        SONOJOT_SYNC_E2E_ACCOUNT: account,
        SONOJOT_SYNC_E2E_A: a,
        SONOJOT_SYNC_E2E_B: b,
        SONOJOT_SYNC_E2E_LOST: sessions["lost-device"],
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
  }, process.argv.includes("--meeting-library") ? 600000 : 300000);
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
    process.argv.includes("--meeting-library")
      ? "Two devices converged 200 synthetic meetings across bounded rate-limit windows; idle passes wrote no references or holds."
      : process.argv.includes("--meetings")
      ? "Two Rust devices converged meeting layers, resolved an edit conflict, resumed authenticated audio, and retained local edits after deletion through Miniflare; R2 held only encrypted SJV1 envelopes."
      : "Three Rust devices signed in, paired and synced through Miniflare. Removing one re-keyed the other two automatically; old keys and a forged roster were rejected. R2 held only encrypted SJV1 envelopes.",
  );
} finally {
  await mf.dispose();
}
