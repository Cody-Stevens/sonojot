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

/** Only unfinished automatic setup includes this download without a separate action. */
export function profileIncludesPunctuation(
  saved: SetupProfile | null,
  chosen: SetupProfile,
): boolean {
  return saved === null && chosen !== "custom";
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
