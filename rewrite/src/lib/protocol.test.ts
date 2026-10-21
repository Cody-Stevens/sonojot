import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  duration,
  parseResponse,
  previewSnapshot,
  safeFilename,
} from "./protocol";

test("snapshot rejects malformed data at the native boundary", () => {
  assert.deepEqual(
    parseResponse("get_snapshot", previewSnapshot),
    previewSnapshot,
  );
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      audioLevel: Number.NaN,
    }),
  );
  assert.throws(() =>
    parseResponse("get_snapshot", { ...previewSnapshot, state: "complete" }),
  );
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { theme: "fog" },
    }),
  );
});
test("smart layout defaults off for legacy settings and preserves saved choices", () => {
  const { smartLayout, ...legacySettings } = previewSnapshot.settings;
  assert.equal(smartLayout, false);
  assert.deepEqual(
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: legacySettings,
    }).settings,
    { ...legacySettings, smartLayout: false },
  );
  for (const enabled of [false, true]) {
    for (const cleanupMode of ["original", "format", "clean"] as const) {
      const settings = { ...legacySettings, cleanupMode, smartLayout: enabled };
      assert.deepEqual(
        parseResponse("save_settings", { ...previewSnapshot, settings }).settings,
        settings,
      );
    }
  }
  for (const invalid of ["true", 1, null]) {
    assert.throws(() =>
      parseResponse("get_snapshot", {
        ...previewSnapshot,
        settings: { ...legacySettings, smartLayout: invalid },
      }),
    );
  }
});
test("app hints default off and preserve choices while shape by type is inactive", () => {
  const { smartLayoutAppHint, ...legacySettings } = previewSnapshot.settings;
  assert.equal(smartLayoutAppHint, false);
  assert.equal(
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: legacySettings,
    }).settings.smartLayoutAppHint,
    false,
  );
  for (const enabled of [false, true]) {
    for (const smartLayout of [false, true]) {
      for (const cleanupMode of ["original", "format", "clean"] as const) {
        const settings = {
          ...legacySettings,
          smartLayout,
          cleanupMode,
          smartLayoutAppHint: enabled,
        };
        assert.deepEqual(
          parseResponse("save_settings", { ...previewSnapshot, settings }).settings,
          settings,
        );
      }
    }
  }
  for (const invalid of ["true", 1, null]) {
    assert.throws(() =>
      parseResponse("get_snapshot", {
        ...previewSnapshot,
        settings: { ...legacySettings, smartLayoutAppHint: invalid },
      }),
    );
  }
});
test("dictation history accepts legacy records and preserves an optional target app", () => {
  const legacy = {
    id: "legacy",
    createdAt: "2026-10-20",
    raw: "hello",
    published: "Hello.",
  };
  assert.equal(parseResponse("list_dictations", [legacy])[0].targetApp, null);
  for (const targetApp of [null, "outlook.exe", "com.apple.mail"]) {
    assert.equal(
      parseResponse("list_dictations", [{ ...legacy, targetApp }])[0].targetApp,
      targetApp,
    );
  }
  assert.throws(() =>
    parseResponse("list_dictations", [{ ...legacy, targetApp: 42 }]),
  );
});
test("written form defaults on and validates saved choices", () => {
  const { writtenForm, ...legacySettings } = previewSnapshot.settings;
  assert.equal(writtenForm, true);
  assert.equal(
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: legacySettings,
    }).settings.writtenForm,
    true,
  );
  for (const enabled of [false, true]) {
    assert.equal(
      parseResponse("save_settings", {
        ...previewSnapshot,
        settings: { ...legacySettings, writtenForm: enabled },
      }).settings.writtenForm,
      enabled,
    );
  }
  for (const invalid of ["true", 1, null]) {
    assert.throws(() =>
      parseResponse("get_snapshot", {
        ...previewSnapshot,
        settings: { ...legacySettings, writtenForm: invalid },
      }),
    );
  }
});
test("responses preserve literal user text and omit unexpected fields", () => {
  const text = '<script>alert("private")</script> & Do not replace 4.6';
  const snapshot = parseResponse("get_snapshot", {
    ...previewSnapshot,
    originalText: text,
    token: "never expose",
  });
  assert.equal(snapshot.originalText, text);
  assert.equal("token" in snapshot, false);
});
test("live meeting source notices default empty and stay separate from fatal errors", () => {
  const { meetingSourceStatus, ...legacy } = previewSnapshot;
  assert.deepEqual(meetingSourceStatus, []);
  assert.deepEqual(parseResponse("get_snapshot", legacy).meetingSourceStatus, []);
  const notices = ["Microphone lost — reconnecting", "System audio reconnected"];
  const next = parseResponse("get_snapshot", { ...legacy, meetingSourceStatus: notices });
  assert.deepEqual(next.meetingSourceStatus, notices);
  assert.equal(next.error, null);
  assert.throws(() => parseResponse("get_snapshot", { ...legacy, meetingSourceStatus: [12] }));
});
test("speaker labels default on for legacy settings and preserve an explicit opt-out", () => {
  const { meetingSpeakerLabels, ...legacySettings } = previewSnapshot.settings;
  assert.equal(meetingSpeakerLabels, true);
  assert.equal(
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: legacySettings,
    }).settings.meetingSpeakerLabels,
    true,
  );
  assert.equal(
    parseResponse("save_settings", {
      ...previewSnapshot,
      settings: { ...legacySettings, meetingSpeakerLabels: false },
    }).settings.meetingSpeakerLabels,
    false,
  );
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...legacySettings, meetingSpeakerLabels: "true" },
    }),
  );
});
test("snapshot accepts auxiliary diarization models without weakening kind validation", () => {
  const model = {
    id: "nemotron-3-diarization",
    name: "Nemotron 3 Diarization",
    kind: "diarization",
    ready: true,
    description: "CPU · ONNX Runtime",
    sizeBytes: 103871648,
  };
  const snapshot = parseResponse("get_snapshot", {
    ...previewSnapshot,
    models: [model],
  });
  assert.equal(snapshot.models[0].kind, "diarization");
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      models: [{ ...model, kind: "speakers" }],
    }),
  );
});
test("recognition completeness survives snapshot and history boundaries", () => {
  const snapshot = parseResponse("get_snapshot", {
    ...previewSnapshot,
    resultSessionId: 41,
    recognitionCompleteness: "incomplete",
    recognitionWarnings: ["segment two remained ambiguous after one retry"],
  });
  assert.equal(snapshot.resultSessionId, 41);
  assert.equal(snapshot.recognitionCompleteness, "incomplete");
  assert.deepEqual(snapshot.recognitionWarnings, [
    "segment two remained ambiguous after one retry",
  ]);
  const [legacy, incomplete] = parseResponse("list_dictations", [
    {
      id: "legacy",
      createdAt: "2026-09-10T20:00:00Z",
      raw: "Legacy raw",
      published: "Legacy text",
    },
    {
      id: "review",
      createdAt: "2026-09-10T20:01:00Z",
      raw: "<thö",
      published: "<thö",
      recognitionCompleteness: "incomplete",
      recognitionWarnings: ["review required"],
    },
  ]);
  assert.equal(legacy.recognitionCompleteness, "complete");
  assert.deepEqual(legacy.recognitionWarnings, []);
  assert.equal(incomplete.recognitionCompleteness, "incomplete");
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      recognitionCompleteness: "unknown",
    }),
  );
});

test("void responses accept Tauri null and undefined, not invented success objects", () => {
  assert.equal(parseResponse("show_page", null), null);
  assert.equal(parseResponse("copy_text", null), null);
  assert.equal(parseResponse("delete_meeting", undefined), undefined);
  assert.throws(() => parseResponse("export_meeting", { success: true }));
});
test("meeting and vocabulary responses validate nested fields", () => {
  assert.throws(() =>
    parseResponse("get_vocabulary", [
      { id: "one", heard: "grock", written: "Grok", enabled: "yes" },
    ]),
  );
  assert.throws(() =>
    parseResponse("list_meetings", [{ id: "one", durationSeconds: -3 }]),
  );
});
test("AI responses default legacy speech fields and validate audio modalities", () => {
  const legacy = {
    backend: "local",
    model: "mock/text",
    connected: true,
    supported: true,
    help: "",
  };
  assert.equal(parseResponse("get_ai_settings", legacy).speechModel, "");
  const saved = parseResponse("save_ai_settings", {
    ...legacy,
    speechModel: "mock/audio",
  });
  assert.equal(saved.model, "mock/text");
  assert.equal(saved.speechModel, "mock/audio");
  const entry = {
    id: "mock/audio",
    name: "Speech",
    contextLength: 32000,
    promptPrice: null,
    completionPrice: null,
  };
  assert.deepEqual(
    parseResponse("list_ai_models", [entry])[0].inputModalities,
    [],
  );
  assert.deepEqual(
    parseResponse("list_ai_catalog", [
      { ...entry, inputModalities: ["text", "audio"] },
    ])[0].inputModalities,
    ["text", "audio"],
  );
  assert.throws(() =>
    parseResponse("list_ai_catalog", [{ ...entry, inputModalities: "audio" }]),
  );
  assert.throws(() =>
    parseResponse("get_ai_settings", { ...legacy, speechModel: 42 }),
  );
});
test("meeting responses retain structured text and remote provenance without changing old records", () => {
  const legacy = {
    id: "meeting",
    title: "Review",
    createdAt: "2026-09-19T20:00:00Z",
    durationSeconds: 60,
    status: "transcribed",
    hasAudio: true,
    hasTranscript: true,
    transcript: "Plain text.",
    notes: "",
    source: "both",
    audioPath: "C:/fixture.wav",
    error: null,
  };
  assert.equal(parseResponse("get_meeting", legacy).transcriptionModel, null);
  assert.deepEqual(parseResponse("get_meeting", legacy).recordingWarnings, []);
  const transcript =
    "[0:12] You: Keep the backup.\n\n[0:31] Other speakers: Agreed.";
  const next = parseResponse("get_meeting", {
    ...legacy,
    transcript,
    transcriptionModel: "openrouter:mock/audio",
    recordingWarnings: ["[0:12] Microphone lost; reconnected at 0:15."],
  });
  assert.equal(next.transcript, transcript);
  assert.equal(next.transcriptionModel, "openrouter:mock/audio");
  assert.deepEqual(next.recordingWarnings, ["[0:12] Microphone lost; reconnected at 0:15."]);
  assert.deepEqual(next.recognitionWarnings, []);
  assert.throws(() => parseResponse("get_meeting", { ...legacy, recordingWarnings: "lost audio" }));
});
test("durations and suggested filenames handle edge cases", () => {
  assert.equal(duration(3661.9), "1:01:01");
  assert.equal(duration(0), "0:00");
  assert.equal(safeFilename("Review: 4/6?"), "Review- 4-6-");
  assert.equal(safeFilename("..."), "Meeting");
});
test("new retention settings default without resetting existing profiles", () => {
  const snapshot = parseResponse("get_snapshot", {
    ...previewSnapshot,
    settings: {
      theme: "fog",
      material: true,
      cleanupMode: "clean",
      launchAtLogin: false,
      hotkey: "Ctrl+Space",
      hotkeyMode: "toggle",
      autoPaste: true,
      audioCues: true,
      autoStopSeconds: 0,
      microphone: null,
      asrModel: "moonshine-medium",
      cleanupModel: "lfm2.5-1.2b",
    },
  });
  assert.equal(snapshot.settings.doNotSaveDictations, false);
  assert.equal(snapshot.settings.learnFromCorrections, false);
  assert.equal(snapshot.settings.memoryMode, "overlap");
  assert.equal(snapshot.settings.modelRetention, "10m");
  assert.equal(snapshot.settings.notesModel, "qwen3-4b-instruct-2507");
  assert.equal(snapshot.settings.textAcceleration, "auto");
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...previewSnapshot.settings, memoryMode: "parallel" },
    }),
  );
  assert.throws(() =>
    parseResponse("get_snapshot", {
      ...previewSnapshot,
      settings: { ...previewSnapshot.settings, modelRetention: "1h" },
    }),
  );
});
test("model library entries keep unknown numbers null, never zero-filled", () => {
  const entry = {
    id: "mock-cpu",
    name: "Mock CPU",
    kind: "cleanup",
    roles: ["cleanup", "notes"],
    family: "Mock",
    languages: ["English"],
    older: false,
    diskBytes: 1000,
    ramMinBytes: null,
    ramMaxBytes: null,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "Unrated",
    guidance: "Mock",
    evidence: "Unrated",
    backend: "cpu",
    compatibility: "Windows CPU",
    state: "available",
    downloadedBytes: 0,
    totalBytes: 1000,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "r1",
    quantization: "q4",
    contextTokens: null,
  };
  const library = parseResponse("get_model_library", {
    hardware: {
      totalRamBytes: null,
      availableRamBytes: null,
      dedicatedVramBytes: null,
    },
    models: [entry],
    guidance: "Mock guidance",
  });
  assert.equal(library.models[0].ramMinBytes, null);
  assert.equal(library.hardware.totalRamBytes, null);
  assert.throws(() =>
    parseResponse("get_model_library", {
      hardware: {
        totalRamBytes: 0,
        availableRamBytes: 0,
        dedicatedVramBytes: 0,
      },
      models: [{ ...entry, kind: "notes" }],
      guidance: "Mock",
    }),
  );
});
test("learning entries validate kind, sources and flags", () => {
  const entries = parseResponse("get_learning", [
    {
      id: "one",
      kind: "correction",
      text: "Grok",
      heard: "grock",
      scope: "AI model discussion",
      sourceIds: ["dictation-one"],
      sourceType: "dictation-correction",
      enabled: true,
      userPinned: false,
    },
  ]);
  assert.equal(entries[0].kind, "correction");
  assert.throws(() =>
    parseResponse("get_learning", [
      {
        id: "bad",
        kind: "guess",
        text: "x",
        heard: null,
        scope: null,
        sourceIds: [],
        sourceType: "dictation-correction",
        enabled: true,
        userPinned: false,
      },
    ]),
  );
  const saved = parseResponse("save_dictation_correction", {
    record: {
      id: "dictation-one",
      createdAt: "2026-09-10T20:00:00Z",
      raw: "first keep grock 4.6",
      published: "First, keep Grok 4.6.",
    },
    learnedCount: 1,
  });
  assert.equal(saved.learnedCount, 1);
  assert.equal(saved.record.recognitionCompleteness, "complete");
  const removed = parseResponse("delete_all_dictations", {
    deletedCount: 3,
    snapshot: previewSnapshot,
  });
  assert.equal(removed.deletedCount, 3);
});
