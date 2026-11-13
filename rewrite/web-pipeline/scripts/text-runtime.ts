import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import * as ort from "onnxruntime-node";
import { loadPunctuationPackage } from "../src/text/index.ts";
import type { PunctuationPackage } from "../src/text/contract.ts";

export const modelDirectory =
  process.env.SONOJOT_PUNCT_MODEL_DIR ??
  join(process.env.LOCALAPPDATA ?? "", "SonoJot/models/rewrite/punctuation/sonojot-punct-v3");
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
  const pkg = loadPunctuationPackage(config, tokenizer, {
    async run({ inputIds }) {
      const shape = [1, inputIds.length];
      const output = await session.run({
        input_ids: new ort.Tensor("int64", BigInt64Array.from(inputIds, BigInt), shape),
        attention_mask: new ort.Tensor("int64", new BigInt64Array(inputIds.length).fill(1n), shape),
      });
      const logits = output.logits,
        spans = output.span_logits;
      if (
        !logits ||
        !(logits.data instanceof Float32Array) ||
        logits.dims[0] !== 1 ||
        logits.dims[1] !== inputIds.length ||
        logits.dims.length !== 3
      )
        throw new Error("Invalid logits tensor");
      if (
        spans &&
        (!(spans.data instanceof Float32Array) ||
          spans.dims[0] !== 1 ||
          spans.dims[1] !== inputIds.length ||
          spans.dims.length !== 3)
      )
        throw new Error("Invalid span tensor");
      return { logits: logits.data, ...(spans ? { spanLogits: spans.data as Float32Array } : {}) };
    },
  });
  return { package: pkg, close: () => session.release() };
}
