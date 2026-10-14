// October 14 field report (0.28.3, 16 GB Apple Silicon): Balanced was saved
// on 0.27.1 with Whisper Small. 0.28 recommends Qwen Q4 Metal for Balanced, so
// the card said "In use" and listed Qwen while dictation used Whisper Small.
// A profile card now says In use only when the models match, names the change
// and offers one switch that downloads first and never runs by itself.
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { openApp } from "./app-helpers.js";
import { macDriftFixture } from "./desktop-fixture.js";

// Tall enough that the whole card fits in the page for review captures.
test.use({ viewport: { width: 1220, height: 1100 } });

const q4 = "qwen3-asr-1.7b-q4-metal";
const disk =
  "Not enough disk space for the speech model and Metal runtime. Free space and try again.";
const changed =
  "Balanced now uses Qwen3-ASR 1.7B (Q4 Metal) for speech and Qwen 3 4B for notes. You're using Whisper Small · Metal and LFM 2.5.";

async function screenshot(page, name) {
  const directory = new URL("../../../artifacts/profile-drift/", import.meta.url);
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await card(page).screenshot({
    path: fileURLToPath(new URL(`${name}.png`, directory)),
  });
}

const card = (page) =>
  page.getByRole("region", { name: "Speech and text models", exact: true });
const details = (page) =>
  page.getByRole("group", { name: "Selected profile details", exact: true });
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const installs = (page) =>
  page.evaluate(() =>
    window.__testCalls.filter((call) => call.command === "start_model_install"),
  );
const switchButton = (page) =>
  card(page).getByRole("button", { name: /^Switch to Balanced/ });

async function open(page, options = {}, theme = "fog") {
  await openApp(page, { ...macDriftFixture(), ...options }, `/?page=setup&theme=${theme}`);
  await expect(page.getByLabel("Computer hardware")).toContainText("Apple Silicon");
}

for (const theme of ["fog", "slate"]) {
  test(`a changed Balanced recommendation is named, not shown as In use (${theme})`, async ({ page }) => {
    await open(page, {}, theme);
    const before = await settings(page);
    await expect(
      page
        .getByRole("group", { name: "Setup profile", exact: true })
        .getByRole("button", { name: /^Balanced/ }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(card(page)).not.toContainText("In use");
    await expect(details(page).getByRole("group", { name: "Speech" })).toHaveText(
      "Speech: Qwen3-ASR 1.7B (Q4 Metal) · Not installed",
    );
    await expect(details(page).getByRole("group", { name: "Notes" })).toHaveText(
      "Notes: Qwen 3 4B · Installed",
    );
    await expect(card(page)).toContainText(changed);
    await expect(switchButton(page)).toHaveText("Switch to Balanced · 1.6 GB download");
    await expect(switchButton(page)).toBeEnabled();
    await screenshot(page, `after-changed-${theme}`);
    // Showing the card never downloads or rewrites the saved choices.
    expect(await installs(page)).toHaveLength(0);
    expect(await settings(page)).toEqual(before);
  });
}

test("Switch to Balanced downloads the missing speech model, then switches", async ({ page }) => {
  await open(page);
  await switchButton(page).click();
  const download = card(page).getByRole("region", { name: "Model download" });
  await expect(download).toHaveAttribute("data-model-id", q4);
  await expect(download).toContainText("Downloading");
  // Nothing switches while the download runs.
  expect((await settings(page)).asrModel).toBe("whisper-small-metal");
  await page.evaluate((id) => {
    const model = window.__testLibrary().models.find((entry) => entry.id === id);
    window.__testLibraryProgress(id, model.totalBytes, "installed");
    window.dispatchEvent(new Event("focus"));
  }, q4);
  await expect.poll(async () => (await settings(page)).asrModel).toBe(q4);
  expect(await settings(page)).toMatchObject({
    setupProfile: "balanced",
    cleanupModel: "local-rules",
    notesModel: "qwen3-4b-instruct-2507",
  });
  await expect(card(page).locator(".in-use")).toHaveText("In use");
  await expect(card(page)).not.toContainText("now uses");
  await expect(details(page).getByRole("group", { name: "Speech" })).toHaveText(
    "Speech: Qwen3-ASR 1.7B (Q4 Metal) · Installed",
  );
  expect((await installs(page)).map((call) => call.args.id)).toEqual([q4]);
  await screenshot(page, "after-switched-fog");
});

test("a disk-space failure is shown plainly on the card and nothing switches", async ({ page }) => {
  await open(page, { installJobErrors: { [q4]: disk } });
  const before = await settings(page);
  await switchButton(page).click();
  const failure = card(page).locator(".profile-failure");
  await expect(failure).toHaveText(disk);
  await expect(failure).toHaveAttribute("role", "alert");
  // Once, beside the action: no generic banner and no second copy in the
  // download row, which keeps its Failed state and Retry.
  await expect(page.locator(".error-banner")).toHaveCount(0);
  await expect(page.getByText(disk)).toHaveCount(1);
  const download = card(page).getByRole("region", { name: "Model download" });
  await expect(download).toContainText("Failed");
  await expect(download.getByRole("button", { name: "Retry", exact: true })).toBeEnabled();
  await expect(switchButton(page)).toBeEnabled();
  await expect(card(page)).toContainText(changed);
  expect(await settings(page)).toEqual(before);
  await screenshot(page, "after-disk-space-fog");
  // Another tab clears it; the failed row then carries the message itself.
  await page
    .getByRole("group", { name: "Setup profile", exact: true })
    .getByRole("button", { name: "Light", exact: true })
    .click();
  await expect(card(page).locator(".profile-failure")).toHaveCount(0);
  await expect(download).toContainText(disk);
});

test("matching models say In use; other tabs offer their ordinary Use action", async ({ page }) => {
  const fixture = macDriftFixture();
  await open(page, {
    settings: { ...fixture.settings, asrModel: q4, notesModel: "qwen3-4b-instruct-2507" },
    libraryModels: fixture.libraryModels.map((model) =>
      model.id === q4 ? { ...model, state: "installed", downloadedBytes: model.diskBytes } : model,
    ),
  });
  await expect(card(page).locator(".in-use")).toHaveText("In use");
  await expect(details(page).getByRole("group", { name: "Speech" })).toHaveText(
    "Speech: Qwen3-ASR 1.7B (Q4 Metal) · Installed",
  );
  await expect(switchButton(page)).toHaveCount(0);
  await page
    .getByRole("group", { name: "Setup profile", exact: true })
    .getByRole("button", { name: "Light", exact: true })
    .click();
  await expect(card(page).locator(".in-use")).toHaveCount(0);
  await expect(card(page).getByRole("button", { name: "Use Light", exact: true })).toBeEnabled();
  await expect(details(page).getByRole("group", { name: "Speech" })).toHaveText(
    "Speech: Whisper Small · Metal · Installed",
  );
});
