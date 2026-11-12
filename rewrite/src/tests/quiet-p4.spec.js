import { expect, test } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { installDesktopFixture } from "./desktop-fixture.js";

const folder = new URL("../../artifacts/redesign-p4/", import.meta.url);
const transcript =
  "[0:01] Speaker 1: The deadline is Friday.\n\n[0:02] You: I will send the draft.\n\n[0:04] Speaker 2: Keep the backup.";
const notes =
  "# Summary\n- The deadline is Friday.\n- Keep the backup.\n\n# Tasks\n- Send the draft before the review.";
const calls = (page, command) =>
  page.evaluate(
    (command) => window.__testCalls.filter((call) => call.command === command),
    command,
  );
const options = (page) => page.getByRole("button", { name: "Meeting options" });
async function open(page, extra = {}, theme = "fog") {
  await installDesktopFixture(page, {
    ...extra,
    settings: {
      meetingSource: "both",
      meetingConsentAcknowledged: true,
      ...extra.settings,
    },
    meeting: {
      createdAt: new Date().toISOString(),
      source: "both",
      speakerCount: 2,
      speakerLabels: true,
      transcript,
      notes,
      rawTranscript: "um the deadline is Friday keep the backup",
      autoNamed: true,
      ...extra.meeting,
    },
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`/?page=meetings&theme=${theme}`);
  await expect(
    page.getByRole("button", { name: /Design review/ }),
  ).toBeVisible();
}
async function meeting(page) {
  await page.getByRole("button", { name: /Design review/ }).click();
}
async function menu(page, choice) {
  await options(page).click();
  const panel = page.getByRole("dialog", { name: "Meeting", exact: true });
  if (choice)
    await panel.getByRole("button", { name: choice, exact: true }).click();
  return page.getByRole("dialog");
}
async function capture(page, file) {
  await mkdir(folder, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: fileURLToPath(new URL(file, folder)) });
}

test("meeting slips search titles and expose Markdown export on hover or focus", async ({
  page,
}) => {
  await open(page);
  const first = page.locator(".meeting-slip").first();
  await page.mouse.move(0, 0);
  await expect(first.locator(".slip-actions")).toHaveCSS("opacity", "0");
  await first.getByRole("button", { name: /Design review/ }).focus();
  await expect(first.locator(".slip-actions")).toHaveCSS("opacity", "1");
  await capture(page, "01-meetings-fog.png");
  await first.getByRole("button", { name: "Export .md" }).click();
  expect((await calls(page, "export_meeting"))[0].args).toMatchObject({
    id: "meeting-one",
    format: "markdown",
    transcriptVersion: "recognized",
  });
  await page.getByLabel("Search meetings").fill("Client");
  await expect(page.locator(".meeting-slip")).toHaveCount(1);
  await expect(page.locator(".meeting-slip")).toContainText("MISSING 14 S");
  await page.getByLabel("Search meetings").fill("not a title");
  await expect(page.getByText("No matching meetings.")).toBeVisible();
});

test("first-record consent, live traces, loss, Stop and transcription cancellation use the existing owner", async ({
  page,
}) => {
  await page.clock.install();
  await open(page, { settings: { meetingConsentAcknowledged: false } });
  await page
    .getByRole("button", { name: "Record meeting", exact: false })
    .click();
  const consent = page.getByRole("dialog", { name: "Before you record" });
  await consent.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(await calls(page, "start_meeting")).toHaveLength(0);
  await page
    .getByRole("button", { name: "Record meeting", exact: false })
    .click();
  await consent.getByRole("button", { name: "Got it", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Recording this meeting" }),
  ).toBeVisible();
  await expect(page.locator('[data-meeting-id="new-meeting"]')).toHaveCount(0);
  for (let i = 0; i < 24; i += 1) {
    await page.evaluate(
      (i) =>
        window.__testPushSnapshot({
          meetingLevels: { microphone: (i % 5) / 6, system: ((i + 2) % 7) / 8 },
        }),
      i,
    );
    await page.clock.runFor(500);
  }
  await expect
    .poll(() =>
      page.locator('svg[aria-label="You input trace"] path').getAttribute("d"),
    )
    .not.toBe("");
  await page.evaluate(() =>
    window.__testPushSnapshot({
      meetingLevels: { microphone: 0.3, system: 0 },
      meetingSystemAudio: "lost",
      meetingSourceStatus: ["System audio lost — reconnecting…"],
    }),
  );
  await page.clock.runFor(1500);
  await capture(page, "02-recording-loss.png");
  await page.getByRole("button", { name: "Stop", exact: false }).click();
  expect(await calls(page, "stop_meeting")).toHaveLength(1);
  await expect(
    page.getByRole("region", { name: "Recording this meeting" }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Transcribe recording", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Transcribe", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Meeting progress" }),
  ).toContainText("Transcribing");
  await capture(page, "03-processing.png");
  await page
    .getByRole("button", { name: "Stop transcribing", exact: true })
    .click();
  expect((await calls(page, "cancel_meeting"))[0].args).toEqual({
    id: "new-meeting",
  });
  await expect(
    page.getByRole("button", { name: "Continue", exact: true }),
  ).toBeVisible();
  expect(await calls(page, "generate_notes")).toHaveLength(0);
});

test("notes edit and refresh preserve drafts; all three transcript versions stay separate", async ({
  page,
}) => {
  await open(page, {
    meeting: {
      cleanedTranscript: transcript.replace(
        "Keep the backup.",
        "Preserve the backup.",
      ),
    },
  });
  await meeting(page);
  await expect(
    page.getByRole("tab", { name: "Notes", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await capture(page, "04-notes-page.png");
  await page.getByRole("button", { name: "Edit notes" }).click();
  await page
    .getByRole("textbox", { name: "Meeting notes", exact: true })
    .fill("# Summary\nMy unsaved notes.");
  await page.evaluate(() =>
    window.__testUpdateMeeting({
      notes: "Background notes.",
      title: "Background title",
    }),
  );
  await expect(
    page.getByRole("heading", { name: "Background title", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Meeting notes", exact: true }),
  ).toHaveValue("# Summary\nMy unsaved notes.");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  expect((await calls(page, "save_meeting"))[0].args.title).toBe(
    "Background title",
  );
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
  await expect(
    page.getByText("Cleaned transcript", { exact: true }),
  ).toBeVisible();
  await menu(page, "Transcript version…");
  await page.getByRole("button", { name: "As heard", exact: true }).click();
  await expect(page.locator(".raw-transcript")).toHaveText(
    "um the deadline is Friday keep the backup",
  );
  await menu(page, "Transcript version…");
  await page.getByRole("button", { name: "Edited", exact: true }).click();
  await page
    .getByRole("button", { name: "Keep the backup.", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Edit Speaker 2 passage" })
    .fill("Keep both backups.");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  expect((await calls(page, "save_meeting")).at(-1).args).toMatchObject({
    transcript: transcript.replace("Keep the backup.", "Keep both backups."),
    cleanedTranscript: transcript.replace(
      "Keep the backup.",
      "Preserve the backup.",
    ),
  });
});

test("speaker renames use the command, survive labels off/on, and keep timecodes and outages", async ({
  page,
}) => {
  await open(
    page,
    {
      meeting: {
        notes: "",
        audioOutages: [
          {
            source: "system",
            startSeconds: 3,
            endSeconds: 15,
            reason: "System audio disconnected",
          },
        ],
        missingSeconds: 12,
      },
    },
    "slate",
  );
  await meeting(page);
  await page
    .getByRole("button", { name: "The deadline is Friday.", exact: true })
    .click();
  await page.getByLabel("Speaker name", { exact: true }).fill("dana");
  await capture(page, "05-transcript-rename-slate.png");
  await page.getByRole("button", { name: "Rename everywhere" }).click();
  expect((await calls(page, "rename_meeting_speaker"))[0].args).toEqual({
    id: "meeting-one",
    from: "Speaker 1",
    to: "dana",
  });
  await expect(page.locator(".passage-who").first()).toContainText("dana");
  await menu(page);
  await page.getByRole("switch", { name: "Speaker names" }).click();
  await page.getByRole("switch", { name: "Speaker names" }).click();
  await page.getByRole("button", { name: "Close sheet" }).click();
  await expect(page.locator(".passage-who").first()).toContainText("dana");
  await expect(page.getByLabel("Transcript gap")).toContainText(
    "System audio disconnected",
  );
  await expect(page.locator(".passage-who time").first()).toHaveText("0:01");
});

test("the single menu joins without deleting parts and deletion requires confirmation", async ({
  page,
}) => {
  await open(page);
  await meeting(page);
  await menu(page, "Join with other meetings…");
  await page
    .getByRole("checkbox", { name: "Client call", exact: false })
    .check();
  await capture(page, "06-join-menu.png");
  await page.getByRole("button", { name: "Join 2", exact: true }).click();
  expect((await calls(page, "join_meetings"))[0].args.ids).toEqual([
    "meeting-one",
    "meeting-outage",
  ]);
  expect(await calls(page, "delete_meeting")).toHaveLength(0);
  await menu(page, "Delete…");
  await page
    .getByRole("dialog", { name: "Delete this meeting?" })
    .getByRole("button", { name: "Cancel" })
    .click();
  expect(await calls(page, "delete_meeting")).toHaveLength(0);
  await menu(page, "Delete…");
  await page
    .getByRole("dialog", { name: "Delete this meeting?" })
    .getByRole("button", { name: "Delete meeting", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: /Design review/ }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: /Client call/ })).toBeVisible();
});

test("Notes stage shares OpenRouter, keeps manual generation, and is reachable from Settings", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  await page.getByLabel("Style", { exact: true }).selectOption("tasks");
  await capture(page, "07-notes-stage.png");
  await page.getByRole("button", { name: /Online ·/ }).click();
  await expect(
    page.getByRole("dialog", { name: "OpenRouter", exact: true }),
  ).toContainText("For meeting notes");
  await page.getByRole("button", { name: "Close sheet" }).click();
  expect(await calls(page, "generate_notes")).toHaveLength(0);
  await page
    .getByRole("button", { name: "Back to Meetings", exact: true })
    .click();
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("button", { name: "Settings", exact: true })
    .click();
  await page.getByRole("button", { name: /^Meetings / }).click();
  await page.getByRole("button", { name: /^Notes / }).click();
  await expect(page.getByLabel("Style", { exact: true })).toHaveValue("tasks");
  await page
    .getByRole("button", { name: "Back to Settings", exact: true })
    .click();
  await page
    .getByRole("navigation", { name: "Main navigation" })
    .getByRole("button", { name: "Meetings", exact: true })
    .click();
  await meeting(page);
  await menu(page, "Rewrite notes…");
  await expect(page.getByLabel("Style", { exact: true })).toHaveValue("tasks");
  await page.getByRole("button", { name: "Write notes", exact: true }).click();
  expect((await calls(page, "generate_notes"))[0].args.preset).toBe("tasks");
});

test("finalizing blocks audio actions, then a missed completion event recovers the saved recording", async ({
  page,
}) => {
  await open(page, {
    meeting: {
      status: "finalizing",
      notes: "",
      transcript: "",
      hasTranscript: false,
    },
  });
  await meeting(page);
  await expect(
    page.getByRole("region", { name: "Meeting progress" }),
  ).toContainText("Saving audio…");
  await expect(
    page.getByRole("button", { name: "Transcribe recording", exact: true }),
  ).toBeDisabled();
  await menu(page, "Other formats…");
  const wav = page.getByRole("button", {
    name: "Save recording (.wav)",
    exact: true,
  });
  await expect(wav).toBeDisabled();
  await page.evaluate(() =>
    window.__testUpdateMeeting({ status: "ready", durationSeconds: 72 }, false),
  );
  await expect(wav).toBeEnabled({ timeout: 6500 });
  await page.getByRole("button", { name: "Close sheet" }).click();
  await expect(
    page.getByRole("region", { name: "Meeting progress" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Transcribe recording", exact: true }),
  ).toBeEnabled();
});

test("a disconnected remote transcription choice never falls back to a local model", async ({
  page,
}) => {
  await open(page, {
    aiSettings: {
      backend: "local",
      connected: true,
      speechModel: "mock/audio",
    },
  });
  await meeting(page);
  await menu(page, "Transcribe again…");
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("openrouter");
  await page.getByRole("button", { name: "Close sheet" }).click();
  await page.evaluate(() => window.__testPatchAiSettings({ connected: false }));
  await menu(page, "Transcribe again…");
  await expect(
    page.locator('#transcription-model option[value="openrouter"]'),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Transcribe", exact: true }),
  ).toBeDisabled();
  expect(await calls(page, "transcribe_meeting")).toHaveLength(0);
  await page
    .getByLabel("Speech model", { exact: true })
    .selectOption("moonshine-medium");
  await expect(
    page.getByRole("button", { name: "Transcribe", exact: true }),
  ).toBeEnabled();
});

test("speaker-model download progress and failed settings keep the saved preference", async ({
  page,
}) => {
  await open(page);
  await page
    .getByRole("button", { name: "Sources, speech and speakers" })
    .click();
  await page.getByRole("button", { name: /^Install ·/ }).click();
  await expect(
    page
      .getByRole("region", { name: "Model download" })
      .getByRole("progressbar"),
  ).toBeVisible();
  await page.evaluate(() =>
    window.__testLibraryProgress(
      "nemotron-3-diarization",
      103871648,
      "installed",
    ),
  );
  const toggle = page.getByRole("switch", {
    name: "Speaker names",
    exact: true,
  });
  await expect(toggle).toBeChecked({ timeout: 7000 });
  await page.evaluate(() => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command === "save_settings")
        throw new Error("Mock settings write failed");
      return invoke(command, args);
    };
  });
  await toggle.click();
  await expect(page.getByRole("alert")).toContainText(
    "Mock settings write failed",
  );
  await expect(toggle).toBeChecked();
});

test("changing speaker labels asks before replacing saved text edits", async ({
  page,
}) => {
  await open(page, { meeting: { notes: "" } });
  await meeting(page);
  await page
    .getByRole("button", { name: "The deadline is Friday.", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Edit Speaker 1 passage" })
    .fill("Edited.");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await menu(page);
  const toggle = page.getByRole("switch", { name: "Speaker names" });
  await toggle.click();
  const confirm = page.getByRole("dialog", {
    name: "Replace your transcript edits?",
  });
  await confirm.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(toggle).toBeChecked();
  await toggle.click();
  await confirm
    .getByRole("button", { name: "Replace edits", exact: true })
    .click();
  await expect(toggle).not.toBeChecked();
  expect(
    (await calls(page, "set_meeting_speaker_labels")).at(-1).args,
  ).toMatchObject({ confirmed: true, enabled: false });
});

test("import passes a single speaker preference and audio export stays independent of unsaved text", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "Import…", exact: true }).click();
  await page.getByRole("button", { name: "Choose file", exact: true }).click();
  await page.getByLabel("One speaker?", { exact: false }).fill("Dana");
  await capture(page, "08-import-sheet.png");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  expect((await calls(page, "import_media"))[0].args).toEqual({
    path: "C:/fixture.wav",
    speaker: "Dana",
  });
  await page.getByRole("tab", { name: "Transcript", exact: true }).click();
  await page
    .getByRole("button", { name: "The deadline is Friday.", exact: true })
    .click();
  await page
    .getByRole("textbox", { name: "Edit Speaker 1 passage" })
    .fill("Unsaved edit.");
  await menu(page, "Other formats…");
  const dialog = page.getByRole("dialog", {
    name: "Other formats",
    exact: true,
  });
  await expect(
    dialog.getByRole("button", { name: "Export", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() => window.__testDialogSave(null));
  await dialog.getByRole("button", { name: "Save recording (.wav)" }).click();
  expect(await calls(page, "export_meeting_audio")).toHaveLength(0);
  await page.evaluate(() => window.__testDialogSave("C:/audio-copy.wav"));
  await dialog.getByRole("button", { name: "Save recording (.wav)" }).click();
  expect((await calls(page, "export_meeting_audio"))[0].args).toEqual({
    id: "imported",
    path: "C:/audio-copy.wav",
  });
});

test("macOS permission states and confirmed discard keep capture controls available", async ({
  page,
}) => {
  await open(page, {
    capabilities: {
      platform: "macos",
      systemAudioStatus: "denied",
      systemAudio: true,
    },
  });
  await page
    .getByRole("button", { name: "Sources, speech and speakers" })
    .click();
  await expect(page.getByRole("main")).toContainText("System Settings");
  await page.getByRole("button", { name: "Back to Meetings" }).click();
  await page
    .getByRole("button", { name: "Record meeting", exact: false })
    .click();
  const consent = page.getByRole("dialog", { name: "System audio is off" });
  await consent
    .getByRole("button", { name: "Record microphone only", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "Recording this meeting" }),
  ).toContainText("aren't being recorded");
  await page.getByRole("button", { name: "Recording options" }).click();
  await page.getByRole("button", { name: "Discard recording…" }).click();
  await page
    .getByRole("dialog", { name: "Discard this recording?" })
    .getByRole("button", { name: "Cancel", exact: true })
    .click();
  expect(await calls(page, "discard_meeting")).toHaveLength(0);
  await page.getByRole("button", { name: "Recording options" }).click();
  await page.getByRole("button", { name: "Discard recording…" }).click();
  await page
    .getByRole("dialog", { name: "Discard this recording?" })
    .getByRole("button", { name: "Discard recording", exact: true })
    .click();
  expect((await calls(page, "discard_meeting"))[0].args).toEqual({
    id: "new-meeting",
    confirmed: true,
  });
  expect(await calls(page, "stop_meeting")).toHaveLength(0);
  await expect(
    page.getByRole("region", { name: "Recording this meeting" }),
  ).toHaveCount(0);
});
