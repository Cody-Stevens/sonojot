import { strict as assert } from "node:assert";
import { test } from "node:test";
import { parseResponse, previewSnapshot, updateStatusSchema } from "./protocol";
import {
  restartBlockedReason,
  updateAction,
  updateActionDisabled,
  updateNoticeMessage,
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
    noticeDismissed: true,
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

test("older snapshots without the notice flag parse as not dismissed", () => {
  const { noticeDismissed: _, ...update } = previewSnapshot.update;
  assert.equal(
    parseResponse("get_snapshot", { ...previewSnapshot, update }).update
      .noticeDismissed,
    false,
  );
});

const available = {
  state: "available",
  version: "0.28.1",
  notes: null,
  size: null,
} as const;
const readyStatus = { state: "ready", version: "0.28.1", notes: null } as const;

test("About and the notice offer the sidebar's next step in plain words", () => {
  assert.equal(updateAction({ state: "idle" }, false), null);
  assert.equal(updateAction({ state: "checking" }, false), null);
  assert.equal(
    updateAction(
      { state: "error", message: "Updates are unavailable." },
      false,
    ),
    null,
  );
  assert.deepEqual(updateAction(available, false), {
    kind: "download",
    label: "Download update",
  });
  assert.equal(
    updateAction({ state: "downloading", percent: 42 }, false),
    null,
  );
  assert.equal(
    updateAction({ state: "downloading", percent: null }, false),
    null,
  );
  assert.deepEqual(updateAction(readyStatus, false), {
    kind: "restart",
    label: "Restart to update",
  });
  assert.deepEqual(updateAction(readyStatus, true), {
    kind: "restart",
    label: "Restarting…",
  });
});

test("the shared action waits for downloads, restarts, blockers and pending calls", () => {
  const idle = () => false;
  assert.equal(updateActionDisabled(available, false, null, true, idle), false);
  assert.equal(updateActionDisabled(available, false, null, false, idle), true);
  assert.equal(
    updateActionDisabled(
      available,
      false,
      null,
      true,
      (key) => key === "update-download",
    ),
    true,
  );
  assert.equal(
    updateActionDisabled(
      { state: "downloading", percent: 5 },
      false,
      null,
      true,
      idle,
    ),
    true,
  );
  assert.equal(
    updateActionDisabled(readyStatus, false, null, true, idle),
    false,
  );
  assert.equal(
    updateActionDisabled(
      readyStatus,
      false,
      "Dictation is in progress.",
      true,
      idle,
    ),
    true,
  );
  assert.equal(
    updateActionDisabled(
      readyStatus,
      false,
      null,
      true,
      (key) => key === "update-install",
    ),
    true,
  );
  assert.equal(updateActionDisabled(readyStatus, true, null, true, idle), true);
  // A blocker for restart never blocks downloading.
  assert.equal(
    updateActionDisabled(
      available,
      false,
      "Dictation is in progress.",
      true,
      idle,
    ),
    false,
  );
});

test("the main-window notice covers available, downloading and ready until Later", () => {
  const notice = (
    status: Parameters<typeof updateNoticeMessage>[0]["status"],
    patch = {},
  ) =>
    updateNoticeMessage({
      status,
      installing: false,
      noticeDismissed: false,
      ...patch,
    });
  assert.equal(notice({ state: "idle" }), null);
  assert.equal(notice({ state: "checking" }), null);
  assert.equal(
    notice({ state: "error", message: "Updates are unavailable." }),
    null,
  );
  assert.equal(notice(available), "SonoJot 0.28.1 is available.");
  assert.equal(
    notice({ state: "downloading", percent: 42 }),
    "Downloading update · 42%",
  );
  assert.equal(notice(readyStatus), "SonoJot 0.28.1 is ready.");
  assert.equal(
    notice(readyStatus, { installing: true }),
    "Restarting to update…",
  );
  for (const status of [
    available,
    readyStatus,
    { state: "downloading", percent: 1 },
  ] as const)
    assert.equal(notice(status, { noticeDismissed: true }), null);
});
