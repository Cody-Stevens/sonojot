import { expect, test } from "@playwright/test";
import {
  goToPage,
  mainNavigation,
  openApp,
  openFold,
  openMeeting,
  openMeetingMenu,
  openMoreOptions,
  openSaveCopy,
  openTranscribeAgain,
  showRecentDictations,
} from "./app-helpers.js";

const warning =
  "Not a local model. Audio or text is sent to OpenRouter and may not be private.";

// Since 0.24 OpenRouter models are kept in a short list ("Your OpenRouter
// models") and offered inside the Notes and Meeting transcription pickers.
// The old "OpenRouter model" / "OpenRouter speech model" selects are gone.
test("audio-only catalog models are offered for meeting transcription only", async ({
  page,
}) => {
  await openApp(
    page,
    {
      aiSettings: { connected: true, model: "mock/text", speechModel: "" },
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
    },
    "/?page=setup",
  );
  const online = page.getByRole("region", {
    name: "Online models (OpenRouter)",
  });
  await online
    .getByRole("searchbox", { name: "Search OpenRouter models" })
    .fill("Audio-only");
  const results = online.getByRole("list", {
    name: "OpenRouter catalog results",
  });
  await expect(
    results.getByRole("button", {
      name: "Add Audio-only model for Meeting transcription",
    }),
  ).toBeEnabled();
  await expect(
    results.getByRole("button", { name: /^Add Audio-only model for Text/ }),
  ).toHaveCount(0);
  // Every catalog row carries the privacy warning.
  await results
    .getByRole("button", { name: "Remote model privacy" })
    .first()
    .focus();
  await expect(
    page.getByRole("tooltip").filter({ hasText: warning }),
  ).toBeVisible();
  await results
    .getByRole("button", {
      name: "Add Audio-only model for Meeting transcription",
    })
    .click();
  await expect(
    online.getByText("Audio-only model added for meeting transcription."),
  ).toBeVisible();
  // Adding makes a model available; it selects nothing.
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/text",
    speechModel: "",
    quickModels: [
      { id: "mock/text", role: "text" },
      { id: "mock/audio-only", role: "speech" },
    ],
  });
  await page
    .getByRole("button", { name: "Meeting transcription model", exact: true })
    .click();
  const meeting = page.getByRole("dialog", {
    name: "Meeting transcription models",
  });
  await expect(
    meeting.getByRole("option", { name: /Audio-only model/ }),
  ).toBeVisible();
  await expect(
    meeting.getByRole("option", { name: /^Text model/ }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Notes model", exact: true }).click();
  const notes = page.getByRole("dialog", { name: "Notes models" });
  await expect(
    notes.getByRole("option", { name: /^Text model/ }),
  ).toBeVisible();
  await expect(
    notes.getByRole("option", { name: /Audio-only model/ }),
  ).toHaveCount(0);
});

test("choosing an online meeting speech model keeps the separate text choice", async ({
  page,
}) => {
  await openApp(
    page,
    {
      aiSettings: {
        connected: true,
        model: "mock/text",
        speechModel: "",
        quickModels: [
          { id: "mock/text", role: "text" },
          { id: "mock/audio", role: "speech" },
        ],
      },
    },
    "/?page=setup",
  );
  const trigger = page.getByRole("button", {
    name: "Meeting transcription model",
    exact: true,
  });
  await trigger.click();
  const picker = page.getByRole("dialog", {
    name: "Meeting transcription models",
  });
  const remote = picker.getByRole("option", { name: /Mock speech model/ });
  await remote.focus();
  await expect(remote).toHaveAccessibleDescription(warning);
  await expect(remote.locator("button, [tabindex]")).toHaveCount(0);
  await remote.click();
  await expect(trigger).toContainText("Mock speech model");
  await expect(trigger).toHaveAccessibleDescription(warning);
  await expect(page.locator("#setup-meeting")).toContainText(
    "Sent to OpenRouter and billed there · $1 input, $2 output per 1M tokens",
  );
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/text",
    speechModel: "mock/audio",
  });
  expect(
    await page.evaluate(() => window.__testSnapshot().settings.meetingAsrModel),
  ).toBe("");
  // Back to the local default clears only the online speech route.
  await trigger.click();
  await picker
    .getByRole("option", { name: "Same as Speech (on this computer)" })
    .click();
  await expect(trigger).toContainText("Same as Speech (on this computer)");
  expect(await page.evaluate(() => window.__testAiSettings())).toMatchObject({
    model: "mock/text",
    speechModel: "",
  });
});

test("remote Notes choices and the Notes summary expose privacy without nested controls", async ({
  page,
}) => {
  await openApp(
    page,
    {
      aiSettings: {
        connected: true,
        backend: "openrouter",
        model: "mock/text",
      },
    },
    "/?page=setup",
  );
  const notes = page.getByRole("button", { name: "Notes model", exact: true });
  await notes.click();
  const remote = page
    .getByRole("dialog", { name: "Notes models", exact: true })
    .getByRole("option", { name: /Mock text model/ });
  await remote.focus();
  await expect(remote).toHaveAccessibleDescription(warning);
  await expect(remote.locator("button, [tabindex]")).toHaveCount(0);
  await expect(
    page.getByRole("tooltip").filter({ hasText: warning }),
  ).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(notes).toContainText("Mock text model");
  await expect(notes).toHaveAccessibleDescription(warning);
  await expect(page.locator("#setup-notes")).toContainText(
    "Sent to OpenRouter and billed there · $1 input, $2 output per 1M tokens",
  );
  await goToPage(page, "Dictation");
  const models = await openMoreOptions(page);
  const summary = models
    .locator(".model-summary")
    .filter({ hasText: "OpenRouter" });
  await expect(summary).toHaveCount(1);
  await expect(summary).toContainText("Notes · mock/text · OpenRouter");
  await summary
    .getByRole("button", { name: "Remote model privacy", exact: true })
    .focus();
  await expect(
    page.getByRole("tooltip").filter({ hasText: warning }),
  ).toBeVisible();
});

test("desktop editing workflow preserves words, drafts and native command arguments", async ({
  page,
}) => {
  const pageErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await openApp(page);
  await goToPage(page, "Microphone");
  await page
    .getByRole("button", { name: "Test with a dictation", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Finish dictation", exact: true })
    .click();
  await goToPage(page, "Dictation");
  const history = await showRecentDictations(page);
  await history.getByRole("button", { name: "Copy latest" }).click();
  await history.getByRole("button", { name: "All recent dictations" }).click();
  await goToPage(page, "Shortcuts");
  await expect(page.locator("#hotkey")).toHaveValue("Ctrl+Shift+D");
  await page.locator("#hotkey").fill("Ctrl+Alt+D");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await goToPage(page, "Vocabulary");
  await page.getByRole("button", { name: "Add a word", exact: true }).click();
  await page.getByLabel("When it hears", { exact: true }).fill("grock");
  await page.getByLabel("Write it as", { exact: true }).fill("Grok");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await page.getByRole("button", { name: "Save words", exact: true }).click();
  await goToPage(page, "Meetings");
  await openMeeting(page, /Design review/);
  await expect(page.locator("#meeting-transcript")).toHaveValue(
    "Keep 4.6. Do not replace the backup. <img src=x onerror=alert(1)>",
  );
  await expect(page.locator("#meeting-panel-transcript img")).toHaveCount(0);
  await (
    await openMeetingMenu(page)
  )
    .getByRole("button", { name: "Rename", exact: true })
    .click();
  await page.locator("#meeting-title").fill("Edited review");
  // Leaving with an unsaved title asks first; Keep working stays.
  await mainNavigation(page)
    .getByRole("button", { name: "Appearance", exact: true })
    .click();
  await page.getByRole("button", { name: "Keep working", exact: true }).click();
  await expect(page.locator("#meeting-title")).toHaveValue("Edited review");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await page
    .getByRole("button", { name: "Process notes", exact: true })
    .click();
  await page.locator("#note-preset").selectOption("tasks");
  await page
    .getByRole("button", { name: "Generate notes", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "Notes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#meeting-notes")).toHaveValue(
    "- Keep 4.6.\n- Preserve the backup.",
  );
  const saveCopy = await openSaveCopy(page);
  await saveCopy.getByLabel("Format", { exact: true }).selectOption("pdf");
  await saveCopy.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
  const transcribe = await openTranscribeAgain(page);
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("mock-whisper-small-cpu");
  await transcribe.getByRole("button", { name: "Start", exact: true }).click();
  await page
    .getByRole("button", { name: "Cancel processing", exact: true })
    .click();
  await page.getByRole("button", { name: "All meetings", exact: true }).click();
  const meetingsView = page.getByRole("group", { name: "Meetings" });
  await meetingsView
    .getByRole("button", { name: "Start a meeting", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Import a recording", exact: true })
    .click();
  await expect(page.locator(".meeting-heading h1")).toHaveText(
    "Imported recording",
  );
  await (
    await openMeetingMenu(page)
  )
    .getByRole("button", { name: "Delete meeting…", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Delete meeting", exact: true })
    .click();
  await meetingsView
    .getByRole("button", { name: "Start a meeting", exact: true })
    .click();
  await page.locator("#new-meeting-title").fill("Recording fixture");
  await page
    .getByRole("button", { name: "Start recording", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Finish recording", exact: true })
    .click();
  await goToPage(page, "Appearance");
  await page.getByRole("button", { name: /Slate paper Charcoal/ }).click();
  await page
    .getByRole("switch", { name: "Paper and print", exact: true })
    .click();
  await expect(page.locator(".studio")).toHaveAttribute("data-theme", "slate");
  await expect(page.locator(".studio")).toHaveAttribute("data-material", "off");
  await goToPage(page, "About");
  await openFold(page, "Troubleshooting");
  await page
    .getByRole("button", { name: "Show diagnostics", exact: true })
    .click();
  await expect(page.getByLabel("Desktop diagnostics")).toHaveValue(
    /Isolated UI fixture/,
  );
  const calls = await page.evaluate(() => window.__testCalls);
  for (const [command, args] of [
    [
      "save_settings",
      { settings: expect.objectContaining({ hotkey: "Ctrl+Alt+D" }) },
    ],
    [
      "save_vocabulary",
      {
        entries: [expect.objectContaining({ heard: "grock", written: "Grok" })],
      },
    ],
    ["save_meeting", { title: "Edited review" }],
    ["export_meeting", { format: "pdf" }],
    ["transcribe_meeting", { model: "mock-whisper-small-cpu" }],
    ["generate_notes", { preset: "tasks" }],
    ["delete_meeting", { id: "imported" }],
    ["start_meeting", { title: "Recording fixture" }],
  ])
    expect(calls).toContainEqual(
      expect.objectContaining({ command, args: expect.objectContaining(args) }),
    );
  expect(pageErrors).toEqual([]);
});

test("incomplete current-session text requires the explicit review copy", async ({
  page,
}) => {
  await openApp(page);
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
  await openApp(page);
  const history = await showRecentDictations(page);
  await history
    .getByRole("button", { name: /Keep the first instruction/ })
    .click();
  await expect(history.getByRole("status")).toContainText(
    "Recognition was incomplete",
  );
  await history.getByRole("button", { name: "Compare versions" }).click();
  const recognized = history
    .getByRole("region", { name: "Compare versions" })
    .locator(":scope > div")
    .filter({ has: page.getByRole("heading", { name: "As recognized" }) });
  await recognized.getByRole("button", { name: "Copy", exact: true }).click();
  expect(await page.evaluate(() => window.__testCalls)).toContainEqual({
    command: "copy_history",
    args: { id: "dictation-incomplete", original: true },
  });
  await expect(
    page.getByRole("region", { name: "Current dictation" }),
  ).toHaveCount(0);
});

test("history correction validates the original phrase and copies the selected original", async ({
  page,
}) => {
  await openApp(page);
  const history = await showRecentDictations(page);
  await history.getByRole("button", { name: /First, keep Grok/ }).click();
  await openFold(history, "More");
  await history
    .getByRole("button", { name: "Add a word to Vocabulary…", exact: true })
    .click();
  const addWord = history.getByRole("region", {
    name: "Add a word to Vocabulary",
  });
  await addWord
    .getByLabel("When it hears", { exact: true })
    .fill("not present");
  await addWord.getByLabel("Write it as", { exact: true }).fill("Grok");
  await expect(
    addWord.getByText(
      "Use a phrase that appears in the As recognized version.",
    ),
  ).toBeVisible();
  await expect(
    addWord.getByRole("button", { name: "Add to vocabulary", exact: true }),
  ).toBeDisabled();
  await addWord.getByLabel("When it hears", { exact: true }).fill("grock");
  await addWord
    .getByRole("button", { name: "Add to vocabulary", exact: true })
    .click();
  await history.getByRole("button", { name: "Compare versions" }).click();
  await history
    .getByRole("region", { name: "Compare versions" })
    .locator(":scope > div")
    .filter({ has: page.getByRole("heading", { name: "As recognized" }) })
    .getByRole("button", { name: "Copy", exact: true })
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
