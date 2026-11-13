import { json, now } from "./common";
export async function requestCount(env: Env): Promise<void> {
  const minute = Math.floor(now() / 60) * 60;
  await env.DB.prepare(
    "INSERT INTO operations(name,last_ok,count) VALUES(?,?,1) ON CONFLICT(name) DO UPDATE SET count=count+1",
  )
    .bind("http:" + minute, minute)
    .run();
}
export async function serverFailure(env: Env): Promise<void> {
  // Only aggregate counts. Never URLs, exception strings, accounts or request bodies.
  const minute = Math.floor(now() / 60) * 60;
  await env.DB.prepare(
    "INSERT INTO operations(name,last_failure,count) VALUES(?,?,1) ON CONFLICT(name) DO UPDATE SET count=count+1",
  )
    .bind("http5xx:" + minute, minute)
    .run();
}
export async function health(env: Env): Promise<Response> {
  await env.DB.prepare("SELECT 1").first();
  const rows = (
    await env.DB.prepare(
      "SELECT name,last_ok,last_failure FROM operations WHERE name IN ('cron','backup')",
    ).all<{
      name: string;
      last_ok: number | null;
      last_failure: number | null;
    }>()
  ).results;
  await env.DB.prepare(
    "INSERT OR IGNORE INTO operations(name,last_ok) VALUES('started',?)",
  )
    .bind(now())
    .run();
  const started = await env.DB.prepare(
    "SELECT last_ok FROM operations WHERE name='started'",
  ).first<{ last_ok: number }>();
  const stale =
    (rows.length !== 2 && (started?.last_ok ?? now()) < now() - 36 * 3600) ||
    rows.some(
      (r) =>
        !r.last_ok ||
        r.last_ok < now() - 36 * 3600 ||
        (r.last_failure ?? 0) > r.last_ok,
    );
  const failures = await env.DB.prepare(
    "SELECT SUM(count) AS n FROM operations WHERE name LIKE 'http5xx:%' AND last_failure>?",
  )
    .bind(now() - 900)
    .first<{ n: number | null }>();
  const requests = await env.DB.prepare(
    "SELECT SUM(count) AS n FROM operations WHERE name LIKE 'http:%' AND last_ok>?",
  )
    .bind(now() - 900)
    .first<{ n: number | null }>();
  const highErrors =
    (failures?.n ?? 0) >= 5 &&
    (failures?.n ?? 0) / Math.max(1, requests?.n ?? 0) >= 0.05;
  const growth = await env.DB.prepare(
    "SELECT count FROM operations WHERE name='storage-growth'",
  ).first<{ count: number }>();
  const ok = !stale && !highErrors && (growth?.count ?? 0) === 0;
  return json(
    {
      ok,
      stage: "free-vocabulary",
      maintenance:
        rows.length === 2 ? (stale ? "attention" : "ok") : "awaiting-first-run",
    },
    ok ? 200 : 503,
  );
}
