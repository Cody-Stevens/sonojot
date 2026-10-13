import type { Snapshot } from "./protocol";

/** Only saved meeting or capture transitions invalidate the meeting library. */
export function meetingDataChanged(
  previous: Snapshot,
  next: Snapshot,
): boolean {
  return (
    previous.activeMeetingId !== next.activeMeetingId ||
    previous.meetingRecording !== next.meetingRecording ||
    previous.meetingsRevision !== next.meetingsRevision
  );
}
