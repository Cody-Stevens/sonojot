import { strict as assert } from "node:assert";
import { test } from "node:test";
import type { ModelEntry } from "./model-library";
import { previewSnapshot, type Settings } from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";
import {
  qualifiesForSmartLayout,
  smartLayoutUnavailableReason,
} from "./smart-layout";
import { punctuationVersion } from "./punctuation";

const settings: Settings = {
  ...previewSnapshot.settings,
  cleanupModel: "lfm2.5-1.2b",
  notesModel: "qwen3-4b-instruct-2507",
  localProofreader: false,
  proofreader: false,
};
function model(id: string, patch: Partial<ModelEntry> = {}): ModelEntry {
  return {
    id,
    name: id,
    kind: "cleanup",
    roles: ["cleanup", "notes"],
    family: "Fixture",
    languages: [],
    older: false,
    deprecated: false,
    diskBytes: 1_000,
    ramMinBytes: null,
    ramMaxBytes: null,
    vramMinBytes: null,
    vramMaxBytes: null,
    memoryBasis: "Fixture",
    guidance: "Fixture",
    evidence: "Fixture",
    backend: "cpu",
    compatibility: "Fixture",
    state: "installed",
    downloadedBytes: 1_000,
    totalBytes: 1_000,
    error: null,
    installable: true,
    removable: true,
    usedBy: [],
    revision: "fixture",
    quantization: "fixture",
    contextTokens: 4096,
    supported: true,
    stage: null,
    ...patch,
  };
}
const cleanup = model(settings.cleanupModel);
const notes = model(settings.notesModel);
const reason = (patch: Partial<Settings>, models: ModelEntry[], sonojot = false) =>
  smartLayoutUnavailableReason({ ...settings, ...patch }, models, sonojot);

test("layout runs on the cleanup model, never on Notes", () => {
  assert.equal(reason({}, [cleanup, notes]), null);
  // A ready Notes model no longer makes layout available.
  assert.match(
    reason({ cleanupModel: "local-rules" }, [notes]) ?? "",
    /Needs a cleanup model on this computer/,
  );
  assert.match(reason({}, [notes]) ?? "", /Install the cleanup model/);
});

test("the SonoJot model shapes text; online models say why they cannot", () => {
  assert.equal(
    reason({ localProofreader: true, cleanupModel: "local-rules" }, [], true),
    null,
  );
  assert.match(
    reason({ proofreader: true, cleanupModel: "local-rules" }, [notes]) ?? "",
    /Online cleanup models can't shape text yet/,
  );
  // Without the installed model, the SonoJot preference falls back to Off.
  assert.match(
    reason({ localProofreader: true, cleanupModel: "local-rules" }, []) ?? "",
    /Needs a cleanup model/,
  );
});

test("layout requires an installed cleanup model", () => {
  for (const state of ["checking", "queued", "downloading", "failed", "future-state"])
    assert.match(
      reason({}, [model(cleanup.id, { state }), notes]) ?? "",
      /Install the cleanup model/,
      state,
    );
  for (const state of ["installed", "unloaded", "loading", "ready", "busy"])
    assert.equal(reason({}, [model(cleanup.id, { state })]), null, state);
});

test("layout never uses SpeakoFlow or a non-generation model", () => {
  const smart = model(SMART_CLEANUP_MODEL);
  assert.match(
    reason({ cleanupModel: smart.id }, [smart, notes]) ?? "",
    /SpeakoFlow Mini can't shape text/,
  );
  assert.match(
    reason({}, [model(cleanup.id, { roles: ["cleanup"] })]) ?? "",
    /can't shape text/,
  );
});

test("layout is unavailable in As heard", () => {
  assert.match(
    reason({ cleanupMode: "original" }, [cleanup]) ?? "",
    /Choose Format only or Clean/,
  );
  for (const cleanupMode of ["format", "clean"] as const)
    assert.equal(reason({ cleanupMode }, [cleanup]), null);
});

test("local general text models qualify, installed or not", () => {
  assert.equal(qualifiesForSmartLayout(notes), true);
  assert.equal(
    qualifiesForSmartLayout(model("lfm", { state: "not_installed" })),
    true,
  );
  assert.equal(
    qualifiesForSmartLayout(model(SMART_CLEANUP_MODEL, { roles: ["cleanup"] })),
    false,
  );
  assert.equal(
    qualifiesForSmartLayout(model("moonshine", { kind: "asr", roles: ["asr"] })),
    false,
  );
});

test("cleanup package versions read cleanly for every package id", () => {
  assert.equal(punctuationVersion("sonojot-cleanup-v4"), "v4");
  assert.equal(punctuationVersion("sonojot-punct-v3"), "v3");
  assert.equal(punctuationVersion("sonojot-punct-v1"), "v1");
});
