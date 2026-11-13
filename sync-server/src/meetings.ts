import { authorizeKeys } from "./authorization";
import { references, retainMaintenance } from "./meeting_retention";
import {
  Fault,
  body,
  id,
  integer,
  json,
  limit,
  native,
  now,
  session,
} from "./common";

export const MEETING_QUOTA = 100_000_000_000;
/** Billing replaces this function. Meeting writes remain staging-only until then. */
export function meetingEntitlement(env: Env): boolean {
  return env.ENVIRONMENT === "staging";
}
export async function reconcileMeetingQuota(
  env: Env,
  account: string,
): Promise<void> {
  const allowed = (env.MEETING_FULL_ALLOWLIST ?? "")
    .split(",")
    .map((value) => value.trim())
    .includes(account);
  await env.DB.prepare(
    "UPDATE accounts SET meeting_quota=CASE WHEN is_test=1 OR ? THEN 100000000000 ELSE 2000000000 END WHERE id=? AND meeting_quota IN(2000000000,100000000000)",
  )
    .bind(allowed ? 1 : 0, account)
    .run();
}
export type MeetingRow = {
  id: string;
  created_at: number;
  state: "active" | "deleted" | "dropped";
  version: number;
  updated_at: number;
};
export function meetingObject(
  object: string,
): { meeting: string; immutable: boolean } | null {
  const match = /^(?:m|blob)\/([A-Za-z0-9_-]+)\/(.+)$/.exec(object);
  if (!match) {
    if (object.startsWith("m/") || object.startsWith("blob/"))
      throw new Fault(400, "Invalid meeting object.");
    return null;
  }
  const suffix = match[2];
  if (
    object.startsWith("m/") &&
    !/^(?:head|raw\/[-\w]+|clean\/[-\w]+|ops\/[-\w]+\/[-\w]+|snap\/[-\w]+\/[-\w]+|notes\/[-\w]+|part\/[-\w]+)$/.test(
      suffix,
    )
  )
    throw new Fault(400, "Invalid meeting layer.");
  return {
    meeting: match[1],
    immutable:
      object.startsWith("blob/") ||
      !(suffix === "head" || suffix.startsWith("notes/")),
  };
}

// Rechecked inside the D1 transaction that drops a meeting. Revoked or expired
// sessions cannot authorize eviction; any layer/blob write invalidates the hold.
const validHold = `EXISTS(SELECT 1 FROM holds h JOIN objects o ON o.account=h.account AND o.id=h.object JOIN devices d ON d.account=h.account AND d.id=h.device WHERE h.account=meetings.account AND h.object='m/'||meetings.id||'/head' AND h.rev=o.rev AND o.deleted=0 AND h.version=meetings.version AND h.created_at>unixepoch()-2592000 AND d.last_seen_at>unixepoch()-2592000 AND d.revoked_at IS NULL AND EXISTS(SELECT 1 FROM sessions s WHERE s.account=h.account AND s.device=h.device AND s.expires_at>? AND s.created_at>?))`;

export async function makeRoom(
  env: Env,
  account: string,
  incoming: string,
  growth: number,
  verifier: string,
  epoch: number,
): Promise<{ statements: D1PreparedStatement[]; droppedIncoming: boolean }> {
  const plan = {
    statements: [] as D1PreparedStatement[],
    droppedIncoming: false,
  };
  if (growth <= 0) return plan;
  const usage = await env.DB.prepare(
    "SELECT meeting_quota AS quota,COALESCE((SELECT SUM(size) FROM objects WHERE account=? AND meeting IS NOT NULL),0) AS used FROM accounts WHERE id=?",
  )
    .bind(account, account)
    .first<{ quota: number; used: number }>();
  if (!usage) throw new Fault(401, "Account unavailable.");
  if (growth > usage.quota)
    throw new Fault(413, "This upload exceeds meeting storage.");
  let needed = usage.used + growth - usage.quota;
  if (needed <= 0) return plan;
  const candidates = (
    await env.DB.prepare(
      `SELECT id,version,(SELECT COALESCE(SUM(size),0) FROM objects WHERE objects.account=meetings.account AND meeting=meetings.id) AS bytes FROM meetings WHERE account=? AND state='active' AND ${validHold} ORDER BY created_at,id LIMIT 100`,
    )
      .bind(account, now(), now() - 365 * 86400)
      .all<{ id: string; version: number; bytes: number }>()
  ).results;
  for (const candidate of candidates) {
    if (!candidate.bytes) continue;
    plan.statements.push(
      env.DB.prepare(
        `UPDATE meetings SET state='dropped',updated_at=? WHERE account=? AND id=? AND version=? AND state='active' AND ${validHold} AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=? AND epoch=?)`,
      ).bind(
        now(),
        account,
        candidate.id,
        candidate.version,
        now(),
        now() - 365 * 86400,
        account,
        verifier,
        epoch,
      ),
      env.DB.prepare(
        "UPDATE objects SET deleted=2,size=0,body_key='',rev=rev+1,updated_at=? WHERE account=? AND meeting=? AND deleted=0 AND EXISTS(SELECT 1 FROM meetings WHERE account=? AND id=? AND state='dropped')",
      ).bind(now(), account, candidate.id, account, candidate.id),
      env.DB.prepare(
        "UPDATE meeting_writes SET previous=-1 WHERE account=? AND NOT EXISTS(SELECT 1 FROM meetings WHERE account=? AND id=? AND state='dropped')",
      ).bind(account, account, candidate.id),
    );
    needed -= candidate.bytes;
    plan.droppedIncoming ||= candidate.id === incoming;
    if (needed <= 0 || plan.droppedIncoming) return plan;
  }
  throw new Fault(
    507,
    "Storage is full. Keep a complete local copy of an older meeting, including synced audio, then retry.",
  );
}

export async function meetings(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const url = new URL(request.url);
  if (!url.pathname.startsWith("/v1/meetings")) return null;
  const s = await session(request, env);
  native(s);
  const write = request.method !== "GET";
  const verifier = write ? await authorizeKeys(request, env, s) : null;
  if (!meetingEntitlement(env))
    throw new Fault(402, "Meeting sync is available in staging only.");
  if (write) await reconcileMeetingQuota(env, s.account);
  await limit(
    env,
    (write ? "write:" : "read:") + s.account,
    write ? 60 : 300,
    60,
  );
  if (url.pathname === "/v1/meetings" && request.method === "GET") {
    const after = url.searchParams.get("after") ?? "";
    const rows = (
      await env.DB.prepare(
        "SELECT id,created_at,state,version,updated_at FROM meetings WHERE account=? AND id>? ORDER BY id LIMIT 250",
      )
        .bind(s.account, after)
        .all<MeetingRow>()
    ).results;
    const storage = await env.DB.prepare(
      "SELECT CASE WHEN meeting_quota NOT IN(2000000000,100000000000) THEN meeting_quota WHEN is_test=1 OR ? THEN 100000000000 ELSE 2000000000 END AS quota,COALESCE((SELECT SUM(size) FROM objects WHERE account=? AND meeting IS NOT NULL),0) AS used FROM accounts WHERE id=?",
    )
      .bind(
        (env.MEETING_FULL_ALLOWLIST ?? "")
          .split(",")
          .map((value) => value.trim())
          .includes(s.account)
          ? 1
          : 0,
        s.account,
        s.account,
      )
      .first();
    return json({
      meetings: rows,
      hasMore: rows.length === 250,
      storage,
      entitled: true,
    });
  }
  const parts = url.pathname.split("/");
  if (parts.length < 4 || parts.length > 5) throw new Fault(404, "Not found.");
  const meeting = id(parts[3]);
  const row = await env.DB.prepare(
    "SELECT id,created_at,state,version,updated_at FROM meetings WHERE account=? AND id=?",
  )
    .bind(s.account, meeting)
    .first<MeetingRow>();
  if (parts.length === 4 && request.method === "POST") {
    const b = await body(request);
    const created = Math.floor(integer(b.createdAt) / 86400) * 86400;
    if (row && row.state !== "active")
      throw new Fault(410, "This meeting no longer syncs.");
    await env.DB.prepare(
      "INSERT INTO meetings(account,id,created_at,updated_at) SELECT ?,?,?,? WHERE EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?) ON CONFLICT(account,id) DO NOTHING",
    )
      .bind(s.account, meeting, created, now(), s.account, verifier)
      .run();
    return json({ registered: true });
  }
  if (!row) throw new Fault(404, "Meeting not found.");
  if (parts[4] === "references" && request.method === "POST")
    return references(request, env, s.account, meeting, verifier!);
  if (parts.length === 4 && request.method === "GET") {
    const after = url.searchParams.get("after") ?? "";
    const objects = (
      await env.DB.prepare(
        "SELECT id,rev,size,deleted FROM objects WHERE account=? AND meeting=? AND id>? ORDER BY id LIMIT 250",
      )
        .bind(s.account, meeting, after)
        .all()
    ).results;
    return json({ ...row, objects, hasMore: objects.length === 250 });
  }
  if (parts.length === 4 && request.method === "DELETE") {
    const b = await body(request);
    if (b.confirmed !== true) throw new Fault(400, "Confirm meeting deletion.");
    const headRevision = integer(b.headRevision, 1);
    const deleted = await env.DB.batch([
      env.DB.prepare(
        "UPDATE meetings SET state='deleted',updated_at=? WHERE account=? AND id=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?) AND EXISTS(SELECT 1 FROM objects WHERE account=? AND id=? AND rev=? AND deleted=0)",
      ).bind(
        now(),
        s.account,
        meeting,
        s.account,
        verifier,
        s.account,
        `m/${meeting}/head`,
        headRevision,
      ),
      env.DB.prepare(
        "UPDATE objects SET deleted=1,size=0,body_key='',rev=rev+1,updated_at=? WHERE account=? AND meeting=? AND id!=? AND deleted!=1 AND EXISTS(SELECT 1 FROM meetings WHERE account=? AND id=? AND state='deleted')",
      ).bind(
        now(),
        s.account,
        meeting,
        `m/${meeting}/head`,
        s.account,
        meeting,
      ),
    ]);
    if (deleted[0].meta.changes !== 1)
      throw new Fault(
        412,
        "Publish the authenticated tombstone head before deleting.",
      );
    return json({ deleted: true });
  }
  if (parts[4] === "compact" && request.method === "POST") {
    const b = await body(request);
    const snapshot = typeof b.snapshot === "string" ? b.snapshot : "";
    if (!snapshot.startsWith(`m/${meeting}/snap/`) || !meetingObject(snapshot))
      throw new Fault(400, "Invalid snapshot.");
    if (!Array.isArray(b.folded) || b.folded.length > 250)
      throw new Fault(400, "Compaction is limited to 250 batches per request.");
    const folded = b.folded.map((v) => {
      if (
        typeof v !== "string" ||
        !v.startsWith(`m/${meeting}/ops/`) ||
        !meetingObject(v)
      )
        throw new Fault(400, "Invalid folded batch.");
      return v;
    });
    await env.DB.batch(
      folded.map((object) =>
        env.DB.prepare(
          "INSERT INTO compactions(account,meeting,object,snapshot,purge_at) SELECT ?,?,?,?,? WHERE EXISTS(SELECT 1 FROM objects WHERE account=? AND id=? AND meeting=? AND deleted=0) AND EXISTS(SELECT 1 FROM objects WHERE account=? AND id=? AND meeting=? AND deleted=0) AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?) ON CONFLICT(account,object) DO NOTHING",
        ).bind(
          s.account,
          meeting,
          object,
          snapshot,
          now() + 30 * 86400,
          s.account,
          object,
          meeting,
          s.account,
          snapshot,
          meeting,
          s.account,
          verifier,
        ),
      ),
    );
    return json({ scheduled: true });
  }
  throw new Fault(404, "Not found.");
}

export async function meetingMaintenance(env: Env): Promise<void> {
  await retainMaintenance(env);
  // Folded ops stay available for thirty days. A snapshot reference is retained
  // for the meeting's lifetime, so a stale device can always replay its own edits.
  await env.DB.prepare(
    "UPDATE objects SET deleted=3,size=0,body_key='',rev=rev+1,updated_at=? WHERE deleted=0 AND EXISTS(SELECT 1 FROM compactions c JOIN objects s ON s.account=c.account AND s.id=c.snapshot WHERE c.account=objects.account AND c.object=objects.id AND c.purge_at<=? AND s.deleted=0)",
  )
    .bind(now(), now())
    .run();
  await env.DB.prepare("DELETE FROM compactions WHERE purge_at<=?")
    .bind(now())
    .run();
  // 180-day tombstones cover devices returning after the changes feed expires.
  await env.DB.batch([
    // Authenticated deletion heads still have bodies. Queue them before removing
    // their last index entry, without resetting the meeting's retention clock.
    env.DB.prepare(
      "INSERT OR IGNORE INTO trash SELECT body_key,account,? FROM objects WHERE size>0 AND body_key!='' AND meeting IS NOT NULL AND EXISTS(SELECT 1 FROM meetings m WHERE m.account=objects.account AND m.id=objects.meeting AND m.state!='active' AND m.updated_at<?)",
    ).bind(now() + 30 * 86400, now() - 180 * 86400),
    env.DB.prepare(
      "UPDATE accounts SET bytes_used=bytes_used-COALESCE((SELECT SUM(o.size) FROM objects o JOIN meetings m ON m.account=o.account AND m.id=o.meeting WHERE o.account=accounts.id AND m.state!='active' AND m.updated_at<?),0)",
    ).bind(now() - 180 * 86400),
    env.DB.prepare(
      "DELETE FROM objects WHERE meeting IS NOT NULL AND EXISTS(SELECT 1 FROM meetings m WHERE m.account=objects.account AND m.id=objects.meeting AND m.state!='active' AND m.updated_at<?)",
    ).bind(now() - 180 * 86400),
    env.DB.prepare(
      "DELETE FROM meetings WHERE state!='active' AND updated_at<?",
    ).bind(now() - 180 * 86400),
  ]);
}
