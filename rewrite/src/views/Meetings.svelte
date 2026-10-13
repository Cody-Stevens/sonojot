<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import Card from "../components/Card.svelte";
  import Icon from "../components/Icon.svelte";
  import Meter from "../components/Meter.svelte";
  import MeetingActivity from "../components/MeetingActivity.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import Toggle from "../components/Toggle.svelte";
  import { presentError, type ErrorAction } from "../lib/errors";
  import { app, STATUS_NOTICE_MS } from "../lib/app.svelte";
  import {
    meetingSourceWarning,
    prepareMeetingStart,
  } from "../lib/meeting-source";
  import { call, chooseExport, chooseMedia } from "../lib/api";
  import MeetingAudioExport from "../components/MeetingAudioExport.svelte";
  import ModelSummary from "../components/ModelSummary.svelte";
  import RemoteModelBadge from "../components/RemoteModelBadge.svelte";
  import type { AiSettings, AiModel } from "../lib/ai";
  import {
    checking,
    formatBytes,
    installed,
    installing,
    modelUnavailableReason,
    type ModelRole,
  } from "../lib/model-library";
  import { library } from "../lib/model-library.svelte";
  import {
    defaultMeetingModel,
    meetingSpeechModels,
  } from "../lib/meeting-models";
  import {
    hasStructure,
    parseTranscript,
    serializeTranscript,
    type TranscriptBlock,
  } from "../lib/meeting-transcript";
  import {
    meetingClock,
    meetingNotice,
    outageLabel,
    transcriptEntries,
    visibleRecognitionWarnings,
  } from "../lib/meeting-notices";
  import {
    duration,
    errorMessage,
    safeFilename,
    type MeetingDetail,
    type MeetingSummary,
    type NotesPreset,
  } from "../lib/protocol";

  let {
    onerroraction,
    onmanage,
  }: {
    onerroraction?: (action: ErrorAction) => void;
    onmanage?: (role: ModelRole) => void;
  } = $props();
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
  let startRequested = $state(false);
  let title = $state("");
  let transcript = $state("");
  let recognizedDraft = $state("");
  let cleanedDraft = $state<string | null>(null);
  let showCleaned = $state(false);
  let renameOpen = $state(false);
  let notesOpen = $state(false);
  let exportOpen = $state(false);
  let defaultsOpen = $state(false);
  let meetingError = $state("");
  let notes = $state("");
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
  let instructions = $state("");
  let meetingPage = $state<"view" | "start">("view");
  const notePresets = [
    { id: "summary", label: "Summary" },
    { id: "tasks", label: "Tasks by person" },
    { id: "speakers", label: "Structure by speaker" },
    { id: "brief", label: "Executive brief" },
    { id: "formattedTranscript", label: "Corrected transcript (AI)" },
    { id: "custom", label: "Custom instructions…" },
  ] satisfies { id: NotesPreset; label: string }[];
  let notePreset = $state<NotesPreset>("summary");
  const detailTabs = [
    { id: "transcript", label: "Transcript" },
    { id: "notes", label: "Notes" },
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
  const filtered = $derived(
    meetings.filter((item) =>
      item.title.toLowerCase().includes(search.toLowerCase()),
    ),
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
  const notesRemote = $derived(
    aiSettings?.backend === "openrouter" ||
      app.snapshot.settings.notesModel === "openrouter",
  );
  const remoteNotesName = $derived(
    aiCatalog.find((model) => model.id === aiSettings?.model)?.name ??
      aiSettings?.model ??
      "OpenRouter",
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
  $effect(() => {
    // The speaker setting and speech selector share the library's live install state.
    // acquire() counts its users in reactive state, so it must not become a dependency.
    if (meetingPage === "start" || transcribeSetup || defaultsOpen || notesOpen)
      return untrack(() => library.acquire());
  });
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
    if (app.native) void refresh(true);
    void loadAiSettings();
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
    if (target !== "notes" || selected?.notes) tab = target;
    document.getElementById(`meeting-tab-${target}`)?.focus();
  }
  function accept(detail: MeetingDetail, preserveDraft = false): void {
    const keepDraft = preserveDraft && selected?.id === detail.id && dirty;
    const keepTitle = keepDraft && title !== selected?.title;
    if (selected?.id !== detail.id) {
      transcribeSetup = false;
      renameOpen = false;
      notesOpen = false;
      exportOpen = false;
      meetingError = "";
      showCleaned = detail.cleanedTranscript !== null;
      tab = "transcript";
    } else if (
      selected.cleanedTranscript === null &&
      detail.cleanedTranscript !== null
    )
      showCleaned = true;
    if (detail.cleanedTranscript === null) showCleaned = false;
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
    meetings = [
      detail,
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
      instructions = "";
      tab = "transcript";
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
        const recordingSource = await prepareMeetingStart(
          app.snapshot.settings,
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
        if (recordingSource === null) return;
        return call("start_meeting", {
          title: newTitle.trim(),
          source: recordingSource,
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
  async function importFile(): Promise<void> {
    if (!(await canLeave())) return;
    const detail = await app.perform("meeting-import", async () => {
      const path = await chooseMedia();
      return path ? call("import_media", { path }) : undefined;
    });
    if (detail) accept(detail);
  }
  async function saveChanges(): Promise<void> {
    const id = selected?.id;
    if (!id || !title.trim()) return;
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
        instructions:
          notePreset === "formattedTranscript" ? "" : instructions.trim(),
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
        `“${detail.title}” and its files in SonoJot will be deleted.`,
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
</script>

{#snippet meetingDefaults()}
  <SettingRow
    label="Name meetings automatically"
    help="From what was said. Uses local AI when no title was entered."
  >
    <Toggle
      label="Name meetings automatically"
      checked={app.snapshot.settings.autoMeetingTitles}
      disabled={!app.native || app.busy("settings")}
      onchange={(autoMeetingTitles) => app.saveSettings({ autoMeetingTitles })}
    />
  </SettingRow>
  {#if speakerLabelsAvailable && speakerModel}
    <SettingRow
      label="Label speakers"
      help={installed(speakerModel) || checking(speakerModel)
        ? "Separates voices on this computer after transcription."
        : "Downloads Nemotron 3 Diarization (" +
          formatBytes(speakerModel.totalBytes || speakerModel.diskBytes) +
          ")."}
    >
      {#if installed(speakerModel)}
        <Toggle
          label="Label speakers"
          checked={app.snapshot.settings.meetingSpeakerLabels}
          disabled={!app.native || app.busy("settings")}
          onchange={(meetingSpeakerLabels) =>
            app.saveSettings({ meetingSpeakerLabels })}
        />
      {:else}
        <button
          class="button small"
          disabled={speakerInstallDisabled}
          onclick={installSpeakerLabels}
        >
          {checking(speakerModel)
            ? "Checking…"
            : speakerInstallPending
              ? "Installing…"
              : "Install"}
        </button>
      {/if}
    </SettingRow>
    {#if speakerInstallPending}
      <progress
        aria-label="Speaker package download"
        max={speakerModel.totalBytes || 1}
        value={speakerModel.downloadedBytes}
      ></progress>
      <p class="field-help" data-text-surface="flat">
        {formatBytes(speakerModel.downloadedBytes)} of {formatBytes(
          speakerModel.totalBytes,
        )}
      </p>
    {/if}
  {/if}
  {#if library.error || speakerModel?.error}
    <ErrorBanner
      error={library.error || speakerModel?.error}
      onaction={onerroraction}
    />
  {/if}
  <button class="button quiet small" onclick={() => onmanage?.("asr")}
    >Speaker package in Models</button
  >
{/snippet}

{#if !selected}
  <div class="segments" role="group" aria-label="Meetings">
    <button
      type="button"
      aria-pressed={meetingPage === "view"}
      onclick={() => showPage("view")}>View meetings</button
    >
    <button
      type="button"
      aria-pressed={meetingPage === "start"}
      onclick={() => showPage("start")}>Start a meeting</button
    >
  </div>
{/if}
{#if app.snapshot.meetingRecording && selected?.id !== app.snapshot.activeMeetingId}
  <Card title="Meeting recording">
    <Meter level={app.snapshot.audioLevel} active />
    {#if app.snapshot.meetingSourceStatus.length}
      <p
        class="field-help"
        data-text-surface="flat"
        role="status"
        aria-label="Recording source status"
      >
        {app.snapshot.meetingSourceStatus.join(" · ")}
      </p>
    {/if}
    <div class="actions">
      <button
        class="button primary"
        disabled={app.busy("meeting-record")}
        onclick={stop}><Icon name="stop" size={19} />Finish recording</button
      >
      <button
        class="button"
        onclick={() => {
          const id = app.snapshot.activeMeetingId;
          if (id) void select(id);
        }}>Open recording</button
      >
    </div>
  </Card>
{/if}
{#if selected}
  <button class="back-button" disabled={writing} onclick={back}
    ><Icon name="back" size={19} />All meetings</button
  >
  <header class="page-heading meeting-heading">
    {#if renameOpen}
      <div class="rename-field">
        <label class="sr-only" for="meeting-title">Meeting title</label>
        <input
          id="meeting-title"
          bind:value={title}
          disabled={writing || recording || processing}
          maxlength="200"
        />
        <button
          class="button quiet small"
          onclick={() => {
            title = selected?.title ?? title;
            renameOpen = false;
          }}>Cancel rename</button
        >
      </div>
    {:else}
      <h1 class="text-grain" data-text-surface="emboss" title={selected.title}>
        {selected.title}
      </h1>
    {/if}
    <div class="actions compact">
      <button
        class="button"
        aria-expanded={exportOpen}
        aria-controls="meeting-export-drawer"
        onclick={() => (exportOpen = !exportOpen)}>Save a copy</button
      >
      <details class="meeting-more">
        <summary class="button">More</summary>
        <div class="meeting-menu stock paper-popover paper-menu">
          <button
            class="button quiet"
            disabled={writing || recording || processing}
            onclick={() => (renameOpen = true)}>Rename</button
          >
          <button
            class="button quiet"
            disabled={transcribeBlocked}
            onclick={openTranscribeSetup}>Transcribe again…</button
          >
          {#if selected.speakerCount >= 2}
            <SettingRow
              label="Speaker labels"
              help="Applies to the transcript, saved copies and new notes."
            >
              <Toggle
                label="Speaker labels"
                checked={selected.speakerLabels}
                disabled={!app.native ||
                  dirty ||
                  recording ||
                  processing ||
                  writing ||
                  app.busy("meeting-speakers")}
                onchange={setSpeakerLabels}
              />
            </SettingRow>
          {/if}
          <button
            class="button quiet danger-text"
            disabled={!app.native ||
              recording ||
              processing ||
              writing ||
              app.busy("meeting-speakers")}
            onclick={remove}>Delete meeting…</button
          >
        </div>
      </details>
    </div>
    <p class="meeting-metadata" data-text-surface="flat">
      {date(selected.createdAt)} · {duration(elapsed)} · {sourceName(
        selected.source,
      )}{#if selected.speakerCount > 0}
        · {selected.speakerCount}
        {selected.speakerCount === 1 ? "speaker" : "speakers"}{/if}
    </p>
  </header>
  {#if audioNotice}<p
      class="meeting-notice field-help"
      data-text-surface="flat"
      aria-label="Missing audio notice"
    >
      {audioNotice}
    </p>{/if}
  {#if (selected.error || meetingError) && (!statusKey || statusKey !== hiddenStatus)}
    {#if statusKey}
      <ErrorBanner
        error={selected.error}
        onaction={onerroraction}
        ondismiss={() => (hiddenStatus = statusKey)}
      />
    {:else}
      <div class="meeting-notice" role="status">
        <p data-text-surface="flat">
          {selected.transcriptionStoppedAtSeconds !== null
            ? "Transcription stopped at " +
              meetingClock(selected.transcriptionStoppedAtSeconds)
            : selected.status === "interrupted"
              ? "Transcription stopped before it finished."
              : meetingError || selected.error}
        </p>
        {#if selected.hasAudio && (selected.transcriptionStoppedAtSeconds !== null || selected.status === "interrupted" || meetingError || !selected.hasTranscript)}<button
            class="button small"
            disabled={transcribeBlocked}
            onclick={openTranscribeSetup}>Transcribe again</button
          >{/if}
        {#if selected.transcriptionStoppedAtSeconds !== null || selected.status === "interrupted"}
          <details>
            <summary>Details</summary>
            <p class="field-help" data-text-surface="flat">
              {meetingError || selected.error}
            </p>
          </details>
        {/if}
      </div>
    {/if}
  {/if}
  {#if recording}
    <Card title="Recording this meeting">
      <Meter level={app.snapshot.audioLevel} active />
      <p class="field-help" data-text-surface="flat">
        Recording {sourceName(selected.source).toLowerCase()}.
      </p>
      {#if app.snapshot.meetingSourceStatus.length}
        <p
          class="field-help"
          data-text-surface="flat"
          role="status"
          aria-label="Recording source status"
        >
          {app.snapshot.meetingSourceStatus.join(" · ")}
        </p>
      {/if}
      <button
        class="button primary"
        disabled={app.busy("meeting-record")}
        onclick={stop}
        ><Icon name="stop" size={19} />{app.busy("meeting-record")
          ? "Finishing…"
          : "Finish recording"}</button
      >
    </Card>
  {/if}
  {#if exportOpen}
    <Card title="Save a copy">
      <div id="meeting-export-drawer" class="meeting-export-grid">
        <section aria-label="Document export">
          <h3 data-text-surface="emboss">Document</h3>
          <label class="field-label" for="document-format">Format</label>
          <select
            id="document-format"
            bind:value={exportFormat}
            disabled={app.busy("meeting-export")}
          >
            <option value="markdown">Markdown (.md)</option><option value="text"
              >Plain text (.txt)</option
            >
            <option value="html">Web page (.html)</option><option value="pdf"
              >PDF (.pdf)</option
            >
          </select>
          <div class="actions spaced">
            <button
              class="button primary"
              disabled={!app.native ||
                dirty ||
                recording ||
                processing ||
                app.busy("meeting-export") ||
                writing}
              onclick={exportFile}
              >{app.busy("meeting-export") ? "Exporting…" : "Export"}</button
            >
          </div>
          {#if dirty}<p class="field-help" data-text-surface="flat">
              Save your changes before exporting.
            </p>{/if}
          <p class="field-help" data-text-surface="flat">
            Includes the {showCleaned ? "cleaned" : "as recognized"} transcript and
            any notes.
          </p>
        </section>
        <section aria-label="Original audio export">
          <h3 data-text-surface="emboss">Original audio</h3>
          <MeetingAudioExport
            meetingId={selected.id}
            title={selected.title}
            status={selected.status}
            hasAudio={selected.hasAudio}
            disabled={selected.status === "finalizing"}
          />
        </section>
      </div>
    </Card>
  {/if}
  {#if !recording}
    <div class="meeting-steps" aria-label="What you can do">
      <Card title="Clean up the transcript">
        <p class="field-help" data-text-surface="flat">
          {selected.cleanedTranscript !== null
            ? "Done. Switch between cleaned and as recognized below."
            : "Removes filler words and repeats, fixes punctuation. The original stays."}
        </p>
        {#if selected.cleanedTranscript === null}
          <button
            class="button primary"
            disabled={processingBlocked}
            onclick={cleanTranscript}
            >{app.busy("meeting-clean") ? "Cleaning…" : "Clean up"}</button
          >
        {/if}
      </Card>
      <Card title="Process notes">
        <p class="field-help" data-text-surface="flat">
          {selected.notes
            ? "Done. Open the Notes tab, or process again in another format."
            : "Summary, tasks by person, executive brief or your own instructions."}
        </p>
        <button
          class="button"
          disabled={processingBlocked}
          aria-expanded={notesOpen}
          aria-controls="meeting-notes-drawer"
          onclick={() => {
            notesOpen = !notesOpen;
            void loadAiSettings();
          }}>{selected.notes ? "Process again" : "Process notes"}</button
        >
      </Card>
    </div>
    {#if dirty}<p class="field-help" data-text-surface="flat">
        Save your changes before processing this meeting.
      </p>{/if}
    {#if dictating}<p class="field-help" data-text-surface="flat">
        Finish dictation before processing this meeting.
      </p>{/if}
    {#if notesOpen}
      <Card title="Process notes">
        <div id="meeting-notes-drawer">
          <label class="field-label" for="note-preset">Format</label>
          <select
            id="note-preset"
            bind:value={notePreset}
            disabled={processing}
          >
            {#each notePresets as preset}<option value={preset.id}
                >{preset.label}</option
              >{/each}
          </select>
          {#if notesRemote}
            <p class="field-help spaced" data-text-surface="flat">
              Notes · {remoteNotesName} · OpenRouter <RemoteModelBadge />
            </p>
          {:else}<ModelSummary role="notes" {onmanage} />{/if}
          <button class="button quiet small" onclick={() => onmanage?.("notes")}
            >Change in Models</button
          >
          {#if notePreset === "formattedTranscript"}<p
              class="field-help"
              data-text-surface="flat"
            >
              Creates an AI-corrected transcript in Notes. Both transcript
              versions stay available.
            </p>{/if}
          <details class="meeting-fold" open={notePreset === "custom"}>
            <summary
              >{notePreset === "custom"
                ? "Your instructions"
                : "Add instructions (optional)"}</summary
            >
            <label class="sr-only" for="note-instructions"
              >{notePreset === "custom"
                ? "Your instructions"
                : "Additional instructions"}</label
            >
            <textarea
              id="note-instructions"
              bind:value={instructions}
              rows="3"
              placeholder="For example, decisions and action items with owners."
              disabled={processing || notePreset === "formattedTranscript"}
              maxlength="2000"></textarea>
            {#if notePreset === "formattedTranscript"}<p
                class="field-help"
                data-text-surface="flat"
              >
                This format uses its own correction instructions.
              </p>{/if}
          </details>
          <div class="actions spaced">
            <button
              class="button primary"
              disabled={processingBlocked ||
                (notePreset === "custom" && !instructions.trim())}
              onclick={generateNotes}
              >{app.busy("meeting-notes")
                ? "Writing notes…"
                : "Generate notes"}</button
            >
            <button class="button" onclick={() => (notesOpen = false)}
              >Cancel</button
            >
          </div>
        </div>
      </Card>
    {/if}
    {#if transcribeSetup}
      <Card
        title={selected.hasTranscript
          ? "Transcribe again"
          : "Transcribe recording"}
      >
        {#if selected.hasTranscript}<p
            class="field-help"
            data-text-surface="flat"
          >
            Transcribing again replaces the transcript and any cleaned copy.
            Notes stay saved.
          </p>{/if}
        <ModelSummary role="asr" {onmanage} />
        <button class="button quiet small" onclick={() => onmanage?.("asr")}
          >Change in Models</button
        >
        <label class="field-label spaced" for="transcription-model"
          >Speech model</label
        >
        <select
          id="transcription-model"
          value={transcriptionModelAvailable ? transcriptionModel : ""}
          onchange={(event) =>
            (transcriptionOverride = event.currentTarget.value)}
        >
          {#if !transcriptionModelAvailable}<option value="" disabled
              >{transcriptionModel === "openrouter"
                ? "OpenRouter unavailable"
                : transcriptionModel
                  ? `${modelName(transcriptionModel)} · ${transcriptionEntry && checking(transcriptionEntry) ? "Checking…" : "Unavailable"}`
                  : "Choose a speech model"}</option
            >{/if}
          {#each speechModels as model (model.id)}<option value={model.id}
              >{model.name}</option
            >{/each}
          {#if remoteSpeechModel}<option value="openrouter"
              >OpenRouter · {remoteSpeechName}</option
            >{/if}
        </select>
        {#if transcriptionUnavailableReason}<p
            class="field-help"
            data-text-surface="flat"
          >
            {transcriptionUnavailableReason}
          </p>{/if}
        {#if transcriptionModel === "openrouter"}<RemoteModelBadge />{/if}
        {#if selected.transcriptionModel}<p
            class="field-help"
            data-text-surface="flat"
          >
            Last transcribed with {modelName(selected.transcriptionModel)}
          </p>{/if}
        <div class="actions spaced">
          <button
            class="button primary"
            disabled={transcribeBlocked || !transcriptionModelAvailable}
            onclick={transcribe}>Start</button
          >
          <button class="button" onclick={() => (transcribeSetup = false)}
            >Cancel</button
          >
        </div>
      </Card>
    {:else if !selected.hasTranscript}
      <div class="actions spaced">
        <button
          class="button"
          disabled={transcribeBlocked}
          onclick={openTranscribeSetup}>Transcribe recording</button
        >
      </div>
      {#if !selected.hasAudio}<p class="field-help" data-text-surface="flat">
          No audio file is available for this meeting.
        </p>{/if}
    {/if}
  {/if}

  <div class="section-toolbar meeting-document-toolbar">
    <div
      class="text-tabs"
      role="tablist"
      aria-label="Meeting document"
      tabindex={-1}
      onkeydown={tabKeydown}
    >
      {#each detailTabs as item (item.id)}
        <span class="meeting-tab-wrap">
          <button
            type="button"
            id={"meeting-tab-" + item.id}
            role="tab"
            class:chosen={tab === item.id}
            aria-selected={tab === item.id}
            aria-disabled={item.id === "notes" && !selected.notes}
            aria-describedby={item.id === "notes" && !selected.notes
              ? "meeting-notes-tip"
              : undefined}
            aria-controls={"meeting-panel-" + item.id}
            tabindex={tab === item.id ||
            (item.id === "notes" && !selected.notes)
              ? 0
              : -1}
            onclick={() => {
              if (item.id !== "notes" || selected?.notes) tab = item.id;
            }}>{item.label}</button
          >
          {#if item.id === "notes" && !selected.notes}<span
              id="meeting-notes-tip"
              class="meeting-tooltip paper-well"
              role="tooltip"
              >No notes yet. Use Process notes above to create them.</span
            >{/if}
        </span>
      {/each}
    </div>
    <div class="actions compact">
      {#if processing}
        <MeetingActivity
          status={app.busy("meeting-clean") ? "cleaning" : selected.status}
          announce
        />
        {#if selected.status !== "finalizing"}<button
            class="button"
            disabled={app.busy("meeting-cancel")}
            onclick={cancel}
            >{app.busy("meeting-cancel")
              ? "Cancelling…"
              : "Cancel processing"}</button
          >{/if}
      {/if}
      {#if dirty}
        <button
          class="button primary"
          disabled={!app.native ||
            !title.trim() ||
            writing ||
            recording ||
            processing ||
            app.busy("meeting-speakers")}
          onclick={saveChanges}
          >{app.busy("meeting-save") ? "Saving…" : "Save changes"}</button
        >
      {:else}<span class="field-help" data-text-surface="flat">Saved</span>{/if}
    </div>
  </div>
  {#if !title.trim()}<p class="field-help" data-text-surface="flat">
      Give the meeting a title before saving.
    </p>{/if}
  {#if tab === "transcript" && selected.cleanedTranscript !== null}
    <div class="section-toolbar">
      <span class="field-help" data-text-surface="flat"
        >{showCleaned ? "Cleaned transcript" : "As recognized"}</span
      >
      <div
        class="segments transcript-versions"
        role="group"
        aria-label="Transcript version"
      >
        <button
          type="button"
          aria-pressed={showCleaned}
          onclick={() => switchTranscript(true)}>Cleaned</button
        >
        <button
          type="button"
          aria-pressed={!showCleaned}
          onclick={() => switchTranscript(false)}>As recognized</button
        >
      </div>
    </div>
  {/if}
  <Card>
    {#if tab === "transcript"}
      <div
        role="tabpanel"
        id="meeting-panel-transcript"
        aria-labelledby="meeting-tab-transcript"
      >
        {#if structured}
          <div
            class="transcript-editor"
            aria-label="Editable meeting transcript"
          >
            {#each entries as entry}
              {#if entry.kind === "outage"}
                <p
                  class="meeting-gap"
                  data-text-surface="flat"
                  aria-label={entry.outage.source === "recognition"
                    ? "Missing transcription"
                    : "Missing audio"}
                >
                  {outageLabel(entry.outage)}
                </p>
              {:else}
                {@const block = entry.block}
                <article
                  class:transcript-you={block.side === "you"}
                  class:transcript-other={block.side === "other"}
                  class="transcript-block"
                >
                  <header class="transcript-block-heading">
                    <label for={"meeting-transcript-" + block.id}
                      >{block.speaker}</label
                    >
                    {#if block.timecode}<time>{block.timecode}</time>{/if}
                  </header>
                  <textarea
                    id={"meeting-transcript-" + block.id}
                    class="transcript-block-text"
                    value={block.text}
                    disabled={writing ||
                      recording ||
                      processing ||
                      app.busy("meeting-speakers")}
                    rows="1"
                    oninput={(event) => {
                      block.text = event.currentTarget.value;
                      transcript = serializeTranscript(transcriptBlocks);
                    }}></textarea>
                </article>
              {/if}
            {/each}
          </div>
        {:else}
          <label class="sr-only" for="meeting-transcript"
            >Meeting transcript</label
          >
          <textarea
            id="meeting-transcript"
            class="document-editor"
            bind:value={transcript}
            disabled={writing ||
              recording ||
              processing ||
              app.busy("meeting-speakers")}
            placeholder={processing
              ? "Transcription is in progress…"
              : "Transcribe the recording to put the conversation here."}
            rows="11"></textarea>
        {/if}
        <p class="field-help" data-text-surface="flat">
          You can edit this version. Original recognition stays in the meeting
          history.
        </p>
        {#each recognitionNotices as warning}<p
            class="field-help"
            data-text-surface="flat"
          >
            {warning}
          </p>{/each}
      </div>
    {:else}
      <div
        role="tabpanel"
        id="meeting-panel-notes"
        aria-labelledby="meeting-tab-notes"
      >
        <label class="sr-only" for="meeting-notes">Notes</label>
        <textarea
          id="meeting-notes"
          class="document-editor"
          bind:value={notes}
          disabled={writing || recording || processing}
          rows="11"></textarea>
        <p class="field-help" data-text-surface="flat">
          Generated notes can miss details. Review them against the transcript.
        </p>
      </div>
    {/if}
  </Card>
{:else}
  <header class="page-heading">
    <h1 class="text-grain" data-text-surface="emboss">
      {meetingPage === "start" ? "Start a meeting" : "Meetings"}
    </h1>
  </header>
  {#if meetingPage === "start"}
    <Card>
      <div class="field-label" id="recording-source-label">Record from</div>
      <div
        class="segments meeting-sources"
        role="group"
        aria-labelledby="recording-source-label"
      >
        {#each [{ id: "both", label: "Microphone and system audio" }, { id: "microphone", label: "Microphone only" }, { id: "system", label: "System audio only" }] as choice}
          <button
            type="button"
            aria-pressed={source === choice.id}
            disabled={!app.native ||
              app.snapshot.meetingRecording ||
              app.busy("meeting-record") ||
              app.busy("settings") ||
              (choice.id !== "microphone" &&
                !app.snapshot.capabilities.systemAudio)}
            onclick={() =>
              changeSource(
                choice.id === "both"
                  ? "both"
                  : choice.id === "system"
                    ? "system"
                    : "microphone",
              )}>{choice.label}</button
          >
        {/each}
      </div>
      {#if sourceWarning}<p
          class="field-help meeting-source-warning"
          data-text-surface="flat"
          role="status"
        >
          {sourceWarning}
        </p>{/if}
      {#if !app.snapshot.capabilities.systemAudio}<p
          class="field-help"
          data-text-surface="flat"
        >
          System audio is unavailable on this device. Choose Microphone only.
        </p>{/if}
      <label class="field-label spaced" for="new-meeting-title"
        >Title <span class="field-help" data-text-surface="flat"
          >(optional)</span
        ></label
      >
      <input
        id="new-meeting-title"
        bind:value={newTitle}
        placeholder="SonoJot can name it for you"
        maxlength="200"
        disabled={app.snapshot.meetingRecording || app.busy("meeting-record")}
      />
      <div class="actions spaced">
        <button
          class="button primary"
          disabled={!app.native ||
            !app.ready ||
            app.snapshot.meetingRecording ||
            dictating ||
            app.busy("meeting-record") ||
            startRequested ||
            app.busy("settings") ||
            (source !== "microphone" && !app.snapshot.capabilities.systemAudio)}
          onclick={start}
        >
          <Icon name="microphone" size={21} />{app.busy("meeting-record")
            ? "Starting…"
            : "Start recording"}
        </button>
        <span class="field-help" data-text-surface="flat">or</span>
        <button
          class="button quiet"
          disabled={!app.native || app.busy("meeting-import")}
          onclick={importFile}
          >{app.busy("meeting-import")
            ? "Importing…"
            : "Import a recording"}</button
        >
      </div>
      {#if dictating}<p class="field-help" data-text-surface="flat">
          Finish dictation before recording a meeting.
        </p>{/if}
      <details class="meeting-fold">
        <summary>Meeting defaults</summary>{@render meetingDefaults()}
      </details>
    </Card>
  {:else}
    <div class="section-toolbar">
      <label class="search-field"
        ><span class="sr-only">Search meetings</span><input
          type="search"
          bind:value={search}
          placeholder="Find a meeting"
        /></label
      >
      <button
        class="button"
        disabled={!app.native || app.busy("meeting-import")}
        onclick={importFile}
        >{app.busy("meeting-import")
          ? "Importing…"
          : "Import a recording"}</button
      >
      <details class="meeting-more">
        <summary class="button">More</summary>
        <div class="meeting-menu stock paper-popover paper-menu">
          <button
            class="button quiet"
            onclick={() => (defaultsOpen = !defaultsOpen)}
            >Meeting defaults…</button
          >
          <button
            class="button quiet"
            disabled={!app.native || app.busy("meeting-load")}
            onclick={() => refresh(true)}>Refresh list</button
          >
        </div>
      </details>
    </div>
    {#if defaultsOpen}<Card title="Meeting defaults"
        >{@render meetingDefaults()}</Card
      >{/if}
    <Card title="Your meetings">
      {#if !loaded}<p class="empty-copy" data-text-surface="flat">
          Loading your meetings…
        </p>
      {:else if !meetings.length}
        <div class="empty-state">
          <Icon name="meetings" size={36} />
          <h3 data-text-surface="emboss">No meetings yet</h3>
          <p data-text-surface="flat">
            Start a meeting or import an audio or video file.
          </p>
          <button class="button primary" onclick={() => showPage("start")}
            >Start a meeting</button
          >
        </div>
      {:else if !filtered.length}<p class="empty-copy" data-text-surface="flat">
          No meetings match “{search}”.
        </p>
      {:else}
        <div class="meeting-list">
          {#each filtered as meeting (meeting.id)}
            <button
              class="meeting-item"
              title={meeting.title}
              disabled={app.busy("meeting-load")}
              onclick={() => select(meeting.id)}
            >
              <div>
                <strong>{meeting.title}</strong><small data-text-surface="flat"
                  >{date(meeting.createdAt)} · {duration(
                    meeting.durationSeconds,
                  )}</small
                >
              </div>
              <MeetingActivity status={meeting.status} plain />
            </button>
          {/each}
        </div>
      {/if}
    </Card>
  {/if}
{/if}

<style>
  .meeting-item {
    flex-wrap: wrap;
  }
  .meeting-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 9px 18px;
  }
  .meeting-heading h1,
  .rename-field {
    flex: 1 1 240px;
    min-width: 0;
    margin: 0;
  }
  .meeting-heading .meeting-metadata {
    flex-basis: 100%;
    margin: 0;
  }
  .meeting-more {
    position: relative;
    flex: none;
  }
  .meeting-more > summary {
    list-style: none;
  }
  .meeting-more > summary::after {
    content: "▾";
    margin-left: 9px;
  }
  .meeting-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 9px);
    z-index: 5;
    min-width: min(310px, 75vw);
  }
  .meeting-fold {
    margin-top: 27px;
  }
  .meeting-fold > summary {
    cursor: pointer;
    margin-bottom: 9px;
  }
  .meeting-notice,
  .meeting-gap {
    padding: 9px 18px;
    border-left: 2px solid var(--line);
    margin: 18px 0;
    color: var(--soft-ink);
  }
  .meeting-notice p {
    margin: 0 0 9px;
  }
  .meeting-gap {
    width: 100%;
  }
  .meeting-export-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 27px;
  }
  .meeting-export-grid h3 {
    margin-top: 0;
  }
  .meeting-steps {
    margin-top: 27px;
  }
  .meeting-steps :global(.paper-card) {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 9px 18px;
    align-items: center;
    margin-top: 18px;
    padding: 18px 27px;
  }
  .meeting-steps :global(.card-heading) {
    grid-column: 1;
  }
  .meeting-steps :global(h2) {
    font-size: 1.15rem;
    margin: 0;
  }
  .meeting-steps :global(.field-help) {
    grid-column: 1;
    margin: 0;
  }
  .meeting-steps :global(.button) {
    grid-column: 2;
    grid-row: 1 / span 2;
  }
  .transcript-versions {
    width: auto;
    margin: 0;
    flex: none;
  }
  .transcript-versions button {
    flex: 0 0 auto;
    font-size: 1rem;
    padding: 9px 18px;
    white-space: nowrap;
  }
  .meeting-document-toolbar {
    margin-top: 27px;
  }
  .meeting-tab-wrap {
    position: relative;
  }
  .meeting-tab-wrap [aria-disabled="true"] {
    opacity: 0.5;
    cursor: default;
  }
  .meeting-tooltip {
    display: none;
    position: absolute;
    z-index: 6;
    top: calc(100% + 9px);
    left: 0;
    width: min(300px, 65vw);
    padding: 9px 18px;
    background-color: var(--card);
    color: var(--ink);
  }
  .meeting-tab-wrap:hover .meeting-tooltip,
  .meeting-tab-wrap:focus-within .meeting-tooltip {
    display: block;
  }
  .meeting-sources {
    flex-wrap: wrap;
  }
  .meeting-sources button {
    white-space: normal;
  }
  @media (max-width: 720px) {
    .meeting-export-grid {
      grid-template-columns: 1fr;
    }
    .meeting-steps :global(.paper-card) {
      padding: 18px;
    }
  }
</style>
