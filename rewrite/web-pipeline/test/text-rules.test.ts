import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { postprocess } from "../src/text/index.ts";
import { localRules } from "../src/text/rules.ts";
import { applyWrittenForm } from "../src/text/written-form.ts";
import { renderSpan, renderTerms, projectSpanLabels, applySpans } from "../src/text/written-form.ts";
import type { Class } from "../src/text/written-form.ts";
import { Document, lowerSegmentStartCapitals, renderGap, respaceGluedApostrophes } from "../src/text/punctuation.ts";
import type { Mark } from "../src/text/punctuation.ts";
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

// Rust punctuation.rs::a_word_with_a_leading_apostrophe_keeps_its_space_on_both_paths (rules path).
function agreeingRebuild(raw: string): string {
  const text = respaceGluedApostrophes(raw);
  const document = new Document(text);
  const count = document.words.length;
  const labels: Mark[] = document.words.map((_, i) => {
    const gap = document.gap(i);
    if (gap.includes("?")) return "?";
    if (/[.!]/.test(gap) && !gap.includes("…")) return ".";
    if (/[,…]/.test(gap)) return ",";
    return i + 1 === count ? "." : "";
  });
  return document.rebuild(labels, true);
}

test("a word with a leading apostrophe keeps its space after a mark", () => {
  for (const a of ["'", "’"]) {
    const cases: [string, string][] = [
      [`is the oven still on?${a}Cause it smells hot`, `Is the oven still on? ${a}Cause it smells hot.`],
      [`is it done?${a}cause I want it`, `Is it done? ${a}Cause I want it.`],
      [`is it done? ${a}cause I want it`, `Is it done? ${a}Cause I want it.`],
      [`we waited.${a}til noon it rained`, `We waited. ${a}Til noon it rained.`],
      [`call them now!${a}em too`, `Call them now. ${a}Em too.`],
      [`we talked…${a}bout the plan`, `We talked, ${a}bout the plan.`],
      [`yeah,${a}cause you said so`, `Yeah, ${a}cause you said so.`],
      [`it was loud.${a}tis the season`, `It was loud. ${a}Tis the season.`],
      [`it was the music then.${a}90s songs mostly`, `It was the music then. ${a}90s songs mostly.`],
      [`${a}cause I said so`, `${a}Cause I said so.`],
      [`she said "${a}tis fine" today`, `She said "${a}tis fine" today.`],
      [`we left early (${a}cause it rained) today`, `We left early (${a}cause it rained) today.`],
      [`we play rock ${a}n${a} roll ${a}til late`, `We play rock ${a}n${a} roll ${a}til late.`],
      [`he yelled ${a}stop?${a}Then he left`, `He yelled ${a}stop?${a} Then he left.`],
      [`the dogs${a} bowls and James${a} car`, `The dogs${a} bowls and James${a} car.`],
      [`we fed the dogs${a}, then James${a} cat`, `We fed the dogs${a}, then James${a} cat.`],
    ];
    for (const [raw, expected] of cases) assert.equal(agreeingRebuild(raw), expected, raw);
  }
  assert.equal(agreeingRebuild("is it done?‘cause I want it"), "Is it done? ‘Cause I want it.");
});

test("glued apostrophe respacing leaves other apostrophes alone", () => {
  for (const text of [
    "the U.S.'s plan works",
    "it's fine, isn't it",
    "rock'n'roll",
    "the dogs' bowls",
    "James' car",
    "he said 'no' and left",
    "I know.'",
    "a.m.'d",
    "",
  ])
    assert.equal(respaceGluedApostrophes(text), text);
  assert.equal(respaceGluedApostrophes("one?'Cause two,’cuz three.'em"), "one? 'Cause two, ’cuz three. 'em");
  assert.equal(respaceGluedApostrophes("say 'stop?'Then go"), "say 'stop?' Then go");
  assert.equal(respaceGluedApostrophes("say 'stop?'cause go"), "say 'stop? 'cause go");
  assert.equal(respaceGluedApostrophes("'cause I said.'Then go"), "'cause I said. 'Then go");
  assert.equal(respaceGluedApostrophes("say 'stop'\nok?'Then go"), "say 'stop'\nok? 'Then go");
});

test("postprocess restores the space without a punctuation model", async () => {
  assert.equal((await postprocess("is it done?'cause I want it", { mode: "format" })).text, "Is it done? 'Cause I want it.");
});

// Rust punctuation.rs::segment_start_capitals_are_lowered_only_where_no_sentence_ended.
test("segment-start capitals are lowered only where no sentence ended", async () => {
  const lower = (segments: string[]) => lowerSegmentStartCapitals(segments.join(" "), segments) ?? segments.join(" ");
  assert.equal(lower(["we should file a feature", "Request for the export button"]), "we should file a feature request for the export button");
  assert.equal(lower(["it feels kind of weird and", "Kind of slow when it starts"]), "it feels kind of weird and kind of slow when it starts");
  assert.equal(lower(["move the files into", "The archive", "Folder now"]), "move the files into the archive folder now");
  assert.equal(lower(["the build passed.", "Then the tests failed"]), "the build passed. Then the tests failed");
  assert.equal(lower(["did it pass?", "Maybe not"]), "did it pass? Maybe not");
  for (const segments of [["then", "I think so"], ["ask the", "NASA team"], ["open", "GitHub today"], ["pick plan", "A today"], ["call", "Zorvath tomorrow"], ["she said", '"Hello there']])
    assert.equal(lower(segments), segments.join(" "));
  assert.equal(lower(["open the Speech page then the", "Speech settings"]), "open the Speech page then the Speech settings");
  assert.equal(lowerSegmentStartCapitals("other text", ["a", "Request"]), undefined);
  assert.equal(lowerSegmentStartCapitals("one segment", ["one segment"]), undefined);
  const input = lowerSegmentStartCapitals("we should file a feature Request for the export button", ["we should file a feature", "Request for the export button"])!;
  assert.equal((await postprocess(input, { mode: "format" })).text, "We should file a feature request for the export button.");
});
