import {
  authorizeKeys,
  authorizeAccountChange,
  RESET_WINDOW,
} from "./authorization";
import {
  Fault,
  bytes,
  body,
  id,
  str,
  integer,
  json,
  now,
  session,
  native,
  fresh,
  limit,
} from "./common";
type ObjectRow = {
  account: string;
  id: string;
  rev: number;
  size: number;
  body_key: string;
  deleted: number;
  updated_at: number;
};
function objectId(s: string): string {
  let v: string;
  try {
    v = decodeURIComponent(s);
  } catch {
    throw new Fault(400, "Invalid object id encoding.");
  }
  if (v.length > 256 || !/^[-a-zA-Z0-9_/]+$/.test(v) || v.includes(".."))
    throw new Fault(400, "Invalid object id.");
  return v;
}
function envelope(data: Uint8Array): void {
  if (
    data.length < 4096 ||
    data.length % 4096 !== 0 ||
    new TextDecoder().decode(data.slice(0, 5)) !== "SJV1\x01"
  )
    throw new Fault(400, "Expected an encrypted SJV1 envelope.");
}
export async function store(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const u = new URL(request.url);
  const path = u.pathname;
  if (
    !/^\/v1\/(account|changes|objects|blobs|holds|poll|heartbeat)(\/|$)/.test(
      path,
    )
  )
    return null;
  const s = await session(request, env);
  if (path === "/v1/heartbeat" && request.method === "POST")
    return json({ renewed: true });
  if (path === "/v1/poll" && request.method === "GET") {
    native(s);
    return json({
      ...(await env.DB.prepare(
        "SELECT a.seq,a.epoch,a.changes_floor,COALESCE(w.rev,0) AS ring_rev,r.requested_at,r.ready_at FROM accounts a LEFT JOIN wraps w ON w.account=a.id AND w.kind='keyring' AND w.ref='main' LEFT JOIN pending_resets r ON r.account=a.id AND r.ready_at>? WHERE a.id=?",
      )
        .bind(now() - RESET_WINDOW, s.account)
        .first()),
      refreshAfter: s.expires_at - 89 * 86400,
    });
  }
  const verifier =
    request.method === "GET"
      ? null
      : path === "/v1/account" && request.method === "DELETE"
        ? await authorizeAccountChange(request, env, s)
        : await authorizeKeys(request, env, s);
  await limit(
    env,
    (request.method === "GET" ? "read:" : "write:") + s.account,
    request.method === "GET" ? 300 : 60,
    60,
  );
  const account = await env.DB.prepare(
    "SELECT id,email,created_at,bytes_used,quota,epoch,seq,changes_floor,(auth_verifier IS NOT NULL OR EXISTS(SELECT 1 FROM wraps WHERE account=accounts.id)) AS encrypted FROM accounts WHERE id=?",
  )
    .bind(s.account)
    .first<{
      id: string;
      email: string;
      created_at: number;
      bytes_used: number;
      quota: number;
      epoch: number;
      seq: number;
      changes_floor: number;
    }>();
  if (!account) throw new Fault(401, "Account is unavailable.");
  if (path === "/v1/account" && request.method === "GET")
    return json({ ...account, plan: "free" });
  if (path === "/v1/account/export" && request.method === "GET") {
    fresh(s);
    const rows = await env.DB.batch(
      ["identities", "devices", "objects"].map((table) =>
        env.DB.prepare(`SELECT * FROM ${table} WHERE account=?`).bind(
          s.account,
        ),
      ),
    );
    const sessions = (
      await env.DB.prepare(
        "SELECT device,created_at,expires_at,fresh_at FROM sessions WHERE account=?",
      )
        .bind(s.account)
        .all()
    ).results;
    const r = json({
      schema: 1,
      account,
      identities: rows[0].results,
      devices: rows[1].results,
      sessions,
      objects: rows[2].results,
    });
    r.headers.set(
      "Content-Disposition",
      'attachment; filename="sonojot-account.json"',
    );
    return r;
  }
  if (path === "/v1/account" && request.method === "DELETE") {
    fresh(s);
    const b = await body(request);
    if (b.confirmed !== true) throw new Fault(400, "Confirm account deletion.");
    const erased = await env.DB.batch([
      env.DB.prepare(
        "UPDATE accounts SET deleted_at=? WHERE id=? AND auth_verifier IS ? AND (auth_verifier IS NOT NULL OR NOT EXISTS(SELECT 1 FROM wraps WHERE account=accounts.id)) AND deleted_at IS NULL",
      ).bind(now(), s.account, verifier),
      env.DB.prepare(
        "DELETE FROM sessions WHERE account=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND deleted_at IS NOT NULL)",
      ).bind(s.account, s.account),
    ]);
    if (erased[0].meta.changes !== 1)
      throw new Fault(
        409,
        "Encryption authorization changed. Unlock and retry.",
      );
    await eraseAccount(env, s.account);
    return json({ deleted: true });
  }
  native(s);
  if (path === "/v1/changes" && request.method === "GET") {
    const since = Number(u.searchParams.get("since") ?? 0);
    integer(since);
    if (since > account.seq)
      throw new Fault(
        409,
        "Change cursor is ahead of the service. Restore requires review.",
      );
    if (since < account.changes_floor)
      return json({
        fullResync: true,
        cursor: account.seq,
        epoch: account.epoch,
        changes: [],
      });
    const rows = (
      await env.DB.prepare(
        "SELECT seq,id,rev,deleted FROM changes WHERE account=? AND seq>? ORDER BY seq LIMIT 250",
      )
        .bind(s.account, since)
        .all<{ seq: number; id: string; rev: number; deleted: number }>()
    ).results;
    return json({
      changes: rows,
      cursor: rows.at(-1)?.seq ?? since,
      hasMore: rows.length === 250,
      epoch: account.epoch,
    });
  }
  if (path === "/v1/holds" && request.method === "POST") {
    const b = await body(request);
    const object = objectId(str(b.object));
    const rev = integer(b.rev, 1);
    if (b.complete !== true)
      throw new Fault(400, "A hold requires a complete local copy.");
    const r = await env.DB.prepare(
      "INSERT INTO holds(account,object,device,rev,created_at) SELECT ?,?,?,?,? WHERE EXISTS(SELECT 1 FROM objects WHERE account=? AND id=? AND rev=? AND deleted=0) ON CONFLICT(account,object,device) DO UPDATE SET rev=excluded.rev,created_at=excluded.created_at RETURNING rev",
    )
      .bind(s.account, object, s.device, rev, now(), s.account, object, rev)
      .first();
    if (!r) throw new Fault(412, "Object changed before the hold.");
    return json({ held: true });
  }
  const blob = path.startsWith("/v1/blobs/");
  if (!blob && !path.startsWith("/v1/objects/"))
    throw new Fault(404, "Not found.");
  const object = (blob ? "blob/" : "") + objectId(path.slice(blob ? 10 : 12));
  const row = await env.DB.prepare(
    "SELECT * FROM objects WHERE account=? AND id=?",
  )
    .bind(s.account, object)
    .first<ObjectRow>();
  if (request.method === "GET") {
    if (!row || row.deleted) throw new Fault(404, "Object not found.");
    const stored = await env.CONTENT.get(row.body_key);
    if (!stored)
      throw new Fault(503, "Stored object is temporarily unavailable.");
    return new Response(stored.body, {
      headers: {
        "Content-Type": "application/octet-stream",
        ETag: `"${row.rev}"`,
        "X-Sync-Epoch": String(account.epoch),
        "Content-Length": String(row.size),
      },
    });
  }
  if (request.method !== "PUT" && request.method !== "DELETE")
    throw new Fault(404, "Not found.");
  if (Number(request.headers.get("x-sync-epoch")) !== account.epoch)
    throw new Fault(409, "Encryption keys changed. Unlock this device again.");
  const match = request.headers.get("if-match");
  if (!match || !/^"\d+"$/.test(match))
    throw new Fault(428, "A revision is required.");
  const previous = integer(Number(match.slice(1, -1)));
  if ((row?.rev ?? 0) !== previous || (blob && row))
    throw new Fault(412, "Object changed. Merge and retry.");
  if (request.method === "DELETE" && !row)
    throw new Fault(404, "Object not found.");
  const data =
    request.method === "DELETE"
      ? new Uint8Array()
      : await bytes(request, blob ? 8 * 1024 * 1024 + 16384 : 524288);
  if (request.method === "PUT") envelope(data);
  const key = s.account + "/" + crypto.randomUUID();
  if (data.length)
    await env.CONTENT.put(key, data, {
      httpMetadata: { contentType: "application/octet-stream" },
    });
  try {
    const params = [
      data.length,
      key,
      request.method === "DELETE" ? 1 : 0,
      now(),
      s.account,
      object,
      previous,
    ];
    const written =
      previous === 0
        ? await env.DB.prepare(
            "INSERT INTO objects(account,id,rev,size,body_key,deleted,updated_at) SELECT ?,?,1,?,?,0,? WHERE EXISTS(SELECT 1 FROM accounts WHERE id=? AND epoch=? AND auth_verifier=?) ON CONFLICT(account,id) DO NOTHING RETURNING rev",
          )
            .bind(
              s.account,
              object,
              data.length,
              key,
              now(),
              s.account,
              account.epoch,
              verifier,
            )
            .first<{ rev: number }>()
        : await env.DB.prepare(
            "UPDATE objects SET rev=rev+1,size=?,body_key=?,deleted=?,updated_at=? WHERE account=? AND id=? AND rev=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND epoch=? AND auth_verifier=?) RETURNING rev",
          )
            .bind(...params, s.account, account.epoch, verifier)
            .first<{ rev: number }>();
    if (!written) throw new Fault(412, "Object changed. Merge and retry.");
    return json(written);
  } catch (e) {
    await env.CONTENT.delete(key);
    if (e instanceof Fault) throw e;
    if (String(e).includes("quota"))
      throw new Fault(
        413,
        "The free vocabulary allowance is 512 KiB. Your local words are unchanged.",
      );
    throw e;
  }
}
export async function eraseAccount(env: Env, account: string): Promise<void> {
  // An erasure marker outlives retained snapshots, so restore cannot resurrect an account.
  await env.BACKUPS.put("erased/" + account, String(now()));
  // Revoke before removing bodies; cron resumes a failed deletion. No plaintext is exported.
  let cursor: string | undefined;
  do {
    const page = await env.CONTENT.list({
      prefix: account + "/",
      cursor,
      limit: 500,
    });
    if (page.objects.length)
      await env.CONTENT.delete(page.objects.map((o) => o.key));
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  await env.DB.batch([
    env.DB.prepare("DELETE FROM trash WHERE account=?").bind(account),
    env.DB.prepare("DELETE FROM auth_requests WHERE account=?").bind(account),
    env.DB.prepare("DELETE FROM accounts WHERE id=?").bind(account),
  ]);
}
