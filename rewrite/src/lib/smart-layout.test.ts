import { strict as assert } from "node:assert";
import { test } from "node:test";
import type { ModelEntry } from "./model-library";
import { previewSnapshot, type Settings } from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";
import { smartLayoutModel, smartLayoutUnavailableReason } from "./smart-layout";

const settings: Settings = {
  ...previewSnapshot.settings,
  cleanupModel: "lfm2.5-1.2b",
  notesModel: "qwen3-4b-instruct-2507",
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
    ...patch,
  };
}
const cleanup = model(settings.cleanupModel);
const notes = model(settings.notesModel);

test("layout prefers selected Cleanup over selected Notes regardless of catalog order", () => {
  assert.equal(smartLayoutModel(settings, [notes, cleanup]), cleanup);
  assert.equal(smartLayoutModel(settings, [notes]), notes);
  assert.equal(
    smartLayoutModel({ ...settings, notesModel: cleanup.id }, [cleanup]),
    cleanup,
  );
});

test("layout requires verification and falls back to the selected ready Notes model", () => {
  for (const state of [
    "checking",
    "queued",
    "downloading",
    "verifying",
    "installing",
    "not-installed",
    "unavailable",
    "failed",
    "cancelled",
    "future-state",
  ]) {
    const unavailable = model(cleanup.id, { state });
    assert.equal(
      smartLayoutModel(settings, [unavailable, notes]),
      notes,
      state,
    );
    assert.equal(smartLayoutModel(settings, [unavailable]), undefined, state);
  }
  // Runtime occupancy affects each job, not the user's persisted preference.
  for (const state of [
    "installed",
    "unloaded",
    "loading",
    "ready",
    "busy",
    "unloading",
  ]) {
    const installed = model(cleanup.id, { state });
    assert.equal(
      smartLayoutModel(settings, [installed, notes]),
      installed,
      state,
    );
  }
});

test("layout never uses SpeakoFlow, even with an incorrect generation role", () => {
  const smart = model(SMART_CLEANUP_MODEL);
  assert.equal(
    smartLayoutModel({ ...settings, cleanupModel: smart.id }, [smart, notes]),
    notes,
  );
  assert.equal(
    smartLayoutModel(
      { ...settings, cleanupModel: smart.id, notesModel: smart.id },
      [smart],
    ),
    undefined,
  );
});

test("layout rejects non-generation models and never picks an unselected installed model", () => {
  for (const candidate of [
    model(cleanup.id, { roles: ["cleanup"] }),
    model(cleanup.id, { kind: "asr" }),
    model(cleanup.id, { kind: "diarization" }),
  ]) {
    assert.equal(smartLayoutModel(settings, [candidate, notes]), notes);
    assert.equal(smartLayoutModel(settings, [candidate]), undefined);
  }
  const rules = model("local-rules", { roles: ["cleanup"] });
  for (const cleanupModel of ["local-rules", "off", "openrouter", "missing"]) {
    assert.equal(
      smartLayoutModel({ ...settings, cleanupModel }, [rules, notes]),
      notes,
    );
    assert.equal(
      smartLayoutModel(
        { ...settings, cleanupModel, notesModel: "openrouter" },
        [rules, cleanup, notes],
      ),
      undefined,
    );
  }
});

test("layout is unavailable in As heard and without a verified selected text model", () => {
  assert.match(
    smartLayoutUnavailableReason({ ...settings, cleanupMode: "original" }, [
      cleanup,
    ]) ?? "",
    /Choose Format only or Clean/,
  );
  for (const cleanupMode of ["format", "clean"] as const) {
    const selection = { ...settings, cleanupMode };
    assert.equal(smartLayoutUnavailableReason(selection, [cleanup]), null);
    assert.match(
      smartLayoutUnavailableReason(selection, []) ?? "",
      /Select an installed local text model for Cleanup or Notes in Models\./,
    );
  }
});
