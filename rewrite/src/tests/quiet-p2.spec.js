import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

const folder = new URL("../../artifacts/redesign-p2/", import.meta.url);
const nav = (page) => page.getByRole("navigation", { name: "Main navigation" });
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const snapshot = (page) => page.evaluate(() => window.__testSnapshot());
const record = (id, day, hour, published, extra = {}) => {
  const date = new Date();
  date.setDate(date.getDate() - day);
  date.setHours(hour, 20, 0, 0);
  return {
    id,
    createdAt: date.toISOString(),
    raw: published.toLowerCase(),
    published,
    targetApp: "Notepad",
    recognitionCompleteness: "complete",
    recognitionWarnings: [],
    ...extra,
  };
};
const history = () => [
  record(
    "today-one",
    0,
    11,
    "Pushed the fix for the export bug. Can you give it a quick look before lunch?",
    { targetApp: "Slack" },
  ),
  record(
    "today-two",
    0,
    10,
    "Send the north wind quote to Dana by Friday and copy the finance team.",
    {
      targetApp: "Outlook",
      preModelText: "Send the north wind quote to Dana.",
    },
  ),
  record(
    "today-three",
    0,
    9,
    "Agenda for Thursday: launch checklist, pricing review, and who owns the support inbox.",
  ),
  record(
    "today-four",
    0,
    8,
    "Hi Dana, thanks for the contract. I have signed page four.",
  ),
  record(
    "yesterday-one",
    1,
    17,
    "Call the dentist Thursday, pick up the tripod, renew the domain before the 12th.",
  ),
  record(
    "yesterday-two",
    1,
    16,
    "Running ten minutes late, start without me and I will catch up on the notes.",
  ),
  record(
    "incomplete",
    1,
    14,
    "The latency numbers look good. Keep the available words.",
    {
      recognitionCompleteness: "incomplete",
      recognitionWarnings: ["Some words could not be confirmed."],
    },
  ),
];
async function open(page, options = {}, path = "/") {
  await installDesktopFixture(page, {
    history: history(),
    settings: { autoPaste: true },
    ...options,
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(path);
  await expect(nav(page)).toBeVisible();
}
async function capture(page, name) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(name, folder)) });
}
test.afterEach(async ({ page }, info) => {
  if (info.status === info.expectedStatus) await page.goto("about:blank");
});

test("day pages, keyboard actions, saved versions, word fixes and export", async ({
  page,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open(page);
  await expect(
    page.getByRole("heading", { name: "Ready", exact: true }),
  ).toBeVisible();
  const stages = page.getByRole("group", { name: "Dictation settings" });
  await expect(stages.getByRole("button")).toHaveCount(4);
  await expect(
    page.getByRole("region", { name: "Yesterday", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Fewest errors", { exact: false })).toHaveCount(
    0,
  );
  await page.mouse.move(2, 2);
  const row = page.locator(".dictation-row").first();
  await expect(row.locator(".quiet-row-actions")).toHaveCSS("opacity", "0");
  await capture(page, "01-dictation-fog.png");
  const yesterday = page
    .getByRole("region", { name: "Yesterday", exact: true })
    .locator(".dictation-row")
    .first();
  await yesterday.locator(".row-open").focus();
  await expect(yesterday.locator(".quiet-row-actions")).toHaveCSS(
    "opacity",
    "1",
  );
  await page.keyboard.press("Tab");
  await expect(
    yesterday.getByRole("button", { name: "Copy", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  expect((await calls(page, "copy_history")).at(-1).args).toEqual({
    id: "yesterday-one",
    original: false,
  });
  await page
    .getByLabel("Search dictations", { exact: true })
    .fill("north wind");
  await expect(page.locator(".dictation-row")).toHaveCount(1);
  await page.locator(".row-open").click();
  const detail = page.getByRole("region", { name: "Open dictation" });
  await detail
    .getByRole("button", { name: "Before cleanup", exact: true })
    .click();
  await expect(detail.getByLabel("Before cleanup", { exact: true })).toHaveText(
    "Send the north wind quote to Dana.",
  );
  await detail.getByRole("button", { name: "Copy", exact: true }).click();
  expect((await calls(page, "copy_history_pre_model")).at(-1).args.id).toBe(
    "today-two",
  );
  await detail.getByRole("button", { name: "As heard", exact: true }).click();
  await detail.getByRole("button", { name: "Copy", exact: true }).click();
  expect((await calls(page, "copy_history")).at(-1).args.original).toBe(true);
  await detail.getByRole("button", { name: "Final", exact: true }).click();
  await capture(page, "02-dictation-open.png");
  await detail.getByRole("button", { name: "Fix a word" }).click();
  const fix = page.getByRole("dialog", { name: "Fix a word" });
  await fix.getByLabel("When it hears").fill("north wind");
  await fix.getByLabel("Write it as").fill("Northwind");
  await capture(page, "03-fix-word.png");
  await page.keyboard.press("Escape");
  await page
    .getByRole("dialog", { name: "Leave these changes?" })
    .getByRole("button", { name: "Keep working" })
    .click();
  await expect(fix).toBeVisible();
  await fix.getByRole("button", { name: "Save", exact: true }).click();
  await expect(fix).not.toBeVisible();
  expect((await calls(page, "save_dictation_correction")).at(-1).args).toEqual({
    id: "today-two",
    text: "Send the Northwind quote to Dana by Friday and copy the finance team.",
    scope: null,
  });
  expect((await calls(page, "learn_correction")).at(-1).args).toEqual({
    id: "today-two",
    heard: "north wind",
    written: "Northwind",
  });
  await detail.getByRole("button", { name: "As heard", exact: true }).click();
  await expect(detail.getByLabel("As heard", { exact: true })).toContainText(
    "north wind",
  );
  await page.evaluate(() => window.__testDialogSave(null));
  await detail.getByRole("button", { name: "Export .md" }).click();
  await expect(
    detail.getByRole("button", { name: "Export .md" }),
  ).toBeEnabled();
  expect(await calls(page, "export_dictation")).toHaveLength(0);
  await page.evaluate(() => window.__testDialogSave("C:/Exports/dictation.md"));
  await detail.getByRole("button", { name: "Export .md" }).click();
  await expect
    .poll(async () => (await calls(page, "export_dictation")).length)
    .toBe(1);
  expect((await calls(page, "export_dictation"))[0].args).toEqual({
    id: "today-two",
    path: "C:/Exports/dictation.md",
  });
  await expect(
    page.getByText("Dictation exported.", { exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("input wells open, key capture cancels, settings save, and mic test shares capture", async ({
  page,
}) => {
  await open(page, {}, "/?page=microphone");
  await expect(
    page.getByRole("heading", { name: "Shortcut and mic", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    window.__testPushSnapshot({
      devices: [
        { id: "mic", name: "Jabra Evolve2", isDefault: true },
        { id: "array", name: "Realtek microphone array", isDefault: false },
      ],
    }),
  );
  const keys = page.getByRole("button", { name: "Change shortcut" });
  await keys.click();
  await page.keyboard.press("Escape");
  expect(await calls(page, "save_settings")).toHaveLength(0);
  await keys.click();
  await page.keyboard.press("Control+Alt+d");
  await expect(keys).toHaveText("CtrlAltD");
  expect((await snapshot(page)).settings.hotkey).toBe("Ctrl+Alt+D");
  await page.getByRole("button", { name: "Stops when", exact: true }).click();
  await page.getByRole("option", { name: "I let go (hold to talk)" }).click();
  expect((await snapshot(page)).settings.hotkeyMode).toBe("hold");
  await page.getByRole("button", { name: "Microphone", exact: true }).click();
  await expect(page.getByRole("listbox", { name: "Microphone" })).toBeVisible();
  await capture(page, "04-input-microphone-picker.png");
  await page.getByRole("option", { name: "Realtek microphone array" }).click();
  expect((await snapshot(page)).settings.microphone).toBe("array");
  await expect(
    page.getByRole("button", { name: "Microphone", exact: true }),
  ).toBeFocused();
  await page
    .getByRole("button", { name: "Microphone", exact: true })
    .press("ArrowDown");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("listbox")).toHaveCount(0);
  expect(await calls(page, "cancel_dictation")).toHaveLength(0);
  await page.getByText("Test microphone", { exact: true }).click();
  await page.getByRole("button", { name: "Test with a dictation" }).click();
  expect(await calls(page, "test_microphone")).toHaveLength(1);
  await expect(
    page.getByRole("button", { name: "Microphone", exact: true }),
  ).toBeDisabled();
  await page.getByRole("button", { name: "Finish dictation" }).click();
  await page.getByRole("button", { name: "Review test result" }).click();
  await expect(
    page.getByRole("region", { name: "Current dictation" }),
  ).toBeVisible();
  await page.evaluate(() => window.__testNavigatePage("shortcuts"));
  await expect(
    page.getByRole("heading", { name: "Shortcut and mic", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    window.__testPushSnapshot({ error: "hotkey could not be registered" }),
  );
  await expect(page.getByText("TAKEN", { exact: true })).toBeVisible();
  await page.evaluate(() =>
    window.__testPushSnapshot({
      error: null,
      capabilities: {
        ...window.__testSnapshot().capabilities,
        platform: "linux",
        globalShortcut: false,
      },
    }),
  );
  await expect(
    page.getByText(/This desktop has no global shortcuts/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await calls(page, "copy_shortcut_command")).toHaveLength(1);
  expect(await page.evaluate(() => window.__testClipboard)).toBe(
    "sonojot --shortcut press",
  );
});

test("output shared Settings door, retention and confirmed deletion", async ({
  page,
}) => {
  await open(page, {
    dictationAudio: { count: 3, bytes: 2048000 },
    settings: { autoPaste: true },
  });
  await nav(page).getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: /^Dictation / }).click();
  await page.getByRole("button", { name: /^Paste and history/ }).click();
  await expect(
    page.getByRole("heading", { name: "Paste and history", exact: true }),
  ).toBeVisible();
  await capture(page, "05-paste-history.png");
  await page.getByRole("switch", { name: "Paste where I was typing" }).click();
  expect((await snapshot(page)).settings.autoPaste).toBe(false);
  await page
    .getByRole("button", { name: "Text and audio", exact: true })
    .click();
  expect((await snapshot(page)).settings.keepDictationAudio).toBe(true);
  await page.getByRole("button", { name: "Nothing", exact: true }).click();
  expect((await snapshot(page)).settings).toMatchObject({
    keepDictationAudio: false,
    doNotSaveDictations: true,
  });
  await page.getByRole("button", { name: "Delete…", exact: true }).click();
  await page.getByRole("button", { name: "Delete audio", exact: true }).click();
  await page
    .getByRole("dialog", { name: "Delete all kept dictation audio?" })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  expect(await calls(page, "delete_dictation_audio")).toHaveLength(0);
  await page
    .getByRole("button", { name: "Delete dictations", exact: true })
    .click();
  await page
    .getByRole("dialog", { name: "Delete all 7 dictations?" })
    .getByRole("button", { name: "Delete all", exact: true })
    .click();
  expect((await calls(page, "delete_all_dictations"))[0].args).toEqual({
    confirmed: true,
  });
  await page.getByRole("button", { name: "Close sheet" }).click();
  await page.getByRole("button", { name: "Back to Settings" }).click();
  await expect(
    page.getByRole("heading", { name: "Settings", exact: true }),
  ).toBeVisible();
});

test("Slate capture band, Escape paths and incomplete review", async ({
  page,
}) => {
  await open(page, { settings: { theme: "slate", autoPaste: true } });
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "listening",
      sessionId: 41,
      audioLevel: 0.6,
      interimText: "So the export keeps the speaker names and the",
      previewText: "latest notes",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Listening", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("group", { name: "Dictation settings" }),
  ).toHaveCount(0);
  await capture(page, "06-listening-slate.png");
  await page.keyboard.press("Escape");
  expect(await calls(page, "cancel_dictation")).toHaveLength(1);
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "cleaning", sessionId: 42 }),
  );
  await page.getByRole("button", { name: "Stop cleanup" }).click();
  expect(await calls(page, "stop_cleanup")).toHaveLength(1);
  await page.getByLabel("Search dictations", { exact: true }).fill("latency");
  const row = page.locator(".dictation-row");
  await row.hover();
  await row.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await calls(page, "copy_history")).toHaveLength(0);
  await expect(
    page.getByRole("region", { name: "Open dictation" }),
  ).toContainText("INCOMPLETE · NOT PASTED");
  await page
    .getByRole("region", { name: "Open dictation" })
    .getByRole("button", { name: "Copy", exact: true })
    .click();
  expect((await calls(page, "copy_history"))[0].args.id).toBe("incomplete");
});

test("correction drafts survive refresh, validate raw words, and use confirmed deletion", async ({
  page,
}) => {
  await open(page, {
    history: [
      record("one", 0, 11, "First, keep Grok 4.6.", {
        raw: "first keep grock 4.6",
      }),
    ],
  });
  await page.locator(".row-open").click();
  const detail = page.getByRole("region", { name: "Open dictation" });
  await detail.getByRole("button", { name: "Fix a word" }).click();
  const fix = page.getByRole("dialog", { name: "Fix a word" });
  await fix.getByLabel("When it hears").fill("missing");
  await fix.getByLabel("Write it as").fill("Grok");
  await expect(
    fix.getByRole("button", { name: "Save", exact: true }),
  ).toBeDisabled();
  await fix.getByLabel("When it hears").fill("grock");
  await fix.getByRole("button", { name: "Save", exact: true }).click();
  await expect(fix).not.toBeVisible();
  expect(await calls(page, "save_dictation_correction")).toHaveLength(0);
  expect((await calls(page, "learn_correction"))[0].args).toEqual({
    id: "one",
    heard: "grock",
    written: "Grok",
  });
  await detail.locator("summary", { hasText: "More" }).click();
  await detail.getByRole("button", { name: "Edit full text" }).click();
  await detail
    .getByLabel("Corrected text")
    .fill("First, keep Grok 4.6 revised.");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      resultSessionId: 99,
      originalText: "A newer arrival.",
      outputText: "A newer arrival.",
      recognitionCompleteness: "complete",
    }),
  );
  await expect(detail.getByLabel("Corrected text")).toHaveValue(
    "First, keep Grok 4.6 revised.",
  );
  await nav(page).getByRole("button", { name: "Settings" }).click();
  await page
    .getByRole("dialog", { name: "Leave these changes?" })
    .getByRole("button", { name: "Keep working" })
    .click();
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "save_dictation_correction") {
        window.__TAURI_INTERNALS__.invoke = invoke;
        throw new Error("Cannot save the corrected dictation.");
      }
      return invoke(command, args);
    };
  });
  await detail.getByRole("button", { name: "Save correction" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(detail.getByLabel("Corrected text")).toHaveValue(
    "First, keep Grok 4.6 revised.",
  );
  expect(await calls(page, "learn_correction")).toHaveLength(1);
  await detail.getByRole("button", { name: "Save correction" }).click();
  await expect(detail.getByRole("status")).toHaveText(
    "Saved correction to this dictation.",
  );
  expect((await calls(page, "save_dictation_correction"))[0].args).toEqual({
    id: "one",
    text: "First, keep Grok 4.6 revised.",
    scope: null,
  });
  expect(await calls(page, "learn_correction")).toHaveLength(1);
  await detail.getByRole("button", { name: "As heard", exact: true }).click();
  await expect(detail.locator(".full-text")).toHaveText("first keep grock 4.6");
  await detail.getByRole("button", { name: "Delete this dictation" }).click();
  await page
    .getByRole("dialog", { name: "Delete this dictation?" })
    .getByRole("button", { name: "Cancel" })
    .click();
  expect(await calls(page, "delete_dictation")).toHaveLength(0);
  await detail.getByRole("button", { name: "Delete this dictation" }).click();
  await page
    .getByRole("dialog", { name: "Delete this dictation?" })
    .getByRole("button", { name: "Delete", exact: true })
    .click();
  await expect(detail).not.toBeVisible();
  expect((await calls(page, "delete_dictation"))[0].args).toEqual({
    id: "one",
    confirmed: true,
  });
});
