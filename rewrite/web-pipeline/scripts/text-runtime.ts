import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import * as ort from "onnxruntime-node";
import { loadPunctuationPackage } from "../src/text/index.ts";
import { punctuationRunner } from "../src/text/runner.ts";
import { PUNCTUATION } from "../src/models.ts";
import type { PunctuationPackage } from "../src/text/contract.ts";

// The package the browser ships (src/models.ts PUNCTUATION). Set SONOJOT_PUNCT_MODEL_DIR to its folder.
export const modelDirectory =
  process.env.SONOJOT_PUNCT_MODEL_DIR ??
  join(process.env.LOCALAPPDATA ?? "", "SonoJot/models/rewrite/punctuation/sonojot-punct-v4");
export const hasModel = ["model.int8.onnx", "tokenizer.json", "punct-config.json"].every((name) =>
  existsSync(join(modelDirectory, name)),
);

export async function openPunctuation(): Promise<{ package: PunctuationPackage; close: () => Promise<void> }> {
  const config: unknown = JSON.parse(readFileSync(join(modelDirectory, "punct-config.json"), "utf8"));
  const tokenizer: unknown = JSON.parse(readFileSync(join(modelDirectory, "tokenizer.json"), "utf8"));
  const session = await ort.InferenceSession.create(join(modelDirectory, "model.int8.onnx"), {
    executionProviders: ["cpu"],
    intraOpNumThreads: 4,
    interOpNumThreads: 1,
    graphOptimizationLevel: "all",
  });
  // The same feeds, output checks and pinned manifest values as the browser worker.
  const pkg = loadPunctuationPackage(config, tokenizer, punctuationRunner(ort, () => session), PUNCTUATION.manifest);
  return { package: pkg, close: () => session.release() };
}
