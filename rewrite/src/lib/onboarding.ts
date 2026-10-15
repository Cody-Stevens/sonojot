import type { SetupProfile } from "./setup-profiles";
import type { ProfileSelection } from "./setup-profiles";
import type { AiSettings } from "./ai";
import type { Snapshot } from "./protocol";
import {
  checking,
  installed,
  installing,
  type ModelEntry,
} from "./model-library";
import { PUNCTUATION_MODEL } from "./punctuation";

const allSteps = [
  { id: "welcome", title: "Welcome to SonoJot", dwell: 3000 },
  { id: "terms", title: "Terms of Use", dwell: 0 },
  // macOS only: microphone, paste and system audio in one place, before the
  // microphone check so it just works. Continue never waits for them.
  { id: "permissions", title: "Permissions", dwell: 2000 },
  { id: "microphone", title: "Your microphone", dwell: 4000 },
  { id: "shortcut", title: "Your shortcut", dwell: 4000 },
  { id: "dictation", title: "Dictation", dwell: 3000 },
  // Records the profile choice; the Downloads step confirms it.
  { id: "models", title: "Models", dwell: 4000 },
  // Everything setup can download, with one consent review.
  { id: "downloads", title: "Downloads", dwell: 2000 },
  { id: "meetings", title: "Meetings", dwell: 4000 },
  { id: "where", title: "Where things are", dwell: 4000 },
  { id: "ready", title: "Ready", dwell: 0 },
] as const;

export type OnboardingStep = (typeof allSteps)[number];
export type OnboardingStepId = OnboardingStep["id"];

/** The tour for this platform. Only macOS asks for permissions up front. */
export function onboardingSteps(platform: string): OnboardingStep[] {
  return allSteps.filter(
    (step) => step.id !== "permissions" || platform === "macos",
  );
}

/** The profile's own downloads (punctuation is queued for every profile). */
export function onboardingDownloadIds(selection: ProfileSelection): string[] {
  return [
    ...new Set([selection.asrModel, PUNCTUATION_MODEL, selection.notesModel]),
  ].filter((id) => id !== "openrouter" && id !== "local-rules");
}

/** Step progress never depends on dwell time or the direction of navigation. */
export function stepProgress(step: number, count: number): number {
  return (step + 1) / count;
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

export function dwellProgress(step: OnboardingStep, elapsed: number): number {
  if (step.dwell === 0) return 1;
  return Math.max(0, Math.min(1, elapsed / step.dwell));
}

export function canContinue(
  step: OnboardingStep,
  elapsed: number,
  microphoneConfirmed: boolean,
  termsAgreed: boolean = false,
): boolean {
  return (
    dwellProgress(step, elapsed) === 1 &&
    (step.id !== "microphone" || microphoneConfirmed) &&
    (step.id !== "terms" || termsAgreed)
  );
}

/** A setup download's state in the Downloads step's words. */
export function setupDownloadStatus(model: ModelEntry): string {
  if (installed(model)) return "Installed";
  if (checking(model)) return "Checking…";
  switch (model.state) {
    case "queued":
      return "Queued";
    case "downloading":
      return model.totalBytes > 0
        ? `Downloading · ${Math.min(100, Math.floor((model.downloadedBytes / model.totalBytes) * 100))}%`
        : "Downloading…";
    case "verifying":
      return "Verifying…";
    case "installing":
      return "Installing…";
    case "cancelling":
    case "canceling":
      return "Cancelling…";
    case "failed":
      return "Didn't finish";
    default:
      return "Not installed";
  }
}

/** Installed, downloading or queued: nothing left to choose. */
export function setupDownloadSettled(model: ModelEntry): boolean {
  return installed(model) || installing(model) || checking(model);
}

/** One row of the Downloads step. */
export interface SetupDownload {
  /** Catalog model id, or "ffmpeg". */
  id: string;
  /** "profile" models are queued by confirming the profile. */
  kind: "profile" | "extra" | "media";
  /** Installed, or already downloading or queued: no checkbox. */
  settled: boolean;
}

/**
 * What one "Download" press does: the consent ids for the single review, the
 * profile models to skip when confirming the profile, and which profile
 * models and extras to queue. Unchecked rows are left for Models or Meetings.
 */
export function downloadPlan(
  rows: SetupDownload[],
  unchecked: string[],
): {
  consent: string[];
  skip: string[];
  profile: string[];
  extras: string[];
  media: boolean;
} {
  const pending = rows.filter((row) => !row.settled);
  const chosen = pending.filter((row) => !unchecked.includes(row.id));
  const ids = (kind: SetupDownload["kind"]) =>
    chosen.filter((row) => row.kind === kind).map((row) => row.id);
  return {
    consent: chosen.map((row) => row.id),
    skip: pending
      .filter((row) => row.kind === "profile" && unchecked.includes(row.id))
      .map((row) => row.id),
    profile: ids("profile"),
    extras: ids("extra"),
    media: ids("media").length > 0,
  };
}

/** Continue without downloading: every pending profile model is skipped. */
export function skipAllPlan(rows: SetupDownload[]): string[] {
  return rows
    .filter((row) => row.kind === "profile" && !row.settled)
    .map((row) => row.id);
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
