import { record } from "./common";
export const backupColumns: Record<string, string[]> = {
  workos_revocations: ["kind", "subject", "at"],
  accounts: [
    "id",
    "email",
    "created_at",
    "last_seen_at",
    "bytes_used",
    "quota",
    "epoch",
    "deleted_at",
    "seq",
    "auth_verifier",
    "changes_floor",
    "is_test",
    "meeting_quota",
  ],
  identities: ["provider", "subject", "account"],
  devices: [
    "id",
    "account",
    "platform",
    "public_key",
    "created_at",
    "last_seen_at",
    "revoked_at",
  ],
  wraps: ["account", "kind", "ref", "ciphertext", "rev"],
  meetings: ["account", "id", "created_at", "state", "version", "updated_at"],
  objects: [
    "account",
    "id",
    "rev",
    "size",
    "body_key",
    "deleted",
    "updated_at",
    "meeting",
  ],
  changes: ["account", "seq", "id", "rev", "deleted", "at"],
  holds: ["account", "object", "device", "rev", "created_at", "version"],
  compactions: ["account", "meeting", "object", "snapshot", "purge_at"],
  rekey_pages: [
    "account",
    "epoch",
    "nonce",
    "page",
    "body_key",
    "size",
    "expires_at",
  ],
  trash: ["body_key", "account", "purge_at"],
  pending_resets: ["account", "device", "requested_at", "ready_at", "epoch"],
};

/** Offline operator procedure; never exposed as a Worker route. Restore to an empty staging DB. */
export async function restore(
  env: Env,
  source: R2Bucket,
  prefix: string,
  erasedAccounts: ReadonlySet<string>,
): Promise<void> {
  const erased = new Set(erasedAccounts);
  let cursor: string | undefined;
  do {
    const page = await source.list({ prefix: "erased/", cursor, limit: 500 });
    for (const marker of page.objects) erased.add(marker.key.slice(7));
    cursor = page.truncated ? page.cursor : undefined;
  } while (cursor);
  if (env.ENVIRONMENT !== "staging" || !/^snapshot\/[\dTZ:.-]+\/$/.test(prefix))
    throw Error("restore_target_invalid");
  const run = await env.DB.prepare(
    "SELECT prefix FROM restore_run WHERE singleton=1",
  ).first<{ prefix: string }>();
  if (run) {
    if (run.prefix !== prefix)
      throw Error("restore_retry_requires_same_snapshot");
    if (await env.DB.prepare("SELECT 1 FROM sessions LIMIT 1").first())
      throw Error("restore_target_in_use");
    await env.DB.batch([
      env.DB.prepare(
        "DELETE FROM trash WHERE account IN(SELECT id FROM restore_accounts)",
      ),
      env.DB.prepare(
        "DELETE FROM accounts WHERE id IN(SELECT id FROM restore_accounts)",
      ),
      env.DB.prepare("DELETE FROM restore_accounts"),
    ]);
  }
  if (await env.DB.prepare("SELECT 1 FROM accounts LIMIT 1").first())
    throw Error("restore_requires_empty_database");
  const object = await source.get(prefix + "manifest.json");
  if (!object) throw Error("restore_not_complete");
  const manifest = record(await object.json());
  if (manifest.schema !== 1 && manifest.schema !== 2)
    throw Error("restore_schema");
  const tables = record(manifest.tables);
  if (!run)
    await env.DB.prepare(
      "INSERT INTO restore_run(singleton,prefix) VALUES(1,?)",
    )
      .bind(prefix)
      .run();
  async function* rows(table: string): AsyncGenerator<unknown> {
    if (manifest.schema === 1) {
      const values =
        tables[table] ??
        ([
          "pending_resets",
          "workos_revocations",
          "meetings",
          "compactions",
          "rekey_pages",
        ].includes(table)
          ? []
          : null);
      if (!Array.isArray(values)) throw Error("restore_rows");
      yield* values;
      return;
    }
    const count =
      tables[table] ??
      ([
        "workos_revocations",
        "meetings",
        "compactions",
        "rekey_pages",
      ].includes(table)
        ? 0
        : undefined);
    if (typeof count !== "number" || !Number.isSafeInteger(count) || count < 0)
      throw Error("restore_pages");
    for (let page = 0; page < count; page++) {
      const object = await source.get(`${prefix}rows/${table}/${page}.json`);
      if (!object) throw Error("restore_missing_page");
      const values: unknown = await object.json();
      if (!Array.isArray(values) || values.length > 250)
        throw Error("restore_rows");
      yield* values;
    }
  }
  for (const [table, fields] of Object.entries(backupColumns)) {
    // Never resurrect a reset that was cancelled after the snapshot.
    if (table === "pending_resets" || table === "holds") continue;
    if (table === "changes") await env.DB.prepare("DELETE FROM changes").run();
    for await (const value of rows(table)) {
      const row = record(value);
      if (table === "workos_revocations") {
        await env.DB.prepare(
          "INSERT OR IGNORE INTO workos_revocations(kind,subject,at) VALUES(?,?,?)",
        )
          .bind(row.kind, row.subject, row.at)
          .run();
        continue;
      }
      const account = String(table === "accounts" ? row.id : row.account);
      if (
        erased.has(account) ||
        (table === "accounts" && row.deleted_at !== null)
      )
        continue;
      if (
        table !== "accounts" &&
        !(await env.DB.prepare("SELECT 1 FROM accounts WHERE id=?")
          .bind(account)
          .first())
      )
        continue;
      if (
        (table === "objects" && row.deleted === 0) ||
        table === "trash" ||
        table === "rekey_pages"
      ) {
        const key = String(row.body_key);
        if (!key.startsWith(account + "/")) throw Error("restore_key");
        const body = await source.get(prefix + "content/" + key);
        if (!body) throw Error("restore_missing_body");
        await env.CONTENT.put(key, body.body);
      }
      const values = fields.map((f) =>
        table === "accounts" && (f === "bytes_used" || f === "seq")
          ? 0
          : (row[f] ??
            (["changes_floor", "is_test", "version"].includes(f)
              ? 0
              : f === "meeting_quota"
                ? 2000000000
                : null)),
      );
      if (
        values.some(
          (v) => v !== null && typeof v !== "string" && typeof v !== "number",
        )
      )
        throw Error("restore_field");
      const insert = env.DB.prepare(
        `INSERT INTO ${table}(${fields.join(",")}) VALUES(${fields.map(() => "?").join(",")})`,
      ).bind(...values);
      if (table === "accounts")
        await env.DB.batch([
          env.DB.prepare("INSERT INTO restore_accounts(id) VALUES(?)").bind(
            account,
          ),
          insert,
        ]);
      else await insert.run();
    }
  }
  for await (const value of rows("accounts")) {
    const row = record(value);
    await env.DB.prepare("UPDATE accounts SET seq=? WHERE id=?")
      .bind(row.seq, row.id)
      .run();
  }
  await env.DB.batch([
    env.DB.prepare("DELETE FROM restore_accounts"),
    env.DB.prepare("DELETE FROM restore_run"),
  ]);
  // Sessions/challenges are absent by design: every restored device signs in afresh.
}
