import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";
import { goToPage, openApp } from "./app-helpers.js";

const label = "Shape by type (experimental)";
const help = "Lays out emails, lists, notes and steps with a local text model.";
const hintLabel = "Use the app I'm in (experimental)";
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
  const toggle = page.getByRole("switch", { name: label, exact: true });
  const hint = page.getByRole("switch", { name: hintLabel, exact: true });
  await expect(page.locator("details.more-options")).not.toHaveAttribute(
    "open",
    "",
  );
  await expect(toggle).toBeVisible();
  await expect(hint).toBeVisible();
  await expect(hint).toBeDisabled();
  await expect(hint).not.toBeChecked();
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText(help);
  await toggle.click();
  await expect(toggle).toBeChecked();
  await expect(hint).toBeEnabled();
  expect(await savedSettings(page)).toEqual([
    expect.objectContaining({
      smartLayout: true,
      cleanupModel: cleanupId,
      notesModel: notesId,
    }),
  ]);
  await hint.click();
  await expect(hint).toBeChecked();
  expect((await savedSettings(page)).at(-1)).toMatchObject({
    smartLayoutAppHint: true,
  });

  await goToPage(page, "Models");
  await goToPage(page, "Dictation");
  await expect(toggle).toBeChecked();
  await expect(hint).toBeChecked();
  const modes = page.getByRole("group", { name: "Dictation cleanup mode" });
  await modes.getByRole("button", { name: /^Format only/ }).click();
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await modes.getByRole("button", { name: /^As heard/ }).click();
  await expect(toggle).toHaveCount(0);
  await expect(hint).toBeDisabled();
  await expect(hint).toBeChecked();
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText("Choose Format only or Clean to use Shape by type.");
  await page
    .getByRole("button", { name: "Use Format only", exact: true })
    .click();
  await expect(
    modes.getByRole("button", { name: /^Format only/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await expect(hint).toBeEnabled();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(hint).toBeDisabled();
  await expect(hint).toBeChecked();
  expect((await savedSettings(page)).at(-1)).toMatchObject({
    smartLayout: false,
  });
});

test("Shape by type waits for a settings save and disables during dictation", async ({
  page,
}) => {
  await openApp(page, { holdCommands: ["save_settings"] }, "/?page=dictation");
  const toggle = page.getByRole("switch", { name: label, exact: true });
  const hint = page.getByRole("switch", { name: hintLabel, exact: true });
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect.poll(async () => (await savedSettings(page)).length).toBe(1);
  await expect(toggle).toBeDisabled();
  await expect(hint).toBeDisabled();
  await expect(toggle).not.toBeChecked();
  await page.evaluate(() => window.__testReleaseCommand("save_settings"));
  await expect(toggle).toBeEnabled();
  await expect(toggle).toBeChecked();
  await expect(hint).toBeEnabled();
  for (const state of ["loading", "listening", "finishing", "cleaning"]) {
    await page.evaluate((state) => window.__testPushSnapshot({ state }), state);
    await expect(toggle).toBeDisabled();
    await expect(hint).toBeDisabled();
    await expect(toggle).toBeChecked();
  }
  await page.evaluate(() => window.__testPushSnapshot({ state: "idle" }));
  await expect(toggle).toBeEnabled();
  expect(await savedSettings(page)).toHaveLength(1);
});

for (const modelState of ["checking", "not-installed"]) {
  test(`Shape by type offers model recovery while selected local models are ${modelState}`, async ({
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
    const toggle = page.getByRole("switch", { name: label, exact: true });
    await expect(toggle).toHaveCount(0);
    await expect(page.getByRole("switch", { name: hintLabel })).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Choose text model", exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole("group", { name: label, exact: true }),
    ).toContainText("Choose a ready local text model for Cleanup or Notes.");
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
  await expect(toggle).toHaveCount(0);
  await expect(
    page.getByRole("group", { name: label, exact: true }),
  ).toContainText("Choose a ready local text model for Cleanup or Notes.");
  expect(await savedSettings(page)).toHaveLength(0);
  await page
    .getByRole("button", { name: "Choose text model", exact: true })
    .click();
  await expect(page.locator("#setup-notes")).toBeFocused();
  await page.getByRole("button", { name: "Notes model", exact: true }).click();
  const picker = page.getByRole("dialog", {
    name: "Notes models",
    exact: true,
  });
  await expect(picker).toBeVisible();
  const localNotes = picker.getByRole("option", { name: /Qwen 3 4B/ });
  await expect(localNotes).toBeVisible();
  await expect(
    picker.getByRole("option", { name: /Moonshine|Smart cleanup/ }),
  ).toHaveCount(0);
  await localNotes.click();
  await goToPage(page, "Dictation");
  await expect(toggle).toBeEnabled();
});

test("the app hint saves with the keyboard and waits for the host reply", async ({
  page,
}) => {
  await openApp(
    page,
    { settings: { smartLayout: true }, holdCommands: ["save_settings"] },
    "/?page=dictation",
  );
  const toggle = page.getByRole("switch", { name: label, exact: true });
  const hint = page.getByRole("switch", { name: hintLabel, exact: true });
  await hint.focus();
  await hint.press("Space");
  await expect.poll(async () => (await savedSettings(page)).length).toBe(1);
  expect((await savedSettings(page))[0]).toMatchObject({
    smartLayout: true,
    smartLayoutAppHint: true,
  });
  await expect(hint).toBeDisabled();
  await expect(hint).not.toBeChecked();
  await expect(toggle).toBeDisabled();
  await page.evaluate(() => window.__testReleaseCommand("save_settings"));
  await expect(hint).toBeEnabled();
  await expect(hint).toBeChecked();
});

for (const theme of ["fog", "slate"]) {
  test(`Shape by type visual smoke: ${theme}`, async ({ page }) => {
    const directory = new URL("../../../artifacts/app-hint/", import.meta.url);
    await mkdir(directory, { recursive: true });
    await openApp(
      page,
      { settings: { smartLayout: true, smartLayoutAppHint: true } },
      `/?page=dictation&theme=${theme}`,
    );
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
      ).toHaveCount(0);
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
      ).toContainText("Choose a ready local text model");
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
