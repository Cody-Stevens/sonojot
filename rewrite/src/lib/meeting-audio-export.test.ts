import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  audioExportFilename,
  audioExportSaveOptions,
  canExportAudio,
  audioExportInvokeArgs,
  audioExportFallbackArgs,
  shouldInvokeAudioExport,
} from "./meeting-audio-export";

test("audio filename sanitizes the meeting title and stays an honest wav", () => {
  assert.equal(audioExportFilename("Release planning"), "Release planning.wav");
  assert.equal(audioExportFilename(""), "Meeting.wav");
  assert.equal(audioExportFilename("   "), "Meeting.wav");
  assert.equal(audioExportFilename('a<b>c:d"e/f\\g|h?i*j'), "a-b-c-d-e-f-g-h-i-j.wav");
  assert.equal(audioExportFilename("trailing. "), "trailing.wav");
  assert.ok(audioExportFilename("x".repeat(200)).length <= 104);
});

test("save dialog defaults to the sanitized title with a wav-only filter", () => {
  const options = audioExportSaveOptions("Sprint review");
  assert.equal(options.defaultPath, "Sprint review.wav");
  assert.equal(options.dialogTitle, "Save original recording");
  assert.deepEqual(options.filters, [
    { name: "Original recording (WAV)", extensions: ["wav"] },
  ]);
});

test("audio export stays enabled while transcription or notes process", () => {
  for (const status of [
    "ready",
    "transcribed",
    "processed",
    "transcribing",
    "processingNotes",
    "generatingTitle",
    "failed",
    "cancelled",
  ]) {
    const gate = canExportAudio({
      status,
      hasAudio: true,
      busy: false,
      native: true,
    });
    assert.equal(gate.enabled, true, status);
  }
});

test("recording, missing audio, busy and browser preview disable with clear reasons", () => {
  assert.deepEqual(
    canExportAudio({ status: "recording", hasAudio: true, busy: false, native: true }),
    { enabled: false, reason: "Stop the recording before exporting its audio." },
  );
  assert.deepEqual(
    canExportAudio({ status: "ready", hasAudio: false, busy: false, native: true }),
    { enabled: false, reason: "No saved audio for this meeting yet." },
  );
  assert.equal(
    canExportAudio({ status: "ready", hasAudio: true, busy: true, native: true }).enabled,
    false,
  );
  assert.equal(
    canExportAudio({ status: "ready", hasAudio: true, busy: false, native: false }).enabled,
    false,
  );
});

test("cancel never invokes: null dialog result is a no-op", () => {
  assert.equal(shouldInvokeAudioExport(null), false);
  assert.equal(shouldInvokeAudioExport(""), false);
  assert.equal(shouldInvokeAudioExport("/tmp/copy.wav"), true);
  // No invoke happens on cancel: the component returns before any command.
  let invoked = 0;
  const dialogResult: string | null = null;
  if (shouldInvokeAudioExport(dialogResult)) invoked += 1;
  assert.equal(invoked, 0);
});

test("invoke shapes match the backend commands", () => {
  assert.deepEqual(audioExportInvokeArgs("abc", "/tmp/a.wav"), {
    id: "abc",
    path: "/tmp/a.wav",
  });
  assert.deepEqual(audioExportFallbackArgs("abc", "/tmp/a.wav"), {
    id: "abc",
    path: "/tmp/a.wav",
    format: "audio",
  });
});
