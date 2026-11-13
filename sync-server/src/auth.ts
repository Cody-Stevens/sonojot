import { authorizeKeys, authorizeAccountChange } from "./authorization";
import {
  Fault,
  body,
  id,
  str,
  encoded,
  integer,
  json,
  now,
  sha,
  token,
  limit,
  session,
  native,
  fresh,
  configuredOrigin,
} from "./common";
import {
  authorize,
  exchange,
  configured,
  type Identity,
  type AuthRequest,
} from "./authkit";
export type AuthDependencies = {
  exchange: typeof exchange;
  mail: (env: Env, to: string, subject: string, text: string) => Promise<void>;
};
export const productionAuth: AuthDependencies = {
  exchange,
  mail: async (env, to, subject, text) => {
    if (env.MAIL_ENABLED !== "true")
      throw new Fault(503, "Security notices are not configured yet.");
    await env.MAIL.send({
      from: "notices@sonojot.com",
      to,
      subject,
      text,
      replyTo: "support@sonojot.com",
    });
  },
};
async function requestRow(env: Env, requestId: string): Promise<AuthRequest> {
  const a = await env.DB.prepare(
    "SELECT * FROM auth_requests WHERE id=? AND expires_at>? AND used=0",
  )
    .bind(requestId, now())
    .first<AuthRequest>();
  if (!a) throw new Fault(401, "Sign-in expired. Start again.");
  return a;
}
async function identify(env: Env, identity: Identity): Promise<string> {
  // Never automatically link identities by email: provider subjects are the identity boundary.
  const account = crypto.randomUUID();
  const t = now();
  await env.DB.batch([
    env.DB.prepare(
      "INSERT INTO accounts(id,email,created_at,last_seen_at) SELECT ?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM identities WHERE provider=? AND subject=?)",
    ).bind(account, identity.email, t, t, identity.provider, identity.subject),
    env.DB.prepare(
      "INSERT OR IGNORE INTO identities(provider,subject,account) SELECT ?,?,id FROM accounts WHERE id=?",
    ).bind(identity.provider, identity.subject, account),
    env.DB.prepare(
      "DELETE FROM accounts WHERE id=? AND NOT EXISTS(SELECT 1 FROM identities WHERE account=?)",
    ).bind(account, account),
  ]);
  const row = await env.DB.prepare(
    "SELECT account FROM identities WHERE provider=? AND subject=?",
  )
    .bind(identity.provider, identity.subject)
    .first<{ account: string }>();
  if (!row) throw new Fault(503, "Sign-in could not finish. Try again.");
  return row.account;
}
async function grant(
  env: Env,
  a: AuthRequest,
  identity: Identity,
): Promise<string> {
  if (
    await env.DB.prepare(
      "SELECT 1 FROM workos_revocations WHERE (kind='user' AND subject=?) OR (kind='session' AND subject=?)",
    )
      .bind(identity.subject, identity.session)
      .first()
  )
    throw new Fault(401, "WorkOS sign-in was revoked. Start again.");
  let account: string;
  if (a.link_account) {
    const valid = await env.DB.prepare(
      "SELECT 1 FROM sessions s JOIN accounts a ON a.id=s.account WHERE s.hash=? AND s.account=? AND s.expires_at>? AND a.deleted_at IS NULL",
    )
      .bind(a.link_session, a.link_account, now())
      .first();
    if (!valid)
      throw new Fault(401, "Sign in again before linking an identity.");
    const exists = await env.DB.prepare(
      "SELECT account FROM identities WHERE provider=? AND subject=?",
    )
      .bind(identity.provider, identity.subject)
      .first<{ account: string }>();
    if (exists && exists.account !== a.link_account)
      throw new Fault(409, "That identity belongs to another account.");
    const guard =
      "EXISTS(SELECT 1 FROM accounts a JOIN sessions s ON s.account=a.id WHERE a.id=? AND a.auth_verifier IS ? AND (a.auth_verifier IS NOT NULL OR NOT EXISTS(SELECT 1 FROM wraps WHERE account=a.id)) AND a.deleted_at IS NULL AND s.hash=? AND s.expires_at>?)";
    const bound = [
      a.link_account,
      a.link_verifier ?? null,
      a.link_session ?? null,
      now(),
    ];
    const result = await env.DB.batch([
      env.DB.prepare(
        `INSERT OR IGNORE INTO identities(provider,subject,account) SELECT ?,?,? WHERE ${guard}`,
      ).bind(identity.provider, identity.subject, a.link_account, ...bound),
      env.DB.prepare(
        `INSERT INTO notifications(id,account,kind,created_at,recipient) SELECT ?,id,'email-changed',?,email FROM accounts WHERE id=? AND email<>? AND ${guard} AND EXISTS(SELECT 1 FROM identities WHERE provider=? AND subject=? AND account=accounts.id)`,
      ).bind(
        token(),
        now(),
        a.link_account,
        identity.email,
        ...bound,
        identity.provider,
        identity.subject,
      ),
      env.DB.prepare(
        `UPDATE accounts SET email=? WHERE id=? AND ${guard} AND EXISTS(SELECT 1 FROM identities WHERE provider=? AND subject=? AND account=accounts.id)`,
      ).bind(
        identity.email,
        a.link_account,
        ...bound,
        identity.provider,
        identity.subject,
      ),
    ]);
    if (result[2].meta.changes !== 1)
      throw new Fault(
        409,
        "Encryption authorization changed. Unlock and start linking again.",
      );
    account = a.link_account;
  } else account = await identify(env, identity);
  const code = token();
  const r = await env.DB.prepare(
    "UPDATE auth_requests SET account=?,grant_hash=?,workos_user=?,workos_session=?,auth_time=? WHERE id=? AND account IS NULL AND used=0 AND expires_at>?",
  )
    .bind(
      account,
      await sha(code),
      identity.subject,
      identity.session,
      identity.authTime,
      a.id,
      now(),
    )
    .run();
  if (r.meta.changes !== 1)
    throw new Fault(401, "Sign-in has already been used.");
  return code;
}
export async function auth(
  request: Request,
  env: Env,
  deps: AuthDependencies,
): Promise<Response | null> {
  const u = new URL(request.url);
  const path = u.pathname;
  const origin = configuredOrigin(request, env);
  if (!path.startsWith("/v1/auth/")) return null;
  const ip = request.headers.get("cf-connecting-ip") ?? "local";
  if (path === "/v1/auth/start" && request.method === "POST") {
    await limit(env, "signin:" + (await sha(ip)), 10);
    const b = await body(request);
    const p = b.provider ?? "authkit";
    if (p !== "authkit") throw new Fault(400, "Unknown provider.");
    configured(env);
    let redirect = b.redirect === "web" ? "web" : "sonojot://sync";
    if (typeof b.redirect === "string" && b.redirect.startsWith("http:")) {
      const r = new URL(b.redirect);
      if (
        r.hostname !== "127.0.0.1" ||
        r.pathname !== "/sync" ||
        Number(r.port) < 1024 ||
        r.username ||
        r.password ||
        r.search ||
        r.hash
      )
        throw new Fault(400, "Invalid loopback return.");
      redirect = r.toString();
    }
    const a: AuthRequest = {
      id: token(),
      provider: p,
      challenge: encoded(b.challenge, 32),
      nonce: token(),
      device: id(b.device),
      public_key: redirect === "web" ? "" : encoded(b.publicKey, 32),
      platform: redirect === "web" ? "Web" : str(b.platform, 32),
      redirect,
      expires_at: now() + 300,
      account: null,
      grant_hash: null,
    };
    const linked = b.link === true ? await session(request, env) : null;
    if (linked) {
      // A revoked identity can be replaced by its key holder. The callback still
      // requires fresh WorkOS authentication; a provider-only caller cannot enter here.
      if (!linked.provider_revoked) fresh(linked);
      a.link_verifier = await authorizeAccountChange(request, env, linked);
    }
    a.link_account = linked?.account ?? null;
    a.link_session = linked?.hash ?? null;
    const url = origin + "/v1/auth/browser?request=" + a.id;
    await env.DB.prepare(
      "INSERT INTO auth_requests(id,provider,challenge,nonce,device,public_key,platform,redirect,expires_at,link_account,link_session,link_verifier) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
    )
      .bind(
        a.id,
        p,
        a.challenge,
        a.nonce,
        a.device,
        a.public_key,
        a.platform,
        a.redirect,
        a.expires_at,
        a.link_account,
        a.link_session,
        a.link_verifier ?? null,
      )
      .run();
    return json({ id: a.id, url });
  }
  if (path === "/v1/auth/browser" && request.method === "GET") {
    const a = await requestRow(env, id(u.searchParams.get("request")));
    const verifier = token();
    const challenge = await sha(verifier);
    const url = authorize(env, a, origin, challenge);
    const claimed = await env.DB.prepare(
      "UPDATE auth_requests SET browser_hash=? WHERE id=? AND browser_hash IS NULL AND used=0 AND expires_at>?",
    )
      .bind(challenge, a.id, now())
      .run();
    if (claimed.meta.changes !== 1)
      throw new Fault(401, "Start sign-in again in this browser.");
    return new Response(null, {
      status: 303,
      headers: {
        Location: url,
        "Set-Cookie": `__Host-sj_login=${a.id}.${verifier}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=300`,
      },
    });
  }
  if (path === "/v1/auth/callback" && request.method === "GET") {
    const state = u.searchParams.getAll("state"),
      codes = u.searchParams.getAll("code");
    if (state.length !== 1 || codes.length !== 1 || u.searchParams.has("error"))
      throw new Fault(400, "Sign-in could not be completed. Start again.");
    const a = await requestRow(env, id(state[0]));
    const cookies =
      request.headers
        .get("cookie")
        ?.split(";")
        .map((v) => v.trim())
        .filter((v) => v.startsWith("__Host-sj_login=")) ?? [];
    const cookie =
      cookies.length === 1
        ? /^__Host-sj_login=([A-Za-z0-9_-]{43})\.([A-Za-z0-9_-]{43})$/.exec(
            cookies[0],
          )
        : null;
    if (
      !cookie ||
      cookie[1] !== a.id ||
      a.browser_hash !== (await sha(cookie[2]))
    )
      throw new Fault(401, "Start sign-in again in this browser.");
    await limit(env, "callback:" + (await sha(ip)), 30);
    const claimed = await env.DB.prepare(
      "UPDATE auth_requests SET callback_started=1 WHERE id=? AND callback_started=0 AND used=0 AND expires_at>?",
    )
      .bind(a.id, now())
      .run();
    if (claimed.meta.changes !== 1)
      throw new Fault(401, "Sign-in already used.");
    const identity = await deps.exchange(env, str(codes[0], 2048), cookie[2]);
    const code = await grant(env, a, identity);
    return new Response(null, {
      status: 303,
      headers: {
        Location:
          a.redirect === "web"
            ? origin + "/account#request=" + a.id + "&code=" + code
            : a.redirect + "?request=" + a.id + "&code=" + code,
        "Set-Cookie":
          "__Host-sj_login=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0",
      },
    });
  }
  if (path === "/v1/auth/token" && request.method === "POST") {
    await limit(env, "token:" + (await sha(ip)), 60);
    const b = await body(request);
    const a = await requestRow(env, id(b.request));
    if (
      !a.account ||
      a.grant_hash !== (await sha(str(b.code, 128))) ||
      a.challenge !== (await sha(str(b.verifier, 128)))
    )
      throw new Fault(401, "Sign-in could not be verified. Start again.");
    const claim = await env.DB.prepare(
      "UPDATE auth_requests SET used=1 WHERE id=? AND used=0 RETURNING id",
    )
      .bind(a.id)
      .first();
    if (!claim) throw new Fault(401, "Sign-in already used.");
    const device = await env.DB.prepare(
      "SELECT public_key,revoked_at FROM devices WHERE account=? AND id=?",
    )
      .bind(a.account, a.device)
      .first<{ public_key: string; revoked_at: number | null }>();
    if (device && (device.public_key !== a.public_key || device.revoked_at))
      throw new Fault(
        403,
        "Device identity changed. Start with a new device identity.",
      );
    if (!device && a.public_key) {
      const count = await env.DB.prepare(
        "SELECT COUNT(*) AS n FROM devices WHERE account=? AND revoked_at IS NULL AND public_key<>''",
      )
        .bind(a.account)
        .first<{ n: number }>();
      if ((count?.n ?? 0) >= 64)
        throw new Fault(409, "Remove an unused device before adding another.");
    }
    const raw = token();
    const t = now();
    await env.DB.batch([
      env.DB.prepare(
        "INSERT INTO devices(id,account,platform,public_key,created_at,last_seen_at) VALUES(?,?,?,?,?,?) ON CONFLICT(account,id) DO UPDATE SET last_seen_at=excluded.last_seen_at",
      ).bind(a.device, a.account, a.platform, a.public_key, t, t),
      env.DB.prepare(
        "INSERT INTO sessions(hash,account,device,created_at,expires_at,fresh_at,workos_user,workos_session,provider_revoked) VALUES(?,?,?,?,?,?,?,?,EXISTS(SELECT 1 FROM workos_revocations WHERE (kind='user' AND subject=?) OR (kind='session' AND subject=?)))",
      ).bind(
        await sha(raw),
        a.account,
        a.device,
        t,
        t + 90 * 86400,
        a.auth_time ?? 0,
        a.workos_user ?? null,
        a.workos_session ?? null,
        a.workos_user ?? null,
        a.workos_session ?? null,
      ),
    ]);
    if (!device && a.public_key)
      await env.DB.prepare(
        "INSERT INTO notifications(id,account,kind,created_at) VALUES(?,?,'new-device',?)",
      )
        .bind(crypto.randomUUID(), a.account, t)
        .run();
    const account = await env.DB.prepare(
      "SELECT id,email,epoch FROM accounts WHERE id=?",
    )
      .bind(a.account)
      .first();
    const response = json({
      account,
      device: a.device,
      ...(a.redirect === "web" ? {} : { token: raw }),
    });
    if (a.redirect === "web")
      response.headers.set(
        "Set-Cookie",
        `sj_session=${raw}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${90 * 86400}`,
      );
    return response;
  }
  if (path === "/v1/auth/signout" && request.method === "POST") {
    const s = await session(request, env);
    await env.DB.prepare("DELETE FROM sessions WHERE hash=?")
      .bind(s.hash)
      .run();
    const logout = s.workos_session
      ? (await import("./authkit"))
          .client(env)
          .userManagement.getLogoutUrl({
            sessionId: s.workos_session,
            returnTo: origin + "/account",
          })
      : null;
    const r = json({ signedOut: true, url: logout });
    r.headers.set(
      "Set-Cookie",
      "sj_session=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0",
    );
    return r;
  }
  throw new Fault(404, "Not found.");
}
export async function devicesAndWraps(
  request: Request,
  env: Env,
): Promise<Response | null> {
  const u = new URL(request.url);
  if (
    !u.pathname.startsWith("/v1/devices") &&
    !u.pathname.startsWith("/v1/wraps")
  )
    return null;
  const s = await session(request, env);
  await limit(env, "metadata:" + s.account, 180, 60);
  if (u.pathname === "/v1/devices" && request.method === "GET")
    return json(
      (
        await env.DB.prepare(
          "SELECT id,platform,public_key,created_at,last_seen_at,revoked_at FROM devices WHERE account=? ORDER BY created_at",
        )
          .bind(s.account)
          .all()
      ).results,
    );
  if (u.pathname.startsWith("/v1/devices/") && request.method === "DELETE") {
    fresh(s);
    const verifier = await authorizeKeys(request, env, s);
    const device = id(u.pathname.slice("/v1/devices/".length));
    const b = await body(request);
    if (b.confirmed !== true) throw new Fault(400, "Confirm device removal.");
    const revoked = await env.DB.batch([
      env.DB.prepare(
        "UPDATE devices SET revoked_at=? WHERE account=? AND id=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?)",
      ).bind(now(), s.account, device, s.account, verifier),
      env.DB.prepare(
        "DELETE FROM sessions WHERE account=? AND device=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?)",
      ).bind(s.account, device, s.account, verifier),
      env.DB.prepare(
        "DELETE FROM wraps WHERE account=? AND kind='device' AND ref=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?)",
      ).bind(s.account, device, s.account, verifier),
      env.DB.prepare(
        "UPDATE pairings SET state='denied',ciphertext=NULL WHERE account=? AND (device=? OR approver=?) AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?)",
      ).bind(s.account, device, device, s.account, verifier),
    ]);
    if (revoked[0].meta.changes !== 1)
      throw new Fault(409, "Device or encryption authorization changed.");
    return json({ removed: true });
  }
  native(s);
  if (u.pathname === "/v1/wraps" && request.method === "GET")
    return json(
      (
        await env.DB.prepare(
          "SELECT kind,ref,ciphertext,rev FROM wraps WHERE account=?",
        )
          .bind(s.account)
          .all()
      ).results,
    );
  if (u.pathname === "/v1/wraps" && request.method === "PUT") {
    const verifier = await authorizeKeys(request, env, s);
    const b = await body(request, 65536);
    const kind = str(b.kind, 16);
    const ref = id(b.ref);
    const ciphertext = str(b.ciphertext, 44000);
    const previous = integer(b.previous);
    if (
      !["keyring", "recovery", "device"].includes(kind) ||
      (kind === "device" && ref !== s.device)
    )
      throw new Fault(400, "Invalid wrap.");
    if (kind !== "device" && ref !== "main")
      throw new Fault(400, "Invalid wrap reference.");
    if (!/^[A-Za-z0-9_-]+$/.test(ciphertext))
      throw new Fault(400, "Invalid wrap encoding.");
    const r = await env.DB.prepare(
      "INSERT INTO wraps(account,kind,ref,ciphertext,rev) SELECT ?,?,?,?,1 WHERE ?=0 AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?) ON CONFLICT(account,kind,ref) DO UPDATE SET ciphertext=excluded.ciphertext,rev=wraps.rev+1 WHERE wraps.rev=? RETURNING rev",
    )
      .bind(
        s.account,
        kind,
        ref,
        ciphertext,
        previous,
        s.account,
        verifier,
        previous,
      )
      .first<{ rev: number }>();
    // SQLite INSERT...SELECT does not reach the conflict handler when the SELECT is empty.
    if (!r && previous > 0) {
      const update = await env.DB.prepare(
        "UPDATE wraps SET ciphertext=?,rev=rev+1 WHERE account=? AND kind=? AND ref=? AND rev=? AND EXISTS(SELECT 1 FROM accounts WHERE id=? AND auth_verifier=?) RETURNING rev",
      )
        .bind(ciphertext, s.account, kind, ref, previous, s.account, verifier)
        .first<{ rev: number }>();
      if (update) return json(update);
    }
    if (!r) throw new Fault(412, "Wrap changed. Reload before saving.");
    return json(r);
  }
  throw new Fault(404, "Not found.");
}
