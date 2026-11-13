// Test bundle only. This entry is never referenced by wrangler.jsonc.
import { worker } from "../src/index";
import { Fault } from "../src/common";
const app = worker({
  exchange: async (_env, code) => {
    if (
      ![
        "synthetic-provider-code",
        "synthetic-other-email",
        "synthetic-linked-code",
      ].includes(code)
    )
      throw new Fault(401, "Provider sign-in failed.");
    return {
      provider: "workos",
      subject:
        code === "synthetic-linked-code" ? "user_linked" : "user_synthetic",
      session: "session_synthetic",
      authTime: Math.floor(Date.now() / 1000),
      email:
        code === "synthetic-other-email"
          ? "changed@example.invalid"
          : "person@example.invalid",
    };
  },
  sendCode: async (_env, email) => {
    if (email.startsWith("reject")) throw new Fault(400, "synthetic rejection");
    return { code: "123456", radarAttempt: "synthetic-radar" };
  },
  verifyCode: async (_env, email, code) => {
    if (code !== "123456") throw new Fault(401, "Invalid code.");
    return { provider: "workos", subject: email === "person@example.invalid" ? "user_synthetic" : "user_new", session: "session_synthetic", email, authTime: Math.floor(Date.now() / 1000) };
  },
  mail: async () => {},
});

import { maintenance } from "../src/maintenance";
export default {
  ...app,
  async fetch(request: Request, env: Env) {
    if (new URL(request.url).pathname === "/__test/cron") {
      await maintenance(env);
      return new Response("ok");
    }
    return app.fetch(request, env);
  },
};
