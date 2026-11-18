import assert from "node:assert/strict";
import { test } from "node:test";
import { folderUrls, huggingFaceUrls, PARAKEET, PUNCTUATION } from "../src/models.ts";

test("huggingFaceUrls resolves files at the repository root or in a subfolder", () => {
  const url = huggingFaceUrls({
    [PUNCTUATION.id]: { repo: "SonoJot/sonojot-punct-v4", revision: "abc123" },
    [PARAKEET.id]: { repo: "SonoJot/sonojot-parakeet-v1", revision: "def456", path: "web" },
  });
  assert.equal(
    url(PUNCTUATION.files.tokenizer),
    "https://huggingface.co/SonoJot/sonojot-punct-v4/resolve/abc123/tokenizer.json",
  );
  assert.equal(
    url(PARAKEET.files.encoder),
    "https://huggingface.co/SonoJot/sonojot-parakeet-v1/resolve/def456/web/encoder-model.q4.onnx",
  );
});

test("huggingFaceUrls ignores stray slashes in the subfolder and rejects unknown models", () => {
  const url = huggingFaceUrls({ [PARAKEET.id]: { repo: "o/r", revision: "1", path: "/web/" } });
  assert.equal(url(PARAKEET.files.vocab), "https://huggingface.co/o/r/resolve/1/web/vocab.txt");
  assert.throws(() => url(PUNCTUATION.files.model), /No Hugging Face repository/);
});

test("folderUrls lays files out as <model>/<name>", () => {
  assert.equal(
    folderUrls("https://models.example.com")(PARAKEET.files.decoder),
    `https://models.example.com/${PARAKEET.id}/decoder_joint-model.int8.onnx`,
  );
});

test("the pinned punctuation release matches the desktop's (manifest thresholds, commit and file hashes)", async () => {
  const { readFileSync } = await import("node:fs");
  const rust = readFileSync(new URL("../../src-tauri/src/punctuation_model.rs", import.meta.url), "utf8");
  const deleteThreshold = /const P4_DELETE_THRESHOLD: f32 = ([0-9.]+);/.exec(rust)?.[1];
  assert.equal(Number(deleteThreshold), PUNCTUATION.manifest.delete_threshold);
  const caseBlock = /const P4_CASE_THRESHOLDS[^{]*\{([^}]*)\}/.exec(rust)?.[1] ?? "";
  const caseThresholds = Object.fromEntries([...caseBlock.matchAll(/(\w+):\s*([0-9.]+)/g)].map((m) => [m[1], Number(m[2])]));
  assert.deepEqual(caseThresholds, PUNCTUATION.manifest.case_thresholds);
  assert.match(rust, new RegExp(`P4_RELEASE_COMMIT: Option<&str> = Some\\("${PUNCTUATION.revision}"\\)`));
  for (const file of Object.values(PUNCTUATION.files)) assert.ok(rust.includes(file.sha256), `${file.name} hash pinned by the desktop`);
});
