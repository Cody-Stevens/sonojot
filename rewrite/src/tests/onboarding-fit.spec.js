// Every first-run step must fit the window: the main onboarding area never
// scrolls, and the primary action plus any required checkbox stay on screen.
// Walks the whole flow at the minimum and default window sizes (windows.rs),
// for the Windows and Mac fixtures, in Fog and Slate, and saves screenshots.
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

const directory = new URL("../../../artifacts/onboarding-review/", import.meta.url);
const sizes = [
  { name: "min", width: 740, height: 540 },
  { name: "default", width: 1220, height: 860 },
];
const platforms = {
  windows: () => ({ firstRun: true, capabilities: { platform: "windows" }, setup: { mediaImport: { state: "missing", detail: null } } }),
  mac: () => ({ ...macProfileFixture(16), firstRun: true, setup: { mediaImport: { state: "missing", detail: null } } }),
  // macOS 14.2+ before System Audio Recording is allowed; Allow grants it.
  "mac-permission": () => {
    const mac = macProfileFixture(16);
    return { ...mac, firstRun: true, setup: { mediaImport: { state: "missing", detail: null } },
      capabilities: { ...mac.capabilities, systemAudioStatus: "notDetermined" }, systemAudioAnswer: "ready" };
  },
  // Permission turned off earlier; the user defers it with Not now.
  "mac-denied": () => {
    const mac = macProfileFixture(16);
    return { ...mac, firstRun: true, setup: { mediaImport: { state: "missing", detail: null } },
      capabilities: { ...mac.capabilities, systemAudioStatus: "denied" } };
  },
};
const order = ["welcome", "terms", "microphone", "shortcut", "dictation", "models", "meetings", "extras", "where", "ready"];

async function screenshot(page, folder, name) {
  const target = new URL(`${folder}/`, directory);
  await mkdir(target, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(`${name}.png`, target)) });
}

/** Layout facts for the current step, measured in the page. */
function measure(page) {
  return page.evaluate(() => {
    const content = document.querySelector(".onboarding-content");
    const box = content.getBoundingClientRect();
    const outside = [];
    for (const element of content.querySelectorAll(
      "button, input, select, textarea, a, summary, label, [role=progressbar], h1",
    )) {
      // Links inside the bounded Terms panel scroll with that panel.
      if (element.closest("[data-bounded-scroll]")) continue;
      if (!element.getClientRects().length) continue;
      const rect = element.getBoundingClientRect();
      if (rect.top < box.top - 1 || rect.bottom > box.bottom + 1 || rect.left < box.left - 1 || rect.right > box.right + 1)
        outside.push(`${element.tagName.toLowerCase()} "${(element.textContent || element.getAttribute("aria-label") || "").trim().slice(0, 40)}"`);
    }
    const primary = document.querySelector(".onboarding-footer .continue").getBoundingClientRect();
    const scroller = document.scrollingElement;
    return {
      step: document.querySelector(".onboarding").dataset.step,
      overflowY: content.scrollHeight - content.clientHeight,
      overflowX: content.scrollWidth - content.clientWidth,
      documentOverflow: scroller.scrollHeight - window.innerHeight,
      primaryVisible: primary.top >= 0 && primary.bottom <= window.innerHeight && primary.right <= window.innerWidth,
      outside,
    };
  });
}

async function expectFits(page, label) {
  const facts = await measure(page);
  if (process.env.FIT_LOG) console.log(label, JSON.stringify(facts));
  expect.soft(facts.overflowY, `${label}: main area vertical overflow`).toBeLessThanOrEqual(1);
  expect.soft(facts.overflowX, `${label}: main area horizontal overflow`).toBeLessThanOrEqual(1);
  expect.soft(facts.documentOverflow, `${label}: window overflow`).toBeLessThanOrEqual(0);
  expect.soft(facts.primaryVisible, `${label}: primary action on screen`).toBe(true);
  expect.soft(facts.outside, `${label}: controls outside the main area`).toEqual([]);
}

async function acceptConsentIfShown(page) {
  const dialog = page.getByRole("dialog", { name: "Before downloading" });
  if (!(await dialog.isVisible())) return;
  for (const box of await dialog.getByRole("checkbox").all()) await box.check();
  await dialog.getByRole("button", { name: "Agree and continue" }).click();
  await expect(dialog).toHaveCount(0);
}

for (const [platform, options] of Object.entries(platforms)) {
  for (const size of sizes) {
    for (const theme of ["fog", "slate"]) {
      test(`${platform} ${size.name} ${theme}: every step fits without scrolling`, async ({ page }) => {
        test.setTimeout(60000);
        const folder = `${platform}-${theme}-${size.name}`;
        await page.setViewportSize({ width: size.width, height: size.height });
        await page.clock.install();
        await installDesktopFixture(page, options());
        await page.goto(`/?theme=${theme}`);
        const section = page.locator(".onboarding");
        const visited = [];
        await expect(section).toHaveAttribute("data-step", order[0]);
        for (let index = 0; index < order.length + 2; index++) {
          const id = await section.getAttribute("data-step");
          await page.clock.runFor(5000);
          await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
          visited.push(id);
          const name = `${String(index + 1).padStart(2, "0")}-${id}`;
          await expectFits(page, `${folder} ${id}`);
          if (id === "terms") {
            const agree = page.getByRole("checkbox", { name: "I agree to the SonoJot Terms of Use" });
            await expect(agree).toBeInViewport({ ratio: 1 });
            await expect(page.locator(".legal-links").getByRole("link", { name: "Privacy Notice" })).toBeInViewport({ ratio: 1 });
            await expect(page.getByRole("region", { name: "Full SonoJot Terms of Use" })).toBeInViewport();
            await screenshot(page, folder, name);
            await agree.check();
          } else {
            await screenshot(page, folder, name);
          }
          if (id === "extras") {
            // Both optional downloads report progress in place, still fitting.
            await page.getByRole("button", { name: "Review license and download" }).click();
            await acceptConsentIfShown(page);
            await expect(page.getByText("FFmpeg is installed.")).toBeVisible();
            await page.getByRole("button", { name: "Download", exact: true }).click();
            await acceptConsentIfShown(page);
            await expect(page.getByRole("region", { name: "Speaker labels" }).getByRole("status")).toHaveText(/Queued|Downloading/);
            await expectFits(page, `${folder} extras started`);
            await screenshot(page, folder, `${name}-started`);
          }
          const allow = page.getByRole("button", { name: "Allow system audio", exact: true });
          if (id === "meetings" && (await allow.isVisible())) {
            await allow.click();
            await expect(page.getByRole("status", { name: "System audio permission" })).toContainText("Ready");
            await expectFits(page, `${folder} meetings allowed`);
          }
          const later = page.getByRole("button", { name: "Not now", exact: true });
          if (id === "meetings" && (await later.isVisible())) {
            await later.click();
            await expectFits(page, `${folder} meetings deferred`);
            await screenshot(page, folder, `${name}-deferred`);
          }
          const primary = page.locator(".onboarding-footer .continue");
          await expect(primary).toBeEnabled();
          await expect(primary).toBeInViewport({ ratio: 1 });
          if (id === "ready") break;
          await primary.click();
          if (id === "models") await acceptConsentIfShown(page);
          await expect(section).not.toHaveAttribute("data-step", id);
        }
        expect(visited).toEqual(order);
      });
    }
  }
}
