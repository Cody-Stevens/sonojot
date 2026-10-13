import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

const ready = { state: "ready", version: "0.23.0", notes: "A signed update." };
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

test("About checks explicitly and receives background download progress", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=about");
  await expect(page.getByText("0.22.0", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Check for updates" }).click();
  expect(await calls(page, "check_for_updates")).toHaveLength(1);
  await expect(page.getByText("Checking for updates…")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Check for updates" }),
  ).toBeDisabled();
  await pushUpdate(page, { status: { state: "downloading", percent: 42 } });
  await expect(page.getByText("Downloading update · 42%")).toBeVisible();
  await pushUpdate(page, { status: ready });
  await expect(
    page.getByRole("button", { name: "Restart to update" }),
  ).toBeEnabled();
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});

for (const theme of ["fog", "slate"]) {
  test(`About ready notice in ${theme} waits for explicit restart`, async ({
    page,
  }) => {
    await installDesktopFixture(page, { update: { status: ready } });
    await page.goto(`/?page=about&theme=${theme}`);
    const restart = page.getByRole("button", { name: "Restart to update" });
    await expect(restart).toBeEnabled();
    expect(await calls(page, "restart_to_update")).toHaveLength(0);
    const directory = new URL("../../../artifacts/updates/", import.meta.url);
    await mkdir(directory, { recursive: true });
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({
      path: fileURLToPath(new URL(`about-${theme}.png`, directory)),
    });
    await restart.click();
    expect(await calls(page, "restart_to_update")).toHaveLength(1);
    await expect(
      page.getByRole("button", { name: "Restarting…", exact: true }),
    ).toBeDisabled();
  });
}

test("busy reason disables restart and clears when the backend becomes idle", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    update: {
      status: ready,
      restartBlockedReason: "Wait for meeting transcription to finish.",
    },
  });
  await page.goto("/?page=about");
  const restart = page.getByRole("button", { name: "Restart to update" });
  await expect(restart).toBeDisabled();
  await expect(
    page.getByText("Wait for meeting transcription to finish."),
  ).toBeVisible();
  await pushUpdate(page, { restartBlockedReason: null });
  await expect(restart).toBeEnabled();
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
    page.getByText("Could not check for updates. Try again later."),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Check for updates" }),
  ).toBeEnabled();
});

test("ready notice is absent from the screen tab", async ({ page }) => {
  await installDesktopFixture(page, { update: { status: ready } });
  await page.goto("/?view=overlay");
  await expect(page.locator(".screen-tab")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Restart to update" }),
  ).toHaveCount(0);
});

test("ready notice fits a narrow About window", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 700 });
  await installDesktopFixture(page, { update: { status: ready } });
  await page.goto("/?page=about");
  await expect(
    page.getByRole("button", { name: "Restart to update" }),
  ).toBeVisible();
  expect(
    await page
      .locator(".page-content")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
});
