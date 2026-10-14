import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  MAC_PASTE_COPY,
  offerReset,
  PERMISSION_RESET_LABEL,
  permissionState,
} from "./permissions";
import { parseResponse, permissionStatusSchema } from "./protocol";

const status = {
  kind: "accessibility" as const,
  required: true,
  granted: false,
  assistantOpen: false,
  title: "Allow SonoJot to paste",
  body: "Switch SonoJot on in the list.",
  staleHint: "Already on but paste still doesn't work? Reset it.",
  canReset: true,
};

test("the reset is offered only after a try that left access off", () => {
  assert.equal(offerReset(status, false), false);
  assert.equal(offerReset(status, true), true);
  assert.equal(offerReset({ ...status, assistantOpen: true }, true), true);
  assert.equal(offerReset({ ...status, granted: true }, true), false);
  assert.equal(offerReset({ ...status, canReset: false }, true), false);
  // Windows needs no permission and has nothing to reset.
  assert.equal(
    offerReset({ ...status, required: false, canReset: false }, true),
    false,
  );
  assert.equal(offerReset(null, true), false);
});

test("reset copy replaces the remove-with-minus advice", () => {
  assert.equal(MAC_PASTE_COPY.reset, "Reset and ask again");
  assert.equal(PERMISSION_RESET_LABEL, MAC_PASTE_COPY.reset);
  assert.equal(
    MAC_PASTE_COPY.resetHelp,
    "If SonoJot is already switched on but paste still doesn't work, reset it. macOS will ask again; switch SonoJot on.",
  );
  const sources = [
    Object.values(MAC_PASTE_COPY).join(" "),
    readFileSync(
      new URL("../../src-tauri/src/permissions.rs", import.meta.url),
      "utf8",
    ),
    readFileSync(
      new URL("../components/PermissionHelper.svelte", import.meta.url),
      "utf8",
    ),
  ];
  for (const source of sources) {
    assert.doesNotMatch(source, /remove it with/i);
    assert.doesNotMatch(source, /\\u\{2212\}|−/);
  }
});

test("reset is a registered command that returns the status", () => {
  assert.deepEqual(parseResponse("permission_reset", status), status);
  const lib = readFileSync(
    new URL("../../src-tauri/src/lib.rs", import.meta.url),
    "utf8",
  );
  assert.match(lib, /permissions::permission_reset/);
});

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
