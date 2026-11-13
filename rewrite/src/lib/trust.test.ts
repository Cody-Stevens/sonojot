import assert from "node:assert/strict";
import { test } from "node:test";
import {
  decodePcm,
  describe,
  optionsFor,
  placeChip,
  placePopover,
  segments,
  type TrustMark,
} from "./trust";

const word: TrustMark = {
  id: 1,
  kind: "word",
  heard: "Castrel",
  alternatives: [
    { text: "Kestrel", label: "Your words" },
    { text: "Castle", label: "Next guess" },
  ],
  hasAudio: true,
  rects: [],
  from: 10,
  to: 17,
};
const gap: TrustMark = {
  ...word,
  id: 2,
  kind: "gap",
  heard: "vendor",
  alternatives: [],
  from: 30,
  to: 30,
};
const area = { x: 0, y: 0, w: 1000, h: 700 };

test("choices end with keep, gaps offer to add", () => {
  assert.deepEqual(
    optionsFor(word).map((o) => o.label),
    ["Kestrel", "Castle", 'Keep "Castrel"'],
  );
  assert.deepEqual(
    optionsFor(gap).map((o) => o.label),
    ["Nothing missing"],
  );
  assert.match(
    describe(word, 0, 2),
    /Spot 1 of 2: Castrel\. Could be Kestrel, your words/,
  );
  assert.match(describe(gap, 1, 2), /missing after "vendor"/);
});

test("the popover stays on screen and flips above near the bottom", () => {
  const below = placePopover(
    { x: 100, y: 100, w: 60, h: 20 },
    { w: 304, h: 200 },
    area,
    "card",
  );
  assert.equal(below.above, false);
  assert.equal(below.y, 130);
  assert.equal(below.x, 94);
  const edge = placePopover(
    { x: 960, y: 650, w: 30, h: 20 },
    { w: 304, h: 200 },
    area,
    "card",
  );
  assert.equal(edge.above, true);
  assert.ok(edge.x + 304 <= 992, "clamped to the right edge");
  assert.ok(edge.notch >= 10 && edge.notch <= 280, "notch stays on the card");
  const strip = placePopover(
    { x: 500, y: 100, w: 40, h: 20 },
    { w: 200, h: 38 },
    area,
    "strip",
  );
  assert.equal(strip.x, 420, "strip centred on the word");
});

test("chip clamps inside the area", () => {
  const chip = placeChip(
    { x: 990, y: 690, w: 1, h: 20 },
    { w: 110, h: 30 },
    area,
  );
  assert.ok(chip.x + 110 <= 992 && chip.y + 30 <= 692);
});

test("panel text splits around marks", () => {
  const text = "Fix the cash eviction";
  const parts = segments(text, [{ ...word, from: 8, to: 12 }]);
  assert.deepEqual(
    parts.map((p) => p.text),
    ["Fix the ", "cash", " eviction"],
  );
  assert.equal(parts[1].mark?.id, 1);
});

test("pcm decodes signed 16-bit little endian", () => {
  const bytes = String.fromCharCode(0x00, 0x40, 0x00, 0xc0);
  const samples = decodePcm(btoa(bytes));
  assert.deepEqual(Array.from(samples), [0.5, -0.5]);
});
