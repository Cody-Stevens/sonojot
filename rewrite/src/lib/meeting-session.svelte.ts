import { onMount, onDestroy, untrack } from "svelte";
import { presentError, type ErrorAction } from "./errors";
import { app, STATUS_NOTICE_MS } from "./app.svelte";
import {
  meetingSourceWarning,
  prepareMeetingStart,
  recordingSystemNotice,
  systemAudioFixable,
  systemAudioState,
} from "./meeting-source";
import { fixSystemAudio, watchSystemAudio } from "./system-audio.svelte";
import { call, chooseExport, chooseMedia } from "./api";
import { ensureDownloadConsent } from "./download-consent.svelte";
import { activePunctuation } from "./punctuation";
import type { AiSettings, AiModel } from "./ai";
import {
  checking,
  formatBytes,
  installed,
  installing,
  modelUnavailableReason,
  type ModelsTarget,
} from "./model-library";
import { library } from "./model-library.svelte";
import { defaultMeetingModel, meetingSpeechModels } from "./meeting-models";
import {
  hasStructure,
  parseTranscript,
  serializeTranscript,
  type TranscriptBlock,
} from "./meeting-transcript";
import {
  meetingClock,
  meetingNotice,
  outageLabel,
  transcriptEntries,
  visibleRecognitionWarnings,
} from "./meeting-notices";
import {
  duration,
  errorMessage,
  safeFilename,
  type MeetingDetail,
  type MeetingSummary,
  type NotesPreset,
} from "./protocol";

export function createMeetingSession() {
  let meetings = $state<MeetingSummary[]>([]);
  let selected = $state<MeetingDetail | null>(null);
  // A saved transcript without a title is a status, not a failure: like other
  // notices it clears after a few seconds or on Dismiss. Other errors stay.
  let hiddenStatus = $state("");
  const statusKey = $derived(
    selected?.error && presentError(selected.error).family === "meeting-title"
      ? `${selected.id}\n${selected.error}`
      : "",
  );
  $effect(() => {
    const key = statusKey;
    if (!key) return;
    const timer = setTimeout(() => (hiddenStatus = key), STATUS_NOTICE_MS);
    return () => clearTimeout(timer);
  });
  let loaded = $state(!app.native);
  let search = $state("");
  let newTitle = $state("");
  let source = $state<"microphone" | "system" | "both">(
    app.snapshot.settings.meetingSource,
  );
  $effect(() => {
    source = app.snapshot.settings.meetingSource;
  });
  const sourceWarning = $derived(meetingSourceWarning(source));
  const systemStatus = $derived(app.snapshot.capabilities.systemAudioStatus);
  const systemState = $derived(
    systemAudioState(systemStatus, app.snapshot.capabilities.platform),
  );
  const systemNotice = $derived(
    recordingSystemNotice(app.snapshot.meetingSystemAudio),
  );
  let systemFixError = $state("");
  async function fixSystem(): Promise<void> {
    systemFixError = "";
    try {
      await fixSystemAudio();
    } catch (failure: unknown) {
      systemFixError =
        failure instanceof Error ? failure.message : String(failure);
    }
  }
  let startRequested = $state(false);
  let title = $state("");
  let transcript = $state("");
  let recognizedDraft = $state("");
  let cleanedDraft = $state<string | null>(null);
  let showCleaned = $state(false);
  let transcriptConflict = $state(false);
  let renameOpen = $state(false);
  let notesOpen = $state(false);
  let exportOpen = $state(false);
  let defaultsOpen = $state(false);
  let meetingError = $state("");
  let notes = $state("");
  let rawTranscript = $state<string | null>(null);
  let rawError = $state("");
  let rawLoading = $state(false);
  let showRaw = $state(false);
  let rawRequest = 0;
  let transcriptBlocks = $state<TranscriptBlock[]>([]);
  let transcribeSetup = $state(false);
  let transcriptionOverride = $state<string | null>(null);
  let aiSettings = $state<AiSettings | null>(null);
  const transcriptionModel = $derived(
    transcriptionOverride ??
      defaultMeetingModel(app.snapshot.settings, aiSettings),
  );
  let aiCatalog = $state<AiModel[]>([]);
  let aiSettingsRequest = 0;
  let aiCatalogRequested = false;
  let instructions = $state(app.snapshot.settings.meetingNotesInstructions);
  let meetingPage = $state<"view" | "start">("view");
  const notePresets = [
    { id: "summary", label: "Summary" },
    { id: "tasks", label: "Tasks by person" },
    { id: "speakers", label: "Structure by speaker" },
    { id: "brief", label: "Executive brief" },
    { id: "formattedTranscript", label: "Corrected transcript (AI)" },
    { id: "custom", label: "Custom instructions…" },
  ] satisfies { id: NotesPreset; label: string }[];
  let notePreset = $state<NotesPreset>(
    app.snapshot.settings.meetingNotesPreset ?? "summary",
  );
  const detailTabs = [
    { id: "notes", label: "Notes" },
    { id: "transcript", label: "Transcript" },
  ] as const;
  type DetailTab = (typeof detailTabs)[number]["id"];
  let tab = $state<DetailTab>("transcript");
  let exportFormat = $state<"markdown" | "text" | "html" | "pdf">("markdown");
  let refreshRunning = false;
  let refreshPending = false;
  let destroyed = false;
  let selectionRequest = 0;
  let now = $state(Date.now());
  const dirty = $derived(
    selected !== null &&
      (title !== selected.title ||
        (showCleaned ? recognizedDraft : transcript) !== selected.transcript ||
        (showCleaned ? transcript : cleanedDraft) !==
          selected.cleanedTranscript ||
        notes !== selected.notes),
  );
  let searchMatches = $state<{ id: string; line: string }[]>([]);
  let searching = $state(false);
  let searchError = $state("");
  let searchRequest = 0;
  $effect(() => {
    const query = search.trim();
    const revision = app.meetingRevision;
    const request = ++searchRequest;
    searchMatches = [];
    searchError = "";
    searching = !!query && app.native;
    if (!query || !app.native) return;
    const timer = setTimeout(() => {
      void call("search_meetings", { query })
        .then((matches) => {
          if (!destroyed && request === searchRequest) searchMatches = matches;
        })
        .catch((error: unknown) => {
          if (!destroyed && request === searchRequest)
            searchError = errorMessage(error);
        })
        .finally(() => {
          if (!destroyed && request === searchRequest) searching = false;
        });
    }, 180);
    return () => {
      clearTimeout(timer);
      searchRequest++;
    };
  });
  const filtered = $derived(
    search.trim()
      ? meetings.filter((item) =>
          searchMatches.some((match) => match.id === item.id),
        )
      : meetings,
  );
  const recording = $derived(
    selected !== null &&
      (selected.status === "recording" ||
        (app.snapshot.meetingRecording &&
          app.snapshot.activeMeetingId === selected.id)),
  );
  const processing = $derived(
    selected !== null &&
      (app.busy("meeting-transcribe") ||
        app.busy("meeting-clean") ||
        app.busy("meeting-notes") ||
        [
          "finalizing",
          "transcribing",
          "processing",
          "processingNotes",
          "generatingTitle",
          "importing",
          "generating_notes",
          "generating-notes",
        ].includes(selected.status)),
  );
  const writing = $derived(
    app.busy("meeting-save") || app.busy("meeting-delete"),
  );
  const dictating = $derived(
    ["listening", "loading", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const structured = $derived(
    hasStructure(transcriptBlocks) || !!selected?.audioOutages.length,
  );
  const entries = $derived(
    transcriptEntries(transcriptBlocks, selected?.audioOutages ?? []),
  );
  const audioNotice = $derived(selected ? meetingNotice(selected) : "");
  const recognitionNotices = $derived(
    visibleRecognitionWarnings(selected?.recognitionWarnings ?? []),
  );
  const processingBlocked = $derived(
    !app.native ||
      !selected?.hasTranscript ||
      dirty ||
      recording ||
      dictating ||
      processing ||
      writing ||
      app.busy("meeting-speakers"),
  );
  const cleanupPackage = $derived(
    activePunctuation(library.data?.models ?? []),
  );
  const speechModels = $derived(
    meetingSpeechModels(library.data?.models ?? []),
  );
  const transcriptionEntry = $derived(
    library.data?.models.find((model) => model.id === transcriptionModel),
  );
  const transcriptionUnavailableReason = $derived(
    transcriptionEntry ? modelUnavailableReason(transcriptionEntry) : null,
  );
  const speakerModel = $derived(
    library.data?.models.find((model) => model.id === "nemotron-3-diarization"),
  );
  const speakerLabelsAvailable = $derived(
    speakerModel &&
      (installed(speakerModel) ||
        checking(speakerModel) ||
        installing(speakerModel) ||
        speakerModel.installable),
  );
  const speakerInstallPending = $derived(
    app.busy("model-install") || !!(speakerModel && installing(speakerModel)),
  );
  const speakerInstallDisabled = $derived(
    !app.native ||
      !library.data ||
      speakerInstallPending ||
      !!(speakerModel && checking(speakerModel)) ||
      app.busy("settings") ||
      dictating ||
      app.snapshot.meetingRecording ||
      library.data.models.some(
        (model) =>
          installing(model) ||
          ["loading", "busy", "unloading"].includes(model.state),
      ),
  );
  async function installSpeakerLabels(): Promise<void> {
    if (
      speakerInstallDisabled ||
      !speakerModel?.installable ||
      installed(speakerModel)
    )
      return;
    const id = speakerModel.id;
    await app.perform("model-install", async () => {
      if (!(await ensureDownloadConsent([id]))) return;
      const data = await call("start_model_install", { id });
      library.downloadId = id;
      library.apply(data);
    });
  }
  const remoteSpeechModel = $derived(
    aiSettings?.connected ? aiSettings.speechModel : "",
  );
  const remoteSpeechName = $derived(
    aiCatalog.find((model) => model.id === remoteSpeechModel)?.name ??
      remoteSpeechModel,
  );
  const transcriptionModelAvailable = $derived(
    transcriptionModel === "openrouter"
      ? !!remoteSpeechModel
      : speechModels.some((model) => model.id === transcriptionModel),
  );
  const transcribeBlocked = $derived(
    !app.native ||
      !selected?.hasAudio ||
      dirty ||
      recording ||
      dictating ||
      processing ||
      writing,
  );
  function modelName(id: string): string {
    if (id.startsWith("openrouter:")) {
      const remoteId = id.slice("openrouter:".length);
      return `OpenRouter · ${aiCatalog.find((model) => model.id === remoteId)?.name ?? remoteId}`;
    }
    return (
      library.data?.models.find((model) => model.id === id)?.name ??
      app.snapshot.models.find((model) => model.id === id)?.name ??
      id
    );
  }
  function openTranscribeSetup(): void {
    transcribeSetup = true;
    void loadAiSettings();
  }
  async function loadAiSettings(): Promise<void> {
    if (!app.native) return;
    const request = ++aiSettingsRequest;
    try {
      const next = await call("get_ai_settings");
      if (destroyed || request !== aiSettingsRequest) return;
      aiSettings = next;
      if (
        next.connected &&
        (next.speechModel || next.model) &&
        !aiCatalogRequested
      ) {
        aiCatalogRequested = true;
        try {
          const catalog = await call("list_ai_catalog");
          if (!destroyed) aiCatalog = catalog;
        } catch {
          aiCatalogRequested = false;
        }
      }
    } catch {
      if (!destroyed && request === aiSettingsRequest) aiSettings = null;
    }
  }
  const elapsed = $derived(
    selected && recording && Number.isFinite(Date.parse(selected.createdAt))
      ? Math.max(
          selected.durationSeconds,
          (now - Date.parse(selected.createdAt)) / 1000,
        )
      : (selected?.durationSeconds ?? 0),
  );
  $effect(() => {
    app.dirty = dirty;
  });
  $effect(() => {
    const revision = app.meetingRevision;
    if (app.native && loaded && revision > 0)
      untrack(() => {
        void refresh();
      });
  });
  onDestroy(() => {
    destroyed = true;
    app.dirty = false;
    selectionRequest += 1;
  });
  onMount(() => {
    const releaseLibrary = library.acquire();
    if (app.native) void refresh(true);
    void loadAiSettings();
    const stopWatching = watchSystemAudio();
    const interval = window.setInterval(() => {
      if (app.snapshot.meetingRecording) now = Date.now();
    }, 1000);
    const recovery = window.setInterval(() => {
      if (
        processing ||
        meetings.some((item) =>
          [
            "finalizing",
            "transcribing",
            "processingNotes",
            "generatingTitle",
            "importing",
          ].includes(item.status),
        )
      )
        void refresh();
    }, 3000);
    const onFocus = () => {
      void refresh();
      void loadAiSettings();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      releaseLibrary();
      stopWatching();
      window.clearInterval(interval);
      window.clearInterval(recovery);
      window.removeEventListener("focus", onFocus);
    };
  });
  function date(value: string): string {
    const parsed = new Date(value);
    return Number.isNaN(parsed.getTime())
      ? value
      : new Intl.DateTimeFormat(undefined, {
          month: "short",
          day: "numeric",
          year: "numeric",
        }).format(parsed);
  }
  function tabKeydown(event: KeyboardEvent): void {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const index = detailTabs.findIndex((item) => item.id === tab);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? detailTabs.length - 1
          : (index + (event.key === "ArrowRight" ? 1 : detailTabs.length - 1)) %
            detailTabs.length;
    const target = detailTabs[next].id;
    tab = target;
    document.getElementById(`meeting-tab-${target}`)?.focus();
  }
  function accept(detail: MeetingDetail, preserveDraft = false): void {
    const keepDraft = preserveDraft && selected?.id === detail.id && dirty;
    const keepTitle = keepDraft && title !== selected?.title;
    if (
      keepDraft &&
      selected &&
      (selected.transcript !== detail.transcript ||
        selected.cleanedTranscript !== detail.cleanedTranscript)
    )
      transcriptConflict = true;
    if (!keepDraft) transcriptConflict = false;
    if (
      !keepDraft &&
      selected?.id === detail.id &&
      !selected.notes &&
      detail.notes
    )
      tab = "notes";
    if (selected?.id !== detail.id) {
      transcriptionOverride = null;
      showRaw = false;
      rawTranscript = null;
      rawError = "";
      rawRequest += 1;
      transcribeSetup = false;
      renameOpen = false;
      notesOpen = false;
      exportOpen = false;
      meetingError = "";
      showCleaned = detail.cleanedTranscript !== null;
      tab = detail.notes ? "notes" : "transcript";
    } else if (
      !keepDraft &&
      selected.cleanedTranscript === null &&
      detail.cleanedTranscript !== null
    )
      showCleaned = true;
    if (!keepDraft && detail.cleanedTranscript === null) showCleaned = false;
    selected = detail;
    // A background title can arrive while transcript/notes edits are unsaved.
    // Keep a title draft only when the user actually changed the title.
    if (!keepTitle) title = detail.title;
    if (!keepDraft) {
      recognizedDraft = detail.transcript;
      cleanedDraft = detail.cleanedTranscript;
      transcript = showCleaned
        ? (detail.cleanedTranscript ?? detail.transcript)
        : detail.transcript;
      transcriptBlocks = parseTranscript(transcript);
      notes = detail.notes;
    }
    const prior = meetings.find((row) => row.id === detail.id);
    meetings = [
      {
        ...detail,
        hasNotes: !!detail.notes.trim(),
        autoNamed: prior?.title === detail.title && prior.autoNamed,
      },
      ...meetings.filter((item) => item.id !== detail.id),
    ].sort((left, right) => right.createdAt.localeCompare(left.createdAt));
  }
  async function refresh(reportError = false): Promise<void> {
    if (!app.native || destroyed) return;
    if (refreshRunning) {
      refreshPending = true;
      return;
    }
    refreshRunning = true;
    try {
      const rows = await call("list_meetings");
      if (destroyed) return;
      meetings = rows;
      loaded = true;
      const selectedId = selected?.id;
      if (selectedId && !writing) {
        const detail = await call("get_meeting", { id: selectedId });
        if (!destroyed && selected?.id === selectedId) accept(detail, true);
      }
    } catch (error: unknown) {
      if (reportError) app.error = errorMessage(error);
    } finally {
      refreshRunning = false;
      if (refreshPending && !destroyed) {
        refreshPending = false;
        void refresh(reportError);
      }
    }
  }
  async function canLeave(): Promise<boolean> {
    return (
      !dirty ||
      app.confirm(
        "Leave these changes?",
        "The title, transcript or notes have changes that have not been saved.",
      )
    );
  }
  async function select(id: string): Promise<void> {
    if (id === selected?.id || !(await canLeave())) return;
    const request = ++selectionRequest;
    const detail = await app.perform("meeting-load", () =>
      call("get_meeting", { id }),
    );
    if (detail && request === selectionRequest) {
      accept(detail);
      meetingPage = "view";
      instructions = app.snapshot.settings.meetingNotesInstructions;
      notePreset = app.snapshot.settings.meetingNotesPreset ?? "summary";
      tab = detail.notes ? "notes" : "transcript";
    }
  }
  async function back(): Promise<void> {
    await showPage("view");
  }
  async function showPage(page: "view" | "start"): Promise<void> {
    if (await canLeave()) {
      selected = null;
      meetingPage = page;
      selectionRequest += 1;
    }
  }
  async function start(): Promise<void> {
    if (startRequested || app.busy("settings")) return;
    startRequested = true;
    try {
      if (!(await canLeave())) return;
      const detail = await app.perform("meeting-record", async () => {
        const plan = await prepareMeetingStart(
          app.snapshot.settings,
          app.snapshot.capabilities.systemAudioStatus,
          (prompt) => app.chooseConfirmation(prompt),
          async (patch) => {
            app.apply(
              await call("save_settings", {
                settings: { ...app.snapshot.settings, ...patch },
              }),
            );
          },
          () =>
            !destroyed &&
            !app.snapshot.meetingRecording &&
            ["idle", "error"].includes(app.snapshot.state),
        );
        if (plan === null) return;
        if (plan.action === "fix") {
          // Nothing records yet; the user starts again once it is fixed.
          await fixSystemAudio();
          return;
        }
        return call("start_meeting", {
          title: newTitle.trim(),
          source: plan.source,
        });
      });
      if (detail) {
        accept(detail);
        newTitle = "";
        await app.perform("snapshot", async () =>
          app.apply(await call("get_snapshot")),
        );
      }
    } finally {
      startRequested = false;
    }
  }
  async function stop(): Promise<void> {
    const detail = await app.perform("meeting-record", () =>
      call("stop_meeting"),
    );
    if (detail) {
      if (!selected || selected.id === detail.id) accept(detail, true);
      await refresh();
    }
  }
  async function importFile(path?: string, speaker?: string): Promise<boolean> {
    if (!(await canLeave())) return false;
    const detail = await app.perform("meeting-import", async () => {
      const chosen = path ?? (await chooseMedia());
      return chosen
        ? call("import_media", {
            path: chosen,
            ...(speaker?.trim() ? { speaker: speaker.trim() } : {}),
          })
        : undefined;
    });
    if (detail) accept(detail);
    return !!detail;
  }
  async function reloadSaved(): Promise<void> {
    const id = selected?.id;
    if (
      !id ||
      !(await app.confirm(
        "Reload saved meeting?",
        "Discard unsaved title, transcript and notes changes and load the saved meeting.",
        "Reload saved meeting",
      ))
    )
      return;
    const detail = await app.perform("meeting-load", () =>
      call("get_meeting", { id }),
    );
    if (detail && selected?.id === id) accept(detail);
  }
  async function saveChanges(): Promise<void> {
    const id = selected?.id;
    if (!id || !title.trim()) return;
    if (transcriptConflict) {
      app.error =
        "The transcript changed elsewhere. Reload the saved meeting before saving.";
      return;
    }
    const detail = await app.perform(
      "meeting-save",
      () =>
        call("save_meeting", {
          id,
          title: title.trim(),
          transcript: showCleaned ? recognizedDraft : transcript,
          cleanedTranscript:
            (showCleaned ? transcript : cleanedDraft) ?? undefined,
          notes,
        }),
      "Meeting saved.",
    );
    if (detail && selected?.id === id) {
      accept(detail);
      renameOpen = false;
    }
  }
  function switchTranscript(cleaned: boolean): void {
    showRaw = false;
    if (cleaned === showCleaned) return;
    if (showCleaned) cleanedDraft = transcript;
    else recognizedDraft = transcript;
    showCleaned = cleaned;
    transcript = cleaned ? (cleanedDraft ?? "") : recognizedDraft;
    transcriptBlocks = parseTranscript(transcript);
  }
  async function cleanTranscript(): Promise<void> {
    const id = selected?.id;
    if (!id || processingBlocked) return;
    const detail = await app.perform("meeting-clean", () =>
      call("clean_meeting", { id }),
    );
    if (detail && selected?.id === id) {
      accept(detail, true);
      tab = "transcript";
    }
  }
  async function changeSource(
    value: "microphone" | "system" | "both",
  ): Promise<void> {
    source = value;
    await app.saveSettings({ meetingSource: value });
    source = app.snapshot.settings.meetingSource;
    // Choosing system audio without permission offers the fix right away.
    if (value !== "microphone" && systemAudioFixable(systemStatus))
      await fixSystem();
  }
  function sourceName(value: string): string {
    return value === "both"
      ? "Microphone and system audio"
      : value === "system"
        ? "System audio"
        : value === "microphone"
          ? "Microphone"
          : "Imported recording";
  }
  async function transcribe(): Promise<void> {
    const id = selected?.id;
    const model = transcriptionModel;
    if (!id || !transcriptionModelAvailable || transcribeBlocked) return;
    transcribeSetup = false;
    meetingError = "";
    const detail = await app.perform("meeting-transcribe", async () => {
      try {
        return await call("transcribe_meeting", { id, model });
      } catch (error: unknown) {
        if (selected?.id === id) meetingError = errorMessage(error);
        return undefined;
      }
    });
    if (detail && selected?.id === id) accept(detail, true);
    else await refresh();
  }
  async function generateNotes(): Promise<void> {
    const id = selected?.id;
    if (!id) return;
    const detail = await app.perform("meeting-notes", () =>
      call("generate_notes", {
        id,
        preset: notePreset,
        instructions: notePreset === "formattedTranscript" ? "" : instructions,
      }),
    );
    if (detail && selected?.id === id) {
      accept(detail, true);
      tab = "notes";
      notesOpen = false;
    }
  }
  // Must match meetings::SPEAKER_LABELS_REPLACE_EDITS.
  const speakerLabelsReplaceEdits =
    "Changing speaker labels rebuilds the transcript and replaces your edits.";
  async function setSpeakerLabels(enabled: boolean): Promise<void> {
    const id = selected?.id;
    if (!id) return;
    const detail = await app.perform("meeting-speakers", async () => {
      try {
        return await call("set_meeting_speaker_labels", {
          id,
          enabled,
          confirmed: false,
        });
      } catch (failure: unknown) {
        if (errorMessage(failure) !== speakerLabelsReplaceEdits) throw failure;
        const replace = await app.confirm(
          "Replace your transcript edits?",
          "Changing speaker labels rebuilds the transcript from the recording's recognized speech. Any cleaned copy is also replaced.",
          "Replace edits",
          true,
        );
        return replace
          ? await call("set_meeting_speaker_labels", {
              id,
              enabled,
              confirmed: true,
            })
          : undefined;
      }
    });
    if (detail && selected?.id === id) accept(detail);
  }
  async function cancel(): Promise<void> {
    const id = selected?.id;
    if (!id) return;
    await app.perform("meeting-cancel", () => call("cancel_meeting", { id }));
    await refresh(true);
  }
  async function remove(): Promise<void> {
    const detail = selected;
    if (
      !detail ||
      !(await app.confirm(
        "Delete this meeting?",
        `“${detail.title}” and its files in SonoJot will be deleted.${app.snapshot.capabilities.sync ? " If this meeting is synced, it will be deleted on all synced devices." : ""}`,
        "Delete meeting",
        true,
      ))
    )
      return;
    await app.perform(
      "meeting-delete",
      async () => {
        await call("delete_meeting", { id: detail.id });
        if (selected?.id === detail.id) selected = null;
        meetings = meetings.filter((item) => item.id !== detail.id);
      },
      "Meeting deleted.",
    );
  }
  async function exportFile(): Promise<void> {
    const detail = selected;
    if (!detail) return;
    await app.perform("meeting-export", async () => {
      const format = exportFormat;
      const transcriptVersion = showCleaned ? "cleaned" : "recognized";
      const extension =
        format === "markdown" ? "md" : format === "text" ? "txt" : format;
      const path = await chooseExport(
        `${safeFilename(detail.title)}.${extension}`,
        extension,
      );
      if (path) {
        await call("export_meeting", {
          id: detail.id,
          path,
          format,
          transcriptVersion,
        });
        app.notice = "Meeting exported.";
      }
    });
  }

  async function viewRaw(): Promise<void> {
    const id = selected?.id;
    if (!id) return;
    showRaw = true;
    rawError = "";
    rawLoading = true;
    const request = ++rawRequest;
    try {
      const result = await call("get_timed_transcript", { id });
      if (!destroyed && request === rawRequest && selected?.id === id)
        rawTranscript = result.rawTranscript;
    } catch (failure) {
      if (!destroyed && request === rawRequest)
        rawError = errorMessage(failure);
    } finally {
      if (request === rawRequest) rawLoading = false;
    }
  }
  async function renameSpeaker(from: string, to: string): Promise<boolean> {
    const id = selected?.id;
    if (!id || processingBlocked || app.busy("meeting-speakers")) return false;
    const detail = await app.perform("meeting-speakers", () =>
      call("rename_meeting_speaker", { id, from, to }),
    );
    if (detail && selected?.id === id) accept(detail);
    return !!detail;
  }
  async function reassignPassage(
    clock: string,
    from: string,
    to: string,
  ): Promise<boolean> {
    const id = selected?.id;
    if (!id || processingBlocked || app.busy("meeting-speakers")) return false;
    const detail = await app.perform("meeting-speakers", () =>
      call("reassign_meeting_passage", { id, clock, from, to }),
    );
    if (detail && selected?.id === id) accept(detail);
    return !!detail;
  }
  async function join(ids: string[]): Promise<boolean> {
    if (
      dirty ||
      recording ||
      processing ||
      dictating ||
      writing ||
      ids.length < 2
    )
      return false;
    const detail = await app.perform("meeting-join", () =>
      call("join_meetings", { ids }),
    );
    if (detail) {
      accept(detail);
      await refresh();
    }
    return !!detail;
  }
  async function exportMarkdown(row: MeetingSummary): Promise<void> {
    if (dirty || app.busy("meeting-export")) return;
    await app.perform("meeting-export", async () => {
      const detail =
        selected?.id === row.id
          ? selected
          : await call("get_meeting", { id: row.id });
      const path = await chooseExport(`${safeFilename(detail.title)}.md`, "md");
      if (path) {
        await call("export_meeting", {
          id: detail.id,
          path,
          format: "markdown",
          transcriptVersion:
            detail.cleanedTranscript === null ? "recognized" : "cleaned",
        });
        app.notice = "Meeting exported.";
      }
    });
  }
  async function discardRecording(): Promise<void> {
    const id =
      app.snapshot.activeMeetingId ?? (recording ? selected?.id : null);
    if (
      !id ||
      app.busy("meeting-record") ||
      !(await app.confirm(
        "Discard this recording?",
        "This recording and its files will be deleted.",
        "Discard recording",
        true,
      ))
    )
      return;
    await app.perform("meeting-delete", async () => {
      await call("discard_meeting", { id, confirmed: true });
      if (selected?.id === id) selected = null;
      meetings = meetings.filter((row) => row.id !== id);
    });
    await refresh();
  }

  return {
    get aiSettings() {
      return aiSettings;
    },
    get showRaw() {
      return showRaw;
    },
    get rawTranscript() {
      return rawTranscript;
    },
    get rawLoading() {
      return rawLoading;
    },
    get rawError() {
      return rawError;
    },
    viewRaw,
    renameSpeaker,
    reassignPassage,
    get searching() {
      return searching;
    },
    get searchError() {
      return searchError;
    },
    searchLine: (id: string) =>
      searchMatches.find((match) => match.id === id)?.line ?? "",
    join,
    exportMarkdown,
    discardRecording,
    get meetings() {
      return meetings;
    },
    set meetings(value: typeof meetings) {
      meetings = value;
    },
    get selected() {
      return selected;
    },
    set selected(value: typeof selected) {
      selected = value;
    },
    get hiddenStatus() {
      return hiddenStatus;
    },
    set hiddenStatus(value: typeof hiddenStatus) {
      hiddenStatus = value;
    },
    get statusKey() {
      return statusKey;
    },
    get loaded() {
      return loaded;
    },
    set loaded(value: typeof loaded) {
      loaded = value;
    },
    get search() {
      return search;
    },
    set search(value: typeof search) {
      search = value;
    },
    get newTitle() {
      return newTitle;
    },
    set newTitle(value: typeof newTitle) {
      newTitle = value;
    },
    get source() {
      return source;
    },
    set source(value: typeof source) {
      source = value;
    },
    get sourceWarning() {
      return sourceWarning;
    },
    get systemStatus() {
      return systemStatus;
    },
    get systemState() {
      return systemState;
    },
    get systemNotice() {
      return systemNotice;
    },
    get systemFixError() {
      return systemFixError;
    },
    set systemFixError(value: typeof systemFixError) {
      systemFixError = value;
    },
    fixSystem,
    get startRequested() {
      return startRequested;
    },
    set startRequested(value: typeof startRequested) {
      startRequested = value;
    },
    get title() {
      return title;
    },
    set title(value: typeof title) {
      title = value;
    },
    get transcript() {
      return transcript;
    },
    set transcript(value: typeof transcript) {
      transcript = value;
    },
    get showCleaned() {
      return showCleaned;
    },
    set showCleaned(value: typeof showCleaned) {
      showCleaned = value;
    },
    get renameOpen() {
      return renameOpen;
    },
    set renameOpen(value: typeof renameOpen) {
      renameOpen = value;
    },
    get notesOpen() {
      return notesOpen;
    },
    set notesOpen(value: typeof notesOpen) {
      notesOpen = value;
    },
    get exportOpen() {
      return exportOpen;
    },
    set exportOpen(value: typeof exportOpen) {
      exportOpen = value;
    },
    get defaultsOpen() {
      return defaultsOpen;
    },
    set defaultsOpen(value: typeof defaultsOpen) {
      defaultsOpen = value;
    },
    get meetingError() {
      return meetingError;
    },
    set meetingError(value: typeof meetingError) {
      meetingError = value;
    },
    get notes() {
      return notes;
    },
    set notes(value: typeof notes) {
      notes = value;
    },
    get transcriptBlocks() {
      return transcriptBlocks;
    },
    set transcriptBlocks(value: typeof transcriptBlocks) {
      transcriptBlocks = value;
    },
    get transcribeSetup() {
      return transcribeSetup;
    },
    set transcribeSetup(value: typeof transcribeSetup) {
      transcribeSetup = value;
    },
    get transcriptionOverride() {
      return transcriptionOverride;
    },
    set transcriptionOverride(value: typeof transcriptionOverride) {
      transcriptionOverride = value;
    },
    get transcriptionModel() {
      return transcriptionModel;
    },
    get instructions() {
      return instructions;
    },
    set instructions(value: typeof instructions) {
      instructions = value;
    },
    get meetingPage() {
      return meetingPage;
    },
    set meetingPage(value: typeof meetingPage) {
      meetingPage = value;
    },
    get notePresets() {
      return notePresets;
    },
    get notePreset() {
      return notePreset;
    },
    set notePreset(value: typeof notePreset) {
      notePreset = value;
    },
    get detailTabs() {
      return detailTabs;
    },
    get tab() {
      return tab;
    },
    set tab(value: typeof tab) {
      tab = value;
    },
    get exportFormat() {
      return exportFormat;
    },
    set exportFormat(value: typeof exportFormat) {
      exportFormat = value;
    },
    get dirty() {
      return dirty;
    },
    get filtered() {
      return filtered;
    },
    get recording() {
      return recording;
    },
    get processing() {
      return processing;
    },
    get writing() {
      return writing;
    },
    get dictating() {
      return dictating;
    },
    get structured() {
      return structured;
    },
    get entries() {
      return entries;
    },
    get audioNotice() {
      return audioNotice;
    },
    get recognitionNotices() {
      return recognitionNotices;
    },
    get processingBlocked() {
      return processingBlocked;
    },
    get cleanupPackage() {
      return cleanupPackage;
    },
    get speechModels() {
      return speechModels;
    },
    get transcriptionEntry() {
      return transcriptionEntry;
    },
    get transcriptionUnavailableReason() {
      return transcriptionUnavailableReason;
    },
    get speakerModel() {
      return speakerModel;
    },
    get speakerLabelsAvailable() {
      return speakerLabelsAvailable;
    },
    get speakerInstallPending() {
      return speakerInstallPending;
    },
    get speakerInstallDisabled() {
      return speakerInstallDisabled;
    },
    installSpeakerLabels,
    get remoteSpeechModel() {
      return remoteSpeechModel;
    },
    get remoteSpeechName() {
      return remoteSpeechName;
    },
    get transcriptionModelAvailable() {
      return transcriptionModelAvailable;
    },
    get transcribeBlocked() {
      return transcribeBlocked;
    },
    modelName,
    openTranscribeSetup,
    loadAiSettings,
    get elapsed() {
      return elapsed;
    },
    date,
    tabKeydown,
    refresh,
    select,
    back,
    showPage,
    start,
    stop,
    importFile,
    saveChanges,
    reloadSaved,
    get transcriptConflict() {
      return transcriptConflict;
    },
    switchTranscript,
    cleanTranscript,
    changeSource,
    sourceName,
    transcribe,
    generateNotes,
    setSpeakerLabels,
    cancel,
    remove,
    exportFile,
  };
}

export type MeetingSession = ReturnType<typeof createMeetingSession>;
