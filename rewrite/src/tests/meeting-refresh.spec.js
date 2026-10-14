import { expect, test } from "@playwright/test";
import { goToPage, openApp, openMeeting } from "./app-helpers.js";

test.beforeEach(async ({ page }) => {
  await openApp(page);
  await goToPage(page, "Meetings");
  await openMeeting(page, /Design review/);
  await expect(page.locator("#meeting-transcript")).toBeEnabled();
});

test("completion during an outstanding read refreshes the open meeting", async ({ page }) => {
  await page.evaluate(() => {
    window.__testUpdateMeeting({ status: "transcribing" });
  });
  await expect(page.locator("#meeting-transcript")).toBeDisabled();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await invoke(command, args);
      if (command === "get_meeting" && !window.__heldRead) {
        window.__heldRead = true;
        return new Promise((resolve) => {
          window.__releaseRead = () => resolve(result);
        });
      }
      return result;
    };
    window.__testPushSnapshot({});
  });
  await page.waitForFunction(() => !!window.__releaseRead);
  await page.evaluate(() => window.__testUpdateMeeting({
    status: "transcribed", transcript: "Completed while the app stayed open.", hasTranscript: true,
  }));
  // Let the completion notification arrive while the old detail read is held.
  await expect.poll(() => page.evaluate(() => window.__testCalls.filter(
    (call) => call.command === "get_meeting",
  ).length)).toBeGreaterThan(1);
  await page.waitForTimeout(1700);
  await page.evaluate(() => window.__releaseRead());
  await expect(page.locator("#meeting-transcript")).toHaveValue("Completed while the app stayed open.");
  await expect(page.locator("#meeting-transcript")).toBeEnabled();
  await expect(page.getByRole("button", { name: "Cancel processing", exact: true })).toHaveCount(0);
});

test("active meeting polling recovers a missed completion event", async ({ page }) => {
  await page.evaluate(() => window.__testUpdateMeeting({ status: "transcribing" }));
  await expect(page.locator("#meeting-transcript")).toBeDisabled();
  await page.evaluate(() => window.__testUpdateMeeting({
    status: "transcribed", transcript: "Recovered completed transcript.", hasTranscript: true,
  }, false));
  await expect(page.locator("#meeting-transcript")).toHaveValue("Recovered completed transcript.", { timeout: 6000 });
  await expect(page.locator("#meeting-transcript")).toBeEnabled();
});

test("background refresh preserves unsaved edits", async ({ page }) => {
  await page.locator("#meeting-transcript").fill("My unsaved correction.");
  await page.evaluate(() => window.__testUpdateMeeting({ notes: "New saved notes." }));
  await expect(page.getByRole("button", { name: "Save changes", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Notes", exact: true }).click();
  await expect(page.locator("#meeting-notes")).not.toHaveValue("New saved notes.");
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
  await expect(page.locator("#meeting-transcript")).toHaveValue("My unsaved correction.");
});
