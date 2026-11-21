<script lang="ts">
  import { call } from "../../lib/api";
  import { app } from "../../lib/app.svelte";
  import type { CommandResult } from "../../lib/protocol";
  let { onrestored }: { onrestored: () => Promise<void> } = $props();
  let open = $state(false),
    busy = $state(false),
    error = $state("");
  let rows = $state<CommandResult<"list_meeting_trash">>([]);
  async function reload() {
    try {
      rows = await call("list_meeting_trash");
    } catch (e) {
      error = String(e);
    }
  }
  async function toggle() {
    open = !open;
    if (open) {
      error = "";
      await reload();
    }
  }
  async function change(key: string, restore: boolean) {
    if (
      !restore &&
      !(await app.confirm(
        "Delete this saved copy now?",
        "Its files on this computer will be permanently removed.",
        "Delete now",
        true,
      ))
    )
      return;
    busy = true;
    error = "";
    try {
      if (restore) {
        await call("restore_meeting_trash", { key });
        await onrestored();
      } else await call("delete_meeting_trash", { key });
      await reload();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="recently-deleted" data-text-surface="flat">
  <div class="list-footer">
    <span>Copies deleted on another device are kept here for 30 days.</span>
    <button
      type="button"
      class="text-action"
      aria-expanded={open}
      onclick={toggle}>Recently deleted</button
    >
  </div>
  {#if open}
    <section
      class="stock paper-card ui-card trash-card"
      aria-label="Recently deleted meetings"
    >
      <p class="field-help">
        Kept for 30 days. Restore keeps a copy on this computer.
      </p>
      {#each rows as row (row.key)}
        <div class="deleted-row">
          <span class="deleted-title">{row.title}</span>
          <span class="field-help"
            >{new Date(row.deletedAt * 1000).toLocaleDateString()}</span
          >
          <button
            class="button small"
            disabled={busy}
            onclick={() => change(row.key, true)}>Restore</button
          >
          <button
            class="button small"
            disabled={busy}
            onclick={() => change(row.key, false)}>Delete now</button
          >
        </div>
      {:else}<p class="field-help">No recently deleted meetings.</p>{/each}
      {#if error}<p role="alert">{error}</p>{/if}
    </section>
  {/if}
</div>

<style>
  .recently-deleted {
    margin-top: 16px;
  }
  .trash-card {
    margin-top: 12px;
    padding: 16px 20px;
  }
  .trash-card > p:first-child {
    margin: 0 0 12px;
  }
  .deleted-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    padding: 10px 0;
    border-top: 1px solid var(--line);
  }
  .deleted-title {
    flex: 1 1 140px;
    color: var(--ink);
  }
</style>
