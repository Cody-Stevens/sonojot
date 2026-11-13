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

import {
  paretoFrontier,
  paretoPoints,
  speechLine,
  waitRange,
  waitTicks,
  memoryText,
  speechSources,
  otherRunScore,
  speechViews,
  viewScore,
} from "./speech-chart";
import type { ModelEntry } from "./model-library";

const entry = (id: string, backend = "CPU", diskBytes = 1e9) =>
  ({
    id,
    name: id,
    backend,
    diskBytes,
    languages: ["English"],
    ramMinBytes: 1e9,
    ramMaxBytes: 2e9,
    vramMinBytes: 0,
    vramMaxBytes: 0,
  }) as unknown as ModelEntry;

test("the Compare chart plots only models measured in the chosen run", () => {
  const points = paretoPoints(
    [
      entry("qwen3-asr-1.7b-q8-cuda", "CUDA"),
      entry(SONOJOT_PARAKEET),
      entry("whisper-large-v3-turbo"),
      entry("granite-speech-4.1-2b-nar-q8", "CUDA"),
      entry("moonshine-small"),
    ],
    false,
  );
  // Everyday speech by default; Moonshine has no everyday-speech run.
  assert.deepEqual(
    points.map((point) => [point.id, point.errors]),
    [
      ["qwen3-asr-1.7b-q8-cuda", 7.16],
      [SONOJOT_PARAKEET, 7.54],
    ],
  );
  assert.equal(points[0].gpu, true);
  // Talks come from one run too: the desktop study, not the October bench.
  assert.deepEqual(
    paretoPoints([entry("qwen3-asr-1.7b-q8-cuda", "CUDA")], false, "talks").map(
      (point) => point.errors,
    ),
    [2.6],
  );
  // Moonshine's talks score is real but from another run: listed, not plotted.
  assert.deepEqual(paretoPoints([entry("moonshine-small")], false, "talks"), []);
  assert.equal(otherRunScore("moonshine-small", false, "talks")?.value, 8.4);
  // On a Mac only the Mac app run counts, with SonoJot Parakeet's Mac wait.
  const mac = paretoPoints([entry(SONOJOT_PARAKEET)], true, "talks");
  assert.deepEqual(
    mac.map((point) => [point.errors, point.wait]),
    [[3.73, 0.23]],
  );
  assert.deepEqual(speechViews(true), ["talks"]);
  assert.equal(viewScore(SONOJOT_PARAKEET, true, "everyday"), null);
  assert.equal(viewScore("whisper-small-metal", true, "talks"), null);
});

test("the frontier keeps models nothing beats on both axes", () => {
  const ids = [
    "qwen3-asr-1.7b-q8-cuda",
    "qwen3-asr-1.7b-q4-cpu",
    "qwen3-asr-0.6b-q8-cpu",
    "parakeet-tdt-0.6b-v2",
    SONOJOT_PARAKEET,
  ];
  const models = ids.map((id) => entry(id, id.includes("cuda") ? "CUDA" : "CPU"));
  const everyday = paretoPoints(models, false, "everyday");
  assert.deepEqual(paretoFrontier(everyday), [
    SONOJOT_PARAKEET,
    "qwen3-asr-1.7b-q8-cuda",
    "qwen3-asr-1.7b-q4-cpu",
  ]);
  // Without a graphics card.
  assert.deepEqual(
    paretoFrontier(everyday.filter((point) => !point.gpu)),
    [SONOJOT_PARAKEET, "qwen3-asr-1.7b-q4-cpu"],
  );
  // Accented English: the small Qwen and SonoJot Parakeet lead.
  assert.deepEqual(paretoFrontier(paretoPoints(models, false, "accented")), [
    SONOJOT_PARAKEET,
    "qwen3-asr-0.6b-q8-cpu",
  ]);
  // Mac talks: Q8 makes more errors than Q4 and waits longer.
  const mac = paretoPoints(
    [
      entry("qwen3-asr-1.7b-q8-metal", "Metal"),
      entry("qwen3-asr-1.7b-q4-metal", "Metal"),
      entry(SONOJOT_PARAKEET),
    ],
    true,
    "talks",
  );
  assert.deepEqual(paretoFrontier(mac), [
    SONOJOT_PARAKEET,
    "qwen3-asr-1.7b-q4-metal",
  ]);
});

test("close waits still separate on the log axis, with readable ticks", () => {
  const [min, max] = waitRange([0.635, 0.685, 0.777]);
  assert.ok(max / min >= 1.6);
  assert.ok(waitTicks(min, max).length >= 2);
  const [wide, wider] = waitRange([0.11, 2]);
  assert.deepEqual(waitTicks(wide, wider), [0.1, 0.2, 0.5, 1, 2]);
});

test("speech card lines stay short and make only measured claims", () => {
  const cards = [
    entry(SONOJOT_PARAKEET),
    entry("qwen3-asr-1.7b-q8-cuda", "CUDA"),
    entry("qwen3-asr-1.7b-q4-cpu"),
    entry("qwen3-asr-0.6b-q8-cpu"),
    entry("parakeet-tdt-0.6b-v2"),
    entry("moonshine-medium"),
  ];
  for (const mac of [false, true])
    for (const card of cards) {
      const line = speechLine(card, { mac });
      assert.ok(line.split(/\s+/).length <= 8, line);
      assert.ok(!line.includes("—"), line);
    }
  assert.equal(
    speechLine(entry(SONOJOT_PARAKEET), { mac: false }),
    "English only. Fast and small.",
  );
  assert.match(
    speechLine(entry("qwen3-asr-1.7b-q8-cuda", "CUDA"), { mac: false }),
    /Many languages; fewer errors on prepared talks/,
  );
  // On a Mac there is no measured comparison with SonoJot Parakeet.
  assert.doesNotMatch(
    speechLine(entry("qwen3-asr-1.7b-q4-metal", "Metal"), { mac: true }),
    /fewest/,
  );
});

test("model details name measured memory and report sources, not code", () => {
  assert.equal(
    memoryText(entry(SONOJOT_PARAKEET)),
    "1.09 GB system memory",
  );
  assert.match(
    memoryText(entry("qwen3-asr-1.7b-q8-cuda", "CUDA")),
    /^3\.43 GB graphics memory and 1 GB system memory$/,
  );
  assert.match(memoryText(entry("moonshine-small")), /estimate/);
  for (const source of speechSources("qwen3-asr-1.7b-q4-cpu", false))
    assert.ok(source.endsWith(".md"), source);
});
