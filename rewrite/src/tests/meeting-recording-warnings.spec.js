import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

const lost = "Microphone lost — reconnecting…";
const reconnected = "Microphone reconnected";
const savedWarning = "Microphone disconnected at 0:12; recording continued. Reconnected to the default microphone at 0:15.";

test("finalizing recording blocks audio actions until the eventual result arrives", async ({ page }, testInfo) => {
  await installDesktopFixture(page, {
    meeting: { status: "finalizing", transcript: "", hasTranscript: false, durationSeconds: 1 },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  await expect(page.locator(".meeting-heading .meeting-activity")).toHaveText("Saving recording");
  await expect(page.getByRole("button", { name: "Transcribe recording", exact: true })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Cancel processing", exact: true })).toHaveCount(0);
  await page.getByRole("tab", { name: "Options", exact: true }).click();
  await expect(page.getByRole("button", { name: "Cancel processing", exact: true })).toHaveCount(0);
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save original recording (.wav)", exact: true })).toBeDisabled();
  await page.screenshot({ path: testInfo.outputPath("meeting-finalizing.png") });
  await page.evaluate((warning) => window.__testUpdateMeeting({
    status: "ready", durationSeconds: 5, recordingWarnings: [warning],
  }), savedWarning);
  await expect(page.locator(".meeting-heading .meeting-activity")).toHaveText("Ready");
  await expect(page.getByLabel("Recording warnings", { exact: true })).toContainText(savedWarning);
  await expect(page.getByRole("button", { name: "Save original recording (.wav)", exact: true })).toBeEnabled();
  await page.getByRole("tab", { name: "Full transcript", exact: true }).click();
  await expect(page.getByRole("button", { name: "Transcribe recording", exact: true })).toBeEnabled();
});

test("live source notices keep recording controls available and clear after stop", async ({ page }, testInfo) => {
  await installDesktopFixture(page, {
    meeting: { status: "recording", source: "both", transcript: "", hasTranscript: false, createdAt: new Date().toISOString() },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  await page.evaluate((notice) => window.__testPushSnapshot({
    meetingRecording: true, activeMeetingId: "meeting-one", audioLevel: 0.5,
    meetingLevels: { microphone: 0, system: 0.5 }, meetingSourceStatus: [notice],
  }), lost);
  const status = page.getByRole("status", { name: "Recording source status" });
  await expect(status).toHaveText(lost);
  await expect(page.getByRole("button", { name: "Finish recording", exact: true })).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("meeting-source-lost.png") });
  await page.evaluate((notice) => window.__testPushSnapshot({
    meetingSourceStatus: [notice], meetingLevels: { microphone: 0.4, system: 0.5 },
  }), reconnected);
  await expect(status).toHaveText(reconnected);
  // The backend response owns warnings; this fixture supplies its saved result.
  await page.evaluate((warning) => window.__testUpdateMeeting({ recordingWarnings: [warning] }), savedWarning);
  await page.getByRole("button", { name: "Finish recording", exact: true }).click();
  await expect(status).toHaveCount(0);
  await expect(page.getByLabel("Recording warnings", { exact: true })).toContainText(savedWarning);
  await expect(page.getByRole("button", { name: "Transcribe recording", exact: true })).toBeEnabled();
});

test("saved recording warnings survive transcription, edits and reopening", async ({ page }, testInfo) => {
  await installDesktopFixture(page, {
    meeting: { recordingWarnings: [savedWarning], transcript: "", hasTranscript: false },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  const warnings = page.getByLabel("Recording warnings", { exact: true });
  await expect(warnings).toContainText(savedWarning);
  await page.getByRole("button", { name: "Transcribe recording", exact: true }).click();
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(page.locator(".meeting-heading .meeting-activity")).toContainText("Transcribing");
  await page.evaluate(() => window.__testCompleteTranscription({
    transcript: "Words from the saved audio.", hasTranscript: true,
    recognitionWarnings: ["Speaker labeling failed: test worker unavailable."],
  }));
  await expect(page.locator("#meeting-transcript")).toHaveValue("Words from the saved audio.");
  await expect(warnings).toContainText(savedWarning);
  await expect(page.getByText("Speaker labeling failed: test worker unavailable.", { exact: true })).toBeVisible();
  await page.locator("#meeting-transcript").fill("My transcript correction.");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(page.getByRole("button", { name: "Save changes", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "All meetings", exact: true }).click();
  await page.getByRole("button", { name: /Design review/ }).click();
  await expect(warnings).toContainText(savedWarning);
  await expect(page.locator("#meeting-transcript")).toHaveValue("My transcript correction.");
  await page.screenshot({ path: testInfo.outputPath("meeting-recording-warnings-saved.png") });
  await page.getByRole("tab", { name: "Options", exact: true }).click();
  await expect(warnings).toContainText(savedWarning);
});

test("legacy failed saved audio shows recovered duration and transcription guidance", async ({ page }, testInfo) => {
  await installDesktopFixture(page, {
    meeting: {
      status: "failed", durationSeconds: 72, hasAudio: true, hasTranscript: false, transcript: "",
      error: "System capture reported lost audio; this meeting is incomplete. Partial files are preserved.",
      // Older records omit recordingWarnings; the frontend boundary supplies [].
    },
  });
  await page.goto("/?page=meetings");
  await page.getByRole("button", { name: /Design review/ }).click();
  await expect(page.locator(".meeting-heading")).toContainText("1:12");
  await expect(page.getByRole("alert")).toContainText("Recording saved with interruptions");
  await expect(page.getByRole("alert")).toContainText("The saved recording can still be transcribed.");
  await expect(page.getByRole("alert")).not.toContainText("Check the Windows output device");
  await page.screenshot({ path: testInfo.outputPath("legacy-saved-recording.png") });
  await page.getByRole("button", { name: "Transcribe recording", exact: true }).click();
  await expect(page.getByRole("button", { name: "Start", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(page.locator(".meeting-heading .meeting-activity")).toContainText("Transcribing");
  expect(await page.evaluate(() => window.__testCalls.some((call) => call.command === "transcribe_meeting"))).toBe(true);
});

test("screen tab shows recoverable source notices beside active meeting meters", async ({ page }) => {
  await installDesktopFixture(page);
  // Menubar presentation uses the same capture row without native window translation.
  await page.goto("/?view=overlay&presentation=menubar");
  await expect(page.locator(".screen-tab")).toBeVisible();
  await page.evaluate((notice) => window.__testPushSnapshot({
    meetingRecording: true, activeMeetingId: "meeting-one", tabExpanded: true,
    meetingLevels: { microphone: 0, system: 0.6 }, meetingSourceStatus: [notice],
  }), lost);
  const status = page.getByRole("status", { name: "Recording source status" });
  await expect(status).toHaveText(lost);
  await expect(page.locator(".tab-stems")).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop meeting", exact: true })).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.evaluate((notice) => window.__testPushSnapshot({ meetingSourceStatus: [notice] }), reconnected);
  await expect(status).toHaveText(reconnected);
  await page.evaluate(() => window.__testPushSnapshot({ meetingSourceStatus: [] }));
  await expect(status).toHaveCount(0);
});
