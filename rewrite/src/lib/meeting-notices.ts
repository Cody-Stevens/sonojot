import type { MeetingDetail } from "./protocol";
import type { TranscriptBlock } from "./meeting-transcript";

export type MeetingOutage = MeetingDetail["audioOutages"][number];
export type TranscriptEntry =
  | { kind: "passage"; block: TranscriptBlock }
  | { kind: "outage"; outage: MeetingOutage };

export function meetingClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  return minutes >= 60
    ? `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`
    : `${minutes}:${String(total % 60).padStart(2, "0")}`;
}

export function clockSeconds(clock: string | null): number | null {
  if (!clock || !/^\d{1,3}:\d{2}(?::\d{2})?$/.test(clock)) return null;
  return clock.split(":").reduce((total, part) => total * 60 + Number(part), 0);
}

export function outageLabel(outage: MeetingOutage): string {
  const kind =
    outage.source === "recognition" ? "Transcription missing" : "Audio missing";
  const when =
    outage.startSeconds === outage.endSeconds
      ? `around ${meetingClock(outage.startSeconds)}`
      : `${meetingClock(outage.startSeconds)}–${meetingClock(outage.endSeconds)}`;
  return `${kind} ${when} · ${outage.reason}`;
}

// Marker rows are derived from saved metadata, never part of editable text.
export function transcriptEntries(
  blocks: TranscriptBlock[],
  outages: MeetingOutage[],
): TranscriptEntry[] {
  const pending = [...outages].sort((a, b) => a.startSeconds - b.startSeconds);
  const entries: TranscriptEntry[] = [];
  let cursor = 0;
  for (const block of blocks) {
    const start = clockSeconds(block.timecode);
    while (
      cursor < pending.length &&
      start !== null &&
      pending[cursor].startSeconds <= start
    ) {
      entries.push({ kind: "outage", outage: pending[cursor++] });
    }
    entries.push({ kind: "passage", block });
  }
  for (; cursor < pending.length; cursor++)
    entries.push({ kind: "outage", outage: pending[cursor] });
  return entries;
}

export function meetingNotice(
  meeting: Pick<
    MeetingDetail,
    "audioOutages" | "missingSeconds" | "durationSeconds"
  >,
): string {
  const audio = meeting.audioOutages.filter(
    (outage) => outage.source !== "recognition",
  );
  const omitted = meeting.audioOutages.filter(
    (outage) => outage.source === "recognition",
  );
  const significantTotal =
    meeting.durationSeconds > 0 &&
    meeting.missingSeconds > meeting.durationSeconds * 0.01;
  const parts: string[] = [];
  if (audio.length || significantTotal) {
    const seconds = Math.max(1, Math.round(meeting.missingSeconds));
    const where =
      audio.length === 1
        ? ` around ${meetingClock(audio[0].startSeconds)}`
        : "";
    const amount =
      meeting.missingSeconds > 0
        ? `${seconds} ${seconds === 1 ? "second of audio is" : "seconds of audio are"} missing${where}.`
        : audio.length === 1
          ? `${audio[0].reason}${where}.`
          : "Some audio is missing.";
    const reason =
      audio.length === 1 && meeting.missingSeconds > 0
        ? ` ${audio[0].reason}.`
        : "";
    parts.push(
      `${amount}${reason}${audio.length === 1 ? " The gap is marked in the transcript." : audio.length ? " The gaps are marked in the transcript." : ""}`,
    );
  }
  if (omitted.length)
    parts.push(
      omitted.some((gap) => gap.reason.includes("previous transcript kept"))
        ? `The latest transcription missed speech around ${meetingClock(omitted[0].startSeconds)}. The previous transcript and any cleaned copy were kept.`
        : `Some speech could not be transcribed around ${meetingClock(omitted[0].startSeconds)}. The missing sections are marked below.`,
    );
  return parts.join(" ");
}

export function visibleRecognitionWarnings(warnings: string[]): string[] {
  return warnings.filter(
    (warning) =>
      !/Repeated text was reduced to its first occurrence|repeating loop.*section was omitted|repetition.*(?:omitted|dropped)|token limit/i.test(
        warning,
      ),
  );
}
