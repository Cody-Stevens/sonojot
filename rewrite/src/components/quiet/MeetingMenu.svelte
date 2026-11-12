<script lang="ts">
  import {
    instructionLength,
    notesInstructionsError,
    NOTES_INSTRUCTIONS_LIMIT,
  } from "../../lib/notes-instructions";
  import { untrack } from "svelte";
  import Sheet from "./Sheet.svelte";
  import Toggle from "../Toggle.svelte";
  import MeetingAudioExport from "../MeetingAudioExport.svelte";
  import RemoteModelBadge from "../RemoteModelBadge.svelte";
  import { app } from "../../lib/app.svelte";
  import { meetingBusy } from "../../lib/meeting-display";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  let {
    session,
    anchor,
    initial = "menu",
    onclose,
  }: {
    session: MeetingSession;
    anchor: HTMLElement | null;
    initial?: "menu" | "formats" | "notes" | "transcribe";
    onclose: () => void;
  } = $props();
  let pane = $state<
    "menu" | "formats" | "notes" | "transcribe" | "join" | "versions"
  >(untrack(() => initial));
  let joins = $state<string[]>(
    untrack(() => (session.selected ? [session.selected.id] : [])),
  );
  const detail = $derived(session.selected);
  const title = $derived(
    {
      menu: "Meeting",
      formats: "Other formats",
      notes: "Write notes",
      transcribe: "Transcribe again",
      join: "Join in this order",
      versions: "Transcript version",
    }[pane],
  );
  function change(next: typeof pane) {
    pane = next;
    if (next === "transcribe") session.openTranscribeSetup();
    if (next === "notes") {
      session.notesOpen = true;
      void session.loadAiSettings();
    }
  }
  function close() {
    session.transcribeSetup = false;
    session.notesOpen = false;
    onclose();
  }
</script>

{#if detail}<Sheet {title} {anchor} onclose={close}>
    {#if pane === "menu"}
      <button
        class="quiet-option"
        disabled={session.writing || session.processing || session.recording}
        onclick={() => {
          session.renameOpen = true;
          close();
        }}>Rename</button
      >
      <button class="quiet-option" onclick={() => change("formats")}
        >Other formats…</button
      >
      {#if session.tab === "transcript"}<button
          class="quiet-option"
          onclick={() => change("versions")}>Transcript version…</button
        >
      {:else}<button
          class="quiet-option"
          disabled={session.processingBlocked}
          onclick={() => change("notes")}>Rewrite notes…</button
        >{/if}
      <button
        class="quiet-option"
        disabled={session.transcribeBlocked}
        onclick={() => change("transcribe")}>Transcribe again…</button
      >
      {#if detail.speakerCount >= 2}<div class="speaker-toggle">
          <span>Speaker names</span><Toggle
            label="Speaker names"
            checked={detail.speakerLabels}
            disabled={session.processingBlocked}
            onchange={session.setSpeakerLabels}
          />
        </div>{/if}
      <button
        class="quiet-option"
        disabled={session.dirty ||
          session.recording ||
          session.processing ||
          session.writing}
        onclick={() => change("join")}>Join with other meetings…</button
      >
      <button
        class="quiet-option danger-text"
        disabled={!app.native ||
          session.recording ||
          session.processing ||
          session.writing ||
          app.busy("meeting-speakers")}
        onclick={async () => {
          close();
          await session.remove();
        }}>Delete…</button
      >
    {:else if pane === "versions"}
      <div class="version-field">
        <span>Version</span>
        <div class="segments" role="group" aria-label="Transcript version">
          <button
            aria-pressed={!session.showRaw && !session.showCleaned}
            onclick={() => {
              session.switchTranscript(false);
              close();
            }}>Edited</button
          ><button
            aria-pressed={!session.showRaw && session.showCleaned}
            disabled={detail.cleanedTranscript === null}
            onclick={() => {
              session.switchTranscript(true);
              close();
            }}>Cleaned</button
          ><button
            aria-pressed={session.showRaw}
            onclick={() => {
              void session.viewRaw();
              close();
            }}>As heard</button
          >
        </div>
      </div>
      {#if detail.cleanedTranscript === null}<button
          class="quiet-option"
          disabled={session.processingBlocked}
          onclick={async () => {
            await session.cleanTranscript();
            if (!app.error) close();
          }}>Clean transcript</button
        >{/if}
    {:else if pane === "formats"}
      <label class="field-label" for="document-format">Document</label><select
        id="document-format"
        bind:value={session.exportFormat}
        ><option value="markdown">Markdown (.md)</option><option value="text"
          >Plain text (.txt)</option
        ><option value="html">Web page (.html)</option><option value="pdf"
          >PDF (.pdf)</option
        ></select
      >
      <p class="field-help">
        Includes the {session.showCleaned ? "cleaned" : "edited"} transcript and saved
        notes.
      </p>
      <button
        class="button"
        disabled={!app.native ||
          session.dirty ||
          session.recording ||
          session.processing ||
          session.writing ||
          app.busy("meeting-export")}
        onclick={session.exportFile}>Export</button
      >
      {#if session.dirty}<p class="field-help">
          Save your changes before exporting the document.
        </p>{/if}
      <hr />
      <MeetingAudioExport
        meetingId={detail.id}
        title={detail.title}
        status={detail.status}
        hasAudio={detail.hasAudio}
        disabled={detail.status === "finalizing"}
      />
    {:else if pane === "notes"}
      <label class="field-label" for="note-preset">Style</label><select
        id="note-preset"
        bind:value={session.notePreset}
        >{#each session.notePresets as preset}<option value={preset.id}
            >{preset.label.replace(" (AI)", "")}</option
          >{/each}</select
      >
      <label class="field-label spaced" for="note-instructions"
        >{session.notePreset === "custom"
          ? "Your instructions"
          : "Extra instructions (optional)"}</label
      ><textarea
        id="note-instructions"
        rows="3"
        bind:value={session.instructions}
        disabled={session.notePreset === "formattedTranscript"}></textarea>
      {#if session.notePreset !== "formattedTranscript"}<small
          class="field-help"
          >{instructionLength(session.instructions)} / {NOTES_INSTRUCTIONS_LIMIT}
          characters</small
        >
        {#if notesInstructionsError(session.notePreset, session.instructions)}<p
            class="field-help"
            role="alert"
          >
            {notesInstructionsError(session.notePreset, session.instructions)}
          </p>{/if}
      {/if}
      <p class="field-help">
        {session.aiSettings?.backend === "openrouter"
          ? session.aiSettings.model
          : session.modelName(app.snapshot.settings.notesModel)}
        {#if session.aiSettings?.backend === "openrouter"}<RemoteModelBadge
          />{/if}
      </p>
      <button
        class="button primary"
        disabled={session.processingBlocked ||
          (session.notePreset !== "formattedTranscript" &&
            !!notesInstructionsError(session.notePreset, session.instructions))}
        onclick={async () => {
          await session.generateNotes();
          if (!app.error) close();
        }}>Write notes</button
      >
    {:else if pane === "transcribe"}
      <p class="field-help">
        Replaces the edited and cleaned transcript. Notes stay saved. Speaker
        names can change with a new recognition.
      </p>
      <label class="field-label" for="transcription-model">Speech model</label
      ><select
        id="transcription-model"
        value={session.transcriptionModelAvailable
          ? session.transcriptionModel
          : ""}
        onchange={(event) =>
          (session.transcriptionOverride = event.currentTarget.value)}
      >
        {#if !session.transcriptionModelAvailable}<option value="" disabled
            >Selected model unavailable</option
          >{/if}
        {#each session.speechModels as model}<option value={model.id}
            >{model.name}</option
          >{/each}
        {#if session.remoteSpeechModel}<option value="openrouter"
            >OpenRouter · {session.remoteSpeechName}</option
          >{/if}
      </select>
      {#if session.transcriptionModel === "openrouter"}<RemoteModelBadge />{/if}
      {#if session.transcriptionUnavailableReason}<p class="field-help">
          {session.transcriptionUnavailableReason}
        </p>{/if}
      {#if detail.transcriptionModel}<p class="field-help">
          Last used: {session.modelName(detail.transcriptionModel)}
        </p>{/if}
      <div class="actions spaced">
        <button
          class="button primary"
          disabled={session.transcribeBlocked ||
            !session.transcriptionModelAvailable}
          onclick={() => {
            void session.transcribe();
            close();
          }}>Transcribe</button
        >
      </div>
    {:else if pane === "join"}
      <p class="field-help">
        Creates a new meeting. The original recordings stay saved.
      </p>
      {#each session.meetings as row}<label class="join-row"
          ><input
            type="checkbox"
            checked={joins.includes(row.id)}
            disabled={row.id === detail.id ||
              !row.hasAudio ||
              meetingBusy(row.status)}
            onchange={(event) =>
              (joins = event.currentTarget.checked
                ? [...joins, row.id]
                : joins.filter((id) => id !== row.id))}
          /><span
            >{joins.includes(row.id)
              ? `${joins.indexOf(row.id) + 1}. `
              : ""}{row.title}</span
          ></label
        >{/each}
      <button
        class="button primary"
        disabled={joins.length < 2 ||
          session.dirty ||
          session.processing ||
          app.busy("meeting-join")}
        onclick={async () => {
          if (await session.join(joins)) close();
        }}>Join {joins.length}</button
      >
    {/if}
    {#if app.error}<p role="alert" class="field-help">{app.error}</p>{/if}
  </Sheet>{/if}

<style>
  .version-field {
    padding: 12px 8px;
  }
  .version-field > span {
    display: block;
    margin-bottom: 8px;
  }
  .segments {
    font-size: 14px;
  }
  .speaker-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 8px;
    border-bottom: 1px solid var(--line);
  }
  select,
  textarea {
    width: 100%;
  }
  .join-row {
    display: flex;
    gap: 12px;
    align-items: center;
    margin: 16px 0;
    font-size: 16px;
  }
  .join-row input[type="checkbox"] {
    flex: none;
    width: 20px;
    height: 20px;
    margin: 0;
  }
  .segments button {
    font-size: 14px;
    padding: 9px 8px;
  }
  .quiet-option {
    font-size: 16px;
    min-height: 43px;
  }
</style>
