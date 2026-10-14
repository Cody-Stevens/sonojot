import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";
import {
  meetingActivity,
  openApp,
  openMeeting,
  openSaveCopy,
} from "./app-helpers.js";

const lost = "Microphone lost — reconnecting…";
const reconnected = "Microphone reconnected";
// Since 0.24 short capture warnings stay in diagnostics. A real outage is
// saved as an audio outage: one quiet notice plus a marker in the transcript.
const outage = {
  source: "microphone",
  startSeconds: 12,
  endSeconds: 15,
  reason: "Microphone disconnected",
};
const outageNotice =
  "3 seconds of audio are missing around 0:12. Microphone disconnected. The gap is marked in the transcript.";

test("finalizing recording blocks audio actions until the eventual result arrives", async ({
  page,
}, testInfo) => {
  await openApp(
    page,
    {
      meeting: {
        status: "finalizing",
        transcript: "",
        hasTranscript: false,
        durationSeconds: 1,
      },
    },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  await expect(meetingActivity(page)).toHaveText("Saving recording");
  await expect(
    page.getByRole("button", { name: "Transcribe recording", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Cancel processing", exact: true }),
  ).toHaveCount(0);
  const saveCopy = await openSaveCopy(page);
  const wav = saveCopy.getByRole("button", {
    name: "Save recording (.wav)",
    exact: true,
  });
  await expect(wav).toBeDisabled();
  await page.screenshot({
    path: testInfo.outputPath("meeting-finalizing.png"),
  });
  await page.evaluate(
    (outage) =>
      window.__testUpdateMeeting({
        status: "ready",
        durationSeconds: 300,
        audioOutages: [outage],
        missingSeconds: 3,
      }),
    outage,
  );
  await expect(meetingActivity(page)).toHaveCount(0);
  await expect(page.getByLabel("Missing audio notice")).toHaveText(
    outageNotice,
  );
  await expect(wav).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Transcribe recording", exact: true }),
  ).toBeEnabled();
});

test("live source notices keep recording controls available and clear after stop", async ({
  page,
}, testInfo) => {
  await openApp(
    page,
    {
      meeting: {
        status: "recording",
        source: "both",
        transcript: "",
        hasTranscript: false,
        createdAt: new Date().toISOString(),
      },
    },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  await page.evaluate(
    (notice) =>
      window.__testPushSnapshot({
        meetingRecording: true,
        activeMeetingId: "meeting-one",
        audioLevel: 0.5,
        meetingLevels: { microphone: 0, system: 0.5 },
        meetingSourceStatus: [notice],
      }),
    lost,
  );
  const status = page.getByRole("status", { name: "Recording source status" });
  await expect(status).toHaveText(lost);
  await expect(
    page.getByRole("button", { name: "Finish recording", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("meeting-source-lost.png"),
  });
  await page.evaluate(
    (notice) =>
      window.__testPushSnapshot({
        meetingSourceStatus: [notice],
        meetingLevels: { microphone: 0.4, system: 0.5 },
      }),
    reconnected,
  );
  await expect(status).toHaveText(reconnected);
  await page
    .getByRole("button", { name: "Finish recording", exact: true })
    .click();
  await expect(status).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Transcribe recording", exact: true }),
  ).toBeEnabled();
});

test("saved audio outages survive transcription, edits and reopening", async ({
  page,
}, testInfo) => {
  await openApp(
    page,
    {
      meeting: {
        audioOutages: [outage],
        missingSeconds: 3,
        transcript: "",
        hasTranscript: false,
      },
    },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  const notice = page.getByLabel("Missing audio notice");
  await expect(notice).toHaveText(outageNotice);
  await page
    .getByRole("button", { name: "Transcribe recording", exact: true })
    .click();
  await page
    .getByRole("region", { name: "Transcribe recording" })
    .getByRole("button", { name: "Start", exact: true })
    .click();
  await expect(meetingActivity(page)).toContainText("Transcribing");
  await page.evaluate(() =>
    window.__testCompleteTranscription({
      transcript:
        "[0:05] You: Words from the saved audio.\n\n[0:20] Other speakers: After the gap.",
      hasTranscript: true,
      recognitionWarnings: [
        "Speaker labeling failed: test worker unavailable.",
      ],
    }),
  );
  const marker = page.getByLabel("Missing audio", { exact: true });
  const you = page.getByRole("textbox", { name: "You", exact: true });
  await expect(you).toHaveValue("Words from the saved audio.");
  await expect(notice).toHaveText(outageNotice);
  // The non-editable marker sits between the passages around the gap.
  await expect(page.locator(".transcript-editor > *")).toHaveCount(3);
  await expect(page.locator(".transcript-editor > *").nth(1)).toHaveText(
    "Audio missing 0:12–0:15 · Microphone disconnected",
  );
  await expect(marker).toBeVisible();
  await expect(
    page.getByText("Speaker labeling failed: test worker unavailable.", {
      exact: true,
    }),
  ).toBeVisible();
  await you.fill("My transcript correction.");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save changes", exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(
      () =>
        window.__testCalls.findLast((call) => call.command === "save_meeting")
          ?.args.transcript,
    ),
  ).toBe(
    "[0:05] You: My transcript correction.\n\n[0:20] Other speakers: After the gap.",
  );
  await page.getByRole("button", { name: "All meetings", exact: true }).click();
  await openMeeting(page, /Design review/);
  await expect(notice).toHaveText(outageNotice);
  await expect(marker).toBeVisible();
  await expect(you).toHaveValue("My transcript correction.");
  await page.screenshot({
    path: testInfo.outputPath("meeting-outage-saved.png"),
  });
});

test("legacy failed saved audio shows recovered duration and transcription guidance", async ({
  page,
}, testInfo) => {
  const error =
    "System capture reported lost audio; this meeting is incomplete. Partial files are preserved.";
  await openApp(
    page,
    {
      meeting: {
        status: "failed",
        durationSeconds: 72,
        hasAudio: true,
        hasTranscript: false,
        transcript: "",
        error,
        // Older records omit recordingWarnings; the frontend boundary supplies [].
      },
    },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  await expect(page.locator(".meeting-heading")).toContainText("1:12");
  // Calm pages: a quiet status line with the repair, not an alert.
  const status = page.locator(".meeting-notice[role=status]");
  await expect(status).toContainText(error);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("legacy-saved-recording.png"),
  });
  await status
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  const start = page
    .getByRole("region", { name: "Transcribe recording" })
    .getByRole("button", { name: "Start", exact: true });
  await expect(start).toBeEnabled();
  await start.click();
  await expect(meetingActivity(page)).toContainText("Transcribing");
  expect(
    await page.evaluate(() =>
      window.__testCalls.some((call) => call.command === "transcribe_meeting"),
    ),
  ).toBe(true);
});

test("screen tab shows recoverable source notices beside active meeting meters", async ({
  page,
}) => {
  await installDesktopFixture(page);
  // Menubar presentation uses the same capture row without native window translation.
  await page.goto("/?view=overlay&presentation=menubar");
  await expect(page.locator(".screen-tab")).toBeVisible();
  await page.evaluate(
    (notice) =>
      window.__testPushSnapshot({
        meetingRecording: true,
        activeMeetingId: "meeting-one",
        tabExpanded: true,
        meetingLevels: { microphone: 0, system: 0.6 },
        meetingSourceStatus: [notice],
      }),
    lost,
  );
  const status = page.getByRole("status", { name: "Recording source status" });
  await expect(status).toHaveText(lost);
  await expect(page.locator(".tab-stems")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop meeting", exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.evaluate(
    (notice) => window.__testPushSnapshot({ meetingSourceStatus: [notice] }),
    reconnected,
  );
  await expect(status).toHaveText(reconnected);
  await page.evaluate(() =>
    window.__testPushSnapshot({ meetingSourceStatus: [] }),
  );
  await expect(status).toHaveCount(0);
});
