import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { postprocess } from "../src/text/index.ts";
import { localRules } from "../src/text/rules.ts";
import { applyWrittenForm } from "../src/text/written-form.ts";
import { renderSpan, renderTerms, projectSpanLabels, applySpans } from "../src/text/written-form.ts";
import type { Class } from "../src/text/written-form.ts";
import { Document, renderGap } from "../src/text/punctuation.ts";
import { decodeLogits } from "../src/text/model.ts";

test("fillers, repetitions and numeric restarts preserve guarded content", () => {
  assert.equal(localRules("um the the file is ready", "clean"), "The file is ready.");
  assert.equal(localRules("we need twelve, actually, fifteen pages", "clean"), "We need fifteen pages.");
  assert.equal(localRules("keep `um the the` unchanged", "clean"), "Keep `um the the` unchanged.");
  assert.equal(localRules("email sam at comet.", "clean"), "Email sam at comet.");
  assert.equal(localRules("email today at comet dot io", "clean"), "Email today@comet.io");
  assert.equal(localRules("email sam at tomorrow dot example dot org", "clean"), "Email sam@tomorrow.example.org");
  assert.equal(localRules("um the the file is ready", "format"), "Um the the file is ready.");
});

test("Original preserves all bytes and abort prevents a publishable result", async () => {
  const raw = "  um\n naïve\t";
  assert.equal((await postprocess(raw, { mode: "original" })).text, raw);
  await assert.rejects(postprocess(raw, { mode: "clean", signal: AbortSignal.abort() }), { name: "AbortError" });
  assert.equal((await postprocess("  \t\n", { mode: "clean" })).text, "  \t\n");
});

test("written-form renderer agrees with Rust span goldens without loading a model", () => {
  const fixture: {
    writtenFormCases: {
      id: string;
      text: string;
      class: Class;
      before?: string;
      after?: string;
      expected: string | null;
    }[];
    termCases: [string, string][];
  } = JSON.parse(readFileSync(new URL("./fixtures/text-parity.json", import.meta.url), "utf8"));
  const failures = fixture.writtenFormCases.flatMap((row) => {
    const actual = renderSpan(row.text, row.class, row.before, row.after) ?? null;
    return actual === row.expected ? [] : [{ ...row, actual }];
  });
  assert.deepEqual(failures, []);
  for (const [input, expected] of fixture.termCases) assert.equal(renderTerms(input), expected, input);
});

test("fixed BIO labels select safe complete spans and do not license unrelated rewrites", () => {
  assert.equal(applyWrittenForm("pay ten dollars.", ["O", "B-MONEY", "I-MONEY"]), "pay $10.");
  assert.equal(applyWrittenForm("pay ten dollars.", ["O", "O", "O"]), "pay ten dollars.");
  assert.equal(applyWrittenForm("ten dollars", ["B-MONEY"]), "ten dollars");
  assert.equal(applyWrittenForm("ten dollars", ["I-MONEY", "I-MONEY"]), "$10");
  assert.equal(applyWrittenForm("`ten dollars`", ["B-MONEY", "I-MONEY"]), "`ten dollars`");
  assert.equal(applyWrittenForm("ten\n dollars", ["B-MONEY", "I-MONEY"]), "ten\n dollars");
  assert.equal(
    applyWrittenForm("two dollars and one hundred cents", [
      "B-MONEY",
      "I-MONEY",
      "I-MONEY",
      "I-MONEY",
      "I-MONEY",
      "I-MONEY",
    ]),
    "two dollars and one hundred cents",
  );
  assert.deepEqual(projectSpanLabels("4.2 ten dollars", ["B-MONEY", "O", "B-MONEY", "I-MONEY"]), [
    "O",
    "B-MONEY",
    "I-MONEY",
  ]);
  assert.equal(
    applySpans("ten dollars", [
      { start: 0, end: 2, class: "MONEY" },
      { start: 1, end: 2, class: "CARDINAL" },
    ]),
    "ten dollars",
  );
});

test("punctuation protects literals and uses strict first-class argmax ties", () => {
  assert.equal(renderGap(" — ", "", false), ", ");
  assert.equal(renderGap(" 😀 ", ".", false), " 😀 ");
  assert.equal(
    new Document("visit https://example.com today").rebuild(["", "", "", "", "."]),
    "Visit https://example.com today.",
  );
  assert.deepEqual(decodeLogits(new Float32Array([0, 0, 2, 2, 0, 0]), 3, [1], ["first", "second"]), ["first"]);
  assert.throws(() => decodeLogits(new Float32Array([0, 0, NaN, 2, 0, 0]), 3, [1], ["first", "second"]));
});
