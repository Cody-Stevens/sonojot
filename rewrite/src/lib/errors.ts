export type ErrorAction =
  | "microphone"
  | "shortcuts"
  | "setup"
  | "converter"
  | "install-media"
  | "copy-text"
  | "meetings"
  | "vocabulary"
  | "appearance"
  | "retry"
  | "restart";
export interface ErrorPresentation {
  family: string;
  title: string;
  body: string;
  action: { id: ErrorAction; label: string };
  details: string;
}
interface ErrorRule {
  family: string;
  match: RegExp;
  title: string;
  body: string;
  action: ErrorPresentation["action"];
}

// Specific recovery cases precede broader subsystem matches. Native details never
// become primary copy, including unknown OS, permission and protocol errors.
const rules: readonly ErrorRule[] = [
  {
    // A hotkey or tray action after a terms update; the main window shows them.
    family: "terms-required",
    match: /^TERMS_REQUIRED:/,
    title: "Review the updated Terms of Use",
    body: "Open SonoJot to read and accept the Terms of Use, then try again.",
    action: { id: "setup", label: "Open SonoJot" },
  },
  {
    family: "license-acknowledgment",
    match: /^LICENSE_ACKNOWLEDGMENT_REQUIRED:/,
    title: "Review the download's license terms",
    body: "Open Models and choose the download again to review its license terms.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "meeting-consent",
    match: /^MEETING_CONSENT_REQUIRED:/,
    title: "Review the recording reminder",
    body: "Start the meeting again to review the recording reminder.",
    action: { id: "meetings", label: "Open meetings" },
  },
  {
    family: "setup-notes-download",
    match: /^SETUP_NOTES_DOWNLOAD:/,
    title: "Profile saved; notes download needs attention",
    body: "Your profile and speech model are ready. Open Models to retry the notes download.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "setup-incomplete",
    match: /^SETUP_INCOMPLETE:.*meetings/,
    title: "Finish setup to record meetings.",
    body: "Choose a profile in Models.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "setup-incomplete",
    match: /^SETUP_INCOMPLETE:/,
    title: "Finish setup to start dictating.",
    body: "Choose a profile in Models.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    // The transcript is saved; only the automatic name is missing. This
    // precedes the cloud and model rules its details would otherwise match.
    family: "meeting-title",
    match: /automatic title unavailable/i,
    title: "Transcript saved without a title",
    body: "Type a title for this meeting, or choose an installed notes model in Models to name meetings automatically.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "cloud-ai",
    match: /openrouter|cloud model|AI settings|sign-in|connection key/i,
    title: "The AI connection needs attention",
    body: "Check your connection and selected model in Models. You can also use the included local model.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "vocabulary-review",
    match:
      /vocabulary (?:review|suggestion)|review part|AI operation cancelled|local generation cancelled/i,
    title: "Vocabulary review did not finish",
    body: "Your saved vocabulary is unchanged. Try the review again or choose another model in Models.",
    action: { id: "vocabulary", label: "Open vocabulary" },
  },
  {
    family: "app-update",
    match:
      /desktop update could not be read|desktop service returned an invalid response/i,
    title: "The app update could not be read",
    body: "Reconnect to SonoJot and try the action again. If it continues, reopen the app.",
    action: { id: "restart", label: "Reopen app help" },
  },
  {
    family: "nothing-to-copy",
    match:
      /no (?:dictation|finished dictation|text).*copy|finish a dictation before copying/i,
    title: "Nothing to copy yet",
    body: "Finish a dictation first. Your completed text will then be available to copy.",
    action: { id: "retry", label: "Reload app state" },
  },
  {
    family: "folder",
    match:
      /unknown app folder|app folder does not exist|cannot open (?:the )?folder/i,
    title: "That folder is unavailable",
    body: "Check the saved-data location in Models, or reopen the app and try again.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "preview",
    match:
      /browser preview|native recording.*preview|open the desktop app|window controls.*desktop app/i,
    title: "Open the desktop app",
    body: "Recording, files and saved settings are available in the installed SonoJot app.",
    action: { id: "restart", label: "Desktop app help" },
  },
  {
    family: "incomplete-recognition",
    match:
      /unfinished final segment|incomplete.*transcript|partial transcript was not published/i,
    title: "Dictation could not finish",
    body: "Recognition ended before the final words were complete. Start a new dictation to try again.",
    action: { id: "retry", label: "Reload app state" },
  },
  {
    family: "incomplete-recording",
    match:
      /partial (?:WAV |audio|files)|meeting is incomplete|lost audio|audio.*(?:queue overflow|timing buffer|timeline|timestamp|overlapped)|audio arrived after/i,
    title: "This recording is incomplete",
    body: "Part of the audio could not be captured or finalized. Open the meeting to review the available recording before transcribing it.",
    action: { id: "meetings", label: "Open meetings" },
  },
  {
    family: "media-support",
    match:
      /media support|media package|media install|downloaded (?:converter|file|media)/i,
    title: "Media support could not be installed",
    body: "Check the connection and try again. Nothing was changed. You can still locate an existing converter in Models.",
    action: { id: "install-media", label: "Open Models" },
  },
  {
    family: "converter",
    match:
      /ffmpeg|media converter|could not decode|media import.*(?:timed out|limit)/i,
    title: "This file could not be imported",
    body: "Check the media converter in Models, then choose a file with a supported audio track.",
    action: { id: "converter", label: "Check media converter" },
  },
  {
    family: "shortcut",
    match: /shortcut|hotkey|cannot register .*another app/i,
    title: "The shortcut is unavailable",
    body: "Choose a valid key combination that another app is not using, then save it again.",
    action: { id: "shortcuts", label: "Open shortcuts" },
  },
  {
    family: "clipboard",
    match:
      /clipboard|paste|already being copied|no (?:dictation|finished dictation|text).*copy|no dictation to copy/i,
    title: "Text could not be copied",
    body: "Return to your text and try Copy again. If text is available below, you can select and copy it manually.",
    action: { id: "copy-text", label: "Copy available text" },
  },
  {
    family: "microphone",
    match:
      /microphone|input (?:audio |sample )?(?:device|format|rate)|no default input/i,
    title: "The microphone is unavailable",
    body: "Check the selected microphone and its access permissions, then try recording again.",
    action: { id: "microphone", label: "Check microphone" },
  },
  {
    family: "system-audio",
    match:
      /system audio|system capture|output device|WASAPI|loopback|audio COM|Windows.*audio clock/i,
    title: "System audio is unavailable",
    body: "Check the Windows output device. You can choose Microphone in Meetings if you only need your own voice.",
    action: { id: "meetings", label: "Open meetings" },
  },
  {
    family: "model-install",
    match:
      /speech model download|text model download|downloadable (?:speech|text)|optional (?:speech|text) model|speech runtime|a (?:speech |text )?model is already being installed/i,
    title: "The speech model could not be installed",
    body: "Check the connection and try again. Nothing was changed. Installed models stay in place.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "model-memory",
    match: /^Memory unavailable:/i,
    title: "Not enough memory to load this model",
    body: "Free some memory or choose a smaller model in Models. Saved dictations are unchanged.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "model-setup",
    match:
      /model.*(?:not installed|not found|missing|identifier|catalogue|folder|path)|choose.*model|moonshine.*(?:requires|ABI|load)|cannot load (?:.*[.]dll|moonshine)|recognizer:|native runtime|cleanup.*server.*not installed/i,
    title: "Speech files need attention",
    body: "Open Models and choose the local folder containing the required speech files. Models will check the folder before it is used.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "local-model",
    match:
      /local (?:cleanup|model|notes)|cleanup (?:process|worker|edit|executable)|cleanup.*(?:lifetime|deadline)|notes (?:plan|evidence)|model (?:response|output|returned|selected)|invalid notes evidence|unsupported or duplicate notes/i,
    title: "Text processing could not finish",
    body: "Try again. If this keeps happening, check the local speech files in Models. The original transcript remains separate from generated notes.",
    action: { id: "setup", label: "Check speech files" },
  },
  {
    family: "notes-input",
    match:
      /note instructions|note.*2,000|transcript is required|transcribe.*before.*notes|instructions.*characters/i,
    title: "Notes need a saved transcript",
    body: "Transcribe and save this meeting first. Keep any note instructions within the field’s limit, then try again.",
    action: { id: "meetings", label: "Open meeting" },
  },
  {
    family: "vocabulary",
    match:
      /vocabulary|heard phrase|written form|each heard|enabled correction|correction.*unambiguous/i,
    title: "Vocabulary needs a correction",
    body: "Give each entry a heard phrase and written form. Use each heard phrase once and check the entry limits before saving.",
    action: { id: "vocabulary", label: "Review vocabulary" },
  },
  {
    family: "export-format",
    match: /PDF|export.*(?:format|text|markdown|html)|embedded.*font/i,
    title: "This document could not be exported",
    body: "Choose HTML or plain text to keep every character, then export the meeting again.",
    action: { id: "meetings", label: "Choose export format" },
  },
  {
    family: "export-destination",
    match:
      /export (?:destination|directory)|destination directory|destination file|original meeting media|overwrite.*(?:linked|original)/i,
    title: "Choose another export location",
    body: "Choose an existing folder outside the app’s saved data and use a new filename.",
    action: { id: "meetings", label: "Return to export" },
  },
  {
    family: "recording-conflict",
    match:
      /(?:finish|stop|cancel).*(?:before|current)|already (?:active|running|recording|has.*(?:active operation|saved audio))|no (?:dictation|meeting).*active|no longer (?:recording|current)|operation.*still stopping|recognition stream.*stopped/i,
    title: "Another recording action is in progress",
    body: "Finish or cancel the current recording or processing step, then try this action again.",
    action: { id: "retry", label: "Reload app state" },
  },
  {
    family: "interrupted",
    match:
      /cancelled|canceled|interrupted|lock failed|worker.*(?:unavailable|disconnected)|event handler/i,
    title: "The operation stopped",
    body: "Try the action again. If the app no longer responds, reopen it before continuing.",
    action: { id: "retry", label: "Reload app state" },
  },
  {
    family: "recording-file",
    match:
      /WAV|RIFF|PCM|audio.*(?:format|buffer|sample|frame|header|chunk|byte rate)|(?:no audio|contains no audio)|(?:recording|media file|meeting audio)|meeting (?:recorder|capture|source|folder)/i,
    title: "The recording could not be used",
    body: "Check the recording source or choose another file with an audio track, then try again.",
    action: { id: "meetings", label: "Review recording" },
  },
  {
    family: "recognition",
    match: /moonshine|recognition|recognizer|transcription/i,
    title: "Speech recognition could not finish",
    body: "Try recording again. If it fails again, check the local speech files in Models.",
    action: { id: "setup", label: "Check speech files" },
  },
  {
    family: "startup",
    match:
      /launch.at.login|login registration|installed executable|installed .*exe|startup/i,
    title: "Start at login could not be changed",
    body: "Open Shortcuts and try saving the setting again. This option requires the installed desktop app.",
    action: { id: "shortcuts", label: "Open Shortcuts" },
  },
  {
    family: "settings",
    match: /auto.stop|settings(?! file)|valid.*model/i,
    title: "The setting could not be saved",
    body: "Check the selected value and try saving it again. Finish any active recording before changing dictation settings.",
    action: { id: "setup", label: "Review settings" },
  },
  {
    family: "meeting-data",
    match:
      /meeting (?:title|duration|library|index|record)|(?:list|inspect|remove).*meeting|transcript history/i,
    title: "This meeting needs attention",
    body: "Check the meeting title and saved files, then try the action again.",
    action: { id: "meetings", label: "Open meetings" },
  },
  {
    family: "storage",
    match:
      /data (?:file|directory|path|store)|saved (?:file|directory)|schema version|serialize|record identifier|mismatched identifier|creation date|dictation history|legacy|linked files|cannot (?:read|save|replace|flush|create|inspect)|directory does not exist|writable/i,
    title: "Saved data could not be accessed",
    body: "Check that the app’s data folder is available and writable, then try again. Details identify the file or limit involved.",
    action: { id: "setup", label: "Open Models" },
  },
  {
    family: "desktop",
    match:
      /desktop (?:app|service)|invalid response|window|tauri|https?:|ipc|permission|not allowed|not implemented|platform|unknown app folder|cannot open.*folder/i,
    title: "The desktop app could not complete this action",
    body: "Try again in the installed app. If the problem continues, reopen SonoJot and check Details.",
    action: { id: "restart", label: "Reopen app help" },
  },
];

function detailsFor(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error) ?? String(error);
  } catch {
    return "The error details could not be read.";
  }
}

export function presentError(
  error: unknown,
  context: { savedMeetingAudio?: boolean } = {},
): ErrorPresentation {
  const details = detailsFor(error);
  const rule = rules.find((candidate) => candidate.match.test(details));
  if (
    context.savedMeetingAudio &&
    rule &&
    [
      "incomplete-recording",
      "recording-file",
      "microphone",
      "system-audio",
    ].includes(rule.family)
  )
    return {
      family: "saved-recording",
      title: "Recording saved with interruptions",
      body: "The saved recording can still be transcribed. Some audio may be missing. Choose Transcribe recording below the transcript.",
      action: { id: "meetings", label: "Open meetings" },
      details,
    };
  if (rule)
    return {
      family: rule.family,
      title: rule.title,
      body: rule.body,
      action: rule.action,
      details,
    };
  return {
    family: "unknown",
    title: "The action could not be completed",
    body: "Try again. If the problem continues, reopen the app. Details may help explain what happened.",
    action: { id: "retry", label: "Reload app state" },
    details,
  };
}
