// Focused Save-copy original-audio export checks (mock host only).
// No real user recordings are read or written; the fixture returns synthetic
// dialog paths and a null export result. Covers the mounted
// MeetingAudioExport component inside Meetings.svelte.
import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

async function openSaveCopy(page, theme = "fog") {
  await installDesktopFixture(page);
  await page.goto(`/?page=meetings&theme=${theme}`);
  await page.getByRole("button", { name: /Design review/ }).click();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  await expect(page.locator("#meeting-panel-export")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Save original recording (.wav)" }),
  ).toBeVisible();
}

test("audio export click opens wav dialog and calls typed command with correct id", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  await page.evaluate(() => window.__testDialogSave("C:/audio-copy.wav"));
  await page
    .getByRole("button", { name: "Save original recording (.wav)" })
    .click();
  await expect(
    page.locator("#meeting-panel-export").getByRole("status"),
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
  await page
    .getByRole("button", { name: "Save original recording (.wav)" })
    .click();
  await page.waitForTimeout(300);
  const calls = await page.evaluate(() => window.__testCalls);
  expect(
    calls.filter((c) => c.command === "export_meeting_audio"),
  ).toHaveLength(0);
  await expect(
    page.locator("#meeting-panel-export").getByRole("status"),
  ).toHaveCount(0);
  await expect(
    page.locator("#meeting-panel-export").getByRole("alert"),
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
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("button", { name: /No audio yet/ }).click();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  const missing = page.getByRole("button", {
    name: "Save original recording (.wav)",
  });
  await expect(missing).toBeDisabled();
  await expect(page.locator("#meeting-panel-export")).toContainText(
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
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("button", { name: /Live capture/ }).click();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  const recording = page.getByRole("button", {
    name: "Save original recording (.wav)",
  });
  await expect(recording).toBeDisabled();
  await expect(page.locator("#meeting-panel-export")).toContainText(
    "Stop the recording before exporting its audio.",
  );
});

test("unsaved text drafts and absent transcript do not block audio export", async ({
  page,
}) => {
  await openSaveCopy(page, "fog");
  // Make a text draft without saving: document export must gate, audio must not.
  await page.getByRole("tab", { name: "Full transcript", exact: true }).click();
  await page.locator("#meeting-transcript").fill("My unsaved correction.");
  await expect(page.getByText("Unsaved changes", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Export meeting", exact: true }),
  ).toBeDisabled();
  const audio = page.getByRole("button", {
    name: "Save original recording (.wav)",
  });
  await expect(audio).toBeEnabled();
  await page.evaluate(() => window.__testDialogSave("C:/draft-audio.wav"));
  await audio.click();
  await expect(
    page.locator("#meeting-panel-export").getByRole("status"),
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
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("button", { name: /No text yet/ }).click();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  const noText = page.getByRole("button", {
    name: "Save original recording (.wav)",
  });
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
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("button", { name: /Busy notes/ }).click();
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  const audio = page.getByRole("button", {
    name: "Save original recording (.wav)",
  });
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
  await page
    .getByRole("button", { name: "Save original recording (.wav)" })
    .click();
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
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await page.getByRole("button", { name: /Second review/ }).click();
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
  await page
    .getByRole("button", { name: "Save original recording (.wav)" })
    .click();
  await expect(
    page.locator("#meeting-panel-export").getByRole("alert"),
  ).toContainText("Disk is full (mock).");
  await expect(
    page.locator("#meeting-panel-export").getByRole("status"),
  ).toHaveCount(0);
});

for (const theme of ["fog", "slate"]) {
  test(`save-copy audio panel capture in ${theme} (mock host)`, async ({
    page,
  }, testInfo) => {
    await openSaveCopy(page, theme);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    const panel = page.locator("#meeting-panel-export");
    await expect(panel).toContainText("Save original recording (.wav)");
    const shotPath = `../artifacts/round6/audio-ui-shots-mock/audio-save-copy-${theme}.png`;
    await panel.screenshot({ path: shotPath });
    await testInfo.attach(`audio-save-copy-${theme} (mock host)`, {
      path: shotPath,
      contentType: "image/png",
    });
  });
}
