import { authorizeKeys } from "./authorization";
import {
  Fault,
  body,
  id,
  encoded,
  str,
  json,
  now,
  session,
  native,
  limit,
  unb64,
  b64,
} from "./common";
async function commitment(
  account: string,
  requestId: string,
  device: string,
  pk: string,
  b: string,
): Promise<string> {
  const parts = [
    new TextEncoder().encode("pair-receiver/v2"),
    new TextEncoder().encode(account),
    new TextEncoder().encode(requestId),
    new TextEncoder().encode(device),
    unb64(pk),
    unb64(b),
  ];
  const all = new Uint8Array(parts.reduce((n, p) => n + 4 + p.length, 0));
  let at = 0;
  for (const p of parts) {
    new DataView(all.buffer).setUint32(at, p.length);
    all.set(p, at + 4);
    at += 4 + p.length;
  }
  return b64(new Uint8Array(await crypto.subtle.digest("SHA-256", all)));
}
type Pair = {
  id: string;
  account: string;
  device: string;
  public_key: string;
  commitment: string;
  approver: string | null;
  nonce_a: string | null;
  public_key_a: string | null;
  commitment_a: string | null;
  nonce_b: string | null;
  state: string;
  ciphertext: string | null;
  expires_at: number;
};
export async function pairing(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const u = new URL(request.url);
  if (!u.pathname.startsWith("/v1/pairings")) return null;
  const s = await session(request, env);
  native(s);
  await limit(env, "pair-read:" + s.account, 120, 60);
  if (u.pathname === "/v1/pairings" && request.method === "GET")
    return json(
      (
        await env.DB.prepare(
          "SELECT p.*,d.platform FROM pairings p JOIN devices d ON d.account=p.account AND d.id=p.device WHERE p.account=? AND p.expires_at>? AND p.state NOT IN ('denied','consumed')",
        )
          .bind(s.account, now())
          .all()
      ).results,
    );
  if (u.pathname === "/v1/pairings" && request.method === "POST") {
    await limit(env, "pair:" + s.account, 5);
    const b = await body(request);
    const requestId = id(b.id);
    const c = encoded(b.commitment, 32);
    await env.DB.prepare(
      "INSERT INTO pairings(id,account,device,public_key,commitment,expires_at,created_at,location) VALUES(?,?,?,?,?,?,?,?)",
    )
      .bind(
        requestId,
        s.account,
        s.device,
        s.public_key,
        c,
        now() + 300,
        now(),
        String(request.cf?.country ?? "Unknown"),
      )
      .run();
    return json({ id: requestId, expiresAt: now() + 300 }, 201);
  }
  const requestId = id(u.pathname.slice("/v1/pairings/".length));
  const p = await env.DB.prepare(
    "SELECT * FROM pairings WHERE id=? AND account=? AND expires_at>?",
  )
    .bind(requestId, s.account, now())
    .first<Pair>();
  if (!p) throw new Fault(404, "Pairing expired. Start again.");
  if (request.method === "GET") return json(p);
  if (request.method !== "POST") throw new Fault(404, "Not found.");
  const b = await body(request, 700000);
  let query: D1PreparedStatement;
  if (b.action === "accept" && p.device !== s.device) {
    await authorizeKeys(request, env, s);
    const has = await env.DB.prepare(
      "SELECT 1 FROM wraps WHERE account=? AND kind='device' AND ref=?",
    )
      .bind(s.account, s.device)
      .first();
    if (!has)
      throw new Fault(403, "Unlock this device before approving another.");
    query = env.DB.prepare(
      "UPDATE pairings SET approver=?,public_key_a=?,commitment_a=?,state='accepted' WHERE id=? AND state='committed'",
    ).bind(s.device, s.public_key, encoded(b.commitment, 32), requestId);
  } else if (b.action === "reveal" && p.device === s.device) {
    const nonce = encoded(b.nonce, 32);
    if (
      (await commitment(
        s.account,
        requestId,
        p.device,
        p.public_key,
        nonce,
      )) !== p.commitment
    )
      throw new Fault(400, "Pairing commitment changed.");
    query = env.DB.prepare(
      "UPDATE pairings SET nonce_b=?,state='revealed' WHERE id=? AND state='accepted'",
    ).bind(nonce, requestId);
  } else if (b.action === "reveal_approver" && p.approver === s.device) {
    await authorizeKeys(request, env, s);
    query = env.DB.prepare(
      "UPDATE pairings SET nonce_a=?,state='ready' WHERE id=? AND state='revealed'",
    ).bind(encoded(b.nonce, 32), requestId);
  } else if (
    b.action === "approve" &&
    p.approver === s.device &&
    b.confirmed === true
  ) {
    await authorizeKeys(request, env, s);
    const ciphertext = str(b.ciphertext, 700000);
    if (!/^[A-Za-z0-9_-]+$/.test(ciphertext))
      throw new Fault(400, "Invalid wrapped key.");
    query = env.DB.prepare(
      "UPDATE pairings SET ciphertext=?,state='approved' WHERE id=? AND state='ready'",
    ).bind(ciphertext, requestId);
  } else if (b.action === "deny")
    query = env.DB.prepare(
      "UPDATE pairings SET state='denied',ciphertext=NULL WHERE id=? AND state NOT IN ('denied','consumed')",
    ).bind(requestId);
  else if (b.action === "consume" && p.device === s.device)
    query = env.DB.prepare(
      "UPDATE pairings SET state='consumed',ciphertext=NULL WHERE id=? AND state='approved'",
    ).bind(requestId);
  else throw new Fault(409, "Pairing step is not allowed.");
  const r = await query.run();
  if (r.meta.changes !== 1)
    throw new Fault(409, "Pairing already changed. Start again.");
  return json({ updated: true });
}
