import { z } from "zod";
import {
  pendingCrashSchema,
  reportSchema,
  reportReceiptSchema,
  type Report,
  type ReportKind,
} from "./reports";
import { downloadConsentSchema } from "./download-consent";
import { setupStatusSchema } from "./setup";
import {
  hardwareProfileSchema,
  setupProfileSchema,
  type SetupProfile,
} from "./setup-profiles";
import { accelerationSchema } from "./acceleration";
import { modelKindSchema, modelLibrarySchema } from "./model-library";
import {
  aiSettingsSchema,
  aiModelSchema,
  aiModelCheckSchema,
  vocabularySuggestionSchema,
} from "./ai";

export const memoryModeSchema = z.enum(["overlap", "sequential"]);
export type MemoryMode = z.infer<typeof memoryModeSchema>;
export const modelRetentionSchema = z.enum([
  "immediately",
  "10m",
  "30m",
  "exit",
]);
export type ModelRetention = z.infer<typeof modelRetentionSchema>;

export const notesPresetSchema = z.enum([
  "summary",
  "tasks",
  "speakers",
  "brief",
  "formattedTranscript",
  "custom",
]);
export type NotesPreset = z.infer<typeof notesPresetSchema>;

export const trustSettingsSchema = z.object({
  enabled: z.boolean().default(true),
  sensitivity: z.enum(["fewer", "balanced", "more"]).default("balanced"),
  markStyle: z.enum(["band", "dots"]).default("band"),
  popoverStyle: z.enum(["card", "strip"]).default("card"),
  playOn: z.enum(["hover", "click"]).default("hover"),
  offApps: z.array(z.string()).default([]),
  nextShortcut: z.string().default(""),
});
export type TrustSettings = z.infer<typeof trustSettingsSchema>;
const trustDefaults: TrustSettings = {
  enabled: true,
  sensitivity: "balanced",
  markStyle: "band",
  popoverStyle: "card",
  playOn: "hover",
  offApps: [],
  nextShortcut: "",
};
export const settingsSchema = z.object({
  updateChannel: z.enum(["stable", "nightly"]).default("stable"),
  termsAcceptedVersion: z.number().int().nonnegative().default(0),
  termsAcceptedAt: z.string().nullable().default(null),
  meetingConsentAcknowledged: z.boolean().default(false),
  componentAcknowledgments: z.record(z.string(), z.string()).default({}),
  setupProfile: setupProfileSchema.nullable().default("custom"),
  punctuationRecommendationAcknowledged: z.boolean().default(false),
  punctuationV2RecommendationAcknowledged: z.boolean().default(false),
  punctuationV3RecommendationAcknowledged: z.boolean().default(false),
  theme: z.enum(["fog", "slate"]),
  material: z.boolean(),
  cleanupMode: z.enum(["clean", "original", "format"]),
  cleanupTrial: z.boolean().default(false),
  smartLayout: z.boolean().default(false),
  smartLayoutAppHint: z.boolean().default(false),
  proofreader: z.boolean().default(false),
  localProofreader: z.boolean().default(true),
  proofreaderModel: z.string().default("anthropic/claude-opus-5.5"),
  proofreaderTrainingModels: z.array(z.string()).default([]),
  writtenForm: z.boolean().default(true),
  sonojotPunctuation: z.boolean().default(true),
  launchAtLogin: z.boolean(),
  hotkey: z.string(),
  hotkeyMode: z.enum(["toggle", "hold"]),
  autoPaste: z.boolean(),
  audioCues: z.boolean(),
  /** Tiny-model draft beside the speech model while dictating. Display only. */
  livePreview: z.boolean().default(true),
  autoStopSeconds: z.number().finite().nonnegative(),
  microphone: z.string().nullable(),
  asrModel: z.string(),
  meetingAsrModel: z.string().default(""),
  cleanupModel: z.string(),
  notesModel: z.string().default("qwen3-4b-instruct-2507"),
  textAcceleration: z.enum(["auto", "cpu"]).default("auto"),
  startTabHidden: z.boolean().default(false),
  cursorIndicator: z.boolean().default(true),
  autoMeetingTitles: z.boolean().default(true),
  meetingNotesPreset: notesPresetSchema.nullable().default(null),
  meetingNotesInstructions: z.string().default(""),
  meetingSpeakerLabels: z.boolean().default(true),
  /** Two-source meetings: drop far-end words the microphone picked up from speakers. Null: automatic (on with word-timed recognizers). */
  meetingRemoveEcho: z.boolean().nullable().default(null),
  meetingSource: z.enum(["microphone", "system", "both"]).default("both"),
  meetingSourceMigrated: z.boolean().default(false),
  systemAudioDefaultApplied: z.boolean().default(false),
  doNotSaveDictations: z.boolean().default(false),
  /** Keep dictation audio locally for 30 days to improve the user's own models. */
  keepDictationAudio: z.boolean().default(false),
  learnFromCorrections: z.boolean().default(false),
  memoryMode: memoryModeSchema.default("overlap"),
  modelRetention: modelRetentionSchema.default("10m"),
  /** Doubtful-word marks after a paste. */
  trust: trustSettingsSchema.default(trustDefaults),
});
export type Settings = z.infer<typeof settingsSchema>;
export const systemAudioStatusSchema = z.enum([
  "ready",
  "notDetermined",
  "denied",
  "unsupported",
]);
export type SystemAudioStatus = z.infer<typeof systemAudioStatusSchema>;

export const modelSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: modelKindSchema,
  ready: z.boolean(),
  description: z.string(),
  sizeBytes: z.number().nonnegative(),
  installable: z.boolean().default(false),
  removable: z.boolean().default(false),
});
export type Model = z.infer<typeof modelSchema>;
export const recognitionCompletenessSchema = z.enum(["complete", "incomplete"]);
export type RecognitionCompleteness = z.infer<
  typeof recognitionCompletenessSchema
>;
export const updateStatusSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("idle") }),
  z.object({ state: z.literal("checking") }),
  z.object({
    state: z.literal("current"),
    channel: z.enum(["stable", "nightly"]),
    version: z.string().min(1),
    platform: z.string().min(1),
    reason: z.enum(["noBuild", "noNewer", "current"]),
  }),
  z.object({
    state: z.literal("available"),
    version: z.string().min(1),
    notes: z.string().nullable(),
    size: z.number().int().nonnegative().nullable(),
    /** Not newer: reinstalling finishes an incomplete installation. */
    repair: z.boolean().optional(),
    /** Older than this build: the newest build on the channel just chosen. */
    older: z.boolean().optional(),
    switchChannel: z.enum(["stable", "nightly"]).nullish(),
  }),
  z.object({
    state: z.literal("downloading"),
    percent: z.number().finite().min(0).max(100).nullable(),
    switchChannel: z.enum(["stable", "nightly"]).nullish(),
  }),
  z.object({
    state: z.literal("ready"),
    version: z.string().min(1),
    notes: z.string().nullable(),
    older: z.boolean().optional(),
    switchChannel: z.enum(["stable", "nightly"]).nullish(),
  }),
  z.object({ state: z.literal("error"), message: z.string() }),
]);
export type UpdateStatus = z.infer<typeof updateStatusSchema>;
export const updateSchema = z.object({
  currentVersion: z.string(),
  status: updateStatusSchema,
  restartBlockedReason: z.string().nullable(),
  installing: z.boolean(),
  /** Later was chosen on the main-window notice for the offered version. */
  noticeDismissed: z.boolean().default(false),
  /** Why the installed files don't match the running app, if they don't. */
  installProblem: z.string().nullable().default(null),
});

/** One version's user-facing notes. Plain text, rendered as text only. */
export const releaseNotesSchema = z.object({
  version: z.string().min(1),
  /** YYYY-MM-DD, or empty when only the update manifest's notes are known. */
  date: z.string(),
  notes: z.array(z.string()),
});
export type ReleaseNotes = z.infer<typeof releaseNotesSchema>;
export const whatsNewSchema = z.object({
  from: z.string().nullable(),
  to: z.string(),
  /** Newest first. */
  releases: z.array(releaseNotesSchema),
});
export type WhatsNew = z.infer<typeof whatsNewSchema>;
export const whatsNewStateSchema = z.object({
  /** Notes for the update just installed, not yet seen. */
  pending: whatsNewSchema.nullable(),
  /** The most recent update's notes, kept for Updates after dismissal. */
  latest: whatsNewSchema.nullable(),
});
export type WhatsNewState = z.infer<typeof whatsNewStateSchema>;

export const snapshotSchema = z.object({
  localProofreaderAvailable: z.boolean().default(false),
  revision: z.number().int().nonnegative().default(0),
  meetingsRevision: z.number().int().nonnegative().default(0),
  sessionId: z.number().int().nonnegative().nullable().default(null),
  tabVisible: z.boolean().default(true),
  tabExpanded: z.boolean().default(false),
  tabTemporary: z.boolean().default(false),
  canCopyLast: z.boolean().default(false),
  deliveryNotice: z.string().nullable().default(null),
  resultSessionId: z.number().int().nonnegative().nullable().default(null),
  recognitionCompleteness: recognitionCompletenessSchema
    .nullable()
    .default(null),
  recognitionWarnings: z.array(z.string()).default([]),
  recognitionProgress: z
    .object({
      stage: z.enum(["loading", "transcribing", "recovering"]),
      elapsedMs: z.number().int().nonnegative(),
    })
    .nullable()
    .default(null),
  settings: settingsSchema,
  state: z.enum([
    "idle",
    "loading",
    "listening",
    "finishing",
    "cleaning",
    "error",
  ]),
  originalText: z.string(),
  outputText: z.string(),
  preModelText: z.string().nullable().default(null),
  publicationStage: z.string().nullable().default(null),
  cleanupStatus: z.string().nullable().default(null),
  guardReason: z.string().nullable().default(null),
  interimText: z.string(),
  /** Live-preview draft after interimText. Never saved, copied or pasted. */
  previewText: z.string().default(""),
  audioLevel: z.number().finite().min(0).max(1),
  meetingLevels: z
    .object({
      microphone: z.number().finite().min(0).max(1),
      system: z.number().finite().min(0).max(1),
    })
    .nullable()
    .default(null),
  meetingSourceStatus: z.array(z.string()).default([]),
  /** Why the other side of the current meeting is missing, if it is. */
  meetingSystemAudio: z
    .enum(["permission", "lost", "unsupported"])
    .nullable()
    .default(null),
  error: z.string().nullable(),
  models: z.array(modelSchema),
  devices: z.array(
    z.object({ id: z.string(), name: z.string(), isDefault: z.boolean() }),
  ),
  meetingRecording: z.boolean(),
  activeMeetingId: z.string().nullable(),
  capabilities: z.object({
    sync: z.boolean().default(false),
    autoPaste: z.boolean(),
    systemAudio: z.boolean(),
    systemAudioStatus: systemAudioStatusSchema.default("unsupported"),
    globalShortcut: z.boolean(),
    platform: z.string(),
  }),
  version: z.string(),
  update: updateSchema.default({
    currentVersion: "",
    status: { state: "idle" },
    restartBlockedReason: null,
    installing: false,
    noticeDismissed: false,
    installProblem: null,
  }),
});
export type Snapshot = z.infer<typeof snapshotSchema>;

export const meetingSummarySchema = z.object({
  id: z.string(),
  title: z.string(),
  createdAt: z.string(),
  durationSeconds: z.number().nonnegative(),
  status: z.string(),
  hasAudio: z.boolean(),
  hasTranscript: z.boolean(),
  missingSeconds: z.number().nonnegative().optional(),
  speakerCount: z.number().int().nonnegative().optional(),
  autoNamed: z.boolean().optional(),
  source: z.string().optional(),
  hasNotes: z.boolean().optional(),
});
export const meetingDetailSchema = meetingSummarySchema.extend({
  recognitionCompleteness: recognitionCompletenessSchema.default("complete"),
  recognitionWarnings: z.array(z.string()).default([]),
  recordingWarnings: z.array(z.string()).default([]),
  audioOutages: z
    .array(
      z.object({
        source: z.string(),
        startSeconds: z.number().nonnegative(),
        endSeconds: z.number().nonnegative(),
        reason: z.string(),
      }),
    )
    .default([]),
  missingSeconds: z.number().nonnegative().default(0),
  cleanedTranscript: z.string().nullable().default(null),
  transcriptionStoppedAtSeconds: z
    .number()
    .nonnegative()
    .nullable()
    .default(null),
  transcript: z.string(),
  notes: z.string(),
  source: z.string(),
  audioPath: z.string().nullable(),
  error: z.string().nullable(),
  transcriptionModel: z.string().nullable().default(null),
  speakerCount: z.number().int().nonnegative().default(0),
  speakerLabels: z.boolean().default(true),
});
export type MeetingSummary = z.infer<typeof meetingSummarySchema>;
export type MeetingDetail = z.infer<typeof meetingDetailSchema>;
export const vocabularySchema = z.array(
  z.object({
    kind: z.enum(["replace", "hint"]).default("replace"),
    aliases: z.array(z.string()).default([]),
    context: z.array(z.string()).default([]),
    id: z.string(),
    heard: z.string(),
    written: z.string(),
    enabled: z.boolean(),
    scope: z.string().nullable().default(null),
  }),
);
export type VocabularyEntry = z.infer<typeof vocabularySchema>[number];
export const dictationRecordSchema = z.object({
  targetApp: z.string().nullable().default(null),
  id: z.string(),
  createdAt: z.string(),
  raw: z.string(),
  published: z.string(),
  preModelText: z.string().nullable().default(null),
  publicationStage: z.string().nullable().default(null),
  cleanupStatus: z.string().nullable().default(null),
  guardReason: z.string().nullable().default(null),
  recognitionCompleteness: recognitionCompletenessSchema.default("complete"),
  recognitionWarnings: z.array(z.string()).default([]),
});
export type DictationRecord = z.infer<typeof dictationRecordSchema>;
export const learnedEntrySchema = z.object({
  id: z.string(),
  kind: z.enum(["correction", "canonical", "context", "style"]),
  text: z.string(),
  heard: z.string().nullable(),
  scope: z.string().nullable(),
  sourceIds: z.array(z.string()),
  sourceType: z.string(),
  enabled: z.boolean(),
  userPinned: z.boolean(),
});
export type LearnedEntry = z.infer<typeof learnedEntrySchema>;
const nothing = z.union([z.undefined(), z.null()]);
/** Kinds registered in src-tauri/src/permissions.rs `SPECS`. */
export const permissionKinds = [
  "microphone",
  "accessibility",
  "system_audio",
] as const;
export type PermissionKind = (typeof permissionKinds)[number];
/** A macOS permission and its helper copy; other platforms report granted. */
export const permissionStatusSchema = z.object({
  kind: z.enum(permissionKinds),
  required: z.boolean(),
  granted: z.boolean(),
  assistantOpen: z.boolean(),
  title: z.string(),
  body: z.string(),
  staleHint: z.string(),
  canReset: z.boolean(),
  /** The helper offers the app icon to drag into the list. */
  draggable: z.boolean().default(true),
});
export type PermissionStatus = z.infer<typeof permissionStatusSchema>;

export const commandSchemas = {
  get_pending_crash: pendingCrashSchema.nullable(),
  prepare_report: reportSchema,
  send_report: reportReceiptSchema,
  dismiss_crash: nothing,
  copy_report: nothing,
  open_report_issues: nothing,
  check_for_updates: snapshotSchema,
  download_update: snapshotSchema,
  restart_to_update: snapshotSchema,
  dismiss_update_notice: snapshotSchema,
  get_whats_new: whatsNewStateSchema,
  dismiss_whats_new: whatsNewStateSchema,
  get_update_changelog: whatsNewSchema.nullable(),
  test_microphone: snapshotSchema,
  get_hardware_profile: hardwareProfileSchema,
  apply_setup_profile: snapshotSchema,
  begin_onboarding: z.boolean(),
  confirm_onboarding_profile: snapshotSchema,
  finish_onboarding: snapshotSchema,
  onboarding_microphone: z.object({
    level: z.number().finite().min(0).max(1),
    active: z.boolean(),
  }),
  open_permission_settings: nothing,
  onboarding_paste_permission: z.string().nullable(),
  permission_status: permissionStatusSchema,
  permission_assistant: permissionStatusSchema,
  permission_drag: nothing,
  permission_reset: permissionStatusSchema,
  get_model_library: modelLibrarySchema,
  open_model_license: nothing,
  open_legal_link: nothing,
  open_openrouter_privacy: nothing,
  get_download_requirements: z.array(downloadConsentSchema),
  acknowledge_download_terms: snapshotSchema,
  open_download_license: nothing,
  start_model_install: modelLibrarySchema,
  queue_onboarding_download: modelLibrarySchema,
  cancel_model_install: modelLibrarySchema,
  remove_model: modelLibrarySchema,
  delete_dictation: snapshotSchema,
  delete_all_dictations: z.object({
    deletedCount: z.number().int().nonnegative(),
    snapshot: snapshotSchema,
  }),
  count_dictations: z.number().int().nonnegative(),
  trust_recent_apps: z.array(z.string()),
  get_dictation_audio_usage: z.object({
    count: z.number().int().nonnegative(),
    bytes: z.number().finite().nonnegative(),
  }),
  delete_dictation_audio: z.object({
    deleted: z.number().int().nonnegative(),
  }),
  save_dictation_correction: z.object({
    record: dictationRecordSchema,
    learnedCount: z.number().int().nonnegative(),
  }),
  get_learning: z.array(learnedEntrySchema),
  save_learning: z.array(learnedEntrySchema),
  reset_learning: nothing,
  get_model_acceleration: accelerationSchema,
  get_ai_settings: aiSettingsSchema,
  save_ai_settings: aiSettingsSchema,
  connect_openrouter: aiSettingsSchema,
  connect_openrouter_key: aiSettingsSchema,
  cancel_ai_connect: nothing,
  disconnect_openrouter: aiSettingsSchema,
  list_ai_models: z.array(aiModelSchema),
  list_ai_catalog: z.array(aiModelSchema),
  check_ai_model: aiModelCheckSchema,
  suggest_vocabulary: z.array(vocabularySuggestionSchema),
  cancel_vocabulary_suggestions: nothing,
  get_setup: setupStatusSchema,
  complete_setup: nothing,
  locate_models: setupStatusSchema,
  locate_converter: setupStatusSchema,
  install_media_support: setupStatusSchema,
  install_asr_model: setupStatusSchema,
  uninstall_asr_model: setupStatusSchema,
  install_text_model: setupStatusSchema,
  uninstall_text_model: setupStatusSchema,
  get_snapshot: snapshotSchema,
  save_settings: snapshotSchema,
  start_dictation: snapshotSchema,
  stop_dictation: snapshotSchema,
  cancel_dictation: snapshotSchema,
  stop_cleanup: snapshotSchema,
  set_screen_tab: snapshotSchema,
  drag_screen_tab: nothing,
  screen_tab_record: snapshotSchema,
  copy_last_dictation: nothing,
  copy_smart_original: nothing,
  copy_history_pre_model: nothing,
  screen_tab_meeting: meetingDetailSchema,
  system_audio_status: systemAudioStatusSchema,
  show_page: nothing,
  copy_text: nothing,
  list_meetings: z.array(meetingSummarySchema),
  list_meeting_trash: z.array(z.object({ key: z.string(), id: z.string(), title: z.string(), deletedAt: z.number() })),
  restore_meeting_trash: z.string(),
  delete_meeting_trash: nothing,
  search_meetings: z.array(z.object({ id: z.string(), line: z.string() })),
  get_meeting: meetingDetailSchema,
  get_timed_transcript: z.object({
    rawTranscript: z.string().nullable().default(null),
  }),
  meeting_speakers: z.array(
    z.object({ label: z.string(), passages: z.number().int().nonnegative() }),
  ),
  rename_meeting_speaker: meetingDetailSchema,
  reassign_meeting_passage: meetingDetailSchema,
  join_meetings: meetingDetailSchema,
  start_meeting: meetingDetailSchema,
  stop_meeting: meetingDetailSchema,
  discard_meeting: nothing,
  import_media: meetingDetailSchema,
  transcribe_meeting: meetingDetailSchema,
  cancel_meeting: nothing,
  save_meeting: meetingDetailSchema,
  set_meeting_speaker_labels: meetingDetailSchema,
  clean_meeting: meetingDetailSchema,
  delete_meeting: nothing,
  generate_notes: meetingDetailSchema,
  export_meeting: nothing,
  export_meeting_audio: nothing,
  get_vocabulary: vocabularySchema,
  save_vocabulary: vocabularySchema,
  open_path: nothing,
  open_punctuation_model_page: nothing,
  get_diagnostics: z.string(),
  get_cli_path: z.string(),
  copy_cli_path: nothing,
  list_dictations: z.array(dictationRecordSchema),
  copy_history: nothing,
  export_dictation: nothing,
  copy_shortcut_command: nothing,
  learn_correction: vocabularySchema,
};
export interface CommandArguments {
  get_pending_crash: undefined;
  prepare_report: { kind: ReportKind; includeDiagnostics: boolean };
  send_report: { report: Report };
  dismiss_crash: { id: string };
  copy_report: { report: Report };
  open_report_issues: undefined;
  check_for_updates: undefined;
  download_update: undefined;
  restart_to_update: undefined;
  dismiss_update_notice: undefined;
  get_whats_new: undefined;
  dismiss_whats_new: undefined;
  get_update_changelog: undefined;
  test_microphone: undefined;
  get_hardware_profile: undefined;
  apply_setup_profile: { profile: SetupProfile };
  begin_onboarding: undefined;
  /** `skip`: profile downloads unchecked on setup's Downloads step. */
  confirm_onboarding_profile: { profile: SetupProfile; skip?: string[] };
  finish_onboarding: undefined;
  onboarding_microphone: { action: "start" | "level" | "stop" };
  open_permission_settings: { kind: "microphone" | "accessibility" };
  onboarding_paste_permission: undefined;
  permission_status: { kind: PermissionKind };
  permission_assistant: { kind: PermissionKind; action: "open" | "close" };
  permission_drag: undefined;
  permission_reset: { kind: PermissionKind };
  get_model_library: undefined;
  open_model_license: { id: string };
  open_legal_link: { url: string };
  open_openrouter_privacy: undefined;
  get_download_requirements: { ids: string[] };
  acknowledge_download_terms: {
    acknowledgments: { id: string; version: string }[];
  };
  open_download_license: { group: string; index: number };
  start_model_install: { id: string };
  queue_onboarding_download: { id: string };
  cancel_model_install: { id: string };
  remove_model: { id: string; confirmed: boolean };
  delete_dictation: { id: string; confirmed: boolean };
  delete_all_dictations: { confirmed: boolean };
  count_dictations: undefined;
  trust_recent_apps: undefined;
  get_dictation_audio_usage: undefined;
  delete_dictation_audio: undefined;
  save_dictation_correction: { id: string; text: string; scope: string | null };
  get_learning: undefined;
  save_learning: { entries: LearnedEntry[] };
  reset_learning: { confirmed: boolean };
  get_ai_settings: undefined;
  save_ai_settings: {
    settings: {
      backend: "local" | "openrouter";
      model: string;
      speechModel: string;
      quickModels: { id: string; role: "text" | "speech" }[];
    };
  };
  connect_openrouter: undefined;
  connect_openrouter_key: { key: string };
  cancel_ai_connect: undefined;
  disconnect_openrouter: undefined;
  list_ai_models: undefined;
  list_ai_catalog: undefined;
  check_ai_model: { model: string };
  suggest_vocabulary: undefined;
  cancel_vocabulary_suggestions: undefined;
  get_setup: undefined;
  get_model_acceleration: undefined;
  complete_setup: undefined;
  locate_models: { path: string };
  locate_converter: { path: string };
  install_media_support: undefined;
  install_asr_model: { id: string };
  uninstall_asr_model: { id: string };
  install_text_model: { id: string };
  uninstall_text_model: { id: string };
  get_snapshot: undefined;
  save_settings: { settings: Settings };
  start_dictation: undefined;
  stop_dictation: undefined;
  cancel_dictation: undefined;
  stop_cleanup: undefined;
  set_screen_tab: {
    visible: boolean;
    expanded?: boolean;
    focus?: boolean;
    pinned?: boolean;
  };
  screen_tab_record: undefined;
  drag_screen_tab: undefined;
  copy_last_dictation: undefined;
  copy_smart_original: { resultSessionId: number };
  copy_history_pre_model: { id: string };
  screen_tab_meeting: undefined;
  system_audio_status: undefined;
  show_page: {
    page:
      | "dictation"
      | "meetings"
      | "setup"
      | "appearance"
      | "microphone"
      | "shortcuts";
  };
  copy_text: { original: boolean; resultSessionId: number | null };
  list_meetings: undefined;
  list_meeting_trash: undefined;
  restore_meeting_trash: { key: string };
  delete_meeting_trash: { key: string };
  search_meetings: { query: string };
  get_meeting: { id: string };
  start_meeting: { title: string; source: "microphone" | "system" | "both" };
  stop_meeting: undefined;
  discard_meeting: { id: string; confirmed: boolean };
  import_media: { path: string; speaker?: string };
  get_timed_transcript: { id: string; latestAttempt?: boolean };
  meeting_speakers: { id: string };
  rename_meeting_speaker: { id: string; from: string; to: string };
  reassign_meeting_passage: {
    id: string;
    clock: string;
    from: string;
    to: string;
  };
  join_meetings: { ids: string[]; title?: string };
  transcribe_meeting: { id: string; model?: string };
  clean_meeting: { id: string };
  cancel_meeting: { id: string };
  save_meeting: {
    id: string;
    title: string;
    transcript: string;
    notes: string;
    cleanedTranscript?: string;
  };
  set_meeting_speaker_labels: {
    id: string;
    enabled: boolean;
    confirmed: boolean;
  };
  delete_meeting: { id: string };
  generate_notes: { id: string; preset: NotesPreset; instructions: string };
  export_meeting: {
    id: string;
    path: string;
    format: "markdown" | "text" | "html" | "pdf";
    transcriptVersion?: "cleaned" | "recognized";
  };
  export_meeting_audio: { id: string; path: string };
  get_vocabulary: undefined;
  save_vocabulary: { entries: VocabularyEntry[] };
  open_path: { kind: "data" | "models" | "logs" };
  open_punctuation_model_page: { id?: string } | undefined;
  get_diagnostics: undefined;
  get_cli_path: undefined;
  copy_cli_path: undefined;
  list_dictations: { limit: number };
  copy_history: { id: string; original: boolean };
  export_dictation: { id: string; path: string };
  copy_shortcut_command: undefined;
  learn_correction: { id: string; heard: string; written: string };
}
export type CommandName = keyof CommandArguments;
export type CommandResult<C extends CommandName> = z.output<
  (typeof commandSchemas)[C]
>;

/** The only native boundary. Unknown responses must pass their command's schema. */
export function parseResponse<C extends CommandName>(
  command: C,
  value: unknown,
): CommandResult<C> {
  const result = commandSchemas[command].safeParse(value);
  if (!result.success)
    throw new Error(
      `The desktop service returned an invalid response for ${command}. Reopen SonoJot and try again.`,
    );
  // The indexed schema and result share C; TypeScript otherwise widens their correlation.
  return result.data as CommandResult<C>;
}

export const previewSnapshot: Snapshot = {
  localProofreaderAvailable: false,
  recognitionProgress: null,
  revision: 0,
  meetingsRevision: 0,
  sessionId: null,
  tabVisible: true,
  tabExpanded: false,
  tabTemporary: false,
  canCopyLast: false,
  deliveryNotice: null,
  resultSessionId: null,
  recognitionCompleteness: null,
  recognitionWarnings: [],
  settings: {
    updateChannel: "stable",
    termsAcceptedVersion: 0,
    termsAcceptedAt: null,
    meetingConsentAcknowledged: false,
    componentAcknowledgments: {},
    setupProfile: "custom",
    punctuationRecommendationAcknowledged: false,
    punctuationV2RecommendationAcknowledged: false,
    punctuationV3RecommendationAcknowledged: false,
    theme: "fog",
    material: true,
    cleanupMode: "clean",
    cleanupTrial: false,
    smartLayout: false,
    smartLayoutAppHint: false,
    proofreader: false,
    localProofreader: true,
    proofreaderModel: "anthropic/claude-opus-5.5",
    proofreaderTrainingModels: [],
    writtenForm: true,
    sonojotPunctuation: true,
    launchAtLogin: false,
    hotkey: "Ctrl+Space",
    hotkeyMode: "toggle",
    autoPaste: true,
    audioCues: true,
    livePreview: true,
    autoStopSeconds: 0,
    microphone: null,
    asrModel: "",
    meetingAsrModel: "",
    cleanupModel: "",
    notesModel: "qwen3-4b-instruct-2507",
    textAcceleration: "auto",
    doNotSaveDictations: false,
    keepDictationAudio: false,
    learnFromCorrections: false,
    memoryMode: "overlap",
    modelRetention: "10m",
    trust: { ...trustDefaults },
    startTabHidden: false,
    cursorIndicator: true,
    meetingSource: "both",
    meetingSourceMigrated: true,
    systemAudioDefaultApplied: true,
    autoMeetingTitles: true,
    meetingNotesPreset: null,
    meetingNotesInstructions: "",
    meetingSpeakerLabels: true,
    meetingRemoveEcho: null,
  },
  state: "idle",
  originalText: "",
  outputText: "",
  preModelText: null,
  publicationStage: null,
  cleanupStatus: null,
  guardReason: null,
  interimText: "",
  previewText: "",
  audioLevel: 0,
  meetingLevels: null,
  meetingSourceStatus: [],
  meetingSystemAudio: null,
  error: null,
  models: [],
  devices: [],
  meetingRecording: false,
  activeMeetingId: null,
  capabilities: {
    sync: false,
    autoPaste: false,
    systemAudio: false,
    systemAudioStatus: "unsupported",
    globalShortcut: false,
    platform: "Browser preview",
  },
  version: "0.22.0",
  update: {
    currentVersion: "0.22.0",
    status: { state: "idle" },
    restartBlockedReason: null,
    installing: false,
    noticeDismissed: false,
    installProblem: null,
  },
};

export function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : "The action could not be completed. Please try again.";
}
export function duration(seconds: number): string {
  const total = Math.floor(Math.max(0, seconds));
  const hours = Math.floor(total / 3600);
  return hours > 0
    ? `${hours}:${String(Math.floor(total / 60) % 60).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`
    : `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
export function safeFilename(title: string): string {
  return (
    title
      .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "-")
      .replace(/[. ]+$/g, "")
      .slice(0, 100) || "Meeting"
  );
}
