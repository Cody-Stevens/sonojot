import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { installDesktopFixture } from "./desktop-fixture.js";
import { speechProfiles } from "./speech-page-fixture.js";

// Review captures for the light Speech page (November 13, 2026). Opt-in:
// SPEECH_SHOTS=<folder> npx playwright test speech-page-shots
const folder = process.env.SPEECH_SHOTS;
test.skip(!folder, "Set SPEECH_SHOTS to an output folder to capture.");
test.setTimeout(240_000);

const sizes = [
  ["default", { width: 1220, height: 860 }],
  ["min", { width: 740, height: 540 }],
];
const themes = ["fog", "slate"];
const hardware = ["nvidia", "cpu", "mac"];

async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(folder, `${name}.png`) });
}
async function door(page, label) {
  // Memory and On disk sit in the Advanced fold after November 13.
  const fold = page.locator("details.advanced");
  if (
    (label === "Memory" || label === "On disk") &&
    (await fold.count()) &&
    !(await fold.evaluate((node) => node.open))
  )
    await fold.locator("summary").click();
  await page
    .locator(".settings-link")
    .filter({ has: page.getByText(label, { exact: true }) })
    .first()
    .click();
}
async function settingsRoot(page) {
  await page.locator('[data-nav="settings"]').click();
  await expect(
    page.getByRole("heading", { name: "Settings", exact: true }),
  ).toBeVisible();
}

const screens = [
  ["settings", []],
  ["settings-dictation", ["Dictation"]],
  ["input", ["Dictation", "Shortcut and mic"]],
  ["cleanup", ["Dictation", "Cleanup"]],
  ["output", ["Dictation", "Paste and history"]],
  ["settings-meetings", ["Meetings"]],
  ["meeting-speech", ["Meetings", "Speech and speakers"]],
  ["notes", ["Meetings", "Notes"]],
  ["look", ["Look and startup"]],
  ["memory", ["Memory"]],
  ["disk", ["On disk"]],
  ["updates", ["Updates"]],
  ["help", ["Help"]],
];

for (const hw of hardware)
  for (const theme of themes)
    for (const [sizeName, viewport] of sizes) {
      const tag = `${hw}-${theme}-${sizeName}`;
      test(`screens ${tag}`, async ({ page }) => {
        await page.setViewportSize(viewport);
        await installDesktopFixture(page, speechProfiles(false)[hw]);
        await page.emulateMedia({ reducedMotion: "reduce" });
        await page.goto(`/?page=setup&theme=${theme}`);
        await expect(
          page.getByRole("heading", { name: "Speech", exact: true }),
        ).toBeVisible();
        await page.waitForTimeout(300);
        await shot(page, `${tag}-speech`);
        const compare = page.getByRole("button", { name: "Compare models" });
        if (await compare.count()) {
          await compare.click();
          await expect(page.getByRole("dialog")).toBeVisible();
          await page.waitForTimeout(200);
          await shot(page, `${tag}-speech-compare`);
          const kinds = page.getByRole("group", { name: "Kind of speech" });
          for (const [name, file] of [
            ["Accented English", "accented"],
            ["Talks", "talks"],
          ])
            if (await kinds.count()) {
              await kinds.getByRole("button", { name }).click();
              await shot(page, `${tag}-speech-compare-${file}`);
            }
          await page
            .getByRole("group", { name: "Show as" })
            .getByRole("button", { name: "Table" })
            .click();
          await shot(page, `${tag}-speech-compare-table`);
          await page.keyboard.press("Escape");
        }
        const info = page.locator(".speech-screen .info-button").first();
        if (await info.count()) {
          await info.click();
          await shot(page, `${tag}-speech-info`);
          await page.keyboard.press("Escape");
        }
        // SPEECH_ONLY=1 refreshes the Speech page and chart captures only.
        if (process.env.SPEECH_ONLY) return;
        const fold = page.locator("details.advanced");
        await settingsRoot(page);
        if (await fold.count()) {
          await fold.locator("summary").click();
          await shot(page, `${tag}-settings-advanced-open`);
          await fold.locator("summary").click();
        }
        await settingsRoot(page);
        await door(page, "Dictation");
        await door(page, "Cleanup");
        const advanced = page.getByRole("button", { name: /^Advanced/ });
        if (await advanced.count()) {
          await advanced.click();
          await shot(page, `${tag}-cleanup-advanced`);
          await page.keyboard.press("Escape");
        }
        for (const [name, doors] of screens) {
          await settingsRoot(page);
          for (const label of doors) await door(page, label);
          await page.waitForTimeout(250);
          await shot(page, `${tag}-${name}`);
        }
      });
      test(`first run ${tag}`, async ({ page }) => {
        test.skip(!!process.env.SPEECH_ONLY, "Speech captures only");
        await page.setViewportSize(viewport);
        await page.clock.install();
        await installDesktopFixture(page, speechProfiles(true)[hw]);
        await page.emulateMedia({ reducedMotion: "reduce" });
        await page.goto(`/?page=setup&theme=${theme}`, {
          waitUntil: "domcontentloaded",
        });
        const go = async (name = "Continue") => {
          await page.clock.runFor(5000);
          await page.getByRole("button", { name, exact: true }).click();
        };
        await go();
        await page
          .getByRole("checkbox", {
            name: "I agree to the SonoJot Terms of Use",
          })
          .check();
        await page
          .getByRole("button", { name: "Agree and continue", exact: true })
          .click();
        const balanced = page.getByRole("button", {
          name: /^Balanced/,
        });
        for (let step = 0; step < 6 && !(await balanced.count()); step++)
          await go();
        await expect(balanced.first()).toBeVisible();
        await shot(page, `${tag}-setup-models`);
        await go("Use Balanced");
        await expect(
          page.getByRole("heading", { name: "Downloads", level: 1 }),
        ).toBeVisible();
        await shot(page, `${tag}-setup-downloads`);
      });
    }
