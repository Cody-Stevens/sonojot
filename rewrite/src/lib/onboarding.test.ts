import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  canContinue,
  dwellProgress,
  microphoneConfirmed,
  onboardingSteps,
  onboardingDownloadIds,
  onboardingModelDetails,
  practiceResult,
  stepProgress,
  shouldApplyTourProfile,
} from "./onboarding";
import { plainProfileReason, profileSpeechMemory } from "./setup-profiles";
import { previewSnapshot } from "./protocol";
import { aiSettingsSchema } from "./ai";
import type { ModelEntry } from "./model-library";
import { PUNCTUATION_MODEL } from "./punctuation";

test("step progress follows forward, Back and tour navigation independently of dwell", () => {
  for (const mode of ["first-run", "tour"]) {
    for (const step of [0, 1, 2, 3, 4, 5, 6, 7, 6, 5, 4, 3, 2, 1, 0]) {
      assert.equal(
        stepProgress(step),
        (step + 1) / onboardingSteps.length,
        `${mode}, step ${step + 1}`,
      );
      for (const elapsed of [0, 2400, 60000]) {
        dwellProgress(step, elapsed);
        assert.equal(stepProgress(step), (step + 1) / onboardingSteps.length);
      }
    }
  }
});

test("guided screens enforce dwell; Ready can be left immediately", () => {
  onboardingSteps.forEach(({ dwell }, step) => {
    if (dwell > 0)
      assert.equal(canContinue(step, dwell - 1, true, true), false);
    assert.equal(canContinue(step, dwell, true, true), true);
    assert.equal(dwellProgress(step, -100), dwell === 0 ? 1 : 0);
    assert.equal(dwellProgress(step, dwell * 2), 1);
  });
});
test("microphone confirmation gates Continue; model metadata does not", () => {
  const mic = onboardingSteps.findIndex((step) => step.id === "microphone");
  const models = onboardingSteps.findIndex((step) => step.id === "models");
  assert.equal(canContinue(mic, 60000, false, true), false);
  assert.equal(canContinue(mic, 60000, true, true), true);
  assert.equal(canContinue(models, 60000, true, true), true);
});

test("terms require an explicit checkbox before the microphone step", () => {
  const terms = onboardingSteps.findIndex((step) => step.id === "terms");
  const mic = onboardingSteps.findIndex((step) => step.id === "microphone");
  assert.ok(terms < mic);
  assert.equal(canContinue(terms, 60000, true), false);
  assert.equal(canContinue(terms, 60000, true, false), false);
  assert.equal(canContinue(terms, 0, false, true), true);
});

test("profile consent covers all local roles and punctuation without duplicate IDs", () => {
  assert.deepEqual(
    onboardingDownloadIds({
      asrModel: "speech",
      cleanupModel: "cleanup",
      notesModel: "notes",
      reasons: [],
    }),
    ["speech", "cleanup", "notes", PUNCTUATION_MODEL],
  );
  assert.deepEqual(
    onboardingDownloadIds({
      asrModel: "speech",
      cleanupModel: "notes",
      notesModel: "notes",
      reasons: [],
    }),
    ["speech", "notes", PUNCTUATION_MODEL],
  );
  assert.deepEqual(
    onboardingDownloadIds({
      asrModel: "speech",
      cleanupModel: "local-rules",
      notesModel: "openrouter",
      reasons: [],
    }),
    ["speech", PUNCTUATION_MODEL],
  );
});

const selection = {
  asrModel: "speech",
  cleanupModel: "rules",
  notesModel: "notes",
  reasons: [],
};
const ai = aiSettingsSchema.parse({
  backend: "local",
  model: "vendor/notes",
  connected: true,
  supported: true,
  help: "",
});
function model(id: string, state = "installed"): ModelEntry {
  return {
    id,
    name: `${id} name`,
    kind: "asr",
    roles: [],
    family: "fixture",
    languages: [],
    older: false,
    deprecated: false,
    diskBytes: 100,
    ramMinBytes: null,
    ramMaxBytes: null,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "",
    guidance: "",
    evidence: "",
    backend: "cpu",
    compatibility: "",
    state,
    downloadedBytes: 0,
    totalBytes: 100,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "",
    quantization: "",
    contextTokens: null,
  };
}
const models = [
  model("speech"),
  model("rules"),
  model("notes"),
  model(PUNCTUATION_MODEL),
];

test("Custom model details use the saved online route and exclude its local download", () => {
  const catalog = models.filter((model) => model.id !== "notes");
  for (const notesModel of ["openrouter", "notes"]) {
    const result = onboardingModelDetails(
      { ...selection, notesModel },
      catalog,
      true,
      { ...ai, backend: "openrouter" },
    );
    assert.deepEqual(result, {
      speech: "speech name",
      notes: "vendor/notes (OpenRouter)",
      downloadBytes: 0,
    });
  }
  assert.equal(
    onboardingModelDetails(selection, models, false, {
      ...ai,
      backend: "openrouter",
    }).notes,
    "notes name",
  );
});

test("unknown or checking local entries stay unavailable; known downloads count once", () => {
  assert.deepEqual(onboardingModelDetails(selection, null, true, null), {
    speech: null,
    notes: null,
    downloadBytes: null,
  });
  const absent = onboardingModelDetails(
    { ...selection, notesModel: "missing" },
    models,
    true,
    ai,
  );
  assert.equal(absent.notes, null);
  assert.equal(absent.downloadBytes, null);
  assert.equal(
    onboardingModelDetails(
      selection,
      [model("speech", "checking"), ...models.slice(1)],
      true,
      ai,
    ).downloadBytes,
    null,
  );
  assert.equal(
    onboardingModelDetails(
      { ...selection, cleanupModel: "notes" },
      [
        model("speech"),
        model("notes", "available"),
        model(PUNCTUATION_MODEL, "available"),
      ],
      true,
      ai,
    ).downloadBytes,
    200,
  );
});

test("practice uses new published text, never stale, interim, cancelled or meeting output", () => {
  const snapshot = {
    ...previewSnapshot,
    state: "idle" as const,
    resultSessionId: 42,
    outputText: "Published words.",
    originalText: "raw words",
  };
  assert.equal(practiceResult(snapshot, 41), "Published words.");
  assert.equal(practiceResult(snapshot, 42), null);
  assert.equal(
    practiceResult({ ...snapshot, resultSessionId: null }, 41),
    null,
  );
  assert.equal(
    practiceResult(
      { ...snapshot, state: "listening", interimText: "interim" },
      41,
    ),
    null,
  );
  assert.equal(practiceResult({ ...snapshot, state: "cleaning" }, 41), null);
  assert.equal(practiceResult({ ...snapshot, outputText: "" }, 41), null);
  assert.equal(
    practiceResult({ ...snapshot, meetingRecording: true }, 41),
    null,
  );
});
test("speech-level input requires consecutive samples, not a click", () => {
  let count = microphoneConfirmed(0.08, 0);
  assert.equal(count, 1);
  assert.equal(microphoneConfirmed(0.001, count), 0);
  count = microphoneConfirmed(0.1, count);
  assert.equal(microphoneConfirmed(0.1, count), 3);
});
test("unchanged tours reapply no profile, including Custom", () => {
  for (const profile of ["accurate", "balanced", "light", "custom"] as const) {
    assert.equal(shouldApplyTourProfile(profile, profile), false);
  }
  assert.equal(shouldApplyTourProfile("custom", "balanced"), true);
});
test("plain recommendation has no dependency on diagnostic reason strings", () => {
  assert.equal(
    plainProfileReason("light", undefined).includes("benchmark"),
    false,
  );
  assert.equal(
    profileSpeechMemory(undefined, "windows").label,
    "Speech memory estimate unavailable",
  );
});
