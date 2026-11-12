import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  hasStructure,
  parseTranscript,
  serializeTranscript,
} from "./meeting-transcript";

test("blank lines within an edited passage keep its speaker on reload", () => {
  const blocks = parseTranscript("[0:12] Speaker 1: a\n\n[0:20] Rachel: next");
  blocks[0].text = "a\n\n b\r\n \r\n\r\nc";
  const reloaded = parseTranscript(serializeTranscript(blocks));
  assert.equal(reloaded.length, 2);
  assert.equal(reloaded[0].speaker, "Speaker 1");
  assert.equal(reloaded[0].timecode, "0:12");
  assert.equal(reloaded[0].text, "a\n b\nc");
  assert.equal(reloaded[1].speaker, "Rachel");
});

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
      segments: [
        { speaker: "Priya", side: "other", startSeconds: 75, text: "Hi" },
      ],
    }),
  );
  assert.deepEqual(blocks, [
    {
      id: "block-0",
      speaker: "Priya",
      side: "other",
      timecode: "1:15",
      text: "Hi",
    },
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
      {
        side: "other",
        timecode: "00:10",
        text: ">> That's awesome. Very helpful.",
      },
      { side: "you", timecode: "00:20", text: "Yeah, perfect." },
    ],
  );
});

test("named speakers count only after a timecode", () => {
  const blocks = parseTranscript(
    "[0:01] Rachel Afant: Hi.\n\n[0:02] Cody Stevens: Hello.\n\nNote: plain prose.",
  );
  assert.deepEqual(
    blocks.map(({ speaker, side, text }) => ({ speaker, side, text })),
    [
      { speaker: "Rachel Afant", side: "unknown", text: "Hi." },
      { speaker: "Cody Stevens", side: "unknown", text: "Hello." },
      { speaker: "Transcript", side: "unknown", text: "Note: plain prose." },
    ],
  );
  assert.equal(
    serializeTranscript(blocks).startsWith("[0:01] Rachel Afant: Hi."),
    true,
  );
});

test("renamed lowercase and international speakers survive passage editing", () => {
  const text = "[0:01] dana: Hello.\n\n[0:02] 李: Hi.\n\nNote: ordinary prose.";
  const blocks = parseTranscript(text);
  assert.deepEqual(
    blocks.map((block) => block.speaker),
    ["dana", "李", "Transcript"],
  );
  blocks[0].text = "Hello again.";
  assert.equal(
    serializeTranscript(blocks),
    text.replace("Hello.", "Hello again."),
  );
});
