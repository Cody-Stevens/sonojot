import { strict as assert } from "node:assert";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { compileModule } from "svelte/compiler";
import ts from "typescript";
import { previewSnapshot, parseResponse } from "./protocol";
import {
  checking,
  installed,
  installing,
  modelEntrySchema,
  removalBlock,
  selectable,
  type ModelLibrary,
} from "./model-library";
import {
  PUNCTUATION_MODEL,
  profileIncludesPunctuation,
  punctuationRecommendation,
  punctuationModelPage,
} from "./punctuation";
import {
  applyProfileSelection,
  hardwareLine,
  hardwareProfileSchema,
  missingDownloadBytes,
  profileLabel,
  profileSpeechMemory,
  sameProfileModels,
  selectedProfile,
  setupProfiles,
  showCustomModels,
  type ProfileOperations,
} from "./setup-profiles";

const selection = {
  asrModel: "speech",
  cleanupModel: "text",
  notesModel: "text",
  reasons: ["Fits memory"],
};
const hardware = {
  platform: "windows",
  appleSilicon: false,
  totalRamBytes: 32 * 2 ** 30,
  physicalCores: 8,
  logicalCores: 16,
  gpuName: "NVIDIA fixture GPU",
  gpuDriverVersion: null,
  gpuComputeCapability: "8.6",
  totalVramBytes: 8 * 2 ** 30,
  cudaOk: true,
};
const report = {
  hardware,
  recommendations: {
    accurate: selection,
    balanced: selection,
    light: selection,
  },
};
function model(id: string, state = "available") {
  return modelEntrySchema.parse({
    id,
    name: id === "speech" ? "Fixture speech" : "Fixture text",
    kind: id === "speech" ? "asr" : "cleanup",
    roles: id === "speech" ? ["asr"] : ["cleanup", "notes"],
    family: "Fixture",
    languages: [],
    older: false,
    diskBytes: 1e9,
    ramMinBytes: 1e9,
    ramMaxBytes: 2e9,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "Fixture",
    guidance: "Fixture",
    evidence: "Fixture",
    backend: "cpu",
    compatibility: "Fixture",
    state,
    downloadedBytes: 0,
    totalBytes: 1e9,
    error: null,
    installable: true,
    removable: false,
    usedBy: [],
    revision: "fixture",
    quantization: "fixture",
    contextTokens: null,
  });
}

function punctuation(state = "installed") {
  return {
    ...model(PUNCTUATION_MODEL, state),
    name: "SonoJot Punctuation v2",
    kind: "punctuation" as const,
    roles: ["punctuation"],
    diskBytes: 302728954,
    totalBytes: 302728954,
    removable: true,
  };
}

test("profile wire contract distinguishes unfinished setup from legacy fixtures", () => {
  const { setupProfile: _profile, ...legacy } = previewSnapshot.settings;
  assert.equal(
    parseResponse("get_snapshot", { ...previewSnapshot, settings: legacy })
      .settings.setupProfile,
    "custom",
  );
  assert.equal(
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...legacy, setupProfile: null },
    }).settings.setupProfile,
    null,
  );
  for (const profile of setupProfiles) {
    assert.equal(
      parseResponse("apply_setup_profile", {
        ...previewSnapshot,
        settings: { ...legacy, setupProfile: profile.id },
      }).settings.setupProfile,
      profile.id,
    );
  }
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...legacy, setupProfile: "fast" },
    }),
  );
  assert.deepEqual(parseResponse("get_hardware_profile", report), report);
  for (const patch of [
    { cudaOk: "yes" },
    { totalRamBytes: -1 },
    { physicalCores: 1.5 },
  ]) {
    assert.throws(() =>
      parseResponse("get_hardware_profile", {
        ...report,
        hardware: { ...hardware, ...patch },
      }),
    );
  }
  assert.throws(() =>
    parseResponse("get_hardware_profile", {
      ...report,
      recommendations: { balanced: selection },
    }),
  );
});

test("profile row names and explicit selection control visibility", () => {
  assert.deepEqual(
    setupProfiles.map(({ title, subtitle }) => [title, subtitle]),
    [
      ["Accurate", "Best transcription, more memory"],
      ["Balanced", "Accurate and responsive"],
      ["Light", "Fastest start, least memory"],
      ["Custom", "Choose every model yourself"],
    ],
  );
  assert.equal(selectedProfile(null, null), "balanced");
  assert.equal(selectedProfile("custom", null), "custom");
  assert.equal(selectedProfile("balanced", "light"), "light");
  assert.equal(showCustomModels(null, "custom"), true);
  assert.equal(showCustomModels("custom", "balanced"), false);
  assert.equal(showCustomModels("balanced", null), false);
  // Previewing Custom leaves the saved automatic profile available on return.
  for (const saved of ["accurate", "balanced", "light"] as const) {
    assert.equal(selectedProfile(saved, "custom"), "custom");
    assert.equal(showCustomModels(saved, "custom"), true);
    assert.equal(selectedProfile(saved, null), saved);
    assert.equal(showCustomModels(saved, saved), false);
  }
  assert.equal(profileLabel("balanced"), "Balanced");
});

test("Custom and automatic tabs preview without saving or installing", async () => {
  const javascript = ts.transpileModule(
    readFileSync(
      new URL("./setup-profiles.svelte.ts", import.meta.url),
      "utf8",
    ),
    {
      compilerOptions: {
        target: ts.ScriptTarget.ES2022,
        module: ts.ModuleKind.ESNext,
      },
    },
  ).outputText;
  const code = ts.transpileModule(
    compileModule(javascript, {
      filename: "setup-profiles.svelte.js",
      generate: "server",
    }).js.code,
    {
      compilerOptions: {
        target: ts.ScriptTarget.ES2022,
        module: ts.ModuleKind.CommonJS,
      },
    },
  ).outputText;
  for (const saved of [
    null,
    "accurate",
    "balanced",
    "light",
    "custom",
  ] as const) {
    const commands: string[] = [];
    const snapshot = structuredClone(previewSnapshot);
    snapshot.settings.setupProfile = saved;
    const app = {
      native: true,
      ready: true,
      snapshot,
      busy: () => false,
      perform: async () => {
        commands.push("perform");
      },
    };
    const dependencies: Record<string, unknown> = {
      "./api": {
        call: async (command: string) => {
          commands.push(command);
        },
      },
      "./app.svelte": { app },
      "./download-consent.svelte": { ensureDownloadConsent: async () => true },
      "./ai.svelte": { ai: { busy: false } },
      "./model-library.svelte": { library: { data: { models: [] } } },
      "./model-library": { checking, installed, installing },
      "./protocol": { errorMessage: String },
      "./setup-profiles": {
        applyProfileSelection,
        profileLabel,
        selectedProfile,
        showCustomModels,
      },
    };
    const exports: {
      profiles?: typeof import("./setup-profiles.svelte").profiles;
    } = {};
    new Function("require", "exports", code)((name: string) => {
      assert.ok(name in dependencies, `Unexpected dependency: ${name}`);
      return dependencies[name];
    }, exports);
    assert.ok(exports.profiles);
    const state = exports.profiles;
    state.retrySpeechId = "previous-download";
    await state.choose("custom");
    assert.equal(state.selected, "custom");
    assert.equal(state.custom, true);
    assert.equal(state.retrySpeechId, "");
    assert.equal(snapshot.settings.setupProfile, saved);
    await state.choose("light");
    assert.equal(state.selected, "light");
    assert.equal(state.custom, false);
    assert.equal(snapshot.settings.setupProfile, saved);
    state.draft = null;
    assert.equal(state.selected, saved ?? "balanced");
    assert.deepEqual(commands, []);
  }
});

test("hardware and catalog summaries retain unknown values and count shared downloads once", () => {
  const parsed = hardwareProfileSchema.parse(report);
  assert.equal(
    hardwareLine(parsed.hardware),
    "NVIDIA fixture GPU · 8 GB graphics memory · 32 GB RAM · 8 cores",
  );
  assert.equal(
    hardwareLine({
      ...parsed.hardware,
      appleSilicon: true,
      platform: "macos",
      gpuName: null,
      totalVramBytes: null,
    }),
    "Apple Silicon · 32 GB unified memory · 8 cores",
  );
  assert.equal(
    hardwareLine({
      ...parsed.hardware,
      totalRamBytes: null,
      physicalCores: null,
      logicalCores: null,
      gpuName: null,
    }),
    "Graphics details unavailable · Memory unavailable · Core count unknown",
  );
  const models = [model("speech"), model("text")];
  assert.equal(missingDownloadBytes(selection, models), 2e9);
  assert.equal(
    missingDownloadBytes(selection, [
      model("speech", "installed"),
      model("text"),
    ]),
    1e9,
  );
  assert.equal(missingDownloadBytes(selection, [model("speech")]), null);
  assert.equal(
    missingDownloadBytes(selection, [
      { ...model("speech"), downloadedBytes: 5e8 },
      model("text", "installed"),
    ]),
    1e9,
  );
  assert.equal(
    missingDownloadBytes(selection, [
      model("speech", "checking"),
      model("text"),
    ]),
    null,
  );
  assert.equal(
    profileSpeechMemory(models[0], "windows").label,
    "Speech about 1.0 GB–2.0 GB RAM",
  );
  assert.match(
    profileSpeechMemory(
      { ...models[0], backend: "cuda", vramMinBytes: 3e9, vramMaxBytes: 4e9 },
      "windows",
    ).label,
    /3.0 GB–4.0 GB graphics memory/,
  );
  assert.equal(
    profileSpeechMemory(undefined, "windows").label,
    "Speech memory estimate unavailable",
  );
});

test("only measured Windows model IDs use observed peaks; other packages keep honest ranges", () => {
  for (const [id, label, measured] of [
    ["moonshine-medium", "Speech about 0.6 GB RAM", "0.56"],
    ["whisper-small", "Speech about 1.3 GB RAM", "1.28"],
    ["whisper-tiny.en", "Speech about 0.5 GB RAM", "0.53"],
    [
      "qwen3-asr-1.7b-q8-cuda",
      "Speech about 1 GB RAM + 3.4 GB graphics memory",
      "1",
    ],
  ]) {
    const memory = profileSpeechMemory(model(id), "windows");
    assert.equal(memory.label, label);
    assert.match(memory.detail, /BENCHMARK-WINDOWS-2026-09-28.md/);
    assert.ok(memory.detail.includes(`${measured} GB peak worker RAM`));
    assert.match(memory.detail, /one computer/);
  }
  for (const [id, platform] of [
    ["moonshine-small", "windows"],
    ["whisper-small.en", "windows"],
    ["whisper-small-metal", "macos"],
    ["moonshine-medium", "unknown"],
  ] as const) {
    const memory = profileSpeechMemory(model(id), platform);
    assert.match(memory.label, /1.0 GB–2.0 GB/);
    assert.match(memory.detail, /Estimated range, not a measured worker peak/);
  }
});

test("profile equivalence compares all three roles, independent of reasons", () => {
  assert.equal(
    sameProfileModels(selection, { ...selection, reasons: [] }),
    true,
  );
  for (const key of ["asrModel", "cleanupModel", "notesModel"] as const) {
    assert.equal(
      sameProfileModels(selection, { ...selection, [key]: "other" }),
      false,
    );
  }
});

test("background checking is neither an install nor ready for selection or removal", () => {
  const pending = model("speech", "checking");
  assert.equal(checking(pending), true);
  assert.equal(installed(pending), false);
  assert.equal(installing(pending), false);
  assert.equal(selectable(pending, "asr"), false);
  assert.equal(removalBlock(pending, []), "Checking…");
  assert.equal(installing(model("speech", "verifying")), true);
  assert.equal(checking(model("speech", "verifying")), false);
});

function operationFixture() {
  let catalog: ModelLibrary = {
    hardware: {
      totalRamBytes: null,
      availableRamBytes: null,
      dedicatedVramBytes: null,
    },
    guidance: "Fixture",
    models: [model("speech"), model("text"), punctuation()],
  };
  const calls: string[] = [];
  let snapshot = structuredClone(previewSnapshot);
  snapshot.settings.setupProfile = null;
  const operations: ProfileOperations = {
    readLibrary: async () => catalog,
    install: async (id) => {
      calls.push(`install:${id}`);
      catalog = {
        ...catalog,
        models: catalog.models.map((entry) =>
          entry.id === id ? { ...entry, state: "downloading" } : entry,
        ),
      };
      return catalog;
    },
    wait: async () => {
      calls.push("wait");
      catalog = {
        ...catalog,
        models: catalog.models.map((entry) =>
          installing(entry) ? { ...entry, state: "installed" } : entry,
        ),
      };
    },
    apply: async (profile) => {
      calls.push(`apply:${profile}`);
      snapshot = {
        ...snapshot,
        settings: { ...snapshot.settings, ...selection, setupProfile: profile },
      };
      return snapshot;
    },
    complete: async () => {
      calls.push("complete");
    },
  };
  return {
    operations,
    calls,
    snapshot: () => snapshot,
    catalog: () => catalog,
  };
}

test("speech becomes ready before the atomic apply; notes download starts after completion", async () => {
  const fixture = operationFixture();
  await applyProfileSelection("balanced", selection, fixture.operations, null);
  assert.deepEqual(fixture.calls, [
    "install:speech",
    "wait",
    "apply:balanced",
    "complete",
    "install:text",
  ]);
  assert.equal(
    fixture.snapshot().settings.cleanupMode,
    previewSnapshot.settings.cleanupMode,
  );
});

test("failed or cancelled speech downloads never commit settings or the completion marker", async () => {
  for (const state of ["failed", "cancelled", "available"]) {
    const fixture = operationFixture();
    const before = structuredClone(fixture.snapshot());
    fixture.operations.wait = async () => {
      fixture.catalog().models[0].state = state;
      fixture.catalog().models[0].error =
        state === "failed" ? "Network failed" : null;
    };
    await assert.rejects(
      applyProfileSelection("accurate", selection, fixture.operations, null),
      /Network failed|installation stopped/,
    );
    assert.deepEqual(fixture.snapshot(), before);
    assert.deepEqual(fixture.calls, ["install:speech"]);
  }
});

test("a rejected atomic apply cannot mark completion or start notes", async () => {
  const fixture = operationFixture();
  fixture.operations.apply = async () => {
    throw new Error("Save failed");
  };
  await assert.rejects(
    applyProfileSelection("balanced", selection, fixture.operations, null),
    /Save failed/,
  );
  assert.equal(fixture.snapshot().settings.setupProfile, null);
  assert.deepEqual(fixture.calls, ["install:speech", "wait"]);
});

test("notes start failure retains the committed profile and reports a post-setup error", async () => {
  const fixture = operationFixture();
  const install = fixture.operations.install;
  fixture.operations.install = (id) =>
    id === "text"
      ? Promise.reject(new Error("Notes network failed"))
      : install(id);
  await assert.rejects(
    applyProfileSelection("balanced", selection, fixture.operations, null),
    /SETUP_NOTES_DOWNLOAD: Notes network failed/,
  );
  assert.equal(fixture.snapshot().settings.setupProfile, "balanced");
  assert.equal(fixture.snapshot().settings.asrModel, "speech");
  assert.ok(fixture.calls.includes("complete"));
});

test("first Custom completion needs installed speech and never queues notes", async () => {
  const fixture = operationFixture();
  await assert.rejects(
    applyProfileSelection("custom", selection, fixture.operations, null),
    /Install your chosen speech/,
  );
  assert.deepEqual(fixture.calls, []);
  fixture.catalog().models[0].state = "installed";
  await applyProfileSelection("custom", selection, fixture.operations, null);
  assert.deepEqual(fixture.calls, ["apply:custom", "complete"]);
});

test("checking speech waits before deciding to install, including a check that finds missing files", async () => {
  for (const state of ["installed", "available"]) {
    const fixture = operationFixture();
    fixture.catalog().models[0].state = "checking";
    let checks = 0;
    const finishInstall = fixture.operations.wait;
    fixture.operations.wait = async () => {
      if (checks++ === 0) {
        assert.deepEqual(fixture.calls, []);
        fixture.catalog().models[0].state = state;
      } else await finishInstall();
    };
    await applyProfileSelection(
      "balanced",
      selection,
      fixture.operations,
      null,
    );
    assert.equal(
      fixture.calls.includes("install:speech"),
      state === "available",
    );
    assert.ok(fixture.calls.includes("apply:balanced"));
  }
});

test("checking notes defers only its download after setup commits", async () => {
  const fixture = operationFixture();
  fixture.catalog().models[0].state = "installed";
  fixture.catalog().models[1].state = "checking";
  const deferred = await applyProfileSelection(
    "balanced",
    selection,
    fixture.operations,
    null,
  );
  assert.equal(deferred, "text");
  assert.deepEqual(fixture.calls, ["apply:balanced", "complete"]);
});

test("unrelated checks do not delay a ready profile", async () => {
  const fixture = operationFixture();
  fixture.catalog().models[0].state = "installed";
  fixture.catalog().models[1].state = "installed";
  fixture.catalog().models.push(model("unrelated", "checking"));
  await applyProfileSelection("balanced", selection, fixture.operations, null);
  assert.deepEqual(fixture.calls, ["apply:balanced", "complete"]);
});

test("all first-run automatic profiles include v2 after speech before committing setup", async () => {
  assert.equal(PUNCTUATION_MODEL, "sonojot-punct-v2");
  for (const profile of ["accurate", "balanced", "light"] as const) {
    assert.equal(profileIncludesPunctuation(null, profile), true);
    const fixture = operationFixture();
    fixture.catalog().models[2].state = "available";
    await applyProfileSelection(profile, selection, fixture.operations, null);
    assert.deepEqual(fixture.calls, [
      "install:speech",
      "wait",
      `install:${PUNCTUATION_MODEL}`,
      "wait",
      `apply:${profile}`,
      "complete",
      "install:text",
    ]);
  }
  assert.equal(profileIncludesPunctuation(null, "custom"), false);
});

test("completed profiles and first-run Custom never automatically download punctuation", async () => {
  for (const saved of [
    null,
    "custom",
    "accurate",
    "balanced",
    "light",
  ] as const) {
    for (const chosen of ["custom", "accurate", "balanced", "light"] as const) {
      if (saved === null && chosen !== "custom") continue;
      assert.equal(profileIncludesPunctuation(saved, chosen), false);
      const fixture = operationFixture();
      fixture.catalog().models[0].state = "installed";
      fixture.catalog().models[1].state = "installed";
      fixture.catalog().models[2].state = "available";
      await applyProfileSelection(chosen, selection, fixture.operations, saved);
      assert.deepEqual(fixture.calls, [`apply:${chosen}`, "complete"]);
    }
  }
});

test("default download total includes missing punctuation and its runtime, but not installed files", () => {
  const models = [
    model("speech", "installed"),
    model("text", "installed"),
    punctuation("available"),
  ];
  assert.equal(missingDownloadBytes(selection, models, true), 302728954);
  models[2].totalBytes += 20_000_000;
  assert.equal(missingDownloadBytes(selection, models, true), 322728954);
  assert.equal(missingDownloadBytes(selection, models, false), 0);
  models[2].state = "checking";
  assert.equal(missingDownloadBytes(selection, models, true), null);
  models[2].state = "installed";
  assert.equal(missingDownloadBytes(selection, models, true), 0);
  models[2].state = "available";
  models[2].installable = false;
  assert.equal(missingDownloadBytes(selection, models, true), 0);
});

test("punctuation failure or cancellation leaves setup unfinished and an explicit retry resumes it", async () => {
  for (const state of ["failed", "cancelled", "available"]) {
    const fixture = operationFixture();
    fixture.catalog().models[0].state = "installed";
    fixture.catalog().models[2].state = "available";
    const finishInstall = fixture.operations.wait;
    fixture.operations.wait = async () => {
      fixture.catalog().models[2].state = state;
    };
    await assert.rejects(
      applyProfileSelection("balanced", selection, fixture.operations, null),
      /Punctuation model installation stopped/,
    );
    assert.equal(fixture.snapshot().settings.setupProfile, null);
    assert.deepEqual(fixture.calls, [`install:${PUNCTUATION_MODEL}`]);
    fixture.operations.wait = finishInstall;
    await applyProfileSelection(
      "balanced",
      selection,
      fixture.operations,
      null,
    );
    assert.equal(fixture.snapshot().settings.setupProfile, "balanced");
    assert.deepEqual(fixture.calls, [
      `install:${PUNCTUATION_MODEL}`,
      `install:${PUNCTUATION_MODEL}`,
      "wait",
      "apply:balanced",
      "complete",
      "install:text",
    ]);
  }
});

test("punctuation check resolves before deciding on a download; unsupported targets retain fallback", async () => {
  for (const state of ["installed", "available", "unsupported"]) {
    const fixture = operationFixture();
    fixture.catalog().models[0].state = "installed";
    fixture.catalog().models[2].state = "checking";
    const finishInstall = fixture.operations.wait;
    fixture.operations.wait = async () => {
      if (fixture.catalog().models[2].state === "checking") {
        fixture.catalog().models[2].state = state;
        fixture.catalog().models[2].installable = state !== "unsupported";
      } else await finishInstall();
    };
    await applyProfileSelection(
      "balanced",
      selection,
      fixture.operations,
      null,
    );
    assert.equal(
      fixture.calls.includes(`install:${PUNCTUATION_MODEL}`),
      state === "available",
    );
    assert.equal(fixture.snapshot().settings.setupProfile, "balanced");
  }
});

test("legacy settings offer punctuation until Install or Dismiss acknowledgment is saved", () => {
  const { punctuationV2RecommendationAcknowledged: _acknowledged, ...legacy } =
    previewSnapshot.settings;
  const models = [punctuation("available")];
  for (const profile of ["custom", "accurate", "balanced", "light"] as const) {
    const settings = parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...legacy, setupProfile: profile },
    }).settings;
    assert.equal(settings.punctuationV2RecommendationAcknowledged, false);
    assert.equal(
      punctuationRecommendation(settings, models)?.id,
      PUNCTUATION_MODEL,
    );
    const saved = parseResponse("save_settings", {
      ...previewSnapshot,
      settings: { ...settings, punctuationV2RecommendationAcknowledged: true },
    }).settings;
    assert.equal(punctuationRecommendation(saved, models), undefined);
    // The stored choice survives a restart and later model removal.
    assert.equal(
      punctuationRecommendation(JSON.parse(JSON.stringify(saved)), models),
      undefined,
    );
  }
  assert.equal(
    punctuationRecommendation(
      { ...previewSnapshot.settings, setupProfile: null },
      models,
    ),
    undefined,
  );
  for (const state of ["checking", "downloading", "installed", "busy"]) {
    assert.equal(
      punctuationRecommendation(previewSnapshot.settings, [punctuation(state)]),
      undefined,
    );
  }
  assert.equal(
    punctuationRecommendation(previewSnapshot.settings, [
      { ...models[0], installable: false },
    ]),
    undefined,
  );
  assert.equal(
    punctuationRecommendation(previewSnapshot.settings, []),
    undefined,
  );
});

test("v1 acknowledgment and installation do not suppress the one-time v2 recommendation", () => {
  const { punctuationV2RecommendationAcknowledged: _acknowledged, ...legacy } =
    previewSnapshot.settings;
  for (const acknowledged of [false, true]) {
    const settings = parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: {
        ...legacy,
        setupProfile: "custom",
        punctuationRecommendationAcknowledged: acknowledged,
      },
    }).settings;
    assert.equal(settings.punctuationV2RecommendationAcknowledged, false);
    for (const v1State of ["available", "installed", "checking"]) {
      const models = [
        { ...punctuation(v1State), id: "sonojot-punct-v1" },
        punctuation("available"),
      ];
      assert.equal(
        punctuationRecommendation(settings, models)?.id,
        "sonojot-punct-v2",
      );
      models[1].state = "installed";
      assert.equal(punctuationRecommendation(settings, models), undefined);
    }
  }
});

test("punctuation library pages identify each package", () => {
  assert.equal(
    punctuationModelPage("sonojot-punct-v1"),
    "https://huggingface.co/SonoJot/sonojot-punct-v1",
  );
  assert.equal(
    punctuationModelPage(PUNCTUATION_MODEL),
    "https://huggingface.co/SonoJot/sonojot-punct-v2",
  );
});
