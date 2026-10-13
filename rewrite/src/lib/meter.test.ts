import { strict as assert } from "node:assert";
import { test } from "node:test";
import { meterDescription, normalizeRms, smoothMeter } from "./meter";

test("meter maps real RMS onto a bounded decibel display scale", () => {
  assert.equal(normalizeRms(0), 0);
  assert.equal(normalizeRms(0.0001), 0);
  assert.equal(normalizeRms(0.001), 0);
  assert.ok(Math.abs(normalizeRms(0.01) - 20 / 48) < 1e-12);
  assert.equal(normalizeRms(1), 1);
  for (const invalid of [NaN, Infinity, -1])
    assert.equal(normalizeRms(invalid), 0);
});
test("attack is faster than release and silence settles to zero", () => {
  const attack = smoothMeter(0, 1, 45);
  const release = smoothMeter(1, 0, 45);
  assert.ok(attack > 0.6 && attack < 0.7);
  assert.ok(release > 0.7 && release < 0.8);
  assert.equal(smoothMeter(0, 0, 500), 0);
  assert.equal(smoothMeter(0.5, 0, 2000), 0);
  assert.equal(smoothMeter(0.5, 1, 0), 0.5);
  assert.ok(
    Math.abs(smoothMeter(smoothMeter(0, 1, 20), 1, 25) - attack) < 1e-12,
  );
});
test("accessible meter text reports signal measurements, not invented activity", () => {
  assert.equal(meterDescription(0, true), "Silence");
  assert.equal(meterDescription(0.01, true), "-40 dBFS");
  assert.equal(meterDescription(0.0001, true), "Below -60 dBFS");
  assert.equal(meterDescription(0.9, false), "Audio input inactive");
});
