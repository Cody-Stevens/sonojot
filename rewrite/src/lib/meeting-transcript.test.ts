import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  hasStructure,
  parseTranscript,
  serializeTranscript,
} from "./meeting-transcript";

test("ordinary prose is never mistaken for speakers or timecodes", () => {
  for (const text of [
    "So the plan is simple: we ship Friday.\n\nOkay.",
    "[laughter] That was funny.",
    "Note: see https://example.com for details",
  ]) {
    const blocks = parseTranscript(text);
    assert.equal(hasStructure(blocks), false, text);
    assert.equal(serializeTranscript(blocks), text);
  }
});

test("known speaker labels keep their side, timecode and text", () => {
  const text = "[0:12] You: hello there\n\n[1:02:15] Other speakers: hi\nagain";
  const blocks = parseTranscript(text);
  assert.equal(hasStructure(blocks), true);
  assert.deepEqual(
    blocks.map(({ side, timecode, text }) => ({ side, timecode, text })),
    [
      { side: "you", timecode: "0:12", text: "hello there" },
      { side: "other", timecode: "1:02:15", text: "hi\nagain" },
    ],
  );
  assert.equal(serializeTranscript(blocks), text);
});

test("segment JSON carries speakers and start times", () => {
  const blocks = parseTranscript(
    JSON.stringify({
      segments: [{ speaker: "Priya", side: "other", startSeconds: 75, text: "Hi" }],
    }),
  );
  assert.deepEqual(blocks, [
    { id: "block-0", speaker: "Priya", side: "other", timecode: "1:15", text: "Hi" },
  ]);
});

test("the legacy two-column table becomes alternating passages", () => {
  const blocks = parseTranscript(
    "Time\tOther speakers\tMicrophone\r\n00:00\tGreat, thank you.\tcan do easily.\r\n00:10\t>> That's awesome.\t\r\n00:20\tVery helpful.\tYeah, perfect.",
  );
  assert.equal(hasStructure(blocks), true);
  assert.deepEqual(
    blocks.map(({ side, timecode, text }) => ({ side, timecode, text })),
    [
      { side: "other", timecode: "00:00", text: "Great, thank you." },
      { side: "you", timecode: "00:00", text: "can do easily." },
      { side: "other", timecode: "00:10", text: ">> That's awesome. Very helpful." },
      { side: "you", timecode: "00:20", text: "Yeah, perfect." },
    ],
  );
});
