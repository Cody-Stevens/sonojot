import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { openApp, openMeeting } from "./app-helpers.js";

test.setTimeout(60_000);
test.beforeEach(async ({ page }) =>
  page.emulateMedia({ reducedMotion: "reduce" }),
);
test.afterEach(async ({ page }, info) => {
  if (info.status === info.expectedStatus) await page.goto("about:blank");
});
const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
async function menu(page, name) {
  await page
    .getByRole("button", { name: "Meeting options", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name, exact: true })
    .click();
}
async function shot(page, name) {
  const folder = new URL("../../artifacts/review-fixes/", import.meta.url);
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
async function cleanup(page, options = {}) {
  await openApp(page, {
    ...options,
    settings: {
      localProofreader: false,
      cleanupModel: "local-rules",
      ...options.settings,
    },
  });
  await page.getByRole("button", { name: "Cleanup", exact: true }).click();
}

test("shortcut capture ignores bare typing and navigation keys before saving a chord", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=shortcuts");
  const before = (await settings(page)).hotkey;
  await page.getByRole("button", { name: "Change shortcut" }).click();
  for (const key of ["Tab", "Enter", "a", "Shift+A", "ArrowLeft"])
    await page.keyboard.press(key);
  expect((await settings(page)).hotkey).toBe(before);
  expect(await calls(page, "save_settings")).toHaveLength(0);
  await shot(page, "01-shortcut-chord.png");
  await page.keyboard.press("Control+Shift+K");
  await expect
    .poll(async () => (await settings(page)).hotkey)
    .toBe("Ctrl+Shift+K");
  await page.getByRole("button", { name: "Change shortcut" }).click();
  await page.keyboard.press("F8");
  await expect.poll(async () => (await settings(page)).hotkey).toBe("F8");
});

test("an external re-transcription cannot save a cleaned draft over new recognition", async ({
  page,
}) => {
  await openApp(
    page,
    {
      meeting: {
        transcript: "Original recognition.",
        cleanedTranscript: "Cleaned copy.",
        notes: "",
      },
    },
    "/?page=meetings",
  );
  await openMeeting(page, /Design review/);
  const text = page.getByRole("textbox", {
    name: "Meeting transcript",
    exact: true,
  });
  await text.fill("Unsaved cleaned edits.");
  await page.evaluate(() =>
    window.__testUpdateMeeting({
      transcript: "New recognition.",
      cleanedTranscript: null,
    }),
  );
  await expect(page.getByRole("alert")).toContainText(
    "transcript changed elsewhere",
  );
  await expect(
    page.getByText("Cleaned transcript", { exact: true }),
  ).toBeVisible();
  await expect(text).toHaveValue("Unsaved cleaned edits.");
  await expect(
    page.getByRole("button", { name: "Save changes" }),
  ).toBeDisabled();
  expect(await calls(page, "save_meeting")).toHaveLength(0);
  await shot(page, "02-transcript-conflict.png");
  await page
    .getByRole("button", { name: "Reload saved meeting", exact: true })
    .click();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(text).toHaveValue("Unsaved cleaned edits.");
  await page
    .getByRole("button", { name: "Reload saved meeting", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Reload saved meeting?" })
    .getByRole("button", { name: "Reload saved meeting", exact: true })
    .click();
  await expect(text).toHaveValue("New recognition.");
  await expect(
    page.getByText("Edited transcript", { exact: true }),
  ).toBeVisible();
  await text.fill("Edited new recognition.");
  await page.getByRole("button", { name: "Save changes" }).click();
  expect((await calls(page, "save_meeting")).at(-1).args).toMatchObject({
    transcript: "Edited new recognition.",
  });
  expect(
    (await calls(page, "save_meeting")).at(-1).args.cleanedTranscript,
  ).toBeUndefined();
});

test("a successful import clears its file and speaker before the sheet reopens", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=meetings");
  await page.getByRole("button", { name: "Import…", exact: true }).click();
  let sheet = page.getByRole("dialog", { name: "Import a recording" });
  await sheet.getByRole("button", { name: "Choose file" }).click();
  await sheet.getByRole("textbox").fill("Rachel");
  await sheet.getByRole("button", { name: "Import", exact: true }).click();
  await expect(sheet).toHaveCount(0);
  expect((await calls(page, "import_media")).at(-1).args).toMatchObject({
    path: "C:/fixture.wav",
    speaker: "Rachel",
  });
  await page.getByRole("button", { name: "All meetings" }).click();
  await page.getByRole("button", { name: "Import…", exact: true }).click();
  sheet = page.getByRole("dialog", { name: "Import a recording" });
  await expect(
    sheet.getByRole("button", { name: "Choose file", exact: true }),
  ).toBeVisible();
  await expect(sheet.getByRole("textbox")).toHaveValue("");
  await expect(
    sheet.getByRole("button", { name: "Import", exact: true }),
  ).toBeDisabled();
});

test("Custom notes stays a draft and uses the same character limit as manual notes", async ({
  page,
}) => {
  await openApp(
    page,
    { settings: { meetingNotesPreset: "summary" } },
    "/?page=meetings",
  );
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  await page.getByLabel("Style", { exact: true }).selectOption("custom");
  expect((await settings(page)).meetingNotesPreset).toBe("summary");
  const instructions = page.getByLabel("Instructions", { exact: true });
  const save = page.getByRole("button", { name: "Save instructions" });
  await expect(save).toBeDisabled();
  await instructions.fill("界".repeat(800));
  await save.click();
  expect((await settings(page)).meetingNotesPreset).toBe("custom");
  expect((await settings(page)).meetingNotesInstructions).toBe(
    "界".repeat(800),
  );
  await instructions.fill("😀".repeat(2001));
  await expect(save).toBeDisabled();
  await instructions.fill("😀".repeat(2000));
  await expect(save).toBeEnabled();
  await shot(page, "03-notes-character-limit.png");
  await save.click();
  await page.locator('[data-nav="meetings"]').click();
  await openMeeting(page, /Design review/);
  await page.getByRole("tab", { name: "Notes", exact: true }).click();
  await page.getByRole("button", { name: "Write notes", exact: true }).click();
  const manual = page.getByLabel("Your instructions", { exact: true });
  const writeNotes = page
    .getByRole("dialog", { name: "Write notes", exact: true })
    .getByRole("button", { name: "Write notes", exact: true });
  await manual.fill("界".repeat(2001));
  await expect(writeNotes).toBeDisabled();
  await manual.fill("😀".repeat(2000));
  await writeNotes.click();
  expect((await calls(page, "generate_notes")).at(-1).args.instructions).toBe(
    "😀".repeat(2000),
  );
});

test("Manage saved models asks before discarding a pasted key", async ({
  page,
}) => {
  await cleanup(page, { aiSettings: { connected: false } });
  await page
    .getByRole("button", { name: "Choose online cleanup model" })
    .click();
  const key = page.getByRole("textbox", { name: "OpenRouter key" });
  await key.fill("sk-test-placeholder");
  await page.getByRole("button", { name: "Manage saved models" }).click();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(key).toHaveValue("sk-test-placeholder");
  await page.getByRole("button", { name: "Manage saved models" }).click();
  await page
    .getByRole("dialog", { name: "Discard this key?" })
    .getByRole("button", { name: "Discard changes", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "OpenRouter" }).locator(".sheet-context"),
  ).toContainText("Your online models");
  await expect(key).toHaveValue("");
  await expect(
    page.getByRole("button", { name: "Manage saved models" }),
  ).toHaveCount(0);
  expect(await calls(page, "connect_openrouter_key")).toHaveLength(0);
});

test("a per-meeting speech choice never changes the stage chip or another meeting", async ({
  page,
}) => {
  await openApp(
    page,
    { modelPatches: { "mock-whisper-small-cuda": { state: "installed" } } },
    "/?page=meetings",
  );
  const chip = page.getByRole("button", {
    name: "Sources, speech and speakers",
    exact: true,
  });
  const original = await chip.textContent();
  await openMeeting(page, /Design review/);
  await menu(page, "Transcribe again…");
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("mock-whisper-small-cuda");
  await page.getByRole("button", { name: "Transcribe", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "transcribe_meeting")).length)
    .toBe(1);
  await page.evaluate(() => window.__testCompleteTranscription());
  await page.getByRole("button", { name: "All meetings" }).click();
  await expect(chip).toHaveText(original);
  await openMeeting(page, /Client call/);
  await menu(page, "Transcribe again…");
  await expect(page.getByLabel("Speech model", { exact: true })).toHaveValue(
    "moonshine-medium",
  );
  expect((await settings(page)).meetingAsrModel).toBe("");
});

for (const state of ["installed", "failed", "rejected"]) {
  test(`cleanup download row clears when the install is ${state}`, async ({
    page,
  }) => {
    const id = "qwen3-4b-instruct-2507";
    await cleanup(page, {
      modelPatches: {
        [id]: { state: "available", downloadedBytes: 0, installable: true },
      },
    });
    await page
      .getByRole("button", { name: "Choose local cleanup model" })
      .click();
    const sheet = page.getByRole("dialog", { name: "Local cleanup model" });
    if (state === "rejected")
      await page.evaluate(
        (id) => window.__testFailModelInstall(id, "No space for this model."),
        id,
      );
    await sheet.getByRole("button", { name: /Qwen.*4B/ }).click();
    await sheet
      .getByRole("button", { name: "Download anyway", exact: true })
      .click();
    if (state !== "rejected") {
      await expect(
        sheet.getByRole("region", { name: "Model download" }),
      ).toBeVisible();
      await page.evaluate(
        ({ id, state }) => {
          window.__testLibraryProgress(id, 4e9, state);
          window.dispatchEvent(new Event("focus"));
        },
        { id, state },
      );
    } else await expect(sheet.getByRole("alert")).toContainText("No space");
    await expect(
      sheet.getByRole("region", { name: "Model download" }),
    ).toHaveCount(0);
    await sheet.getByRole("button", { name: "Close sheet" }).click();
    await page
      .getByRole("button", { name: "Choose local cleanup model" })
      .click();
    await expect(
      sheet.getByRole("region", { name: "Model download" }),
    ).toHaveCount(0);
  });
}

test("suggestions keep written arrows without reparsing them as separators", async ({
  page,
}) => {
  await openApp(
    page,
    {
      aiSettings: { backend: "local" },
      vocabularySuggestions: [
        {
          heard: "ex to why",
          written: "x->y",
          reason: "Recurring written form",
        },
      ],
    },
    "/?page=vocabulary",
  );
  await page.getByRole("button", { name: "Suggested", exact: true }).click();
  await page.getByRole("button", { name: "Review recent dictations" }).click();
  await page.getByRole("button", { name: "Add", exact: true }).click();
  expect(
    (await calls(page, "save_vocabulary")).at(-1).args.entries.at(-1),
  ).toMatchObject({
    heard: "ex to why",
    written: "x->y",
    aliases: ["ex to why"],
  });
});

test("Keep all ready replaces Immediately with a compatible retention in one save", async ({
  page,
}) => {
  await openApp(
    page,
    { settings: { memoryMode: "sequential", modelRetention: "immediately" } },
    "/?page=settings",
  );
  await page.getByRole("button", { name: /^Memory / }).click();
  await page
    .getByRole("button", { name: "Keep all ready", exact: true })
    .click();
  await expect
    .poll(async () => (await settings(page)).memoryMode)
    .toBe("overlap");
  expect((await settings(page)).modelRetention).toBe("10m");
  expect(await calls(page, "save_settings")).toHaveLength(1);
  await expect(
    page.getByRole("button", { name: "After 10 min", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
});
