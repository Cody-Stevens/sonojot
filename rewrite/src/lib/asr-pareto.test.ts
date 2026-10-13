import assert from "node:assert/strict";
import test from "node:test";
import {
  chartLayout,
  devicePoints,
  frontier,
  points,
  type AsrPoint,
} from "./asr-pareto";

test("chart keeps finite comparable values and distinct names", () => {
  assert.equal(new Set(points.map((p) => p.name)).size, points.length);
  for (const p of points) {
    assert.ok(Number.isFinite(p.gb) && p.gb > 0);
    assert.ok(Number.isFinite(p.wer) && p.wer > 0 && p.wer < 100);
    assert.ok(p.reason.length > 10);
  }
  assert.ok(devicePoints("CPU").every((p) => p.device !== "GPU"));
  assert.ok(devicePoints("GPU").every((p) => p.device !== "CPU"));
});

test("frontier handles ties, dominance, empty input and input order", () => {
  const p = (name: string, gb: number, wer: number): AsrPoint => ({
    name,
    gb,
    wer,
    offered: false,
    device: "CPU",
    reason: "test",
  });
  const input = [
    p("large", 3, 5),
    p("bad tie", 1, 8),
    p("small", 1, 7),
    p("equal", 1, 7),
    p("dominated", 2, 7),
  ];
  assert.deepEqual(
    frontier(input).map((p) => p.name),
    ["small", "equal", "large"],
  );
  assert.equal(input[0].name, "large");
  assert.deepEqual(frontier([]), []);
});

test("chart labels keep offered models and Whisper readable at every width", () => {
  for (const device of ["CPU", "GPU"] as const) {
    for (const width of [270, 330, 590, 1000]) {
      const rows = devicePoints(device);
      const { labels, x, y } = chartLayout(rows, width);
      assert.ok(x(1) < x(10));
      assert.ok(y(6) > y(9), "fewer errors are toward the bottom");
      for (const p of rows.filter(
        (p) => p.offered || p.name.startsWith("Whisper"),
      )) {
        assert.ok(
          labels.some((l) => l.point === p),
          `${device} ${width}: missing ${p.name}`,
        );
      }
      for (const l of labels) {
        assert.ok(
          l.x >= 28 && l.x + l.width <= width,
          `${width}: clipped ${l.point.name}`,
        );
        assert.ok(l.y >= 32 && l.y + l.height <= 283);
        for (const other of labels.filter((other) => other !== l)) {
          assert.ok(
            l.x >= other.x + other.width ||
              l.x + l.width <= other.x ||
              l.y >= other.y + other.height ||
              l.y + l.height <= other.y,
            `${device} ${width}: ${l.point.name} overlaps ${other.point.name}`,
          );
        }
      }
      if (width >= 590) {
        for (const p of rows.filter((p) =>
          /^(Canary|Voxtral|Granite 3\.3|Phi-4)/.test(p.name),
        )) {
          assert.ok(
            labels.some((l) => l.point === p),
            `${width}: missing notable ${p.name}`,
          );
        }
      }
    }
  }
});
