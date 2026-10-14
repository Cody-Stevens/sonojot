import { z } from "zod";
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

export const settingsSchema = z.object({
  termsAcceptedVersion: z.number().int().nonnegative().default(0),
  termsAcceptedAt: z.string().nullable().default(null),
  meetingConsentAcknowledged: z.boolean().default(false),
  componentAcknowledgments: z.record(z.string(), z.string()).default({}),
  setupProfile: setupProfileSchema.nullable().default("custom"),
  punctuationRecommendationAcknowledged: z.boolean().default(false),
  punctuationV2RecommendationAcknowledged: z.boolean().default(false),
  theme: z.enum(["fog", "slate"]),
  material: z.boolean(),
  cleanupMode: z.enum(["clean", "original", "format"]),
  cleanupTrial: z.boolean().default(false),
  smartLayout: z.boolean().default(false),
  writtenForm: z.boolean().default(true),
  launchAtLogin: z.boolean(),
  hotkey: z.string(),
  hotkeyMode: z.enum(["toggle", "hold"]),
  autoPaste: z.boolean(),
  audioCues: z.boolean(),
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
  meetingSpeakerLabels: z.boolean().default(true),
  meetingSource: z.enum(["microphone", "system", "both"]).default("both"),
  meetingSourceMigrated: z.boolean().default(false),
  systemAudioDefaultApplied: z.boolean().default(false),
  doNotSaveDictations: z.boolean().default(false),
  learnFromCorrections: z.boolean().default(false),
  memoryMode: memoryModeSchema.default("overlap"),
  modelRetention: modelRetentionSchema.default("10m"),
});
export type Settings = z.infer<typeof settingsSchema>;
export const systemAudioStatusSchema = z.enum([
  "ready",
  "notDetermined",
  "denied",
  "unsupported",
]);
export type SystemAudioStatus = z.infer<typeof systemAudioStatusSchema>;

export const notesPresetSchema = z.enum([
  "summary",
  "tasks",
  "speakers",
  "brief",
  "formattedTranscript",
  "custom",
]);
export type NotesPreset = z.infer<typeof notesPresetSchema>;

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
    state: z.literal("available"),
    version: z.string().min(1),
    notes: z.string().nullable(),
    size: z.number().int().nonnegative().nullable(),
  }),
  z.object({
    state: z.literal("downloading"),
    percent: z.number().finite().min(0).max(100).nullable(),
  }),
  z.object({
    state: z.literal("ready"),
    version: z.string().min(1),
    notes: z.string().nullable(),
  }),
  z.object({ state: z.literal("error"), message: z.string() }),
]);
export type UpdateStatus = z.infer<typeof updateStatusSchema>;
export const updateSchema = z.object({
  currentVersion: z.string(),
  status: updateStatusSchema,
  restartBlockedReason: z.string().nullable(),
  installing: z.boolean(),
});

export const snapshotSchema = z.object({
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
  guardReason: z.string().nullable().default(null),
  interimText: z.string(),
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
});
export const meetingDetailSchema = meetingSummarySchema.extend({
  recognitionCompleteness: recognitionCompletenessSchema.default("complete"),
  recognitionWarnings: z.array(z.string()).default([]),
  recordingWarnings: z.array(z.string()).default([]),
  audioOutages: z.array(z.object({
    source: z.string(),
    startSeconds: z.number().nonnegative(),
    endSeconds: z.number().nonnegative(),
    reason: z.string(),
  })).default([]),
  missingSeconds: z.number().nonnegative().default(0),
  cleanedTranscript: z.string().nullable().default(null),
  transcriptionStoppedAtSeconds: z.number().nonnegative().nullable().default(null),
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
    id: z.string(),
    heard: z.string(),
    written: z.string(),
    enabled: z.boolean(),
    scope: z.string().nullable().default(null),
  }),
);
export type VocabularyEntry = z.infer<typeof vocabularySchema>[number];
export const dictationRecordSchema = z.object({
  id: z.string(),
  createdAt: z.string(),
  raw: z.string(),
  published: z.string(),
  preModelText: z.string().nullable().default(null),
  publicationStage: z.string().nullable().default(null),
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
export const permissionKinds = ["accessibility", "system_audio"] as const;
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
});
export type PermissionStatus = z.infer<typeof permissionStatusSchema>;

export const commandSchemas = {
  check_for_updates: snapshotSchema,
  download_update: snapshotSchema,
  restart_to_update: snapshotSchema,
  test_microphone: snapshotSchema,
  get_hardware_profile: hardwareProfileSchema,
  apply_setup_profile: snapshotSchema,
  begin_onboarding: z.boolean(),
  confirm_onboarding_profile: snapshotSchema,
  finish_onboarding: snapshotSchema,
  onboarding_microphone: z.object({ level: z.number().finite().min(0).max(1), active: z.boolean() }),
  open_permission_settings: nothing,
  onboarding_paste_permission: z.string().nullable(),
  permission_status: permissionStatusSchema,
  permission_assistant: permissionStatusSchema,
  permission_drag: nothing,
  get_model_library: modelLibrarySchema,
  open_model_license: nothing,
  open_legal_link: nothing,
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
  get_meeting: meetingDetailSchema,
  start_meeting: meetingDetailSchema,
  stop_meeting: meetingDetailSchema,
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
  list_dictations: z.array(dictationRecordSchema),
  copy_history: nothing,
  learn_correction: vocabularySchema,
};
export interface CommandArguments {
  check_for_updates: undefined;
  download_update: undefined;
  restart_to_update: undefined;
  test_microphone: undefined;
  get_hardware_profile: undefined;
  apply_setup_profile: { profile: SetupProfile };
  begin_onboarding: undefined;
  confirm_onboarding_profile: { profile: SetupProfile };
  finish_onboarding: undefined;
  onboarding_microphone: { action: "start" | "level" | "stop" };
  open_permission_settings: { kind: "microphone" | "accessibility" };
  onboarding_paste_permission: undefined;
  permission_status: { kind: PermissionKind };
  permission_assistant: { kind: PermissionKind; action: "open" | "close" };
  permission_drag: undefined;
  get_model_library: undefined;
  open_model_license: { id: string };
  open_legal_link: { url: string };
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
  get_meeting: { id: string };
  start_meeting: { title: string; source: "microphone" | "system" | "both" };
  stop_meeting: undefined;
  import_media: { path: string };
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
  list_dictations: { limit: number };
  copy_history: { id: string; original: boolean };
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
    termsAcceptedVersion: 0,
    termsAcceptedAt: null,
    meetingConsentAcknowledged: false,
    componentAcknowledgments: {},
    setupProfile: "custom",
    punctuationRecommendationAcknowledged: false,
    punctuationV2RecommendationAcknowledged: false,
    theme: "fog",
    material: true,
    cleanupMode: "clean",
    cleanupTrial: false,
    smartLayout: false,
    writtenForm: true,
    launchAtLogin: false,
    hotkey: "Ctrl+Space",
    hotkeyMode: "toggle",
    autoPaste: true,
    audioCues: true,
    autoStopSeconds: 0,
    microphone: null,
    asrModel: "",
    meetingAsrModel: "",
    cleanupModel: "",
    notesModel: "qwen3-4b-instruct-2507",
    textAcceleration: "auto",
    doNotSaveDictations: false,
    learnFromCorrections: false,
    memoryMode: "overlap",
    modelRetention: "10m",
    startTabHidden: false,
    cursorIndicator: true,
    meetingSource: "both",
    meetingSourceMigrated: true,
    systemAudioDefaultApplied: true,
    autoMeetingTitles: true,
    meetingSpeakerLabels: true,
  },
  state: "idle",
  originalText: "",
  outputText: "",
  preModelText: null,
  publicationStage: null,
  guardReason: null,
  interimText: "",
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
