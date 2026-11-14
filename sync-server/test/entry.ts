// Test bundle only. This entry is never referenced by wrangler.jsonc.
import { worker } from "../src/index";
import { Fault } from "../src/common";
import type { Mail } from "../src/mail";
import { maintenance } from "../src/maintenance";
import { sendNotices } from "../src/notices";

// Messages "sent" in this isolate. Nothing leaves the test.
let outbox: Mail[] = [];
let failures = 0;
const mail = async (_env: Env, message: Mail) => {
  if (failures > 0 || message.to.startsWith("mailfail")) {
    failures = Math.max(0, failures - 1);
    throw new Fault(503, "synthetic mail failure");
  }
  outbox.push(message);
};

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
    return {
      code: "123456",
      expiresAt: Date.now() + 600000,
      radarAttempt: "synthetic-radar",
    };
  },
  verifyCode: async (_env, email, code) => {
    if (code !== "123456") throw new Fault(401, "Invalid code.");
    return { provider: "workos", subject: email === "person@example.invalid" ? "user_synthetic" : "user_new", session: "session_synthetic", email, authTime: Math.floor(Date.now() / 1000) };
  },
  mail,
});

export default {
  ...app,
  async fetch(request: Request, env: Env, ctx?: ExecutionContext) {
    const path = new URL(request.url).pathname;
    if (path === "/__test/cron") {
      await maintenance(env, mail);
      return new Response("ok");
    }
    if (path === "/__test/notices") {
      await sendNotices(env, mail);
      return new Response("ok");
    }
    if (path === "/__test/mail") {
      const sent = outbox;
      outbox = [];
      return Response.json(sent);
    }
    if (path === "/__test/mail-fail") {
      failures = Number(new URL(request.url).searchParams.get("count") ?? 1);
      return new Response("ok");
    }
    return app.fetch(request, env, ctx);
  },
};
