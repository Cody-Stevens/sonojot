import { now } from "./common";
import { eraseAccount } from "./store";
import { productionAuth } from "./auth";
import { RESET_WINDOW } from "./authorization";
import { backupColumns } from "./restore";
export async function backup(env: Env): Promise<string> {
  const stamp = new Date().toISOString();
  const prefix = "snapshot/" + stamp + "/";
  const run = crypto.randomUUID();
  const tables = Object.keys(backupColumns);
  // Materialize all tables in one D1 transaction, then page immutable rows.
  // Concurrent edits cannot mix revisions between pages or tables.
  await env.DB.batch(
    tables.map((table) =>
      env.DB.prepare(
        `INSERT INTO backup_rows(run,table_name,ordinal,payload,created_at) SELECT ?,?,ROW_NUMBER() OVER (),json_object(${backupColumns[table].map((c) => `'${c}',${c}`).join(",")}),? FROM ${table}`,
      ).bind(run, table, now()),
    ),
  );
  try {
    const pages: Record<string, number> = {};
    for (const table of tables) {
      let after = 0,
        page = 0;
      for (;;) {
        const rows = (
          await env.DB.prepare(
            "SELECT ordinal,payload FROM backup_rows WHERE run=? AND table_name=? AND ordinal>? ORDER BY ordinal LIMIT 250",
          )
            .bind(run, table, after)
            .all<{ ordinal: number; payload: string }>()
        ).results;
        if (!rows.length) break;
        const values = rows.map(
          (row) => JSON.parse(row.payload) as Record<string, unknown>,
        );
        if (table === "objects" || table === "trash")
          for (const row of values) {
            if (table === "objects" && row.deleted) continue;
            const key = String(row.body_key);
            if (!key) continue;
            const body = await env.CONTENT.get(key);
            if (!body) throw Error("backup_missing_body");
            await env.BACKUPS.put(prefix + "content/" + key, body.body);
          }
        await env.BACKUPS.put(
          `${prefix}rows/${table}/${page++}.json`,
          JSON.stringify(values),
        );
        after = rows[rows.length - 1].ordinal;
      }
      pages[table] = page;
    }
    await env.BACKUPS.put(
      prefix + "manifest.json",
      JSON.stringify({ schema: 2, at: stamp, tables: pages }),
    );
    await env.DB.prepare(
      "INSERT INTO operations(name,last_ok,count) VALUES('backup',?,1) ON CONFLICT(name) DO UPDATE SET last_ok=excluded.last_ok,count=count+1",
    )
      .bind(now())
      .run();
    return prefix;
  } finally {
    await env.DB.prepare("DELETE FROM backup_rows WHERE run=?").bind(run).run();
  }
}
export async function maintenance(env: Env): Promise<void> {
  try {
    const t = now();
    await env.DB.prepare("DELETE FROM backup_rows WHERE created_at<?")
      .bind(t - 86400)
      .run();
    await env.DB.prepare(
      "DELETE FROM operations WHERE name LIKE 'http5xx:%' AND last_failure<?",
    )
      .bind(t - 7 * 86400)
      .run();
    await env.DB.prepare(
      "DELETE FROM operations WHERE name LIKE 'http:%' AND last_ok<?",
    )
      .bind(t - 7 * 86400)
      .run();
    const bytes = await env.DB.prepare(
      "SELECT COALESCE(SUM(bytes_used),0) AS n FROM accounts",
    ).first<{ n: number }>();
    const previous = await env.DB.prepare(
      "SELECT count FROM operations WHERE name='storage-bytes'",
    ).first<{ count: number }>();
    const rapidGrowth =
      previous &&
      (bytes?.n ?? 0) > Math.max(10 * 1024 * 1024, previous.count * 2);
    await env.DB.batch([
      env.DB.prepare(
        "INSERT INTO operations(name,last_ok,count) VALUES('storage-bytes',?,?) ON CONFLICT(name) DO UPDATE SET last_ok=excluded.last_ok,count=excluded.count",
      ).bind(t, bytes?.n ?? 0),
      env.DB.prepare(
        "INSERT INTO operations(name,last_ok,count) VALUES('storage-growth',?,?) ON CONFLICT(name) DO UPDATE SET last_ok=excluded.last_ok,count=excluded.count",
      ).bind(t, rapidGrowth ? 1 : 0),
    ]);
    await env.DB.prepare(
      "UPDATE accounts SET deleted_at=? WHERE deleted_at IS NULL AND last_seen_at<?",
    )
      .bind(t, t - 365 * 86400)
      .run();
    const expired = (
      await env.DB.prepare(
        "SELECT id FROM accounts WHERE deleted_at IS NOT NULL LIMIT 100",
      ).all<{ id: string }>()
    ).results;
    for (const a of expired) await eraseAccount(env, a.id);
    const trash = (
      await env.DB.prepare(
        "SELECT body_key FROM trash WHERE purge_at<=? LIMIT 500",
      )
        .bind(t)
        .all<{ body_key: string }>()
    ).results;
    for (const o of trash) {
      if (o.body_key) await env.CONTENT.delete(o.body_key);
      await env.DB.prepare("DELETE FROM trash WHERE body_key=?")
        .bind(o.body_key)
        .run();
    }
    await env.DB.batch([
      env.DB.prepare("DELETE FROM workos_events WHERE at<?").bind(
        t - 30 * 86400,
      ),
      env.DB.prepare("DELETE FROM workos_revocations WHERE at<?").bind(
        t - 400 * 86400,
      ),
      env.DB.prepare(
        "DELETE FROM sessions WHERE expires_at<=? OR created_at<=?",
      ).bind(t, t - 365 * 86400),
      env.DB.prepare(
        "DELETE FROM notifications WHERE kind='reset-requested' AND EXISTS(SELECT 1 FROM pending_resets WHERE account=notifications.account AND ready_at<=?)",
      ).bind(t - RESET_WINDOW),
      env.DB.prepare("DELETE FROM pending_resets WHERE ready_at<=?").bind(
        t - RESET_WINDOW,
      ),
      env.DB.prepare("DELETE FROM auth_requests WHERE expires_at<=?").bind(t),
      env.DB.prepare("DELETE FROM pairings WHERE expires_at<=?").bind(t),
      env.DB.prepare("DELETE FROM rate_limits WHERE expires_at<=?").bind(t),
      env.DB.prepare(
        "DELETE FROM devices WHERE public_key='' AND NOT EXISTS(SELECT 1 FROM sessions WHERE sessions.account=devices.account AND sessions.device=devices.id)",
      ),
      env.DB.prepare(
        "UPDATE accounts SET changes_floor=MAX(changes_floor,COALESCE((SELECT MAX(seq) FROM changes WHERE changes.account=accounts.id AND at<?),0))",
      ).bind(t - 30 * 86400),
      env.DB.prepare("DELETE FROM changes WHERE at<?").bind(t - 30 * 86400),
    ]);
    let cursor: string | undefined;
    do {
      const page = await env.CONTENT.list({ cursor, limit: 500 });
      for (const o of page.objects) {
        if (o.uploaded.getTime() > Date.now() - 86400000) continue;
        const ref = await env.DB.prepare(
          "SELECT 1 FROM objects WHERE body_key=? UNION SELECT 1 FROM trash WHERE body_key=? LIMIT 1",
        )
          .bind(o.key, o.key)
          .first();
        if (!ref) await env.CONTENT.delete(o.key);
      }
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor);
    if (env.MAIL_ENABLED === "true") {
      const notices = (
        await env.DB.prepare(
          "SELECT n.id,n.kind,COALESCE(n.recipient,a.email) AS email FROM notifications n JOIN accounts a ON a.id=n.account WHERE n.attempts<5 ORDER BY n.created_at LIMIT 50",
        ).all<{ id: string; kind: string; email: string }>()
      ).results;
      for (const n of notices) {
        await env.DB.prepare(
          "UPDATE notifications SET attempts=attempts+1 WHERE id=?",
        )
          .bind(n.id)
          .run();
        await productionAuth.mail(
          env,
          n.email,
          n.kind === "identity-updated"
            ? "WorkOS identity changed"
            : n.kind === "email-changed"
              ? "SonoJot contact email changed"
              : n.kind === "reset-requested"
                ? "SonoJot sync reset requested"
                : "New device added to SonoJot",
          n.kind === "identity-updated"
            ? "Your WorkOS email changed. SonoJot kept this address for reset notices. Open Account and sync in an unlocked SonoJot app and confirm the contact email if this change was yours. Otherwise secure your WorkOS sign-in."
            : n.kind === "email-changed"
              ? "Your SonoJot contact email changed. This notice was sent to your previous address. If this was not you, open SonoJot on an unlocked device and re-key your account, then secure your sign-in accounts."
              : n.kind === "reset-requested"
                ? "A sync reset was requested. It cannot complete for 72 hours and expires 7 days later. Open SonoJot on an unlocked device to cancel it automatically."
                : "A device signed in to your SonoJot account. It cannot read encrypted words until approved or recovered. If this was not you, open Account and sync on a trusted device and remove it.",
        );
        await env.DB.prepare("DELETE FROM notifications WHERE id=?")
          .bind(n.id)
          .run();
      }
    }
    await backup(env);
    do {
      const page = await env.BACKUPS.list({ cursor, limit: 500 });
      const old = page.objects.filter(
        (o) =>
          o.uploaded.getTime() <
          Date.now() - (o.key.startsWith("erased/") ? 35 : 30) * 86400000,
      );
      if (old.length) await env.BACKUPS.delete(old.map((o) => o.key));
      cursor = page.truncated ? page.cursor : undefined;
    } while (cursor);
    await env.DB.prepare(
      "INSERT INTO operations(name,last_ok,count) VALUES('cron',?,1) ON CONFLICT(name) DO UPDATE SET last_ok=excluded.last_ok,count=count+1",
    )
      .bind(t)
      .run();
  } catch {
    await env.DB.prepare(
      "INSERT INTO operations(name,last_failure) VALUES('cron',?) ON CONFLICT(name) DO UPDATE SET last_failure=excluded.last_failure",
    )
      .bind(now())
      .run();
    throw Error("sync_maintenance_failed");
  }
}
