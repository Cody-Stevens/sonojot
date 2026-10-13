import assert from "node:assert/strict";
import test from "node:test";
import {
  defaultMeetingModel,
  meetingModelSettings,
  meetingSpeechModels,
  saveMeetingModelChoice,
  showMeetingModelPicker,
} from "./meeting-models";
import {
  GRANITE_SPEECH_MODEL,
  modelEntrySchema,
  modelRoles,
  modelUnavailableReason,
  removalBlock,
  selectable,
  shortDeviceLabel,
  type ModelEntry,
} from "./model-library";
import { parseResponse, previewSnapshot } from "./protocol";
import { selectionModels } from "./setup-profiles";

function granite(patch: Partial<ModelEntry> = {}): ModelEntry {
  return modelEntrySchema.parse({
    id: GRANITE_SPEECH_MODEL,
    name: "Granite Speech 4.1 NAR",
    kind: "asr",
    roles: ["meetingSpeech"],
    family: "Granite Speech",
    languages: ["English"],
    older: false,
    diskBytes: 2.5e9,
    ramMinBytes: null,
    ramMaxBytes: null,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "Fixture estimate unavailable",
    guidance: "More accurate on room microphones and calls; meetings only.",
    evidence: "Fixture",
    backend: "transcribe.cpp CUDA",
    compatibility: "NVIDIA CUDA / Apple Metal",
    state: "installed",
    downloadedBytes: 2.5e9,
    totalBytes: 2.5e9,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "fixture",
    quantization: "Q8_0",
    contextTokens: null,
    ...patch,
  });
}

const speech = granite({
  id: "dictation-speech",
  name: "Dictation speech",
  roles: ["asr"],
});
const settings = { ...previewSnapshot.settings, asrModel: speech.id };

test("legacy settings keep Same as Speech and the wire contract retains Granite's meeting role", () => {
  const { meetingAsrModel: _meeting, ...legacy } = settings;
  const snapshot = parseResponse("get_snapshot", {
    ...previewSnapshot,
    settings: legacy,
  });
  assert.equal(snapshot.settings.meetingAsrModel, "");
  assert.equal(defaultMeetingModel(snapshot.settings, null), speech.id);
  const saved = parseResponse("save_settings", {
    ...snapshot,
    settings: { ...settings, meetingAsrModel: GRANITE_SPEECH_MODEL },
  });
  assert.equal(saved.settings.meetingAsrModel, GRANITE_SPEECH_MODEL);
  assert.equal(saved.settings.asrModel, speech.id);
  assert.deepEqual(modelEntrySchema.parse(granite()).roles, ["meetingSpeech"]);
});

test("installed Granite is available for meeting retries and excluded from dictation/profile roles", () => {
  const models = [
    speech,
    granite(),
    granite({ id: "text", kind: "cleanup", roles: ["cleanup", "notes"] }),
  ];
  assert.deepEqual(
    meetingSpeechModels(models).map((model) => model.id),
    [speech.id, GRANITE_SPEECH_MODEL],
  );
  assert.deepEqual(
    models.filter((model) => selectable(model, "asr")).map((model) => model.id),
    [speech.id],
  );
  for (const role of modelRoles)
    assert.equal(selectable(granite(), role.id), false);
  const profile = {
    asrModel: speech.id,
    cleanupModel: "text",
    notesModel: "text",
    reasons: [],
  };
  assert.deepEqual(
    selectionModels(profile, models).map((model) => model?.id),
    [speech.id, "text", "text"],
  );
  assert.deepEqual(
    modelRoles.map((role) => role.setting),
    ["asrModel", "cleanupModel", "notesModel"],
  );
});

test("meeting picker works offline with Granite and with connected OpenRouter alternatives", () => {
  assert.equal(showMeetingModelPicker([speech], false, 0), false);
  assert.equal(showMeetingModelPicker([speech, granite()], false, 0), true);
  assert.equal(
    showMeetingModelPicker([granite({ state: "not-installed" })], false, 0),
    false,
  );
  assert.equal(showMeetingModelPicker([speech], true, 1), true);
  assert.equal(showMeetingModelPicker([speech], false, 1), false);
  assert.equal(showMeetingModelPicker([speech], true, 0), false);
});

test("catalog availability controls selection and preserves a selected unavailable model's reason", () => {
  for (const state of ["installed", "unloaded", "ready"]) {
    assert.equal(selectable(granite({ state }), "meetingSpeech"), true);
  }
  const reason = "NVIDIA CUDA is unavailable on this computer.";
  for (const state of [
    "not-installed",
    "checking",
    "downloading",
    "failed",
    "unsupported",
    "unavailable",
  ]) {
    assert.deepEqual(meetingSpeechModels([granite({ state })]), []);
  }
  const unavailable = granite({
    state: "unavailable",
    installable: false,
    error: reason,
  });
  assert.equal(showMeetingModelPicker([unavailable], false, 0), false);
  assert.equal(
    showMeetingModelPicker([unavailable], false, 0, unavailable.id),
    true,
  );
  assert.equal(modelUnavailableReason(unavailable), reason);
  assert.equal(
    modelUnavailableReason({
      ...unavailable,
      error: null,
      compatibility: reason,
    }),
    reason,
  );
  // Backend labels and unknown memory are not a hardware capability probe.
  assert.equal(
    modelUnavailableReason(
      granite({ backend: "unverified", vramMaxBytes: null }),
    ),
    null,
  );
  assert.equal(shortDeviceLabel(granite()), "NVIDIA CUDA / Apple Metal");
  assert.equal(
    shortDeviceLabel(granite({ backend: "transcribe.cpp Metal" })),
    "NVIDIA CUDA / Apple Metal",
  );
  assert.equal(
    removalBlock(granite(), [GRANITE_SPEECH_MODEL]),
    "Selected model",
  );
});

test("meeting defaults preserve an explicit Granite choice and otherwise follow Speech or OpenRouter", () => {
  const online = { connected: true, speechModel: "provider/audio" };
  assert.equal(defaultMeetingModel(settings, null), speech.id);
  assert.equal(defaultMeetingModel(settings, online), "openrouter");
  assert.equal(
    defaultMeetingModel(settings, { ...online, connected: false }),
    speech.id,
  );
  assert.equal(
    defaultMeetingModel(
      { ...settings, meetingAsrModel: GRANITE_SPEECH_MODEL },
      online,
    ),
    GRANITE_SPEECH_MODEL,
  );
});

test("meeting choices update only the two meeting routes, leaving dictation and profiles intact", async () => {
  let local = { ...settings, setupProfile: "balanced" as const };
  let online = "provider/audio";
  const operations = {
    saveOnline: async (speechModel: string) => {
      online = speechModel;
    },
    saveLocal: async (meetingAsrModel: string) => {
      local = { ...local, meetingAsrModel };
    },
  };
  await saveMeetingModelChoice(
    { kind: "local", id: GRANITE_SPEECH_MODEL },
    online,
    operations,
  );
  assert.equal(local.meetingAsrModel, GRANITE_SPEECH_MODEL);
  assert.equal(online, "");
  assert.equal(local.asrModel, speech.id);
  assert.equal(local.setupProfile, "balanced");
  await saveMeetingModelChoice(
    { kind: "online", id: "provider/other-audio" },
    online,
    operations,
  );
  assert.equal(local.meetingAsrModel, "");
  assert.equal(online, "provider/other-audio");
  await saveMeetingModelChoice({ kind: "local", id: "" }, online, operations);
  assert.equal(local.meetingAsrModel, "");
  assert.equal(online, "");
  assert.deepEqual(meetingModelSettings({ kind: "local", id: "" }), {
    meetingAsrModel: "",
    speechModel: "",
  });
  assert.deepEqual(local, { ...settings, setupProfile: "balanced" });
});

test("a rejected online save cannot change the saved local meeting route", async () => {
  await assert.rejects(
    saveMeetingModelChoice(
      { kind: "local", id: GRANITE_SPEECH_MODEL },
      "provider/audio",
      {
        saveOnline: async () => {
          throw new Error("Online save failed");
        },
        saveLocal: async () => {
          assert.fail("Must not change local route after a failed online save");
        },
      },
    ),
    /Online save failed/,
  );
});

test("a failed local save restores the previous online route in either direction", async () => {
  for (const choice of [
    { kind: "local", id: GRANITE_SPEECH_MODEL },
    { kind: "online", id: "provider/audio" },
  ] as const) {
    const previous = choice.kind === "local" ? "provider/audio" : "";
    const saved: string[] = [];
    await assert.rejects(
      saveMeetingModelChoice(choice, previous, {
        saveOnline: async (id) => {
          saved.push(id);
        },
        saveLocal: async () => {
          throw new Error("Local save failed");
        },
      }),
      /Local save failed/,
    );
    assert.deepEqual(saved, [
      meetingModelSettings(choice).speechModel,
      previous,
    ]);
  }
});

test("rollback failure tells the user to inspect the meeting choice", async () => {
  let saves = 0;
  await assert.rejects(
    saveMeetingModelChoice(
      { kind: "local", id: GRANITE_SPEECH_MODEL },
      "provider/audio",
      {
        saveOnline: async () => {
          if (++saves > 1) throw new Error("Rollback failed");
        },
        saveLocal: async () => {
          throw new Error("Local save failed");
        },
      },
    ),
    /previous online route restored/,
  );
});
