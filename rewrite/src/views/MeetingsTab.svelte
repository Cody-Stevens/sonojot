<script lang="ts">
  import MeetingSyncNotice from "../components/quiet/MeetingSyncNotice.svelte";
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { createMeetingSession } from "../lib/meeting-session.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { defaultMeetingModel } from "../lib/meeting-models";
  import { meetingDays } from "../lib/meeting-display";
  import type { ModelsTarget } from "../lib/model-library";
  import type { ErrorAction } from "../lib/errors";
  import type { RouteTarget } from "../lib/route.svelte";
  import Chip from "../components/quiet/Chip.svelte";
  import Well from "../components/quiet/Well.svelte";
  import MeetingSlip from "../components/quiet/MeetingSlip.svelte";
  import MeetingPage from "../components/quiet/MeetingPage.svelte";
  import RecordingBand from "../components/quiet/RecordingBand.svelte";
  import MeetingSources from "../components/quiet/MeetingSources.svelte";
  import MeetingImport from "../components/quiet/MeetingImport.svelte";
  import MeetingTrash from "../components/quiet/MeetingTrash.svelte";
  let {
    onnavigate,
    onmanage,
    onerroraction,
  }: {
    onnavigate: (target: RouteTarget) => unknown;
    onmanage: (target: ModelsTarget) => void;
    onerroraction?: (action: ErrorAction) => void;
  } = $props();
  const session = createMeetingSession();
  let sources = $state(false),
    importing = $state(false),
    dropping = $state(false);
  let importPath = $state("");
  let list = $state<HTMLDivElement>();
  const recording = $derived(
    app.snapshot.meetingRecording || session.recording,
  );
  const listedMeetings = $derived(
    session.filtered.filter(
      (meeting) =>
        meeting.status !== "recording" &&
        !(
          app.snapshot.meetingRecording &&
          meeting.id === app.snapshot.activeMeetingId
        ),
    ),
  );
  const groups = $derived(meetingDays(listedMeetings));
  const configuredSpeech = $derived(
    defaultMeetingModel(app.snapshot.settings, ai.data),
  );
  const speech = $derived(
    configuredSpeech === "openrouter"
      ? session.remoteSpeechName
      : session.modelName(configuredSpeech),
  );
  const notes = $derived(
    ai.data?.backend === "openrouter"
      ? ai.data.model
      : session.modelName(app.snapshot.settings.notesModel),
  );
  onMount(() => {
    void ai.refresh();
    let disposed = false,
      stop: (() => void) | undefined;
    if (app.native)
      void getCurrentWebview()
        .onDragDropEvent((event) => {
          if (event.payload.type === "enter" || event.payload.type === "over")
            dropping = !recording;
          else if (event.payload.type === "drop") {
            dropping = false;
            if (!recording && event.payload.paths.length === 1) {
              importPath = event.payload.paths[0];
              importing = true;
            } else if (event.payload.paths.length > 1)
              app.error = "Import one recording at a time, then use Join.";
          } else dropping = false;
        })
        .then((next) => {
          if (disposed) next();
          else stop = next;
        })
        .catch(() => {});
    return () => {
      disposed = true;
      stop?.();
    };
  });
  async function open(id: string) {
    await session.select(id);
    await tick();
    document
      .querySelector<HTMLButtonElement>('[aria-label="All meetings"]')
      ?.focus({ preventScroll: true });
  }
  function keyboard(event: KeyboardEvent) {
    if (
      !list ||
      !(event.target instanceof HTMLElement) ||
      !event.target.matches("[data-meeting-id]")
    )
      return;
    const rows = [...list.querySelectorAll<HTMLElement>("[data-meeting-id]")];
    const index = rows.indexOf(event.target);
    const next =
      event.key === "ArrowDown"
        ? index + 1
        : event.key === "ArrowUp"
          ? index - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? rows.length - 1
              : null;
    if (next !== null) {
      event.preventDefault();
      rows[Math.max(0, Math.min(rows.length - 1, next))]?.focus();
    }
  }
</script>

<svelte:window onkeydown={keyboard} />
{#if recording}<RecordingBand
    {session}
    onsources={() => (sources = true)}
  />{/if}
{#if session.selected && !session.recording}<MeetingPage
    {session}
    {onerroraction}
  />
{:else}
  {#if !recording}<section
      class="stock paper-card meetings-band"
      aria-label="Meetings status"
    >
      <div class="idle-head">
        <span class="idle-dot"></span>
        <h1 data-text-surface="emboss">Not recording</h1>
        <span class="spacer"></span><button
          class="button danger record-button"
          disabled={!app.native ||
            session.dictating ||
            session.startRequested ||
            app.busy("meeting-record") ||
            app.busy("settings")}
          onclick={session.start}>● Record meeting</button
        ><button
          class="button"
          disabled={!app.native ||
            session.dictating ||
            app.busy("meeting-import")}
          onclick={() => (importing = true)}>Import…</button
        >
      </div>
      <div class="meeting-chips" role="group" aria-label="Meeting settings">
        <Chip
          label="Sources, speech and speakers"
          onclick={() => void onnavigate({ tab: "meetings", stage: "speech" })}
          ><span
            >{session.source === "both"
              ? "Mic + system"
              : session.source === "microphone"
                ? "Mic"
                : "System"}</span
          ><span>·</span><span>{speech}</span><span
            >· {app.snapshot.settings.meetingSpeakerLabels
              ? "Speakers"
              : "No speakers"}</span
          ></Chip
        ><span class="arrow" aria-hidden="true">→</span><Chip
          label="Notes"
          onclick={() => void onnavigate({ tab: "meetings", stage: "notes" })}
          >{app.snapshot.settings.meetingNotesPreset === null
            ? "No notes"
            : `Notes · ${notes}`}</Chip
        >
      </div>
      {#if session.dictating}<p class="field-help">
          Finish dictation before recording a meeting.
        </p>{/if}
    </section>{/if}
  <div class="meeting-search">
    <Well
      ><span aria-hidden="true">⌕</span><input
        aria-label="Search meetings"
        placeholder="Search titles, notes and transcripts"
        bind:value={session.search}
      />{#if session.search}<button
          class="button small"
          aria-label="Clear search"
          onclick={() => (session.search = "")}>×</button
        >{/if}</Well
    >
  </div>
  {#if app.snapshot.capabilities.sync}<MeetingSyncNotice />{/if}
  {#if app.snapshot.capabilities.sync}<MeetingTrash onrestored={() => session.refresh(true)} />{/if}
  {#if session.searching}<p role="status">
      Searching meetings…
    </p>{:else if session.searchError}<p role="alert">
      {session.searchError}
    </p>{/if}
  <div bind:this={list} class="meeting-list">
    {#each groups as group}<section aria-label={group.label}>
        <h2 class="day-label" data-text-surface="flat">{group.label}</h2>
        {#each group.rows as meeting (meeting.id)}<MeetingSlip
            match={session.searchLine(meeting.id)}
            {meeting}
            onopen={() => void open(meeting.id)}
            onexport={() => void session.exportMarkdown(meeting)}
          />{/each}
      </section>{/each}
    {#if !session.loaded}<p role="status">
        Loading meetings…
      </p>{:else if !listedMeetings.length && !session.searching && !session.searchError}<p
        class="empty-list"
      >
        {session.search
          ? "No matching meetings."
          : "Your recordings will appear here."}
      </p>{/if}
  </div>
{/if}
{#if sources}<MeetingSources
    {session}
    onclose={() => (sources = false)}
    onspeech={() => void onnavigate({ tab: "meetings", stage: "speech" })}
  />{/if}
{#if importing}<MeetingImport
    {session}
    bind:path={importPath}
    onclose={() => (importing = false)}
  />{/if}
{#if dropping}<div class="drop-catch" role="status">
    Drop to import as a meeting
  </div>{/if}

<style>
  .meetings-band {
    padding: 25px 28px;
    border-radius: 17px;
    margin: 0 0 24px;
  }
  .idle-head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .idle-head h1 {
    font-size: 25px;
    margin: 0;
  }
  .idle-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--soft-ink);
  }
  .spacer {
    flex: 1;
  }
  .record-button {
    font-weight: 600;
  }
  .meeting-chips {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin: 24px 0 0 23px;
  }
  .arrow {
    color: var(--soft-ink);
  }
  .meeting-search {
    margin: 22px 0;
  }
  .day-label {
    font-size: 14px;
    font-weight: 400;
    color: var(--soft-ink);
    margin: 25px 0 10px;
  }
  .empty-list {
    color: var(--soft-ink);
    padding: 24px 0;
  }
  .drop-catch {
    position: fixed;
    inset: 25% 20%;
    display: grid;
    place-items: center;
    z-index: 100;
    background: var(--paper);
    border: 2px dashed var(--dye);
    border-radius: 18px;
    font-size: 24px;
    box-shadow: var(--raise);
    pointer-events: none;
  }
  @media (max-width: 760px) {
    .idle-head {
      flex-wrap: wrap;
    }
    .meeting-chips {
      margin-left: 0;
    }
    .meetings-band {
      padding: 22px;
    }
  }
</style>
