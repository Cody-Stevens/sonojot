import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { compileModule } from "svelte/compiler";
import ts from "typescript";
import {
  aiModelSchema,
  aiSettingsSchema,
  modelRoles,
  priceLine,
  quickModelRemoval,
  routeNotes,
  type AiSettings,
} from "./ai";
import { selectable } from "./model-library";
import {
  modelSchema,
  parseResponse,
  settingsSchema,
  type Model,
} from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";

const base = {
  backend: "local" as const,
  model: "",
  connected: false,
  supported: true,
  help: "Fixture",
};

test("AI status migrates configured models only when the quick list is absent", () => {
  assert.deepEqual(aiSettingsSchema.parse(base).quickModels, []);
  const legacy = {
    ...base,
    model: "provider/text",
    speechModel: "provider/audio",
  };
  for (const command of [
    "get_ai_settings",
    "save_ai_settings",
    "connect_openrouter",
    "disconnect_openrouter",
  ] as const) {
    assert.deepEqual(parseResponse(command, legacy).quickModels, [
      { id: "provider/text", role: "text" },
      { id: "provider/audio", role: "speech" },
    ]);
  }
  assert.deepEqual(
    aiSettingsSchema.parse({ ...legacy, quickModels: [] }).quickModels,
    [],
  );
  assert.deepEqual(
    aiSettingsSchema.parse({ ...legacy, speechModel: legacy.model })
      .quickModels,
    [
      { id: "provider/text", role: "text" },
      { id: "provider/text", role: "speech" },
    ],
  );
  assert.equal(
    aiSettingsSchema.safeParse({ ...base, quickModels: null }).success,
    false,
  );
  assert.equal(
    aiSettingsSchema.safeParse({
      ...base,
      quickModels: [{ id: "provider/text", role: "cleanup" }],
    }).success,
    false,
  );
});

function model(patch = {}) {
  return aiModelSchema.parse({
    id: "provider/model",
    name: "Model",
    contextLength: 32000,
    promptPrice: "0.000001",
    completionPrice: "0.000002",
    ...patch,
  });
}

test("removing a quick model resets only its selected role", () => {
  const text = { id: "provider/model", role: "text" as const };
  const speech = { id: "provider/model", role: "speech" as const };
  const other = { id: "provider/other", role: "text" as const };
  const settings = aiSettingsSchema.parse({
    ...base,
    backend: "openrouter",
    model: text.id,
    speechModel: speech.id,
    quickModels: [text, speech, other],
  });
  assert.deepEqual(quickModelRemoval(settings, text), {
    backend: "local",
    quickModels: [speech, other],
  });
  assert.deepEqual(quickModelRemoval(settings, speech), {
    speechModel: "",
    quickModels: [text, other],
  });
  assert.deepEqual(quickModelRemoval(settings, other), {
    quickModels: [text, speech],
  });
  assert.deepEqual(settings.quickModels, [text, speech, other]);
});

test("catalog roles separate audio-only, multimodal and legacy text models", () => {
  assert.deepEqual(modelRoles(model()), ["text"]);
  assert.deepEqual(modelRoles(model({ inputModalities: ["audio"] })), [
    "speech",
  ]);
  assert.deepEqual(modelRoles(model({ inputModalities: ["text", "audio"] })), [
    "text",
    "speech",
  ]);
  assert.deepEqual(modelRoles(model({ inputModalities: ["image"] })), []);
});

test("prices retain the per-million units and never describe missing or invalid prices as free", () => {
  assert.equal(priceLine(model()), "$1 input, $2 output per 1M tokens");
  assert.equal(
    priceLine(model({ promptPrice: "0", completionPrice: "0" })),
    "$0 input, $0 output per 1M tokens",
  );
  assert.equal(
    priceLine(model({ promptPrice: "0.00000015", completionPrice: "0.00001" })),
    "$0.15 input, $10 output per 1M tokens",
  );
  for (const promptPrice of [
    null,
    "",
    " ",
    "unknown",
    "-1",
    "Infinity",
    "1e309",
  ]) {
    assert.equal(
      priceLine(model({ promptPrice })),
      "OpenRouter does not list a price for this model.",
    );
  }
  assert.match(
    priceLine(model({ promptPrice: "0.00000000001" })),
    /^<\$0.0001 input/,
  );
});

test("route notes preserve unavailable routes, variant warnings and qualified retention information", () => {
  assert.deepEqual(routeNotes(null, false), []);
  assert.match(routeNotes(null, true)[0], /could not be checked/);
  const notes = routeNotes(
    {
      availability: "unlisted",
      privacy: "unknown",
      variant: "batch",
      routes: 0,
      activeRoutes: 0,
    },
    false,
  );
  assert.equal(notes.length, 2);
  assert.match(notes[0], /asynchronous/);
  assert.match(notes[1], /does not list an active provider route/);
  assert.match(
    routeNotes(
      {
        availability: "listed",
        privacy: "zdrListed",
        variant: "free",
        routes: 2,
        activeRoutes: 1,
      },
      false,
    ).join(" "),
    /currently active listed routes appear/,
  );
  assert.match(
    routeNotes(
      {
        availability: "listed",
        privacy: "unknown",
        variant: null,
        routes: 1,
        activeRoutes: 1,
      },
      false,
    )[0],
    /account-level provider filters cannot be checked/,
  );
});

// Compile runes as plain server state to exercise the actual command ordering
// without a browser, native service or a second implementation of the store.
const stateSource = ts.transpileModule(
  readFileSync(new URL("./ai.svelte.ts", import.meta.url), "utf8"),
  {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
    },
  },
).outputText;
const stateCode = ts.transpileModule(
  compileModule(stateSource, { filename: "ai.svelte.js", generate: "server" })
    .js.code,
  {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.CommonJS,
    },
  },
).outputText;
type AiStore = typeof import("./ai.svelte").ai;
type SavedSettings = Pick<
  AiSettings,
  "backend" | "model" | "speechModel" | "quickModels"
>;

function harness(initial: Partial<AiSettings> = {}) {
  let saved = aiSettingsSchema.parse({ ...base, ...initial });
  const commands: { name: string; settings?: SavedSettings }[] = [];
  const failures = new Set<string>();
  let hold: (() => Promise<void>) | null = null;
  const pending = new Set<string>();
  const snapshotModels: Model[] = [];
  const app = {
    native: true,
    error: "",
    snapshot: {
      settings: { notesModel: "local/notes" },
      models: snapshotModels,
    },
    busy: (key: string) => pending.has(key),
    async perform<T>(
      key: string,
      action: () => Promise<T>,
    ): Promise<T | undefined> {
      pending.add(key);
      app.error = "";
      try {
        return await action();
      } catch (error) {
        app.error = error instanceof Error ? error.message : String(error);
      } finally {
        pending.delete(key);
      }
    },
    async saveSettings(patch: { notesModel: string }): Promise<void> {
      commands.push({ name: "save_settings" });
      if (failures.delete("save_settings")) {
        app.error = "Local save failed";
        return;
      }
      app.snapshot.settings = { ...app.snapshot.settings, ...patch };
    },
  };
  const dependencies: Record<string, unknown> = {
    "./api": {
      call: async (name: string, args?: { settings: SavedSettings }) => {
        commands.push({
          name,
          ...(args ? { settings: structuredClone(args.settings) } : {}),
        });
        const gate = hold;
        hold = null;
        if (gate) await gate();
        if (failures.delete(name)) throw new Error(`${name} failed`);
        if (name === "save_ai_settings" && args)
          saved = { ...saved, ...structuredClone(args.settings) };
        if (name === "connect_openrouter")
          saved = { ...saved, connected: true };
        if (name === "disconnect_openrouter")
          saved = { ...saved, connected: false, backend: "local" };
        return structuredClone(saved);
      },
    },
    "./app.svelte": { app },
    "./model-library.svelte": { library: { data: null } },
    "./model-library": { selectable },
    "./protocol": { settingsSchema },
    "./smart-cleanup": { SMART_CLEANUP_MODEL },
  };
  const exports: { ai?: AiStore } = {};
  new Function("require", "exports", stateCode)((name: string) => {
    assert.ok(name in dependencies, `Unexpected dependency: ${name}`);
    return dependencies[name];
  }, exports);
  assert.ok(exports.ai);
  return {
    ai: exports.ai,
    app,
    commands,
    failures,
    holdNext() {
      let release: () => void = () => {};
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      hold = () => gate;
      return release;
    },
  };
}

test("shared refresh is deduplicated and retains saved state on a later load failure", async () => {
  const { ai, commands, failures } = harness({ model: "provider/text" });
  const first = ai.refresh();
  assert.equal(ai.refresh(), first);
  assert.equal(ai.busy, true);
  await first;
  assert.equal(commands.length, 1);
  assert.equal(ai.busy, false);
  failures.add("get_ai_settings");
  await ai.refresh();
  assert.equal(ai.data?.model, "provider/text");
  await ai.refresh();
  assert.equal(ai.busy, false);
});

test("queued patches merge with the latest saved fields and publish successful saves", async () => {
  const { ai, commands, holdNext } = harness({ connected: true });
  await ai.refresh();
  const release = holdNext();
  const first = ai.save({ backend: "openrouter", model: "provider/text" });
  const quickModels = [{ id: "provider/audio", role: "speech" as const }];
  const second = ai.save({ quickModels, speechModel: "provider/audio" });
  quickModels[0].id = "provider/changed-after-save";
  await Promise.resolve();
  assert.equal(ai.busy, true);
  release();
  assert.deepEqual(await Promise.all([first, second]), [true, true]);
  assert.equal(ai.data?.model, "provider/text");
  assert.equal(ai.data?.backend, "openrouter");
  assert.equal(ai.data?.speechModel, "provider/audio");
  assert.deepEqual(ai.data?.quickModels, [
    { id: "provider/audio", role: "speech" },
  ]);
  assert.equal(
    commands.filter((entry) => entry.name === "get_ai_settings").length,
    1,
  );
  assert.equal(ai.busy, false);
});

test("failed initial loads and saves do not invent defaults or poison later actions", async () => {
  const { ai, failures, commands } = harness();
  failures.add("get_ai_settings");
  assert.equal(await ai.save({ speechModel: "provider/audio" }), false);
  assert.equal(Boolean(ai.data), false);
  assert.equal(
    commands.some((entry) => entry.name === "save_ai_settings"),
    false,
  );
  assert.equal(await ai.save({ speechModel: "provider/audio" }), true);
  failures.add("save_ai_settings");
  assert.equal(await ai.save({ speechModel: "provider/failed" }), false);
  assert.equal(ai.data?.speechModel, "provider/audio");
  assert.equal(
    await ai.save({
      model: "provider/text",
      speechModel: undefined,
      quickModels: undefined,
    }),
    true,
  );
  assert.equal(ai.data?.speechModel, "provider/audio");
});

test("connect, refresh and disconnect are ordered without reviving a stale connection", async () => {
  const { ai, commands } = harness();
  await Promise.all([ai.connect(), ai.refresh(), ai.disconnect()]);
  assert.deepEqual(
    commands.map((entry) => entry.name),
    ["connect_openrouter", "get_ai_settings", "disconnect_openrouter"],
  );
  assert.equal(ai.data?.connected, false);
  assert.equal(ai.data?.backend, "local");
  assert.equal(ai.busy, false);
});

test("returning to local repairs legacy Notes before saving AI settings or disconnecting", async () => {
  const { ai, app, commands, failures } = harness({
    backend: "openrouter",
    model: "provider/text",
  });
  app.snapshot.settings.notesModel = "openrouter";
  assert.equal(await ai.save({ backend: "local" }), true);
  assert.equal(
    app.snapshot.settings.notesModel,
    settingsSchema.shape.notesModel.parse(undefined),
  );
  assert.deepEqual(
    commands.map((entry) => entry.name),
    ["get_ai_settings", "save_settings", "save_ai_settings"],
  );
  app.snapshot.settings.notesModel = "openrouter";
  failures.add("save_settings");
  const prior = commands.length;
  await ai.disconnect();
  assert.deepEqual(
    commands.slice(prior).map((entry) => entry.name),
    ["save_settings"],
  );
  assert.match(app.error, /Local save failed/);
  await ai.disconnect();
  assert.equal(commands.at(-1)?.name, "disconnect_openrouter");
});

test("legacy Notes fallback excludes the dictation-only Smart cleanup model", async () => {
  const { ai, app } = harness({
    backend: "openrouter",
    model: "provider/text",
  });
  app.snapshot.settings.notesModel = "openrouter";
  app.snapshot.models.push(
    modelSchema.parse({
      id: SMART_CLEANUP_MODEL,
      name: "Smart cleanup",
      kind: "cleanup",
      ready: true,
      description: "",
      sizeBytes: 0,
    }),
  );
  assert.equal(await ai.save({ backend: "local" }), true);
  assert.equal(
    app.snapshot.settings.notesModel,
    settingsSchema.shape.notesModel.parse(undefined),
  );
});
