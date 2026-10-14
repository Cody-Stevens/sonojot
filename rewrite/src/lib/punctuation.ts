import {
  checking,
  installed,
  installing,
  type ModelEntry,
} from "./model-library";
import type { Settings } from "./protocol";
import type { SetupProfile } from "./setup-profiles";

export const PUNCTUATION_MODEL = "sonojot-punct-v2";
export const PUNCTUATION_HELP =
  "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU";
export const PUNCTUATION_UPGRADE_REASON =
  "Adds written numbers, money and dates.";

export function punctuationModelPage(id: string): string {
  return id === "sonojot-punct-v1"
    ? "https://huggingface.co/SonoJot/sonojot-punct-v1"
    : "https://huggingface.co/SonoJot/sonojot-punct-v2";
}

/** Every unfinished setup includes this download, matching first-run onboarding. */
export function profileIncludesPunctuation(
  saved: SetupProfile | null,
  _chosen: SetupProfile,
): boolean {
  return saved === null;
}

/** The verified package dictation uses: v2 when ready, otherwise v1. */
export function activePunctuation(
  models: ModelEntry[],
): ModelEntry | undefined {
  return [PUNCTUATION_MODEL, "sonojot-punct-v1"]
    .map((id) => models.find((model) => model.id === id))
    .find((model) => !!model && installed(model));
}

/**
 * Default Cleanup ("local-rules") applies the rules and vocabulary, then the
 * punctuation model. Name the model when it is ready or can be installed.
 */
export function defaultCleanupName(models: ModelEntry[]): string {
  const model =
    activePunctuation(models) ??
    models.find((entry) => entry.id === PUNCTUATION_MODEL && entry.installable);
  return model ? `${model.name} with your vocabulary` : "Local rules";
}

export function punctuationRecommendation(
  settings: Pick<
    Settings,
    "setupProfile" | "punctuationV2RecommendationAcknowledged"
  >,
  models: ModelEntry[],
): ModelEntry | undefined {
  if (
    settings.setupProfile === null ||
    settings.punctuationV2RecommendationAcknowledged
  )
    return;
  return models.find(
    (model) =>
      model.id === PUNCTUATION_MODEL &&
      model.installable &&
      !checking(model) &&
      !installed(model) &&
      !installing(model),
  );
}
