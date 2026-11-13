export class Fault extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
export const now = () => Math.floor(Date.now() / 1000);
export const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: {
      "Content-Type": "application/json",
      "Cache-Control": "no-store",
    },
  });
export const token = () => b64(crypto.getRandomValues(new Uint8Array(32)));
export function b64(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replaceAll("=", "");
}
export function unb64(text: string): Uint8Array {
  try {
    return Uint8Array.from(
      atob(text.replaceAll("-", "+").replaceAll("_", "/")),
      (c) => c.charCodeAt(0),
    );
  } catch {
    throw new Fault(400, "Invalid encoding.");
  }
}
export const sha = async (text: string) =>
  b64(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)),
    ),
  );
export async function mac(secret: string, text: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  return b64(
    new Uint8Array(
      await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(text)),
    ),
  );
}
export function record(x: unknown): Record<string, unknown> {
  if (!x || typeof x !== "object" || Array.isArray(x))
    throw new Fault(400, "Expected an object.");
  return x as Record<string, unknown>;
}
export function str(x: unknown, max = 256): string {
  if (
    typeof x !== "string" ||
    !x.length ||
    x.length > max ||
    /[\x00-\x1f\x7f]/.test(x)
  )
    throw new Fault(400, "Invalid field.");
  return x;
}
export function id(x: unknown): string {
  const v = str(x, 128);
  if (!/^[A-Za-z0-9_-]+$/.test(v)) throw new Fault(400, "Invalid identifier.");
  return v;
}
export function encoded(x: unknown, size: number): string {
  const s = id(x);
  if (unb64(s).length !== size) throw new Fault(400, "Invalid key.");
  return s;
}
export function integer(x: unknown, min = 0): number {
  if (typeof x !== "number" || !Number.isSafeInteger(x) || x < min)
    throw new Fault(400, "Invalid number.");
  return x;
}
export async function bytes(
  request: Request | Response,
  limit: number,
): Promise<Uint8Array> {
  if (Number(request.headers.get("content-length")) > limit)
    throw new Fault(413, "Upload is too large.");
  const reader = request.body?.getReader();
  if (!reader) return new Uint8Array();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const r = await reader.read();
      if (r.done) break;
      size += r.value.length;
      if (size > limit) throw new Fault(413, "Upload is too large.");
      chunks.push(r.value);
    }
  } finally {
    await reader.cancel();
  }
  const result = new Uint8Array(size);
  let at = 0;
  for (const chunk of chunks) {
    result.set(chunk, at);
    at += chunk.length;
  }
  return result;
}
export async function body(
  request: Request,
  limit = 32768,
): Promise<Record<string, unknown>> {
  try {
    return record(
      JSON.parse(new TextDecoder().decode(await bytes(request, limit))),
    );
  } catch (e) {
    if (e instanceof Fault) throw e;
    throw new Fault(400, "Invalid JSON.");
  }
}
export async function limit(
  env: Env,
  key: string,
  max: number,
  seconds = 3600,
): Promise<void> {
  const t = now();
  const row = await env.DB.prepare(
    "INSERT INTO rate_limits(key,count,expires_at) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=CASE WHEN expires_at<=? THEN 1 ELSE count+1 END, expires_at=CASE WHEN expires_at<=? THEN ? ELSE expires_at END RETURNING count",
  )
    .bind(key, t + seconds, t, t, t + seconds)
    .first<{ count: number }>();
  if (!row || row.count > max)
    throw new Fault(429, "Too many attempts. Try again later.");
}
export type Session = {
  account: string;
  device: string;
  hash: string;
  fresh_at: number;
  expires_at: number;
  public_key: string;
  workos_session: string | null;
  provider_revoked: number;
};
export async function session(request: Request, env: Env): Promise<Session> {
  const bearer = request.headers
    .get("authorization")
    ?.match(/^Bearer ([A-Za-z0-9_-]{43})$/)?.[1];
  const cookie = request.headers
    .get("cookie")
    ?.match(/(?:^|; )sj_session=([A-Za-z0-9_-]{43})(?:;|$)/)?.[1];
  if (
    !bearer &&
    cookie &&
    !["GET", "HEAD"].includes(request.method) &&
    request.headers.get("origin") !== new URL(request.url).origin
  )
    throw new Fault(403, "Open the account page and try again.");
  const raw = bearer ?? cookie;
  if (!raw) throw new Fault(401, "Sign in to continue.");
  const hash = await sha(raw);
  const s = await env.DB.prepare(
    "SELECT s.account,s.device,s.hash,s.fresh_at,s.expires_at,s.workos_session,s.provider_revoked,d.public_key FROM sessions s JOIN devices d ON d.account=s.account AND d.id=s.device JOIN accounts a ON a.id=s.account WHERE s.hash=? AND s.expires_at>? AND s.created_at>? AND d.revoked_at IS NULL AND a.deleted_at IS NULL",
  )
    .bind(hash, now(), now() - 365 * 86400)
    .first<Session>();
  if (!s) throw new Fault(401, "Sign in to continue.");
  if (s.provider_revoked) {
    if (!s.public_key) throw new Fault(401, "Sign-in was revoked.");
    await (await import("./authorization")).authorizeKeys(request, env, s);
  }
  if (!["GET", "HEAD"].includes(request.method))
    await env.DB.batch([
      env.DB.prepare(
        "UPDATE sessions SET expires_at=MIN(?,created_at+31536000) WHERE hash=? AND expires_at<?",
      ).bind(now() + 90 * 86400, hash, now() + 89 * 86400),
      env.DB.prepare(
        "UPDATE accounts SET last_seen_at=? WHERE id=? AND last_seen_at<?",
      ).bind(now(), s.account, now() - 86400),
    ]);
  return s;
}
export function fresh(s: Session): void {
  if (s.provider_revoked || s.fresh_at < now() - 600)
    throw new Fault(403, "Sign in again before this action.");
}
export function native(s: Session): void {
  if (!s.public_key)
    throw new Fault(403, "Use a signed-in SonoJot app for this action.");
}
export function configuredOrigin(request: Request, env: Env): string {
  const actual = new URL(request.url).origin;
  if (env.PUBLIC_ORIGIN && env.PUBLIC_ORIGIN !== actual && env.AUTH_ORIGIN !== actual)
    throw new Fault(400, "Wrong service address.");
  return actual;
}
export function headers(response: Response): Response {
  const h = new Headers(response.headers);
  // Keep the authentication document unchanged by automatic analytics injection.
  h.set("Cache-Control", "no-store, no-transform");
  h.set("X-Content-Type-Options", "nosniff");
  h.set("Referrer-Policy", "no-referrer");
  h.set("X-Frame-Options", "DENY");
  h.set("Strict-Transport-Security", "max-age=31536000");
  h.set("Permissions-Policy", "camera=(), microphone=(), geolocation=()");
  return new Response(response.body, { status: response.status, headers: h });
}
