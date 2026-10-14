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

async function agree(page) {
  await expect(page.getByRole("heading", { name: "Terms of Use", exact: true })).toBeVisible();
  await page.getByRole("checkbox", { name: "I agree to the SonoJot Terms of Use" }).check();
  await page.getByRole("button", { name: "Agree and continue", exact: true }).click();
}
const heading = (page, name) => expect(page.getByRole("heading", { name, exact: true, level: 1 })).toBeVisible();

test("first-run: ten screens, dwell, background downloads, extras and finish", async ({ page }) => {
  await open(page, { firstRun: true, modelPatches: { "sonojot-punct-v2": { state: "available", downloadedBytes: 0 } },
    setup: { mediaImport: { state: "missing", detail: null } } });
  await expect(page.getByRole("heading", { name: "Welcome to SonoJot" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main navigation" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await expect(page.getByRole("progressbar", { name: "Setup step progress" })).toHaveAttribute("aria-valuetext", "Step 1 of 10");
  await screenshot(page, "01-welcome");
  await next(page);
  await expect(page.getByRole("button", { name: "Agree and continue", exact: true })).toBeDisabled();
  await agree(page);
  await heading(page, "Your microphone");
  await dwell(page);
  await expect(page.getByRole("meter")).toHaveAttribute("aria-valuenow", /[1-9]/);
  await screenshot(page, "02-microphone");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  expect((await page.evaluate(() => window.__testOnboarding())).microphoneActive).toBe(false);
  await heading(page, "Your shortcut");
  await expect(page.getByRole("textbox", { name: "Keyboard shortcut" })).toBeVisible();
  await screenshot(page, "03-shortcut");
  await next(page);
  await heading(page, "Dictation");
  await expect(page.getByText("When you finish speaking, SonoJot pastes the text where your cursor is.")).toBeVisible();
  await screenshot(page, "04-dictation");
  await next(page);
  await expect(page.getByRole("button", { name: "Balanced Recommended" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("group", { name: "Setup profile" }).getByRole("button")).toHaveCount(3);
  await expect(page.locator(".profile-memory")).not.toContainText(/benchmark|2026|b10964|576|CC BY|\.md/i);
  await expect(page.getByText("Later, in Models, you can change these or add online OpenRouter models.")).toBeVisible();
  await screenshot(page, "05-models");
  await next(page, "Use Balanced");
  expect((await page.evaluate(() => window.__testOnboarding().queue))[1]).toBe("sonojot-punct-v2");
  expect((await state(page)).settings.setupProfile).toBeNull();
  await heading(page, "Meetings");
  await expect(page.locator(".meetings-copy em.key")).toHaveCount(3);
  await screenshot(page, "06-meetings");
  await next(page);
  await heading(page, "Meeting extras");
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Download", exact: true }).click();
  await expect(page.getByRole("region", { name: "Speaker labels" }).getByRole("status")).toHaveText("Queued. It downloads after your models.");
  expect(await page.evaluate(() => window.__testOnboarding().queue)).toContain("nemotron-3-diarization");
  expect(await calls(page, "start_model_install")).toHaveLength(0);
  await screenshot(page, "07-extras");
  await next(page);
  await heading(page, "Where things are");
  const notes = page.getByRole("list", { name: "Sidebar sections" });
  await expect(notes.getByRole("listitem")).toHaveCount(9);
  await expect(notes).toContainText("Models: Profiles, downloads and online models");
  await expect(page.getByRole("navigation", { name: "Main navigation" })).toHaveCount(0);
  await screenshot(page, "08-sidebar");
  await next(page);
  await expect(page.getByRole("heading", { name: "Ready", exact: true })).toBeVisible();
  await expect(page.locator('[data-model-id="nemotron-3-diarization"]')).toContainText("Queued");
  await expect(page.locator('[data-model-id="sonojot-punct-v2"]')).toContainText("Queued");
  await screenshot(page, "09-ready");
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
  await agree(page);
  await expect(page.getByText("Microphone permission denied.", { exact: true })).toBeVisible();
  await dwell(page);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Open System Settings", exact: true }).click();
  expect((await calls(page, "open_permission_settings"))[0].args.kind).toBe("microphone");
  await dwell(page, 5100);
  await page.getByRole("button", { name: "It's working", exact: true }).click();
  await expect(page.getByText("Input detected. You can continue.", { exact: true })).toHaveCount(0);
  // The denied state, with its error and three actions, still fits at the minimum size.
  expect(await page.locator(".onboarding-content").evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(1);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeInViewport({ ratio: 1 });
  await screenshot(page, "mac-microphone-denied-slate");
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await next(page);
  await expect(page.getByText(/needs Accessibility access/)).toBeVisible();
  await page.getByRole("button", { name: "Grant Accessibility", exact: true }).click();
  expect((await calls(page, "permission_assistant"))[0].args).toEqual({ kind: "accessibility", action: "open" });
  await expect(page.getByText(/drag the icon from the SonoJot helper/)).toBeVisible();
  // After one try, the reset sits beside Grant and the step still fits.
  await expect(page.getByText(/already switched on but paste still doesn't work, reset it/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Reset and ask again", exact: true })).toBeInViewport({ ratio: 1 });
  await screenshot(page, "mac-dictation-slate");
  expect(await page.locator(".onboarding-content").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  expect(await page.locator(".onboarding-content").evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(1);
  await page.getByRole("button", { name: "Reset and ask again", exact: true }).click();
  expect((await calls(page, "permission_reset"))[0].args).toEqual({ kind: "accessibility" });
});

test("Mac permission helper: drag icon, copy from the spec and the reset offer", async ({ page }) => {
  // permissions.rs native::WIDTH and HEIGHT.
  await page.setViewportSize({ width: 520, height: 172 });
  for (const theme of ["fog", "slate"]) {
    await open(page, { ...macProfileFixture(16) }, `/?view=permission&kind=accessibility&theme=${theme}`);
    await expect(page.getByText("Allow SonoJot to paste")).toBeVisible();
    await expect(page.getByRole("img", { name: /SonoJot app icon/ })).toHaveAttribute("draggable", "true");
    const reset = page.getByRole("button", { name: "Reset and ask again", exact: true });
    await expect(reset).toHaveCount(0);
    await expect(page.getByText(/remove it with/)).toHaveCount(0);
    await page.clock.runFor(1000);
    await screenshot(page, `mac-permission-helper-start-${theme}`);
    await page.clock.runFor(5100);
    await expect(page.getByText("Already on but not working?")).toBeVisible();
    await expect(reset).toBeVisible();
    expect(await page.locator(".helper-sheet").evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
    await screenshot(page, `mac-permission-helper-${theme}`);
    await reset.click();
    expect((await calls(page, "permission_reset")).at(-1).args).toEqual({ kind: "accessibility" });
    await page.getByRole("button", { name: "Close", exact: true }).click();
    expect((await calls(page, "permission_assistant")).at(-1).args).toEqual({ kind: "accessibility", action: "close" });
  }
  // The longer system audio copy fits the same sheet; a failed reset shows why.
  const fixture = macProfileFixture(16);
  await open(page, { ...fixture, permissionResetError: "macOS could not reset the permission.",
    capabilities: { ...fixture.capabilities, systemAudio: false, systemAudioStatus: "denied" } },
    "/?view=permission&kind=system_audio&theme=fog");
  await page.clock.runFor(6100);
  await expect(page.getByText("Already on but not recording?")).toBeVisible();
  expect(await page.locator(".helper-sheet").evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
  await screenshot(page, "mac-permission-helper-system-audio-fog");
  await page.getByRole("button", { name: "Reset and ask again", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveText("macOS could not reset the permission.");
  expect(await page.locator(".helper-sheet").evaluate((el) => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
});

test("completed Custom users see the app; an unchanged replay writes nothing", async ({ page }) => {
  await open(page, {}, "/?page=about");
  await expect(page.getByRole("heading", { name: "About", exact: true })).toBeVisible();
  const before = (await state(page)).settings;
  await page.getByRole("button", { name: "Take the tour again" }).click();
  await next(page);
  await next(page, "Agree and continue"); // already accepted: the tour keeps the box ticked
  await next(page); await next(page); await next(page);
  await expect(page.getByRole("button", { name: "Custom Keep my models", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByRole("group", { name: "Setup profile" }).getByRole("button")).toHaveCount(4);
  expect(await page.locator(".onboarding-content").evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThanOrEqual(1);
  await next(page, "Keep my models");
  await next(page); await next(page); await next(page); await next(page, "Go to SonoJot");
  expect((await state(page)).settings).toEqual(before);
  for (const command of ["save_settings", "confirm_onboarding_profile", "finish_onboarding", "start_model_install", "apply_setup_profile"]) {
    expect(await calls(page, command)).toHaveLength(0);
  }
});

test("early close applies recommended defaults and queues v2 after an edited choice", async ({ page }) => {
  await open(page, { firstRun: true });
  await next(page);
  await agree(page);
  await page.getByRole("combobox", { name: "Input device" }).selectOption("mic");
  await next(page);
  await page.getByRole("textbox", { name: "Keyboard shortcut" }).fill("Alt+Shift+D");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await next(page);
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
  await agree(page);
  await dwell(page);
  await expect(page.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await next(page, "Agree and continue");
  await dwell(page, 10100);
  await page.getByRole("button", { name: "It's working", exact: true }).click();
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await next(page);
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
