import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  backRoute,
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

test("stage doors share their screen while Settings retains its Back destination", () => {
  const chip = { tab: "dictation", stage: "speech" } as const;
  const door = { ...chip, from: "settings" } as const;
  assert.equal(routeTitle(chip), routeTitle(door));
  assert.deepEqual(backRoute(chip), { tab: "dictation" });
  assert.deepEqual(backRoute(door), { tab: "settings" });
  assert.equal(sameRoute(chip, door), false);
  assert.equal(sameRoute(chip, { ...chip }), true);
});

test("Back closes a sheet before leaving its stage", () => {
  assert.deepEqual(
    backRoute({ tab: "settings", stage: "help", sheet: "report" }),
    { tab: "settings", stage: "help", sheet: undefined },
  );
});
