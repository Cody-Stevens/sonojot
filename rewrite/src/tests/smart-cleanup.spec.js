import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";
import { showRecentDictations } from "./app-helpers.js";

const result = {
  state: "idle",
  resultSessionId: 7,
  sessionId: null,
  recognitionCompleteness: "complete",
  originalText: "please send the summary send the summary before lunch",
  preModelText: "Please send the summary send the summary before lunch.",
  outputText: "Please send the summary before lunch.",
  publicationStage: "smart_cleanup",
  guardReason: null,
  tabVisible: true,
  tabExpanded: true,
  canCopyLast: true,
};
const record = {
  id: "smart-one",
  createdAt: "2026-09-28T20:00:00Z",
  raw: result.originalText,
  published: result.outputText,
  preModelText: result.preModelText,
  publicationStage: result.publicationStage,
  guardReason: null,
  recognitionCompleteness: "complete",
  recognitionWarnings: [],
};
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
async function screenshot(page, name, locator = page) {
  const directory = new URL(
    "../../../artifacts/smart-cleanup/",
    import.meta.url,
  );
  await mkdir(directory, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await locator.screenshot({
    path: fileURLToPath(new URL(`${name}.png`, directory)),
  });
}

for (const theme of ["fog", "slate"]) {
  test(`latest private recovery copies only and fits the idle tab: ${theme}`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      settings: { doNotSaveDictations: true, autoPaste: true },
      history: [],
    });
    await page.goto(`/?view=overlay&theme=${theme}`);
    await page.evaluate((patch) => window.__testPushSnapshot(patch), {
      ...result,
      tabTemporary: true,
    });
    const recovery = page.getByRole("button", {
      name: "Copy before AI changes",
      exact: true,
    });
    await expect(recovery).toBeVisible();
    await expect(recovery).toHaveAttribute(
      "title",
      "Copy text from before cleanup, proofreading or layout changes",
    );
    await page.evaluate(() => document.fonts.ready);
    const sheet = await page.locator(".tab-sheet").boundingBox();
    const actions = await page.locator(".tab-actions").boundingBox();
    expect(sheet.height).toBe(48);
    expect(actions.x + actions.width).toBeLessThanOrEqual(
      sheet.x + sheet.width,
    );
    await screenshot(page, `idle-${theme}`, page.locator(".screen-tab"));
    await recovery.click();
    await expect(page.getByRole("status")).toHaveText("Copied");
    expect(await page.evaluate(() => window.__testClipboard)).toBe(
      result.preModelText,
    );
    expect(await calls(page, "copy_smart_original")).toEqual([
      { command: "copy_smart_original", args: { resultSessionId: 7 } },
    ]);
    expect(await calls(page, "copy_text")).toHaveLength(0);
    await page.evaluate(() =>
      window.__testPushSnapshot({
        state: "listening",
        sessionId: 8,
        resultSessionId: null,
      }),
    );
    await expect(recovery).toHaveCount(0);
  });
}

// "Copy before AI changes" is longer than the old "Use original". The 600px tab
// (Windows and the Mac menu bar panel alike) makes room by showing Keep and
// Hide as icons while the link is up.
for (const theme of ["fog", "slate"]) {
  for (const temporary of [true, false]) {
    test(`the recovery link clears the idle tab actions: ${theme}${temporary ? ", revealed" : ""}`, async ({
      page,
    }) => {
      await installDesktopFixture(page, {
        settings: { doNotSaveDictations: true, autoPaste: true },
        history: [],
      });
      await page.goto(`/?view=overlay&theme=${theme}`);
      await page.evaluate((patch) => window.__testPushSnapshot(patch), {
        ...result,
        tabTemporary: temporary,
      });
      const recovery = page.getByRole("button", {
        name: "Copy before AI changes",
        exact: true,
      });
      await expect(recovery).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      const lead = await recovery.boundingBox();
      const sheet = await page.locator(".tab-sheet").boundingBox();
      const buttons = page.locator(".tab-actions button");
      const first = await buttons.first().boundingBox();
      const last = await buttons.last().boundingBox();
      expect(lead.x + lead.width + 8).toBeLessThanOrEqual(first.x);
      expect(last.x + last.width).toBeLessThanOrEqual(sheet.x + sheet.width);
      await expect(
        page.getByRole("button", { name: "Hide", exact: true }),
      ).toHaveAttribute("title", "Hide");
      await expect(
        page.getByRole("button", { name: "Keep visible", exact: true }),
      ).toHaveCount(temporary ? 1 : 0);
      await screenshot(
        page,
        `idle-actions-${theme}${temporary ? "-revealed" : ""}`,
        page.locator(".screen-tab"),
      );
      // A new capture clears recovery and the labels come back.
      await page.evaluate(() =>
        window.__testPushSnapshot({
          resultSessionId: null,
          publicationStage: "rules",
          preModelText: null,
        }),
      );
      await expect(recovery).toHaveCount(0);
      await expect(buttons.last()).toHaveText("Hide");
    });
  }
}

test("delayed recovery is refused after a newer dictation", async ({
  page,
}) => {
  await installDesktopFixture(page, { holdCommands: ["copy_smart_original"] });
  await page.goto("/?view=overlay");
  await page.evaluate((patch) => window.__testPushSnapshot(patch), result);
  await page.getByRole("button", { name: "Copy before AI changes" }).click();
  await expect
    .poll(async () => (await calls(page, "copy_smart_original")).length)
    .toBe(1);
  await page.evaluate(() => {
    window.__testPushSnapshot({
      resultSessionId: 8,
      publicationStage: "rules",
      preModelText: null,
    });
    window.__testReleaseCommand("copy_smart_original");
  });
  await expect(page.getByRole("alert")).toBeVisible();
  expect(await page.evaluate(() => window.__testClipboard)).toBeUndefined();
});

test("history separates raw recognition, rules original, Cleaned and guard fallback", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    history: [
      record,
      {
        ...record,
        id: "fallback",
        createdAt: "2026-09-27T20:00:00Z",
        publicationStage: "rules",
        guardReason: "number_changed",
        published: "Keep zero point zero four volts.",
      },
    ],
  });
  await page.goto("/?page=dictation");
  const history = await showRecentDictations(page);
  await history
    .getByRole("button", {
      name: `Open dictation: ${result.outputText}`,
      exact: true,
    })
    .click();
  let detail = page.getByRole("region", { name: "Open dictation" });
  await detail
    .getByRole("button", { name: "Before cleanup", exact: true })
    .click();
  await expect(detail.locator(".full-text")).toHaveText(result.preModelText);
  await detail.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await calls(page, "copy_history_pre_model")).toEqual([
    { command: "copy_history_pre_model", args: { id: record.id } },
  ]);
  await detail.getByRole("button", { name: "As heard", exact: true }).click();
  await expect(detail.locator(".full-text")).toHaveText(record.raw);
  await detail.getByRole("button", { name: "Final", exact: true }).click();
  await expect(detail.locator(".full-text")).toHaveText(record.published);
  await detail.getByRole("button", { name: "Close dictation" }).click();
  await history
    .getByRole("button", {
      name: "Open dictation: Keep zero point zero four volts.",
      exact: true,
    })
    .click();
  detail = page.getByRole("region", { name: "Open dictation" });
  await detail.locator("summary", { hasText: "More" }).click();
  await detail.locator("summary", { hasText: "Processing details" }).click();
  await expect(detail.getByRole("definition")).toHaveText([
    "Complete",
    "Rules",
    "Number change blocked",
  ]);
});
