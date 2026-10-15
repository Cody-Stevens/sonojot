import { strict as assert } from "node:assert";
import { test } from "node:test";
import { LIVE_PREVIEW_MODEL, livePreviewHelpText } from "./live-preview";
import type { ModelEntry } from "./model-library";

const model = (state: string, supported = true): ModelEntry => ({
  id: LIVE_PREVIEW_MODEL,
  name: "Moonshine Tiny Streaming",
  kind: "asr",
  roles: ["preview"],
  family: "Moonshine",
  languages: ["en"],
  older: false,
  deprecated: false,
  diskBytes: 45_233_659,
  ramMinBytes: null,
  ramMaxBytes: null,
  vramMinBytes: 0,
  vramMaxBytes: 0,
  memoryBasis: "",
  guidance: "",
  evidence: "",
  backend: "moonshine-ort-cpu",
  compatibility: "",
  state,
  downloadedBytes: 0,
  totalBytes: 45_233_659,
  error: null,
  installable: state !== "installed",
  removable: state === "installed",
  usedBy: [],
  revision: "quantized_26_08_21",
  quantization: "bundled-ort",
  contextTokens: null,
  supported,
  stage: null,
});

test("live preview help names the download it still needs", () => {
  const missing = livePreviewHelpText(
    model("not-installed"),
    "qwen3-asr-1.7b-q8-cuda",
  );
  assert.match(missing, /never reaches your text/);
  assert.match(missing, /Install Moonshine Tiny Streaming \(46 MB\) in Models/);
  assert.doesNotMatch(
    livePreviewHelpText(model("installed"), "qwen3-asr-1.7b-q8-cuda"),
    /Install/,
  );
  assert.match(
    livePreviewHelpText(model("unavailable", false), "whisper-small"),
    /Not available on this computer/,
  );
  assert.match(
    livePreviewHelpText(model("installed"), "moonshine-medium"),
    /already shows live text/,
  );
});
