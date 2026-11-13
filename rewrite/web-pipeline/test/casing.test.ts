// Rust punctuation/casing.rs tests, with the same invented sentences.
import assert from "node:assert/strict";
import { test } from "node:test";
import { boundsFor, capitalRun, hintId, modelCase } from "../src/text/casing.ts";
import type { CaseThresholds } from "../src/text/casing.ts";
import { Document } from "../src/text/punctuation.ts";
import type { Mark } from "../src/text/punctuation.ts";

const T: CaseThresholds = { raise: 0.7, upper: 0.8, lower_positional: 0.6, lower: 0.97 };

test("raising needs less than lowering", () => {
  assert.equal(modelCase("paris", [0.2, 0.8, 0], false, false, T), "Paris");
  assert.equal(modelCase("Paris", [0.8, 0.2, 0], false, false, T), undefined);
  assert.equal(modelCase("Paris", [0.8, 0.2, 0], true, false, T), "paris");
  assert.equal(modelCase("Paris", [0.99, 0.01, 0], false, false, T), "paris");
  assert.equal(modelCase("Paris", [0.99, 0.01, 0], true, true, T), undefined);
  assert.equal(modelCase("api", [0.1, 0, 0.9], false, false, T), "API");
  assert.equal(modelCase("Rust", [0.05, 0.1, 0.85], false, false, T), undefined);
  assert.equal(modelCase("github", [1, 0, 0], false, false, T), "GitHub");
  assert.equal(modelCase("version2", [0, 1, 0], false, false, T), undefined);
  assert.equal(modelCase("Because", [0.3, 0.7, 0], false, false, T), "because");
  assert.equal(modelCase("The", [0.3, 0.7, 0], false, true, T), undefined);
});

test("capital runs, hints and piece bounds match the desktop", () => {
  const words = ["we", "support", "Black", "Lives", "Matter", "and", "the", "Elimination", "of", "All", "Forms"];
  const none = words.map(() => false);
  assert.ok(capitalRun(words, none, 2) && capitalRun(words, none, 4) && capitalRun(words, none, 7));
  assert.ok(!capitalRun(["file", "a", "feature", "Request", "for", "the", "export"], none, 3));
  assert.ok(!capitalRun(["to", "Paris", "And", "then"], [false, false, true, false], 2));
  assert.deepEqual([hintId("hello", false), hintId("Hello", true), hintId("NASA", false), hintId("GitHub", false), hintId("42", true)], [3, 6, 7, 9, 2]);
  assert.deepEqual(
    boundsFor(["We", "met", "at", "the", "Summit", "today", "and", "then", "left"], ["um we met at the", "Summit today", "and then left"]),
    [false, false, false, false, true, false, true, false, false],
  );
});

test("the rebuild applies the model choice only off sentence starts", () => {
  const probs = (w: string): [number, number, number] =>
    ["maria", "paris"].includes(w) ? [0.1, 0.9, 0] : w === "summit" || w === "request" ? [0.9, 0.1, 0] : [0.999, 0.001, 0];
  const rebuilt = (text: string, bounds: number[] = []) => {
    const doc = new Document(text);
    doc.casing = {
      probs: doc.words.map((w) => probs(w.text.toLowerCase())),
      bounds: doc.words.map((_, i) => bounds.includes(i)),
      thresholds: T,
    };
    const marks: Mark[] = doc.words.map((_, i) => (i + 1 === doc.words.length ? "." : ""));
    return doc.rebuild(marks, true);
  };
  assert.equal(rebuilt("we met maria in paris"), "We met Maria in Paris.");
  assert.equal(rebuilt("the Summit starts now"), "The Summit starts now.");
  assert.equal(rebuilt("we ate an Apple today"), "We ate an apple today.");
  assert.equal(rebuilt("we ate an Apple Pie today"), "We ate an Apple Pie today.");
  assert.equal(rebuilt("we need the. Summit"), "We need the summit.");
  assert.equal(rebuilt("file a feature Request now", [3]), "File a feature request now.");
  assert.equal(rebuilt("iphone sales grew"), "iPhone sales grew.");
});
