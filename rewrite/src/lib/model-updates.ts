import { z } from "zod";
import { formatBytes } from "./model-library";

const bytes = z.number().finite().nonnegative();
export const modelUpdateSchema = z.object({
  /** The model id after the update; the same id for speech models. */
  id: z.string(),
  /** Installed rows this update replaces. */
  replaces: z.array(z.string()),
  name: z.string(),
  note: z.string(),
  downloadBytes: bytes,
  /** Small built-in models update without asking. */
  automatic: z.boolean(),
  state: z.enum([
    "available",
    "queued",
    "downloading",
    "waiting",
    "installing",
    "failed",
  ]),
  downloadedBytes: bytes,
  error: z.string().nullable(),
});
export const modelUpdatesSchema = z.array(modelUpdateSchema);
export type ModelUpdate = z.infer<typeof modelUpdateSchema>;

export function updateRunning(update: ModelUpdate): boolean {
  return ["queued", "downloading", "waiting", "installing"].includes(
    update.state,
  );
}

/**
 * Updates worth interrupting the user for: speech models that ask first, and
 * any update that failed (automatic ones stay silent while they work).
 */
export function noticeUpdates(updates: ModelUpdate[]): ModelUpdate[] {
  return updates.filter(
    (update) => !update.automatic || update.state === "failed",
  );
}

/** The update shown on a Models page row, matched by the ids it replaces. */
export function updateFor(
  updates: ModelUpdate[],
  modelId: string,
): ModelUpdate | undefined {
  return updates.find(
    (update) => update.id === modelId || update.replaces.includes(modelId),
  );
}

export function updatePercent(update: ModelUpdate): number {
  if (!update.downloadBytes) return 0;
  return Math.min(
    99,
    Math.floor((update.downloadedBytes / update.downloadBytes) * 100),
  );
}

/** One short status line for an update, in the app's plain words. */
export function updateStatus(update: ModelUpdate): string {
  switch (update.state) {
    case "queued":
      return `Updating ${update.name}…`;
    case "downloading":
      return `Updating ${update.name} · ${updatePercent(update)}%`;
    case "waiting":
      return `${update.name} update downloaded. It finishes when dictation and meetings are idle.`;
    case "installing":
      return `Finishing the ${update.name} update…`;
    case "failed":
      return `${update.name} update: ${update.error ?? "it did not finish. SonoJot kept the current version."}`;
    default:
      return `${update.name} has an update. ${update.note}`.trim();
  }
}

/** Download size on the button that starts it: "Update · 653 MB". */
export function updateLabel(update: ModelUpdate): string {
  return update.downloadBytes
    ? `Update · ${formatBytes(update.downloadBytes)}`
    : "Update";
}

export function updateAllLabel(updates: ModelUpdate[]): string {
  const total = updates.reduce((sum, update) => sum + update.downloadBytes, 0);
  return total ? `Update all · ${formatBytes(total)}` : "Update all";
}
