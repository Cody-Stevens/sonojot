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
  : "SonoJot v3 model package missing; set SONOJOT_PUNCT_MODEL_DIR to run real-model parity";

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

test("Clean and Format exactly match the real Rust v3 pipeline", { skip: missing }, async (t) => {
  const runtime = await openPunctuation();
  const failures: { id: string; expected: string; actual: string; stages: string[] }[] = [];
  try {
    for (const row of fixture.rows) {
      const result = await postprocess(row.input, { mode: row.mode, punctuation: runtime.package });
      if (result.text !== row.expected)
        failures.push({ id: row.id, expected: row.expected, actual: result.text, stages: result.stages });
    }
  } finally {
    await runtime.close();
  }
  t.diagnostic(`${fixture.rows.length - failures.length}/${fixture.rows.length} exact matches; no numeric exceptions`);
  assert.deepEqual(failures, []);
});
