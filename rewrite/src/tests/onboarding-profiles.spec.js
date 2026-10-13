import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture, macProfileFixture } from "./desktop-fixture.js";

const directory = new URL("../../../artifacts/onboarding/", import.meta.url);
const calls = (page, command) => page.evaluate((command) => window.__testCalls.filter((call) => call.command === command), command);
const state = (page) => page.evaluate(() => window.__testSnapshot());
async function open(page, options = {}, url = "/") {
  await page.clock.install();
  await installDesktopFixture(page, options);
  await page.goto(url);
}
async function dwell(page, milliseconds = 5000) { await page.clock.runFor(milliseconds); }
async function next(page, name = "Continue") {
  await dwell(page);
  await page.getByRole("button", { name, exact: true }).click();
}
async function screenshot(page, name) {
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(`${name}.png`, directory)) });
}

test("first-run: seven screens, dwell, background downloads and finish", async ({ page }) => {
  await open(page, { firstRun: true, modelPatches: { "sonojot-punct-v2": { state: "available", downloadedBytes: 0 } } });
  await expect(page.getByRole("heading", { name: "Welcome to SonoJot" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main navigation" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await screenshot(page, "01-welcome");
  await next(page);
  await expect(page.getByRole("heading", { name: "Your microphone", exact: true })).toBeVisible();
  await dwell(page);
  await expect(page.getByRole("meter")).toHaveAttribute("aria-valuenow", /[1-9]/);
  await screenshot(page, "02-microphone");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  expect((await page.evaluate(() => window.__testOnboarding())).microphoneActive).toBe(false);
  await screenshot(page, "03-dictation");
  await next(page);
  await expect(page.getByRole("button", { name: "Balanced Recommended" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("group", { name: "Setup profile" }).getByRole("button")).toHaveCount(3);
  await expect(page.locator(".profile-memory")).not.toContainText(/benchmark|2026|b10964|576|CC BY|\.md/i);
  await screenshot(page, "04-models");
  await next(page, "Use Balanced");
  expect((await page.evaluate(() => window.__testOnboarding().queue))[1]).toBe("sonojot-punct-v2");
  expect((await state(page)).settings.setupProfile).toBeNull();
  await screenshot(page, "05-meetings");
  await next(page);
  await screenshot(page, "06-sidebar");
  await next(page);
  await expect(page.getByRole("heading", { name: "Ready", exact: true })).toBeVisible();
  await expect(page.locator('[data-model-id="sonojot-punct-v2"]')).toContainText("Queued");
  await screenshot(page, "07-ready");
  await next(page, "Go to SonoJot");
  await expect(page.getByRole("navigation", { name: "Main navigation" })).toBeVisible();
  expect((await state(page)).settings.setupProfile).toBe("balanced");
  expect(await calls(page, "finish_onboarding")).toHaveLength(1);
  expect(await calls(page, "start_dictation")).toHaveLength(0);
  expect(await calls(page, "test_microphone")).toHaveLength(0);
});

test("Mac denied microphone: permission action, timed confirmation and reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 740, height: 540 });
  await open(page, { ...macProfileFixture(16), firstRun: true, microphoneError: "Microphone permission denied." }, "/?theme=slate");
  await next(page);
  await expect(page.getByText("Microphone permission denied.", { exact: true })).toBeVisible();
  await dwell(page);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Open System Settings", exact: true }).click();
  expect((await calls(page, "open_permission_settings"))[0].args.kind).toBe("microphone");
  await dwell(page, 5100);
  await page.getByRole("button", { name: "It's working", exact: true }).click();
  await expect(page.getByText("Input detected. You can continue.", { exact: true })).toHaveCount(0);
  await screenshot(page, "mac-microphone-denied-slate");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await page.getByRole("button", { name: "Open Accessibility settings", exact: true }).click();
  expect((await calls(page, "open_permission_settings"))[1].args.kind).toBe("accessibility");
  await expect(page.getByText(/Automatic paste is not implemented yet/)).toBeVisible();
  await screenshot(page, "mac-dictation-slate");
  expect(await page.locator(".onboarding-content").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
});

test("completed Custom users see the app; an unchanged replay writes nothing", async ({ page }) => {
  await open(page, {}, "/?page=about");
  await expect(page.getByRole("heading", { name: "About", exact: true })).toBeVisible();
  const before = (await state(page)).settings;
  await page.getByRole("button", { name: "Take the tour again" }).click();
  await next(page); await next(page); await next(page);
  await expect(page.getByRole("button", { name: "Keep my Custom models", exact: true })).toHaveAttribute("aria-pressed", "true");
  await next(page, "Keep my models");
  await next(page); await next(page); await next(page, "Go to SonoJot");
  expect((await state(page)).settings).toEqual(before);
  for (const command of ["save_settings", "confirm_onboarding_profile", "finish_onboarding", "start_model_install", "apply_setup_profile"]) {
    expect(await calls(page, command)).toHaveLength(0);
  }
});

test("early close applies recommended defaults and queues v2 after an edited choice", async ({ page }) => {
  await open(page, { firstRun: true });
  await next(page);
  await page.getByRole("combobox", { name: "Input device" }).selectOption("mic");
  await next(page);
  await page.getByRole("textbox", { name: "Keyboard shortcut" }).fill("Alt+Shift+D");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await next(page);
  await page.getByRole("button", { name: "Light", exact: true }).click();
  await next(page, "Use Light");
  await page.getByRole("button", { name: "Close window", exact: true }).click();
  const result = (await state(page)).settings;
  expect(result.setupProfile).toBe("balanced");
  expect(result.microphone).toBeNull();
  expect(result.hotkey).toBe("Ctrl+Space");
  expect(await page.evaluate(() => window.__testOnboarding())).toMatchObject({ required: false, microphoneActive: false });
  expect((await page.evaluate(() => window.__testOnboarding())).queue[1]).toBe("sonojot-punct-v2");
});

test("Models recommendation has three plain lines and no hidden evidence", async ({ page }) => {
  await open(page, { ...macProfileFixture(16), firstRun: false, settings: { ...macProfileFixture(16).settings, setupProfile: "balanced" } }, "/?page=setup");
  const reason = page.locator(".profile-memory");
  await reason.locator("summary").click();
  await expect(reason.locator("p")).toHaveCount(3);
  await expect(reason).toContainText("Apple Silicon");
  await expect(reason).toContainText("Notes loads only when you use it.");
  await expect(reason).not.toContainText(/Observed|benchmark|2026|\.md|license|CC BY|b10964/i);
  expect(await reason.locator("[title]").count()).toBe(0);
  await screenshot(page, "models-plain-recommendation");
});

test("failed confirmation stays on Models and Back does not reapply choices", async ({ page }) => {
  await open(page, { firstRun: true, microphoneLevel: 0 });
  await next(page);
  await dwell(page);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await next(page);
  await dwell(page, 10100);
  await page.getByRole("button", { name: "It's working", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await next(page);
  await page.evaluate(() => window.__testFailProfileApply("Settings could not be saved."));
  await next(page, "Use Balanced");
  await expect(page.getByRole("heading", { name: "Models", exact: true })).toBeVisible();
  await expect(page.getByRole("alert")).toContainText("Settings could not be saved.");
  await page.getByRole("button", { name: "Use Balanced", exact: true }).click();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await next(page, "Use Balanced");
  expect(await calls(page, "confirm_onboarding_profile")).toHaveLength(2);
});
