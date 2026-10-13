import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  dictationRecordSchema,
  previewSnapshot,
  snapshotSchema,
} from "./protocol";
import {
  guardReasonLabel,
  historyVersions,
  publicationLabel,
  smartCleanupAvailable,
  smartRecoverySession,
} from "./smart-cleanup";

test("guard metadata has a readable label and tolerates newer reasons", () => {
  assert.equal(guardReasonLabel("number_changed"), "Number change blocked");
  assert.equal(guardReasonLabel("future_guard"), "Cleanup guard");
  assert.equal(publicationLabel("punctuation_model"), "Punctuation");
  assert.equal(publicationLabel("punctuation_rules"), "Punctuation rules");
  assert.equal(
    guardReasonLabel("layout:words_changed"),
    "Layout word change blocked",
  );
  assert.equal(guardReasonLabel("layout:timeout"), "Layout timed out");
  assert.equal(guardReasonLabel("layout:future_guard"), "Layout skipped");
});

test("Smart choice is available only in Custom and never selected by default", () => {
  for (const setupProfile of [
    null,
    "accurate",
    "balanced",
    "light",
    "custom",
  ] as const) {
    assert.equal(
      smartCleanupAvailable({
        ...previewSnapshot.settings,
        setupProfile,
      }),
      setupProfile === "custom",
    );
  }
  assert.notEqual(
    previewSnapshot.settings.cleanupModel,
    "speakoflow-mini-q4_k_m",
  );
});

test("metadata survives the native boundary and defaults for legacy records", () => {
  const legacy = dictationRecordSchema.parse({
    id: "old",
    createdAt: "2026-09-28",
    raw: "raw",
    published: "Raw.",
  });
  assert.equal(legacy.preModelText, null);
  assert.equal(legacy.publicationStage, null);
  assert.equal(legacy.guardReason, null);
  const record = dictationRecordSchema.parse({
    ...legacy,
    preModelText: "Raw raw.",
    publicationStage: "smart_cleanup",
    guardReason: "accepted",
  });
  assert.deepEqual(
    dictationRecordSchema.parse(JSON.parse(JSON.stringify(record))),
    record,
  );
  assert.deepEqual(
    historyVersions(record).map(({ title, text }) => [title, text]),
    [
      ["Before AI changes", "Raw raw."],
      ["Cleaned", "Raw."],
      ["As recognized", "raw"],
    ],
  );
});

test("recovery uses a stable session and rules text even with history disabled", () => {
  const snapshot = {
    ...previewSnapshot,
    resultSessionId: 7,
    recognitionCompleteness: "complete" as const,
    publicationStage: "smart_cleanup",
    preModelText: "Please please send it.",
    outputText: "Please send it.",
    originalText: "please please send it",
    settings: { ...previewSnapshot.settings, doNotSaveDictations: true },
  };
  assert.equal(smartRecoverySession(snapshot), 7);
  assert.equal(
    smartRecoverySession({ ...snapshot, publicationStage: "smart_layout" }),
    7,
  );
  for (const patch of [
    { resultSessionId: null },
    { sessionId: 8 },
    { meetingRecording: true },
    { publicationStage: "rules" },
    { publicationStage: "edited" },
    { preModelText: null },
    { preModelText: snapshot.outputText },
    { recognitionCompleteness: "incomplete" as const },
    ...(["loading", "listening", "finishing", "cleaning"] as const).map(
      (state) => ({ state }),
    ),
  ])
    assert.equal(smartRecoverySession({ ...snapshot, ...patch }), null);
});

test("layout history preserves prior text and names the final stage", () => {
  const record = dictationRecordSchema.parse({
    id: "layout",
    createdAt: "2026-10-07",
    raw: "hi alex send the draft thanks sam",
    published: "Hi Alex,\n\nSend the draft.\n\nThanks,\nSam",
    preModelText: "Hi Alex send the draft thanks Sam.",
    publicationStage: "smart_layout",
  });
  assert.equal(publicationLabel(record.publicationStage), "Shape by type");
  assert.equal(publicationLabel("layout_skipped"), "Layout skipped");
  assert.deepEqual(
    historyVersions(record).map((version) => version.title),
    ["Before AI changes", "Shaped", "As recognized"],
  );
});
