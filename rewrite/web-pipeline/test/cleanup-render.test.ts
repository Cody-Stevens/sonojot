// The Clean renderer for packages with a delete head (punct-v4), without a model.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { renderClean, RENDER_WORD_LISTS } from "../src/text/cleanup-render.ts";
import { deleteDecisions } from "../src/text/model.ts";
import { Document } from "../src/text/punctuation.ts";
import type { Mark } from "../src/text/punctuation.ts";

const rust = (file: string) => readFileSync(new URL(`../../src-tauri/src/${file}`, import.meta.url), "utf8");

test("render_clean matches the desktop's synthetic v4 cases", () => {
  // The same cases rewrite/src-tauri/src/punctuation/cleanup_render.rs checks (cleanup_v4_parity).
  const rows: { text: string; punct: string[]; delete: boolean[]; expected: string }[] = JSON.parse(
    readFileSync(new URL("../../evaluation/cleanup-v4/synthetic.json", import.meta.url), "utf8"),
  );
  assert.ok(rows.length > 0);
  for (const row of rows) {
    const marks = row.punct.map((m) => ([".", ",", "?", ";"].includes(m) ? m : "") as Mark);
    assert.equal(renderClean(new Document(row.text), marks, row.delete).text, row.expected, row.text);
  }
});

test("the renderer's word lists are the desktop's", () => {
  const source = rust("punctuation/cleanup_render.rs");
  for (const [name, words] of Object.entries(RENDER_WORD_LISTS)) {
    const body = new RegExp(`const ${name}: &\\[&str\\] =\\s*&\\[([^\\]]*)\\]`).exec(source)?.[1];
    assert.ok(body, name);
    assert.deepEqual([...body.matchAll(/"([^"]*)"/g)].map((m) => m[1]), [...words], name);
  }
});

// The same cases as cleanup_render.rs's literal_gaps tests.
const render = (text: string, deleted: number[] = [], options: { unprotect?: boolean; marks?: Record<number, Mark> } = {}) => {
  const doc = new Document(text);
  if (options.unprotect) for (const w of doc.words) w.protected = false;
  const marks = doc.words.map((_, i) => options.marks?.[i] ?? ("" as Mark));
  return renderClean(doc, marks, doc.words.map((_, i) => deleted.includes(i))).text;
};

test("literal content between kept words is kept as written", () => {
  assert.equal(render("which is 20% of it"), "Which is 20% of it.");
  assert.equal(render("hello \u{1F600} world"), "Hello \u{1F600} world.");
  assert.equal(render("keep `ten dollars` as spoken"), "Keep `ten dollars` as spoken.");
  assert.equal(render("keep cafe\u0301 unchanged"), "Keep cafe\u0301 unchanged.");
  assert.equal(render("The word is `do not delete um`."), "The word is `do not delete um`.");
});

test("model punctuation stays out of a protected span", () => {
  assert.equal(render("The word is `do not delete um`.", [], { marks: { 5: "," } }), "The word is `do not delete um`.");
});

test("a word with a glued piece is a protected literal, and the piece goes with a deleted word", () => {
  assert.equal(render("it is um 20% no 30% of it", [2, 3, 4]), "It is 20% 30% of it.");
  assert.equal(render("it is um 20% no 30% of it", [2, 3, 4], { unprotect: true }), "It is 30% of it.");
  assert.equal(render("it costs 20% um", [3]), "It costs 20%.");
});

test("a free-standing piece stays unless both neighbours are deleted", () => {
  assert.equal(render("hello um \u{1F600} world", [1]), "Hello \u{1F600} world.");
  assert.equal(render("hello \u{1F600} um world", [1]), "Hello \u{1F600} world.");
  assert.equal(render("hello um \u{1F600} uh world", [1, 2]), "Hello world.");
  assert.equal(render('um I need the "blue" folder', [0]), 'I need the "blue" folder.');
});

test("a word is deleted when P(DELETE) reaches the threshold, computed in f32", () => {
  // Logits laid out [tokens x 2]; positions are the words' last tokens.
  const logits = (pairs: [number, number][]) => Float32Array.from(pairs.flat());
  const at = (keep: number, del: number) => deleteDecisions(logits([[0, 0], [keep, del], [0, 0]]), 3, [1], 0.955)[0];
  const edge = Math.log(0.955 / 0.045); // P(DELETE) = 0.955
  assert.equal(at(0, edge + 1e-3), true);
  assert.equal(at(0, edge - 1e-3), false);
  assert.equal(at(5, -5), false);
  assert.throws(() => deleteDecisions(logits([[0, 0]]), 3, [1], 0.955), /Invalid delete output/);
});
