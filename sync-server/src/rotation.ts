import { authorizeKeys, RESET_WINDOW } from "./authorization";
import {
  Fault,
  body,
  id,
  str,
  integer,
  json,
  session,
  native,
  fresh,
  now,
  limit,
  record,
  token,
  encoded,
} from "./common";
export async function rotation(
  request: Request,
  env: Env,
): Promise<Response | null> {
  if (
    new URL(request.url).pathname !== "/v1/rekey" ||
    request.method !== "POST"
  )
    return null;
  const s = await session(request, env);
  native(s);
  fresh(s);
  await limit(env, "rekey:" + s.account, 10);
  const b = await body(request, 262144);
  if (b.confirmed !== true) throw new Fault(400, "Confirm the key change.");
  const epoch = integer(b.epoch, 1),
    previous = integer(b.previous, 1),
    target = id(b.device),
    reset = b.reset === true;
  const verifier = reset ? null : await authorizeKeys(request, env, s);
  const nextVerifier = encoded(b.verifier, 32);
  const vocabularyRevision = reset ? 0 : integer(b.vocabularyRevision);
  const pruneCursor =
    b.pruneCursor === undefined ? null : integer(b.pruneCursor);
  if (reset) {
    const ready = await env.DB.prepare(
      "SELECT 1 FROM pending_resets WHERE account=? AND epoch=? AND ready_at<=? AND ready_at>?",
    )
      .bind(s.account, epoch, now(), now() - RESET_WINDOW)
      .first();
    if (!ready)
      throw new Fault(
        403,
        "Request a reset and wait 72 hours. Complete it within the following 7 days; unlocked devices cancel it.",
      );
  }
  if (target === s.device && !reset)
    throw new Fault(400, "Re-key from a device you are keeping.");
  const cipher = (v: unknown) => {
    const c = str(v, 44000);
    if (!/^[A-Za-z0-9_-]+$/.test(c))
      throw new Fault(400, "Invalid ciphertext.");
    return c;
  };
  const ring = cipher(b.keyring),
    recovery = cipher(b.recovery);
  if (
    !Array.isArray(b.devices) ||
    b.devices.length < 1 ||
    b.devices.length > 64
  )
    throw new Fault(400, "Invalid device wraps.");
  const devices = b.devices.map((v) => {
    const d = record(v);
    return { id: id(d.id), ciphertext: cipher(d.ciphertext) };
  });
  if (
    new Set(devices.map((d) => d.id)).size !== devices.length ||
    !devices.some((d) => d.id === s.device) ||
    (!reset && devices.some((d) => d.id === target))
  )
    throw new Fault(400, "Invalid device roster.");
  const operation = token();
  const guard = "EXISTS(SELECT 1 FROM accounts WHERE id=? AND rotation=?)";
  const statements = [
    env.DB.prepare(
      `UPDATE accounts SET epoch=epoch+1,rotation=?,auth_verifier=? WHERE id=? AND epoch=? AND EXISTS(SELECT 1 FROM wraps WHERE account=? AND kind='keyring' AND ref='main' AND rev=?) AND ${reset ? "EXISTS(SELECT 1 FROM pending_resets WHERE account=accounts.id AND epoch=accounts.epoch AND ready_at<=? AND ready_at>?)" : "auth_verifier=? AND COALESCE((SELECT rev FROM objects WHERE account=accounts.id AND id='vocabulary'),0)=?"}${pruneCursor === null ? "" : " AND seq=?"}`,
    ).bind(
      operation,
      nextVerifier,
      s.account,
      epoch,
      s.account,
      previous,
      ...(reset
        ? [now(), now() - RESET_WINDOW]
        : [verifier, vocabularyRevision]),
      ...(pruneCursor === null ? [] : [pruneCursor]),
    ),
  ];
  if (reset) {
    // Old bodies enter the normal 30-day trash queue through the object trigger.
    statements.push(
      env.DB.prepare(
        `UPDATE objects SET rev=rev+1,size=0,body_key='',deleted=1,updated_at=? WHERE account=? AND ${guard}`,
      ).bind(now(), s.account, s.account, operation),
    );
    statements.push(
      env.DB.prepare(`DELETE FROM objects WHERE account=? AND ${guard}`).bind(
        s.account,
        s.account,
        operation,
      ),
    );
  }
  statements.push(
    env.DB.prepare(
      `UPDATE devices SET revoked_at=? WHERE account=? AND id${reset ? "<>" : "="}? AND ${guard}`,
    ).bind(now(), s.account, reset ? s.device : target, s.account, operation),
  );
  statements.push(
    env.DB.prepare(
      `DELETE FROM sessions WHERE account=? AND device IN(SELECT id FROM devices WHERE account=? AND revoked_at IS NOT NULL) AND ${guard}`,
    ).bind(s.account, s.account, s.account, operation),
  );
  statements.push(
    env.DB.prepare(`DELETE FROM wraps WHERE account=? AND ${guard}`).bind(
      s.account,
      s.account,
      operation,
    ),
  );
  statements.push(
    env.DB.prepare(
      `UPDATE pairings SET state='denied',ciphertext=NULL WHERE account=? AND ${guard}`,
    ).bind(s.account, s.account, operation),
  );
  for (const w of [
    { kind: "keyring", ref: "main", ciphertext: ring, rev: previous + 1 },
    { kind: "recovery", ref: "main", ciphertext: recovery, rev: epoch + 1 },
    ...devices.map((d) => ({
      kind: "device",
      ref: d.id,
      ciphertext: d.ciphertext,
      rev: epoch + 1,
    })),
  ])
    statements.push(
      env.DB.prepare(
        `INSERT INTO wraps(account,kind,ref,ciphertext,rev) SELECT ?,?,?,?,? WHERE ${guard}`,
      ).bind(
        s.account,
        w.kind,
        w.ref,
        w.ciphertext,
        w.rev,
        s.account,
        operation,
      ),
    );
  statements.push(
    env.DB.prepare(
      `DELETE FROM pending_resets WHERE account=? AND ${guard}`,
    ).bind(s.account, s.account, operation),
  );
  const result = await env.DB.batch(statements);
  if (result[0].meta.changes !== 1)
    throw new Fault(412, "Keys changed. Reload before re-keying.");
  return json({ epoch: epoch + 1, rev: previous + 1 });
}
