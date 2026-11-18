import assert from "node:assert/strict";
import { test } from "node:test";
import { isHardwareAdapter } from "../src/gpu.ts";

test("automatic device choice skips software WebGPU adapters", () => {
  assert.equal(isHardwareAdapter(null), false);
  assert.equal(isHardwareAdapter(undefined), false);
  // Chrome on Linux without Vulkan offers SwiftShader, flagged as a fallback adapter.
  assert.equal(isHardwareAdapter({ info: { isFallbackAdapter: true } }), false);
  assert.equal(isHardwareAdapter({ isFallbackAdapter: true }), false);
  assert.equal(isHardwareAdapter({ info: { isFallbackAdapter: false } }), true);
  // Browsers that do not report the flag are taken at their word.
  assert.equal(isHardwareAdapter({}), true);
  assert.equal(isHardwareAdapter({ info: {} }), true);
});
