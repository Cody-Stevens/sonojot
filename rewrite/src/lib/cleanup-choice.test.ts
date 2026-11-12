import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  cleanupChoice,
  cleanupOnlineIds,
  cleanupPatch,
  cleanupState,
  type CleanupChoice,
} from "./cleanup-choice";
import { previewSnapshot } from "./protocol";

const legacy = (
  cleanupModel: string,
  proofreader: boolean,
  localProofreader: boolean,
) => ({
  cleanupModel,
  proofreader,
  localProofreader,
  proofreaderModel: "anthropic/claude-haiku-5.5",
});
const online: CleanupChoice = {
  kind: "online",
  model: "anthropic/claude-haiku-5.5",
};

test("every earlier combination maps to one choice, as in the backend", () => {
  const smart = "speakoflow-mini-q4_k_m";
  const cases: [string, boolean, boolean, boolean, CleanupChoice][] = [
    ["local-rules", false, false, false, { kind: "off" }],
    ["local-rules", false, false, true, { kind: "off" }],
    ["local-rules", false, true, false, { kind: "off" }],
    ["local-rules", false, true, true, { kind: "sonojot" }],
    ["local-rules", true, false, false, online],
    ["local-rules", true, false, true, online],
    ["local-rules", true, true, false, online],
    ["local-rules", true, true, true, { kind: "sonojot" }],
    [smart, false, false, false, { kind: "local", model: smart }],
    [smart, false, true, true, { kind: "sonojot" }],
    [smart, true, false, false, online],
    [
      "qwen3-4b-instruct-2507",
      false,
      false,
      true,
      { kind: "local", model: "qwen3-4b-instruct-2507" },
    ],
    ["qwen3-4b-instruct-2507", true, false, true, online],
    ["qwen3-4b-instruct-2507", false, true, true, { kind: "sonojot" }],
    [
      "lfm2.5-1.2b",
      false,
      true,
      false,
      { kind: "local", model: "lfm2.5-1.2b" },
    ],
    ["off", false, false, false, { kind: "off" }],
  ];
  for (const [cleanup, proofreader, local, available, expected] of cases)
    assert.deepEqual(
      cleanupChoice(legacy(cleanup, proofreader, local), available),
      expected,
      `${cleanup} ${proofreader} ${local} ${available}`,
    );
});

test("fresh settings use SonoJot as soon as it is installed", () => {
  const fresh = previewSnapshot.settings;
  assert.equal(fresh.localProofreader, true);
  assert.equal(fresh.sonojotPunctuation, true);
  assert.deepEqual(cleanupChoice(fresh, false), { kind: "off" });
  assert.deepEqual(cleanupChoice(fresh, true), { kind: "sonojot" });
});

test("each choice saves one route and an explicit Off stays Off", () => {
  const choices: CleanupChoice[] = [
    { kind: "off" },
    { kind: "sonojot" },
    { kind: "local", model: "lfm2.5-1.2b" },
    { kind: "online", model: "anthropic/claude-opus-5.5" },
  ];
  for (const choice of choices) {
    const saved = {
      ...legacy("qwen3-4b-instruct-2507", true, true),
      ...cleanupPatch(choice),
    };
    assert.deepEqual(cleanupChoice(saved, true), choice);
    assert.deepEqual(
      cleanupChoice(saved, false),
      choice.kind === "sonojot" ? { kind: "off" } : choice,
    );
  }
  assert.deepEqual(cleanupPatch({ kind: "off" }), {
    localProofreader: false,
    proofreader: false,
    cleanupModel: "local-rules",
  });
});

test("state words and online options", () => {
  const name = (id: string) => (id === "lfm2.5-1.2b" ? "LFM 2.5" : id);
  assert.equal(cleanupState({ kind: "off" }, name), "Off");
  assert.equal(
    cleanupState({ kind: "sonojot" }, name),
    "SonoJot (experimental)",
  );
  assert.equal(
    cleanupState({ kind: "local", model: "lfm2.5-1.2b" }, name),
    "LFM 2.5",
  );
  assert.equal(
    cleanupState({ kind: "online", model: "x/y" }, name),
    "x/y · OpenRouter",
  );
  assert.deepEqual(cleanupOnlineIds(["x/y", "openai/gpt-6-luna"], "z/w"), [
    "anthropic/claude-opus-5.5",
    "openai/gpt-6-luna",
    "anthropic/claude-sonnet-5.5",
    "x/y",
    "z/w",
  ]);
  assert.deepEqual(cleanupOnlineIds([], ""), [
    "anthropic/claude-opus-5.5",
    "openai/gpt-6-luna",
    "anthropic/claude-sonnet-5.5",
  ]);
});

test("Haiku stays hidden unless saved", () => {
  assert.ok(
    !cleanupOnlineIds(["anthropic/claude-haiku-5.5"], "").includes(
      "anthropic/claude-haiku-5.5",
    ),
  );
  assert.ok(
    cleanupOnlineIds([], "anthropic/claude-haiku-5.5").includes(
      "anthropic/claude-haiku-5.5",
    ),
  );
});
