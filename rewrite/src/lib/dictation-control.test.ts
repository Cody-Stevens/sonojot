import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  canRequestDictation,
  completedDictationSession,
  selectDictationControl,
  type DictationControlInput,
} from "./dictation-control";
import { previewSnapshot } from "./protocol";

function input(
  state: DictationControlInput["snapshot"]["state"],
  pending: string[] = [],
): DictationControlInput {
  return {
    snapshot: { state, meetingRecording: false },
    native: true,
    ready: true,
    pending,
  };
}

test("idle remains Start while delayed start IPC blocks duplicate starts", () => {
  assert.equal(selectDictationControl(input("idle")).label, "Start dictation");
  const waiting = input("idle", ["dictation-start"]);
  assert.equal(selectDictationControl(waiting).label, "Start dictation");
  assert.equal(canRequestDictation("start", waiting), false);
});

test("capture event before the start reply exposes Finish and Cancel immediately", () => {
  for (const state of ["loading", "listening"] as const) {
    const capture = input(state, ["dictation-start", "initialize"]);
    capture.ready = false;
    const control = selectDictationControl(capture);
    assert.equal(control.label, "Finish dictation");
    assert.equal(control.disabled, false);
    assert.equal(canRequestDictation("stop", capture), true);
    assert.equal(canRequestDictation("cancel", capture), true);
    assert.equal(canRequestDictation("start", capture), false);
  }
});

test("duplicate stop is blocked while cancellation stays available", () => {
  const capture = input("loading", ["dictation-start", "dictation-stop"]);
  assert.equal(selectDictationControl(capture).label, "Finish dictation");
  assert.equal(canRequestDictation("stop", capture), false);
  assert.equal(canRequestDictation("cancel", capture), true);
  capture.pending = [...capture.pending, "dictation-cancel"];
  assert.equal(canRequestDictation("cancel", capture), false);
});

test("finishing snapshot wins over late start request settlement", () => {
  for (const pending of [
    ["dictation-start", "dictation-stop"],
    ["dictation-stop"],
    [],
  ]) {
    const finishing = input("finishing", pending);
    assert.equal(selectDictationControl(finishing).label, "Finishing…");
    assert.equal(canRequestDictation("start", finishing), false);
    assert.equal(canRequestDictation("stop", finishing), false);
    assert.equal(canRequestDictation("cancel", finishing), true);
  }
});

test("cancelled idle snapshot cannot restart until previous command replies settle", () => {
  assert.equal(
    canRequestDictation("start", input("idle", ["dictation-start"])),
    false,
  );
  assert.equal(canRequestDictation("start", input("idle")), true);
});

test("cleanup can be stopped without cancelling or starting another recording", () => {
  const cleaning = input("cleaning", ["dictation-stop"]);
  cleaning.ready = false;
  assert.equal(selectDictationControl(cleaning).label, "Stop cleanup");
  assert.equal(canRequestDictation("stop-cleanup", cleaning), true);
  assert.equal(canRequestDictation("cancel", cleaning), false);
  assert.equal(canRequestDictation("start", cleaning), false);
  assert.equal(canRequestDictation("stop", cleaning), false);
  cleaning.pending = ["dictation-stop-cleanup"];
  assert.equal(canRequestDictation("stop-cleanup", cleaning), false);
});

test("native readiness and meeting ownership gate new capture only", () => {
  const unavailable = input("idle");
  unavailable.ready = false;
  assert.equal(canRequestDictation("start", unavailable), false);
  unavailable.ready = true;
  unavailable.snapshot.meetingRecording = true;
  assert.equal(canRequestDictation("start", unavailable), false);
  unavailable.native = false;
  unavailable.snapshot.state = "listening";
  assert.equal(canRequestDictation("stop", unavailable), false);
  assert.equal(canRequestDictation("cancel", unavailable), false);
});

test("test review waits for a new settled result, including incomplete recovery", () => {
  const result = {
    ...previewSnapshot,
    resultSessionId: 8,
    originalText: "test the microphone",
    outputText: "Test the microphone.",
  };
  assert.equal(completedDictationSession(result, 7), 8);
  assert.equal(completedDictationSession(result, null), 8);
  assert.equal(completedDictationSession(result, 8), null);
  assert.equal(
    completedDictationSession({ ...result, resultSessionId: null }, 7),
    null,
  );
  assert.equal(completedDictationSession({ ...result, sessionId: 9 }, 7), null);
  assert.equal(
    completedDictationSession({ ...result, meetingRecording: true }, 7),
    null,
  );
  assert.equal(
    completedDictationSession(
      { ...result, originalText: " ", outputText: "" },
      7,
    ),
    8,
  );
  for (const state of [
    "loading",
    "listening",
    "finishing",
    "cleaning",
  ] as const)
    assert.equal(completedDictationSession({ ...result, state }, 7), null);
  const incomplete = {
    ...result,
    state: "error" as const,
    recognitionCompleteness: "incomplete" as const,
    outputText: "",
    settings: { ...result.settings, doNotSaveDictations: true },
  };
  assert.equal(completedDictationSession(incomplete, 7), 8);
});
