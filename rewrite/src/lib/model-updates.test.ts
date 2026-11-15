import assert from "node:assert/strict";
import { test } from "node:test";
import {
  modelUpdatesSchema,
  noticeUpdates,
  updateAllLabel,
  updateFor,
  updateLabel,
  updateStatus,
  type ModelUpdate,
} from "./model-updates";

const speech = (state: ModelUpdate["state"] = "available"): ModelUpdate => ({
  id: "parakeet-tdt-0.6b-v2-sonojot-ft2",
  replaces: ["parakeet-tdt-0.6b-v2-sonojot-ft2"],
  name: "SonoJot Parakeet",
  note: "Makes fewer mistakes in meetings, at the same size and speed.",
  downloadBytes: 652_808_550,
  automatic: false,
  state,
  downloadedBytes: state === "downloading" ? 326_404_275 : 0,
  error: state === "failed" ? "The download stopped. SonoJot will try again later." : null,
});
const cleanup = (state: ModelUpdate["state"] = "downloading"): ModelUpdate => ({
  id: "sonojot-punct-v4",
  replaces: ["sonojot-punct-v3"],
  name: "SonoJot Punctuation v4",
  note: "Capitalises names and acronyms, and is a smaller download.",
  downloadBytes: 181_000_000,
  automatic: true,
  state,
  downloadedBytes: 0,
  error: state === "failed" ? "The update did not load on this computer. SonoJot kept the current version." : null,
});

test("the backend's update list parses", () => {
  const parsed = modelUpdatesSchema.parse([speech(), cleanup()]);
  assert.equal(parsed.length, 2);
  assert.throws(() => modelUpdatesSchema.parse([{ ...speech(), state: "done" }]));
});

test("automatic updates stay out of the notice unless they fail", () => {
  assert.deepEqual(noticeUpdates([speech(), cleanup()]).map((u) => u.id), [speech().id]);
  assert.deepEqual(noticeUpdates([cleanup("failed")]).map((u) => u.id), ["sonojot-punct-v4"]);
});

test("a Models page row finds the update that replaces it", () => {
  const list = [speech(), cleanup()];
  assert.equal(updateFor(list, "sonojot-punct-v3")?.id, "sonojot-punct-v4");
  assert.equal(updateFor(list, "parakeet-tdt-0.6b-v2-sonojot-ft2")?.name, "SonoJot Parakeet");
  assert.equal(updateFor(list, "qwen3-asr-1.7b"), undefined);
});

test("status and button copy is short and plain", () => {
  assert.equal(
    updateStatus(speech()),
    "SonoJot Parakeet has an update. Makes fewer mistakes in meetings, at the same size and speed.",
  );
  assert.equal(updateStatus(speech("downloading")), "Updating SonoJot Parakeet · 50%");
  assert.match(updateStatus(speech("failed")), /try again later/);
  assert.equal(updateLabel(speech()), "Update · 653 MB");
  assert.equal(updateAllLabel([speech(), speech()]), "Update all · 1.3 GB");
  for (const state of ["available", "queued", "downloading", "waiting", "installing", "failed"] as const) {
    assert.doesNotMatch(updateStatus(speech(state)), /\u2014/);
  }
});
