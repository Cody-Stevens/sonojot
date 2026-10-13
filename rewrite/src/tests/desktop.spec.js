import { expect, test } from "@playwright/test";
import { installDesktopFixture } from "./desktop-fixture.js";

test("audio-only catalog models stay out of text selection", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    aiSettings: {
      connected: true,
      model: "mock/text",
      speechModel: "mock/text",
    },
    aiCatalog: [
      {
        id: "mock/text",
        name: "Text model",
        contextLength: 32000,
        promptPrice: null,
        completionPrice: null,
        inputModalities: ["text"],
      },
      {
        id: "mock/audio-only",
        name: "Audio-only model",
        contextLength: 32000,
        promptPrice: null,
        completionPrice: null,
        inputModalities: ["audio"],
      },
    ],
  });
  await page.goto("/?page=setup");
  const speech = page.getByLabel("OpenRouter speech model", { exact: true });
  await expect(speech).toBeEnabled();
  await expect(speech.locator('option[value="mock/audio-only"]')).toHaveCount(
    1,
  );
  await expect(speech.locator('option[value="mock/text"]')).toBeDisabled();
  const text = page.getByLabel("OpenRouter model", { exact: true });
  await expect(text.locator('option[value="mock/audio-only"]')).toHaveCount(0);
  await page
    .getByLabel("Find another model", { exact: true })
    .fill("Audio-only");
  await expect(
    page.getByText("No catalog model matches.", { exact: true }),
  ).toBeVisible();
  await speech.selectOption("mock/audio-only");
  await page
    .getByRole("button", { name: "Save AI settings", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Save AI settings", exact: true }),
  ).toBeDisabled();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/text",
    speechModel: "mock/audio-only",
  });
});

test("OpenRouter speech settings filter audio models and preserve the separate text choice", async ({
  page,
}, testInfo) => {
  await installDesktopFixture(page, {
    aiSettings: { connected: true, model: "mock/text" },
  });
  await page.goto("/?page=setup");
  const speech = page.getByLabel("OpenRouter speech model", { exact: true });
  await expect(speech).toBeEnabled();
  await expect(speech.locator("option")).toHaveText([
    "No remote speech model",
    "Mock speech model · $$",
  ]);
  await speech.selectOption("mock/audio");
  await page
    .getByRole("button", { name: "Save AI settings", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Save AI settings", exact: true }),
  ).toBeDisabled();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    backend: "local",
    model: "mock/text",
    speechModel: "mock/audio",
  });
  expect(
    await page.evaluate(
      () =>
        window.__testCalls.find((call) => call.command === "save_ai_settings")
          ?.args,
    ),
  ).toEqual({
    settings: {
      backend: "local",
      model: "mock/text",
      speechModel: "mock/audio",
    },
  });
  await speech.focus();
  await page.keyboard.press("Tab");
  const warning =
    "Not a local model. Audio or text is sent to OpenRouter and may not be private.";
  await expect(page.getByRole("tooltip")).toHaveText(warning);
  await page.screenshot({
    path: testInfo.outputPath("openrouter-speech-settings.png"),
  });
  await page.keyboard.press("Escape");
  await page
    .getByLabel("Find another model", { exact: true })
    .fill("Legacy catalog");
  const result = page.getByRole("option", { name: /Legacy catalog model/ });
  await expect(result).toBeVisible();
  await result.focus();
  await expect(result).toHaveAccessibleDescription(warning);
  await expect(result.locator("button, [tabindex]")).toHaveCount(0);
  await expect(page.getByRole("tooltip")).toHaveText(warning);
  await page.keyboard.press("Enter");
  await expect(
    page.getByLabel("OpenRouter model", { exact: true }),
  ).toHaveValue("mock/unknown");
  await page
    .getByRole("button", { name: "Save AI settings", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Save AI settings", exact: true }),
  ).toBeDisabled();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/unknown",
    speechModel: "mock/audio",
  });
  await speech.selectOption("");
  await page
    .getByRole("button", { name: "Save AI settings", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Save AI settings", exact: true }),
  ).toBeDisabled();
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/unknown",
    speechModel: "",
  });
});

test("remote Notes choices and Notes/Cleanup summaries expose privacy without nested controls", async ({
  page,
}) => {
  await installDesktopFixture(page, {
    aiSettings: { connected: true, backend: "openrouter", model: "mock/text" },
  });
  await page.goto("/?page=setup");
  const notes = page.getByRole("button", { name: "Notes model", exact: true });
  await notes.click();
  const remote = page
    .getByRole("dialog", { name: "Notes models", exact: true })
    .getByRole("option", { name: /OpenRouter/ });
  await remote.focus();
  const warning =
    "Not a local model. Audio or text is sent to OpenRouter and may not be private.";
  await expect(remote).toHaveAccessibleDescription(warning);
  await expect(remote.locator("button, [tabindex]")).toHaveCount(0);
  await expect(page.getByRole("tooltip")).toHaveText(warning);
  await page.keyboard.press("Enter");
  await expect(notes).toContainText("OpenRouter · mock/text");
  await expect(notes).toHaveAccessibleDescription(warning);
  await expect(page.locator("#setup-notes .secondary")).toHaveText("Remote");
  await page.evaluate(() => {
    const snapshot = window.__testSnapshot();
    window.__testPushSnapshot({
      settings: { ...snapshot.settings, cleanupModel: "openrouter" },
    });
  });
  await page.getByRole("button", { name: "Dictation", exact: true }).click();
  const summaries = page
    .locator(".model-summary")
    .filter({ hasText: "OpenRouter" });
  await expect(summaries).toHaveCount(2);
  for (const summary of await summaries.all()) {
    await summary
      .getByRole("button", { name: "Remote model privacy", exact: true })
      .focus();
    await expect(page.getByRole("tooltip")).toHaveText(warning);
  }
});

test("desktop editing workflow preserves words, drafts and native command arguments", async ({
  page,
}) => {
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await installDesktopFixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Microphone", exact: true }).click();
  await page
    .getByRole("button", { name: "Start dictation", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Finish dictation", exact: true })
    .click();
  await page.getByRole("button", { name: "Dictation", exact: true }).click();
  await page
    .getByRole("region", { name: "Recent Dictations" })
    .getByRole("button", { name: /Copy newest dictation/ })
    .click();
  await page.getByRole("button", { name: "Shortcuts", exact: true }).click();
  await expect(page.locator("#hotkey")).toHaveValue("Ctrl+Shift+D");
  await page.locator("#hotkey").fill("Ctrl+Alt+D");
  await page
    .getByRole("button", { name: "Save shortcut", exact: true })
    .click();
  await page.getByRole("button", { name: "Vocabulary", exact: true }).click();
  await page.getByLabel("When it hears", { exact: true }).fill("grock");
  await page.getByLabel("Write it as", { exact: true }).fill("Grok");
  await page.getByRole("button", { name: "Add entry", exact: true }).click();
  await page
    .getByRole("button", { name: "Save vocabulary", exact: true })
    .click();
  await page.getByRole("button", { name: "Meetings", exact: true }).click();
  await page.getByRole("button", { name: /Design review/ }).click();
  await expect(page.locator("#meeting-transcript")).toHaveValue(
    "Keep 4.6. Do not replace the backup. <img src=x onerror=alert(1)>",
  );
  await expect(page.locator("#meeting-panel-transcript img")).toHaveCount(0);
  await page.locator("#meeting-title").fill("Edited review");
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(page.locator("#meeting-title")).toHaveValue("Edited review");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page.getByRole("tab", { name: "Options", exact: true }).click();
  await page.locator("#note-preset").selectOption("tasks");
  await page
    .getByRole("button", { name: "Generate notes", exact: true })
    .click();
  await expect(page.locator("#meeting-notes")).toHaveValue(
    "- Keep 4.6.\n- Preserve the backup.",
  );
  await page.getByRole("tab", { name: "Save copy", exact: true }).click();
  await page.getByLabel("Document format", { exact: true }).selectOption("pdf");
  await page
    .getByRole("button", { name: "Export meeting", exact: true })
    .click();
  await page.getByRole("tab", { name: "Full transcript", exact: true }).click();
  await page
    .getByRole("button", { name: "Transcribe again", exact: true })
    .click();
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("mock-whisper-small-cpu");
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await page
    .getByRole("button", { name: "Cancel processing", exact: true })
    .click();
  await page.getByRole("button", { name: "All meetings", exact: true }).click();
  await page
    .getByRole("button", { name: "Start a meeting", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Import recording", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Delete meeting", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete meeting", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Start a meeting", exact: true })
    .click();
  await page.locator("#new-meeting-title").fill("Recording fixture");
  await page
    .getByRole("button", { name: "Start recording", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Finish recording", exact: true })
    .click();
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
  await page.getByRole("button", { name: /Slate paper Charcoal/ }).click();
  await page
    .getByRole("switch", { name: "Paper and print", exact: true })
    .click();
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  await expect(page.locator(".studio")).toHaveAttribute("data-material", "off");
  await page.getByRole("button", { name: "About", exact: true }).click();
  await page
    .getByRole("button", { name: "Show diagnostics", exact: true })
    .click();
  await expect(page.getByLabel("Desktop diagnostics")).toHaveValue(
    /Isolated UI fixture/,
  );
  const calls = await page.evaluate(() => window.__testCalls);
  expect(calls).toContainEqual(
    expect.objectContaining({
      command: "export_meeting",
      args: expect.objectContaining({ format: "pdf" }),
    }),
  );
  expect(calls).toContainEqual(
    expect.objectContaining({
      command: "transcribe_meeting",
      args: expect.objectContaining({ model: "mock-whisper-small-cpu" }),
    }),
  );
  expect(calls).toContainEqual(
    expect.objectContaining({
      command: "generate_notes",
      args: expect.objectContaining({ preset: "tasks" }),
    }),
  );
  expect(pageErrors).toEqual([]);
});

test("incomplete current-session text requires the explicit review copy", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "idle",
      sessionId: null,
      resultSessionId: 41,
      originalText:
        "Keep the first instruction. <thö Do not remove the backup. Final tail: café, 保留.",
      outputText:
        "Keep the first instruction. <thö Do not remove the backup. Final tail: café, 保留.",
      recognitionCompleteness: "incomplete",
      recognitionWarnings: ["segment two remained ambiguous after one retry"],
      deliveryNotice:
        "Recognition incomplete. Review the text before copying it.",
    }),
  );
  const review = page.getByRole("region", { name: "Current dictation" });
  await expect(review.getByRole("status")).toContainText(
    "Recognition is incomplete",
  );
  await review
    .getByRole("button", { name: "Review available text", exact: true })
    .click();
  const reviewCopy = review.getByRole("button", {
    name: "Copy after review",
  });
  await expect(reviewCopy.first()).toBeEnabled();
  await reviewCopy.first().click();
  await expect(
    page.getByText("Reviewed text copied.", { exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "copy_text",
    args: { original: true, resultSessionId: 41 },
  });
  await page.evaluate(() =>
    window.__testPushSnapshot({
      state: "listening",
      sessionId: 42,
      resultSessionId: null,
    }),
  );
  await expect(reviewCopy).toHaveCount(0);
});

test("incomplete history stays reviewable without becoming current-session text", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/");
  const history = page.getByRole("region", { name: "Recent Dictations" });
  await history
    .getByRole("button", { name: /Keep the first instruction/ })
    .click();
  await expect(history.getByRole("status")).toContainText(
    "Recognition was incomplete",
  );
  await history
    .getByRole("button", { name: "Copy after review", exact: true })
    .first()
    .click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "copy_history",
    args: { id: "dictation-incomplete", original: true },
  });
});

test("history correction validates the original phrase and copies the selected original", async ({
  page,
}) => {
  await installDesktopFixture(page);
  await page.goto("/");
  await page
    .getByRole("region", { name: "Recent Dictations" })
    .getByRole("button", { name: /First, keep Grok/ })
    .click();
  await page
    .getByLabel("Heard in this dictation", { exact: true })
    .fill("not present");
  await page
    .getByRole("region", { name: "Recent Dictations" })
    .getByLabel("Write it as", { exact: true })
    .fill("Grok");
  await expect(
    page.getByRole("button", { name: "Add to vocabulary", exact: true }),
  ).toBeDisabled();
  await page
    .getByLabel("Heard in this dictation", { exact: true })
    .fill("grock");
  await page
    .getByRole("button", { name: "Add to vocabulary", exact: true })
    .click();
  await page
    .getByRole("region", { name: "Recent Dictations" })
    .getByRole("button", { name: "Copy", exact: true })
    .first()
    .click();
  const calls = await page.evaluate(() => window.__testCalls);
  expect(calls).toContainEqual({
    command: "learn_correction",
    args: { id: "dictation-one", heard: "grock", written: "Grok" },
  });
  expect(calls).toContainEqual({
    command: "copy_history",
    args: { id: "dictation-one", original: true },
  });
});
