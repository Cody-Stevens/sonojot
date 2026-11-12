import { expect, test } from "@playwright/test";
import { openApp, openMeeting, openSaveCopy } from "./app-helpers.js";
const audio = (page) =>
  page.getByRole("button", { name: "Save recording (.wav)", exact: true });
const calls = (page) =>
  page.evaluate(() =>
    window.__testCalls.filter(
      (call) => call.command === "export_meeting_audio",
    ),
  );
async function open(page, meeting = {}) {
  await openApp(page, { meeting }, "/?page=meetings");
  await openMeeting(page, /Design review/);
  await openSaveCopy(page);
}

test("audio export remains available during transcription and reports failure without success", async ({
  page,
}) => {
  await open(page, { status: "transcribing" });
  await expect(audio(page)).toBeEnabled();
  await page.evaluate(() => {
    window.__testDialogSave("C:/fail-audio.wav");
    window.__testFailAudioExport("Disk is full (mock).");
  });
  await audio(page).click();
  await expect(page.locator(".audio-export").getByRole("alert")).toContainText(
    "Disk is full (mock).",
  );
  await expect(page.locator(".audio-export").getByRole("status")).toHaveCount(
    0,
  );
});

test("absent audio disables export with a useful reason", async ({ page }) => {
  await open(page, { hasAudio: false });
  await expect(audio(page)).toBeDisabled();
  await expect(page.locator(".audio-export")).toContainText("No saved audio");
  expect(await calls(page)).toHaveLength(0);
});

test("changing meeting while the native save dialog waits keeps the original export id", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(() => window.__testDialogSave("__HOLD__"));
  await audio(page).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__testCalls.filter(
            (call) => call.command === "plugin:dialog|save",
          ).length,
      ),
    )
    .toBeGreaterThan(0);
  await page.getByRole("button", { name: "Close sheet" }).click();
  await page.getByRole("button", { name: "All meetings" }).click();
  await openMeeting(page, /Client call/);
  await page.evaluate(() => window.__testReleaseDialogSave("C:/held-copy.wav"));
  await expect
    .poll(() => calls(page))
    .toEqual([
      {
        command: "export_meeting_audio",
        args: { id: "meeting-one", path: "C:/held-copy.wav" },
      },
    ]);
});
