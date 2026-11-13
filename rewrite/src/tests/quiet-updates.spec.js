import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";
import { mkdirSync } from "node:fs";

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
  // Nothing to check while an update is on offer; its changes stay one press away.
  await expect(page.getByRole("button", { name: "Check now" })).toHaveCount(0);
  await expect(
    about(page).getByRole("button", { name: "See changes" }),
  ).toBeVisible();
  await pushUpdate(page, { status: ready });
  await expect(about(page).getByText("SonoJot 0.23.0 is ready.")).toBeVisible();
  await expect(
    about(page).getByRole("button", { name: "Restart to update" }),
  ).toBeEnabled();
  await expect(page.locator(".update-ticket")).toBeEnabled();
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});

const release = (version, date, notes) => ({ version, date, notes });
const twoVersions = {
  from: "0.22.0",
  to: "0.23.0",
  releases: [
    release("0.23.0", "2026-11-12", [
      "Faster cleanup.",
      "<b>Literal</b> text.",
    ]),
    release("0.22.1", "2026-11-01", ["Fixed a paste problem."]),
  ],
};

test("What's new opens once after an update and stays in Updates", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    whatsNew: { pending: twoVersions, latest: twoVersions },
  });
  await page.goto("/?page=dictation");
  const sheet = page.getByRole("dialog", { name: "What's new" });
  await expect(sheet).toBeVisible();
  await expect(sheet.getByText("Changes since 0.22.0.")).toBeVisible();
  const headings = sheet.getByRole("heading", { level: 3 });
  await expect(headings).toHaveText(["SonoJot 0.23.0", "SonoJot 0.22.1"]);
  // Notes are text, never markup.
  await expect(sheet.getByText("<b>Literal</b> text.")).toBeVisible();
  await expect(sheet.locator("b")).toHaveCount(0);
  // No crash notice took its place.
  await expect(
    page.getByRole("heading", { name: /SonoJot (crashed|closed)/ }),
  ).toHaveCount(0);
  await sheet.getByRole("button", { name: "Close", exact: true }).click();
  await expect(sheet).toHaveCount(0);
  expect(await calls(page, "dismiss_whats_new")).toHaveLength(1);
  // Seen once; the backend keeps it dismissed across launches (changelog.rs).
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: /^Updates / }).click();
  await expect(page.getByRole("dialog", { name: "What's new" })).toHaveCount(0);
  await page
    .getByRole("group", { name: "What's new" })
    .getByRole("button", { name: "Show" })
    .click();
  await expect(
    page
      .getByRole("dialog", { name: "What's new" })
      .getByRole("heading", { level: 3 }),
  ).toHaveCount(2);
  expect(await calls(page, "dismiss_whats_new")).toHaveLength(1);
});

test("the update prompt previews the changes before downloading", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    update: { status: available },
    updateChangelog: twoVersions,
  });
  await page.goto("/?page=dictation");
  await page.locator(".update-ticket").click();
  const prompt = page.getByRole("dialog", { name: "Updates" });
  await prompt.getByRole("button", { name: "See changes" }).click();
  const changes = page.getByRole("dialog", { name: "What's new" });
  await expect(changes.getByRole("heading", { level: 3 })).toHaveText([
    "SonoJot 0.23.0",
    "SonoJot 0.22.1",
  ]);
  expect(await calls(page, "get_update_changelog")).toHaveLength(1);
  expect(await calls(page, "download_update")).toHaveLength(0);
  await changes.getByRole("button", { name: "Download update" }).click();
  expect(await calls(page, "download_update")).toHaveLength(1);
});

test("a long nightly download never widens the rail", async ({ page }) => {
  await installDesktopFixture(page, {
    update: { status: { state: "downloading", percent: 99 } },
  });
  await page.setViewportSize({ width: 740, height: 540 });
  await page.goto("/?page=dictation");
  const ticket = page.locator(".update-ticket");
  await expect(ticket).toHaveAccessibleName("Downloading… 99%");
  await expect(ticket.getByText("99%")).toBeVisible();
  const [rail, box] = await Promise.all([
    page.locator(".quiet-rail").boundingBox(),
    ticket.boundingBox(),
  ]);
  expect(box.x + box.width).toBeLessThanOrEqual(rail.x + rail.width);
  const overflow = await page
    .locator(".quiet-rail")
    .evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBe(0);
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

test("channel switch offers and empty results stay legible in the Updates screen and rail", async ({ page }) => {
  const version = "0.28.10-nightly.202611130300";
  await installDesktopFixture(page, {
    settings: { updateChannel: "nightly" },
    update: { status: { ...available, version, switchChannel: "nightly" } },
  });
  await page.setViewportSize({ width: 740, height: 540 });
  await page.goto("/?page=settings");
  await page.evaluate(() => window.__testPushSnapshot({ version: "0.28.9" }));
  await page.getByRole("button", { name: /^Updates / }).click();
  await expect(about(page).getByText(`Switch to Nightly: ${version} is available.`)).toBeVisible();
  await expect(about(page).getByRole("button", { name: `Download ${version}` })).toBeVisible();
  const ticket = page.locator(".update-ticket");
  await expect(ticket).toHaveAccessibleName("Switch to Nightly");
  expect(await page.locator(".quiet-rail").evaluate(el => el.scrollWidth - el.clientWidth)).toBe(0);
  mkdirSync("artifacts/channel-switch", { recursive: true });
  await page.screenshot({ path: "artifacts/channel-switch/nightly-offer.png" });
  await ticket.click();
  const sheet = page.getByRole("dialog", { name: "Updates" });
  await expect(sheet.getByText(`Switch to Nightly: ${version} is available.`)).toBeVisible();
  await pushUpdate(page, { status: { state: "ready", version, notes: null, switchChannel: "nightly" }, restartBlockedReason: "Wait for meeting transcription to finish." });
  await expect(sheet.getByRole("button", { name: "Restart to switch" })).toBeDisabled();
  await sheet.getByRole("button", { name: "Later" }).click();
  await pushUpdate(page, { status: { state: "current", channel: "nightly", version: "0.28.9", platform: "Mac", reason: "noBuild" }, restartBlockedReason: null });
  await expect(about(page).getByText("Nightly has no Mac build newer than 0.28.9 yet. You'll get Nightly builds here when they're published.")).toBeVisible();
  await expect(about(page).getByRole("button", { name: "Check now" })).toBeEnabled();
  await page.screenshot({ path: "artifacts/channel-switch/no-mac-build.png" });
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
});
