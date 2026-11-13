import { Fault, body, id, integer, json, now } from "./common";

/** A complete, version-bound reachability list; partial pages never authorize GC. */
export async function references(
  request: Request,
  env: Env,
  account: string,
  meeting: string,
  verifier: string,
): Promise<Response> {
  const b = await body(request, 100000);
  const token = id(b.token),
    version = integer(b.version),
    head = integer(b.headRevision, 1);
  if (!Array.isArray(b.objects) || b.objects.length > 250)
    throw new Fault(400, "Invalid reference page.");
  const objects = b.objects.map((value: unknown) => {
    if (
      typeof value !== "string" ||
      value.length > 256 ||
      !/^[-\w/]+$/.test(value) ||
      !(
        value.startsWith(`m/${meeting}/`) ||
        value.startsWith(`blob/${meeting}/`)
      )
    )
      throw new Fault(400, "Invalid meeting reference.");
    return value;
  });
  const guard =
    "EXISTS(SELECT 1 FROM meetings m JOIN objects h ON h.account=m.account AND h.id='m/'||m.id||'/head' JOIN accounts a ON a.id=m.account WHERE m.account=? AND m.id=? AND m.state='active' AND m.version=? AND h.rev=? AND h.deleted=0 AND a.auth_verifier=?)";
  const bindings = [account, meeting, version, head, verifier];
  const run = await env.DB.prepare(
    `INSERT INTO meeting_ref_runs VALUES(?,?,?,?,?,?) ON CONFLICT(account,meeting,token) DO UPDATE SET created_at=excluded.created_at WHERE version=excluded.version AND head_rev=excluded.head_rev RETURNING token`,
  )
    .bind(account, meeting, token, version, head, now())
    .first();
  if (!run) throw new Fault(412, "Reference snapshot changed.");
  if (objects.length)
    await env.DB.batch(
      objects.map((object) =>
        env.DB.prepare(
          `INSERT OR IGNORE INTO meeting_refs SELECT ?,?,?,? WHERE ${guard}`,
        ).bind(account, meeting, token, object, ...bindings),
      ),
    );
  if (b.complete !== true) return json({ staged: true });
  const live =
    "SELECT object FROM meeting_refs WHERE account=? AND meeting=? AND token=?";
  const refBindings = [account, meeting, token];
  const committed =
    "EXISTS(SELECT 1 FROM meetings WHERE account=? AND id=? AND refs_token=?)";
  const result = await env.DB.batch([
    env.DB.prepare(
      `UPDATE meetings SET refs_version=?,refs_head=?,refs_token=? WHERE account=? AND id=? AND ${guard} AND EXISTS(SELECT 1 FROM meeting_refs WHERE account=? AND meeting=? AND token=? AND object=?) AND NOT EXISTS(SELECT 1 FROM meeting_refs r LEFT JOIN objects o ON o.account=r.account AND o.id=r.object WHERE r.account=? AND r.meeting=? AND r.token=? AND (o.id IS NULL OR o.deleted!=0))`,
    ).bind(
      version,
      head,
      token,
      account,
      meeting,
      ...bindings,
      ...refBindings,
      `m/${meeting}/head`,
      ...refBindings,
    ),
    env.DB.prepare(
      `DELETE FROM meeting_gc WHERE account=? AND meeting=? AND object IN(${live}) AND ${guard} AND ${committed}`,
    ).bind(account, meeting, ...refBindings, ...bindings, ...refBindings),
    env.DB.prepare(
      `INSERT INTO meeting_gc SELECT account,meeting,id,?+CASE WHEN id LIKE 'blob/%' THEN 86400 ELSE 2592000 END FROM objects WHERE account=? AND meeting=? AND deleted=0 AND id NOT IN(${live}) AND id!=? AND ${guard} AND ${committed} ON CONFLICT(account,object) DO NOTHING`,
    ).bind(
      now(),
      account,
      meeting,
      ...refBindings,
      `m/${meeting}/head`,
      ...bindings,
      ...refBindings,
    ),
    env.DB.prepare(
      `DELETE FROM meeting_live_refs WHERE account=? AND meeting=? AND ${committed}`,
    ).bind(account, meeting, ...refBindings),
    env.DB.prepare(
      `INSERT INTO meeting_live_refs SELECT account,meeting,object FROM meeting_refs WHERE account=? AND meeting=? AND token=? AND ${committed}`,
    ).bind(...refBindings, ...refBindings),
  ]);
  if (result[0].meta.changes !== 1)
    throw new Fault(412, "Meeting changed before reference commit.");
  await env.DB.prepare(
    "DELETE FROM meeting_ref_runs WHERE account=? AND meeting=? AND token=?",
  )
    .bind(account, meeting, token)
    .run();
  return json({ retained: true });
}

export async function retainMaintenance(env: Env): Promise<void> {
  // MATERIALIZED freezes the set before update triggers bump meeting versions.
  await env.DB.prepare(
    `WITH doomed AS MATERIALIZED (
    SELECT o.account,o.id FROM objects o JOIN meetings m ON m.account=o.account AND m.id=o.meeting
    LEFT JOIN meeting_gc g ON g.account=o.account AND g.object=o.id
    WHERE o.deleted=0 AND m.state='active' AND (
      (m.refs_version=m.version AND g.purge_at<=? AND EXISTS(SELECT 1 FROM objects h WHERE h.account=m.account AND h.id='m/'||m.id||'/head' AND h.rev=m.refs_head AND h.deleted=0))
      OR (o.id LIKE 'blob/%' AND o.updated_at<=? AND (
        NOT EXISTS(SELECT 1 FROM objects h WHERE h.account=m.account AND h.id='m/'||m.id||'/head')
        OR (EXISTS(SELECT 1 FROM objects h WHERE h.account=m.account AND h.id='m/'||m.id||'/head' AND h.rev=m.refs_head AND h.deleted=0)
            AND NOT EXISTS(SELECT 1 FROM meeting_live_refs r WHERE r.account=o.account AND r.meeting=o.meeting AND r.object=o.id))
      ))
    )) UPDATE objects SET deleted=CASE WHEN id LIKE 'blob/%' OR id LIKE 'm/%/part/%' THEN 4 ELSE 3 END,size=0,body_key='',rev=rev+1,updated_at=? WHERE (account,id) IN (SELECT account,id FROM doomed)`,
  )
    .bind(now(), now() - 86400, now())
    .run();
  await env.DB.prepare("DELETE FROM meeting_ref_runs WHERE created_at<?")
    .bind(now() - 86400)
    .run();
}

export async function chargeUpload(
  env: Env,
  account: string,
  bytes: number,
): Promise<void> {
  const day = Math.floor(now() / 86400),
    budget = 2_000_000_000;
  const row = await env.DB.prepare(
    "INSERT INTO meeting_upload_bytes VALUES(?,?,?) ON CONFLICT(account) DO UPDATE SET day=excluded.day,bytes=CASE WHEN day=excluded.day THEN bytes+excluded.bytes ELSE excluded.bytes END WHERE CASE WHEN day=excluded.day THEN bytes ELSE 0 END+excluded.bytes<=? RETURNING bytes",
  )
    .bind(account, day, bytes, budget)
    .first();
  if (!row || bytes > budget)
    throw new Fault(
      429,
      "Today's staging audio/text upload budget is used. Try tomorrow.",
    );
}
