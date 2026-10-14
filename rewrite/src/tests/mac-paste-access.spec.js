import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

const directory = new URL("../../../artifacts/mac-paste/", import.meta.url);
const calls = (page, command) => page.evaluate((command) => window.__testCalls.filter((call) => call.command === command), command);
async function screenshot(page, name) {
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(`${name}.png`, directory)) });
}
const mac = macProfileFixture(16);
const settled = { ...mac, firstRun: false, settings: { ...mac.settings, setupProfile: "balanced" } };

test("Dictation page: Accessibility row opens the assistant", async ({ page }) => {
  await installDesktopFixture(page, settled);
  await page.goto("/?theme=fog");
  const row = page.getByRole("group", { name: "Accessibility access" });
  await expect(row).toContainText("Needed to paste on a Mac");
  await row.getByRole("button", { name: "Grant Accessibility", exact: true }).click();
  expect((await calls(page, "permission_assistant"))[0].args).toEqual({ kind: "accessibility", action: "open" });
  await expect(page.getByText(/drag the icon from the SonoJot helper/)).toBeVisible();
  await row.scrollIntoViewIfNeeded();
  await screenshot(page, "dictation-row-needed-fog");
});

test("Dictation page: access already on", async ({ page }) => {
  await installDesktopFixture(page, { ...settled, permissionGranted: true });
  await page.goto("/?theme=slate");
  const row = page.getByRole("group", { name: "Accessibility access" });
  await expect(row).toContainText("On. SonoJot can paste");
  await expect(row.getByRole("button")).toHaveCount(0);
  await row.scrollIntoViewIfNeeded();
  await screenshot(page, "dictation-row-on-slate");
});

test("Windows shows no Accessibility row", async ({ page }) => {
  await installDesktopFixture(page, {});
  await page.goto("/?theme=fog");
  await expect(page.getByRole("group", { name: "Paste into the app I was using" })).toBeVisible();
  await expect(page.getByRole("group", { name: "Accessibility access" })).toHaveCount(0);
});
