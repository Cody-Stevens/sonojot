// The one place that turns a punctuation window into ONNX feeds and reads the
// heads back. The browser worker passes onnxruntime-web and the Node parity
// harness passes onnxruntime-node, so both run the graph the same way. The
// engine is a parameter: this module imports none.
import type { PunctuationInput, PunctuationOutput, PunctuationRunner } from "./contract.ts";

/** The part of an onnxruntime tensor the runner reads. */
export interface TensorLike {
  readonly data: unknown;
  readonly dims: readonly number[];
}

/** The part of an onnxruntime module (web or node) the runner uses. */
export interface OrtLike<T extends TensorLike> {
  Tensor: new (type: "int64", data: BigInt64Array, dims: readonly number[]) => T;
}

/** The part of an `InferenceSession` the runner uses. */
export interface SessionLike<T extends TensorLike> {
  run(feeds: Record<string, T>): Promise<Record<string, TensorLike | undefined>>;
}

function head(output: Record<string, TensorLike | undefined>, name: string, tokens: number): Float32Array | undefined {
  const tensor = output[name];
  if (!tensor) return undefined;
  if (!(tensor.data instanceof Float32Array) || tensor.dims.length !== 3 || tensor.dims[0] !== 1 || tensor.dims[1] !== tokens)
    throw new Error(`Invalid ${name} tensor`);
  return tensor.data;
}

/**
 * Feeds `input_ids` and `attention_mask`, plus `case_hints` when the package has
 * a casing label (punct-v4), as int64 [1 x tokens]. `session` is read on each
 * run, so the caller can create the session after the package is parsed.
 */
export function punctuationRunner<T extends TensorLike>(ort: OrtLike<T>, session: () => SessionLike<T>): PunctuationRunner {
  return {
    async run({ inputIds, caseHints }: PunctuationInput): Promise<PunctuationOutput> {
      const tokens = inputIds.length,
        shape = [1, tokens];
      const feeds: Record<string, T> = {
        input_ids: new ort.Tensor("int64", BigInt64Array.from(inputIds, BigInt), shape),
        attention_mask: new ort.Tensor("int64", new BigInt64Array(tokens).fill(1n), shape),
      };
      if (caseHints) feeds.case_hints = new ort.Tensor("int64", BigInt64Array.from(caseHints, BigInt), shape);
      const output = await session().run(feeds);
      const logits = head(output, "logits", tokens);
      if (!logits) throw new Error("Missing logits tensor");
      const spanLogits = head(output, "span_logits", tokens),
        caseLogits = head(output, "case_logits", tokens),
        deleteLogits = head(output, "delete_logits", tokens);
      return {
        logits,
        ...(spanLogits ? { spanLogits } : {}),
        ...(caseLogits ? { caseLogits } : {}),
        ...(deleteLogits ? { deleteLogits } : {}),
      };
    },
  };
}
