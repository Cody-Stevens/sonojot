// Guards the shared first-run helper that other specs rely on. The flow itself
// is covered step by step in onboarding-profiles.spec.js and onboarding-fit.spec.js.
import { expect, test } from "@playwright/test";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";
import { expectAppShell, mainNavigation, openApp } from "./app-helpers.js";

const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);

test("openApp walks first-run setup and Terms to the app on Windows", async ({
  page,
}) => {
  await openApp(page, { firstRun: true }, "/?page=dictation");
  await expect(mainNavigation(page)).toBeVisible();
  const saved = await settings(page);
  expect(saved.setupProfile).toBe("balanced");
  expect(saved.termsAcceptedVersion).toBeGreaterThanOrEqual(1);
  expect(saved.termsAcceptedAt).toBeTruthy();
});

test("openApp answers the Mac system-audio step with Not now", async ({
  page,
}) => {
  const mac = macProfileFixture(16);
  await openApp(page, {
    ...mac,
    firstRun: true,
    capabilities: { ...mac.capabilities, systemAudioStatus: "notDetermined" },
  });
  await expect(mainNavigation(page)).toBeVisible();
  expect((await settings(page)).setupProfile).toBe("balanced");
});

test("expectAppShell explains a spec that lands on first-run setup", async ({
  page,
}) => {
  await installDesktopFixture(page, { firstRun: true });
  await page.goto("/");
  await expect(expectAppShell(page)).rejects.toThrow(
    /Terms or first-run setup is covering the app/,
  );
});
