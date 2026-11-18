import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { join } from "node:path";
import { Tokenizer } from "@huggingface/tokenizers";
import { hasModel, modelDirectory, openPunctuation } from "../scripts/text-runtime.ts";
import { postprocess } from "../src/text/index.ts";
import { localRules } from "../src/text/rules.ts";
import type { CleanupMode } from "../src/text/contract.ts";

interface Fixture {
  rows: { id: string; input: string; mode: CleanupMode; expected: string; rulesExpected: string }[];
  tokenizerCases: { text: string; ids: number[] }[];
}
const fixture: Fixture = JSON.parse(readFileSync(new URL("./fixtures/text-parity.json", import.meta.url), "utf8"));
const missing = hasModel
  ? false
  : "SonoJot Punctuation v4 package missing; set SONOJOT_PUNCT_MODEL_DIR to run real-model parity";

test("conservative local rules exactly match every Rust golden", () => {
  const failures = fixture.rows.flatMap((row) => {
    const actual = localRules(row.input, row.mode);
    return actual === row.rulesExpected ? [] : [{ id: row.id, expected: row.rulesExpected, actual }];
  });
  assert.deepEqual(failures, []);
});

test("per-word tokenizer IDs match Rust tokenizers 0.22.2", { skip: missing }, () => {
  const data: object = JSON.parse(readFileSync(join(modelDirectory, "tokenizer.json"), "utf8"));
  const tokenizer = new Tokenizer({ ...data, padding: null, truncation: null }, {});
  const failures = fixture.tokenizerCases.flatMap((row) => {
    const actual = tokenizer.encode(row.text, { add_special_tokens: false }).ids;
    return JSON.stringify(actual) === JSON.stringify(row.ids) ? [] : [{ text: row.text, expected: row.ids, actual }];
  });
  assert.deepEqual(failures, []);
});

interface NearTie {
  id: string;
  actual: string;
  reason: string;
}
const nearTies: NearTie[] = JSON.parse(
  readFileSync(new URL("./fixtures/text-parity-near-ties.json", import.meta.url), "utf8"),
).rows;

test("Clean and Format match the real Rust v4 pipeline, apart from the recorded runtime near ties", { skip: missing }, async (t) => {
  const runtime = await openPunctuation();
  const failures: { id: string; expected: string; actual: string; stages: string[] }[] = [];
  const stages = new Map<string, string[]>();
  try {
    for (const row of fixture.rows) {
      const result = await postprocess(row.input, { mode: row.mode, punctuation: runtime.package });
      stages.set(row.id, result.stages);
      if (result.text !== row.expected)
        failures.push({ id: row.id, expected: row.expected, actual: result.text, stages: result.stages });
      const original = await postprocess(row.input, { mode: "original", punctuation: runtime.package });
      assert.equal(original.text, row.input, `${row.id}: Original returns the input`);
    }
  } finally {
    await runtime.close();
  }
  t.diagnostic(`${fixture.rows.length - failures.length}/${fixture.rows.length} exact matches; recorded near ties: ${nearTies.map((n) => n.id).join(", ")}`);
  // Every other row must match exactly; a near tie must differ in exactly the recorded way.
  assert.deepEqual(
    failures.map(({ id, actual }) => ({ id, actual })),
    nearTies.map(({ id, actual }) => ({ id, actual })),
  );
  // The delete head runs in Clean only (whenever the model ran); Format keeps every word.
  for (const row of fixture.rows) {
    const ran = stages.get(row.id)!;
    if (row.mode !== "clean") assert.ok(!ran.includes("delete"), `${row.id}: no delete stage in ${row.mode}`);
    else if (ran.includes("punctuation")) assert.ok(ran.includes("delete"), `${row.id}: delete stage in Clean`);
  }
});
