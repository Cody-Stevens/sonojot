import { expect, test } from "@playwright/test";
import {
  goToPage,
  openApp,
  openFold,
  openMeeting,
  openMeetingMenu,
  openModelLibrary,
} from "./app-helpers.js";

const diarizationId = "nemotron-3-diarization";
const settingRow = (page) =>
  page.getByRole("group", { name: "Label speakers", exact: true });
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );

const titlesLabel = "Name meetings automatically";
// Meeting defaults sit in a fold on the Start a meeting page.
async function openDefaults(page) {
  await page
    .getByRole("group", { name: "Meetings" })
    .getByRole("button", { name: "Start a meeting", exact: true })
    .click();
  return openFold(page, "Meeting defaults");
}

async function startPage(page, options = {}, theme = "fog") {
  await openApp(page, options, `/?page=meetings&theme=${theme}`);
  await openDefaults(page);
  await expect(page.getByRole("switch", { name: titlesLabel })).toBeEnabled();
  await expect
    .poll(async () => (await calls(page, "get_model_library")).length)
    .toBeGreaterThan(0);
}

for (const theme of ["fog", "slate"]) {
  test(`speaker labels sit below titles, install through library polling, then toggle in ${theme}`, async ({
    page,
  }, testInfo) => {
    await startPage(page, {}, theme);
    const row = settingRow(page);
    const before = await settings(page);
    await expect(row).toHaveAccessibleDescription(
      "Downloads Nemotron 3 Diarization (104 MB).",
    );
    expect(
      await row.evaluate(
        (element) => element.previousElementSibling?.textContent,
      ),
    ).toContain(titlesLabel);
    await expect(row.getByRole("switch")).toHaveCount(0);
    await page.locator(".meeting-fold").screenshot({
      path: testInfo.outputPath(`speakers-install-${theme}.png`),
    });
    await row.getByRole("button", { name: "Install", exact: true }).click();
    await expect(
      row.getByRole("button", { name: "Installing…", exact: true }),
    ).toBeDisabled();
    expect(
      (await calls(page, "start_model_install")).map((call) => call.args),
    ).toEqual([{ id: diarizationId }]);
    expect(await settings(page)).toEqual(before);
    await page.evaluate(
      (id) => window.__testLibraryProgress(id, 103871648, "verifying"),
      diarizationId,
    );
    await expect
      .poll(async () => (await calls(page, "get_model_library")).length)
      .toBeGreaterThan(1);
    await expect(
      row.getByRole("button", { name: "Installing…", exact: true }),
    ).toBeDisabled();
    // No focus or snapshot refresh shortcut: completion must arrive through polling.
    await page.evaluate(
      (id) => window.__testLibraryProgress(id, 103871648, "installed"),
      diarizationId,
    );
    const toggle = row.getByRole("switch", { name: "Label speakers" });
    await expect(toggle).toBeChecked({ timeout: 7000 });
    await expect(row).toHaveAccessibleDescription(
      "Separates voices on this computer after transcription.",
    );
    expect(await settings(page)).toEqual(before);
    await page.locator(".meeting-fold").screenshot({
      path: testInfo.outputPath(`speakers-installed-${theme}.png`),
    });
    await toggle.click();
    await expect(toggle).not.toBeChecked();
    expect(await settings(page)).toEqual({
      ...before,
      meetingSpeakerLabels: false,
    });
    await page
      .getByRole("button", { name: "View meetings", exact: true })
      .click();
    await openDefaults(page);
    await expect(toggle).not.toBeChecked();
    await toggle.click();
    await expect(toggle).toBeChecked();
    expect(await settings(page)).toEqual(before);
    expect(await calls(page, "save_settings")).toHaveLength(2);
  });
}

for (const [name, diarization] of [
  ["unsupported", { state: "unsupported", installable: false }],
  ["missing from library", null],
]) {
  test(`speaker setting is omitted when ${name}`, async ({ page }) => {
    await startPage(page, { diarization });
    await expect(settingRow(page)).toHaveCount(0);
    expect(await calls(page, "start_model_install")).toHaveLength(0);
    expect((await settings(page)).meetingSpeakerLabels).toBe(true);
  });
}

test("install errors use the existing error display and can be retried", async ({
  page,
}) => {
  await startPage(page);
  const row = settingRow(page);
  const before = await settings(page);
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    let fail = true;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "start_model_install" && fail) {
        fail = false;
        throw new Error("Mock speaker model download could not start");
      }
      return invoke(command, args);
    };
  });
  await row.getByRole("button", { name: "Install", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.getByRole("alert").locator("pre")).toContainText(
    "Mock speaker model download could not start",
  );
  await row.getByRole("button", { name: "Install", exact: true }).click();
  await expect(
    row.getByRole("button", { name: "Installing…", exact: true }),
  ).toBeDisabled();
  await page.evaluate(
    (id) =>
      window.__testPatchModel(id, {
        state: "failed",
        error: "Mock diarization checksum mismatch",
      }),
    diarizationId,
  );
  await expect(page.getByRole("alert")).toBeVisible({ timeout: 7000 });
  await expect(page.getByRole("alert").locator("pre")).toContainText(
    "Mock diarization checksum mismatch",
  );
  await row.getByRole("button", { name: "Install", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(
    row.getByRole("button", { name: "Installing…", exact: true }),
  ).toBeDisabled();
  expect(await settings(page)).toEqual(before);
});

test("failed speaker preference saves retain the previous toggle state", async ({
  page,
}) => {
  await startPage(page, {
    diarization: { state: "installed", removable: true },
  });
  const before = await settings(page);
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "save_settings")
        throw new Error("Mock settings write failed");
      return invoke(command, args);
    };
  });
  const toggle = settingRow(page).getByRole("switch", {
    name: "Label speakers",
  });
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(page.getByRole("alert")).toContainText(
    "The setting could not be saved",
  );
  expect(await settings(page)).toEqual(before);
});

test("background speaker-model verification waits without disabling other meeting settings", async ({ page }) => {
  await startPage(page, { diarization: { state: "checking" } });
  const row = settingRow(page);
  await expect(row.getByRole("button", { name: "Checking…", exact: true })).toBeDisabled();
  await expect(row).not.toContainText("Downloads");
  const titles = page.getByRole("switch", { name: titlesLabel, exact: true });
  await titles.click();
  await expect.poll(async () => (await settings(page)).autoMeetingTitles).toBe(false);
  expect(await calls(page, "start_model_install")).toHaveLength(0);
  await page.evaluate((id) => window.__testPatchModel(id, { state: "installed" }), diarizationId);
  await expect(row.getByRole("switch", { name: "Label speakers", exact: true })).toBeEnabled();
  expect(await calls(page, "start_model_install")).toHaveLength(0);
});

test("diarization installs and removes in All roles and stays out of speech/text pickers", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=setup");
  const library = await openModelLibrary(page);
  const before = await settings(page);
  await expect(
    library.getByRole("combobox", { name: "Filter model library" }),
  ).toHaveValue("all");
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  await library.getByRole("searchbox").fill("Nemotron");
  await library
    .getByRole("button", {
      name: "Install Nemotron 3 Diarization",
      exact: true,
    })
    .click();
  const download = page.getByRole("region", { name: "Model download" });
  await expect(download).toContainText("Nemotron 3 Diarization");
  await expect(download.getByRole("progressbar")).toBeVisible();
  await page.evaluate(
    (id) => window.__testLibraryProgress(id, 103871648, "installed"),
    diarizationId,
  );
  await expect(download).toContainText("Installed", { timeout: 7000 });
  for (const role of ["Speech", "Cleanup", "Notes"]) {
    await page
      .getByRole("button", { name: `${role} model`, exact: true })
      .click();
    const picker = page.getByRole("dialog", {
      name: `${role} models`,
      exact: true,
    });
    await picker.getByRole("searchbox").fill("Nemotron");
    await expect(picker.getByRole("option")).toHaveCount(0);
    await page.keyboard.press("Escape");
  }
  await library.getByRole("tab", { name: "Installed", exact: true }).click();
  await library
    .getByRole("button", { name: "Remove Nemotron 3 Diarization", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(
    library.getByRole("button", {
      name: "Remove Nemotron 3 Diarization",
      exact: true,
    }),
  ).toHaveCount(0);
  expect((await calls(page, "remove_model")).map((call) => call.args)).toEqual([
    { id: diarizationId, confirmed: true },
  ]);
  expect(await settings(page)).toEqual(before);
  await goToPage(page, "Meetings");
  await openDefaults(page);
  await expect(
    settingRow(page).getByRole("button", { name: "Install", exact: true }),
  ).toBeEnabled();
});

test("a meeting with several voices can switch its speaker labels", async ({
  page,
}, testInfo) => {
  const labeled =
    "[0:01] Speaker 1: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Speaker 2: Reply.";
  await openApp(
    page,
    { meeting: { transcript: labeled, speakerCount: 2, speakerLabels: true } },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  await expect(page.locator(".meeting-metadata")).toContainText("2 speakers");
  const menu = await openMeetingMenu(page);
  const row = menu.getByRole("group", { name: "Speaker labels", exact: true });
  await expect(row).toHaveAccessibleDescription(
    "Applies to the transcript, saved copies and new notes.",
  );
  await menu.screenshot({
    path: testInfo.outputPath("meeting-speaker-labels-option.png"),
  });
  const toggle = row.getByRole("switch", { name: "Speaker labels" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(page.getByText("Other speakers").first()).toBeVisible();
  expect(
    (await calls(page, "set_meeting_speaker_labels")).map((call) => call.args),
  ).toEqual([{ id: "meeting-one", enabled: false, confirmed: false }]);

  // Edited transcripts ask before they are rebuilt; declining keeps them.
  await page.evaluate(() =>
    window.__TAURI_INTERNALS__.invoke("save_meeting", {
      id: "meeting-one",
      title: "Design review",
      transcript: "Edited.",
      notes: "",
    }),
  );
  await openMeetingMenu(page);
  await toggle.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Replace your transcript edits?");
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await dialog.getByRole("button", { name: "Replace edits" }).click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(
    (await calls(page, "set_meeting_speaker_labels")).map((call) => call.args),
  ).toEqual([
    { id: "meeting-one", enabled: false, confirmed: false },
    { id: "meeting-one", enabled: true, confirmed: false },
    { id: "meeting-one", enabled: true, confirmed: false },
    { id: "meeting-one", enabled: true, confirmed: true },
  ]);
});

test("single-voice meetings do not offer speaker labels", async ({ page }) => {
  await openApp(page, { meeting: { speakerCount: 1 } }, "/?page=meetings");
  await openMeeting(page, /Design review/);
  const menu = await openMeetingMenu(page);
  await expect(
    menu.getByRole("button", { name: "Rename", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Speaker labels", exact: true }),
  ).toHaveCount(0);
});
