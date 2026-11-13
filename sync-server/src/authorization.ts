import {
  Fault,
  body,
  encoded,
  json,
  native,
  now,
  session,
  fresh,
  limit,
  token,
  unb64,
  b64,
  type Session,
} from "./common";

export const RESET_WINDOW = 7 * 86400;

// Before encryption is initialized, a fresh provider session still owns the account.
// Legacy wraps without a verifier are not an uninitialized account.
export async function authorizeAccountChange(
  request: Request,
  env: Env,
  s: Session,
): Promise<string | null> {
  const empty = await env.DB.prepare(
    "SELECT 1 FROM accounts WHERE id=? AND auth_verifier IS NULL AND NOT EXISTS(SELECT 1 FROM wraps WHERE account=accounts.id)",
  )
    .bind(s.account)
    .first();
  return empty ? null : authorizeKeys(request, env, s);
}

export async function proofHash(request: Request): Promise<string> {
  const proof = encoded(request.headers.get("X-Sync-Auth"), 32);
  return b64(
    new Uint8Array(await crypto.subtle.digest("SHA-256", unb64(proof))),
  );
}
// A provider session identifies an account; it cannot authorize writes to its keys or data.
export async function authorizeKeys(
  request: Request,
  env: Env,
  s: Session,
): Promise<string> {
  if (!request.headers.has("X-Sync-Auth"))
    throw new Fault(403, "Unlock this device before changing encrypted sync.");
  const hash = await proofHash(request);
  const row = await env.DB.prepare(
    "SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?",
  )
    .bind(s.account, hash)
    .first();
  if (!row)
    throw new Fault(
      403,
      "Encryption authorization failed. Pair or use recovery again.",
    );
  return hash;
}
export async function authorization(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const path = new URL(request.url).pathname;
  if (!["/v1/bootstrap", "/v1/reset"].includes(path)) return null;
  const s = await session(request, env);
  native(s);
  if (path === "/v1/reset") {
    if (request.method === "DELETE") {
      const verifier = await authorizeKeys(request, env, s);
      const result = await env.DB.batch([
        env.DB.prepare(
          "DELETE FROM pending_resets WHERE account=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?)",
        ).bind(s.account, s.account, verifier),
        env.DB.prepare(
          "DELETE FROM notifications WHERE account=? AND kind='reset-requested' AND NOT EXISTS(SELECT 1 FROM pending_resets WHERE account=notifications.account)",
        ).bind(s.account),
      ]);
      if (result[0].meta.changes === 0) await authorizeKeys(request, env, s);
      return json({ cancelled: result[0].meta.changes === 1 });
    }
    if (request.method !== "POST") throw new Fault(404, "Not found.");
    fresh(s);
    await limit(env, "reset:" + s.account, 3, 7 * 86400);
    const b = await body(request);
    if (b.confirmed !== true)
      throw new Fault(400, "Confirm the reset request.");
    await env.DB.batch([
      env.DB.prepare(
        "DELETE FROM notifications WHERE account=? AND kind='reset-requested' AND EXISTS(SELECT 1 FROM pending_resets WHERE account=notifications.account AND ready_at<=?)",
      ).bind(s.account, now() - RESET_WINDOW),
      env.DB.prepare(
        "DELETE FROM pending_resets WHERE account=? AND ready_at<=?",
      ).bind(s.account, now() - RESET_WINDOW),
      env.DB.prepare(
        "INSERT INTO pending_resets(account,device,requested_at,ready_at,epoch) SELECT id,?,?,?,epoch FROM accounts WHERE id=? ON CONFLICT(account) DO NOTHING",
      ).bind(s.device, now(), now() + 72 * 3600, s.account),
      env.DB.prepare(
        "INSERT INTO notifications(id,account,kind,created_at) SELECT ?,?,'reset-requested',? WHERE NOT EXISTS(SELECT 1 FROM notifications WHERE account=? AND kind='reset-requested')",
      ).bind(token(), s.account, now(), s.account),
    ]);
    return json(
      await env.DB.prepare(
        "SELECT requested_at,ready_at FROM pending_resets WHERE account=?",
      )
        .bind(s.account)
        .first(),
      202,
    );
  }
  if (request.method !== "POST") throw new Fault(404, "Not found.");
  fresh(s);
  const b = await body(request, 131072);
  const verifier = await proofHash(request);
  const ciphertext = (v: unknown) => {
    if (typeof v !== "string" || !/^[A-Za-z0-9_-]{1,44000}$/.test(v))
      throw new Fault(400, "Invalid encrypted wrap.");
    return v;
  };
  const ring = ciphertext(b.keyring),
    recovery = ciphertext(b.recovery),
    device = ciphertext(b.device);
  const operation = token();
  const guard = "EXISTS(SELECT 1 FROM accounts WHERE id=? AND rotation=?)";
  const results = await env.DB.batch([
    env.DB.prepare(
      "UPDATE accounts SET auth_verifier=?,rotation=? WHERE id=? AND auth_verifier IS NULL AND NOT EXISTS(SELECT 1 FROM wraps WHERE account=?)",
    ).bind(verifier, operation, s.account, s.account),
    ...[
      ["keyring", "main", ring],
      ["recovery", "main", recovery],
      ["device", s.device, device],
    ].map(([kind, ref, cipher]) =>
      env.DB.prepare(
        `INSERT INTO wraps(account,kind,ref,ciphertext,rev) SELECT ?,?,?,?,1 WHERE ${guard}`,
      ).bind(s.account, kind, ref, cipher, s.account, operation),
    ),
  ]);
  if (results[0].meta.changes !== 1)
    throw new Fault(
      409,
      "Sync already has keys. Pair or recover; existing keys cannot be claimed by sign-in.",
    );
  return json({ initialized: true }, 201);
}
