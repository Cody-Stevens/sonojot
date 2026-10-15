import { formatBytes, installed, type ModelEntry } from "./model-library";

/** Catalog ID of the tiny streaming model behind Live preview. */
export const LIVE_PREVIEW_MODEL = "moonshine-tiny-streaming";

const SUMMARY =
  "A quick draft of your words while you speak. It can be wrong and never reaches your text.";

/** Help for the Live preview switch: what it does, then what it still needs. */
export function livePreviewHelpText(
  model: ModelEntry | undefined,
  speechModel: string,
): string {
  if (!model) return SUMMARY;
  if (!model.supported) return `${SUMMARY} Not available on this computer yet.`;
  if (!installed(model))
    return `${SUMMARY} Install ${model.name} (${formatBytes(model.totalBytes || model.diskBytes)}) in Models to use it.`;
  if (/^moonshine-(small|medium)$/.test(speechModel))
    return `${SUMMARY} Your Moonshine speech model already shows live text.`;
  return SUMMARY;
}
