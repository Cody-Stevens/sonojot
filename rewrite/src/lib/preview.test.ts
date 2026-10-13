import { strict as assert } from "node:assert";
import { test } from "node:test";
import { suffixPreviewOffset } from "./preview";

test("preview only shifts overflowing width to keep the newest right edge visible", () => {
  assert.equal(suffixPreviewOffset(600, 400), 200);
  assert.equal(suffixPreviewOffset(300, 400), 0);
  assert.equal(suffixPreviewOffset(400, 400), 0);
  assert.equal(suffixPreviewOffset(600, 500), 100);
  assert.equal(suffixPreviewOffset(NaN, 400), 0);
  assert.equal(suffixPreviewOffset(300, Infinity), 0);
});
