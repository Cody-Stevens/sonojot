<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { z } from "zod";
  import { sync, runSync, refreshSync } from "../lib/sync.svelte";
  let { onpull }: { onpull: () => void } = $props();
  let conflicts = $state<{ id: string; heard: string; written: string }[]>([]);
  async function refresh() {
    await refreshSync();
    if (sync.status?.conflicts) {
      const r = await runSync("conflicts");
      if (r)
        conflicts = z
          .array(
            z.object({
              id: z.string(),
              heard: z.string(),
              written: z.string(),
            }),
          )
          .parse(r);
    } else conflicts = [];
  }
  onMount(() => {
    void refresh();
    let active = true;
    let unlisten: undefined | (() => void);
    void listen("sync-changed", () => {
      void refresh();
      onpull();
    }).then((fn) => {
      if (active) unlisten = fn;
      else fn();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  });
  async function now() {
    await runSync("now");
    await refresh();
    onpull();
  }
</script>

{#if sync.status?.signedIn}<div class="sync-words" aria-live="polite">
    <span data-text-surface="flat"
      >{!sync.status.unlocked
        ? "Unlock sync in Settings"
        : sync.status.lastSync
          ? `Synced ${new Date(sync.status.lastSync * 1000).toLocaleTimeString()}`
          : "Vocabulary ready to sync"}</span
    ><button
      class="button"
      disabled={sync.busy || !sync.status.unlocked}
      onclick={now}>Sync now</button
    >
  </div>{/if}
{#if sync.error && sync.status?.signedIn}<p role="alert">{sync.error}</p>{/if}
{#each conflicts as conflict}<div class="stock paper-card sync-conflict">
    <p>
      Edited on two devices: {conflict.heard}. Your version: {conflict.written}
    </p>
    <button
      class="button"
      disabled={sync.busy}
      onclick={async () => {
        await runSync("use_local", { id: conflict.id, confirmed: true });
        await now();
      }}>Use this device’s version</button
    >
  </div>{/each}

<style>
  .sync-words {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 18px;
  }
  .sync-conflict {
    padding: 18px;
    margin-bottom: 18px;
  }
</style>
