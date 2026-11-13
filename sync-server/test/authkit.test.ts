import { describe, expect, it } from "vitest";
import { exchange, sendCode, verifyCode } from "../src/authkit";
const env = {
  WORKOS_CLIENT_ID: "client_test",
  WORKOS_API_KEY: "synthetic-api-key",
  WORKOS_WEBHOOK_SECRET: "synthetic-signature-key",
} as Env;
const user = {
  object: "user",
  id: "user_test",
  email: "person@example.invalid",
  email_verified: true,
  first_name: null,
  last_name: null,
  profile_picture_url: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};
const seconds = () => Math.floor(Date.now() / 1000);
function mock(
  options: {
    verified?: boolean;
    mismatch?: boolean;
    stale?: boolean;
    reject?: boolean;
    malformed?: boolean;
  } = {},
) {
  const requests: { url: string; body: unknown }[] = [];
  const fetcher: typeof fetch = async (input, init) => {
    const url = String(input);
    requests.push({
      url,
      body: init?.body ? JSON.parse(String(init.body)) : null,
    });
    if (options.reject)
      return Response.json({ message: "Rejected" }, { status: 401 });
    if (url.endsWith("/authenticate"))
      return Response.json({
        user,
        access_token: options.malformed
          ? "bad"
          : "e30." +
            Buffer.from(
              JSON.stringify({
                sub: user.id,
                sid: "session_test",
                exp: seconds() + 300,
                auth_time: seconds() - (options.stale ? 700 : 0),
              }),
            ).toString("base64url") +
            ".synthetic",
        refresh_token: "discard-this-test-token",
      });
    return Response.json({
      ...user,
      email_verified: options.verified ?? true,
      id: options.mismatch ? "user_other" : user.id,
    });
  };
  return { fetcher, requests };
}
describe("WorkOS adapter", () => {
  it("exchanges the browser PKCE code and fetches the verified immutable user", async () => {
    const m = mock();
    const identity = await exchange(
      env,
      "synthetic-code",
      "synthetic-verifier",
      m.fetcher,
    );
    expect(identity).toMatchObject({
      provider: "workos",
      subject: "user_test",
      email: user.email,
      session: "session_test",
    });
    expect(m.requests.map((r) => r.url)).toEqual([
      "https://api.workos.com/user_management/authenticate",
      "https://api.workos.com/user_management/users/user_test",
    ]);
    expect(m.requests[0].body).toMatchObject({
      grant_type: "authorization_code",
      code_verifier: "synthetic-verifier",
      client_id: "client_test",
    });
    expect(JSON.stringify(identity)).not.toContain("discard-this-test-token");
  });
  it.each([{ verified: false }, { mismatch: true }])(
    "refuses an unverified or substituted user %j",
    async (options) => {
      await expect(
        exchange(env, "code", "verifier", mock(options).fetcher),
      ).rejects.toMatchObject({ status: 403 });
    },
  );
  it.each([{ stale: true }, { reject: true }, { malformed: true }])(
    "returns 4xx for rejected or stale authentication %j",
    async (options) => {
      await expect(
        exchange(env, "code", "verifier", mock(options).fetcher),
      ).rejects.toMatchObject({ status: 401 });
    },
  );
});

describe("headless identity adapter", () => {
  it("returns a server-only code and threads IP, browser and Radar metadata", async () => {
    const calls: Record<string, unknown>[] = [];
    const m = mock();
    const fetcher: typeof fetch = async (input, init) => {
      if (init?.body) calls.push(JSON.parse(String(init.body)));
      if (String(input).endsWith("/magic_auth"))
        return Response.json({
          object: "magic_auth",
          id: "magic_fixture",
          user_id: user.id,
          email: user.email,
          code: "123456",
          expires_at: new Date(Date.now() + 300000).toISOString(),
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          radar_auth_attempt_id: "radar_fixture",
        });
      return m.fetcher(input, init);
    };
    const signals = {
      ipAddress: "203.0.113.10",
      userAgent: "Synthetic browser",
    };
    const sent = await sendCode(env, user.email, signals, fetcher);
    expect(sent).toEqual({ code: "123456", radarAttempt: "radar_fixture" });
    const identity = await verifyCode(
      env,
      user.email,
      "123456",
      sent.radarAttempt,
      signals,
      fetcher,
    );
    expect(identity.subject).toBe(user.id);
    expect(calls[0]).toMatchObject({
      email: user.email,
      ip_address: signals.ipAddress,
      user_agent: signals.userAgent,
    });
    expect(calls[1]).toMatchObject({
      grant_type: "urn:workos:oauth:grant-type:magic-auth:code",
      radar_auth_attempt_id: "radar_fixture",
      ip_address: signals.ipAddress,
    });
    expect(JSON.stringify(identity)).not.toMatch(
      /123456|refresh_token|access_token/,
    );
  });
  it("fails closed on provider challenges, errors and unverified identities without revealing details", async () => {
    for (const code of [
      "radar_blocked",
      "radar_challenge",
      "mfa_challenge",
      "sso_required",
    ]) {
      const fetcher: typeof fetch = async () =>
        Response.json(
          {
            code,
            pending_authentication_token: "must-stay-secret",
            message: "private-provider-detail",
          },
          { status: 403 },
        );
      await expect(
        verifyCode(env, user.email, "123456", null, {}, fetcher),
      ).rejects.toMatchObject({
        status: 401,
        message: "This code could not be verified. Start again.",
      });
    }
    await expect(
      verifyCode(
        env,
        user.email,
        "123456",
        null,
        {},
        mock({ verified: false }).fetcher,
      ),
    ).rejects.toMatchObject({ status: 401 });
  });
});
