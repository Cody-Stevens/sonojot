import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  canContinue,
  downloadPlan,
  dwellProgress,
  microphoneConfirmed,
  onboardingSteps,
  onboardingDownloadIds,
  onboardingModelDetails,
  practiceResult,
  setupDownloadSettled,
  setupDownloadStatus,
  skipAllPlan,
  stepProgress,
  shouldApplyTourProfile,
  type SetupDownload,
} from "./onboarding";
import { plainProfileReason, profileSpeechMemory } from "./setup-profiles";
import { previewSnapshot } from "./protocol";
import { aiSettingsSchema } from "./ai";
import type { ModelEntry } from "./model-library";
import { PUNCTUATION_MODEL } from "./punctuation";

const mac = onboardingSteps("macos");
const windows = onboardingSteps("windows");
const ids = (steps: { id: string }[]) => steps.map((step) => step.id);
const find = (steps: ReturnType<typeof onboardingSteps>, id: string) =>
  steps.find((step) => step.id === id)!;

test("setup reads in eleven steps on a Mac and ten elsewhere", () => {
  assert.deepEqual(ids(mac), [
    "welcome",
    "terms",
    "permissions",
    "microphone",
    "shortcut",
    "dictation",
    "models",
    "downloads",
    "meetings",
    "where",
    "ready",
  ]);
  assert.deepEqual(
    ids(windows),
    ids(mac).filter((id) => id !== "permissions"),
  );
  assert.deepEqual(ids(onboardingSteps("linux")), ids(windows));
  // No long new dwell: both new steps are short.
  assert.ok(find(mac, "permissions").dwell <= 2000);
  assert.ok(find(mac, "downloads").dwell <= 2000);
});

test("step progress follows forward, Back and tour navigation independently of dwell", () => {
  for (const steps of [mac, windows]) {
    for (const step of [0, 1, 2, 3, 4, 5, 6, 7, 6, 5, 4, 3, 2, 1, 0]) {
      assert.equal(stepProgress(step, steps.length), (step + 1) / steps.length);
      for (const elapsed of [0, 2400, 60000]) {
        dwellProgress(steps[step], elapsed);
        assert.equal(
          stepProgress(step, steps.length),
          (step + 1) / steps.length,
        );
      }
    }
    assert.equal(stepProgress(steps.length - 1, steps.length), 1);
  }
});

test("guided screens enforce dwell; Ready can be left immediately", () => {
  for (const step of mac) {
    if (step.dwell > 0)
      assert.equal(canContinue(step, step.dwell - 1, true, true), false);
    assert.equal(canContinue(step, step.dwell, true, true), true);
    assert.equal(dwellProgress(step, -100), step.dwell === 0 ? 1 : 0);
    assert.equal(dwellProgress(step, step.dwell * 2), 1);
  }
});
test("microphone confirmation gates Continue; permissions, models and downloads do not", () => {
  const mic = find(mac, "microphone");
  assert.equal(canContinue(mic, 60000, false, true), false);
  assert.equal(canContinue(mic, 60000, true, true), true);
  for (const id of ["permissions", "models", "downloads"]) {
    const step = find(mac, id);
    assert.equal(canContinue(step, step.dwell, false, false), true, id);
  }
});

test("permissions come before the microphone check; downloads follow Models", () => {
  const order = ids(mac);
  assert.ok(order.indexOf("terms") < order.indexOf("permissions"));
  assert.ok(order.indexOf("permissions") < order.indexOf("microphone"));
  assert.equal(order.indexOf("downloads"), order.indexOf("models") + 1);
  assert.ok(order.indexOf("downloads") < order.indexOf("where"));
  assert.ok(order.indexOf("shortcut") < order.indexOf("dictation"));
  assert.ok(!order.includes("extras"));
});

test("terms require an explicit checkbox before the microphone step", () => {
  const terms = find(mac, "terms");
  assert.ok(ids(mac).indexOf("terms") < ids(mac).indexOf("microphone"));
  assert.equal(canContinue(terms, 60000, true), false);
  assert.equal(canContinue(terms, 60000, true, false), false);
  assert.equal(canContinue(terms, 0, false, true), true);
});

test("profile downloads are speech, punctuation and local notes without duplicates", () => {
  assert.deepEqual(
    onboardingDownloadIds({
      asrModel: "speech",
      cleanupModel: "cleanup",
      notesModel: "notes",
      reasons: [],
    }),
    ["speech", PUNCTUATION_MODEL, "notes"],
  );
  assert.deepEqual(
    onboardingDownloadIds({
      asrModel: "speech",
      cleanupModel: "notes",
      notesModel: "speech",
      reasons: [],
    }),
    ["speech", PUNCTUATION_MODEL],
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

const rows: SetupDownload[] = [
  { id: "speech", kind: "profile", settled: false },
  { id: PUNCTUATION_MODEL, kind: "profile", settled: true },
  { id: "notes", kind: "profile", settled: false },
  { id: "nemotron-3-diarization", kind: "extra", settled: false },
  { id: "ffmpeg", kind: "media", settled: false },
];

test("one download press consents once to every checked row", () => {
  const all = downloadPlan(rows, []);
  assert.deepEqual(all.consent, [
    "speech",
    "notes",
    "nemotron-3-diarization",
    "ffmpeg",
  ]);
  assert.deepEqual(all.skip, []);
  assert.deepEqual(all.profile, ["speech", "notes"]);
  assert.deepEqual(all.extras, ["nemotron-3-diarization"]);
  assert.equal(all.media, true);

  const some = downloadPlan(rows, ["notes", "ffmpeg", PUNCTUATION_MODEL]);
  assert.deepEqual(some.consent, ["speech", "nemotron-3-diarization"]);
  // Only pending profile models are skipped; installed ones need nothing.
  assert.deepEqual(some.skip, ["notes"]);
  assert.equal(some.media, false);

  const none = downloadPlan(rows, [
    "speech",
    "notes",
    "nemotron-3-diarization",
    "ffmpeg",
  ]);
  assert.deepEqual(none.consent, []);
  assert.deepEqual(none.skip, ["speech", "notes"]);
  assert.deepEqual(skipAllPlan(rows), ["speech", "notes"]);
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
    supported: true,
    stage: null,
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

test("download rows read Installed, Queued or a percentage and then lock", () => {
  assert.equal(setupDownloadStatus(model("a")), "Installed");
  assert.equal(setupDownloadStatus(model("a", "available")), "Not installed");
  assert.equal(setupDownloadStatus(model("a", "queued")), "Queued");
  assert.equal(
    setupDownloadStatus({ ...model("a", "downloading"), downloadedBytes: 45 }),
    "Downloading · 45%",
  );
  assert.equal(setupDownloadStatus(model("a", "failed")), "Didn't finish");
  assert.equal(setupDownloadSettled(model("a", "available")), false);
  assert.equal(setupDownloadSettled(model("a", "failed")), false);
  for (const state of ["installed", "queued", "downloading", "checking"])
    assert.equal(setupDownloadSettled(model("a", state)), true, state);
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
