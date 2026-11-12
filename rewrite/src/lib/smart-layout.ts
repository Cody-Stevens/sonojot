import { cleanupChoice } from "./cleanup-choice";
import { installed, type ModelEntry } from "./model-library";
import type { Settings } from "./protocol";
import { SMART_CLEANUP_MODEL } from "./smart-cleanup";

export const SMART_LAYOUT_LABEL = "Shape by type (experimental)";
export const SMART_LAYOUT_HELP =
  "Lays out emails, lists, notes and steps with your cleanup model.";
export const SMART_LAYOUT_APP_HINT_LABEL = "Use the app I'm in (experimental)";
export const SMART_LAYOUT_APP_HINT_HELP =
  "Use the app's type as a hint when your words support it.";

type LayoutSettings = Pick<
  Settings,
  | "cleanupMode"
  | "cleanupModel"
  | "proofreader"
  | "localProofreader"
  | "proofreaderModel"
>;

/**
 * A local general text model. SpeakoFlow is specialized cleanup and must
 * never receive layout prompts. Qualifying models still need to be installed.
 */
export function qualifiesForSmartLayout(model: ModelEntry): boolean {
  return (
    model.id !== SMART_CLEANUP_MODEL &&
    model.kind === "cleanup" &&
    model.roles.includes("notes")
  );
}

/**
 * Shape by type runs on the cleanup model, never on Notes. The SonoJot
 * model and local general text models can do it. Online models cannot yet:
 * the layout check needs the local server's tokenizer and the model's own
 * confidence for each layout type, which OpenRouter models do not report.
 */
export function smartLayoutUnavailableReason(
  settings: LayoutSettings,
  models: readonly ModelEntry[],
  sonojotAvailable: boolean,
): string | null {
  if (settings.cleanupMode === "original")
    return "Choose Format only or Clean to use Shape by type.";
  const choice = cleanupChoice(settings, sonojotAvailable);
  switch (choice.kind) {
    case "sonojot":
      return null;
    case "off":
      return "Needs a cleanup model on this computer.";
    case "online":
      return "Online cleanup models can't shape text yet. Choose one on this computer.";
    case "local": {
      const model = models.find((entry) => entry.id === choice.model);
      if (model && !qualifiesForSmartLayout(model))
        return `${model.id === SMART_CLEANUP_MODEL ? "SpeakoFlow Mini" : model.name} can't shape text. Choose another cleanup model.`;
      if (!model || !installed(model))
        return "Install the cleanup model to use Shape by type.";
      return null;
    }
  }
}
