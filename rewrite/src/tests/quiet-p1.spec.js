import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

test.afterEach(async ({ page }, testInfo) => {
  // Release the fixture and live page before Edge closes its context.
  if (testInfo.status === testInfo.expectedStatus)
    await page.goto("about:blank");
});

const folder = new URL("../../artifacts/redesign-p1/", import.meta.url);
const nav = (page) => page.getByRole("navigation", { name: "Main navigation" });
const heading = (page, name) =>
  page.getByRole("heading", { name, exact: true, level: 1 });
const calls = (page, command) =>
  page.evaluate(
    (command) =>
      window.__testCalls.filter((entry) => entry.command === command),
    command,
  );
async function open(page, path = "/", options = {}) {
  await installDesktopFixture(page, options);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(path);
  await expect(nav(page)).toBeVisible();
}
async function capture(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
async function update(page, status, patch = {}) {
  await page.evaluate(
    ({ status, patch }) => window.__testUpdateStatus(status, patch),
    { status, patch },
  );
}

test("four-item shell, keyboard navigation, legacy ids and Settings doors", async ({
  page,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open(page);
  await expect(nav(page).getByRole("button")).toHaveText([
    "Dictation",
    "Meetings",
    "Words",
    "Settings",
  ]);
  await capture(page, "01-frame-fog.png");
  await nav(page)
    .getByRole("button", { name: "Dictation", exact: true })
    .focus();
  await page.keyboard.press("End");
  await expect(
    nav(page).getByRole("button", { name: "Settings" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(heading(page, "Settings")).toBeVisible();
  await capture(page, "37-settings-fog.png");
  await page.getByRole("button", { name: /^Dictation / }).click();
  await page.getByRole("button", { name: /^Speech / }).click();
  await expect(heading(page, "Speech")).toBeVisible();
  await page.getByRole("button", { name: "Back to Settings" }).click();
  await expect(heading(page, "Settings")).toBeVisible();
  await page.getByRole("button", { name: /^Memory / }).click();
  await expect(heading(page, "Memory")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Keep all ready", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Back to Settings" }).click();
  await page.getByRole("button", { name: /^On disk / }).click();
  await expect(heading(page, "On disk")).toBeVisible();

  for (const [id, tab, title] of [
    ["dictation", "dictation", "Ready"],
    ["meetings", "meetings", "Not recording"],
    ["setup", "dictation", "Speech"],
    ["microphone", "dictation", "Shortcut and mic"],
    ["shortcuts", "dictation", "Shortcut and mic"],
    ["vocabulary", "words", "Words"],
    ["appearance", "settings", "Look and startup"],
    ["about", "settings", "Help"],
  ]) {
    await page.evaluate(
      (page) => window.__TAURI_INTERNALS__.invoke("show_page", { page }),
      id,
    );
    await expect(heading(page, title)).toBeVisible();
    await expect(nav(page).locator(`[data-nav="${tab}"]`)).toHaveAttribute(
      "aria-current",
      "page",
    );
  }
  await page.evaluate(() => window.__testNavigatePage("report"));
  await expect(page.getByRole("textbox").first()).toBeVisible();
  await expect(nav(page).locator('[data-nav="settings"]')).toHaveAttribute(
    "aria-current",
    "page",
  );
  await page.evaluate(() => window.__testNavigatePage("not-a-page"));
  await expect(page).toHaveTitle("Report a problem · SonoJot");
  await page.evaluate(() => window.__testNavigatePage("dictation"));
  await expect(heading(page, "Ready")).toBeVisible();
  expect(errors).toEqual([]);
});

test("unsaved words guard both rail clicks and native routes", async ({
  page,
}) => {
  await open(page, "/?page=vocabulary");
  await page
    .getByRole("textbox", { name: "Add a word", exact: true })
    .fill("sono jot -> SonoJot");
  await nav(page).getByRole("button", { name: "Settings" }).click();
  const dialog = page.getByRole("dialog", { name: "Leave these changes?" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Keep working" }).click();
  await expect(heading(page, "Words")).toBeVisible();
  await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("show_page", { page: "setup" }),
  );
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Discard changes" }).click();
  await expect(heading(page, "Speech")).toBeVisible();
  expect(await calls(page, "save_vocabulary")).toHaveLength(0);
});

test("screen-tab meeting appears in the rail across tabs and stops cleanly", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("screen_tab_meeting"),
  );
  const ticket = page.getByRole("button", { name: "Open recording meeting" });
  await expect(ticket).toBeVisible();
  await expect(ticket.locator("strong")).toHaveText(/\d+:\d{2}/);
  await expect(nav(page).getByLabel("Recording")).toBeVisible();
  await capture(page, "08-recording-elsewhere.png");
  await nav(page).getByRole("button", { name: "Settings" }).click();
  await expect(ticket).toBeVisible();
  await ticket.click();
  await expect(nav(page).locator('[data-nav="meetings"]')).toHaveAttribute(
    "aria-current",
    "page",
  );
  await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("stop_meeting"));
  await expect(ticket).toHaveCount(0);
  await expect(nav(page).getByLabel("Recording")).toHaveCount(0);
  expect(await calls(page, "screen_tab_meeting")).toHaveLength(1);
});

test("repair ticket, progress, restart blocker and modal keyboard behavior", async ({
  page,
}) => {
  const available = {
    state: "available",
    version: "0.28.8",
    repair: true,
    notes: "Repair installed files.",
    size: 38000000,
  };
  await open(page, "/", { update: { status: available } });
  await page
    .getByRole("button", { name: "Repair update", exact: true })
    .click();
  const sheet = page.getByRole("dialog", { name: "Updates", exact: true });
  await expect(sheet).toBeVisible();
  await sheet.getByRole("button", { name: "Download repair" }).click();
  expect(await calls(page, "download_update")).toHaveLength(1);
  await update(page, { state: "downloading", percent: 42 });
  await expect(sheet.getByRole("status")).toContainText("42%");
  const ready = {
    state: "ready",
    version: "0.28.8",
    notes: "Repair installed files.",
  };
  await update(page, ready, {
    restartBlockedReason: "Wait for meeting transcription to finish.",
  });
  const restart = sheet.getByRole("button", { name: "Restart to update" });
  await expect(restart).toBeDisabled();
  await expect(restart).toHaveAccessibleDescription(
    "Wait for meeting transcription to finish.",
  );
  await capture(page, "updates-repair-sheet.png");
  await sheet.getByRole("button", { name: "Later" }).focus();
  await page.keyboard.press("Tab");
  await expect(
    sheet.getByRole("button", { name: "Close sheet" }),
  ).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(sheet.getByRole("button", { name: "Later" })).toBeFocused();
  await page.evaluate(() => window.__testPushSnapshot({ state: "cleaning" }));
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
  expect(await calls(page, "stop_cleanup")).toHaveLength(0);
  await expect(page.locator(".update-ticket")).toBeFocused();
  await page.evaluate(() => window.__testPushSnapshot({ state: "idle" }));
  await page.locator(".update-ticket").click();
  await sheet.getByRole("button", { name: "Later" }).click();
  expect(await calls(page, "dismiss_update_notice")).toHaveLength(1);
  await expect(page.locator(".update-ticket")).toBeVisible();
  await page.locator(".update-ticket").click();
  await update(page, ready, { restartBlockedReason: null });
  await expect(restart).toBeEnabled();
  expect(await calls(page, "restart_to_update")).toHaveLength(0);
  await restart.click();
  expect(await calls(page, "restart_to_update")).toHaveLength(1);
});

test("Fog and Slate controls preserve settings, fixed grain and plain mode", async ({
  page,
}) => {
  await open(page, "/?page=appearance");
  await page.getByRole("button", { name: /Slate/ }).click();
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  const stock = await page.locator(".page-frame").evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      color: style.backgroundColor,
      image: style.backgroundImage,
      blend: style.backgroundBlendMode,
    };
  });
  expect(stock.color).toBe("rgb(15, 19, 19)");
  expect(stock.image).toContain("linear-gradient");
  expect(stock.blend).toBe("normal, soft-light");
  await capture(page, "look-slate.png");
  const paper = page.getByRole("switch", { name: "Paper and print" });
  await paper.click();
  await expect(paper).toHaveAttribute("aria-checked", "false");
  await expect(page.locator(".studio")).toHaveAttribute("data-material", "off");
  expect(
    await page
      .locator(".page-frame")
      .evaluate((node) => getComputedStyle(node).backgroundImage),
  ).toBe("none");
  await paper.click();
  await nav(page).getByRole("button", { name: "Settings" }).click();
  await expect(heading(page, "Settings")).toBeVisible();
  await capture(page, "37-settings-slate.png");
  expect(
    await page
      .locator(".page-frame")
      .evaluate((node) => getComputedStyle(node).backgroundSize),
  ).toContain("620px 620px");
  const geometry = await page.locator(".page-content").evaluate((node) => ({
    width: node.clientWidth,
    scroll: node.scrollWidth,
  }));
  expect(geometry.scroll).toBeLessThanOrEqual(geometry.width);
});

test("error recovery and Escape keep their native command paths", async ({
  page,
}) => {
  await open(page, "/?page=setup");
  await expect(heading(page, "Speech")).toBeVisible();
  for (const [error, action, title] of [
    ["Microphone unavailable", "Check microphone", "Shortcut and mic"],
    ["hotkey could not be registered", "Open shortcuts", "Shortcut and mic"],
    [
      "SETUP_INCOMPLETE: Finish setup to start dictating.",
      "Open Models",
      "Speech",
    ],
    ["Vocabulary entry invalid", "Review vocabulary", "Words"],
    ["System audio unavailable", "Open meetings", "Not recording"],
  ]) {
    await page.evaluate((error) => window.__testPushSnapshot({ error }), error);
    await page
      .locator(".page-notices")
      .getByRole("button", { name: action, exact: true })
      .click();
    await expect(heading(page, title)).toBeVisible();
  }
  await page.evaluate(() => window.__testNavigatePage("dictation"));
  await page.evaluate(() =>
    window.__testPushSnapshot({
      error: null,
      state: "listening",
      sessionId: 41,
    }),
  );
  await page.keyboard.press("Escape");
  expect(await calls(page, "cancel_dictation")).toHaveLength(1);
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "cleaning", sessionId: 42 }),
  );
  await page.keyboard.press("Escape");
  expect(await calls(page, "stop_cleanup")).toHaveLength(1);
});
