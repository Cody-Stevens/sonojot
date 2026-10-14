import assert from "node:assert/strict";
import test from "node:test";
import { modelBudget } from "./model-budget";
import {
  modelLibrarySchema,
  modelRoles,
  planRange,
  removalBlock,
  selectable,
  type ModelEntry,
} from "./model-library";
import { parseResponse } from "./protocol";

const GB = 1e9;
function model(
  id: string,
  kind: ModelEntry["kind"],
  min: number,
  max: number,
  patch: Partial<ModelEntry> = {},
): ModelEntry {
  return {
    id,
    name: id,
    kind,
    roles:
      kind === "asr"
        ? ["asr"]
        : kind === "diarization"
          ? ["speakers"]
          : ["cleanup", "notes"],
    family: "Fixture",
    languages: ["en"],
    older: false,
    deprecated: false,
    diskBytes: GB,
    ramMinBytes: min * GB,
    ramMaxBytes: max * GB,
    vramMinBytes: min * GB,
    vramMaxBytes: max * GB,
    memoryBasis: "Fictional estimate, not measured.",
    guidance: "",
    evidence: "Unrated",
    backend: "cuda",
    compatibility: "Fixture",
    state: "installed",
    downloadedBytes: GB,
    totalBytes: GB,
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
const speech = model("speech", "asr", 6, 10);
const cleanup = model("cleanup", "cleanup", 2, 4);
const notes = model("notes", "cleanup", 3, 5);
const diarization = model("nemotron-3-diarization", "diarization", 1, 2, {
  backend: "CPU · ONNX Runtime",
  ramMinBytes: null,
  ramMaxBytes: null,
  vramMinBytes: null,
  vramMaxBytes: null,
});
const punctuation = model("sonojot-punct-v1", "punctuation", 0, 0, {
  roles: ["punctuation"],
  backend: "onnxruntime-cpu",
  ramMinBytes: null,
  ramMaxBytes: null,
  installable: false,
  removable: false,
});
const hardware = {
  totalRamBytes: 32 * GB,
  availableRamBytes: 2 * GB,
  dedicatedVramBytes: 12 * GB,
  availableVramBytes: GB,
};
const budget = (patch: Partial<Parameters<typeof modelBudget>[0]> = {}) =>
  modelBudget({
    selected: [speech, cleanup, notes],
    hardware,
    sequential: false,
    cpu: false,
    resource: "ram",
    ...patch,
  });

test("overlap includes speech and the largest mutually exclusive text model", () => {
  const result = budget();
  assert.deepEqual(result.demand, {
    min: 9 * GB,
    max: 15 * GB,
    unknown: false,
  });
  assert.equal(result.allowance, 4.8 * GB);
  assert.equal(result.headroom, 12.2 * GB);
});

test("shared cleanup and notes weights count once", () => {
  assert.equal(
    budget({ selected: [speech, cleanup, cleanup] }).demand.max,
    14 * GB,
  );
});
test("library commands accept diarization with the speakers role outside all model pickers", () => {
  for (const command of [
    "get_model_library",
    "start_model_install",
    "cancel_model_install",
    "remove_model",
  ] as const) {
    const parsed = parseResponse(command, {
      hardware,
      models: [speech, cleanup, diarization],
      guidance: "Fixture",
    });
    assert.deepEqual(parsed.models[2], diarization);
    for (const role of modelRoles)
      assert.equal(selectable(parsed.models[2], role.id), false);
  }
});
test("auxiliary diarization does not change known or unknown RAM/VRAM plans", () => {
  for (const sequential of [false, true]) {
    for (const resource of ["ram", "vram"] as const) {
      for (const cpu of [false, true]) {
        const selected = [speech, cleanup, notes];
        const baseline = budget({ selected, sequential, resource, cpu });
        for (const auxiliary of [
          diarization,
          punctuation,
          {
            ...diarization,
            state: "busy",
            ramMinBytes: 90 * GB,
            ramMaxBytes: 100 * GB,
            vramMinBytes: 90 * GB,
            vramMaxBytes: 100 * GB,
          },
        ]) {
          assert.deepEqual(
            budget({
              selected: [...selected, auxiliary],
              sequential,
              resource,
              cpu,
            }),
            baseline,
          );
          assert.equal(
            planRange([...selected, auxiliary], sequential),
            planRange(selected, sequential),
          );
        }
        assert.equal(
          budget({ selected: [diarization, undefined], resource }).unknown,
          true,
        );
      }
    }
    assert.equal(planRange([diarization], sequential), "0 B");
    assert.equal(planRange([diarization, undefined], sequential), "Unrated");
  }
});

test("library accepts unreleased punctuation without adding a selectable model", () => {
  const parsed = parseResponse("get_model_library", {
    hardware,
    models: [speech, cleanup, punctuation],
    guidance: "Fixture",
  });
  assert.deepEqual(parsed.models[2], punctuation);
  assert.equal(parsed.models[2].installable, false);
  for (const role of modelRoles)
    assert.equal(selectable(parsed.models[2], role.id), false);
});

test("sequential mode budgets the largest stage in RAM and VRAM", () => {
  for (const resource of ["ram", "vram"] as const) {
    const result = budget({ sequential: true, resource });
    assert.deepEqual(result.demand, {
      min: 6 * GB,
      max: 10 * GB,
      unknown: false,
    });
    assert.equal(result.over, false);
  }
});

test("free memory is a separate reading and never reduces or doubles the plan", () => {
  const low = budget();
  const high = budget({
    hardware: { ...hardware, availableRamBytes: 31 * GB },
    selected: [{ ...speech, state: "ready" }, cleanup, notes],
  });
  assert.equal(low.free, 2 * GB);
  assert.equal(high.free, 31 * GB);
  assert.deepEqual(low.demand, high.demand);
  assert.equal(low.headroom, high.headroom);
  assert.equal(low.over, false);
});

test("Auto keeps an oversized proposed GPU budget even though backend may choose CPU", () => {
  const result = budget({ resource: "vram" });
  assert.equal(result.demand.max, 15 * GB);
  assert.equal(result.over, true);
  assert.equal(result.headroom, 0);
  assert.equal(result.free, GB);
  assert.match(result.detail, /fall back to CPU before allocation/);
  const cpu = budget({ resource: "vram", cpu: true });
  assert.equal(cpu.demand.max, 10 * GB); // CUDA speech still needs its own VRAM.
  assert.equal(cpu.over, false);
});

test("CPU speech, CPU text and local rules have known zero VRAM demand", () => {
  const result = budget({
    resource: "vram",
    cpu: true,
    selected: [
      {
        ...speech,
        backend: "sherpa-cpu",
        vramMinBytes: null,
        vramMaxBytes: null,
      },
      cleanup,
      model("local-rules", "cleanup", 0, 0),
    ],
  });
  assert.deepEqual(result.demand, { min: 0, max: 0, unknown: false });
});

test("missing model or interval always suppresses headroom", () => {
  for (const selected of [
    [undefined, cleanup],
    [speech, { ...cleanup, ramMinBytes: null }],
    [speech, { ...cleanup, ramMaxBytes: null }],
    [speech, { ...cleanup, ramMinBytes: 5 * GB, ramMaxBytes: GB }],
  ]) {
    const result = budget({ selected });
    assert.equal(result.unknown, true);
    assert.equal(result.headroom, null);
    assert.equal(
      result.segments.some((segment) => segment.label === "Headroom"),
      false,
    );
    assert.match(result.estimate, /unknown/i);
  }
});

test("legacy zero or unknown text VRAM cannot imply a fit under Auto", () => {
  for (const value of [null, 0]) {
    assert.equal(
      budget({
        resource: "vram",
        selected: [{ ...cleanup, vramMinBytes: value, vramMaxBytes: value }],
      }).unknown,
      true,
    );
  }
  const missingCapacity = budget({ hardware: undefined });
  assert.equal(missingCapacity.unknown, true);
  assert.equal(missingCapacity.headroom, null);
});

test("free VRAM is optional on older hosts and does not change planned capacity", () => {
  const parsed = modelLibrarySchema.parse({
    hardware: {
      totalRamBytes: null,
      availableRamBytes: null,
      dedicatedVramBytes: 12 * GB,
    },
    models: [speech],
    guidance: "Fixture",
  });
  const result = budget({
    resource: "vram",
    selected: [speech],
    hardware: parsed.hardware,
  });
  assert.equal(result.free, null);
  assert.equal(result.unknown, false);
  assert.equal(result.headroom, 0.8 * GB);
});

test("selection and removal reject missing, unsupported, selected and active models", () => {
  for (const state of [
    "available",
    "unsupported",
    "failed",
    "downloading",
    "verifying",
  ])
    assert.equal(selectable({ ...cleanup, state }, "cleanup"), false);
  assert.equal(selectable(speech, "notes"), false);
  assert.equal(selectable(cleanup, "notes"), true);
  assert.equal(removalBlock(cleanup, [cleanup.id]), "Selected model");
  assert.equal(
    removalBlock({ ...cleanup, usedBy: ["notes"] }, []),
    "Selected model",
  );
  for (const state of ["busy", "loading", "unloading"])
    assert.equal(removalBlock({ ...cleanup, state }, []), "Model in use");
  assert.equal(removalBlock(cleanup, []), null);
});
