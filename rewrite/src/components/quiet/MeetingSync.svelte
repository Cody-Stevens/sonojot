<script lang="ts">
  import Icon from "../Icon.svelte";
  import {
    syncGlyph,
    type MeetingSyncStatus,
  } from "../../lib/meeting-sync-status.svelte";
  let {
    meetingSync,
    hasAudio,
    disabled = false,
  }: {
    meetingSync: MeetingSyncStatus;
    hasAudio: boolean;
    disabled?: boolean;
  } = $props();
  const status = $derived(meetingSync.status);
  const downloading = $derived(meetingSync.busy === "meeting_download");
  const glyph = $derived(syncGlyph(status, downloading));
  const busy = $derived(meetingSync.busy !== null);
  const upload = (enabled: boolean | null) =>
    meetingSync.act("meeting_audio", { enabled });
</script>

<div class="meeting-sync" aria-label="Meeting sync">
  <p class="sync-state" aria-live="polite">
    {#if glyph}<Icon name={glyph.icon} size={20} />{/if}<span
      >{glyph?.label ?? "Checking sync…"}</span
    >
  </p>
  {#if status}
    {#if status.audioAvailable && !hasAudio}<div class="sync-row">
        <span data-text-surface="flat">The recording isn’t on this computer yet.</span>
        <button
          class="button small"
          disabled={disabled || busy}
          onclick={() => meetingSync.act("meeting_download")}
          >{downloading ? "Downloading…" : "Download recording"}</button
        >
      </div>{/if}
    <div class="sync-field">
      <span>Upload recording</span>
      <div
        class="segments"
        role="group"
        aria-label="Upload recording for this meeting"
      >
        <button
          aria-pressed={status.audioOverride === null}
          disabled={disabled || busy}
          onclick={() => upload(null)}
          >{status.audioOverride === null
            ? `Default (${status.audioEnabled ? "on" : "off"})`
            : "Default"}</button
        ><button
          aria-pressed={status.audioOverride === true}
          disabled={disabled || busy}
          onclick={() => upload(true)}>On</button
        ><button
          aria-pressed={status.audioOverride === false}
          disabled={disabled || busy}
          onclick={() => upload(false)}>Off</button
        >
      </div>
    </div>
    {#if status.conflicts.length}
      <details open>
        <summary>Edited on two devices ({status.conflicts.length})</summary>
        {#each status.conflicts as conflict (conflict.version)}<div
            class="conflict"
          >
            <p>{conflict.text}</p>
            <button
              class="button small"
              disabled={disabled || busy}
              onclick={() =>
                meetingSync.act("meeting_resolve", {
                  sid: conflict.sid,
                  view: conflict.view,
                  version: conflict.version,
                })}>Use the other version</button
            >
          </div>{/each}
      </details>
    {/if}
    {#if status.fieldConflicts && Object.keys(status.fieldConflicts).length}
      <details open>
        <summary>Meeting details changed on two devices</summary>
        <p>The synced version is shown. Your other values are kept here.</p>
        {#each Object.entries(status.fieldConflicts) as [field, value]}<div
            class="conflict"
          >
            <p>{field.split("/").at(-1)}: {JSON.stringify(value)}</p>
            <button
              class="button small"
              disabled={disabled || busy}
              onclick={() =>
                meetingSync.act("meeting_field_resolve", {
                  field,
                  useOther: true,
                })}>Use this value</button
            ><button
              class="button small"
              disabled={disabled || busy}
              onclick={() =>
                meetingSync.act("meeting_field_resolve", {
                  field,
                  useOther: false,
                })}>Keep the shown value</button
            >
          </div>{/each}
      </details>
    {/if}
    {#if status.deleteConflict}<p>
        This meeting is no longer available from sync. A local copy is kept on
        this computer.
      </p>
      <div class="sync-actions">
        <button
          class="button small"
          disabled={disabled || busy}
          onclick={() =>
            meetingSync.act("meeting_delete_resolve", { keep: true })}
          >Keep on this computer</button
        ><button
          class="button small danger"
          disabled={disabled || busy}
          onclick={() =>
            meetingSync.act("meeting_delete_resolve", { keep: false })}
          >Delete</button
        >
      </div>{/if}
  {/if}
  {#if meetingSync.error}<p role="alert">{meetingSync.error}</p>
    <button class="button small" disabled={busy} onclick={meetingSync.reload}
      >Retry</button
    >{/if}
</div>

<style>
  .meeting-sync {
    display: grid;
    gap: 14px;
    padding: 6px 8px 8px;
    min-width: 300px;
  }
  .sync-state {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 0;
    color: var(--ink);
  }
  .sync-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 9px 14px;
    font-size: 15px;
    color: var(--soft-ink);
  }
  .sync-field {
    display: grid;
    gap: 8px;
  }
  .segments {
    font-size: 14px;
  }
  .segments button {
    font-size: 14px;
    padding: 9px 10px;
  }
  .sync-actions {
    display: flex;
    gap: 9px;
  }
  .conflict {
    margin: 9px 0;
  }
  .conflict p {
    white-space: pre-wrap;
    color: var(--ink);
  }
</style>
