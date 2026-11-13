import { readFileSync } from "node:fs";
import { describe, it, expect } from "vitest";
import { stagingConfig, runStaging } from "../scripts/staging-guard.mjs";

const template = JSON.parse(
  readFileSync(new URL("../wrangler.jsonc", import.meta.url), "utf8"),
);
const settings = {
  origin: "https://sonojot-sync-staging.fixture.workers.dev",
  accountId: "1".repeat(32),
  databaseId: "11111111-1111-4111-8111-111111111111",
  workosClientId: "client_fixture",
};

describe("private staging deployment overlay", () => {
  it("resolves only deployment values without changing the public template", () => {
    const config = stagingConfig(template, {
      ...settings,
      routes: ["ignored.example.com/*"],
    });
    expect(config.env.staging.vars.PUBLIC_ORIGIN).toBe(settings.origin);
    expect(config.account_id).toBe(settings.accountId);
    expect(config.env.staging.d1_databases[0].database_id).toBe(
      settings.databaseId,
    );
    expect(config.env.staging.vars.WORKOS_CLIENT_ID).toBe(
      settings.workosClientId,
    );
    expect(config.routes).toBeUndefined();
    expect(template.account_id).toBeUndefined();
    expect(template.env.staging.d1_databases[0].database_id).toMatch(
      /^00000000/,
    );
  });

  it("rejects missing settings, placeholder IDs, production and unsafe origins", () => {
    expect(() => stagingConfig(template, {})).toThrow();
    expect(() =>
      stagingConfig(template, {
        ...settings,
        databaseId: "00000000-0000-0000-0000-000000000000",
      }),
    ).toThrow();
    for (const origin of [
      "",
      "https://sync.sonojot.com",
      "http://sonojot-sync-staging.fixture.workers.dev",
      settings.origin + "/path",
      settings.origin + "?query",
      settings.origin + "\n",
      settings.origin + ".evil.com",
      "https://user@sonojot-sync-staging.fixture.workers.dev",
    ]) {
      expect(() => stagingConfig(template, { ...settings, origin })).toThrow();
    }
  });

  it("keeps production resources and command overrides forbidden", () => {
    for (const mutate of [
      (c) => {
        c.routes = ["example.com/*"];
      },
      (c) => {
        c.env.production = {};
      },
      (c) => {
        c.workers_dev = true;
      },
      (c) => {
        c.env.staging.vars.ENVIRONMENT = "production";
      },
      (c) => {
        c.env.staging.d1_databases[0].database_name = "production";
      },
      (c) => {
        c.env.staging.r2_buckets[0].bucket_name = "production";
      },
    ]) {
      const bad = structuredClone(template);
      mutate(bad);
      expect(() => stagingConfig(bad, settings)).toThrow();
    }
    expect(() => runStaging(["deploy", "--env", "production"])).toThrow();
    expect(() => runStaging(["delete"])).toThrow();
  });
});
