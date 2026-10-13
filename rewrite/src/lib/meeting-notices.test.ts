import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  meetingClock,
  meetingNotice,
  outageLabel,
  transcriptEntries,
  visibleRecognitionWarnings,
} from "./meeting-notices";
import { parseTranscript, serializeTranscript } from "./meeting-transcript";
import { meetingDetailSchema } from "./protocol";

const outage = {
  source: "microphone",
  startSeconds: 760,
  endSeconds: 774,
  reason: "Microphone disconnected",
};

test("outage markers sit between timed passages and never enter editable text", () => {
  const text = "[12:31] Speaker 1: Before.\n\n[12:55] You: After.";
  const blocks = parseTranscript(text);
  const entries = transcriptEntries(blocks, [outage]);
  assert.deepEqual(
    entries.map((entry) => entry.kind),
    ["passage", "outage", "passage"],
  );
  assert.equal(
    outageLabel(outage),
    "Audio missing 12:40–12:54 · Microphone disconnected",
  );
  assert.equal(serializeTranscript(blocks), text);
});

test("quiet summary uses the strict one-percent rule without inventing markers", () => {
  assert.equal(
    meetingNotice({
      audioOutages: [],
      missingSeconds: 7.2,
      durationSeconds: 7680,
    }),
    "",
  );
  assert.equal(
    meetingNotice({
      audioOutages: [],
      missingSeconds: 10,
      durationSeconds: 1000,
    }),
    "",
  );
  assert.match(
    meetingNotice({
      audioOutages: [],
      missingSeconds: 10.1,
      durationSeconds: 1000,
    }),
    /10 seconds/,
  );
  assert.match(
    meetingNotice({
      audioOutages: [outage],
      missingSeconds: 14,
      durationSeconds: 7680,
    }),
    /12:40/,
  );
});

test("recognition omissions do not claim recorded audio was lost", () => {
  const gap = { ...outage, source: "recognition", reason: "Token limit" };
  assert.match(outageLabel(gap), /^Transcription missing/);
  const notice = meetingNotice({
    audioOutages: [gap],
    missingSeconds: 0,
    durationSeconds: 1000,
  });
  assert.match(notice, /could not be transcribed/);
  assert.doesNotMatch(notice, /seconds of audio/);
});

test("early-stop markers without a measurable loss do not invent a duration", () => {
  const notice = meetingNotice({
    audioOutages: [
      {
        source: "recording",
        startSeconds: 10,
        endSeconds: 10,
        reason: "Recording ended early",
      },
    ],
    missingSeconds: 0,
    durationSeconds: 10,
  });
  assert.match(notice, /Recording ended early around 0:10/);
  assert.doesNotMatch(notice, /second of audio/);
});

test("collapse diagnostics stay hidden and useful speaker warnings remain", () => {
  assert.deepEqual(
    visibleRecognitionWarnings([
      "Repeated text was reduced to its first occurrence",
      "Speaker labels could not be completed.",
    ]),
    ["Speaker labels could not be completed."],
  );
  assert.equal(meetingClock(4360), "1:12:40");
});

test("old meeting payloads load without cleaned copies or outage metadata", () => {
  const legacy = meetingDetailSchema.parse({
    id: "old",
    title: "Old",
    createdAt: "2026-10-10",
    durationSeconds: 10,
    status: "transcribed",
    hasAudio: true,
    hasTranscript: true,
    transcript: "Original",
    notes: "",
    source: "microphone",
    audioPath: null,
    error: null,
  });
  assert.equal(legacy.cleanedTranscript, null);
  assert.deepEqual(legacy.audioOutages, []);
  assert.equal(legacy.missingSeconds, 0);
  const cleaned = meetingDetailSchema.parse({
    ...legacy,
    cleanedTranscript: "Cleaned",
  });
  assert.equal(cleaned.transcript, "Original");
  assert.equal(cleaned.cleanedTranscript, "Cleaned");
});
