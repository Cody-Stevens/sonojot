import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { proofreaderCatalog } from "./desktop-fixture.js";
import { goToPage, openApp, showRecentDictations } from "./app-helpers.js";

const label = "Proofread with an online model (experimental)";
const defaultId = "openai/gpt-6-luna";
const trainingId = "meta/muse-spark-1.3-contributor";
const onlineNotice =
  "Local processing stays on this computer. OpenRouter sends the requested audio or text to your selected model and bills your OpenRouter account. SonoJot excludes providers that collect data, so some models may be unavailable. ChatGPT subscriptions do not include OpenRouter usage.";
const base = {
  aiSettings: {
    connected: true,
    model: "mock/notes",
    backend: "openrouter",
    help: onlineNotice,
  },
  aiCatalog: proofreaderCatalog,
  settings: { componentAcknowledgments: { "openrouter-cloud": "1" } },
};
const toggle = (page) => page.getByRole("switch", { name: label, exact: true });
const modelButton = (page) =>
  page.getByRole("button", { name: /^Proofreading model:/ });
const trainingDialog = (page) =>
  page.getByRole("dialog", { name: "Allow this model to train on your text?" });
const saves = (page) =>
  page.evaluate(() =>
    window.__testCalls
      .filter(({ command }) => command === "save_settings")
      .map(({ args }) => args.settings),
  );
async function choose(page, name) {
  if ((await modelButton(page).getAttribute("aria-expanded")) !== "true")
    await modelButton(page).click();
  await page
    .getByRole("searchbox", { name: "Search OpenRouter models" })
    .fill(name);
  await page
    .getByRole("button", { name: `Use ${name} for proofreading`, exact: true })
    .click();
}

test("proofreading defaults off with Luna and selection never changes Notes", async ({
  page,
}) => {
  await openApp(page, base, "/?page=dictation");
  await expect(toggle(page)).not.toBeChecked();
  await expect(toggle(page)).toBeEnabled();
  await expect(modelButton(page)).toHaveText("GPT-6 Luna");
  await expect(page.locator("details.more-options")).not.toHaveAttribute(
    "open",
    "",
  );
  expect(await saves(page)).toHaveLength(0);
  await choose(page, "Another proofreading model");
  await expect(modelButton(page)).toHaveText("Another proofreading model");
  await expect(toggle(page)).not.toBeChecked();
  await toggle(page).click();
  await expect(toggle(page)).toBeChecked();
  expect((await saves(page)).at(-1)).toMatchObject({
    proofreader: true,
    proofreaderModel: "mock/proofreader",
    notesModel: "qwen3-4b-instruct-2507",
  });
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/notes",
    backend: "openrouter",
  });
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        ({ command }) => command === "save_ai_settings",
      ),
    ),
  ).toHaveLength(0);
  await goToPage(page, "Models");
  await goToPage(page, "Dictation");
  await expect(toggle(page)).toBeChecked();
  await expect(modelButton(page)).toHaveText("Another proofreading model");
  await toggle(page).click();
  await expect(toggle(page)).not.toBeChecked();
});

test("missing connection gives a direct action and does not enable proofreading", async ({
  page,
}) => {
  await openApp(
    page,
    { ...base, aiSettings: { ...base.aiSettings, connected: false } },
    "/?page=dictation",
  );
  await expect(toggle(page)).toHaveCount(0);
  await expect(modelButton(page)).toBeDisabled();
  const row = page.getByRole("group", { name: label, exact: true });
  await expect(row).toContainText(
    "Connect OpenRouter to proofread dictations.",
  );
  await row.getByRole("button", { name: "Connect OpenRouter" }).click();
  await expect(toggle(page)).toBeEnabled();
  await expect(toggle(page)).not.toBeChecked();
  expect(await saves(page)).toHaveLength(0);
});

test("connection can be cancelled while sign-in is pending", async ({
  page,
}) => {
  await openApp(
    page,
    {
      ...base,
      aiSettings: { ...base.aiSettings, connected: false },
      aiConnectPending: true,
    },
    "/?page=dictation",
  );
  const row = page.getByRole("group", { name: label, exact: true });
  await row.getByRole("button", { name: "Connect OpenRouter" }).click();
  await expect(row).toContainText("Finish connecting in your browser.");
  await row.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(
    row.getByRole("button", { name: "Connect OpenRouter" }),
  ).toBeEnabled();
  expect(await saves(page)).toHaveLength(0);
});

test("the existing online notice must be accepted and does not switch proofreading on", async ({
  page,
}) => {
  await openApp(page, { ...base, settings: {} }, "/?page=dictation");
  await expect(toggle(page)).toHaveCount(0);
  const review = page.getByRole("button", { name: "Review online notice" });
  await review.click();
  const dialog = page.getByRole("dialog", { name: "Use online proofreading?" });
  await expect(dialog).toContainText(onlineNotice);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await saves(page)).toHaveLength(0);
  await review.click();
  await dialog.getByRole("button", { name: "Accept online notice" }).click();
  await expect(toggle(page)).not.toBeChecked();
  await expect(toggle(page)).toBeEnabled();
  expect((await saves(page)).at(-1)).toMatchObject({
    proofreader: false,
    componentAcknowledgments: { "openrouter-cloud": "1" },
  });
  await toggle(page).click();
  await expect(toggle(page)).toBeChecked();
});

test("training selection supports cancel, exact-model acceptance and persistence", async ({
  page,
}) => {
  await openApp(page, base, "/?page=dictation");
  await choose(page, "Muse Spark Contributor");
  const dialog = trainingDialog(page);
  await expect(dialog).toContainText(trainingId);
  await expect(dialog).toContainText(
    "dictation text and vocabulary to train models",
  );
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(modelButton(page)).toHaveText("GPT-6 Luna");
  expect(await saves(page)).toHaveLength(0);
  await choose(page, "Muse Spark Contributor");
  await dialog.getByRole("button", { name: "Allow for this model" }).click();
  await expect(modelButton(page)).toHaveText("Muse Spark Contributor");
  expect((await saves(page)).at(-1)).toMatchObject({
    proofreader: false,
    proofreaderModel: trainingId,
    proofreaderTrainingModels: [trainingId],
  });
  await goToPage(page, "Models");
  await goToPage(page, "Dictation");
  await expect(toggle(page)).toBeEnabled();
  await toggle(page).click();
  await expect(toggle(page)).toBeChecked();
  await choose(page, "GPT-6 Luna");
  await choose(page, "Muse Spark Contributor");
  await expect(dialog).toHaveCount(0);
  await choose(page, "Another training model");
  await expect(dialog).toContainText("mock/another-training-model");
  await dialog.getByRole("button", { name: "Allow for this model" }).click();
  await expect(modelButton(page)).toHaveText("Another training model");
  expect((await saves(page)).at(-1).proofreaderTrainingModels).toEqual([
    trainingId,
    "mock/another-training-model",
  ]);
});

test("saved training choice without permission requires review", async ({
  page,
}) => {
  await openApp(
    page,
    {
      ...base,
      settings: {
        ...base.settings,
        proofreaderModel: trainingId,
        proofreader: true,
      },
    },
    "/?page=dictation",
  );
  await expect(toggle(page)).toHaveCount(0);
  await page.getByRole("button", { name: "Review model notice" }).click();
  await trainingDialog(page)
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  expect(await saves(page)).toHaveLength(0);
});

test("controls wait for saves and stay disabled throughout capture", async ({
  page,
}) => {
  await openApp(
    page,
    { ...base, holdCommands: ["save_settings"] },
    "/?page=dictation",
  );
  await expect(toggle(page)).toBeEnabled();
  await toggle(page).focus();
  await toggle(page).press("Space");
  await expect.poll(async () => (await saves(page)).length).toBe(1);
  await expect(toggle(page)).toBeDisabled();
  await expect(toggle(page)).not.toBeChecked();
  await expect(modelButton(page)).toBeDisabled();
  await page.evaluate(() => window.__testReleaseCommand("save_settings"));
  await expect(toggle(page)).toBeChecked();
  for (const state of ["loading", "listening", "finishing", "cleaning"]) {
    await page.evaluate((state) => window.__testPushSnapshot({ state }), state);
    await expect(toggle(page)).toBeDisabled();
    await expect(modelButton(page)).toBeDisabled();
    await expect(toggle(page)).toBeChecked();
  }
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "idle", meetingRecording: true }),
  );
  await expect(toggle(page)).toBeDisabled();
  await page.evaluate(() =>
    window.__testPushSnapshot({ meetingRecording: false }),
  );
  await expect(toggle(page)).toBeEnabled();
  expect(await saves(page)).toHaveLength(1);
});

test("capture starting during training confirmation prevents the save", async ({
  page,
}) => {
  await openApp(page, base, "/?page=dictation");
  await choose(page, "Muse Spark Contributor");
  await expect(trainingDialog(page)).toBeVisible();
  await page.evaluate(() =>
    window.__testPushSnapshot({ state: "listening", sessionId: 55 }),
  );
  await trainingDialog(page)
    .getByRole("button", { name: "Allow for this model" })
    .click();
  expect(await saves(page)).toHaveLength(0);
  await expect(modelButton(page)).toHaveText("GPT-6 Luna");
});

test("catalog failure offers retry and search handles no matches", async ({
  page,
}) => {
  await openApp(
    page,
    { ...base, aiErrors: { list_ai_catalog: "Catalog offline" } },
    "/?page=dictation",
  );
  const row = page.getByRole("group", { name: label, exact: true });
  await expect(row).toContainText(
    "Could not load models. Your saved choice is unchanged.",
  );
  await row.getByRole("button", { name: "Try again" }).click();
  await expect(toggle(page)).toBeEnabled();
  await modelButton(page).click();
  await page
    .getByRole("searchbox", { name: "Search OpenRouter models" })
    .fill("no such model");
  await expect(
    page.getByText("No text model matches. Try another name."),
  ).toBeVisible();
  await page.getByRole("searchbox").press("Escape");
  await expect(modelButton(page)).toBeFocused();
  await expect(
    page.getByRole("region", { name: "Proofreading models" }),
  ).toHaveCount(0);
});

const record = {
  id: "proofread-one",
  createdAt: "2026-10-21T12:00:00Z",
  raw: "send the the link",
  published: "Send the link.",
  preModelText: "Send the the link.",
  publicationStage: "proofread",
  cleanupStatus:
    "rules;proofreader_model=openai/gpt-6-luna;proofreader_outcome=accepted;proofreader_reason=accepted",
  recognitionCompleteness: "complete",
  recognitionWarnings: [],
  guardReason: null,
};

test("history compares and copies unproofread, proofread and raw versions", async ({
  page,
}) => {
  await openApp(page, { ...base, history: [record] }, "/?page=dictation");
  await showRecentDictations(page);
  await page.getByRole("button", { name: /Send the link/ }).click();
  await page.getByRole("button", { name: "Compare versions" }).click();
  const comparison = page.getByRole("region", { name: "Compare versions" });
  for (const [title, text] of [
    ["Before AI changes", record.preModelText],
    ["Proofread", record.published],
    ["As recognized", record.raw],
  ]) {
    const version = comparison
      .locator(":scope > div")
      .filter({ has: page.getByRole("heading", { name: title, exact: true }) });
    await expect(version).toContainText(text);
    await version.getByRole("button", { name: "Copy", exact: true }).click();
    expect(await page.evaluate(() => window.__testClipboard)).toBe(text);
  }
  await page.locator("details.record-more summary").click();
  await page.getByRole("button", { name: "Processing details" }).click();
  await expect(
    page.getByRole("region", { name: "Processing details" }),
  ).toContainText(defaultId);
});

test("private current result offers session-checked copy-only recovery", async ({
  page,
}) => {
  await openApp(
    page,
    {
      ...base,
      settings: { ...base.settings, doNotSaveDictations: true },
      history: [],
    },
    "/?page=dictation",
  );
  await page.evaluate(
    (record) =>
      window.__testPushSnapshot({
        state: "idle",
        resultSessionId: 42,
        sessionId: null,
        recognitionCompleteness: "complete",
        originalText: record.raw,
        outputText: record.published,
        preModelText: record.preModelText,
        publicationStage: "proofread",
      }),
    record,
  );
  const current = page.getByRole("region", { name: "Current dictation" });
  await current.getByRole("button", { name: "Review available text" }).click();
  await current.getByRole("button", { name: "Copy before AI changes" }).click();
  expect(await page.evaluate(() => window.__testClipboard)).toBe(
    record.preModelText,
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls
        .filter(({ command }) => command === "copy_smart_original")
        .map(({ args }) => args),
    ),
  ).toEqual([{ resultSessionId: 42 }]);
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "listening",
      sessionId: 43,
      resultSessionId: null,
    }),
  );
  await expect(current).toHaveCount(0);
});

test("account training block links directly to account privacy settings", async ({
  page,
}) => {
  await openApp(
    page,
    {
      ...base,
      history: [
        {
          ...record,
          publicationStage: "rules",
          published: record.preModelText,
          guardReason: "proofreader:account_training_blocked",
        },
      ],
    },
    "/?page=dictation",
  );
  await showRecentDictations(page);
  await page.getByRole("button", { name: /Send the the link/ }).click();
  const link = page.getByRole("link", {
    name: "Open account privacy settings",
  });
  await expect(link).toHaveAttribute(
    "href",
    "https://openrouter.ai/settings/privacy",
  );
  await link.click();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        ({ command }) => command === "open_openrouter_privacy",
      ),
    ),
  ).toHaveLength(1);
});

for (const theme of ["fog", "slate"]) {
  test(`proofreader visual smoke: ${theme}`, async ({ page }) => {
    const directory = new URL(
      "../../../artifacts/proofreader-feature/",
      import.meta.url,
    );
    await mkdir(directory, { recursive: true });
    await page.setViewportSize({ width: 1220, height: 1000 });
    await openApp(
      page,
      { ...base, settings: { ...base.settings, proofreader: true } },
      `/?page=dictation&theme=${theme}`,
    );
    await expect(toggle(page)).toBeChecked();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({
      animations: "disabled",
      path: fileURLToPath(new URL(`${theme}.png`, directory)),
    });
    if (theme === "fog") {
      await choose(page, "Muse Spark Contributor");
      await expect(trainingDialog(page)).toBeVisible();
      await trainingDialog(page).scrollIntoViewIfNeeded();
      await trainingDialog(page).screenshot({
        animations: "disabled",
        path: fileURLToPath(new URL("training-notice.png", directory)),
      });
      await trainingDialog(page)
        .getByRole("button", { name: "Cancel", exact: true })
        .click();
    }
  });
}
