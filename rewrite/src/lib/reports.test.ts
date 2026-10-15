import { test } from "node:test";
import assert from "node:assert/strict";
import {
  reportSchema,
  reportCanSend,
  shouldShowCrash,
  crashHeading,
  type PendingCrash,
  type Report,
} from "./reports";

const report: Report = {
  schema: 1,
  kind: "bug",
  requestId: "602fe893-2af7-4d16-94f0-e43621f95971",
  app: {
    version: "0.28.6",
    platform: "linux",
    arch: "x86_64",
    os: "Arch Linux",
  },
  notes: "",
  crash: null,
  diagnostics: null,
};
const pending: PendingCrash = {
  id: report.requestId,
  reason: "panic",
  app: report.app,
  crash: { summary: "Rust panic", location: "speech.rs:42", backtrace: "" },
};

test("bugs require notes, while crashes require explicit consent", () => {
  assert.equal(reportCanSend(report, false), false);
  assert.equal(reportCanSend({ ...report, notes: " \n " }, true), false);
  assert.equal(reportCanSend({ ...report, notes: "It stopped." }, false), true);
  const crash: Report = { ...report, kind: "crash", crash: pending.crash };
  assert.equal(reportCanSend(crash, false), false);
  assert.equal(reportCanSend(crash, true), true);
});
test("notes use Unicode characters and the whole report has a byte limit", () => {
  assert.equal(
    reportCanSend({ ...report, notes: "📝".repeat(4000) }, false),
    true,
  );
  assert.equal(
    reportCanSend({ ...report, notes: "x".repeat(4001) }, false),
    false,
  );
  assert.equal(
    reportCanSend(
      {
        ...report,
        notes: "Bug",
        diagnostics: { oversized: "x".repeat(65536) },
      },
      false,
    ),
    false,
  );
  assert.equal(
    reportSchema.safeParse({ ...report, transcript: "must not be a field" })
      .success,
    false,
  );
});
test("crash view waits for setup and terms, and marker copy stays honest", () => {
  assert.equal(shouldShowCrash(pending, false, false, false), false);
  assert.equal(shouldShowCrash(pending, true, true, false), false);
  assert.equal(shouldShowCrash(pending, true, false, true), false);
  assert.equal(shouldShowCrash(pending, true, false, false), true);
  assert.equal(shouldShowCrash(null, true, false, false), false);
  assert.equal(crashHeading("panic"), "SonoJot crashed");
  assert.equal(crashHeading("unexpectedExit"), "SonoJot closed unexpectedly");
});
