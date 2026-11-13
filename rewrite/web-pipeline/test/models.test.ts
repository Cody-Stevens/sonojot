import assert from "node:assert/strict";
import { test } from "node:test";
import { folderUrls, huggingFaceUrls, PARAKEET, PUNCTUATION } from "../src/models.ts";

test("huggingFaceUrls resolves files at the repository root or in a subfolder", () => {
  const url = huggingFaceUrls({
    [PUNCTUATION.id]: { repo: "SonoJot/sonojot-punct-v3", revision: "abc123" },
    [PARAKEET.id]: { repo: "SonoJot/sonojot-parakeet-v1", revision: "def456", path: "web" },
  });
  assert.equal(
    url(PUNCTUATION.files.tokenizer),
    "https://huggingface.co/SonoJot/sonojot-punct-v3/resolve/abc123/tokenizer.json",
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
