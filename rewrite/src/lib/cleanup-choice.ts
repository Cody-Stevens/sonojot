import type { Settings } from "./protocol";

/**
 * The one model that cleans dictation after punctuation. It is derived from
 * the stored keys, exactly as `Settings::cleanup_choice` does in the backend:
 *
 * | localProofreader | SonoJot installed | proofreader | cleanupModel  | Choice  |
 * | ---------------- | ----------------- | ----------- | ------------- | ------- |
 * | true             | yes               | any         | any           | SonoJot |
 * | any (else)       | any               | true        | any           | Online  |
 * | any (else)       | any               | false       | a local model | Local   |
 * | any (else)       | any               | false       | "local-rules" | Off     |
 *
 * `localProofreader` defaults on, so the SonoJot model is the default as soon
 * as it is installed. Every choice made in the picker switches it off except
 * SonoJot itself, so an explicit Off stays Off.
 */
export type CleanupChoice =
  | { kind: "off" }
  | { kind: "sonojot" }
  | { kind: "local"; model: string }
  | { kind: "online"; model: string };

type ChoiceSettings = Pick<
  Settings,
  "proofreader" | "localProofreader" | "proofreaderModel" | "cleanupModel"
>;

/** The picker's id for our model; never a catalog id. */
export const SONOJOT_CLEANUP_OPTION = "sonojot-cleanup";
/**
 * Today our model is the sideloaded experiment. A downloadable SonoJot
 * cleanup model takes this slot (and drops "experimental") with no other
 * change to the picker.
 */
export const SONOJOT_CLEANUP_NAME = "SonoJot (experimental)";
/** Always offered online, ahead of the user's own list. */
export const RECOMMENDED_ONLINE_CLEANUP = [
  "anthropic/claude-opus-5.5",
] as const;
export const OTHER_ONLINE_CLEANUP = [
  "openai/gpt-6-luna",
  "anthropic/claude-sonnet-5.5",
] as const;
export const RECOMMENDED_ONLINE_NAMES: Record<string, string> = {
  "anthropic/claude-opus-5.5": "Claude Opus 5.5",
  "anthropic/claude-sonnet-5.5": "Claude Sonnet 5.5",
  "openai/gpt-6-luna": "GPT-6 Luna",
  "anthropic/claude-haiku-5.5": "Claude Haiku 5.5",
};

export function cleanupChoice(
  settings: ChoiceSettings,
  sonojotAvailable: boolean,
): CleanupChoice {
  if (settings.localProofreader && sonojotAvailable) return { kind: "sonojot" };
  if (settings.proofreader)
    return { kind: "online", model: settings.proofreaderModel };
  if (!["", "local-rules", "off"].includes(settings.cleanupModel))
    return { kind: "local", model: settings.cleanupModel };
  return { kind: "off" };
}

/** Settings that save one choice and switch every other one off. */
export function cleanupPatch(choice: CleanupChoice): Partial<ChoiceSettings> {
  return {
    localProofreader: choice.kind === "sonojot",
    proofreader: choice.kind === "online",
    cleanupModel: choice.kind === "local" ? choice.model : "local-rules",
    ...(choice.kind === "online" ? { proofreaderModel: choice.model } : {}),
  };
}

/** Short words for the one-line links, e.g. "SonoJot (experimental)". */
export function cleanupState(
  choice: CleanupChoice,
  nameOf: (id: string) => string,
): string {
  switch (choice.kind) {
    case "off":
      return "Off";
    case "sonojot":
      return SONOJOT_CLEANUP_NAME;
    case "local":
      return nameOf(choice.model);
    case "online":
      return `${nameOf(choice.model)} · OpenRouter`;
  }
}

/**
 * Online options: Opus first, then the other measured models and the shared list.
 * Include the saved model, so a choice made before the list existed stays visible.
 */
export function cleanupOnlineIds(
  quickTextIds: readonly string[],
  savedModel: string,
): string[] {
  return [
    ...new Set([
      ...RECOMMENDED_ONLINE_CLEANUP,
      ...OTHER_ONLINE_CLEANUP,
      ...quickTextIds.filter((id) => id !== "anthropic/claude-haiku-5.5"),
      ...(savedModel ? [savedModel] : []),
    ]),
  ];
}

export function onlineCleanupName(id: string): string {
  return RECOMMENDED_ONLINE_NAMES[id] ?? id;
}
