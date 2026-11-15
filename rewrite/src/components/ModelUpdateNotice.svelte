<script lang="ts">
  import { onMount } from "svelte";
  import { modelUpdates } from "../lib/model-updates.svelte";
  import {
    noticeUpdates,
    updateAllLabel,
    updateLabel,
    updateRunning,
    updateStatus,
  } from "../lib/model-updates";
  /** `page`: the Models page, where Later is not offered. */
  let { place = "window" }: { place?: "window" | "page" } = $props();
  onMount(() => modelUpdates.acquire());
  const shown = $derived(
    noticeUpdates(modelUpdates.list).filter(
      (update) =>
        place === "page" ||
        updateRunning(update) ||
        !modelUpdates.dismissed.includes(update.id),
    ),
  );
  const waiting = $derived(
    shown.filter((update) => update.state === "available"),
  );
  const single = $derived(shown.length === 1 ? shown[0] : null);
</script>

{#if single}<div class="notice model-update" role="status">
    <span>{updateStatus(single)}</span>
    <div class="actions">
      {#if single.state === "available"}<button
          class="button"
          onclick={() => modelUpdates.start(single.id)}
          >{updateLabel(single)}</button
        >{:else if single.state === "failed"}<button
          class="button"
          onclick={() => modelUpdates.start(single.id)}>Try again</button
        >{:else if single.state === "downloading" || single.state === "queued"}<button
          class="button"
          onclick={() => modelUpdates.cancel(single.id)}>Cancel</button
        >{/if}
      {#if place === "window" && !updateRunning(single)}<button
          class="button"
          onclick={() => modelUpdates.dismiss([single.id])}
          >{single.state === "failed" ? "Close" : "Later"}</button
        >{/if}
    </div>
  </div>{:else if shown.length > 1}<div class="notice model-update" role="status">
    <span
      >{waiting.length > 1
        ? `Updates for ${waiting.length} models: ${waiting.map((update) => update.name).join(", ")}.`
        : shown.map(updateStatus).join(" ")}</span
    >
    <div class="actions">
      {#if waiting.length > 1}<button
          class="button"
          onclick={() => modelUpdates.start()}
          >{updateAllLabel(waiting)}</button
        >{/if}
      {#if place === "window" && !shown.some(updateRunning)}<button
          class="button"
          onclick={() => modelUpdates.dismiss(shown.map((update) => update.id))}
          >Later</button
        >{/if}
    </div>
  </div>{/if}

<style>
  .model-update {
    align-items: center;
    flex-wrap: wrap;
  }
  .model-update > span {
    flex: 1 1 260px;
  }
  .actions {
    display: flex;
    gap: 9px;
    flex-wrap: wrap;
  }
</style>
