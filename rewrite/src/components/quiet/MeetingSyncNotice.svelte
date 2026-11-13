<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { sync, runSync, refreshSync } from "../../lib/sync.svelte";
  import { app } from "../../lib/app.svelte";
  const missing = $derived(sync.status?.meetings?.missing ?? 0);
  const result = $derived(sync.meetingResult);
  onMount(() => {
    void refreshSync();
    let active = true;
    let stop: undefined | (() => void);
    void listen("sync-changed", () => {
      void refreshSync();
    }).then((fn) => {
      if (active) stop = fn;
      else fn();
    });
    return () => {
      active = false;
      stop?.();
    };
  });
  async function resolve(deleteEverywhere: boolean) {
    if (
      deleteEverywhere &&
      !(await app.confirm(
        "Delete missing meetings everywhere?",
        "This deletes these meetings on all synced devices. Keep them on other devices if their files may only be temporarily unavailable.",
        "Delete them everywhere",
        true,
      ))
    )
      return;
    await runSync("meeting_missing_resolve", {
      delete: deleteEverywhere,
      confirmed: deleteEverywhere,
    });
  }
</script>

{#if sync.status?.signedIn}
  {#if missing > 0}
    <section aria-label="Missing meetings" data-text-surface="flat">
      <p>{missing} meetings are missing on this computer</p>
      <p class="field-help">
        Keep them on other devices unless you intended to delete them
        everywhere.
      </p>
      <button class="button" disabled={sync.busy} onclick={() => resolve(false)}
        >Keep them on other devices</button
      >
      <button class="button" disabled={sync.busy} onclick={() => resolve(true)}
        >Delete them everywhere</button
      >
    </section>
  {/if}
  {#if result}
    <p role="status" class="field-help">
      {#if result.rateLimited}Meeting sync reached the server limit. It can
        retry after {new Date(result.retryAt * 1000).toLocaleTimeString()}.
      {:else if result.errors > 0}{result.errors} meeting sync items need another
        try. Other meetings can continue syncing.
      {:else}Meeting sync finished.{/if}
    </p>
  {/if}
{/if}
