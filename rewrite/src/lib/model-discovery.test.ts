import assert from "node:assert/strict";
import test from "node:test";
import { discoverable } from "./model-library";

test("legacy packages stay reachable for installed and selected models", () => {
  assert.equal(
    discoverable({ deprecated: true, state: "available", usedBy: [] }),
    false,
  );
  assert.equal(
    discoverable({ deprecated: false, state: "available", usedBy: [] }),
    true,
  );
  assert.equal(
    discoverable({ deprecated: true, state: "installed", usedBy: [] }),
    true,
  );
  assert.equal(
    discoverable({ deprecated: true, state: "checking", usedBy: [] }),
    true,
  );
  assert.equal(
    discoverable({ deprecated: true, state: "missing", usedBy: ["asr"] }),
    true,
  );
});
