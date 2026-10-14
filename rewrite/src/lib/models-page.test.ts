import assert from "node:assert/strict";
import test from "node:test";
import {
  aiSettingsSchema,
  catalogRows,
  effortLabel,
  groupQuickModels,
  quickModelGroupRemoval,
  rolesLabel,
  type AiModel,
} from "./ai";
import {
  SPEECH_REMOVAL_BLOCKED,
  downloadPhaseLabel,
  modelEntrySchema,
  modelUnavailableReason,
  removalDetail,
  removalPlan,
  unavailableHere,
  type ModelEntry,
  type RemovalSelection,
} from "./model-library";
import { parseResponse } from "./protocol";
import {
  PUNCTUATION_MODEL,
  defaultCleanupName,
  defaultCleanupStatus,
  modelDisplayName,
} from "./punctuation";

function entry(id: string, patch: Partial<ModelEntry> = {}): ModelEntry {
  return {
    id,
    name: id,
    kind: "cleanup",
    roles: ["cleanup", "notes"],
    family: "Fixture",
    languages: ["en"],
    older: false,
    deprecated: false,
    diskBytes: 1e9,
    ramMinBytes: 1e9,
    ramMaxBytes: 2e9,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "Fictional estimate, not measured.",
    guidance: "",
    evidence: "Unrated",
    backend: "llama-server",
    compatibility: "Fixture",
    state: "installed",
    downloadedBytes: 1e9,
    totalBytes: 1e9,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "fixture",
    quantization: "fixture",
    contextTokens: null,
    supported: true,
    stage: null,
    ...patch,
  };
}

const rules = entry("local-rules", {
  name: "SonoJot Punctuation with your vocabulary",
  roles: ["cleanup"],
  diskBytes: 0,
  installable: false,
  removable: false,
});
const punctuation = (state: string, installable = true) =>
  entry(PUNCTUATION_MODEL, {
    name: "SonoJot Punctuation v2",
    kind: "punctuation",
    roles: ["punctuation"],
    state,
    installable,
  });
const lfm = entry("lfm2.5-1.2b", { name: "LFM2.5-1.2B Instruct" });
const qwen = entry("qwen3-4b-instruct-2507", {
  name: "Qwen3-4B-Instruct-2507",
});
const speech = entry("qwen3-asr", {
  name: "Qwen3-ASR",
  kind: "asr",
  roles: ["asr"],
});
const otherSpeech = entry("whisper-small", {
  name: "Whisper Small",
  kind: "asr",
  roles: ["asr"],
});

test("default Cleanup is named for the punctuation model in every state", () => {
  const installed = [rules, punctuation("installed")];
  assert.equal(
    modelDisplayName(rules, installed),
    "SonoJot Punctuation v2 with your vocabulary",
  );
  assert.equal(defaultCleanupStatus(installed), "Installed");
  const missing = [rules, punctuation("available")];
  assert.equal(
    modelDisplayName(rules, missing),
    "SonoJot Punctuation v2 with your vocabulary",
  );
  assert.equal(defaultCleanupStatus(missing), "Not installed");
  assert.equal(defaultCleanupStatus(missing, true), "Download included");
  assert.equal(
    defaultCleanupStatus([rules, punctuation("checking")]),
    "Checking…",
  );
  // Without an installable package the shared fallback stays "Local rules".
  const none = [rules, punctuation("available", false)];
  assert.equal(modelDisplayName(rules, none), "Local rules");
  assert.equal(defaultCleanupStatus(none), "");
  assert.equal(defaultCleanupName(none), modelDisplayName(rules, none));
  // Other models keep their own names.
  assert.equal(modelDisplayName(lfm, installed), "LFM2.5-1.2B Instruct");
});

const selection = (
  patch: Partial<RemovalSelection> = {},
): RemovalSelection => ({
  asrModel: speech.id,
  cleanupModel: "local-rules",
  notesModel: qwen.id,
  meetingAsrModel: "",
  ...patch,
});
const models = [
  rules,
  punctuation("installed"),
  speech,
  otherSpeech,
  lfm,
  qwen,
];
const nameOf = (id: string) => {
  const model = models.find((item) => item.id === id);
  return model ? modelDisplayName(model, models) : id;
};

test("an unused installed model can be removed without switching anything", () => {
  const plan = removalPlan(lfm, {
    selection: selection(),
    models,
    remoteNotes: false,
    nameOf,
  });
  assert.deepEqual(plan, { switches: [], blocked: null });
  assert.equal(removalDetail(plan), "Delete its downloaded model files.");
});

test("a model used for Notes stays removable and switches Notes first", () => {
  const plan = removalPlan(lfm, {
    selection: selection({ notesModel: lfm.id }),
    models,
    remoteNotes: false,
    recommended: { notesModel: qwen.id },
    nameOf,
  });
  assert.equal(plan.blocked, null);
  assert.deepEqual(plan.switches, [
    {
      setting: "notesModel",
      to: qwen.id,
      sentence: "Notes will switch to Qwen3-4B-Instruct-2507.",
    },
  ]);
  assert.equal(
    removalDetail(plan),
    "Notes will switch to Qwen3-4B-Instruct-2507. Its downloaded model files will be deleted.",
  );
});

test("a saved local Notes model is idle while Notes use OpenRouter", () => {
  const plan = removalPlan(lfm, {
    selection: selection({ notesModel: lfm.id }),
    models: [rules, speech, lfm],
    remoteNotes: true,
    nameOf,
  });
  assert.equal(plan.blocked, null);
  assert.deepEqual(plan.switches, [
    { setting: "notesModel", to: "openrouter", sentence: "" },
  ]);
  assert.equal(removalDetail(plan), "Delete its downloaded model files.");
  // With another local text model, that becomes the saved fallback quietly.
  assert.deepEqual(
    removalPlan(lfm, {
      selection: selection({ notesModel: lfm.id }),
      models,
      remoteNotes: true,
      nameOf,
    }).switches,
    [{ setting: "notesModel", to: qwen.id, sentence: "" }],
  );
});

test("Cleanup falls back to the punctuation model and meetings to Speech", () => {
  const plan = removalPlan(lfm, {
    selection: selection({ cleanupModel: lfm.id }),
    models,
    remoteNotes: false,
    nameOf,
  });
  assert.deepEqual(plan.switches, [
    {
      setting: "cleanupModel",
      to: "local-rules",
      sentence:
        "Cleanup will switch to SonoJot Punctuation v2 with your vocabulary.",
    },
  ]);
  const meeting = entry("granite", {
    kind: "asr",
    roles: ["meetingSpeech"],
    usedBy: ["meetingSpeech"],
  });
  assert.deepEqual(
    removalPlan(meeting, {
      selection: selection({ meetingAsrModel: "granite" }),
      models: [...models, meeting],
      remoteNotes: false,
      nameOf,
    }).switches,
    [
      {
        setting: "meetingAsrModel",
        to: "",
        sentence: "Meeting transcription will use your Speech model.",
      },
    ],
  );
});

test("Speech prefers the recommended replacement and blocks without one", () => {
  const plan = removalPlan(speech, {
    selection: selection(),
    models,
    remoteNotes: false,
    recommended: { asrModel: otherSpeech.id },
    nameOf,
  });
  assert.deepEqual(
    plan.switches.map((item) => item.to),
    [otherSpeech.id],
  );
  const blocked = removalPlan(speech, {
    selection: selection(),
    models: [rules, speech, qwen],
    remoteNotes: false,
    nameOf,
  });
  assert.deepEqual(blocked.switches, []);
  assert.equal(
    blocked.blocked,
    "SonoJot needs a speech model for dictation. To remove this one, install another speech model first.",
  );
  assert.equal(blocked.needs, "asr");
});

// Owner on the Mac: the only speech model's Remove explains itself and offers
// the fix; any other installed, runnable speech model unblocks it.
const qwenMetal = entry("qwen3-asr-1.7b-q4-metal", {
  name: "Qwen3-ASR 1.7B (Q4 Metal)",
  kind: "asr",
  roles: ["asr"],
  backend: "llama.cpp b10964 Metal",
});
const whisperMetal = entry("whisper-small-metal", {
  name: "Whisper Small · Metal",
  kind: "asr",
  roles: ["asr"],
  backend: "whisper.cpp · Apple Metal",
});
const moonshine = entry("moonshine-medium", {
  name: "Moonshine Medium Streaming",
  kind: "asr",
  roles: ["asr"],
  backend: "moonshine-ort-cpu",
});
const macSelection = selection({ asrModel: qwenMetal.id });

test("the only speech model stays, says why, and points to speech models", () => {
  const plan = removalPlan(qwenMetal, {
    selection: macSelection,
    models: [rules, qwenMetal, { ...whisperMetal, state: "available" }, qwen],
    remoteNotes: false,
    nameOf,
  });
  assert.deepEqual(plan, {
    switches: [],
    blocked:
      "SonoJot needs a speech model for dictation. To remove this one, install another speech model first.",
    needs: "asr",
  });
});

test("another installed speech model unblocks Remove and takes over Speech", () => {
  const plan = removalPlan(qwenMetal, {
    selection: macSelection,
    models: [rules, qwenMetal, whisperMetal, qwen],
    remoteNotes: false,
    // The saved profile recommends the model being removed; fall through.
    recommended: { asrModel: qwenMetal.id },
    nameOf: (id) => (id === whisperMetal.id ? whisperMetal.name : id),
  });
  assert.equal(plan.blocked, null);
  assert.equal(plan.needs, undefined);
  assert.deepEqual(plan.switches, [
    {
      setting: "asrModel",
      to: whisperMetal.id,
      sentence: "Speech will switch to Whisper Small · Metal.",
    },
  ]);
});

test("models this platform cannot run are never Speech replacements", () => {
  for (const impossible of [
    // Files left on disk, but the Mac cannot run the package.
    { ...moonshine, supported: false },
    // The catalog marks it unavailable outright.
    { ...moonshine, state: "unavailable", installable: false },
  ]) {
    const plan = removalPlan(qwenMetal, {
      selection: macSelection,
      models: [rules, impossible, qwenMetal, qwen],
      remoteNotes: false,
      recommended: { asrModel: moonshine.id },
      nameOf,
    });
    assert.equal(plan.blocked, SPEECH_REMOVAL_BLOCKED);
    assert.deepEqual(plan.switches, []);
  }
  // A runnable model after the impossible one is still found.
  const plan = removalPlan(qwenMetal, {
    selection: macSelection,
    models: [
      rules,
      { ...moonshine, supported: false },
      qwenMetal,
      whisperMetal,
    ],
    remoteNotes: false,
    nameOf,
  });
  assert.deepEqual(
    plan.switches.map((item) => item.to),
    [whisperMetal.id],
  );
});

test("pickers set aside models that cannot run here, not missing downloads", () => {
  assert.equal(unavailableHere({ ...moonshine, supported: false }), true);
  assert.equal(
    unavailableHere({ ...moonshine, state: "unavailable", installable: false }),
    true,
  );
  // Needs a download: still an ordinary choice.
  assert.equal(unavailableHere({ ...whisperMetal, state: "available" }), false);
  assert.equal(
    unavailableHere({ ...whisperMetal, state: "downloading" }),
    false,
  );
  assert.equal(unavailableHere(whisperMetal), false);
  assert.equal(
    modelUnavailableReason({
      ...moonshine,
      supported: false,
      compatibility:
        "This package requires Windows; choose a Metal model on this Mac.",
    }),
    "This package requires Windows; choose a Metal model on this Mac.",
  );
});

test("a download names the shared add-on it is fetching", () => {
  assert.equal(
    downloadPhaseLabel({ ...qwenMetal, state: "downloading" }),
    "Downloading",
  );
  assert.equal(
    downloadPhaseLabel({
      ...qwenMetal,
      state: "downloading",
      stage: "speech detector",
    }),
    "Downloading speech detector",
  );
  assert.equal(
    downloadPhaseLabel({
      ...qwenMetal,
      state: "downloading",
      stage: "model files",
    }),
    "Downloading model files",
  );
});

test("Notes without another text model gives a one-line reason", () => {
  const plan = removalPlan(lfm, {
    selection: selection({ notesModel: lfm.id }),
    models: [rules, speech, lfm, qwen].map((item) =>
      item.id === qwen.id ? { ...item, state: "available" } : item,
    ),
    remoteNotes: false,
    nameOf,
  });
  assert.equal(
    plan.blocked,
    "Notes uses this model. Install another text model or choose an OpenRouter model first.",
  );
  assert.equal(plan.needs, "notes");
});

test("older native catalogs without the new fields parse as supported", () => {
  const { supported, stage, ...legacy } = entry("legacy");
  void supported;
  void stage;
  const parsed = modelEntrySchema.parse(legacy);
  assert.equal(parsed.supported, true);
  assert.equal(parsed.stage, null);
});

test("running, checking and installing models explain the wait", () => {
  const context = {
    selection: selection(),
    models,
    remoteNotes: false,
    nameOf,
  };
  assert.equal(
    removalPlan({ ...lfm, state: "busy" }, context).blocked,
    "Running now. Remove it after dictation or notes finish.",
  );
  assert.equal(
    removalPlan({ ...lfm, state: "checking" }, context).blocked,
    "Checking…",
  );
  assert.equal(
    removalPlan({ ...lfm, state: "downloading" }, context).blocked,
    "Installation in progress.",
  );
});

const catalogModel = (
  id: string,
  inputModalities: string[],
  reasoningEffort?: string | null,
): AiModel => ({
  id,
  name: id,
  contextLength: 1,
  promptPrice: "0.000001",
  completionPrice: "0.000002",
  description: null,
  inputModalities,
  reasoningEffort,
});

test("catalog rows list each model once with all of its roles", () => {
  const rows = catalogRows(
    [
      catalogModel("google/gemini-3.8-flash", ["text", "audio"], "high"),
      catalogModel("z-ai/glm-5.3-flash", ["text"], "high"),
      catalogModel("audio/only", ["audio"]),
      catalogModel("image/only", ["image"]),
    ],
    "",
  );
  assert.deepEqual(
    rows.map((row) => [row.item.id, row.roles]),
    [
      ["google/gemini-3.8-flash", ["text", "speech"]],
      ["z-ai/glm-5.3-flash", ["text"]],
      ["audio/only", ["speech"]],
    ],
  );
  assert.equal(
    rolesLabel(["text", "speech"]),
    "Text and notes, meeting transcription",
  );
  assert.deepEqual(
    catalogRows(
      [catalogModel("google/gemini-3.8-flash", ["text", "audio"])],
      "meeting",
    ).map((row) => row.item.id),
    ["google/gemini-3.8-flash"],
  );
});

test("saved per-role entries group into one row and remove together", () => {
  const settings = aiSettingsSchema.parse({
    backend: "openrouter",
    model: "google/gemini-3.8-flash",
    speechModel: "google/gemini-3.8-flash",
    quickModels: [
      { id: "google/gemini-3.8-flash", role: "text" },
      { id: "z-ai/glm-5.3-flash", role: "text" },
      { id: "google/gemini-3.8-flash", role: "speech" },
    ],
    connected: true,
    supported: true,
    help: "",
  });
  assert.deepEqual(groupQuickModels(settings.quickModels), [
    { id: "google/gemini-3.8-flash", roles: ["text", "speech"] },
    { id: "z-ai/glm-5.3-flash", roles: ["text"] },
  ]);
  assert.deepEqual(
    quickModelGroupRemoval(settings, "google/gemini-3.8-flash"),
    {
      quickModels: [{ id: "z-ai/glm-5.3-flash", role: "text" }],
      backend: "local",
      speechModel: "",
    },
  );
  assert.deepEqual(quickModelGroupRemoval(settings, "z-ai/glm-5.3-flash"), {
    quickModels: [
      { id: "google/gemini-3.8-flash", role: "text" },
      { id: "google/gemini-3.8-flash", role: "speech" },
    ],
  });
});

test("recommended lists parse with and without a curated effort", () => {
  const old = parseResponse("list_ai_models", [
    {
      id: "z-ai/glm-5.3-flash",
      name: "Z.ai: GLM 5.3 Flash",
      contextLength: 1,
      promptPrice: null,
      completionPrice: null,
    },
  ]);
  assert.equal(old[0].reasoningEffort, undefined);
  assert.equal(effortLabel(old[0].reasoningEffort), "");
  const current = parseResponse("list_ai_models", [
    {
      id: "openai/gpt-6-luna",
      name: "OpenAI: GPT-6 Luna",
      contextLength: 1,
      promptPrice: null,
      completionPrice: null,
      reasoningEffort: "xhigh",
    },
    {
      id: "a/b",
      name: "A",
      contextLength: 1,
      promptPrice: null,
      completionPrice: null,
      reasoningEffort: null,
    },
  ]);
  assert.equal(current[0].reasoningEffort, "xhigh");
  assert.equal(effortLabel(current[0].reasoningEffort), "Extra-high reasoning");
  assert.equal(effortLabel("high"), "High reasoning");
  assert.equal(effortLabel("medium"), "Medium reasoning");
  assert.equal(effortLabel("none"), "");
  assert.equal(effortLabel(current[1].reasoningEffort), "");
});
