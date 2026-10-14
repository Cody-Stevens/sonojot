import type { Settings, SystemAudioStatus } from "./protocol";
import copy from "./meeting-source-copy.json";

export type MeetingSource = Settings["meetingSource"];

export function systemAudioFixable(status: SystemAudioStatus): boolean {
  return status === "notDetermined" || status === "denied";
}

/** Shared by meeting entry points so a one-sided recording has the same warning. */
export function meetingSourceWarning(source: MeetingSource): string | null {
  switch (source) {
    case "microphone":
      return copy.microphone;
    case "system":
      return copy.system;
    case "both":
      return null;
  }
}

type MeetingStartSettings = Pick<
  Settings,
  "meetingSource" | "meetingConsentAcknowledged"
>;
export type MeetingStartChoice = boolean | "alternative";

/**
 * What a dialog button does. `start` keeps the saved source; `both`,
 * `microphone` and `system` save that source first; `fix` opens the system
 * audio fix and starts nothing.
 */
export type MeetingStartAction =
  | "start"
  | "both"
  | "microphone"
  | "system"
  | "fix";

export interface MeetingStartPrompt {
  title: string;
  detail: string;
  label: string;
  primary: MeetingStartAction;
  alternativeLabel?: string;
  alternative?: MeetingStartAction;
  cancelLabel: string;
}

/**
 * The start dialog for a meeting. Any start that would miss the other people
 * on a call needs an explicit choice. A fixable permission keeps the system
 * side in the recording, so it joins as soon as it is allowed.
 */
export function meetingStartPrompt(
  settings: MeetingStartSettings,
  status: SystemAudioStatus = "ready",
): MeetingStartPrompt | null {
  const remind = !settings.meetingConsentAcknowledged;
  const fixable = systemAudioFixable(status);
  const unsupported = status === "unsupported";
  let prompt: Omit<MeetingStartPrompt, "cancelLabel"> | null = null;
  switch (settings.meetingSource) {
    case "both":
      if (fixable)
        prompt = {
          title: copy.systemOffTitle,
          detail: `${copy.microphone}\n\n${copy.systemPermission}`,
          label: copy.fix,
          primary: "fix",
          alternativeLabel: copy.recordMicrophone,
          alternative: "start",
        };
      else if (unsupported)
        prompt = {
          title: copy.microphoneTitle,
          detail: `${copy.microphone}\n\n${copy.systemUnsupported}`,
          label: copy.recordMicrophone,
          primary: "microphone",
        };
      break;
    case "microphone":
      prompt = {
        title: copy.microphoneTitle,
        detail: unsupported
          ? `${copy.microphone}\n\n${copy.systemUnsupported}`
          : copy.microphone,
        ...(unsupported
          ? { label: copy.recordMicrophone, primary: "start" as const }
          : {
              label: fixable ? copy.fix : copy.recordBoth,
              primary: fixable ? ("fix" as const) : ("both" as const),
              alternativeLabel: copy.recordMicrophone,
              alternative: "start" as const,
            }),
      };
      break;
    case "system":
      prompt = fixable
        ? {
            title: copy.systemOffTitle,
            detail: copy.systemOnlyBlocked,
            label: copy.fix,
            primary: "fix",
            // The microphone records now; the system side joins once allowed.
            alternativeLabel: copy.recordMicrophone,
            alternative: "both",
          }
        : unsupported
          ? {
              title: copy.microphoneTitle,
              detail: `${copy.systemUnsupported}\n\n${copy.microphone}`,
              label: copy.recordMicrophone,
              primary: "microphone",
            }
          : {
              title: copy.systemTitle,
              detail: copy.system,
              label: copy.recordBoth,
              primary: "both",
              alternativeLabel: copy.recordSystem,
              alternative: "start",
            };
      break;
  }
  if (!prompt && !remind) return null;
  prompt ??= {
    title: copy.title,
    detail: copy.reminder,
    label: copy.gotIt,
    primary: "start",
  };
  if (remind && prompt.detail !== copy.reminder)
    prompt = {
      ...prompt,
      title: copy.title,
      detail: `${copy.reminder}\n\n${prompt.detail}`,
    };
  return { ...prompt, cancelLabel: copy.cancel };
}

export type MeetingStartPlan =
  | { action: "start"; source: MeetingSource }
  | { action: "fix" };

/** No save on dismissal; a failed save must never fall through to capture. */
export async function prepareMeetingStart(
  settings: MeetingStartSettings,
  status: SystemAudioStatus,
  confirm: (prompt: MeetingStartPrompt) => Promise<MeetingStartChoice>,
  save: (patch: Partial<MeetingStartSettings>) => Promise<void>,
  isCurrent: () => boolean = () => true,
): Promise<MeetingStartPlan | null> {
  if (!isCurrent()) return null;
  const prompt = meetingStartPrompt(settings, status);
  if (!prompt) return { action: "start", source: settings.meetingSource };
  const choice = await confirm(prompt);
  const action =
    choice === true
      ? prompt.primary
      : choice === "alternative"
        ? prompt.alternative
        : undefined;
  if (!action) return null;
  if (action === "fix") return { action: "fix" };
  if (!isCurrent()) return null;
  const source = action === "start" ? settings.meetingSource : action;
  const patch: Partial<MeetingStartSettings> = {};
  if (!settings.meetingConsentAcknowledged)
    patch.meetingConsentAcknowledged = true;
  if (source !== settings.meetingSource) patch.meetingSource = source;
  if (Object.keys(patch).length) await save(patch);
  return isCurrent() ? { action: "start", source } : null;
}

export interface TabMeetingWarning {
  text: string;
  /** The underlined link that starts now. */
  start: { label: string; source: MeetingSource };
  /** The filled button: record both sides, or fix system audio first. */
  primary: { label: string; action: "both" | "fix" } | null;
}

/**
 * The screen tab's compact version of the start dialog, for a deliberate
 * one-sided source. "both" never waits here: a quick start records the
 * microphone at once and the tab then shows what is missing and its fix.
 */
export function tabMeetingWarning(
  source: MeetingSource,
  status: SystemAudioStatus,
): TabMeetingWarning | null {
  const fixable = systemAudioFixable(status);
  const unsupported = status === "unsupported";
  switch (source) {
    case "both":
      return null;
    case "microphone":
      return {
        text: copy.microphone,
        start: { label: "Start anyway", source: "microphone" },
        primary: unsupported
          ? null
          : fixable
            ? { label: copy.fix, action: "fix" }
            : { label: copy.recordBoth, action: "both" },
      };
    case "system":
      if (fixable)
        return {
          text: copy.systemOnlyBlocked,
          start: { label: copy.recordMicrophone, source: "both" },
          primary: { label: copy.fix, action: "fix" },
        };
      if (unsupported)
        return {
          text: copy.systemUnsupported,
          start: { label: copy.recordMicrophone, source: "microphone" },
          primary: null,
        };
      return {
        text: copy.system,
        start: { label: "Start anyway", source: "system" },
        primary: { label: copy.recordBoth, action: "both" },
      };
  }
}

/** The live notice while the other side of a recording is missing. */
export function recordingSystemNotice(
  problem: "permission" | "lost" | "unsupported" | null,
): { text: string; fix: string | null } | null {
  switch (problem) {
    case "permission":
      return { text: copy.recordingPermission, fix: copy.allow };
    case "unsupported":
      return { text: copy.recordingUnsupported, fix: null };
    default:
      return null;
  }
}

/** Status shown wherever system audio permission is explained. */
export function systemAudioState(
  status: SystemAudioStatus,
  platform: string,
): { label: string; detail: string; action: string | null } {
  const mac = platform === "macos";
  switch (status) {
    case "ready":
      return {
        label: "Ready",
        detail: mac
          ? "Meetings record the other people on your calls."
          : "Windows needs no permission. Meetings record the other people on your calls.",
        action: null,
      };
    case "notDetermined":
      return {
        label: "Not allowed yet",
        detail: "SonoJot records it only while a meeting records.",
        action: copy.allow,
      };
    case "denied":
      return {
        label: "Not allowed",
        detail: "It's turned off in System Settings.",
        action: "Open System Settings",
      };
    case "unsupported":
      return {
        label: "Unavailable",
        detail: mac
          ? `${copy.systemUnsupported} Meetings record your microphone only.`
          : "This computer can't record system audio. Meetings record your microphone only.",
        action: null,
      };
  }
}
