import assert from "node:assert/strict";
import test from "node:test";
import { meetingDataChanged } from "./meeting-update";
import { previewSnapshot } from "./protocol";

test("meter and dictation pushes do not refresh meeting files", () => {
  assert.equal(
    meetingDataChanged(previewSnapshot, {
      ...previewSnapshot,
      revision: 120,
      audioLevel: 0.7,
      meetingSourceStatus: ["Microphone lost — reconnecting"],
      interimText: "New speech",
    }),
    false,
  );
});

test("saved meeting changes and recording transitions do refresh", () => {
  assert.equal(
    meetingDataChanged(previewSnapshot, {
      ...previewSnapshot,
      meetingsRevision: 1,
    }),
    true,
  );
  assert.equal(
    meetingDataChanged(previewSnapshot, {
      ...previewSnapshot,
      activeMeetingId: "new-meeting",
      meetingRecording: true,
    }),
    true,
  );
});
