<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { z } from "zod";
  import { listen } from "@tauri-apps/api/event";
  import { sync, syncAction, refreshSync } from "../../lib/sync.svelte";
  import { app } from "../../lib/app.svelte";
  const schema = z.object({
    status: z.string(),
    audioOverride: z.boolean().nullable(),
    audioEnabled: z.boolean(),
    audioAvailable: z.boolean(),
    deleteConflict: z.boolean(),
    conflicts: z.array(
      z.object({
        sid: z.string(),
        view: z.string(),
        version: z.string(),
        text: z.string(),
      }),
    ),
    fieldConflicts: z.record(z.string(), z.unknown()).nullable(),
  });
  let {
    id,
    disabled = false,
    onchanged,
  }: {
    id: string;
    disabled?: boolean;
    onchanged: () => Promise<void>;
  } = $props();
  let status = $state<z.infer<typeof schema> | null>(null);
  let error = $state("");
  let busy = $state(false);
  let request = 0;
  const labels: Record<string, string> = {
    synced: "Synced",
    pending: "Waiting to sync",
    storageFull: "Not synced (storage full)",
    deletedElsewhere: "Sync copy unavailable",
    localOnly: "Kept on this computer",
    deleted: "Deleted",
  };
  async function reload() {
    const current = ++request;
    const meeting = id;
    try {
      const result = schema.parse(
        await syncAction("meeting_status", { id: meeting }),
      );
      if (current === request && meeting === id) status = result;
    } catch (e) {
      if (current === request) error = String(e);
    }
  }
  $effect(() => {
    const meeting = id;
    const revision = app.meetingRevision;
    const synced = sync.status?.lastSync;
    untrack(() => {
      status = null;
      error = "";
      if (sync.status?.unlocked) void reload();
    });
  });
  onMount(() => {
    void refreshSync();
    let active = true;
    let dispose: (() => void) | undefined;
    void listen("sync-changed", () => {
      void refreshSync().then(() => reload());
    }).then((fn) => {
      if (active) dispose = fn;
      else fn();
    });
    return () => {
      active = false;
      request++;
      dispose?.();
    };
  });
  async function act(operation: string, args: Record<string, unknown> = {}) {
    busy = true;
    error = "";
    try {
      await syncAction(operation, { id, ...args });
      await onchanged();
      await reload();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if app.snapshot.capabilities.sync && sync.status?.unlocked}
  <div class="meeting-sync" data-text-surface="flat" aria-label="Meeting sync">
    <span aria-live="polite"
      >{busy
        ? "Updating meeting sync…"
        : status
          ? (labels[status.status] ?? "Waiting to sync")
          : "Checking sync…"}</span
    >
    {#if status}
      <label
        >Upload audio
        <select
          aria-label="Upload audio for this meeting"
          disabled={disabled || busy}
          value={status.audioOverride === null
            ? "default"
            : status.audioOverride
              ? "on"
              : "off"}
          onchange={(event) =>
            act("meeting_audio", {
              enabled:
                event.currentTarget.value === "default"
                  ? null
                  : event.currentTarget.value === "on",
            })}
        >
          <option value="default"
            >Default ({status.audioEnabled ? "on" : "off"})</option
          ><option value="on">On</option><option value="off">Off</option>
        </select>
      </label>
      {#if status.audioAvailable}<button
          class="button small"
          disabled={disabled || busy}
          onclick={() => act("meeting_download")}
          >Download audio to play, export or transcribe</button
        >{/if}
      {#if status.conflicts.length}
        <details>
          <summary>Edited on two devices ({status.conflicts.length})</summary>
          {#each status.conflicts as conflict (conflict.version)}<div
              class="conflict"
            >
              <p>{conflict.text}</p>
              <button
                class="button small"
                disabled={disabled || busy}
                onclick={() =>
                  act("meeting_resolve", {
                    sid: conflict.sid,
                    view: conflict.view,
                    version: conflict.version,
                  })}>Use the other version</button
              >
            </div>{/each}
        </details>
      {/if}
      {#if status.fieldConflicts && Object.keys(status.fieldConflicts).length}
        <details>
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
                  act("meeting_field_resolve", { field, useOther: true })}
                >Use this value</button
              ><button
                class="button small"
                disabled={disabled || busy}
                onclick={() =>
                  act("meeting_field_resolve", { field, useOther: false })}
                >Keep the shown value</button
              >
            </div>{/each}
        </details>
      {/if}
      {#if status.deleteConflict}<p>
          This meeting is no longer available from sync. A local copy is kept on
          this computer.
        </p>
        <button
          class="button small"
          disabled={disabled || busy}
          onclick={() => act("meeting_delete_resolve", { keep: true })}
          >Keep on this computer</button
        ><button
          class="button small danger"
          disabled={disabled || busy}
          onclick={() => act("meeting_delete_resolve", { keep: false })}
          >Delete</button
        >{/if}
    {/if}
    {#if error}<p role="alert">{error}</p>
      <button class="button small" disabled={busy} onclick={reload}
        >Retry</button
      >{/if}
  </div>
{/if}

<style>
  .meeting-sync {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 9px 18px;
    font-size: 0.85rem;
    margin: 9px 0 18px;
    color: var(--ink-muted);
  }
  label {
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 9px;
  }
  select {
    max-width: 145px;
  }
  .conflict {
    margin: 9px 0;
  }
  details,
  p[role="alert"] {
    flex-basis: 100%;
  }
  .conflict p {
    white-space: pre-wrap;
    color: var(--ink);
  }
</style>
