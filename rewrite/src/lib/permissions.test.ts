import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { MAC_PASTE_COPY, permissionState } from "./permissions";
import { parseResponse, permissionStatusSchema } from "./protocol";

const status = {
  kind: "accessibility" as const,
  required: true,
  granted: false,
  assistantOpen: false,
  title: "Allow SonoJot to paste",
  body: "Switch SonoJot on in the list.",
  staleHint: "Already on? Remove it with −, then add it again.",
};

test("permission state follows the grant and the open helper", () => {
  assert.equal(permissionState(null), "unknown");
  assert.equal(permissionState(status), "needed");
  assert.equal(
    permissionState({ ...status, assistantOpen: true }),
    "assisting",
  );
  assert.equal(
    permissionState({ ...status, granted: true, assistantOpen: true }),
    "granted",
  );
  // Windows needs no permission.
  assert.equal(permissionState({ ...status, required: false }), "granted");
});

test("permission status is validated at the command boundary", () => {
  assert.deepEqual(parseResponse("permission_status", status), status);
  assert.equal(
    permissionStatusSchema.safeParse({ ...status, kind: "camera" }).success,
    false,
  );
  assert.equal(parseResponse("permission_drag", null), null);
});

test("every kind the frontend knows is registered in Rust", () => {
  const rust = readFileSync(
    new URL("../../src-tauri/src/permissions.rs", import.meta.url),
    "utf8",
  );
  for (const kind of permissionStatusSchema.shape.kind.options)
    assert.match(rust, new RegExp(`kind: "${kind}"`));
});

test("Mac copy no longer says automatic paste is missing", () => {
  const copy = Object.values(MAC_PASTE_COPY).join(" ");
  assert.doesNotMatch(copy, /not implemented/i);
  assert.match(MAC_PASTE_COPY.needed, /Command\+V/);
  const onboarding = readFileSync(
    new URL("../views/Onboarding.svelte", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(onboarding, /not implemented/i);
});
