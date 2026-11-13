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
  <button class="button small" aria-expanded={open} onclick={toggle}
    >Recently deleted</button
  >
  {#if open}
    <section aria-label="Recently deleted meetings">
      <p class="field-help">
        Kept for 30 days. Restore keeps a copy on this computer.
      </p>
      {#each rows as row (row.key)}
        <div class="deleted-row">
          <span>{row.title}</span>
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
      {:else}<p>No recently deleted meetings.</p>{/each}
      {#if error}<p role="alert">{error}</p>{/if}
    </section>
  {/if}
</div>

<style>
  .recently-deleted {
    margin: 16px 0;
  }
  .deleted-row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin: 12px 0;
  }
  .deleted-row > span:first-child {
    flex: 1;
    min-width: 120px;
  }
</style>
