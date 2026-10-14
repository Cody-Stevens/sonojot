// Focused Save-copy original-audio export checks (mock host only).
// No real user recordings are read or written; the fixture returns synthetic
// dialog paths and a null export result. Covers the mounted
// MeetingAudioExport component inside Meetings.svelte.
import { expect, test } from "@playwright/test";
import {
  openApp,
  openMeeting,
  openSaveCopy as openSaveCopyCard,
} from "./app-helpers.js";

const wavButton = "Save recording (.wav)";
// The original-audio half of the meeting's "Save a copy" card.
const audioPanel = (page) =>
  page
    .getByRole("region", { name: "Save a copy" })
    .getByRole("region", { name: "Original audio export" });

async function openSaveCopy(page, theme = "fog") {
  await openApp(page, {}, `/?page=meetings&theme=${theme}`);
  await openMeeting(page, /Design review/);
  await openSaveCopyCard(page);
  await expect(page.getByRole("button", { name: wavButton })).toBeVisible();
}

// Meetings added by the fixture appear after an explicit list refresh.
async function reopenFromList(page, title) {
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.locator("details.meeting-more summary").click();
  await page.getByRole("button", { name: "Refresh list", exact: true }).click();
  await openMeeting(page, title);
  await openSaveCopyCard(page);
}

test("audio export click opens wav dialog and calls typed command with correct id", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() => window.__testDialogSave("C:/audio-copy.wav"));
  await page.getByRole("button", { name: wavButton }).click();
  await expect(
    audioPanel(page).getByRole("status"),
  ).toContainText("Original recording saved.");
  const calls = await page.evaluate(() => window.__testCalls);
  const dialog = calls.find((c) => c.command === "plugin:dialog|save");
  expect(dialog.args.options.title).toBe("Save original recording");
  expect(dialog.args.options.defaultPath).toBe("Design review.wav");
  expect(dialog.args.options.filters).toEqual([
    { name: "Original recording (WAV)", extensions: ["wav"] },
  ]);
  expect(calls).toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-one", path: "C:/audio-copy.wav" },
  });
  // No legacy document fallback for the dedicated audio path.
  expect(
    calls.filter(
      (c) =>
        c.command === "export_meeting" &&
        c.args &&
        c.args.format === "audio",
    ),
  ).toHaveLength(0);
});

test("audio export cancel makes no export call and no misleading success", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() => window.__testDialogSave(null));
  await page.getByRole("button", { name: wavButton }).click();
  await page.waitForTimeout(300);
  const calls = await page.evaluate(() => window.__testCalls);
  expect(
    calls.filter((c) => c.command === "export_meeting_audio"),
  ).toHaveLength(0);
  await expect(
    audioPanel(page).getByRole("status"),
  ).toHaveCount(0);
  await expect(
    audioPanel(page).getByRole("alert"),
  ).toHaveCount(0);
});

test("missing audio and active recording disable with useful reasons", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() =>
    window.__testAddMeeting({
      id: "meeting-no-audio",
      title: "No audio yet",
      status: "ready",
      hasAudio: false,
      hasTranscript: false,
      transcript: "",
      notes: "",
    }),
  );
  await reopenFromList(page, /No audio yet/);
  const missing = page.getByRole("button", { name: wavButton });
  await expect(missing).toBeDisabled();
  await expect(audioPanel(page)).toContainText(
    "No saved audio for this meeting yet.",
  );

  await page.evaluate(() =>
    window.__testAddMeeting({
      id: "meeting-recording",
      title: "Live capture",
      status: "recording",
      hasAudio: true,
      hasTranscript: false,
      transcript: "",
      notes: "",
    }),
  );
  await reopenFromList(page, /Live capture/);
  const recording = page.getByRole("button", { name: wavButton });
  await expect(recording).toBeDisabled();
  await expect(audioPanel(page)).toContainText(
    "Stop the recording before exporting its audio.",
  );
});

test("unsaved text drafts and absent transcript do not block audio export", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  // Make a text draft without saving: document export must gate, audio must not.
  await page.locator("#meeting-transcript").fill("My unsaved correction.");
  await expect(
    page.getByRole("button", { name: "Save changes", exact: true }),
  ).toBeVisible();
  const saveCopy = page.getByRole("region", { name: "Save a copy" });
  await expect(
    saveCopy.getByRole("button", { name: "Export", exact: true }),
  ).toBeDisabled();
  await expect(saveCopy).toContainText("Save your changes before exporting.");
  const audio = page.getByRole("button", { name: wavButton });
  await expect(audio).toBeEnabled();
  await page.evaluate(() => window.__testDialogSave("C:/draft-audio.wav"));
  await audio.click();
  await expect(
    audioPanel(page).getByRole("status"),
  ).toContainText("Original recording saved.");
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-one", path: "C:/draft-audio.wav" },
  });

  // Saved meeting with no transcript text still exports audio.
  await page.evaluate(() =>
    window.__testAddMeeting({
      id: "meeting-no-text",
      title: "No text yet",
      status: "ready",
      hasAudio: true,
      hasTranscript: false,
      transcript: "",
      notes: "",
    }),
  );
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.getByRole("button", { name: "Discard changes" }).click();
  await page.locator("details.meeting-more summary").click();
  await page.getByRole("button", { name: "Refresh list", exact: true }).click();
  await openMeeting(page, /No text yet/);
  await openSaveCopyCard(page);
  const noText = page.getByRole("button", { name: wavButton });
  await expect(noText).toBeEnabled();
  await page.evaluate(() => window.__testDialogSave("C:/no-text-audio.wav"));
  await noText.click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-no-text", path: "C:/no-text-audio.wav" },
  });
});

test("transcription processing still allows audio export", async ({ page }) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() =>
    window.__testAddMeeting({
      id: "meeting-busy",
      title: "Busy notes",
      status: "transcribing",
      hasAudio: true,
      hasTranscript: true,
      transcript: "Saved words.",
      notes: "",
    }),
  );
  await reopenFromList(page, /Busy notes/);
  const audio = page.getByRole("button", { name: wavButton });
  await expect(audio).toBeEnabled();
  await page.evaluate(() => window.__testDialogSave("C:/busy-audio.wav"));
  await audio.click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-busy", path: "C:/busy-audio.wav" },
  });
});

test("changing selection while the save dialog awaits keeps the original id", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() =>
    window.__testAddMeeting({
      id: "meeting-two",
      title: "Second review",
      status: "ready",
      hasAudio: true,
      hasTranscript: true,
      transcript: "Second words.",
      notes: "",
    }),
  );
  await page.evaluate(() => window.__testDialogSave("__HOLD__"));
  await page.getByRole("button", { name: wavButton }).click();
  // Wait until the held save dialog has been entered.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__testCalls.filter((c) => c.command === "plugin:dialog|save")
            .length,
      ),
    )
    .toBeGreaterThan(0);
  // Switch meetings while the dialog promise is still pending.
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.locator("details.meeting-more summary").click();
  await page.getByRole("button", { name: "Refresh list", exact: true }).click();
  await openMeeting(page, /Second review/);
  await expect(page.locator(".meeting-heading h1")).toContainText(
    "Second review",
  );
  await page.evaluate(() =>
    window.__testReleaseDialogSave("C:/held-copy.wav"),
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__testCalls.filter(
            (c) => c.command === "export_meeting_audio",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  const calls = await page.evaluate(() => window.__testCalls);
  expect(calls).toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-one", path: "C:/held-copy.wav" },
  });
  expect(calls).not.toContainEqual({
    command: "export_meeting_audio",
    args: { id: "meeting-two", path: "C:/held-copy.wav" },
  });
});

test("audio export failure is visible with no misleading success", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() => {
    window.__testDialogSave("C:/fail-audio.wav");
    window.__testFailAudioExport("Disk is full (mock).");
  });
  await page.getByRole("button", { name: wavButton }).click();
  await expect(
    audioPanel(page).getByRole("alert"),
  ).toContainText("Disk is full (mock).");
  await expect(
    audioPanel(page).getByRole("status"),
  ).toHaveCount(0);
});

for (const theme of ["fog", "slate"]) {
  test(`save-copy audio panel capture in ${theme} (mock host)`, async ({
    page,
  }, testInfo) => {
    await openSaveCopy(page, theme);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    const panel = audioPanel(page);
    await expect(panel).toContainText(wavButton);
    const shotPath = `../artifacts/round6/audio-ui-shots-mock/audio-save-copy-${theme}.png`;
    await panel.screenshot({ path: shotPath });
    await testInfo.attach(`audio-save-copy-${theme} (mock host)`, {
      path: shotPath,
      contentType: "image/png",
    });
  });
}
