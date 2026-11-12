import assert from "node:assert/strict";
import test from "node:test";
import { inkTrace, meetingDays } from "./meeting-display";
import type { MeetingSummary } from "./protocol";

test("source loss breaks the trace while silence remains connected", () => {
  const samples = [0.3, 0, null, 0.5].map((microphone, at) => ({
    at,
    microphone,
    system: null,
  }));
  const path = inkTrace(samples, "microphone");
  assert.equal((path.match(/M/g) ?? []).length, 2);
  assert.equal((path.match(/L/g) ?? []).length, 4);
  assert.equal(inkTrace(samples, "system"), "");
  assert.equal(inkTrace([], "microphone"), "");
});

test("meeting days use local calendar boundaries and retain invalid dates", () => {
  const dates = [
    new Date(2026, 10, 6, 0, 5).toISOString(),
    new Date(2026, 10, 5, 23, 55).toISOString(),
    "unknown",
  ];
  const rows: MeetingSummary[] = dates.map((createdAt, i) => ({
    id: String(i),
    title: "Meeting",
    createdAt,
    durationSeconds: 10,
    status: "ready",
    hasTranscript: true,
    hasAudio: true,
  }));
  const groups = meetingDays(rows, new Date(2026, 10, 6, 12));
  assert.deepEqual(
    groups.map((group) => group.label),
    ["Today", "Yesterday", "Earlier"],
  );
  assert.deepEqual(
    groups.flatMap((group) => group.rows),
    rows,
  );
});
