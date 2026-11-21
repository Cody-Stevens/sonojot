import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  backRoute,
  railTab,
  resolveRoute,
  sameRoute,
  routeTitle,
  legacyRoutes,
} from "./route.svelte";

test("native page ids retain a working view under the four tabs", () => {
  for (const [id, route] of Object.entries(legacyRoutes)) {
    assert.deepEqual(resolveRoute(id), route);
    assert.ok(routeTitle(route));
  }
  assert.equal(resolveRoute("toString"), null);
  assert.equal(resolveRoute("missing"), null);
  assert.equal(resolveRoute(null), null);
});

test("stage doors share their screen while Settings retains its Back destination and rail state", () => {
  const chip = { tab: "dictation", stage: "speech" } as const;
  const direct = { ...chip, from: "settings" } as const;
  const grouped = { ...chip, from: "settings-group" } as const;
  const disk = { ...chip, from: "settings-disk" } as const;

  assert.equal(routeTitle(chip), routeTitle(direct));
  assert.equal(routeTitle(chip), routeTitle(grouped));
  assert.deepEqual(backRoute(chip), { tab: "dictation" });
  assert.deepEqual(backRoute(direct), { tab: "settings" });
  assert.deepEqual(backRoute(grouped), { tab: "settings", stage: "dictation" });
  assert.deepEqual(backRoute(backRoute(grouped)), { tab: "settings" });
  assert.deepEqual(backRoute(disk), { tab: "settings", stage: "disk" });

  assert.equal(railTab(chip), "dictation");
  assert.equal(railTab(direct), "settings");
  assert.equal(railTab(grouped), "settings");
  assert.equal(railTab(disk), "settings");

  assert.equal(sameRoute(chip, direct), false);
  assert.equal(sameRoute(direct, grouped), false);
  assert.equal(sameRoute(chip, { ...chip }), true);
});

test("Settings Meetings group steps back one level before returning to Settings", () => {
  const chip = { tab: "meetings", stage: "notes" } as const;
  const grouped = { ...chip, from: "settings-group" } as const;

  assert.equal(railTab(chip), "meetings");
  assert.equal(railTab(grouped), "settings");
  assert.deepEqual(backRoute(chip), { tab: "meetings" });
  assert.deepEqual(backRoute(grouped), { tab: "settings", stage: "meetings" });
  assert.equal(routeTitle(backRoute(grouped)), "Meetings settings");
  assert.deepEqual(backRoute(backRoute(grouped)), { tab: "settings" });
});

test("Back closes a sheet before leaving its stage", () => {
  assert.deepEqual(
    backRoute({ tab: "settings", stage: "help", sheet: "report" }),
    { tab: "settings", stage: "help", sheet: undefined },
  );
});
