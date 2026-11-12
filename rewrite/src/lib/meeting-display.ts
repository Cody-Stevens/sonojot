import type { MeetingSummary } from "./protocol";

export const meetingStatus: Record<string, string> = {
  recorded: "Recorded",
  imported: "Imported",
  recording: "Recording",
  finalizing: "Saving recording",
  transcribing: "Transcribing",
  processing: "Processing",
  cleaning: "Cleaning transcript",
  processingNotes: "Writing notes",
  generatingTitle: "Naming meeting",
  importing: "Importing",
  ready: "Ready",
  transcribed: "Transcribed",
  processed: "Notes ready",
  complete: "Ready",
  completed: "Ready",
  error: "Needs attention",
  failed: "Needs attention",
  interrupted: "Transcription stopped",
  cancelled: "Stopped",
  canceled: "Stopped",
  generating_notes: "Writing notes",
  "generating-notes": "Writing notes",
};
export const meetingBusy = (status: string) =>
  [
    "recording",
    "finalizing",
    "transcribing",
    "processing",
    "processingNotes",
    "generatingTitle",
    "importing",
    "generating_notes",
    "generating-notes",
  ].includes(status);

export function meetingDays(rows: MeetingSummary[], now = new Date()) {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const groups = new Map<string, { label: string; rows: MeetingSummary[] }>();
  for (const row of rows) {
    const date = new Date(row.createdAt);
    const valid = Number.isFinite(date.getTime());
    const key = valid
      ? `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
      : "unknown";
    const day = valid
      ? new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
      : NaN;
    const label =
      day === today.getTime()
        ? "Today"
        : day === yesterday.getTime()
          ? "Yesterday"
          : valid
            ? date.toLocaleDateString(undefined, {
                weekday: "short",
                month: "short",
                day: "numeric",
                year:
                  date.getFullYear() === today.getFullYear()
                    ? undefined
                    : "numeric",
              })
            : "Earlier";
    if (!groups.has(key)) groups.set(key, { label, rows: [] });
    groups.get(key)!.rows.push(row);
  }
  return [...groups.values()];
}

export type InkSample = {
  at: number;
  microphone: number | null;
  system: number | null;
};
/** A break is missing input, not a zero-amplitude sample. */
export function inkTrace(
  samples: InkSample[],
  source: "microphone" | "system",
  width = 600,
  height = 34,
): string {
  let path = "",
    connected = false;
  samples.forEach((sample, index) => {
    const level = sample[source];
    if (level === null) {
      connected = false;
      return;
    }
    const x = width - ((samples.length - 1 - index) * width) / 119;
    const amplitude = Math.max(0, Math.min(1, level)) * (height / 2 - 2);
    path += `${connected ? " L" : " M"}${x.toFixed(1)},${(height / 2 - amplitude).toFixed(1)} L${x.toFixed(1)},${(height / 2 + amplitude).toFixed(1)}`;
    connected = true;
  });
  return path.trim();
}
