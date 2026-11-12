import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { proofreaderCatalog } from "./desktop-fixture.js";
import { goToPage, openApp, showRecentDictations } from "./app-helpers.js";

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
    quickModels: [
      { id: "mock/notes", role: "text" },
      { id: "mock/proofreader", role: "text" },
      { id: trainingId, role: "text" },
      { id: "mock/another-training-model", role: "text" },
    ],
  },
  aiCatalog: proofreaderCatalog,
  settings: {
    cleanupModel: "local-rules",
    componentAcknowledgments: { "openrouter-cloud": "1" },
  },
};

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
  const detail = page.getByRole("region", { name: "Open dictation" });
  for (const [title, text] of [
    ["Before cleanup", record.preModelText],
    ["Final", record.published],
    ["As heard", record.raw],
  ]) {
    await detail.getByRole("button", { name: title, exact: true }).click();
    await expect(detail.locator(".full-text")).toHaveText(text);
    await detail.getByRole("button", { name: "Copy", exact: true }).click();
    expect(await page.evaluate(() => window.__testClipboard)).toBe(text);
  }
  await detail.locator("summary", { hasText: "More" }).click();
  await detail.locator("summary", { hasText: "Processing details" }).click();
  await expect(detail).toContainText(defaultId);
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
