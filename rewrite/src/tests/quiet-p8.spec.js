import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import {
  installDesktopFixture,
  macProfileFixture,
  macDriftFixture,
} from "./desktop-fixture.js";
import {
  acceptDownloadTerms,
  openApp,
  openMeeting,
  openVocabularyView,
  openFold,
} from "./app-helpers.js";

const folder = new URL("../../artifacts/redesign-p8/", import.meta.url);
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const heading = (page, name) =>
  expect(
    page.getByRole("heading", { name, exact: true, level: 1 }),
  ).toBeVisible();
const primary = (page) => page.locator(".onboarding-footer .continue");
async function next(page) {
  await page.clock.runFor(5000);
  await primary(page).click();
}
async function agree(page) {
  await page
    .getByRole("checkbox", { name: "I agree to the SonoJot Terms of Use" })
    .check();
  await primary(page).click();
}
async function shot(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
async function firstRun(page, options = {}, theme = "fog") {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.clock.install();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installDesktopFixture(page, { ...options, firstRun: true });
  await page.goto(`/?theme=${theme}`, { waitUntil: "domcontentloaded" });
  await heading(page, "Welcome to SonoJot");
  return errors;
}
test.afterEach(async ({ page }, testInfo) => {
  if (testInfo.status === testInfo.expectedStatus)
    await page.goto("about:blank");
});

test("first run retains ten steps, terms, microphone ownership and download consent in paper", async ({
  page,
}) => {
  test.setTimeout(60000);
  const errors = await firstRun(page, {
    modelPatches: {
      "sonojot-punct-v3": { state: "available", downloadedBytes: 0 },
    },
    setup: { mediaImport: { state: "missing", detail: null } },
  });
  await expect(primary(page)).toBeDisabled();
  await expect(
    page.getByRole("progressbar", { name: "Setup step progress" }),
  ).toHaveAttribute("aria-valuetext", "Step 1 of 10");
  await shot(page, "01-welcome-fog.png");
  await next(page);
  await heading(page, "Terms of Use");
  await expect(primary(page)).toBeDisabled();
  await agree(page);
  await heading(page, "Your microphone");
  await page.clock.runFor(5000);
  await expect(page.getByRole("meter")).toHaveAttribute(
    "aria-valuenow",
    /[1-9]/,
  );
  await shot(page, "02-microphone-fog.png");
  await primary(page).click();
  expect(
    await page.evaluate(() => window.__testOnboarding().microphoneActive),
  ).toBe(false);
  await heading(page, "Your shortcut");
  await page
    .getByRole("textbox", { name: "Keyboard shortcut" })
    .fill("Ctrl+Alt+D");
  await page.clock.runFor(5000);
  await expect(primary(page)).toBeDisabled();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await next(page);
  await heading(page, "Dictation");
  await next(page);
  await heading(page, "Models");
  await expect(
    page.getByRole("button", { name: "Balanced Recommended" }),
  ).toHaveAttribute("aria-pressed", "true");
  await shot(page, "03-models-fog.png");
  await next(page);
  await heading(page, "Downloads");
  expect(await calls(page, "confirm_onboarding_profile")).toHaveLength(0);
  await next(page);
  const consent = page.getByRole("dialog", { name: "Before downloading" });
  await expect(consent).toBeVisible();
  expect(await calls(page, "confirm_onboarding_profile")).toHaveLength(0);
  await acceptDownloadTerms(page);
  await heading(page, "Meetings");
  expect((await calls(page, "confirm_onboarding_profile"))[0].args).toEqual({
    profile: "balanced",
    skip: [],
  });
  expect(await calls(page, "install_media_support")).toHaveLength(1);
  await next(page);
  await heading(page, "Where things are");
  await expect(page.locator(".rail-tour dt")).toHaveText([
    "Dictation",
    "Meetings",
    "Words",
    "Settings",
  ]);
  await expect(page.locator(".tour-picture .rail-item")).toHaveCount(4);
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toHaveCount(0);
  await shot(page, "04-four-item-tour.png");
  await next(page);
  await heading(page, "Ready");
  await expect(primary(page)).toBeInViewport({ ratio: 1 });
  await shot(page, "05-ready-fog.png");
  await next(page);
  await expect(
    page.getByRole("navigation", { name: "Main navigation" }),
  ).toBeVisible();
  expect((await settings(page)).hotkey).toBe("Ctrl+Alt+D");
  expect(await calls(page, "finish_onboarding")).toHaveLength(1);
  expect(await calls(page, "start_dictation")).toHaveLength(0);
  expect(errors).toEqual([]);
});

test("Mac permissions retain their helpers and can continue without permission in Slate", async ({
  page,
}) => {
  const mac = macProfileFixture(16);
  const errors = await firstRun(
    page,
    {
      ...mac,
      microphonePermission: "denied",
      microphoneError: "Microphone permission denied.",
      capabilities: { ...mac.capabilities, systemAudioStatus: "denied" },
    },
    "slate",
  );
  await next(page);
  await agree(page);
  await heading(page, "Permissions");
  await expect(
    page.getByRole("progressbar", { name: "Setup step progress" }),
  ).toHaveAttribute("aria-valuetext", "Step 3 of 11");
  await shot(page, "06-mac-permissions-slate.png");
  await page
    .getByRole("button", { name: "Allow Microphone", exact: true })
    .click();
  expect((await calls(page, "permission_assistant"))[0].args).toEqual({
    kind: "microphone",
    action: "open",
  });
  await next(page);
  await heading(page, "Your microphone");
  await page.clock.runFor(11000);
  await expect(primary(page)).toBeDisabled();
  await page.getByRole("button", { name: "It's working", exact: true }).click();
  await primary(page).click();
  await heading(page, "Your shortcut");
  expect(errors).toEqual([]);
});

test("a changed saved recommendation names current models and never switches before its download", async ({
  page,
}) => {
  const q4 = "qwen3-asr-1.7b-q4-metal";
  await openApp(page, macDriftFixture(), "/?page=setup");
  await heading(page, "Speech");
  const before = await settings(page);
  await expect(page.getByText("Custom", { exact: true })).toBeVisible();
  await expect(page.locator(`[data-model-id="${q4}"]`)).toContainText(
    "Recommended",
  );
  expect(await settings(page)).toEqual(before);
  expect(await calls(page, "start_model_install")).toHaveLength(0);
  await page.getByRole("button", { name: "Back to recommended setup" }).click();
  const sheet = page.getByRole("dialog", { name: "Back to recommended setup" });
  await expect(sheet).toContainText("Whisper Small");
  await expect(sheet).toContainText("Qwen3-ASR 1.7B (Q4 Metal)");
  await sheet.getByRole("button", { name: "Use recommended setup" }).click();
  await expect
    .poll(async () => (await calls(page, "start_model_install")).length)
    .toBe(1);
  expect((await settings(page)).asrModel).toBe(before.asrModel);
  await page.evaluate((id) => {
    window.__testLibraryProgress(id, 1.6e9, "installed");
    window.dispatchEvent(new Event("focus"));
  }, q4);
  await expect.poll(async () => (await settings(page)).asrModel).toBe(q4);
  await expect(page.getByText("Custom", { exact: true })).toHaveCount(0);
});

test("a failed recommended setup download explains the failure without changing saved models", async ({
  page,
}) => {
  test.setTimeout(60_000);
  const q4 = "qwen3-asr-1.7b-q4-metal";
  const disk =
    "Not enough disk space for the speech model and Metal runtime. Free space and try again.";
  await openApp(
    page,
    { ...macDriftFixture(), installJobErrors: { [q4]: disk } },
    "/?page=setup",
  );
  await heading(page, "Speech");
  const before = await settings(page);
  await page.getByRole("button", { name: "Back to recommended setup" }).click();
  await page
    .getByRole("dialog", { name: "Back to recommended setup" })
    .getByRole("button", { name: "Use recommended setup" })
    .click();
  await expect(page.getByRole("alert")).toHaveText(disk);
  await expect(page.getByText(disk, { exact: true })).toHaveCount(1);
  await expect(page.getByText("Custom", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Back to recommended setup" }),
  ).toBeEnabled();
  expect(await settings(page)).toEqual(before);
  expect(
    (await calls(page, "start_model_install")).map((call) => call.args.id),
  ).toEqual([q4]);
});

test("an outstanding meeting read cannot swallow a completion notification", async ({
  page,
}) => {
  await openApp(
    page,
    { meeting: { notes: "", transcript: "Saved transcript." } },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  await page.evaluate(() => {
    window.__testUpdateMeeting({ status: "transcribing" });
  });
  await expect(
    page.getByRole("button", { name: "Stop transcribing", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      const result = await invoke(command, args);
      if (command === "get_meeting" && !window.__heldRead) {
        window.__heldRead = true;
        return new Promise((resolve) => {
          window.__releaseRead = () => resolve(result);
        });
      }
      return result;
    };
    window.__testPushSnapshot({});
  });
  await page.waitForFunction(() => !!window.__releaseRead);
  await page.evaluate(() => {
    window.__testUpdateMeeting({
      status: "transcribed",
      transcript: "Completed while the read was waiting.",
      hasTranscript: true,
    });
  });
  await page.evaluate(() => window.__releaseRead());
  await expect(
    page.getByRole("button", { name: "Stop transcribing", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("textbox", { name: "Meeting transcript", exact: true }),
  ).toHaveValue("Completed while the read was waiting.");
});

test("learning remains opt-in and resetting it requires confirmation", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=vocabulary");
  const learning = await openVocabularyView(page, "Learning from your fixes");
  await learning
    .getByRole("switch", {
      name: "Learn from my corrections on this device",
      exact: true,
    })
    .click();
  expect((await settings(page)).learnFromCorrections).toBe(true);
  const entry = learning.locator('[data-learning-id="mock-learned-one"]');
  await entry.getByRole("button", { name: /grock/ }).click();
  await openFold(entry, "Source references");
  await expect(entry).toContainText("dictation-correction");
  await openFold(learning, "More learning options");
  await learning
    .getByRole("button", { name: "Reset learning…", exact: true })
    .click();
  expect(await calls(page, "reset_learning")).toHaveLength(0);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Reset learning", exact: true })
    .click();
  await expect(learning).toContainText("No learned entries.");
});
