import { strict as assert } from "node:assert";
import { test } from "node:test";
import { changelogSummary, releaseDate } from "./changelog";
import { parseResponse } from "./protocol";
import { updateOffered } from "./updates";

const release = (version: string) => ({
  version,
  date: "2026-11-12",
  notes: ["One."],
});

test("What's new parses the backend shape and rejects malformed notes", () => {
  const state = parseResponse("get_whats_new", {
    pending: { from: "0.28.8", to: "0.28.9", releases: [release("0.28.9")] },
    latest: null,
  });
  assert.equal(state.pending?.releases[0]?.notes[0], "One.");
  assert.throws(() =>
    parseResponse("get_whats_new", {
      pending: { from: null, to: "0.28.9", releases: [{ version: "0.28.9" }] },
      latest: null,
    }),
  );
  assert.equal(parseResponse("get_update_changelog", null), null);
});

test("the summary names the starting version only for a range", () => {
  assert.equal(changelogSummary(null), "");
  assert.equal(
    changelogSummary({
      from: "0.28.8",
      to: "0.28.9",
      releases: [release("0.28.9")],
    }),
    "",
  );
  assert.equal(
    changelogSummary({
      from: "0.28.8",
      to: "0.28.10",
      releases: [release("0.28.10"), release("0.28.9")],
    }),
    "Changes since 0.28.8.",
  );
  assert.equal(
    changelogSummary({
      from: null,
      to: "0.28.10",
      releases: [release("0.28.10"), release("0.28.9")],
    }),
    "",
  );
});

test("dates are read as calendar days and unknown dates are omitted", () => {
  assert.match(releaseDate("2026-11-12"), /2026/);
  assert.match(releaseDate("2026-11-12"), /12/);
  assert.equal(releaseDate(""), "");
  assert.equal(releaseDate("12 November"), "");
});

test("an update is on offer while available, downloading or ready", () => {
  assert.equal(updateOffered({ state: "idle" }), false);
  assert.equal(updateOffered({ state: "checking" }), false);
  assert.equal(updateOffered({ state: "error", message: "x" }), false);
  assert.equal(updateOffered({ state: "downloading", percent: 4 }), true);
  assert.equal(
    updateOffered({ state: "ready", version: "1.0.0", notes: null }),
    true,
  );
});
