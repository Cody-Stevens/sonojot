<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../../lib/app.svelte";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  import { duration } from "../../lib/protocol";
  import { meetingClock, outageLabel } from "../../lib/meeting-notices";
  import { serializeTranscript } from "../../lib/meeting-transcript";
  import type { ErrorAction } from "../../lib/errors";
  import ErrorBanner from "../ErrorBanner.svelte";
  import ProcessingBand from "./ProcessingBand.svelte";
  import MeetingMenu from "./MeetingMenu.svelte";
  import Sheet from "./Sheet.svelte";
  import { parseTranscript } from "../../lib/meeting-transcript";
  import { scrollEdges } from "../../lib/scroll-edges";
  let {
    session,
    onerroraction,
  }: {
    session: MeetingSession;
    onerroraction?: (action: ErrorAction) => void;
  } = $props();
  let menu = $state<"menu" | "formats" | "notes" | "transcribe" | null>(null);
  let anchor = $state<HTMLButtonElement | null>(null);
  let editingNotes = $state(false);
  let passage = $state<string | null>(null);
  let speaker = $state("");
  let assigning = $state<{ clock: string; from: string } | null>(null);
  let chosenSpeaker = $state("");
  let newSpeaker = $state("");
  const speakers = $derived([
    ...new Set(
      parseTranscript(session.selected?.transcript ?? "")
        .filter((block) => block.timecode && block.speaker !== "Transcript")
        .map((block) => block.speaker),
    ),
  ]);
  const detail = $derived(session.selected);
  const meetingId = $derived(session.selected?.id);
  const locked = $derived(
    session.processing ||
      session.writing ||
      session.recording ||
      app.busy("meeting-speakers"),
  );
  const notesLines = $derived(session.notes.split(/\r?\n/));
  $effect(() => {
    const id = meetingId;
    if (id) {
      editingNotes = false;
      passage = null;
      assigning = null;
    }
  });
  async function openPassage(id: string, name: string) {
    passage = id;
    speaker = name;
    await tick();
    document.getElementById(`passage-${id}`)?.focus();
  }
</script>

{#if detail}
  <header class="meeting-heading">
    <button
      class="button small"
      aria-label="All meetings"
      disabled={session.writing}
      onclick={session.back}>←</button
    >
    {#if session.renameOpen || session.processing}<input
        class="meeting-title"
        id="meeting-title"
        aria-label="Meeting title"
        placeholder="Name (optional)"
        bind:value={session.title}
        maxlength="200"
        disabled={session.writing}
      />{:else}<h1 data-text-surface="emboss">{detail.title}</h1>{/if}
    {#if session.renameOpen}<button
        class="button small"
        onclick={() => {
          session.title = detail.title;
          session.renameOpen = false;
        }}>Cancel rename</button
      >{/if}
    <button
      class="button"
      disabled={!app.native ||
        session.dirty ||
        locked ||
        app.busy("meeting-export")}
      onclick={() => session.exportMarkdown(detail)}>Export .md</button
    >
    <button
      class="button small"
      bind:this={anchor}
      aria-label="Meeting options"
      onclick={() => (menu = "menu")}>⋯</button
    >
  </header>
  <div class="meeting-meta" data-text-surface="flat">
    <span>{session.date(detail.createdAt)}</span><span
      >· {duration(detail.durationSeconds)}</span
    ><span>· {session.sourceName(detail.source)}</span
    >{#if detail.speakerCount}<span>· {detail.speakerCount} speakers</span
      >{/if}{#if session.meetings.find((row) => row.id === detail.id)?.autoNamed}<span
        class="quiet-tag">AUTO-NAMED</span
      >{/if}{#if detail.missingSeconds}<span class="quiet-tag amber"
        >MISSING {Math.round(detail.missingSeconds)} S</span
      >{/if}
  </div>
  {#if session.processing}<ProcessingBand {session} />{/if}
  {#if (detail.error || session.meetingError) && (!session.statusKey || session.statusKey !== session.hiddenStatus)}<ErrorBanner
      error={session.meetingError || detail.error}
      onaction={onerroraction}
      ondismiss={session.statusKey
        ? () => (session.hiddenStatus = session.statusKey)
        : undefined}
    />{/if}
  {#if session.audioNotice}<p
      class="field-help"
      aria-label="Missing audio notice"
    >
      {session.audioNotice}
    </p>{/if}
  {#if detail.transcriptionStoppedAtSeconds !== null || ["interrupted", "cancelled", "canceled"].includes(detail.status)}<div
      class="stock paper-card stopped-notice"
    >
      <span
        >Transcription stopped{detail.transcriptionStoppedAtSeconds !== null
          ? ` at ${meetingClock(detail.transcriptionStoppedAtSeconds)}`
          : ""}. Audio is kept.</span
      ><button
        class="button"
        disabled={session.transcribeBlocked}
        onclick={() => {
          session.openTranscribeSetup();
          menu = "transcribe";
        }}>Continue</button
      >
    </div>{/if}
  <div class="document-controls">
    <div
      class="segments"
      role="tablist"
      tabindex="-1"
      aria-label="Meeting document"
      onkeydown={session.tabKeydown}
    >
      <button
        id="meeting-tab-notes"
        role="tab"
        aria-selected={session.tab === "notes"}
        tabindex={session.tab === "notes" ? 0 : -1}
        onclick={() => (session.tab = "notes")}>Notes</button
      ><button
        id="meeting-tab-transcript"
        role="tab"
        aria-selected={session.tab === "transcript"}
        tabindex={session.tab === "transcript" ? 0 : -1}
        onclick={() => (session.tab = "transcript")}>Transcript</button
      >
    </div>
    {#if session.dirty}<button
        class="button"
        disabled={!app.native ||
          locked ||
          !session.title.trim() ||
          session.transcriptConflict}
        onclick={async () => {
          await session.saveChanges();
          if (!app.error) {
            editingNotes = false;
            passage = null;
          }
        }}>Save changes</button
      >{/if}
  </div>
  {#if session.transcriptConflict}<div class="notice" role="alert">
      <span
        >The transcript changed elsewhere. Your draft is still here; copy any
        text you want to keep before reloading.</span
      >
      <button class="button" disabled={locked} onclick={session.reloadSaved}
        >Reload saved meeting</button
      >
    </div>{/if}
  <section
    class="stock paper-card meeting-document"
    aria-label={session.tab === "notes"
      ? "Meeting notes"
      : "Meeting transcript"}
  >
    {#if session.tab === "notes"}
      {#if detail.status === "processingNotes" && !session.notes}<p
          role="status"
        >
          Writing notes from the saved transcript…
        </p>
      {:else if !session.notes && !editingNotes}<div class="empty-document">
          <h2 data-text-surface="emboss">No notes yet</h2>
          <p class="field-help">
            Write a summary, tasks or your own format from this transcript.
          </p>
          <button
            class="button"
            disabled={session.processingBlocked}
            onclick={() => {
              session.notesOpen = true;
              void session.loadAiSettings();
              menu = "notes";
            }}>Write notes</button
          >
        </div>
      {:else if editingNotes}<div class="scroll-edges-frame">
          <textarea
            id="meeting-notes"
            aria-label="Meeting notes"
            rows="15"
            bind:value={session.notes}
            disabled={locked}
            use:scrollEdges={{ frame: true }}></textarea>
        </div>
        <button
          class="text-action"
          onclick={() => {
            session.notes = detail.notes;
            editingNotes = false;
          }}>Cancel notes edit</button
        >
      {:else}<button
          class="notes-preview"
          aria-label="Edit notes"
          disabled={locked}
          onclick={async () => {
            editingNotes = true;
            await tick();
            document.getElementById("meeting-notes")?.focus();
          }}
          >{#each notesLines as line}{#if /^#{1,4}\s/.test(line)}<span
                class="notes-heading"
                data-text-surface="emboss"
                >{line.replace(/^#{1,4}\s+/, "")}</span
              >{:else if /^[-*]\s/.test(line)}<span class="notes-bullet"
                >• {line.slice(2)}</span
              >{:else}<span class="notes-line">{line || "\u00a0"}</span
              >{/if}{/each}</button
        >{/if}
      {#if session.notes}<p class="generated-notice" data-text-surface="flat">
          Generated notes may contain mistakes · select to edit
        </p>{/if}
    {:else}
      <div class="version-label" data-text-surface="flat">
        {session.showRaw
          ? "As heard · read only"
          : session.showCleaned
            ? "Cleaned transcript"
            : "Edited transcript"}
      </div>
      {#if session.showRaw}
        {#if session.rawLoading}<p role="status">
            Loading original recognition…
          </p>{:else if session.rawError}<p role="alert">{session.rawError}</p>
          <button class="button" onclick={session.viewRaw}>Try again</button
          >{:else if session.rawTranscript === null}<p class="field-help">
            Original recognition is unavailable for this older meeting.
          </p>{:else}<div class="raw-transcript">
            {session.rawTranscript}
          </div>{/if}
      {:else if !detail.hasTranscript}<div class="empty-document">
          <h2 data-text-surface="emboss">Recording saved</h2>
          <p class="field-help">
            {detail.hasAudio
              ? "Turn the saved audio into a transcript."
              : "No audio file is available for this meeting."}
          </p>
          <button
            class="button"
            disabled={session.transcribeBlocked}
            onclick={() => {
              session.openTranscribeSetup();
              menu = "transcribe";
            }}>Transcribe recording</button
          >
        </div>
      {:else if session.structured}
        {#each session.entries as entry}
          {#if entry.kind === "outage"}<div
              class="transcript-gap"
              aria-label="Transcript gap"
            >
              {outageLabel(entry.outage)}
            </div>
          {:else}{@const block = entry.block}
            <div class="transcript-passage" class:chosen={passage === block.id}>
              <div class="passage-who">
                <span
                  >{block.speaker === "Transcript" ? "" : block.speaker}</span
                >{#if block.timecode}<time>{block.timecode}</time>{/if}
              </div>
              <div class="passage-body">
                {#if passage === block.id}<textarea
                    id={`passage-${block.id}`}
                    aria-label={`Edit ${block.speaker} passage`}
                    rows="3"
                    value={block.text}
                    disabled={locked}
                    oninput={(event) => {
                      block.text = event.currentTarget.value;
                      session.transcript = serializeTranscript(
                        session.transcriptBlocks,
                      );
                    }}></textarea>
                  {#if block.speaker !== "Transcript"}<div class="speaker-edit">
                      {#if block.timecode}<button
                          class="button small"
                          disabled={session.processingBlocked || locked}
                          onclick={() => {
                            if (!block.timecode) return;
                            assigning = {
                              clock: block.timecode,
                              from: block.speaker,
                            };
                            chosenSpeaker = block.speaker;
                            newSpeaker = "";
                          }}>Assign passage…</button
                        >{/if}
                      <label for="speaker-name">Speaker name</label><input
                        id="speaker-name"
                        bind:value={speaker}
                        maxlength="48"
                        disabled={locked}
                      /><button
                        class="button small"
                        disabled={session.processingBlocked ||
                          !speaker.trim() ||
                          speaker === block.speaker}
                        onclick={async () => {
                          if (
                            await session.renameSpeaker(block.speaker, speaker)
                          )
                            passage = null;
                        }}>Rename everywhere</button
                      >
                    </div>
                    {#if session.dirty}<small class="field-help"
                        >Save text edits before changing a speaker.</small
                      >{/if}{/if}
                  <button class="text-action" onclick={() => (passage = null)}
                    >Close passage</button
                  >
                {:else}<button
                    class="passage-text"
                    disabled={locked}
                    onclick={() => void openPassage(block.id, block.speaker)}
                    >{block.text}</button
                  >{/if}
              </div>
            </div>{/if}
        {/each}
      {:else}<div class="scroll-edges-frame">
          <textarea
            id="meeting-transcript"
            aria-label="Meeting transcript"
            rows="14"
            bind:value={session.transcript}
            disabled={locked}
            use:scrollEdges={{ frame: true }}></textarea>
        </div>{/if}
      {#each session.recognitionNotices as warning}<p class="field-help">
          {warning}
        </p>{/each}
    {/if}
  </section>
  {#if menu}<MeetingMenu
      {session}
      {anchor}
      initial={menu}
      onclose={() => (menu = null)}
    />{/if}
  {#if assigning}<Sheet
      title="Assign this passage"
      onclose={() => (assigning = null)}
    >
      <p class="field-help">{assigning.clock} · {assigning.from}</p>
      {#each speakers as name}<button
          class="quiet-option"
          aria-pressed={chosenSpeaker === name}
          disabled={locked}
          onclick={() => (chosenSpeaker = name)}
          ><span class="choice-dot" aria-hidden="true"></span>{name}</button
        >{/each}
      <button
        class="quiet-option"
        aria-pressed={chosenSpeaker === ""}
        disabled={locked}
        onclick={() => (chosenSpeaker = "")}
        ><span class="choice-dot" aria-hidden="true"></span>New name…</button
      >
      {#if !chosenSpeaker}<label class="speaker-picker-name"
          >Speaker name<input
            aria-label="New speaker name"
            maxlength="48"
            bind:value={newSpeaker}
          /></label
        >{/if}
      {#if app.error}<p role="alert">{app.error}</p>{/if}
      <div class="sheet-actions">
        <button class="button" onclick={() => (assigning = null)}>Cancel</button
        >
        <button
          class="button"
          disabled={locked ||
            session.processingBlocked ||
            !(chosenSpeaker || newSpeaker.trim()) ||
            (chosenSpeaker || newSpeaker.trim()) === assigning.from}
          onclick={async () => {
            if (
              assigning &&
              (await session.reassignPassage(
                assigning.clock,
                assigning.from,
                chosenSpeaker || newSpeaker.trim(),
              ))
            ) {
              assigning = null;
              passage = null;
            }
          }}>Assign passage</button
        >
      </div>
    </Sheet>{/if}
{/if}

<style>
  .speaker-picker-name {
    display: grid;
    gap: 9px;
    margin-top: 18px;
  }
  .sheet-actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
    margin-top: 24px;
  }
  .meeting-heading {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .meeting-heading h1 {
    flex: 1;
    font-size: 27px;
    margin: 0;
    overflow-wrap: anywhere;
  }
  .meeting-title {
    flex: 1;
    min-width: 100px;
    font-size: 24px;
  }
  .meeting-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 14px;
    color: var(--soft-ink);
    margin: 12px 0 24px 50px;
  }
  .document-controls {
    display: flex;
    align-items: center;
    gap: 20px;
    justify-content: space-between;
    margin: 14px 0;
  }
  .segments {
    width: max-content;
    margin: 0;
    padding: 4px;
    border-radius: 10px;
  }
  .segments button {
    flex: none;
    padding: 9px 18px;
    font-size: 15px;
    border: 0;
    border-radius: 8px;
  }
  .segments button[aria-selected="true"] {
    background-color: var(--dye);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-blend-mode: multiply;
    color: var(--dye-ink);
    box-shadow: var(--raise-sm);
  }
  .meeting-document {
    position: relative;
    margin-top: 14px;
    padding: 24px 38px 40px 92px;
    border-radius: 5px 12px 0 0;
    min-height: max(400px, calc(100dvh - 310px));
    box-shadow: var(--raise);
  }
  .meeting-document::before {
    content: "";
    position: absolute;
    inset: 0 auto 0 66px;
    border-left: 1px solid color-mix(in srgb, var(--danger) 22%, transparent);
    pointer-events: none;
  }
  .meeting-document textarea {
    width: 100%;
    font-size: 17px;
    line-height: 1.65;
    resize: vertical;
  }
  .notes-preview {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0;
    color: var(--ink);
    background: none;
    border: 0;
    box-shadow: none;
    line-height: 1.7;
  }
  .notes-heading {
    display: block;
    font-size: 23px;
    font-weight: 600;
    margin: 10px 0 12px;
  }
  .notes-bullet {
    display: block;
    margin: 5px 0 5px 12px;
  }
  .notes-line {
    display: block;
    white-space: pre-wrap;
  }
  .generated-notice {
    font-size: 13px;
    color: var(--soft-ink);
    margin: 26px 0 0;
  }
  .transcript-passage {
    display: flex;
    gap: 24px;
    margin: 4px -12px;
    padding: 14px 12px;
    border-radius: 9px;
  }
  .transcript-passage.chosen {
    box-shadow: var(--raise);
    background: var(--well);
  }
  .passage-who {
    width: 120px;
    flex: none;
    color: var(--dye);
    font-size: 15px;
  }
  .passage-who time {
    display: block;
    font-size: 12px;
    color: var(--soft-ink);
    margin-top: 4px;
  }
  .passage-body {
    flex: 1;
    min-width: 0;
  }
  .passage-text {
    width: 100%;
    background: none;
    box-shadow: none;
    border: 0;
    color: var(--ink);
    text-align: left;
    padding: 0;
    line-height: 1.65;
    white-space: pre-wrap;
  }
  .speaker-edit {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 9px;
    font-size: 13px;
    margin: 8px 0;
  }
  .speaker-edit input {
    width: 140px;
    font-size: 15px;
  }
  .transcript-gap {
    border-block: 1px dashed var(--line);
    padding: 14px 0;
    margin: 18px 0;
    color: var(--warning-ink);
    font-size: 14px;
  }
  .version-label {
    font-size: 13px;
    color: var(--soft-ink);
    margin: 0 0 12px;
  }
  .raw-transcript {
    white-space: pre-wrap;
    line-height: 1.65;
  }
  .stopped-notice {
    display: flex;
    gap: 14px;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    border-radius: 10px;
  }
  .empty-document h2 {
    font-size: 23px;
  }
  .empty-document {
    padding: 30px 0;
  }
  .text-action {
    font-size: 14px;
    margin-top: 10px;
  }
  @media (max-width: 760px) {
    .meeting-heading {
      flex-wrap: wrap;
    }
    .meeting-document {
      padding: 24px;
    }
    .meeting-document::before {
      display: none;
    }
    .passage-who {
      width: 85px;
    }
    .transcript-passage {
      gap: 12px;
    }
  }
</style>
