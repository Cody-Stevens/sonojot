import assert from "node:assert/strict";
import test from "node:test";
import { EDGE_FADE_PX, edgeState } from "./scroll-edges";

const metrics = (
  scrollTop: number,
  scrollHeight = 1000,
  clientHeight = 400,
) => ({
  scrollTop,
  scrollHeight,
  clientHeight,
});

test("content that fits shows no scroll edges", () => {
  assert.deepEqual(edgeState(metrics(0, 400, 400)), { start: 0, end: 0 });
  // Sub-pixel rounding is not a scroll range.
  assert.deepEqual(edgeState(metrics(0, 400.6, 400)), { start: 0, end: 0 });
});

test("only the bottom edge shows at the top, only the top at the bottom", () => {
  assert.deepEqual(edgeState(metrics(0)), { start: 0, end: 1 });
  assert.deepEqual(edgeState(metrics(600)), { start: 1, end: 0 });
  assert.deepEqual(edgeState(metrics(300)), { start: 1, end: 1 });
});

test("each edge fades over the last 28px of remaining scroll", () => {
  assert.equal(EDGE_FADE_PX, 28);
  assert.deepEqual(edgeState(metrics(14)), { start: 0.5, end: 1 });
  assert.deepEqual(edgeState(metrics(586)), { start: 1, end: 0.5 });
  // Less than a pixel from an end counts as at the end.
  assert.deepEqual(edgeState(metrics(599.5)), { start: 1, end: 0 });
});

test("overscroll and elastic bounce never invert or exceed the edges", () => {
  assert.deepEqual(edgeState(metrics(-40)), { start: 0, end: 1 });
  assert.deepEqual(edgeState(metrics(700)), { start: 1, end: 0 });
});
