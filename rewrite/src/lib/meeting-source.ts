import type { Settings } from "./protocol";
import copy from "./meeting-source-copy.json";

export type MeetingSource = Settings["meetingSource"];

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

/** The native compact controls read the same bundled copy as the Meetings dialog. */
export function meetingStartPrompt(settings: MeetingStartSettings) {
  const warning = meetingSourceWarning(settings.meetingSource);
  const remind = !settings.meetingConsentAcknowledged;
  if (!warning && !remind) return null;
  return {
    title: remind ? copy.title : "Record only one side?",
    detail: [remind ? copy.reminder : null, warning]
      .filter(Boolean)
      .join("\n\n"),
    label: warning
      ? remind
        ? copy.gotItRecordBoth
        : "Record both sides"
      : copy.gotIt,
    alternativeLabel: warning
      ? remind
        ? copy.gotItStartAnyway
        : "Start anyway"
      : undefined,
    cancelLabel: copy.cancel,
  };
}

/** No save on dismissal; a failed save must never fall through to capture. */
export async function prepareMeetingStart(
  settings: MeetingStartSettings,
  confirm: (
    prompt: NonNullable<ReturnType<typeof meetingStartPrompt>>,
  ) => Promise<MeetingStartChoice>,
  save: (patch: Partial<MeetingStartSettings>) => Promise<void>,
  isCurrent: () => boolean = () => true,
): Promise<MeetingSource | null> {
  if (!isCurrent()) return null;
  const prompt = meetingStartPrompt(settings);
  if (!prompt) return settings.meetingSource;
  const choice = await confirm(prompt);
  if (
    choice === false ||
    (choice === "alternative" && !prompt.alternativeLabel)
  )
    return null;
  if (!isCurrent()) return null;
  const source = choice === true ? "both" : settings.meetingSource;
  const patch: Partial<MeetingStartSettings> = {};
  if (!settings.meetingConsentAcknowledged)
    patch.meetingConsentAcknowledged = true;
  if (source !== settings.meetingSource) patch.meetingSource = source;
  if (Object.keys(patch).length) await save(patch);
  return isCurrent() ? source : null;
}
