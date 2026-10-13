import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

const id = "sonojot-punct-v2";
const modelPage = "https://huggingface.co/SonoJot/sonojot-punct-v2";
const help = "Fixes punctuation and helps write numbers and symbols in Clean dictation; runs on CPU";

function legacyPunctuation(state = "installed") {
  return {
    id: "sonojot-punct-v1", name: "SonoJot Punctuation", kind: "punctuation", roles: ["punctuation"],
    family: "Token classifier", languages: ["English"], older: false,
    diskBytes: 302708257, totalBytes: 302708257, downloadedBytes: state === "installed" ? 302708257 : 0,
    ramMinBytes: 400000000, ramMaxBytes: 400000000, vramMinBytes: 0, vramMaxBytes: 0,
    memoryBasis: "CPU only.", guidance: "Fixes sentence breaks and commas in dictation; runs on CPU",
    evidence: "CC BY-SA 4.0; https://huggingface.co/SonoJot/sonojot-punct-v1",
    backend: "onnxruntime-cpu", compatibility: "Requires the shared ONNX Runtime; removal keeps it.",
    state, error: null, installable: state !== "installed", removable: state === "installed",
    usedBy: [], revision: "fixture", quantization: "int8", contextTokens: null,
  };
}

test("published v2 and v1 expose separate downloads, licenses and model pages", async ({ page }) => {
  await openSetup(page, { libraryModels: [legacyPunctuation("available")] });
  const library = page.getByRole("region", { name: "Model library", exact: true });
  await library.getByRole("tab", { name: "Browse", exact: true }).click();
  await library.getByRole("combobox", { name: "Filter model library" }).selectOption("punctuation");
  for (const [version, name] of [["v1", "SonoJot Punctuation"], ["v2", "SonoJot Punctuation v2"]]) {
    const row = library.locator(`[data-model-id="sonojot-punct-${version}"]`);
    await expect(row).toContainText("303 MB to download");
    await expect(row.getByRole("button", { name: `Install ${name}`, exact: true })).toBeEnabled();
    await row.locator("summary").click();
    await expect(row).toContainText("CC BY-SA 4.0");
    await expect(row.getByRole("link")).toHaveAttribute("href", `https://huggingface.co/SonoJot/sonojot-punct-${version}`);
  }
  expect(await installs(page)).toHaveLength(0);
  await library.scrollIntoViewIfNeeded();
  await page.screenshot({ path: "../artifacts/v2-release/library-fog.png", fullPage: true });
  await page.evaluate(() => window.__testPushSnapshot({
    settings: { ...window.__testSnapshot().settings, theme: "slate" },
  }));
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  await page.screenshot({ path: "../artifacts/v2-release/library-slate.png", fullPage: true });
});

test("v1-only install gets an opt-in v2 upgrade despite its previous acknowledgment and keeps v1", async ({ page }) => {
  await openSetup(page, {
    settings: { punctuationRecommendationAcknowledged: true },
    libraryModels: [legacyPunctuation()],
  });
  const before = await settings(page);
  await expect(recommendation(page)).toContainText("SonoJot Punctuation v2");
  await expect(recommendation(page)).toContainText("Adds written numbers, money and dates.");
  expect(await installs(page)).toHaveLength(0);
  await page.screenshot({ path: "../artifacts/v2-release/recommendation-fog.png", fullPage: true });
  await recommendation(page).getByRole("button", { name: "Install punctuation", exact: true }).click();
  await expect(download(page)).toHaveAttribute("data-model-id", id);
  await finishPunctuation(page);
  await expect(download(page)).toContainText("Installed");
  const models = await page.evaluate(() => window.__testLibrary().models);
  expect(models.find((model) => model.id === "sonojot-punct-v1").state).toBe("installed");
  expect(models.find((model) => model.id === id).state).toBe("installed");
  expect(await settings(page)).toEqual({ ...before, punctuationV2RecommendationAcknowledged: true });
  await expect(recommendation(page)).toHaveCount(0);
});

const settings = (page) =>
  page.evaluate(() => window.__testSnapshot().settings);
const installs = (page) =>
  page.evaluate(() =>
    window.__testCalls.filter((call) => call.command === "start_model_install"),
  );
const recommendation = (page) =>
  page.getByRole("region", { name: "Recommended download", exact: true });
const download = (page) =>
  page.getByRole("region", { name: "Model download", exact: true });
async function openSetup(page, options = {}) {
  await installDesktopFixture(page, {
    punctuation: { state: "available", downloadedBytes: 0 },
    ...options,
  });
  await page.goto("/?page=setup");
  await expect(page.getByRole("button", { name: /^Balanced/ })).toBeEnabled();
  await expect(page.getByLabel("Computer hardware")).toContainText(
    "NVIDIA test GPU",
  );
}
async function finishPunctuation(page) {
  await page.evaluate((id) => {
    const model = window
      .__testLibrary()
      .models.find((model) => model.id === id);
    window.__testLibraryProgress(id, model.totalBytes, "installed");
    window.dispatchEvent(new Event("focus"));
  }, id);
}

for (const chosen of ["Accurate", "Balanced", "Light"]) {
  test(`first-run ${chosen} includes CPU punctuation in the download and saves only after it installs`, async ({
    page,
  }) => {
    await openSetup(page, {
      settings: { setupProfile: null },
      modelPatches: {
        "qwen3-asr-1.7b-fp16-cuda": { state: "installed" },
        "mock-whisper-small-cuda": { state: "installed" },
        "mock-cleanup-2b-cuda": { state: "installed" },
      },
    });
    await page
      .getByRole("group", { name: "Setup profile", exact: true })
      .getByRole("button", { name: new RegExp(`^${chosen}`) })
      .click();
    const row = page.getByRole("group", { name: "Punctuation", exact: true });
    await expect(row).toContainText(
      "SonoJot Punctuation v2 · CPU · Download included",
    );
    await expect(row).toContainText(help);
    await expect(page.locator(".profile-action")).toContainText(
      "303 MB to download",
    );
    await row.locator("summary").click();
    await expect(row).toContainText("CC BY-SA 4.0");
    await expect(
      row.getByRole("link", { name: "Model page ↗" }),
    ).toHaveAttribute("href", modelPage);
    expect(await installs(page)).toHaveLength(0);
    await page
      .getByRole("button", { name: "Finish setup", exact: true })
      .click();
    await expect(download(page)).toHaveAttribute("data-model-id", id);
    await expect(download(page)).toContainText("Downloading");
    expect((await settings(page)).setupProfile).toBeNull();
    await finishPunctuation(page);
    await expect
      .poll(async () => (await settings(page)).setupProfile)
      .toBe(chosen.toLowerCase());
    expect((await settings(page)).punctuationV2RecommendationAcknowledged).toBe(
      true,
    );
    expect(await installs(page)).toEqual([
      { command: "start_model_install", args: { id } },
    ]);
    await expect(recommendation(page)).toHaveCount(0);
  });
}

for (const saved of ["custom", "accurate", "balanced", "light"]) {
  test(`existing ${saved} offers punctuation without downloading or changing choices`, async ({
    page,
  }) => {
    await openSetup(page, { settings: { setupProfile: saved } });
    const before = await settings(page);
    await expect(recommendation(page)).toContainText(
      "Recommended: SonoJot Punctuation v2 · CPU · 303 MB to download",
    );
    await expect(recommendation(page)).toContainText(help);
    expect(await installs(page)).toHaveLength(0);
    await page.getByRole("button", { name: "Dictation", exact: true }).click();
    await page.getByRole("button", { name: "Setup", exact: true }).click();
    await expect(recommendation(page)).toBeVisible();
    expect(await settings(page)).toEqual(before);
    expect(await installs(page)).toHaveLength(0);
  });
}

test("applying a different completed profile still requires a separate punctuation install", async ({
  page,
}) => {
  await openSetup(page, { settings: { setupProfile: "balanced" } });
  await page
    .getByRole("group", { name: "Setup profile", exact: true })
    .getByRole("button", { name: "Light", exact: true })
    .click();
  await page.getByRole("button", { name: "Use this", exact: true }).click();
  await expect
    .poll(async () => (await settings(page)).setupProfile)
    .toBe("light");
  await expect(recommendation(page)).toBeVisible();
  expect((await settings(page)).punctuationV2RecommendationAcknowledged).toBe(
    false,
  );
  expect(await installs(page)).toHaveLength(0);
});

test("recommended punctuation does not resume a cancelled profile change", async ({
  page,
}) => {
  await openSetup(page, { settings: { setupProfile: "balanced" } });
  const before = await settings(page);
  await page
    .getByRole("group", { name: "Setup profile", exact: true })
    .getByRole("button", { name: "Accurate", exact: true })
    .click();
  await page.getByRole("button", { name: "Use this", exact: true }).click();
  await expect(download(page)).toContainText("Downloading");
  await download(page)
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(
    download(page).getByRole("button", { name: "Retry", exact: true }),
  ).toBeEnabled();
  const previousInstalls = await installs(page);
  await recommendation(page)
    .getByRole("button", { name: "Install punctuation", exact: true })
    .click();
  await expect(download(page)).toHaveAttribute("data-model-id", id);
  await finishPunctuation(page);
  await expect(download(page)).toContainText("Installed");
  expect(await installs(page)).toEqual([
    ...previousInstalls,
    { command: "start_model_install", args: { id } },
  ]);
  expect(await settings(page)).toEqual({
    ...before,
    punctuationV2RecommendationAcknowledged: true,
  });
});

test("a failed acknowledgment save leaves the recommendation available and starts no download", async ({
  page,
}) => {
  await openSetup(page);
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "save_settings")
        throw new Error("Mock settings write failed");
      return invoke(command, args);
    };
  });
  await recommendation(page)
    .getByRole("button", { name: "Install punctuation", exact: true })
    .click();
  await expect(recommendation(page)).toBeVisible();
  await expect(page.getByRole("alert")).toContainText(
    "The setting could not be saved",
  );
  expect((await settings(page)).punctuationV2RecommendationAcknowledged).toBe(
    false,
  );
  expect(await installs(page)).toHaveLength(0);
});

test("dismissal is saved once and remains dismissed when the app opens again", async ({
  page,
  context,
}) => {
  await openSetup(page);
  const before = await settings(page);
  await recommendation(page)
    .getByRole("button", { name: "Dismiss", exact: true })
    .click();
  await expect(recommendation(page)).toHaveCount(0);
  const saved = await settings(page);
  expect(saved).toEqual({
    ...before,
    punctuationV2RecommendationAcknowledged: true,
  });
  expect(await installs(page)).toHaveLength(0);
  const restarted = await context.newPage();
  await openSetup(restarted, { settings: saved });
  await expect(recommendation(restarted)).toHaveCount(0);
  expect(await installs(restarted)).toHaveLength(0);
  await restarted.close();
});

test("recommended download uses cancel and retry, preserves choices and does not repeat after removal", async ({
  page,
}) => {
  await openSetup(page);
  const before = await settings(page);
  await recommendation(page)
    .getByRole("button", { name: "Install punctuation", exact: true })
    .click();
  await expect(download(page)).toContainText("Downloading");
  await expect(recommendation(page)).toHaveCount(0);
  await download(page)
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(download(page)).toContainText("Cancelled");
  expect(await settings(page)).toEqual({
    ...before,
    punctuationV2RecommendationAcknowledged: true,
  });
  await download(page)
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(download(page)).toContainText("Downloading");
  await finishPunctuation(page);
  await expect(download(page)).toContainText("Installed");
  const library = page.getByRole("region", {
    name: "Model library",
    exact: true,
  });
  await library
    .getByRole("combobox", { name: "Filter model library" })
    .selectOption("punctuation");
  const row = library.locator(`[data-model-id="${id}"]`);
  await expect(row).toContainText("CPU · 303 MB");
  await row
    .getByRole("button", { name: "Remove SonoJot Punctuation v2", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Remove", exact: true })
    .click();
  await expect(row).toHaveCount(0);
  await expect(recommendation(page)).toHaveCount(0);
  expect(await settings(page)).toEqual({
    ...before,
    punctuationV2RecommendationAcknowledged: true,
  });
});

test("punctuation library row discloses license, source and runtime download size without a new selector", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 640, height: 900 });
  await openSetup(page, {
    settings: { punctuationV2RecommendationAcknowledged: true },
    punctuation: {
      state: "available",
      downloadedBytes: 0,
      totalBytes: 322728954,
    },
  });
  const library = page.getByRole("region", {
    name: "Model library",
    exact: true,
  });
  await library.getByRole("tab", { name: "Browse", exact: true }).click();
  await library
    .getByRole("combobox", { name: "Filter model library" })
    .selectOption("punctuation");
  const row = library.locator(`[data-model-id="${id}"]`);
  await expect(row).toContainText("CPU · 303 MB");
  await expect(row).toContainText("323 MB to download");
  await expect(row).toContainText(help);
  await row.locator("summary").click();
  await expect(row).toContainText("CC BY-SA 4.0");
  await expect(row.getByRole("link", { name: "Model page ↗" })).toHaveAttribute(
    "href",
    modelPage,
  );
  await row.getByRole("link", { name: "Model page ↗" }).click();
  expect(
    await page.evaluate(() =>
      window.__testCalls.filter(
        (call) => call.command === "open_punctuation_model_page",
      ),
    ),
  ).toEqual([{ command: "open_punctuation_model_page", args: { id } }]);
  await expect(
    row.getByRole("button", {
      name: "Install SonoJot Punctuation v2",
      exact: true,
    }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Punctuation model", exact: true }),
  ).toHaveCount(0);
  expect(
    await page
      .locator(".page-content")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: testInfo.outputPath("punctuation-library.png"),
    fullPage: true,
  });
});

test("failed first-run punctuation offers Retry and cancellation keeps setup unfinished", async ({
  page,
}) => {
  await openSetup(page, {
    settings: { setupProfile: null },
    modelPatches: {
      "mock-whisper-small-cuda": { state: "installed" },
      "mock-cleanup-2b-cuda": { state: "installed" },
    },
  });
  await page.evaluate(
    (id) =>
      window.__testFailModelInstall(id, "Fixture punctuation download failed"),
    id,
  );
  await page.getByRole("button", { name: "Finish setup", exact: true }).click();
  await expect(download(page)).toContainText(
    "Fixture punctuation download failed",
  );
  expect((await settings(page)).setupProfile).toBeNull();
  await download(page)
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(download(page)).toContainText("Downloading");
  await download(page)
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  await expect(
    download(page).getByRole("button", { name: "Retry", exact: true }),
  ).toBeEnabled();
  expect((await settings(page)).setupProfile).toBeNull();
  await download(page)
    .getByRole("button", { name: "Retry", exact: true })
    .click();
  await expect(download(page)).toContainText("Downloading");
  await finishPunctuation(page);
  await expect
    .poll(async () => (await settings(page)).setupProfile)
    .toBe("balanced");
});

for (const state of ["checking", "unsupported"]) {
  test(`punctuation ${state} is not offered as an actionable recommendation`, async ({
    page,
  }) => {
    await openSetup(page, {
      punctuation: { state, installable: state !== "unsupported" },
    });
    await expect(recommendation(page)).toHaveCount(0);
    expect(await installs(page)).toHaveLength(0);
  });
}
