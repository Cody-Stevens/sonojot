import { WorkOS } from "@workos-inc/node";
import { decodeJwt } from "jose";
import { Fault, bytes, now } from "./common";

export type Identity = {
  provider: "workos";
  subject: string;
  email: string;
  session: string;
  authTime: number;
};
export type AuthRequest = {
  id: string;
  provider: string;
  nonce: string;
  challenge: string;
  device: string;
  public_key: string;
  platform: string;
  redirect: string;
  expires_at: number;
  account: string | null;
  grant_hash: string | null;
  link_account?: string | null;
  link_session?: string | null;
  link_verifier?: string | null;
  browser_hash?: string | null;
  callback_started?: number;
  workos_user?: string | null;
  workos_session?: string | null;
  auth_time?: number | null;
  intent?: "sign-up" | "sign-in";
  browser_origin?: string | null;
  email?: string | null;
  code_hash?: string | null;
  code_tries?: number;
  code_started?: number;
  radar_attempt?: string | null;
};

export function configured(env: Env): void {
  if (
    !env.WORKOS_CLIENT_ID ||
    !env.WORKOS_API_KEY ||
    !env.WORKOS_WEBHOOK_SECRET
  )
    throw new Fault(503, "Sign-in is not configured yet.");
}

export function client(env: Env, fetcher: typeof fetch = fetch): WorkOS {
  return new WorkOS(env.WORKOS_API_KEY, {
    clientId: env.WORKOS_CLIENT_ID,
    timeout: 8000,
    maxRetries: 0,
    fetchFn: async (input, init) => {
      const response = await fetcher(input, {
        ...init,
        redirect: "error",
        signal: AbortSignal.timeout(8000),
      });
      // Bound the SDK's JSON parser as well as the network request. No response is logged.
      const content = await bytes(response, 65536);
      return new Response(content, {
        status: response.status,
        headers: response.headers,
      });
    },
  });
}

export function authorize(
  env: Env,
  a: AuthRequest,
  origin: string,
  challenge: string,
): string {
  configured(env);
  return client(env).userManagement.getAuthorizationUrl({
    provider: "authkit",
    redirectUri: origin + "/v1/auth/callback",
    state: a.id,
    codeChallenge: challenge,
    codeChallengeMethod: "S256",
    // Every explicit sign-in can authorize a sensitive action for ten minutes.
    maxAge: 0,
    screenHint: "sign-in",
  });
}

export async function exchange(
  env: Env,
  code: string,
  verifier: string,
  fetcher: typeof fetch = fetch,
): Promise<Identity> {
  configured(env);
  try {
    const sdk = client(env, fetcher);
    const result = await sdk.userManagement.authenticateWithCode({
      code,
      codeVerifier: verifier,
    });
    return verifiedIdentity(env, result, fetcher);
  } catch (error) {
    if (error instanceof Fault) throw error;
    const status =
      error && typeof error === "object" && "status" in error
        ? error.status
        : null;
    if (
      typeof status === "number" &&
      status >= 400 &&
      status < 500 &&
      status !== 429
    )
      throw new Fault(401, "Sign-in could not be verified. Start again.");
    throw new Fault(503, "Sign-in is temporarily unavailable. Try again.");
  }
}

async function verifiedIdentity(
  env: Env,
  result: Awaited<
    ReturnType<
      ReturnType<typeof client>["userManagement"]["authenticateWithCode"]
    >
  >,
  fetcher: typeof fetch,
): Promise<Identity> {
  const sdk = client(env, fetcher);
  const user = await sdk.userManagement.getUser(result.user.id);
  if (
    !user.emailVerified ||
    user.id !== result.user.id ||
    !/^user_[A-Za-z0-9]+$/.test(user.id) ||
    !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(user.email) ||
    user.email.length > 254 ||
    result.impersonator
  )
    throw new Fault(403, "A verified email address is required.");
  // Only the HTTPS server-to-server exchange supplies this token. It is never accepted
  // from callers or stored. Decode solely to bind lifecycle events and fresh-auth time.
  let claims;
  try {
    claims = decodeJwt(result.accessToken);
  } catch {
    throw new Fault(401, "Sign-in could not be verified. Start again.");
  }
  if (
    claims.sub !== user.id ||
    typeof claims.sid !== "string" ||
    !/^session_[A-Za-z0-9]+$/.test(claims.sid) ||
    typeof claims.exp !== "number" ||
    claims.exp <= now() ||
    typeof claims.auth_time !== "number" ||
    !Number.isSafeInteger(claims.auth_time) ||
    claims.auth_time > now() + 30 ||
    claims.auth_time < now() - 600
  )
    throw new Fault(401, "Sign in again to confirm your identity.");
  return {
    provider: "workos",
    subject: user.id,
    email: user.email.toLowerCase(),
    session: claims.sid,
    authTime: Math.min(now(), claims.auth_time),
  };
}

export type BrowserSignals = { ipAddress?: string; userAgent?: string };
export async function sendCode(
  env: Env,
  email: string,
  signals: BrowserSignals,
  fetcher: typeof fetch = fetch,
) {
  configured(env);
  const result = await client(env, fetcher).userManagement.createMagicAuth({
    email,
    ...signals,
  });
  if (
    result.email.toLowerCase() !== email ||
    !/^\d{6}$/.test(result.code) ||
    result.expiresAt.getTime() <= Date.now()
  )
    throw new Fault(503, "Sign-in is temporarily unavailable. Try again.");
  // The code is returned server-to-server, but default provider delivery stays on
  // until SonoJot's sending domain is approved. Never persist or log the plaintext.
  return { code: result.code, radarAttempt: result.radarAuthAttemptId ?? null };
}
export async function verifyCode(
  env: Env,
  email: string,
  code: string,
  radarAttempt: string | null,
  signals: BrowserSignals,
  fetcher: typeof fetch = fetch,
): Promise<Identity> {
  configured(env);
  try {
    const result = await client(
      env,
      fetcher,
    ).userManagement.authenticateWithMagicAuth({
      email,
      code,
      ...signals,
      ...(radarAttempt ? { radarAuthAttemptId: radarAttempt } : {}),
    });
    const identity = await verifiedIdentity(env, result, fetcher);
    if (identity.email !== email)
      throw new Fault(401, "This code could not be verified. Start again.");
    return identity;
  } catch {
    // Includes Radar blocks/challenges, MFA and SSO requirements: never bypass
    // them or redirect to a branded third-party page. No pending tokens escape.
    throw new Fault(401, "This code could not be verified. Start again.");
  }
}
