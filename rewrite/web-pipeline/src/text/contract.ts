// Contract between the text pipeline (punctuation and cleanup) and the runtime
// that hosts it. The text code never imports an inference engine: the browser
// worker passes an onnxruntime-web session and the Node parity tests pass an
// onnxruntime-node session, both wrapped in `PunctuationRunner`.

/** Mirrors the desktop's cleanup modes (`cleanup::Mode`). */
export type CleanupMode = "clean" | "format" | "original";

/** One window of token ids for the punctuation model; batch size is always 1. */
export interface PunctuationInput {
  /** Token ids, CLS first and SEP last, as the desktop encodes them. */
  inputIds: number[];
  /** Per-token casing hint ids, present when the package has a casing label (punct-v4). */
  caseHints?: number[];
}

/** Raw head outputs for one window, row-major [tokens x classes]. */
export interface PunctuationOutput {
  logits: Float32Array;
  /** Present when the package has a written-form span head (v2 and later). */
  spanLogits?: Float32Array;
  /** Present when the package has a casing label: [tokens x 3] lower, Capitalised, UPPER. */
  caseLogits?: Float32Array;
}

/** Runs the ONNX graph. Implementations feed `input_ids` and `attention_mask` (int64). */
export interface PunctuationRunner {
  run(input: PunctuationInput): Promise<PunctuationOutput>;
}

/** A loaded punctuation package (for example sonojot-punct-v3). */
export interface PunctuationPackage {
  /** Parsed punct-config.json. */
  config: unknown;
  /** Parsed tokenizer.json. */
  tokenizer: unknown;
  runner: PunctuationRunner;
}

export interface PostprocessOptions {
  mode: CleanupMode;
  /** Absent: rules only, as the desktop does when no punctuation package is installed. */
  punctuation?: PunctuationPackage;
  /** Written numbers, money and dates in Clean mode. Default true, like the desktop. */
  writtenForm?: boolean;
  /** Separately decoded recognizer pieces of `raw`, in order; a casing package reads their starts. */
  pieces?: readonly string[];
  signal?: AbortSignal;
}

export interface PostprocessResult {
  text: string;
  /** Which stages ran, for diagnostics: e.g. ["rules", "punctuation", "written-form"]. */
  stages: string[];
}
