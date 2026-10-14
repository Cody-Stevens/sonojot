// Round 6 daily-use UI checks. All native responses come from the isolated
// mock host in desktop-fixture.js: no audio is recorded and no files change.
import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";
import {
  openFold,
  openMeeting,
  openModelLibrary,
  openMoreOptions,
  openPerformance,
  openVocabularyView,
  showRecentDictations,
} from "./app-helpers.js";

async function chooseInstalled(page, role, name) {
  await page.locator(`#model-${role}`).click();
  const picker = page.getByRole("dialog");
  await picker.getByRole("searchbox").fill(name);
  await picker.getByRole("option").click();
}

test("setup keeps models, performance, library, and storage concise", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  const setup = page.getByRole("region", { name: "Speech and text models" });
  await expect(setup.locator('[data-budget="HOST RAM"]')).toContainText(
    "free now",
  );
  await expect(setup.locator('[data-budget="GPU VRAM"]')).toContainText(
    "free now",
  );
  await expect(setup).not.toContainText("room for other apps");
  await expect(setup).not.toContainText("These are not a guarantee");
  // Independent role selectors, one per task.
  await expect(page.locator("#model-asr")).toBeEnabled();
  await expect(page.locator("#model-cleanup")).toBeEnabled();
  await expect(page.locator("#model-notes")).toBeEnabled();
  await expect(setup).toContainText("Speech");
  await expect(setup).toContainText("Cleanup");
  await expect(setup).toContainText("Notes");
  const performance = await openPerformance(page);
  await expect(
    performance.getByRole("combobox", { name: "Memory use" }),
  ).toBeVisible();
  await expect(
    performance.getByRole("combobox", { name: "Release unused models" }),
  ).toBeVisible();
  // Text weights support both devices; CUDA speech remains a separate package.
  const library = await openModelLibrary(page);
  await expect(
    library.locator('[data-model-id="mock-cleanup-2b-cpu"]'),
  ).toContainText("CPU");
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  await expect(
    library.locator('[data-model-id="mock-whisper-small-cuda"]'),
  ).toContainText("NVIDIA CUDA");
  await expect(library).not.toContainText(
    /AMD.*(?:unsupported|not supported)/i,
  );
  // Storage rows sit at the end of the Performance and storage fold.
  const folder = performance.getByRole("group", { name: "Models folder" });
  await expect(folder.getByRole("button", { name: "Change…" })).toBeVisible();
  await expect(folder.getByRole("button", { name: "Open" })).toBeVisible();
  await expect(
    performance.getByRole("group", { name: "Audio and video import" }),
  ).toContainText("Needed to import recordings.");
  // Only Setup owns the text-acceleration control.
  await expect(
    page.getByRole("combobox", { name: "Text model processing" }),
  ).toHaveCount(1);
});

test("setup polls only while model state is changing", async ({ page }) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  const library = await openModelLibrary(page);
  const commandCount = (command) =>
    page.evaluate(
      (name) =>
        window.__testCalls.filter((call) => call.command === name).length,
      command,
    );
  await expect.poll(() => commandCount("get_model_library")).toBeGreaterThan(0);
  await expect
    .poll(() => commandCount("get_model_acceleration"))
    .toBeGreaterThan(0);
  const settledLibrary = await commandCount("get_model_library");
  const settledAcceleration = await commandCount("get_model_acceleration");
  await page.waitForTimeout(2200);
  expect(await commandCount("get_model_library")).toBe(settledLibrary);
  expect(await commandCount("get_model_acceleration")).toBe(
    settledAcceleration,
  );

  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect
    .poll(() => commandCount("get_model_library"))
    .toBeGreaterThan(settledLibrary);
  await expect
    .poll(() => commandCount("get_model_acceleration"))
    .toBeGreaterThan(settledAcceleration);

  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  const target = library.locator('[data-model-id="mock-cleanup-2b-cuda"]');
  await target.getByRole("button", { name: /^Install / }).click();
  const beforeTransientPoll = await commandCount("get_model_library");
  await expect
    .poll(() => commandCount("get_model_library"), { timeout: 3000 })
    .toBeGreaterThan(beforeTransientPoll);
  await page
    .getByRole("region", { name: "Model download" })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  const afterCancel = await commandCount("get_model_library");
  await page.waitForTimeout(2200);
  expect(await commandCount("get_model_library")).toBe(afterCancel);
});

test("setup install shows progress with cancel, and retry recovers a failed entry", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  const library = await openModelLibrary(page);
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  const target = library.locator('[data-model-id="mock-cleanup-2b-cuda"]');
  await target.getByRole("button", { name: /^Install / }).click();
  const download = page.getByRole("region", { name: "Model download" });
  await expect(download).toContainText(/downloading/i);
  await expect(
    download.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeVisible();
  await download.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(download).toContainText("Cancelled");
  await expect(target.getByRole("button", { name: /^Install / })).toBeVisible();
  const failed = library.locator('[data-model-id="mock-cleanup-old-1b"]');
  await failed.getByRole("button", { name: /^Retry / }).click();
  await expect(download).toContainText(/downloading/i);
  await expect(download).toContainText("Mock Cleanup 1B");
  await page.evaluate(() => {
    window.__testPatchModel("mock-cleanup-old-1b", {
      state: "failed",
      error: "Mock retry failure",
    });
    window.dispatchEvent(new Event("focus"));
  });
  await expect(download).toContainText("Mock retry failure");
  await download.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(download).toContainText("Downloading");
});

test("setup removal protects selected roles and confirms unused model deletion", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  const library = await openModelLibrary(page);
  const target = library.locator('[data-model-id="mock-cleanup-2b-cpu"]');
  // A selected model stays removable; its confirmation names the switch.
  await expect(
    library.getByRole("button", { name: "Remove LFM 2.5", exact: true }),
  ).toBeEnabled();
  await target.getByRole("button", { name: /^Remove / }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Remove Mock Cleanup 2B (CPU)?");
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  await library.getByRole("tab", { name: "Get more", exact: true }).click();
  await expect(target.getByRole("button", { name: /^Install / })).toBeVisible();
});

test("dictation keeps one-line summaries with one deep link per role and no duplicate controls", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  // "Models in use" sits in the Dictation page's More options fold.
  const models = await openMoreOptions(page);
  await expect(models).toContainText("Speech · Moonshine Medium · CPU");
  await expect(page.getByLabel("Cleanup model", { exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Notes model", { exact: true })).toHaveCount(0);
  await expect(
    page.getByRole("region", { name: "Latest dictation" }),
  ).toHaveCount(0);
  for (const role of ["Speech", "Cleanup", "Notes"])
    await expect(
      models.getByRole("button", { name: `Change ${role} model`, exact: true }),
    ).toHaveCount(1);
  await models
    .getByRole("button", { name: "Change Speech model", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Speech and text models" }),
  ).toBeVisible();
  await expect(page.locator("#setup-asr")).toBeFocused();
});

test("incomplete current dictation stays reviewable without a latest panel", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      sessionId: null,
      resultSessionId: 41,
      originalText:
        "Keep the first instruction. <thö Do not remove the backup.",
      outputText: "Keep the first instruction. <thö Do not remove the backup.",
      recognitionCompleteness: "incomplete",
      recognitionWarnings: ["segment two remained ambiguous after one retry"],
    }),
  );
  const review = page.getByRole("region", { name: "Current dictation" });
  await expect(review.getByRole("status")).toContainText(
    "Recognition is incomplete",
  );
  await review
    .getByRole("button", { name: "Review available text", exact: true })
    .click();
  await review
    .getByRole("button", { name: "Copy after review" })
    .first()
    .click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "copy_text",
    args: { original: true, resultSessionId: 41 },
  });
});

test("recent dictations marks the newest row, copies it and confirms every delete", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  const history = await showRecentDictations(page);
  await expect(history).toContainText("Newest first · Last 100 dictations");
  await expect(history.getByText("Latest", { exact: true })).toBeVisible();
  // The newest dictation is incomplete, so Copy latest opens it for review.
  await history.getByRole("button", { name: "Copy latest" }).click();
  await expect(history.getByRole("status")).toContainText(
    "Recognition was incomplete",
  );
  expect(
    (await page.evaluate(() => window.__testCalls)).filter(
      (call) => call.command === "copy_history",
    ),
  ).toEqual([]);
  await history.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "copy_history",
    args: { id: "dictation-incomplete", original: false },
  });
  await history.getByRole("button", { name: "All recent dictations" }).click();
  // Opening a row keeps working while a newer result arrives.
  await history.getByRole("button", { name: /First, keep Grok/ }).click();
  await expect(
    history.getByRole("button", { name: "All recent dictations" }),
  ).toBeVisible();
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      sessionId: null,
      resultSessionId: 9,
      originalText: "A newer arrival.",
      outputText: "A newer arrival.",
      recognitionCompleteness: "complete",
      recognitionWarnings: [],
    }),
  );
  await expect(
    history.getByRole("button", { name: "All recent dictations" }),
  ).toBeVisible();
  // Every dictation has its own confirmed delete under More.
  await openFold(history, "More");
  await history
    .getByRole("button", { name: "Delete this dictation", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Delete this dictation?");
  await expect(dialog).toContainText(
    "This removes the saved original and finished text from SonoJot.",
  );
  await dialog.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByText("Dictation deleted.")).toBeVisible();
});

test("do-not-save toggle never deletes by itself and delete-all names its scope", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  const history = await showRecentDictations(page);
  await openFold(history, "History options");
  await history
    .getByRole("switch", { name: "Save dictation history", exact: true })
    .click();
  expect(
    await page.evaluate(() => window.__testSnapshot().settings),
  ).toMatchObject({ doNotSaveDictations: true });
  // Turning the toggle on keeps the existing rows.
  await expect(history.getByText("Latest", { exact: true })).toBeVisible();
  await history
    .getByRole("button", { name: "Delete all dictations…", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Delete all 2 dictations?");
  await dialog.getByRole("button", { name: "Delete all", exact: true }).click();
  await expect(page.getByText("2 dictations deleted.")).toBeVisible();
  await expect(history.getByText("Latest", { exact: true })).toHaveCount(0);
});

test("vocabulary learning stays opt-in, inspectable and resettable", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=vocabulary&theme=fog");
  const learning = await openVocabularyView(page, "Learning from your fixes");
  await learning
    .getByRole("switch", {
      name: "Learn from my corrections on this device",
      exact: true,
    })
    .click();
  expect(
    await page.evaluate(() => window.__testSnapshot().settings),
  ).toMatchObject({ learnFromCorrections: true });
  // The view loads learning on mount; the button covers the slow path only.
  const entry = learning.locator('[data-learning-id="mock-learned-one"]');
  await entry.getByRole("button", { name: /grock/ }).click();
  await openFold(entry, "Source references");
  await expect(entry).toContainText("dictation-correction");
  await openFold(learning, "More learning options");
  await learning
    .getByRole("button", { name: "Reset learning…", exact: true })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Reset local learning?");
  await dialog
    .getByRole("button", { name: "Reset learning", exact: true })
    .click();
  await expect(learning).toContainText("No learned entries.");
});

test("meeting notes use a one-line model summary without duplicate controls", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=meetings&theme=fog");
  await openMeeting(page, /Design review/);
  await page.getByRole("button", { name: "Process notes", exact: true }).click();
  await expect(page.getByLabel("Notes model", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Notes · Qwen 3 4B · CPU")).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Manage .* in Setup/ }),
  ).toHaveCount(0);
});

for (const theme of ["fog", "slate"]) {
  test(`round 6 screenshots in ${theme} are mock-based captures`, async ({
    page,
  }, testInfo) => {
    await installDesktopFixture(page);
    await page.goto(`/?page=setup&theme=${theme}`);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    const library = await openModelLibrary(page);
    const setupPath = `../artifacts/round6/ui-shots-mock/round6-setup-${theme}.png`;
    await page.screenshot({ path: setupPath });
    await testInfo.attach(`round6-setup-${theme} (mock host)`, {
      path: setupPath,
      contentType: "image/png",
    });
    // Compact installed library, with estimates behind each row's Details.
    const detailsTarget = library.locator(
      '[data-model-id="mock-cleanup-2b-cpu"]',
    );
    await detailsTarget.scrollIntoViewIfNeeded();
    await expect(await openFold(detailsTarget, "Details")).toContainText(
      /Estimated RAM/,
    );
    await expect(detailsTarget).not.toContainText(
      /AMD.*(?:unsupported|not supported)/i,
    );
    const libraryPath = `../artifacts/round6/ui-shots-mock/round6-library-details-${theme}.png`;
    await page.screenshot({ path: libraryPath });
    await testInfo.attach(`round6-library-details-${theme} (mock host)`, {
      path: libraryPath,
      contentType: "image/png",
    });
    await page.goto(`/?page=dictation&theme=${theme}`);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    const history = await showRecentDictations(page);
    await expect(history.getByText("Latest", { exact: true })).toBeVisible();
    const dictationPath = `../artifacts/round6/ui-shots-mock/round6-dictation-${theme}.png`;
    await page.screenshot({ path: dictationPath });
    await testInfo.attach(`round6-dictation-${theme} (mock host)`, {
      path: dictationPath,
      contentType: "image/png",
    });
    // Recent dictations delete dialog (mock host, counted scope).
    await history.getByRole("button", { name: /First, keep Grok/ }).click();
    await openFold(history, "More");
    await history
      .getByRole("button", { name: "Delete this dictation", exact: true })
      .click();
    const deleteDialog = page.getByRole("dialog");
    await expect(deleteDialog).toContainText("Delete this dictation?");
    const deletePath = `../artifacts/round6/ui-shots-mock/round6-delete-dialog-${theme}.png`;
    await page.screenshot({ path: deletePath });
    await testInfo.attach(`round6-delete-dialog-${theme} (mock host)`, {
      path: deletePath,
      contentType: "image/png",
    });
    await deleteDialog
      .getByRole("button", { name: "Cancel", exact: true })
      .click();
    // No-history delete-all scope dialog (mock host).
    await history.getByRole("button", { name: "All recent dictations" }).click();
    await openFold(history, "History options");
    await history
      .getByRole("button", { name: "Delete all dictations…", exact: true })
      .click();
    const deleteAll = page.getByRole("dialog");
    await expect(deleteAll).toContainText(/Delete all \d+ dictations\?/);
    const noHistoryPath = `../artifacts/round6/ui-shots-mock/round6-no-history-${theme}.png`;
    await page.screenshot({ path: noHistoryPath });
    await testInfo.attach(`round6-no-history-${theme} (mock host)`, {
      path: noHistoryPath,
      contentType: "image/png",
    });
    await deleteAll
      .getByRole("button", { name: "Cancel", exact: true })
      .click();
    // Vocabulary learning (mock host).
    await page.goto(`/?page=vocabulary&theme=${theme}`);
    await expect(page.locator(".studio")).toHaveAttribute("data-theme", theme);
    await openVocabularyView(page, "Learning from your fixes");
    const learningPath = `../artifacts/round6/ui-shots-mock/round6-learning-${theme}.png`;
    await page.screenshot({ path: learningPath });
    await testInfo.attach(`round6-learning-${theme} (mock host)`, {
      path: learningPath,
      contentType: "image/png",
    });
  });
}

test("dictation uses daily policy by default with no trial control", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  // Obsolete trial framing is gone; saved cleanupTrial stays in settings but has no toggle.
  await expect(page.getByText("Structured cleanup trial")).toHaveCount(0);
  await expect(
    page.getByLabel("Structured cleanup trial", { exact: true }),
  ).toHaveCount(0);
  const cleanup = page.getByRole("region", { name: "Text cleanup" });
  // Default Clean mode carries the compact daily wording.
  await expect(cleanup).toContainText("Remove clutter, keep intent");
  // Format and As heard copy are truthful once selected (contextual, no word-change claim).
  await cleanup.getByRole("button", { name: /Format only/ }).click();
  await expect(cleanup).toContainText("Keep wording, improve layout");
  await cleanup.getByRole("button", { name: /As heard/ }).click();
  await expect(cleanup).toContainText("Keep recognition unchanged");
});

test("cleanup offers rules-only without forcing a model load", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  const select = page.locator("#model-cleanup");
  await chooseInstalled(page, "cleanup", "with your vocabulary");
  expect(
    await page.evaluate(() => window.__testSnapshot().settings),
  ).toMatchObject({
    cleanupModel: "local-rules",
  });
  // Rules-only stays a short selection; three role selectors remain independent.
  await expect(page.locator("#model-asr")).toBeEnabled();
  await expect(page.locator("#model-cleanup")).toBeEnabled();
  await expect(page.locator("#model-notes")).toBeEnabled();
  await expect(select).toHaveText(
    "SonoJot Punctuation v2 with your vocabulary",
  );
  await expect(page.locator("#setup-cleanup")).toContainText("CPU");
  await expect(page.locator("#setup-cleanup")).not.toContainText(
    "Formatting and vocabulary",
  );
  await expect(
    page.getByText("Rules-only cleanup needs no model load."),
  ).toHaveCount(0);
});

test("asr compatibility is artifact-aware and never invents gpu running", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=setup&theme=fog");
  await page.evaluate(() => {
    for (const id of [
      "qwen3-asr-1.7b",
      "qwen3-asr-1.7b-fp16-cuda",
      "whisper-tiny-fp32-cuda",
    ])
      window.__testPatchModel(id, { state: "installed" });
    window.dispatchEvent(new Event("focus"));
  });
  // CPU Qwen int8 stays CPU only.
  await chooseInstalled(page, "asr", "CPU int8");
  await expect(page.locator("#setup-asr")).toContainText("CPU only");
  // GPU speech packages show NVIDIA package with unverified runtime, never GPU active.
  await chooseInstalled(page, "asr", "FP16 CUDA");
  const gpuRow = page.locator("#setup-asr");
  await expect(gpuRow).toContainText("NVIDIA CUDA");
  await expect(gpuRow).toContainText("not loaded");
  await expect(gpuRow).not.toContainText("AMD is unsupported");
  await expect(gpuRow).not.toContainText("GPU accelerated");
  await chooseInstalled(page, "asr", "FP32 CUDA");
  await expect(page.locator("#setup-asr")).toContainText("not loaded");
});

test("vocabulary retention distinguishes learning from review routing", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=vocabulary&theme=fog");
  const learning = await openVocabularyView(page, "Learning from your fixes");
  await expect(learning).toContainText("Learned entries stay on this device.");
  await expect(await openFold(learning, "More learning options")).toContainText(
    "Reviewing recent dictations uses the AI selected in Models and may send review text to OpenRouter.",
  );
  await page.getByRole("button", { name: "Your words" }).click();
  const suggestions = await openVocabularyView(
    page,
    "Suggestions from recent dictations",
  );
  // Local backend: review stays local with explicit provider choice.
  await expect(suggestions).toContainText(
    "Review text stays on this computer.",
  );
});

test("full-text correction uses the canonical command with no silent fallback", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/?page=dictation&theme=fog");
  const history = await showRecentDictations(page);
  await history.getByRole("button", { name: /First, keep Grok/ }).click();
  await history
    .getByRole("button", { name: "Fix a mistake", exact: true })
    .click();
  await page
    .getByLabel("Corrected text", { exact: true })
    .fill("First, keep Grok 4.6 revised.");
  await history
    .getByRole("button", { name: "Save correction", exact: true })
    .click();
  const calls = await page.evaluate(() => window.__testCalls);
  expect(calls).toContainEqual({
    command: "save_dictation_correction",
    args: {
      id: "dictation-one",
      text: "First, keep Grok 4.6 revised.",
      scope: null,
    },
  });
  // No silent fallback to the legacy heard/written command on this path.
  expect(calls.filter((c) => c.command === "learn_correction")).toHaveLength(0);
});
