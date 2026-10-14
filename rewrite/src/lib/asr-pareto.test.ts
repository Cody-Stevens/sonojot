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

test("every chart point is identifiable at every width", () => {
  for (const device of ["CPU", "GPU"] as const) {
    for (const width of [
      270, 300, 330, 360, 420, 450, 500, 540, 590, 650, 700, 760, 860, 1000,
    ]) {
      const rows = devicePoints(device);
      const { labels, key, x, y, unplaced } = chartLayout(rows, width);
      const where = `${device} ${width}`;
      assert.ok(x(1) < x(10));
      assert.ok(y(6) > y(9), "fewer errors are toward the bottom");
      assert.deepEqual(unplaced, [], `${where}: no free space for some labels`);
      for (const p of rows) {
        const own = labels.filter((l) => l.point === p);
        assert.equal(own.length, 1, `${where}: ${p.name} needs one label`);
        const [label] = own;
        if (p.offered || p.name.startsWith("Whisper")) {
          assert.equal(label.text, p.name, `${where}: ${p.name} is named`);
        } else if (label.key !== undefined) {
          assert.equal(label.text, String(label.key));
          assert.equal(key.find((k) => k.key === label.key)?.point, p);
        } else {
          assert.equal(label.text, p.name);
        }
      }
      assert.deepEqual(
        key.map((k) => k.key),
        key.map((_, i) => i + 1),
        `${where}: key numbers run in order`,
      );
      for (const l of labels) {
        assert.ok(
          l.x >= 28 && l.x + l.width <= width,
          `${where}: clipped ${l.text}`,
        );
        assert.ok(
          l.y >= 32 && l.y + l.height <= 288,
          `${where}: clipped ${l.text}`,
        );
        if (l.leader) {
          const px = Math.max(l.x, Math.min(l.x + l.width, l.leader.x2));
          const py = Math.max(l.y, Math.min(l.y + l.height, l.leader.y2));
          assert.ok(
            Math.hypot(px - l.leader.x2, py - l.leader.y2) < 0.01,
            `${where}: ${l.text} leader ends on its label`,
          );
          assert.equal(l.leader.x1, x(l.point.gb));
          assert.equal(l.leader.y1, y(l.point.wer));
        }
        for (const other of labels.filter((other) => other !== l)) {
          assert.ok(
            l.x >= other.x + other.width ||
              l.x + l.width <= other.x ||
              l.y >= other.y + other.height ||
              l.y + l.height <= other.y,
            `${where}: ${l.text} overlaps ${other.text}`,
          );
        }
      }
    }
  }
});
