import { formatBytes, installed, type ModelEntry } from "./model-library";

/** Optional preview model used beside speech models other than SonoJot Parakeet. */
export const LIVE_PREVIEW_MODEL = "moonshine-tiny-streaming";

/** SonoJot Parakeet drafts its own preview, so it needs no helper model. */
export const PARAKEET_PREVIEW_MODEL = "parakeet-tdt-0.6b-v2-sonojot-ft2";

/** True when the dictation model shows live text without the helper. */
export function previewBuiltIn(speechModel: string): boolean {
  return speechModel === PARAKEET_PREVIEW_MODEL;
}

export const LIVE_PREVIEW_SUMMARY =
  "A quick draft of your words while you speak. It can be wrong and never reaches your text.";

/** Help for the Live preview switch: what it does, then what it still needs. */
export function livePreviewHelpText(
  model: ModelEntry | undefined,
  speechModel: string,
): string {
  const summary = LIVE_PREVIEW_SUMMARY;
  if (previewBuiltIn(speechModel))
    return `${summary} Uses SonoJot Parakeet, with no extra model to download.`;
  if (!model) return summary;
  if (!model.supported) return `${summary} Not available on this computer yet.`;
  if (!installed(model))
    return `${summary} Needs ${model.name} (${formatBytes(model.totalBytes || model.diskBytes)}).`;
  if (/^moonshine-(small|medium)$/.test(speechModel))
    return `${summary} Your Moonshine speech model already shows live text.`;
  return summary;
}
