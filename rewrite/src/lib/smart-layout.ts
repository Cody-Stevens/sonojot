import { installed, type ModelEntry } from "./model-library";
import type { Settings } from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";

export const SMART_LAYOUT_LABEL = "Shape by type (experimental)";
export const SMART_LAYOUT_HELP =
  "Detects emails, lists, notes and steps, then lays them out. Uses your text model; your words stay the same.";

type LayoutSettings = Pick<
  Settings,
  "cleanupMode" | "cleanupModel" | "notesModel"
>;

/** Match the backend's selected Cleanup, then selected Notes order. */
export function smartLayoutModel(
  settings: LayoutSettings,
  models: readonly ModelEntry[],
): ModelEntry | undefined {
  for (const id of [settings.cleanupModel, settings.notesModel]) {
    const model = models.find((entry) => entry.id === id);
    // The local catalog's Notes role denotes general text generation.
    // SpeakoFlow is specialized cleanup and must never receive layout prompts.
    if (
      model &&
      model.id !== SMART_CLEANUP_MODEL &&
      model.kind === "cleanup" &&
      model.roles.includes("notes") &&
      installed(model)
    )
      return model;
  }
  return undefined;
}

export function smartLayoutUnavailableReason(
  settings: LayoutSettings,
  models: readonly ModelEntry[],
): string | null {
  if (settings.cleanupMode === "original")
    return "Choose Format only or Clean to use Shape by type.";
  if (!smartLayoutModel(settings, models))
    return "Select an installed local text model for Cleanup or Notes in Models. Smart cleanup alone cannot provide it.";
  return null;
}
