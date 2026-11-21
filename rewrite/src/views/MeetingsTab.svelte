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
  import PageHeader from "../components/quiet/PageHeader.svelte";
  import PipelineBand from "../components/quiet/PipelineBand.svelte";
  import SearchBar from "../components/quiet/SearchBar.svelte";
  import Chip from "../components/quiet/Chip.svelte";
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
  let lastOpenedId = $state<string | null>(null);
  let returnMeetingId = $state<string | null>(null);
  let savedListScroll = 0;
  const recording = $derived(
    app.snapshot.meetingRecording || session.recording,
  );
  const detailOpen = $derived(!!session.selected && !session.recording);
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
  // Reset scroll position and focus Back when entering a meeting detail or switching IDs,
  // and restore the parent list scroll + meeting row focus when returning Back.
  $effect(() => {
    const id = detailOpen ? (session.selected?.id ?? null) : null;
    if (id && id !== lastOpenedId) {
      lastOpenedId = id;
      returnMeetingId = id;
      void tick().then(() => {
        document.querySelector<HTMLElement>(".page-content")?.scrollTo({
          top: 0,
        });
        document
          .querySelector<HTMLButtonElement>(
            '.meeting-heading button[aria-label="Back to Meetings"]',
          )
          ?.focus({ preventScroll: true });
      });
    } else if (!id && lastOpenedId) {
      const targetId = returnMeetingId ?? lastOpenedId;
      const targetScroll = savedListScroll;
      lastOpenedId = null;
      void tick().then(() => {
        const scroller = document.querySelector<HTMLElement>(".page-content");
        scroller?.scrollTo({ top: targetScroll });
        const row = list?.querySelector<HTMLElement>(
          `[data-meeting-id="${CSS.escape(targetId)}"]`,
        );
        row?.focus({ preventScroll: true });
      });
    }
  });
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
    savedListScroll =
      document.querySelector<HTMLElement>(".page-content")?.scrollTop ?? 0;
    returnMeetingId = id;
    await session.select(id);
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
{#if !detailOpen}
  <PageHeader title="Meetings">
    {#snippet actions()}
      {#if !recording}
        <button
          class="button danger record-button"
          disabled={!app.native ||
            session.dictating ||
            session.startRequested ||
            app.busy("meeting-record") ||
            app.busy("settings")}
          onclick={session.start}>● Record meeting</button
        >
        <button
          class="button"
          disabled={!app.native ||
            session.dictating ||
            app.busy("meeting-import")}
          onclick={() => (importing = true)}>Import…</button
        >
      {/if}
    {/snippet}
  </PageHeader>
{/if}
{#if recording}
  <RecordingBand {session} onsources={() => (sources = true)} />
{/if}
{#if detailOpen}
  <MeetingPage {session} {onerroraction} />
{:else}
  {#if !recording}
    <PipelineBand
      label="Meetings status"
      status={session.dictating ? "Dictation in progress" : ""}
    >
      {#snippet stages()}
        <div class="stage-row" role="group" aria-label="Meeting settings">
          <span class="stage-step">
            <Chip
              label="Sources, speech and speakers"
              onclick={() =>
                void onnavigate({ tab: "meetings", stage: "speech" })}
              ><span
                >{session.source === "both"
                  ? "Mic + system"
                  : session.source === "microphone"
                    ? "Mic"
                    : "System"}</span
              ><span class="stage-dot">·</span><span>{speech}</span><span
                class="stage-dot">·</span
              ><span
                >{app.snapshot.settings.meetingSpeakerLabels
                  ? "Speakers"
                  : "No speakers"}</span
              ></Chip
            >
          </span>
          <span class="stage-step">
            <span class="stage-arrow" aria-hidden="true">→</span>
            <Chip
              label="Notes"
              onclick={() =>
                void onnavigate({ tab: "meetings", stage: "notes" })}
              >{app.snapshot.settings.meetingNotesPreset === null
                ? "No notes"
                : `Notes · ${notes}`}</Chip
            >
          </span>
        </div>
      {/snippet}
      {#if session.dictating}<p class="field-help band-note">
          Finish dictation before recording a meeting.
        </p>{/if}
    </PipelineBand>
  {/if}
  <div class="list-toolbar">
    <SearchBar
      label="Search meetings"
      placeholder="Search titles, notes and transcripts"
      bind:value={session.search}
    />
  </div>
  {#if app.snapshot.capabilities.sync}<MeetingSyncNotice />{/if}
  {#if session.searching}<p class="field-help" role="status">
      Searching meetings…
    </p>{:else if session.searchError}<p role="alert">
      {session.searchError}
    </p>{/if}
  <div bind:this={list} class="meeting-list">
    {#each groups as group (group.key)}
      <section class="day-group" aria-label={group.label}>
        <h2 class="day-heading" data-text-surface="flat">
          {group.label}{#if group.date}<span>{group.date}</span>{/if}
        </h2>
        <div class="stock paper-card ui-card day-card notebook">
          {#each group.rows as meeting (meeting.id)}
            <div class="day-rule">
              <MeetingSlip
                match={session.searchLine(meeting.id)}
                {meeting}
                onopen={() => void open(meeting.id)}
                onexport={() => void session.exportRichText(meeting)}
              />
            </div>
          {/each}
        </div>
      </section>
    {/each}
    {#if !session.loaded}<p class="field-help" role="status">
        Loading meetings…
      </p>{:else if !listedMeetings.length && !session.searching && !session.searchError}<p
        class="empty-list"
      >
        {session.search
          ? "No matching meetings."
          : "Your recordings will appear here."}
      </p>{/if}
  </div>
  {#if app.snapshot.capabilities.sync}
    <MeetingTrash onrestored={() => session.refresh(true)} />
  {/if}
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
  .record-button {
    font-weight: 600;
  }
  .band-note {
    margin: 0;
  }
  .empty-list {
    color: var(--soft-ink);
    padding: 40px 18px;
    text-align: center;
  }
  .drop-catch {
    position: fixed;
    inset: 25% 20%;
    display: grid;
    place-items: center;
    z-index: 100;
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-blend-mode: multiply;
    border: 2px dashed var(--dye);
    border-radius: var(--r-card);
    font-size: 24px;
    box-shadow: var(--raise);
    pointer-events: none;
  }
</style>
