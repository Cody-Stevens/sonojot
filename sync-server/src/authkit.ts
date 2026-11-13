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
};

export function configured(env: Env): void {
  if (
    !env.WORKOS_CLIENT_ID ||
    !env.WORKOS_API_KEY ||
    !env.WORKOS_WEBHOOK_SECRET
  )
    throw new Fault(503, "WorkOS AuthKit is not configured yet.");
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
    const user = await sdk.userManagement.getUser(result.user.id);
    if (
      !user.emailVerified ||
      user.id !== result.user.id ||
      !/^user_[A-Za-z0-9]+$/.test(user.id) ||
      !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(user.email) ||
      user.email.length > 254 ||
      result.impersonator
    )
      throw new Fault(403, "A verified WorkOS identity is required.");
    // Only the HTTPS server-to-server exchange supplies this token. It is never accepted
    // from callers or stored. Decode solely to bind lifecycle events and fresh-auth time.
    let claims;
    try {
      claims = decodeJwt(result.accessToken);
    } catch {
      throw new Fault(
        401,
        "WorkOS sign-in could not be verified. Start again.",
      );
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
      throw new Fault(
        401,
        "WorkOS sign-in could not be verified. Start again.",
      );
    throw new Fault(
      503,
      "WorkOS sign-in is temporarily unavailable. Try again.",
    );
  }
}
