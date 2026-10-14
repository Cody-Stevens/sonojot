import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

const smartId = "speakoflow-mini-q4_k_m";
const smart = {
  id: smartId,
  name: "Smart cleanup (experimental)",
  kind: "cleanup",
  roles: ["cleanup"],
  family: "SpeakoFlow",
  languages: ["English"],
  older: false,
  diskBytes: 500_000_000,
  ramMinBytes: 600_000_000,
  ramMaxBytes: 1_100_000_000,
  vramMinBytes: null,
  vramMaxBytes: null,
  memoryBasis: "Fictional test values",
  guidance: "Fixture",
  evidence: "Fixture",
  backend: "cpu",
  compatibility: "Fixture",
  state: "installed",
  downloadedBytes: 500_000_000,
  totalBytes: 500_000_000,
  error: null,
  installable: false,
  removable: true,
  usedBy: [],
  revision: "fixture",
  quantization: "Q4_K_M",
  contextTokens: 4096,
};
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

test("Custom offers Smart without selecting it and can return to rules", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    libraryModels: [smart],
    settings: { cleanupModel: "local-rules" },
  });
  await page.goto("/?page=setup");
  await expect
    .poll(() =>
      page.evaluate(() => window.__testSnapshot().settings.cleanupModel),
    )
    .toBe("local-rules");
  await page
    .getByRole("button", { name: "Cleanup model", exact: true })
    .click();
  await page.getByRole("option", { name: /Smart cleanup/ }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.__testSnapshot().settings.cleanupModel),
    )
    .toBe(smartId);
  await expect(page.getByText(
    "Small cleanup model; may miss or occasionally change wording. Numbers, names and links are checked.",
  )).toBeVisible();
  await screenshot(page, "custom-selected");
  await page.getByRole("button", { name: "Notes model", exact: true }).click();
  await expect(page.getByRole("option", { name: /Smart cleanup/ })).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Cleanup model", exact: true }).click();
  await page.getByRole("option", { name: /Local rules|Formatting and vocabulary/ }).click();
  await expect
    .poll(() =>
      page.evaluate(() => window.__testSnapshot().settings.cleanupModel),
    )
    .toBe("local-rules");
});

for (const profile of ["accurate", "balanced", "light"]) {
  test(`${profile} offers rules without the experimental switch`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      libraryModels: [smart],
      settings: { setupProfile: profile, cleanupModel: "local-rules" },
    });
    await page.goto("/?page=setup");
    await expect(
      page.getByRole("group", { name: "Selected profile details" }),
    ).toContainText(
      "Cleanup: SonoJot Punctuation v2 with your vocabulary · Installed",
    );
    await expect(
      page.getByRole("switch", { name: "Experimental Smart cleanup" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Cleanup model", exact: true }),
    ).toHaveCount(0);
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
      name: "Use original",
      exact: true,
    });
    await expect(recovery).toBeVisible();
    await expect(recovery).toHaveAttribute("title", "Copy original text");
    await page.evaluate(() => document.fonts.ready);
    const sheet = await page.locator(".tab-sheet").boundingBox();
    const lead = await recovery.boundingBox();
    const actions = await page.locator(".tab-actions").boundingBox();
    expect(sheet.height).toBe(48);
    expect(lead.x + lead.width + 8).toBeLessThanOrEqual(actions.x);
    expect(actions.x + actions.width).toBeLessThanOrEqual(
      sheet.x + sheet.width,
    );
    await screenshot(page, `idle-${theme}`, page.locator(".screen-tab"));
    await recovery.click();
    await expect(page.getByRole("status")).toHaveText("Original copied");
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

test("delayed recovery is refused after a newer dictation", async ({
  page,
}) => {
  await installDesktopFixture(page, { holdCommands: ["copy_smart_original"] });
  await page.goto("/?view=overlay");
  await page.evaluate((patch) => window.__testPushSnapshot(patch), result);
  await page.getByRole("button", { name: "Use original" }).click();
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
  await page
    .locator(".history-item")
    .filter({ hasText: result.outputText })
    .click();
  await expect(
    page.getByRole("heading", { name: "As recognized", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Original", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Cleaned", exact: true }),
  ).toBeVisible();
  const original = page.locator(".history-comparison > div").filter({
    has: page.getByRole("heading", { name: "Original", exact: true }),
  });
  await expect(original).toContainText(result.preModelText);
  await original.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await calls(page, "copy_history_pre_model")).toEqual([
    { command: "copy_history_pre_model", args: { id: record.id } },
  ]);
  await screenshot(page, "history-comparison");
  await page.getByRole("button", { name: "All recent dictations" }).click();
  await page
    .locator(".history-item")
    .filter({ hasText: "Keep zero point zero four volts." })
    .click();
  await expect(page.getByText(/Rules ·/)).toContainText(
    "Number change blocked",
  );
  await expect(
    page.getByRole("heading", { name: "Cleaned", exact: true }),
  ).toHaveCount(0);
});
