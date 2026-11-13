import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  labelPositions,
  pointRadius,
  speechMeasurement,
  speechScore,
  waitText,
} from "./speech-chart";
import { profileChanges } from "./setup-profiles";
import { previewSnapshot } from "./protocol";
import { modelName } from "./model-names";
import {
  primarySpeechPool,
  shortSpeechName,
  SONOJOT_PARAKEET,
} from "./speech-chart";

test("each speech backend belongs to one panel, including unified-memory Metal", () => {
  for (const [backend, pool] of [
    ["CUDA", "vram"],
    ["Metal", "vram"],
    ["CPU", "ram"],
  ] as const)
    assert.equal(primarySpeechPool({ backend }), pool);
});
test("setup names resolve catalogs and never expose unknown ids", () => {
  assert.equal(
    modelName(
      "provider/cleanup",
      [],
      [{ id: "provider/cleanup", name: "Clear text" }],
    ),
    "Clear text",
  );
  assert.equal(modelName("provider/unknown", []), "Selected OpenRouter model");
  assert.equal(modelName("missing-model", []), "Selected model");
});

test("speech scores keep exact packages, corpus and platform separate", () => {
  assert.equal(
    speechScore("qwen3-asr-1.7b-q8-cuda", false, false)?.value,
    2.51,
  );
  assert.equal(
    speechScore("qwen3-asr-1.7b-q4-metal", true, false)?.value,
    3.24,
  );
  assert.equal(speechScore("qwen3-asr-1.7b-q4-metal", false, false), null);
  assert.equal(speechScore("qwen3-asr-1.7b-q8-cuda", true, false), null);
  assert.equal(
    speechScore("granite-speech-4.1-2b-nar-q8", false, true)?.value,
    7.24,
  );
  assert.equal(speechScore("qwen3-asr-1.7b-q4-metal", true, true), null);
  assert.equal(speechScore("unknown", false, false), null);
});
test("waits never substitute short-clip decoding for time after speech", () => {
  assert.equal(waitText("granite-speech-4.1-2b-nar-q8"), "not measured");
  assert.equal(waitText("qwen3-asr-1.7b-q4-cpu"), "2.00 s");
  assert.equal(waitText("parakeet-tdt-0.6b-v2"), "0.15 s");
  assert(
    pointRadius("parakeet-tdt-0.6b-v2") > pointRadius("qwen3-asr-1.7b-q4-cpu"),
  );
  assert.equal(
    speechMeasurement("qwen3-asr-1.7b-q8-cuda")?.vram?.measuredPeak,
    3.43,
  );
});
test("equal-quality Mac labels remain separated within their plot", () => {
  const positions = labelPositions([100, 100, 140], 52, 146, 32);
  assert(positions[0] >= 52);
  assert(positions[1] - positions[0] >= 32);
  assert(positions[2] - positions[1] >= 32);
  assert(positions[2] <= 146);
});
test("whole-setup comparison includes effective cleanup and both online routes", () => {
  const settings = {
    ...previewSnapshot.settings,
    proofreader: true,
    localProofreader: false,
    proofreaderModel: "provider/cleanup",
    meetingAsrModel: "granite-speech-4.1-2b-nar-q8",
  };
  const recommendation = { ...settings, cleanupModel: "local-rules" };
  const changes = profileChanges(settings, recommendation, {
    backend: "openrouter",
    model: "provider/notes",
    speechModel: "provider/speech",
  });
  assert.deepEqual(
    changes.map((change) => change.role),
    ["cleanup", "notes", "meetingSpeech"],
  );
  assert.equal(changes[0].current, "provider/cleanup");
  assert.equal(changes[2].recommended, "Same as dictation");
});
test("setup differences use the active SonoJot cleanup before an inactive online choice", () => {
  const settings = {
    ...previewSnapshot.settings,
    localProofreader: true,
    proofreader: true,
    proofreaderModel: "provider/cleanup",
  };
  const changes = profileChanges(settings, settings, null, true);
  assert.equal(changes[0].current, "SonoJot cleanup");
});
test("SonoJot Parakeet keeps its own short name, apart from stock Parakeet", () => {
  assert.equal(shortSpeechName(SONOJOT_PARAKEET, "x"), "SonoJot Parakeet");
  assert.equal(shortSpeechName("parakeet-tdt-0.6b-v2", "x"), "Parakeet");
});

test("a Mac shows SonoJot Parakeet's word errors measured on a Mac, not Windows ones", () => {
  // November 13, 2026: TED-12 through the app's worker on an M4 Mac mini.
  const mac = speechScore(SONOJOT_PARAKEET, true, false);
  assert.equal(mac?.value, 3.73);
  assert.match(String(mac?.source), /BENCHMARK-MAC-M4/);
  assert.equal(speechScore(SONOJOT_PARAKEET, false, false)?.value, 3.89);
  // A Windows run stored in the mac slot never appears on a Mac.
  assert.equal(speechScore("qwen3-asr-1.7b-q8-cuda", true, false), null);
});
