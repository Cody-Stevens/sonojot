import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status === testInfo.expectedStatus)
    await page.goto("about:blank");
});

// The rail ticket and its sheet are covered by quiet-p1.spec.js.
const ready = { state: "ready", version: "0.23.0", notes: "A signed update." };
const available = {
  state: "available",
  version: "0.23.0",
  notes: "A signed update.",
  size: 38_000_000,
};
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
async function pushUpdate(page, patch) {
  await page.evaluate(
    (patch) =>
      window.__testPushSnapshot({
        update: { ...window.__testSnapshot().update, ...patch },
      }),
    patch,
  );
}
const about = (page) =>
  page.getByRole("group", { name: "Version", exact: true });
const notice = (page) => page.getByRole("region", { name: "Update" });

test("Updates checks explicitly and offers each next step beside Check now", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=settings");
  await page.getByRole("button", { name: /^Updates / }).click();
  await expect(page.getByText("0.22.0", { exact: true })).toBeVisible();
  await expect(about(page).getByRole("button")).toHaveCount(1);
  await page.getByRole("button", { name: "Check now" }).click();
  expect(await calls(page, "check_for_updates")).toHaveLength(1);
  await expect(page.getByText("Checking for updates…")).toBeVisible();
  await expect(page.getByRole("button", { name: "Check now" })).toBeDisabled();
  await expect(about(page).getByRole("button")).toHaveCount(1);
  await pushUpdate(page, { status: available });
  await expect(
    about(page).getByText("SonoJot 0.23.0 is available."),
  ).toBeVisible();
  const download = about(page).getByRole("button", { name: "Download update" });
  await expect(download).toBeEnabled();
  // Updates is the update area, so the main-window notice stays off this page.
  await expect(notice(page)).toHaveCount(0);
  await download.click();
  expect(await calls(page, "download_update")).toHaveLength(1);
  // Nothing to press while downloading; the line under Check now has progress.
  await expect(about(page).getByText("Downloading update…")).toBeVisible();
  await expect(about(page).getByRole("button")).toHaveCount(1);
  await pushUpdate(page, { status: { state: "downloading", percent: 42 } });
  await expect(about(page).getByText("Downloading update · 42%")).toBeVisible();
  await expect(page.getByRole("button", { name: "Check now" })).toBeDisabled();
  await pushUpdate(page, { status: ready });
  await expect(about(page).getByText("SonoJot 0.23.0 is ready.")).toBeVisible();
  await expect(
    about(page).getByRole("button", { name: "Restart to update" }),
  ).toBeEnabled();
  await expect(page.locator(".update-ticket")).toBeEnabled();
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});

test("network errors stay in Updates and permit retry", async ({ page }) => {
  await installDesktopFixture(page, {
    update: {
      status: {
        state: "error",
        message: "Could not check for updates. Try again later.",
      },
    },
  });
  await page.goto("/?page=settings");
  await page.getByRole("button", { name: /^Updates / }).click();
  await expect(
    page.getByText("Could not check for updates. Try again later.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Check now" })).toBeEnabled();
  await expect(about(page).getByRole("button")).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Update failed · Retry" }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Dictation", exact: true }).click();
  await expect(notice(page)).toHaveCount(0);
});

test("ready notice is absent from the screen tab", async ({ page }) => {
  await installDesktopFixture(page, { update: { status: ready } });
  await page.goto("/?view=overlay");
  await expect(page.locator(".screen-tab")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restart to update" }),
  ).toHaveCount(0);
  await expect(notice(page)).toHaveCount(0);
});
