import {
  checking,
  installed,
  installing,
  type ModelEntry,
} from "./model-library";
import type { Settings } from "./protocol";
import type { SetupProfile } from "./setup-profiles";

/** The newest package: first-run Setup installs it and existing users are offered it. */
export const PUNCTUATION_MODEL = "sonojot-punct-v3";
/** Verified packages in the order the app prefers them. */
export const PUNCTUATION_PACKAGES = [
  "sonojot-cleanup-v4",
  PUNCTUATION_MODEL,
  "sonojot-punct-v2",
  "sonojot-punct-v1",
] as const;
export const PUNCTUATION_HELP =
  "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU";
/** What the "SonoJot punctuation and formatting" switch does, in plain words. */
export const PUNCTUATION_SWITCH_HELP =
  "Adds punctuation, sentence breaks and capitals. In Clean, also writes numbers and symbols and removes fillers, repeats and restarts.";
export const PUNCTUATION_UPGRADE_REASON =
  "More accurate sentence breaks and commas, with written numbers, money and dates.";

/** "v4" for sonojot-cleanup-v4, "v3" for sonojot-punct-v3. */
export function punctuationVersion(id: string): string {
  return /-(v\d+)$/.exec(id)?.[1] ?? id;
}

export function punctuationModelPage(id: string): string {
  const known = PUNCTUATION_PACKAGES.find((item) => item === id);
  return `https://huggingface.co/SonoJot/${known ?? PUNCTUATION_MODEL}`;
}

/** Every unfinished setup includes this download, matching first-run onboarding. */
export function profileIncludesPunctuation(
  saved: SetupProfile | null,
  _chosen: SetupProfile,
): boolean {
  return saved === null;
}

/** The verified package dictation uses: the newest one that is ready. */
export function activePunctuation(
  models: ModelEntry[],
): ModelEntry | undefined {
  return PUNCTUATION_PACKAGES.map((id) =>
    models.find((model) => model.id === id),
  ).find((model) => !!model && installed(model));
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

/**
 * Install state of the punctuation package behind default Cleanup, in the
 * words the profile row, first-run setup and the Cleanup picker share. Empty
 * when no package can be installed ("Local rules" needs no state).
 */
export function defaultCleanupStatus(
  models: ModelEntry[],
  downloadIncluded = false,
): string {
  const punctuation = models.find((model) => model.id === PUNCTUATION_MODEL);
  if (punctuation && checking(punctuation)) return "Checking…";
  if (activePunctuation(models)) return "Installed";
  if (!punctuation?.installable) return "";
  return downloadIncluded ? "Download included" : "Not installed";
}

/** Picker and library name: default Cleanup reads as the punctuation model. */
export function modelDisplayName(
  model: ModelEntry,
  models: ModelEntry[],
): string {
  return model.id === "local-rules" ? defaultCleanupName(models) : model.name;
}

export function punctuationRecommendation(
  settings: Pick<
    Settings,
    "setupProfile" | "punctuationV3RecommendationAcknowledged"
  >,
  models: ModelEntry[],
): ModelEntry | undefined {
  if (
    settings.setupProfile === null ||
    settings.punctuationV3RecommendationAcknowledged
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
