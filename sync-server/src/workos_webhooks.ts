import { client } from "./authkit";
import { bytes, Fault, id, json, now, record, token } from "./common";

export async function webhook(
  request: Request,
  env: Env,
): Promise<Response | null> {
  if (new URL(request.url).pathname !== "/v1/auth/webhook") return null;
  if (request.method !== "POST") throw new Fault(405, "Use POST.");
  if (!env.WORKOS_WEBHOOK_SECRET)
    throw new Fault(503, "WorkOS webhooks are not configured yet.");
  const payload = new TextDecoder().decode(await bytes(request, 65536));
  const sigHeader = request.headers.get("workos-signature") ?? "";
  // Check both past and future timestamps; SDK versions have differed on future tolerance.
  const stamp = Number(/^t=(\d+),/.exec(sigHeader)?.[1]);
  if (!Number.isSafeInteger(stamp) || Math.abs(Date.now() - stamp) > 180000)
    throw new Fault(400, "Invalid webhook signature.");
  let event;
  try {
    event = await client(env).webhooks.constructEvent({
      payload,
      sigHeader,
      secret: env.WORKOS_WEBHOOK_SECRET,
      // The SDK compares milliseconds: Math.abs(Date.now() - timestampMs) > tolerance.
      tolerance: 180_000,
    });
  } catch {
    throw new Fault(400, "Invalid webhook signature.");
  }
  if (
    !["user.deleted", "session.revoked", "user.updated"].includes(event.event)
  )
    return json({ received: true });
  const eventId = id(event.id);
  const data = record(event.data);
  const at = Math.floor(event.createdAt.getTime() / 1000);
  if (!Number.isSafeInteger(at) || at < 0 || at > now() + 180)
    throw new Fault(400, "Invalid webhook event.");
  const subject = id(data.id);
  const statements: D1PreparedStatement[] = [];
  if (event.event === "user.updated") {
    // A signed identity event is not an account-key proof. The old contact remains
    // the reset-notice destination until the unlocked app confirms a fresh login.
    if (
      data.emailVerified === true &&
      typeof data.email === "string" &&
      data.email.length <= 254
    ) {
      statements.push(
        env.DB.prepare(
          "INSERT INTO notifications(id,account,kind,created_at,recipient) SELECT ?,a.id,'identity-updated',?,a.email FROM accounts a JOIN identities i ON i.account=a.id WHERE i.provider='workos' AND i.subject=? AND a.email<>? AND a.deleted_at IS NULL AND NOT EXISTS(SELECT 1 FROM workos_events WHERE id=?)",
        ).bind(token(), now(), subject, data.email.toLowerCase(), eventId),
      );
    }
  } else {
    const kind = event.event === "user.deleted" ? "user" : "session";
    if (!subject.startsWith(kind + "_"))
      throw new Fault(400, "Invalid webhook subject.");
    statements.push(
      env.DB.prepare(
        "INSERT OR IGNORE INTO workos_revocations(kind,subject,at) VALUES(?,?,?)",
      ).bind(kind, subject, now()),
    );
    statements.push(
      env.DB.prepare(
        `UPDATE sessions SET provider_revoked=1 WHERE ${kind === "user" ? "workos_user" : "workos_session"}=?`,
      ).bind(subject),
    );
    // Provider revocation closes keyless access. A previously unlocked native device
    // retains access ONLY with the current AK proof (common.session), including reset cancellation.
    // Neither deletion nor revocation touches devices, wraps, vocabulary or reset state.
  }
  statements.push(
    env.DB.prepare(
      "INSERT OR IGNORE INTO workos_events(id,at) VALUES(?,?)",
    ).bind(eventId, now()),
  );
  await env.DB.batch(statements);
  return json({ received: true });
}
