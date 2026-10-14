import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";
import { goToPage, openApp, openMoreOptions } from "./app-helpers.js";

const label = "Shape by type (experimental)";
const help =
  "Detects emails, lists, notes and steps, then lays them out. Uses your text model; your words stay the same.";
const cleanupId = "lfm2.5-1.2b";
const notesId = "qwen3-4b-instruct-2507";
const savedSettings = (page) =>
  page.evaluate(() =>
    window.__testCalls
      .filter((call) => call.command === "save_settings")
      .map((call) => call.args.settings),
  );

test("Shape by type saves, survives navigation and keeps its choice in As heard", async ({
  page,
}) => {
  await openApp(page, {}, "/?page=dictation");
  await openMoreOptions(page);
  const toggle = page.getByRole("switch", { name: label, exact: true });
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText(help);
  await toggle.click();
  await expect(toggle).toBeChecked();
  expect(await savedSettings(page)).toEqual([
    expect.objectContaining({
      smartLayout: true,
      cleanupModel: cleanupId,
      notesModel: notesId,
    }),
  ]);

  await goToPage(page, "Models");
  await goToPage(page, "Dictation");
  await openMoreOptions(page);
  await expect(toggle).toBeChecked();
  const modes = page.getByRole("group", { name: "Dictation cleanup mode" });
  await modes.getByRole("button", { name: /^Format only/ }).click();
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await modes.getByRole("button", { name: /^As heard/ }).click();
  await expect(toggle).toBeDisabled();
  await expect(toggle).toBeChecked();
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText("Choose Format only or Clean to use Shape by type.");
  await modes.getByRole("button", { name: /^Clean/ }).click();
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  expect((await savedSettings(page)).at(-1)).toMatchObject({
    smartLayout: false,
  });
});

test("Shape by type waits for a settings save and disables during dictation", async ({
  page,
}) => {
  await openApp(page, { holdCommands: ["save_settings"] }, "/?page=dictation");
  await openMoreOptions(page);
  const toggle = page.getByRole("switch", { name: label, exact: true });
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect.poll(async () => (await savedSettings(page)).length).toBe(1);
  await expect(toggle).toBeDisabled();
  await expect(toggle).not.toBeChecked();
  await page.evaluate(() => window.__testReleaseCommand("save_settings"));
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  for (const state of ["loading", "listening", "finishing", "cleaning"]) {
    await page.evaluate((state) => window.__testPushSnapshot({ state }), state);
    await expect(toggle).toBeDisabled();
    await expect(toggle).toBeChecked();
  }
  await page.evaluate(() => window.__testPushSnapshot({ state: "idle" }));
  await expect(toggle).toBeEnabled();
  expect(await savedSettings(page)).toHaveLength(1);
});

for (const modelState of ["checking", "not-installed"]) {
  test(`Shape by type stays disabled while selected local models are ${modelState}`, async ({
    page,
  }) => {
    await installDesktopFixture(page, {
      settings: { smartLayout: true },
      modelPatches: {
        [cleanupId]: { state: modelState },
        [notesId]: { state: modelState },
      },
    });
    await page.goto("/?page=dictation");
    await openMoreOptions(page);
    const toggle = page.getByRole("switch", { name: label, exact: true });
    await expect(toggle).toBeDisabled();
    await expect(toggle).toBeChecked();
    await expect(
      page.getByRole("group", { name: label, exact: true }),
    ).toContainText(
      "Select an installed local text model for Cleanup or Notes in Models.",
    );
    expect(await savedSettings(page)).toHaveLength(0);

    // A completed background check refreshes the mounted library without navigation.
    await page.evaluate((id) => {
      window.__testPatchModel(id, { state: "installed" });
      window.dispatchEvent(new Event("focus"));
    }, notesId);
    await expect(toggle).toBeEnabled();
    await expect(toggle).toBeChecked();
    expect(await savedSettings(page)).toHaveLength(0);
  });
}

test("Shape by type uses selected Notes with rules or Smart cleanup and excludes remote-only selection", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    settings: { cleanupModel: "local-rules" },
    // Reuse fixture metadata; specialize the entry to match SpeakoFlow's actual roles.
    modelPatches: {
      [cleanupId]: {
        id: "speakoflow-mini-q4_k_m",
        roles: ["cleanup"],
        family: "SpeakoFlow",
      },
    },
  });
  await page.goto("/?page=dictation");
  await openMoreOptions(page);
  const toggle = page.getByRole("switch", { name: label, exact: true });
  await expect(toggle).toBeEnabled();
  await page.evaluate(() =>
    window.__testPushSnapshot({
      settings: {
        ...window.__testSnapshot().settings,
        cleanupModel: "speakoflow-mini-q4_k_m",
      },
    }),
  );
  await expect(toggle).toBeEnabled();
  await page.evaluate(() =>
    window.__testPushSnapshot({
      settings: {
        ...window.__testSnapshot().settings,
        notesModel: "openrouter",
      },
    }),
  );
  await expect(toggle).toBeDisabled();
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText("Smart cleanup alone cannot provide it.");
  expect(await savedSettings(page)).toHaveLength(0);
});

for (const theme of ["fog", "slate"]) {
  test(`Shape by type visual smoke: ${theme}`, async ({ page }) => {
    const directory = new URL(
      "../../../artifacts/smart-layout/",
      import.meta.url,
    );
    await mkdir(directory, { recursive: true });
    await openApp(page, { settings: { smartLayout: true } }, `/?page=dictation&theme=${theme}`);
    await openMoreOptions(page);
    await expect(
      page.getByRole("switch", { name: label, exact: true }),
    ).toBeChecked();
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({
      animations: "disabled",
      path: fileURLToPath(new URL(`${theme}.png`, directory)),
    });
    if (theme === "fog") {
      await page.setViewportSize({ width: 680, height: 900 });
      await page
        .getByRole("group", { name: "Dictation cleanup mode" })
        .getByRole("button", { name: /^As heard/ })
        .click();
      await expect(
        page.getByRole("switch", { name: label, exact: true }),
      ).toBeDisabled();
      await page.screenshot({
        animations: "disabled",
        path: fileURLToPath(new URL("narrow-as-heard.png", directory)),
      });
      await page.evaluate(() =>
        window.__testPushSnapshot({
          settings: {
            ...window.__testSnapshot().settings,
            cleanupMode: "clean",
            cleanupModel: "local-rules",
            notesModel: "openrouter",
          },
        }),
      );
      await expect(
        page.getByRole("group", { name: label, exact: true }),
      ).toContainText("Select an installed local text model");
      await page.screenshot({
        animations: "disabled",
        path: fileURLToPath(new URL("narrow-no-model.png", directory)),
      });
    }
  });
}

test("a shaped result keeps session-checked copy-only recovery", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    settings: { smartLayout: true, doNotSaveDictations: true, autoPaste: true },
    history: [],
  });
  await page.goto("/?view=overlay&theme=fog");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      resultSessionId: 42,
      sessionId: null,
      recognitionCompleteness: "complete",
      publicationStage: "smart_layout",
      originalText: "hi alex please send the draft thanks sam",
      preModelText: "Hi Alex please send the draft thanks Sam.",
      outputText: "Hi Alex,\n\nPlease send the draft.\n\nThanks,\nSam",
      guardReason: null,
      tabVisible: true,
      tabExpanded: true,
      tabTemporary: true,
      canCopyLast: true,
    }),
  );
  const recovery = page.getByRole("button", {
    name: "Copy before AI changes",
    exact: true,
  });
  await expect(recovery).toBeVisible();
  await recovery.click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__testCalls
          .filter((call) => call.command === "copy_smart_original")
          .map((call) => call.args),
      ),
    )
    .toEqual([{ resultSessionId: 42 }]);
  await expect(page.getByText("Copied", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__testClipboard)).toBe(
    "Hi Alex please send the draft thanks Sam.",
  );
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter((call) => call.command === "copy_text"),
    ),
  ).toHaveLength(0);
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "listening",
      sessionId: 43,
      resultSessionId: null,
    }),
  );
  await expect(recovery).toHaveCount(0);
});
