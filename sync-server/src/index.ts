import { Fault, json, headers, configuredOrigin } from "./common";
import {
  auth,
  devicesAndWraps,
  productionAuth,
  type AuthDependencies,
} from "./auth";
import { webhook } from "./workos_webhooks";
import { testIdentity } from "./test_identity";
import { authorization } from "./authorization";
import { pairing } from "./pairing";
import { store } from "./store";
import { maintenance } from "./maintenance";
import { rotation } from "./rotation";
import { accountPage } from "./account";
import { health, serverFailure, requestCount } from "./monitoring";
export function worker(deps: AuthDependencies = productionAuth) {
  return {
    async fetch(request: Request, env: Env): Promise<Response> {
      try {
        configuredOrigin(request, env);
        const u = new URL(request.url);
        if (
          request.headers.get("origin") &&
          request.headers.get("origin") !== u.origin
        )
          throw new Fault(403, "This origin is not allowed.");
        if (u.pathname === "/v1/health") return headers(await health(env));
        const response =
          accountPage(request) ??
          (await webhook(request, env)) ??
          (await testIdentity(request, env)) ??
          (await auth(request, env, deps)) ??
          (await authorization(request, env)) ??
          (await devicesAndWraps(request, env)) ??
          (await pairing(request, env)) ??
          (await rotation(request, env)) ??
          (await store(request, env));
        return headers(response ?? json({ error: "Not found." }, 404));
      } catch (e) {
        if (!(e instanceof Fault) || e.status >= 500) {
          try {
            await serverFailure(env);
          } catch {}
        }
        return headers(
          json(
            {
              error:
                e instanceof Fault
                  ? e.message
                  : "The sync service could not finish this request. Try again.",
            },
            e instanceof Fault ? e.status : 503,
          ),
        );
      } finally {
        try {
          if (new URL(request.url).pathname !== "/v1/poll")
            await requestCount(env);
        } catch {}
      }
    },
    async scheduled(_event: ScheduledController, env: Env): Promise<void> {
      await maintenance(env);
    },
  };
}
export default worker();
