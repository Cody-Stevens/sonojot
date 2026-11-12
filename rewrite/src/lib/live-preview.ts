import { formatBytes, installed, type ModelEntry } from "./model-library";

/** Catalog ID of the tiny streaming model behind Live preview. */
export const LIVE_PREVIEW_MODEL = "moonshine-tiny-streaming";

export const LIVE_PREVIEW_SUMMARY =
  "A quick draft of your words while you speak. It can be wrong and never reaches your text.";

/** Help for the Live preview switch: what it does, then what it still needs. */
export function livePreviewHelpText(
  model: ModelEntry | undefined,
  speechModel: string,
): string {
  const summary = LIVE_PREVIEW_SUMMARY;
  if (!model) return summary;
  if (!model.supported) return `${summary} Not available on this computer yet.`;
  if (!installed(model))
    return `${summary} Needs ${model.name} (${formatBytes(model.totalBytes || model.diskBytes)}).`;
  if (/^moonshine-(small|medium)$/.test(speechModel))
    return `${summary} Your Moonshine speech model already shows live text.`;
  return summary;
}
