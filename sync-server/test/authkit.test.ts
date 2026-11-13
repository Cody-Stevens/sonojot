import { describe, expect, it } from "vitest";
import { exchange } from "../src/authkit";
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
