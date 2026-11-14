import { now } from "./common";
import { mailOrigin, noticeMail, type MailSender } from "./mail";

/** Must match the notice trigger in wrangler.jsonc. */
export const NOTICE_CRON = "*/10 * * * *";

// A notice may be tried five times. After a failed try it waits attempts² × 15
// minutes (15 min, 1 h, 2 h 15 min, 4 h), which also keeps a send that is still
// in progress from being picked up twice.
const due = "attempts<5 AND (tried_at IS NULL OR tried_at<=?-attempts*attempts*900)";

async function failed(env: Env): Promise<void> {
  try {
    await env.DB.prepare(
      "INSERT INTO operations(name,last_failure) VALUES('notices',?) ON CONFLICT(name) DO UPDATE SET last_failure=excluded.last_failure",
    )
      .bind(now())
      .run();
  } catch {}
}

/** Sends one queued notice now. A failure leaves it queued for the next run. */
export async function deliverNotice(
  env: Env,
  id: string,
  send: MailSender,
): Promise<boolean> {
  if (env.MAIL_ENABLED !== "true") return false;
  const t = now();
  const n = await env.DB.prepare(
    `UPDATE notifications SET attempts=attempts+1,tried_at=? WHERE id=? AND ${due} RETURNING account,kind,detail,created_at,recipient`,
  )
    .bind(t, id, t)
    .first<{
      account: string;
      kind: string;
      detail: string | null;
      created_at: number;
      recipient: string | null;
    }>();
  if (!n) return false;
  try {
    const to =
      n.recipient ??
      (
        await env.DB.prepare("SELECT email FROM accounts WHERE id=?")
          .bind(n.account)
          .first<{ email: string }>()
      )?.email;
    if (!to) throw Error("notice_without_recipient");
    await send(
      env,
      noticeMail(
        n.kind,
        { to, at: n.created_at, detail: n.detail },
        mailOrigin(env),
      ),
    );
  } catch {
    // No address, message or provider detail is recorded.
    await failed(env);
    return false;
  }
  await env.DB.prepare("DELETE FROM notifications WHERE id=?").bind(id).run();
  return true;
}

/** Sends queued notices that are due, oldest first. */
export async function sendNotices(env: Env, send: MailSender): Promise<void> {
  if (env.MAIL_ENABLED !== "true") return;
  const ids = (
    await env.DB.prepare(
      `SELECT id FROM notifications WHERE ${due} ORDER BY created_at LIMIT 50`,
    )
      .bind(now())
      .all<{ id: string }>()
  ).results;
  for (const n of ids) await deliverNotice(env, n.id, send);
}
