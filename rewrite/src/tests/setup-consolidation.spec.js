// October 14 setup in fewer steps: review captures of every first-run step at
// the default window size, on the Windows and emulated macOS fixtures.
// SETUP_PREFIX=before captures the old build for comparison; the default
// prefix is "after". Captures go to artifacts/setup-consolidation/.
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

const prefix = process.env.SETUP_PREFIX ?? "after";
const directory = new URL("../../../artifacts/setup-consolidation/", import.meta.url);
const platforms = {
  windows: () => ({ firstRun: true, capabilities: { platform: "windows" },
    setup: { mediaImport: { state: "missing", detail: null } } }),
  // macOS 14.2+ before any permission is answered.
  mac: () => {
    const mac = macProfileFixture(16);
    return { ...mac, firstRun: true, setup: { mediaImport: { state: "missing", detail: null } },
      capabilities: { ...mac.capabilities, systemAudioStatus: "notDetermined" },
      systemAudioAnswer: "ready", microphonePermission: "notDetermined", microphoneAnswer: "authorized" };
  },
};

async function shot(page, name) {
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(`${prefix}-${name}.png`, directory)) });
}

async function acceptConsentIfShown(page) {
  const dialog = page.getByRole("dialog", { name: "Before downloading" });
  if (!(await dialog.isVisible())) return false;
  for (const box of await dialog.getByRole("checkbox").all()) await box.check();
  await dialog.getByRole("button", { name: "Agree and continue" }).click();
  await expect(dialog).toHaveCount(0);
  return true;
}

for (const [platform, options] of Object.entries(platforms)) {
  test(`${platform}: capture every setup step`, async ({ page }) => {
    test.setTimeout(90000);
    await page.clock.install();
    await installDesktopFixture(page, options());
    await page.goto("/?theme=fog");
    const section = page.locator(".onboarding");
    await expect(section).toHaveAttribute("data-step", "welcome");
    for (let index = 0; index < 14; index++) {
      const id = await section.getAttribute("data-step");
      await page.clock.runFor(5000);
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const name = `${platform}-${String(index + 1).padStart(2, "0")}-${id}`;
      if (id === "downloads") {
        // Show the consent dialog that covers every selected download.
        await shot(page, name);
        // The old build had per-row buttons; the new one starts from the footer.
        await page.getByRole("button", { name: /^Download (all|selected|and continue)/ }).click();
        const dialog = page.getByRole("dialog", { name: "Before downloading" });
        if (await dialog.isVisible()) await shot(page, `${name}-consent`);
        await acceptConsentIfShown(page);
        if ((await section.getAttribute("data-step")) !== "downloads") {
          await page.getByRole("button", { name: "Back", exact: true }).click();
          await expect(section).toHaveAttribute("data-step", "downloads");
        }
        await page.clock.runFor(5000);
        await shot(page, `${name}-started`);
      } else {
        await shot(page, name);
      }
      if (id === "terms")
        await page.getByRole("checkbox", { name: "I agree to the SonoJot Terms of Use" }).check();
      if (id === "permissions") {
        const all = page.getByRole("button", { name: "Allow all", exact: true });
        if (await all.isVisible()) {
          await all.click();
          await page.clock.runFor(2000);
          await shot(page, `${name}-allowed`);
        }
      }
      if (id === "extras") {
        await page.getByRole("button", { name: "Review license and download" }).click();
        await acceptConsentIfShown(page);
        await page.getByRole("button", { name: "Download", exact: true }).click();
        await acceptConsentIfShown(page);
        await shot(page, `${name}-started`);
      }
      const later = page.getByRole("button", { name: "Not now", exact: true });
      if (id === "meetings" && (await later.isVisible())) await later.click();
      if (id === "ready") break;
      const primary = page.locator(".onboarding-footer .continue");
      await expect(primary).toBeEnabled();
      await primary.click();
      if (id === "models") await acceptConsentIfShown(page);
      await expect(section).not.toHaveAttribute("data-step", id);
    }
  });
}
