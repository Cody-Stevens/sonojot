import {
  body,
  encoded,
  Fault,
  id,
  json,
  limit,
  now,
  sha,
  str,
  token,
} from "./common";

/** Staging automation authenticates synthetic identities, never WorkOS users. */
export async function testIdentity(
  request: Request,
  env: Env,
): Promise<Response | null> {
  if (new URL(request.url).pathname !== "/v1/auth/test-session") return null;
  if (
    env.ENVIRONMENT !== "staging" ||
    env.SYNC_TEST_IDENTITY !== "true" ||
    request.method !== "POST"
  ) {
    throw new Fault(404, "Not found.");
  }
  const ip = request.headers.get("cf-connecting-ip") ?? "local";
  await limit(env, "test-signin-ip:" + (await sha(ip)), 30);
  // Hash to equal-size buffers before the runtime's constant-time comparison.
  // Missing configuration fails closed and never falls through to real sign-in.
  const supplied = request.headers.get("X-Sync-Test-Token") ?? "";
  const digest = (value: string) =>
    crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  const [actual, expected] = await Promise.all([
    digest(supplied),
    digest(env.SYNC_TEST_TOKEN ?? ""),
  ]);
  if (
    !env.SYNC_TEST_TOKEN ||
    !crypto.subtle.timingSafeEqual(actual, expected)
  ) {
    throw new Fault(401, "Test sign-in is not authorized.");
  }
  const b = await body(request, 4096);
  const subject = str(b.subject, 80);
  if (!/^synctest-[a-z0-9][a-z0-9-]{0,63}$/.test(subject)) {
    throw new Fault(400, "Use a namespaced test identity.");
  }
  const device = id(b.device);
  const publicKey = encoded(b.publicKey, 32);
  const platform = str(b.platform, 32);
  await limit(env, "test-signin-subject:" + (await sha(subject)), 10);
  const candidate = crypto.randomUUID();
  const t = now();
  await env.DB.batch([
    env.DB.prepare(
      "INSERT INTO accounts(id,email,created_at,last_seen_at,is_test) SELECT ?,?,?,?,1 WHERE NOT EXISTS(SELECT 1 FROM identities WHERE provider='synctest' AND subject=?)",
    ).bind(candidate, subject + "@example.invalid", t, t, subject),
    env.DB.prepare(
      "INSERT OR IGNORE INTO identities(provider,subject,account) SELECT 'synctest',?,id FROM accounts WHERE id=? AND is_test=1",
    ).bind(subject, candidate),
    env.DB.prepare(
      "DELETE FROM accounts WHERE id=? AND NOT EXISTS(SELECT 1 FROM identities WHERE account=?)",
    ).bind(candidate, candidate),
  ]);
  const account = await env.DB.prepare(
    "SELECT a.id,a.email,a.epoch FROM accounts a JOIN identities i ON i.account=a.id WHERE i.provider='synctest' AND i.subject=? AND a.is_test=1 AND a.deleted_at IS NULL",
  )
    .bind(subject)
    .first<{ id: string; email: string; epoch: number }>();
  if (!account) throw new Fault(403, "Test identity is unavailable.");
  const raw = token();
  // A reused or revoked device id cannot replace a public key or restore access.
  const result = await env.DB.batch([
    env.DB.prepare(
      "INSERT INTO devices(id,account,platform,public_key,created_at,last_seen_at) SELECT ?,?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM devices WHERE account=? AND id=?) AND (SELECT COUNT(*) FROM devices WHERE account=? AND revoked_at IS NULL AND public_key<>'')<64",
    ).bind(
      device,
      account.id,
      platform,
      publicKey,
      t,
      t,
      account.id,
      device,
      account.id,
    ),
    env.DB.prepare(
      "INSERT INTO sessions(hash,account,device,created_at,expires_at,fresh_at) SELECT ?,account,id,?,?,? FROM devices WHERE account=? AND id=? AND public_key=? AND revoked_at IS NULL",
    ).bind(await sha(raw), t, t + 90 * 86400, t, account.id, device, publicKey),
  ]);
  if (result[1].meta.changes !== 1)
    throw new Fault(403, "Use a new test device identity.");
  return json({ account, device, token: raw });
}
