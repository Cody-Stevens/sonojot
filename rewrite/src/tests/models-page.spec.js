// October 13 Models page fixes: the built-in Cleanup option is named for the
// punctuation model, Remove explains or performs the role switch, and the
// OpenRouter catalog lists each model once with its curated reasoning effort.
import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { openApp, openModelLibrary } from "./app-helpers.js";

const cleanupName = "SonoJot Punctuation v2 with your vocabulary";

async function screenshot(page, name, locator = page) {
  const directory = new URL("../../../artifacts/models-review/", import.meta.url);
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await locator.screenshot({
    path: fileURLToPath(new URL(`${name}.png`, directory)),
  });
}

const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const calls = (page, command) =>
  page.evaluate(
    (name) => window.__testCalls.filter((call) => call.command === name),
    command,
  );

async function openCleanupPicker(page) {
  await page.getByRole("button", { name: "Cleanup model", exact: true }).click();
  return page.getByRole("dialog", { name: "Cleanup models", exact: true });
}

test("Custom Cleanup lists the punctuation model first with its install state", async ({
  page,
}) => {
  await openApp(
    page,
    { settings: { cleanupModel: "local-rules" } },
    "/?page=setup&theme=fog",
  );
  const trigger = page.getByRole("button", { name: "Cleanup model", exact: true });
  await expect(trigger).toHaveText(cleanupName);
  const picker = await openCleanupPicker(page);
  const options = picker.getByRole("option");
  await expect(options.first()).toContainText(cleanupName);
  await expect(options.first()).toContainText("CPU · Recommended · Installed · Selected");
  await expect(picker).not.toContainText("Formatting and vocabulary");
  await expect(
    picker.getByRole("region", { name: `Details for ${cleanupName}` }),
  ).toBeVisible();
  await screenshot(page, "cleanup-picker-installed");
  await page.keyboard.press("Escape");
  // The profile summary and the Models summary use the same words.
  await expect(page.locator("#setup-cleanup")).toContainText("CPU");
});

test("Cleanup picker offers the punctuation download when it is missing", async ({
  page,
}) => {
  await openApp(
    page,
    {
      settings: { cleanupModel: "lfm2.5-1.2b" },
      punctuation: { state: "available", downloadedBytes: 0 },
    },
    "/?page=setup&theme=fog",
  );
  const picker = await openCleanupPicker(page);
  const first = picker.getByRole("option").first();
  await expect(first).toContainText(cleanupName);
  await expect(first).toContainText("CPU · Recommended · Not installed");
  await first.click();
  // Rules and vocabulary work at once; the package is offered alongside.
  await expect
    .poll(async () => (await settings(page)).cleanupModel)
    .toBe("local-rules");
  const reopened = await openCleanupPicker(page);
  await expect(reopened.getByText("SonoJot Punctuation v2 · 303 MB")).toBeVisible();
  await screenshot(page, "cleanup-picker-not-installed");
  await reopened.getByRole("button", { name: "Install", exact: true }).click();
  const consent = page.getByRole("dialog");
  if (await consent.getByRole("checkbox").count()) {
    await consent.getByRole("checkbox").check();
    await consent.getByRole("button", { name: /Download/ }).click();
  }
  await expect
    .poll(async () =>
      (await calls(page, "start_model_install")).map((call) => call.args.id),
    )
    .toContain("sonojot-punct-v2");
});

test("a model used for Notes is removable after confirming the switch", async ({
  page,
}) => {
  await openApp(
    page,
    { settings: { cleanupModel: "local-rules", notesModel: "lfm2.5-1.2b" } },
    "/?page=setup&theme=fog",
  );
  const library = await openModelLibrary(page);
  const row = library.locator('[data-model-id="lfm2.5-1.2b"]');
  await expect(row).toContainText("Notes");
  const remove = row.getByRole("button", { name: "Remove LFM 2.5", exact: true });
  await expect(remove).toBeEnabled();
  await screenshot(page, "remove-enabled-for-notes-model", row);
  await remove.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Remove LFM 2.5?");
  await expect(dialog).toContainText(
    "Notes will switch to Qwen 3 4B. Its downloaded model files will be deleted.",
  );
  await screenshot(page, "remove-confirm-switch");
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "remove_model")).length)
    .toBe(1);
  expect((await settings(page)).notesModel).toBe("qwen3-4b-instruct-2507");
  await expect(row).toHaveCount(0);
});

test("a saved local Notes model is not shown as in use while Notes run online", async ({
  page,
}) => {
  await openApp(
    page,
    {
      settings: { cleanupModel: "local-rules", notesModel: "lfm2.5-1.2b" },
      aiSettings: { backend: "openrouter", model: "mock/text", connected: true },
    },
    "/?page=setup&theme=fog",
  );
  const library = await openModelLibrary(page);
  const row = library.locator('[data-model-id="lfm2.5-1.2b"]');
  await expect(row).not.toContainText("Notes");
  await row.getByRole("button", { name: "Remove LFM 2.5", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Delete its downloaded model files.");
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await expect
    .poll(async () => (await calls(page, "remove_model")).length)
    .toBe(1);
  expect((await settings(page)).notesModel).toBe("qwen3-4b-instruct-2507");
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    backend: "openrouter",
    model: "mock/text",
  });
});

test("Remove stays disabled with a visible reason when no replacement exists", async ({
  page,
}) => {
  await openApp(
    page,
    {
      settings: { cleanupModel: "local-rules", notesModel: "lfm2.5-1.2b" },
      modelPatches: {
        "qwen3-4b-instruct-2507": { state: "available", downloadedBytes: 0 },
        "mock-cleanup-2b-cpu": { state: "available", downloadedBytes: 0 },
      },
    },
    "/?page=setup&theme=fog",
  );
  const library = await openModelLibrary(page);
  const row = library.locator('[data-model-id="lfm2.5-1.2b"]');
  await expect(
    row.getByRole("button", { name: "Remove LFM 2.5", exact: true }),
  ).toBeDisabled();
  await expect(row).toContainText(
    "Notes uses this model. Install another text model or choose an OpenRouter model first.",
  );
  await screenshot(page, "remove-blocked-reason", row);
  // Not-installed rows offer Install, never a disabled Remove.
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  const qwen = library.locator('[data-model-id="qwen3-4b-instruct-2507"]');
  await expect(qwen.getByRole("button", { name: /^Install / })).toBeEnabled();
  await expect(qwen.getByRole("button", { name: /^Remove / })).toHaveCount(0);
});

const recommended = [
  {
    id: "z-ai/glm-5.3-flash", name: "Z.ai: GLM 5.3 Flash", contextLength: 1048575,
    promptPrice: "0.00000015", completionPrice: "0.0000005",
    inputModalities: ["text", "image", "video"], reasoningEffort: "high",
  },
  {
    id: "openai/gpt-6-luna", name: "OpenAI: GPT-6 Luna", contextLength: 1050000,
    promptPrice: "0.0000001", completionPrice: "0.0000005",
    inputModalities: ["file", "image", "text"], reasoningEffort: "xhigh",
  },
  {
    id: "google/gemini-3.8-flash", name: "Google: Gemini 3.8 Flash", contextLength: 1048576,
    promptPrice: "0.00000075", completionPrice: "0.00000375",
    inputModalities: ["text", "image", "video", "file", "audio"], reasoningEffort: "high",
  },
];

test("OpenRouter lists a two-role model once and adds it for both pickers", async ({
  page,
}) => {
  await openApp(
    page,
    {
      aiSettings: { backend: "local", model: "", connected: true, quickModels: [] },
      aiRecommendedModels: recommended,
      aiCatalog: recommended,
    },
    "/?page=setup&theme=fog",
  );
  const online = page.getByRole("region", { name: "Online models (OpenRouter)" });
  const results = online.getByRole("list", { name: "OpenRouter catalog results" });
  await expect(results.getByRole("listitem")).toHaveCount(3);
  // Curated order, not price order.
  await expect(results.getByRole("listitem").nth(0)).toContainText("GLM 5.3 Flash");
  await expect(results.getByRole("listitem").nth(1)).toContainText("GPT-6 Luna");
  const gemini = results.getByRole("listitem").filter({ hasText: "Gemini 3.8 Flash" });
  await expect(gemini).toHaveCount(1);
  await expect(gemini).toContainText(
    "Text and notes, meeting transcription · High reasoning · $0.75 input, $3.75 output per 1M tokens",
  );
  await expect(results.getByRole("listitem").nth(1)).toContainText("Extra-high reasoning");
  await expect(online).toContainText("Recommended models. Use All models for the full text and speech catalog.");
  await gemini.scrollIntoViewIfNeeded();
  await screenshot(page, "openrouter-add-a-model");
  await gemini
    .getByRole("button", {
      name: "Add Google: Gemini 3.8 Flash for Text and notes, meeting transcription",
    })
    .click();
  await expect(
    online.getByText(
      "Google: Gemini 3.8 Flash added for text, notes and meeting transcription.",
    ),
  ).toBeVisible();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    quickModels: [
      { id: "google/gemini-3.8-flash", role: "text" },
      { id: "google/gemini-3.8-flash", role: "speech" },
    ],
  });
  await expect(gemini.getByText("In your list")).toBeVisible();
  const yours = online.getByRole("list", { name: "Your OpenRouter models" });
  await expect(yours.getByRole("listitem")).toHaveCount(1);
  await expect(yours).toContainText("Text and notes, meeting transcription");
  await screenshot(page, "openrouter-your-models", yours);
  // Meeting transcription can still pick it.
  await page.getByRole("button", { name: "Meeting transcription model", exact: true }).click();
  const meeting = page.getByRole("dialog", { name: "Meeting transcription models", exact: true });
  await meeting.getByRole("option", { name: /Gemini 3\.8 Flash/ }).click();
  await expect
    .poll(async () => (await page.evaluate(() => window.__testAiSettings())).speechModel)
    .toBe("google/gemini-3.8-flash");
  // One Remove takes it out of both pickers and resets the meeting route.
  await yours.getByRole("button", { name: "Remove Google: Gemini 3.8 Flash" }).click();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    quickModels: [],
    speechModel: "",
  });
});
