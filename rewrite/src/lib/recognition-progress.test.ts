import assert from "node:assert/strict";
import test from "node:test";
import { previewSnapshot } from "./protocol";
import { recognitionProgressLabel } from "./recognition-progress";

test("capture stays explicit while speech model loads", () => {
  assert.equal(
    recognitionProgressLabel({
      ...previewSnapshot,
      state: "loading",
      recognitionProgress: { stage: "loading", elapsedMs: 24100 },
    }),
    "Recording · loading model 24s",
  );
});

test("loading stays visible after capture Stop without implying capture resumed", () => {
  const snapshot = {
    ...previewSnapshot,
    state: "finishing" as const,
    recognitionProgress: { stage: "loading" as const, elapsedMs: 94_200 },
  };
  assert.equal(
    recognitionProgressLabel(snapshot),
    "Loading speech model · 94s",
  );
  assert.equal(
    recognitionProgressLabel({ ...snapshot, state: "cleaning" }),
    null,
  );
  assert.equal(recognitionProgressLabel({ ...snapshot, state: "idle" }), null);
});
