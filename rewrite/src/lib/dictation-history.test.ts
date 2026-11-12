import { test } from "node:test";
import assert from "node:assert/strict";
import { dictationRecordSchema } from "./protocol";
import { dayPages, replaceWord, versionText } from "./dictation-history";

const record = (id: string, createdAt: string, published = id) =>
  dictationRecordSchema.parse({
    id,
    createdAt,
    published,
    raw: "as heard",
    preModelText: "before cleanup",
  });
test("history groups local days across month boundaries and searches saved versions", () => {
  const now = new Date(2026, 10, 1, 12);
  const rows = [
    record("older", new Date(2026, 9, 31, 20).toISOString()),
    record("new", new Date(2026, 10, 1, 9).toISOString()),
  ];
  assert.deepEqual(
    dayPages(rows, "", now).map((day) => [day.label, day.records[0].id]),
    [
      ["Today", "new"],
      ["Yesterday", "older"],
    ],
  );
  assert.equal(dayPages(rows, "AS HEARD", now).length, 2);
  assert.equal(dayPages(rows, "missing", now).length, 0);
});
test("versions keep raw and pre-model text separate, including missing pre-model text", () => {
  const row = record("one", "2026-11-06T12:00:00Z", "final");
  assert.equal(versionText(row, "published"), "final");
  assert.equal(versionText(row, "raw"), "as heard");
  assert.equal(versionText(row, "pre_model"), "before cleanup");
  assert.equal(versionText({ ...row, preModelText: null }, "pre_model"), null);
});
test("word correction treats punctuation literally and changes only the first match", () => {
  assert.equal(
    replaceWord("Keep C++ and C++.", "c++", "Rust"),
    "Keep Rust and C++.",
  );
  assert.equal(replaceWord("Keep words", "missing", "new"), null);
  assert.equal(replaceWord("Keep words", "", "new"), null);
  assert.equal(replaceWord("Keep the weather", "he", "she"), null);
  assert.equal(replaceWord("Keep Émilie", "émilie", "Emily"), "Keep Emily");
});
