import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";
const lost = "Microphone lost — reconnecting…";
const reconnected = "Microphone reconnected";

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
