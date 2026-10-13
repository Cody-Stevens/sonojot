import type { SetupProfile } from "./setup-profiles";
import type { ProfileSelection } from "./setup-profiles";
import type { AiSettings } from "./ai";
import type { Snapshot } from "./protocol";
import { checking, installed, type ModelEntry } from "./model-library";
import { PUNCTUATION_MODEL } from "./punctuation";

export const onboardingSteps = [
  { id: "welcome", title: "Welcome to SonoJot", dwell: 3000 },
  { id: "terms", title: "Terms of Use", dwell: 0 },
  { id: "microphone", title: "Your microphone", dwell: 4000 },
  { id: "dictation", title: "Dictation", dwell: 4000 },
  { id: "models", title: "Models", dwell: 4000 },
  { id: "meetings", title: "Meetings", dwell: 4000 },
  { id: "where", title: "Where things are", dwell: 4000 },
  { id: "ready", title: "Ready", dwell: 0 },
] as const;

/** Consent also covers Cleanup and independent punctuation/runtime downloads. */
export function onboardingDownloadIds(selection: ProfileSelection): string[] {
  return [
    ...new Set([
      selection.asrModel,
      selection.cleanupModel,
      selection.notesModel,
      PUNCTUATION_MODEL,
    ]),
  ].filter((id) => id !== "openrouter" && id !== "local-rules");
}

/** Step progress never depends on dwell time or the direction of navigation. */
export function stepProgress(step: number): number {
  return (step + 1) / onboardingSteps.length;
}

export const MODEL_DETAILS_TIMEOUT = 10000;

export function onboardingModelDetails(
  selection: ProfileSelection | undefined,
  models: ModelEntry[] | null,
  custom: boolean,
  ai: AiSettings | null,
): {
  speech: string | null;
  notes: string | null;
  downloadBytes: number | null;
} {
  if (!selection) return { speech: null, notes: null, downloadBytes: null };
  const online =
    custom &&
    (ai?.backend === "openrouter" || selection.notesModel === "openrouter");
  const routeKnown = !custom || ai !== null || online;
  const speech =
    models?.find((model) => model.id === selection.asrModel)?.name ?? null;
  const notes = online
    ? ai?.model
      ? `${ai.model} (OpenRouter)`
      : "OpenRouter"
    : routeKnown
      ? (models?.find((model) => model.id === selection.notesModel)?.name ??
        null)
      : null;
  // Online Notes lives in AI settings, not the downloadable local catalog.
  const ids = new Set([
    selection.asrModel,
    selection.cleanupModel,
    PUNCTUATION_MODEL,
  ]);
  if (!online) ids.add(selection.notesModel);
  let downloadBytes: number | null = models && routeKnown ? 0 : null;
  for (const id of ids) {
    const model = models?.find((entry) => entry.id === id);
    if (!model || checking(model)) {
      downloadBytes = null;
      break;
    }
    if (
      downloadBytes !== null &&
      !installed(model) &&
      !(id === PUNCTUATION_MODEL && !model.installable)
    )
      downloadBytes += model.totalBytes || model.diskBytes;
  }
  return { speech, notes, downloadBytes };
}

/** Only published results from after entering Ready belong in the practice field. */
export function practiceResult(
  snapshot: Snapshot,
  previousSession: number | null,
): string | null {
  if (
    snapshot.resultSessionId === null ||
    snapshot.resultSessionId === previousSession ||
    snapshot.meetingRecording ||
    !["idle", "error"].includes(snapshot.state)
  )
    return null;
  return snapshot.outputText || null;
}

export function dwellProgress(step: number, elapsed: number): number {
  if (onboardingSteps[step].dwell === 0) return 1;
  return Math.max(0, Math.min(1, elapsed / onboardingSteps[step].dwell));
}

export function canContinue(
  step: number,
  elapsed: number,
  microphoneConfirmed: boolean,
  termsAgreed: boolean = false,
): boolean {
  return (
    dwellProgress(step, elapsed) === 1 &&
    (onboardingSteps[step].id !== "microphone" || microphoneConfirmed) &&
    (onboardingSteps[step].id !== "terms" || termsAgreed)
  );
}

export function shouldApplyTourProfile(
  saved: SetupProfile,
  selected: SetupProfile,
): boolean {
  return saved !== selected;
}

export function microphoneConfirmed(
  level: number,
  consecutiveSamples: number,
): number {
  // Three 200 ms samples avoid accepting a single click as speech-level input.
  return level >= 0.015 ? consecutiveSamples + 1 : 0;
}
