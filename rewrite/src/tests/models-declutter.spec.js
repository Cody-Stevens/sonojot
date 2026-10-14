// October 14 Models page clarity pass: review captures of every state of the
// "Speech and text models" card at 1220x860, in Fog and Slate, on the Windows
// and Mac fixtures. DECLUTTER_PREFIX=before captures the old build for
// comparison; the default prefix is "after".
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { openApp, openModelLibrary, openPerformance } from "./app-helpers.js";
import {
  installDesktopFixture,
  macDriftFixture,
  macProfileFixture,
} from "./desktop-fixture.js";

const prefix = process.env.DECLUTTER_PREFIX ?? "after";
const directory = new URL("../../../artifacts/models-declutter/", import.meta.url);

async function shot(page, name) {
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: fileURLToPath(new URL(`${prefix}-${name}.png`, directory)),
  });
}

/** Scroll the main page so `locator` sits at the top of the view. */
async function bringToTop(page, locator) {
  await locator.evaluate((element) => element.scrollIntoView({ block: "start" }));
  await page.waitForTimeout(150);
}

const card = (page) =>
  page.getByRole("region", { name: "Speech and text models", exact: true });
const profileButton = (page, name) =>
  page
    .getByRole("group", { name: "Setup profile", exact: true })
    .getByRole("button", { name: new RegExp(`^${name}`) });

function windowsFixture(extra = {}) {
  return { ...extra };
}
function macFixture(extra = {}) {
  const fixture = macProfileFixture(16);
  return {
    ...fixture,
    ...extra,
    settings: { ...fixture.settings, setupProfile: "custom", ...extra.settings },
    modelPatches: { ...fixture.modelPatches, ...extra.modelPatches },
  };
}
const platforms = {
  windows: windowsFixture,
  mac: macFixture,
};
// The connected OpenRouter account that shows the Meeting transcription row.
const online = {
  aiSettings: { connected: true, speechModel: "mock/audio", model: "" },
};

for (const [platform, fixture] of Object.entries(platforms)) {
  for (const theme of ["fog", "slate"]) {
    const tag = `${platform}-${theme}`;
    test(`declutter review: profiles and pickers (${tag})`, async ({ page }) => {
      await openApp(page, fixture(online), `/?page=setup&theme=${theme}`);
      await expect(card(page)).toBeVisible();
      await bringToTop(page, card(page));
      for (const name of ["Accurate", "Balanced", "Light", "Custom"]) {
        await profileButton(page, name).click();
        await expect(profileButton(page, name)).toHaveAttribute("aria-pressed", "true");
        await page.waitForTimeout(150);
        await shot(page, `profile-${name.toLowerCase()}-${tag}`);
      }
      for (const [role, label] of [
        ["speech", "Speech"],
        ["cleanup", "Cleanup"],
        ["notes", "Notes"],
        ["meeting", "Meeting transcription"],
      ]) {
        const trigger = page.getByRole("button", { name: `${label} model`, exact: true });
        await bringToTop(page, card(page));
        await trigger.scrollIntoViewIfNeeded();
        await trigger.click();
        await expect(page.getByRole("dialog", { name: `${label} models`, exact: true })).toBeVisible();
        await page.waitForTimeout(150);
        await shot(page, `picker-${role}-${tag}`);
        await page.keyboard.press("Escape");
      }
    });

    test(`declutter review: downloads, performance and details (${tag})`, async ({ page }) => {
      await openApp(page, fixture(), `/?page=setup&theme=${theme}`);
      await expect(card(page)).toBeVisible();
      const library = await openModelLibrary(page);
      await bringToTop(page, library);
      await shot(page, `manage-downloads-${tag}`);
      const row = library.locator(".entry, [data-model-id]").filter({ has: page.locator("summary") }).first();
      const details = row.locator("summary").filter({ hasText: /^\s*Details\s*$/ }).first();
      await details.click();
      await bringToTop(page, row);
      await shot(page, `row-details-${tag}`);
      await library.getByRole("tab", { name: "Get more", exact: true }).click();
      await bringToTop(page, library);
      await shot(page, `get-more-${tag}`);
      const performance = await openPerformance(page);
      await bringToTop(page, performance);
      await shot(page, `performance-${tag}`);
    });

    test(`declutter review: download in progress (${tag})`, async ({ page }) => {
      const id = platform === "mac" ? "qwen3-asr-1.7b-q4-metal" : "qwen3-asr-1.7b";
      await openApp(
        page,
        fixture({
          modelPatches: {
            [id]: { state: "downloading", downloadedBytes: 700e6, totalBytes: 1638e6, stage: "model files" },
          },
        }),
        `/?page=setup&theme=${theme}`,
      );
      const download = page.locator(`section.download[data-model-id="${id}"]`).first();
      await expect(download).toBeVisible();
      await bringToTop(page, card(page));
      await shot(page, `download-${tag}`);
      await download.evaluate((element) => element.scrollIntoView({ block: "center" }));
      await page.waitForTimeout(150);
      await shot(page, `download-strip-${tag}`);
    });

    test(`declutter review: whole card (${tag})`, async ({ page }) => {
      await page.setViewportSize({ width: 1220, height: 2400 });
      await openApp(page, fixture(online), `/?page=setup&theme=${theme}`);
      await expect(card(page)).toBeVisible();
      for (const name of ["Balanced", "Custom"]) {
        await profileButton(page, name).click();
        await page.waitForTimeout(150);
        await page.evaluate(() => document.fonts.ready);
        await mkdir(directory, { recursive: true });
        await card(page).screenshot({
          path: fileURLToPath(
            new URL(`${prefix}-card-${name.toLowerCase()}-${tag}.png`, directory),
          ),
        });
      }
    });

    test(`declutter review: drift (${tag})`, async ({ page }) => {
      const options =
        platform === "mac"
          ? macDriftFixture()
          : { settings: { setupProfile: "balanced", asrModel: "moonshine-medium", cleanupModel: "local-rules", notesModel: "lfm2.5-1.2b" } };
      await openApp(page, options, `/?page=setup&theme=${theme}`);
      await expect(card(page)).toContainText("now uses");
      await bringToTop(page, card(page));
      await shot(page, `drift-${tag}`);
    });

    test(`declutter review: first-run Models step (${tag})`, async ({ page }) => {
      await openOnboardingModels(
        page,
        platform === "mac" ? macProfileFixture(16) : {},
        theme,
      );
      await shot(page, `onboarding-models-${tag}`);
    });
  }
}

/** Walk first-run setup to its Models step (the profile row). */
async function openOnboardingModels(page, options, theme) {
  await page.clock.install();
  await installDesktopFixture(page, { ...options, firstRun: true });
  await page.goto(`/?theme=${theme}`);
  const primary = page.locator(".onboarding-footer button.continue");
  const profiles = page.getByRole("group", { name: "Setup profile", exact: true });
  for (let step = 0; step < 8 && !(await profiles.isVisible()); step += 1) {
    const terms = page.getByRole("checkbox", {
      name: "I agree to the SonoJot Terms of Use",
    });
    if ((await terms.isVisible()) && !(await terms.isChecked())) await terms.check();
    await page.clock.runFor(11_000);
    if (!(await primary.isEnabled())) {
      const working = page.getByRole("button", { name: "It's working", exact: true });
      if (await working.isVisible()) await working.click();
    }
    await primary.click();
    await page.waitForTimeout(100);
  }
  await expect(profiles).toBeVisible();
  await page.clock.runFor(11_000);
}
