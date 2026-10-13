import { strict as assert } from "node:assert";
import { test } from "node:test";
import { parseResponse, previewSnapshot, updateStatusSchema } from "./protocol";
import {
  restartBlockedReason,
  updateButtonLabel,
  updateStatusLabel,
  updateTooltip,
} from "./updates";

test("updater boundary rejects invalid progress and incomplete ready states", () => {
  for (const status of [
    { state: "downloading", percent: -1 },
    { state: "downloading", percent: 101 },
    { state: "downloading", percent: Number.NaN },
    { state: "ready", version: "" },
    { state: "available", version: "", notes: null, size: null },
    { state: "available", version: "1.0.0", notes: null, size: -1 },
    { state: "available", version: "1.0.0", notes: null, size: 1.5 },
    { state: "available", version: "1.0.0", notes: null },
    { state: "installing" },
  ])
    assert.equal(updateStatusSchema.safeParse(status).success, false);
  assert.equal(
    updateStatusSchema.safeParse({ state: "downloading", percent: null })
      .success,
    true,
  );
});

test("update commands preserve backend gate and signed-release metadata", () => {
  const update = {
    currentVersion: "0.22.0",
    status: { state: "ready", version: "0.23.0", notes: "<b>Local notes</b>" },
    restartBlockedReason: "Wait for meeting transcription to finish.",
    installing: false,
  };
  for (const command of [
    "check_for_updates",
    "download_update",
    "restart_to_update",
  ] as const) {
    assert.deepEqual(
      parseResponse(command, { ...previewSnapshot, update }).update,
      update,
    );
  }
});

test("available metadata survives the boundary and explains explicit download", () => {
  const status = {
    state: "available",
    version: "0.24.0",
    notes: "Calmer pages.\n Faster notes.",
    size: 38_000_000,
  } as const;
  assert.deepEqual(updateStatusSchema.parse(status), status);
  assert.equal(updateStatusLabel(status), "SonoJot 0.24.0 is available.");
  assert.equal(updateButtonLabel(status, false), "Update available");
  assert.equal(
    updateTooltip(status, false, null),
    "SonoJot 0.24.0 · 38 MB. Calmer pages. Faster notes. Click to download. Nothing changes until you restart.",
  );
  assert.equal(
    updateTooltip({ ...status, notes: null, size: null }, false, null),
    "SonoJot 0.24.0. Click to download. Nothing changes until you restart.",
  );
  assert.ok(
    updateTooltip({ ...status, notes: "x".repeat(500) }, false, null).length <
      400,
  );
});

test("sidebar labels and help cover progress, restart blockers and retry", () => {
  assert.equal(updateButtonLabel({ state: "idle" }, false), "");
  assert.equal(
    updateButtonLabel({ state: "downloading", percent: null }, false),
    "Downloading…",
  );
  assert.equal(
    updateButtonLabel({ state: "downloading", percent: 45.8 }, false),
    "Downloading… 45%",
  );
  assert.equal(
    updateTooltip({ state: "downloading", percent: 45 }, false, null),
    "Keep working. You'll be asked to restart when it's ready.",
  );
  const ready = { state: "ready", version: "0.24.0", notes: null } as const;
  assert.equal(updateButtonLabel(ready, false), "Restart to update");
  assert.match(
    updateTooltip(ready, false, "Dictation is in progress."),
    /Save any edits first\. Dictation is in progress\./,
  );
  assert.equal(updateButtonLabel(ready, true), "Restarting…");
  assert.equal(
    updateTooltip(ready, true, "Wait."),
    "Restarting to finish updating SonoJot.",
  );
  const error = { state: "error", message: "Could not download." } as const;
  assert.equal(updateButtonLabel(error, false), "Update failed · Retry");
  assert.equal(
    updateTooltip(error, false, null),
    "Could not download. Click to check for updates again.",
  );
});

test("restart waits for backend work, unsaved edits and pending UI actions", () => {
  assert.equal(
    restartBlockedReason("Dictation is in progress.", true, ["save"]),
    "Dictation is in progress.",
  );
  assert.equal(
    restartBlockedReason(null, true, []),
    "Save or discard your edits before restarting.",
  );
  assert.equal(
    restartBlockedReason(null, false, ["settings"]),
    "Wait for the current action to finish.",
  );
  assert.equal(restartBlockedReason(null, false, []), null);
});

test("update status copy handles unknown download length without false completion", () => {
  assert.equal(
    updateStatusLabel({ state: "downloading", percent: null }),
    "Downloading update…",
  );
  assert.equal(
    updateStatusLabel({ state: "downloading", percent: 99.9 }),
    "Downloading update · 99%",
  );
  assert.equal(
    updateStatusLabel({
      state: "error",
      message: "Could not check for updates.",
    }),
    "Could not check for updates.",
  );
});
