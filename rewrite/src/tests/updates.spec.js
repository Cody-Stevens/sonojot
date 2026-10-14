import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

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
// About's update row sits in the page; the sidebar button is outside <main>.
const about = (page) => page.locator(".update-check");
const sidebar = (page) => page.locator(".sidebar-update");
const notice = (page) => page.getByRole("region", { name: "Update" });

async function capture(page, directory, name) {
  const folder = new URL(`../../../artifacts/${directory}/`, import.meta.url);
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  // Let navigation and button transitions settle; skip endless pulses.
  await page.evaluate(() =>
    Promise.all(
      document
        .getAnimations()
        .filter((a) => a.effect?.getTiming().iterations !== Infinity)
        .map((a) => a.finished.catch(() => {})),
    ),
  );
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}

test("About checks explicitly and offers each next step beside Check now", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=about");
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
  // About is the update area, so the main-window notice stays off this page.
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
  await expect(
    sidebar(page).getByRole("button", { name: "Restart to update" }),
  ).toBeEnabled();
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});

for (const theme of ["fog", "slate"]) {
  test(`About ready row in ${theme} waits for explicit restart`, async ({
    page,
  }) => {
    await installDesktopFixture(page, { update: { status: ready } });
    await page.goto(`/?page=about&theme=${theme}`);
    const restart = about(page).getByRole("button", {
      name: "Restart to update",
    });
    await expect(restart).toBeEnabled();
    expect(await calls(page, "restart_to_update")).toHaveLength(0);
    await capture(page, "updates", `about-${theme}.png`);
    await restart.click();
    expect(await calls(page, "restart_to_update")).toHaveLength(1);
    await expect(
      about(page).getByRole("button", { name: "Restarting…", exact: true }),
    ).toBeDisabled();
    await expect(
      sidebar(page).getByRole("button", { name: "Restarting…", exact: true }),
    ).toBeDisabled();
    await expect(about(page).getByText("Restarting to update…")).toBeVisible();
  });
}

test("busy reason disables restart and clears when the backend becomes idle", async ({
  page,
}) => {
  const reason = "Wait for meeting transcription to finish.";
  await installDesktopFixture(page, {
    update: { status: ready, restartBlockedReason: reason },
  });
  await page.goto("/?page=about");
  const restart = sidebar(page).getByRole("button", {
    name: "Restart to update",
  });
  const aboutRestart = about(page).getByRole("button", {
    name: "Restart to update",
  });
  await expect(restart).toBeDisabled();
  await expect(aboutRestart).toBeDisabled();
  await expect(aboutRestart).toHaveAccessibleDescription(reason);
  await restart.focus();
  await expect(page.getByRole("tooltip")).toContainText(reason);
  await pushUpdate(page, { restartBlockedReason: null });
  await expect(restart).toBeEnabled();
  await expect(aboutRestart).toBeEnabled();
  await expect(about(page).getByText(reason)).toHaveCount(0);
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});

test("network errors stay in About and permit retry", async ({ page }) => {
  await installDesktopFixture(page, {
    update: {
      status: {
        state: "error",
        message: "Could not check for updates. Try again later.",
      },
    },
  });
  await page.goto("/?page=about");
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

test("ready row fits a narrow About window", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 700 });
  await installDesktopFixture(page, { update: { status: ready } });
  await page.goto("/?page=about");
  await expect(
    about(page).getByRole("button", { name: "Restart to update" }),
  ).toBeVisible();
  expect(
    await page
      .locator(".page-content")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
});

test("main-window notice carries an update from available to restart", async ({
  page,
}) => {
  await installDesktopFixture(page, { update: { status: available } });
  await page.goto("/");
  const slip = notice(page);
  await expect(slip).toBeVisible();
  await expect(slip.getByRole("status")).toHaveText(
    "SonoJot 0.23.0 is available.",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  // The sidebar button stays alongside the notice.
  await expect(
    sidebar(page).getByRole("button", { name: "Update available" }),
  ).toBeVisible();
  await slip.getByRole("button", { name: "Download update" }).click();
  expect(await calls(page, "download_update")).toHaveLength(1);
  await expect(slip.getByRole("status")).toHaveText("Downloading update…");
  await expect(slip.getByRole("button")).toHaveText(["Later"]);
  await pushUpdate(page, { status: { state: "downloading", percent: 42 } });
  await expect(slip.getByRole("status")).toHaveText("Downloading update · 42%");
  await pushUpdate(page, { status: ready });
  await expect(slip.getByRole("status")).toHaveText("SonoJot 0.23.0 is ready.");
  // The notice persists across pages until acted on or dismissed.
  await page.getByRole("button", { name: "Meetings", exact: true }).click();
  await expect(slip).toBeVisible();
  await slip.getByRole("button", { name: "Restart to update" }).click();
  expect(await calls(page, "restart_to_update")).toHaveLength(1);
  await expect(slip.getByRole("status")).toHaveText("Restarting to update…");
  await expect(slip.getByRole("button", { name: "Later" })).toHaveCount(0);
});

test("Later hides the notice for that version; the sidebar keeps the update", async ({
  page,
}) => {
  await installDesktopFixture(page, { update: { status: available } });
  await page.goto("/?page=vocabulary");
  await notice(page).getByRole("button", { name: "Later" }).click();
  expect(await calls(page, "dismiss_update_notice")).toHaveLength(1);
  await expect(notice(page)).toHaveCount(0);
  expect(await calls(page, "download_update")).toHaveLength(0);
  await expect(
    sidebar(page).getByRole("button", { name: "Update available" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Dictation", exact: true }).click();
  await expect(notice(page)).toHaveCount(0);
  // Downloading from the sidebar does not bring it back for this version.
  await sidebar(page).getByRole("button", { name: "Update available" }).click();
  await pushUpdate(page, { status: ready });
  await expect(
    sidebar(page).getByRole("button", { name: "Restart to update" }),
  ).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
  // The backend clears the flag when it offers a newer version.
  await pushUpdate(page, {
    status: { ...available, version: "0.24.0" },
    noticeDismissed: false,
  });
  await expect(notice(page).getByRole("status")).toHaveText(
    "SonoJot 0.24.0 is available.",
  );
});

test("notice explains a blocked restart and stays quiet for checks", async ({
  page,
}) => {
  const reason = "Save or discard your edits before restarting.";
  await installDesktopFixture(page, {
    update: { status: { state: "checking" } },
  });
  await page.goto("/");
  await expect(page.locator(".page-content")).toBeVisible();
  await expect(notice(page)).toHaveCount(0);
  await pushUpdate(page, { status: ready, restartBlockedReason: reason });
  const restart = notice(page).getByRole("button", {
    name: "Restart to update",
  });
  await expect(restart).toBeDisabled();
  await expect(restart).toHaveAccessibleDescription(reason);
  await expect(notice(page).getByText(reason)).toBeVisible();
  await expect(
    notice(page).getByRole("button", { name: "Later" }),
  ).toBeEnabled();
  await pushUpdate(page, { restartBlockedReason: null });
  await expect(restart).toBeEnabled();
});

test("notice fits a narrow window", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 700 });
  await installDesktopFixture(page, { update: { status: available } });
  await page.goto("/");
  await expect(notice(page)).toBeVisible();
  expect(
    await page
      .locator(".page-content")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
});

for (const theme of ["fog", "slate"]) {
  test(`update review screenshots in ${theme}`, async ({ page }) => {
    await installDesktopFixture(page, { update: { status: available } });
    await page.goto(`/?page=about&theme=${theme}`);
    await expect(
      about(page).getByRole("button", { name: "Download update" }),
    ).toBeVisible();
    await capture(page, "update-review", `about-available-${theme}.png`);
    await pushUpdate(page, { status: { state: "downloading", percent: 42 } });
    await expect(
      about(page).getByText("Downloading update · 42%"),
    ).toBeVisible();
    await capture(page, "update-review", `about-downloading-${theme}.png`);
    await pushUpdate(page, { status: ready });
    await expect(
      about(page).getByRole("button", { name: "Restart to update" }),
    ).toBeVisible();
    await capture(page, "update-review", `about-ready-${theme}.png`);
    await pushUpdate(page, { status: available });
    await page.getByRole("button", { name: "Dictation", exact: true }).click();
    await expect(notice(page)).toBeVisible();
    await capture(page, "update-review", `notice-available-${theme}.png`);
    await pushUpdate(page, { status: { state: "downloading", percent: 42 } });
    await expect(notice(page).getByRole("status")).toHaveText(
      "Downloading update · 42%",
    );
    await capture(page, "update-review", `notice-downloading-${theme}.png`);
    await pushUpdate(page, { status: ready });
    await expect(notice(page).getByRole("status")).toHaveText(
      "SonoJot 0.23.0 is ready.",
    );
    await capture(page, "update-review", `notice-ready-${theme}.png`);
  });
}
