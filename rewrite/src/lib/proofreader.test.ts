import assert from "node:assert/strict";
import test from "node:test";
import { aiModelSchema } from "./ai";
import {
  dictationRecordSchema,
  previewSnapshot,
  settingsSchema,
  snapshotSchema,
} from "./protocol";
import {
  historyVersions,
  publicationLabel,
  smartRecoverySession,
} from "./smart-cleanup";
import {
  needsCloudNotice,
  needsTrainingAcceptance,
  proofreaderAccountBlocked,
  proofreaderGuardLabel,
  proofreaderModels,
  proofreaderMetadata,
  proofreaderOutcomeLabel,
  trainingAcceptance,
  trainingNotice,
} from "./proofreader";

const model = (id: string, patch = {}) =>
  aiModelSchema.parse({
    id,
    name: id,
    contextLength: 32000,
    promptPrice: null,
    completionPrice: null,
    ...patch,
  });

test("legacy settings keep proofreading off and a separate Luna choice", () => {
  const settings = settingsSchema.parse({
    ...previewSnapshot.settings,
    proofreader: undefined,
    proofreaderModel: undefined,
    proofreaderTrainingModels: undefined,
  });
  assert.equal(settings.proofreader, false);
  assert.equal(settings.proofreaderModel, "openai/gpt-6-luna");
  assert.deepEqual(settings.proofreaderTrainingModels, []);
  const next = {
    ...settings,
    proofreader: true,
    proofreaderModel: "mock/other",
    proofreaderTrainingModels: ["mock/other"],
  };
  assert.deepEqual(
    settingsSchema.parse(JSON.parse(JSON.stringify(next))),
    next,
  );
  assert.equal(next.notesModel, settings.notesModel);
});

test("catalog parsing retains capability metadata and safely defaults older catalogs", () => {
  const legacy = model("provider/legacy");
  assert.deepEqual(legacy.supportedParameters, []);
  assert.equal(legacy.trainingRequired, false);
  const training = model("provider/training", {
    supportedParameters: ["reasoning"],
    trainingRequired: true,
  });
  assert.deepEqual(training.supportedParameters, ["reasoning"]);
  assert.equal(training.trainingRequired, true);
  assert.equal(
    aiModelSchema.safeParse({ ...training, trainingRequired: "yes" }).success,
    false,
  );
});

test("proofreading search uses the text catalog including legacy models, never audio-only models", () => {
  const models = [
    model("one/legacy"),
    model("one/text", { inputModalities: ["text", "audio"] }),
    model("one/audio", { inputModalities: ["audio"] }),
  ];
  assert.deepEqual(
    proofreaderModels(models, "  ONE/ ").map(({ id }) => id),
    ["one/legacy", "one/text"],
  );
  assert.deepEqual(proofreaderModels(models, "missing"), []);
});

test("cloud consent requires the current notice, independent of connection or training choice", () => {
  assert.equal(needsCloudNotice({ componentAcknowledgments: {} }), true);
  assert.equal(
    needsCloudNotice({ componentAcknowledgments: { "openrouter-cloud": "0" } }),
    true,
  );
  assert.equal(
    needsCloudNotice({ componentAcknowledgments: { "openrouter-cloud": "1" } }),
    false,
  );
});

test("training permission belongs to the exact model and preserves earlier permissions", () => {
  const first = model("provider/model-contributor", { trainingRequired: true });
  const variant = model("provider/model-contributor:free", {
    trainingRequired: true,
  });
  const settings = { proofreaderTrainingModels: [first.id] };
  assert.equal(needsTrainingAcceptance(first, settings), false);
  assert.equal(needsTrainingAcceptance(variant, settings), true);
  assert.equal(
    needsTrainingAcceptance(model("provider/standard"), settings),
    false,
  );
  assert.deepEqual(trainingAcceptance(variant, settings), [
    first.id,
    variant.id,
  ]);
  assert.deepEqual(trainingAcceptance(first, settings), [first.id]);
  assert.deepEqual(settings.proofreaderTrainingModels, [first.id]);
  assert.match(trainingNotice(first), /dictation text and vocabulary/);
  assert.ok(trainingNotice(first).includes(first.id));
});

test("proofread history and current results retain all three text versions and metadata", () => {
  const record = dictationRecordSchema.parse({
    id: "proofread",
    createdAt: "2026-10-21",
    raw: "send the the link",
    published: "Send the link.",
    preModelText: "Send the the link.",
    publicationStage: "proofread",
    cleanupStatus:
      "rules;proofreader_model=openai/gpt-6-luna;proofreader_outcome=accepted;proofreader_reason=accepted",
  });
  assert.deepEqual(
    dictationRecordSchema.parse(JSON.parse(JSON.stringify(record))),
    record,
  );
  assert.deepEqual(proofreaderMetadata(record.cleanupStatus), {
    model: "openai/gpt-6-luna",
    outcome: "accepted",
    reason: "accepted",
  });
  assert.equal(proofreaderMetadata("rules;proofreader_model=only-model"), null);
  assert.equal(proofreaderMetadata(null), null);
  assert.equal(
    proofreaderOutcomeLabel("rejected"),
    "Proposal rejected; earlier text kept",
  );
  assert.equal(publicationLabel(record.publicationStage), "Proofread");
  assert.deepEqual(
    historyVersions(record).map(({ title, text }) => [title, text]),
    [
      ["Before AI changes", "Send the the link."],
      ["Proofread", "Send the link."],
      ["As recognized", "send the the link"],
    ],
  );
  const snapshot = snapshotSchema.parse({
    ...previewSnapshot,
    resultSessionId: 42,
    recognitionCompleteness: "complete",
    publicationStage: "proofread",
    preModelText: record.preModelText,
    originalText: record.raw,
    outputText: record.published,
    cleanupStatus: record.cleanupStatus,
  });
  assert.equal(smartRecoverySession(snapshot), 42);
  for (const patch of [
    { sessionId: 43 },
    { state: "listening" as const },
    { meetingRecording: true },
    { recognitionCompleteness: "incomplete" as const },
    { preModelText: record.published },
  ]) {
    assert.equal(smartRecoverySession({ ...snapshot, ...patch }), null);
  }
  assert.equal(
    dictationRecordSchema.parse({
      id: "old",
      createdAt: "old",
      raw: "",
      published: "",
    }).cleanupStatus,
    null,
  );
});

test("account privacy recovery only appears for the account training block", () => {
  assert.equal(
    proofreaderAccountBlocked("proofreader:account_training_blocked"),
    true,
  );
  assert.equal(
    proofreaderAccountBlocked("paid-model-training-violation-by-account"),
    true,
  );
  assert.equal(proofreaderAccountBlocked("proofreader:timeout"), false);
  assert.equal(proofreaderAccountBlocked(null), false);
  assert.equal(
    proofreaderGuardLabel("proofreader:timeout"),
    "Proofreading timed out; earlier text kept",
  );
  assert.equal(
    proofreaderGuardLabel("proofreader:deadline_exceeded"),
    "Proofreading timed out; earlier text kept",
  );
  assert.equal(proofreaderGuardLabel("proofreader:ok"), "Proofread");
  assert.equal(
    proofreaderGuardLabel("future_reason"),
    "Proofreading skipped; earlier text kept",
  );
});
