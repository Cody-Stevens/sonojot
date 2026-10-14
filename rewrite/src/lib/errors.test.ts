import { strict as assert } from "node:assert";
import { test } from "node:test";
import { presentError } from "./errors";

const families: [string, string][] = [
  ["setup-incomplete", "SETUP_INCOMPLETE: Finish setup to start dictating."],
  ["setup-incomplete", "SETUP_INCOMPLETE: Finish setup to record meetings."],
  [
    "terms-required",
    "TERMS_REQUIRED: Open SonoJot and review the Terms of Use to continue.",
  ],
  [
    "license-acknowledgment",
    "LICENSE_ACKNOWLEDGMENT_REQUIRED: Review NVIDIA components for Granite in Models before downloading.",
  ],
  [
    "meeting-consent",
    "MEETING_CONSENT_REQUIRED: Review the recording reminder before starting a meeting.",
  ],
  [
    "setup-notes-download",
    "SETUP_NOTES_DOWNLOAD: The text model download could not start.",
  ],
  [
    "setup-notes-download",
    "SETUP_NOTES_DOWNLOAD: Not enough disk space: need about 2.8 GB free, found 1.7 GB.",
  ],
  [
    "disk-space",
    "Not enough disk space for the speech model and Metal runtime. Free space and try again.",
  ],
  [
    "disk-space",
    "Not enough disk space: need about 1.9 GB free, found 1.7 GB. Free space and try again.",
  ],
  [
    "meeting-title",
    "Transcript saved. Automatic title unavailable: The selected notes model is not installed. Choose an installed model in Setup before generating titles.",
  ],
  [
    "meeting-title",
    "Transcript saved. Automatic title unavailable: OpenRouter returned HTTP 502.",
  ],
  [
    "app-update",
    "A desktop update could not be read. Reopen SonoJot to reconnect.",
  ],
  ["folder", "The app folder does not exist yet."],
  ["nothing-to-copy", "Finish a dictation before copying its text."],
  [
    "preview",
    "This action is available in the desktop app. Browser preview does not record audio or save app data.",
  ],
  [
    "incomplete-recognition",
    "Recognition returned an unfinished final segment; the partial transcript was not published.",
  ],
  [
    "incomplete-recording",
    "System capture reported lost audio; this meeting is incomplete. Partial files are preserved.",
  ],
  [
    "incomplete-recording",
    "Timed out stopping meeting capture. Partial WAV files are preserved; finalization continues in the background.",
  ],
  [
    "converter",
    "FFmpeg is required for media import. Install FFmpeg or set SONOJOT_FFMPEG to its executable.",
  ],
  ["converter", "Cannot start FFmpeg: access denied"],
  [
    "shortcut",
    "Cannot register Ctrl+Space. Another app may be using it: permission denied",
  ],
  ["shortcut", "Choose a valid shortcut (up to 128 characters)."],
  ["clipboard", "Cannot open native clipboard utility: access denied"],
  ["nothing-to-copy", "There is no dictation to copy yet."],
  ["microphone", "Microphone did not become ready within 8 seconds."],
  ["microphone", "The microphone identifier is invalid."],
  [
    "system-audio",
    "Cannot initialize WASAPI system audio capture: unavailable",
  ],
  [
    "system-audio",
    "The default output device uses an unsupported audio format.",
  ],
  ["model-setup", "Local cleanup model or llama.cpp server is not installed."],
  [
    "model-install",
    "The speech model download could not start: connection reset",
  ],
  [
    "model-install",
    "The text model download could not start: connection reset",
  ],
  [
    "model-setup",
    "Moonshine ABI 1 is incompatible with the bundled API 2; install a matching native runtime.",
  ],
  ["model-setup", "Choose a model from the installed model catalogue."],
  ["local-model", "Local cleanup returned HTTP 500."],
  [
    "local-model",
    "The local model selected unsupported or duplicate notes evidence.",
  ],
  ["local-model", "Cannot contain cleanup process lifetime: access denied"],
  ["notes-input", "Keep note instructions under 2,000 characters."],
  ["notes-input", "Transcribe this meeting before generating notes."],
  [
    "vocabulary",
    "This heard phrase is already in your vocabulary. Edit its existing entry.",
  ],
  [
    "vocabulary",
    "Only one enabled correction is allowed for each heard phrase.",
  ],
  [
    "export-format",
    "The PDF font does not contain U+FFFF. Export as HTML or text to preserve every character.",
  ],
  ["export-format", "Choose text, markdown, html, or pdf export."],
  [
    "export-destination",
    "The export destination is original meeting media. Choose a different filename.",
  ],
  [
    "recording-conflict",
    "Stop the meeting recording before starting dictation.",
  ],
  ["recording-conflict", "This meeting already has an active operation."],
  ["recording-conflict", "The previous meeting operation is still stopping."],
  [
    "interrupted",
    "An operation was interrupted. Restart SonoJot before continuing.",
  ],
  ["interrupted", "Meeting operation cancelled."],
  ["recording-file", "Meeting audio has an incomplete WAV header."],
  ["recording-file", "The selected file contains no audio."],
  ["recognition", "Moonshine returned invalid UTF-8 text."],
  ["startup", "Cannot register launch at login: access denied"],
  ["startup", "The installed executable path contains unsupported characters."],
  ["settings", "Auto-stop must be between 0 and 300 seconds."],
  [
    "meeting-data",
    "Meeting titles must contain 1–1,024 bytes without control characters.",
  ],
  ["storage", "Set SONOJOT_DATA_DIR to a writable data directory."],
  ["storage", "This settings file uses an unsupported schema version."],
  ["storage", "Cannot read data file: access denied"],
  [
    "app-update",
    "The desktop service returned an invalid response for get_snapshot. Reopen SonoJot and try again.",
  ],
  ["preview", "Window controls are available in the desktop app."],
  ["desktop", "IPC rejected tauri://localhost: HTTP 403 SONOJOT_INTERNAL"],
];
for (const [family, raw] of families) {
  test(`error family ${family}: ${raw.slice(0, 65)}`, () => {
    const message = presentError(raw);
    assert.equal(message.family, family);
    assert.equal(message.details, raw);
    assert.ok(message.title && message.body && message.action.label);
    assert.doesNotMatch(
      `${message.title} ${message.body} ${message.action.label}`,
      /tauri:|https?:|HTTP|SONOJOT_|llama[.]cpp|FFmpeg|IPC/i,
    );
  });
}

test("unfinished setup errors keep a usable Setup action for app and screen tab", () => {
  for (const raw of [
    "SETUP_INCOMPLETE: Finish setup to start dictating.",
    "SETUP_INCOMPLETE: Finish setup to record meetings.",
  ]) {
    const message = presentError(raw);
    assert.equal(message.action.id, "setup");
    assert.equal(message.action.label, "Open Models");
    assert.doesNotMatch(`${message.title} ${message.body}`, /SETUP_INCOMPLETE/);
  }
});

test("unrecognized failures keep raw diagnostics behind Details", () => {
  const raw = "Error 0x80000001 at private-file-path";
  const message = presentError(new Error(raw));
  assert.equal(message.family, "unknown");
  assert.equal(message.details, raw);
  assert.ok(!message.body.includes(raw));
  assert.equal(message.action.id, "retry");
});

test("reload recovery labels describe the app-state refresh they trigger", () => {
  for (const raw of [
    "Finish a dictation before copying its text.",
    "Recognition returned an unfinished final segment.",
    "A recording is already active.",
    "Meeting operation cancelled.",
    "An unknown failure.",
  ]) {
    assert.deepEqual(presentError(raw).action, {
      id: "retry",
      label: "Reload app state",
    });
  }
  assert.deepEqual(
    presentError("Media support could not be installed.").action,
    { id: "install-media", label: "Open Models" },
  );
});

test("saved meeting audio gets transcription guidance for legacy capture failures", () => {
  for (const raw of [
    "System capture reported lost audio; this meeting is incomplete. Partial files are preserved.",
    "Microphone disconnected.",
    "Meeting recorder failed: worker stopped.",
  ]) {
    const message = presentError(raw, { savedMeetingAudio: true });
    assert.equal(message.family, "saved-recording");
    assert.match(message.body, /saved recording can still be transcribed/i);
    assert.match(message.body, /Transcribe recording/);
    assert.equal(message.details, raw);
    assert.notEqual(presentError(raw).family, "saved-recording");
  }
  assert.equal(
    presentError("The selected speech model is not installed.", {
      savedMeetingAudio: true,
    }).family,
    "model-setup",
  );
});

test("unknown structured and circular errors have a safe primary presentation", () => {
  assert.equal(
    presentError({ private: "HTTP 500" }).details,
    '{"private":"HTTP 500"}',
  );
  const circular: { loop?: unknown } = {};
  circular.loop = circular;
  assert.equal(presentError(circular).family, "unknown");
});
