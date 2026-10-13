import test from "node:test";
import assert from "node:assert/strict";
import {
  selectSetup,
  setupStatusSchema,
  setupLabel,
  type SetupStatus,
} from "./setup";

function status(): SetupStatus {
  return {
    microphone: { state: "installed", detail: null },
    speech: { state: "installed", detail: null },
    shortcut: { state: "ready", detail: null },
    cleanup: { state: "missing", detail: null },
    mediaImport: { state: "missing", detail: null },
    modelsPath: "C:/Models",
  };
}
const idle = { state: "idle" as const, meetingRecording: false };

test("installed files and active capture do not claim a loaded speech model", () => {
  const report = status();
  assert.equal(setupLabel("speech", report.speech), "Installed");
  const control = selectSetup({
    status: report,
    snapshot: { ...idle, state: "listening" },
    native: true,
  });
  assert.equal(control.keepCaptureControls, true);
  assert.equal(control.canStart, false);
  assert.equal(setupLabel("speech", report.speech), "Installed");
  report.speech.state = "loading";
  assert.equal(
    selectSetup({
      status: report,
      snapshot: { ...idle, state: "listening" },
      native: true,
    }).requiredNeedsAttention,
    false,
  );
});

test("optional cleanup and converter failures do not prevent microphone dictation", () => {
  const report = status();
  report.cleanup.state = "failed";
  report.mediaImport.state = "unsupported";
  const control = selectSetup({ status: report, snapshot: idle, native: true });
  assert.equal(control.canStart, true);
  assert.equal(control.requiredNeedsAttention, false);
});

test("unregistered shortcut needs setup but mouse recording remains available", () => {
  const report = status();
  report.shortcut.state = "failed";
  const control = selectSetup({ status: report, snapshot: idle, native: true });
  assert.equal(control.canStart, true);
  assert.equal(control.requiredNeedsAttention, true);
});

test("unavailable required items block new capture without removing stop and cancel", () => {
  for (const state of [
    "checking",
    "missing",
    "loading",
    "failed",
    "unsupported",
  ] as const) {
    const report = status();
    report.speech.state = state;
    assert.equal(
      selectSetup({ status: report, snapshot: idle, native: true }).canStart,
      false,
    );
    assert.equal(
      selectSetup({
        status: report,
        snapshot: { ...idle, state: "listening" },
        native: true,
      }).keepCaptureControls,
      true,
    );
  }
  assert.equal(
    selectSetup({ status: null, snapshot: idle, native: true }).canStart,
    false,
  );
  assert.equal(
    selectSetup({ status: status(), snapshot: idle, native: false }).canStart,
    false,
  );
  assert.equal(
    selectSetup({
      status: status(),
      snapshot: { ...idle, meetingRecording: true },
      native: true,
    }).canStart,
    false,
  );
});

test("checking capabilities keep Setup controls usable without claiming speech is ready", () => {
  const report = status();
  report.speech.state = "checking";
  report.cleanup.state = "checking";
  assert.equal(setupStatusSchema.safeParse(report).success, true);
  assert.equal(setupLabel("speech", report.speech), "Checking…");
  const control = selectSetup({ status: report, snapshot: idle, native: true });
  assert.equal(control.canConfigure, true);
  assert.equal(control.canStart, false);
  assert.equal(control.requiredNeedsAttention, false);
});

test("setup protocol rejects missing states and unknown success promises", () => {
  const report = status();
  assert.equal(setupStatusSchema.safeParse(report).success, true);
  assert.equal(
    setupStatusSchema.safeParse({
      ...report,
      speech: { state: "downloaded", detail: null },
    }).success,
    false,
  );
  assert.equal(
    setupStatusSchema.safeParse({ ...report, speech: { ready: true } }).success,
    false,
  );
});
