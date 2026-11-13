import { authorizeKeys } from "./authorization";
import {
  Fault,
  bytes,
  id,
  integer,
  json,
  limit,
  native,
  now,
  session,
} from "./common";
import { chargeUpload } from "./meeting_retention";

export async function rekeyPages(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const path = new URL(request.url).pathname;
  if (!path.startsWith("/v1/rekey-pages/")) return null;
  const s = await session(request, env);
  native(s);
  await limit(env, `checkpoint:${s.account}`, 300, 60);
  const parts = path.split("/");
  if (parts.length !== 6) throw new Fault(400, "Invalid checkpoint path.");
  const epoch = integer(Number(parts[3]), 2),
    nonce = id(parts[4]),
    page = integer(Number(parts[5]));
  if (page >= 128) throw new Fault(413, "Too many checkpoint pages.");
  const row = await env.DB.prepare(
    "SELECT body_key FROM rekey_pages WHERE account=? AND epoch=? AND nonce=? AND page=?",
  )
    .bind(s.account, epoch, nonce, page)
    .first<{ body_key: string }>();
  if (request.method === "GET") {
    if (!row) throw new Fault(404, "Checkpoint page is unavailable.");
    const stored = await env.CONTENT.get(row.body_key);
    if (!stored) throw new Fault(503, "Checkpoint page is unavailable.");
    return new Response(stored.body, {
      headers: { "Content-Type": "application/octet-stream", ETag: '"1"' },
    });
  }
  if (request.method !== "PUT") throw new Fault(404, "Not found.");
  const verifier = await authorizeKeys(request, env, s);
  if (row) throw new Fault(412, "Checkpoint pages are immutable.");
  const data = await bytes(request, 524288);
  if (
    data.length < 4096 ||
    data.length % 4096 ||
    new TextDecoder().decode(data.slice(0, 5)) !== "SJV1\x01"
  )
    throw new Fault(400, "Expected an encrypted checkpoint.");
  await chargeUpload(env, s.account, data.length);
  const available = await env.DB.prepare(
    "SELECT 1 FROM accounts WHERE id=? AND epoch=? AND auth_verifier=? AND COALESCE((SELECT SUM(size) FROM rekey_pages WHERE account=?),0)+?<=134217728",
  )
    .bind(s.account, epoch - 1, verifier, s.account, data.length)
    .first();
  if (!available)
    throw new Fault(409, "Keys changed or checkpoint staging storage is full.");
  const key = s.account + "/" + crypto.randomUUID();
  await env.CONTENT.put(key, data);
  try {
    const result = await env.DB.prepare(
      "INSERT INTO rekey_pages(account,epoch,nonce,page,body_key,size,expires_at) SELECT ?,?,?,?,?,?,? WHERE EXISTS(SELECT 1 FROM accounts WHERE id=? AND epoch=? AND auth_verifier=?) AND COALESCE((SELECT SUM(size) FROM rekey_pages WHERE account=?),0)+?<=134217728 ON CONFLICT DO NOTHING RETURNING page",
    )
      .bind(
        s.account,
        epoch,
        nonce,
        page,
        key,
        data.length,
        now() + 86400,
        s.account,
        epoch - 1,
        verifier,
        s.account,
        data.length,
      )
      .first();
    if (!result)
      throw new Fault(
        409,
        "Keys changed or checkpoint staging storage is full.",
      );
    return json({ rev: 1 });
  } catch (error) {
    await env.CONTENT.delete(key);
    throw error;
  }
}

export async function rekeyPageMaintenance(env: Env): Promise<void> {
  const expired = (
    await env.DB.prepare(
      "SELECT account,epoch,nonce,page,body_key FROM rekey_pages WHERE expires_at>0 AND expires_at<=? LIMIT 500",
    )
      .bind(now())
      .all<{
        account: string;
        epoch: number;
        nonce: string;
        page: number;
        body_key: string;
      }>()
  ).results;
  for (const row of expired) {
    // Claim and queue body cleanup atomically. Failed R2 deletion remains
    // retryable; a completed rotation pins pages in its own DB transaction.
    const binding = [row.account, row.epoch, row.nonce, row.page, now()];
    await env.DB.batch([
      env.DB.prepare(
        "INSERT OR IGNORE INTO trash SELECT body_key,account,? FROM rekey_pages WHERE account=? AND epoch=? AND nonce=? AND page=? AND expires_at>0 AND expires_at<=?",
      ).bind(now(), ...binding),
      env.DB.prepare(
        "DELETE FROM rekey_pages WHERE account=? AND epoch=? AND nonce=? AND page=? AND expires_at>0 AND expires_at<=?",
      ).bind(...binding),
    ]);
  }
}
