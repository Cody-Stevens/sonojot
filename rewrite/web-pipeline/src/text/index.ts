import type { PostprocessOptions, PostprocessResult } from "./contract.ts";
import { localRules } from "./rules.ts";
import { Document, respaceGluedApostrophes, sameWords } from "./punctuation.ts";
import { predict } from "./model.ts";
import { finishWrittenForm, projectSpanLabels } from "./written-form.ts";
import type { BioLabel } from "./written-form.ts";
import { check, trimSpace } from "./shared.ts";
import { boundsFor } from "./casing.ts";

export { loadPunctuationPackage } from "./model.ts";
export { lowerSegmentStartCapitals } from "./punctuation.ts";
export { hasCasing } from "./model.ts";
export type * from "./contract.ts";

// Rust cleanup.rs::run_with_delete_policy and punctuation.rs::finish_prediction (published v3).
export async function postprocess(raw: string, options: PostprocessOptions): Promise<PostprocessResult> {
  check(options.signal);
  if (options.mode === "original") return { text: raw, stages: [] };
  const prepared = respaceGluedApostrophes(localRules(raw, options.mode));
  const stages = ["rules"];
  if (!trimSpace(prepared)) return { text: prepared, stages };
  check(options.signal);
  const document = new Document(prepared);
  const written = options.mode === "clean" && (options.writtenForm ?? true);
  let text: string;
  let spans: BioLabel[] | undefined;
  try {
    if (!options.punctuation || !document.words.length) throw new Error("No punctuation prediction");
    const bounds = options.pieces ? boundsFor(document.words.map((w) => w.text), options.pieces) : [];
    const prediction = await predict(document, options.punctuation, options.signal, bounds);
    spans = prediction.spans;
    if (prediction.cases && prediction.caseThresholds && prediction.cases.length === document.words.length)
      document.casing = {
        probs: prediction.cases,
        bounds: document.words.map((_, i) => bounds[i] ?? false),
        thresholds: prediction.caseThresholds,
      };
    text = document.rebuild(prediction.labels, !written);
    if (!sameWords(prepared, text)) throw new Error("Punctuation changed source words");
    stages.push("punctuation");
  } catch {
    check(options.signal);
    text = document.rebuild(undefined, !written);
    if (!sameWords(prepared, text)) text = prepared;
    stages.push("punctuation-rules");
  }
  if (written) {
    const projected = spans === undefined ? undefined : (projectSpanLabels(text, spans) ?? []);
    text = finishWrittenForm(text, projected);
    stages.push("written-form");
  }
  check(options.signal);
  return { text, stages };
}
