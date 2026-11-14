<script lang="ts">
  import { onMount } from "svelte";
  import { z } from "zod";
  import { syncAction, refreshSync } from "../lib/sync.svelte";

  // The native side also keeps asking in the background; while this card is
  // open it asks more often so the key arrives within seconds.
  const waitSchema = z.object({
    unlocked: z.boolean().optional(),
    waiting: z.boolean().optional(),
  });
  let error = $state("");
  let working = false;
  let active = false;
  let resumeAt = 0;

  async function poll() {
    if (!active || working || Date.now() < resumeAt) return;
    working = true;
    try {
      const result = waitSchema.parse(await syncAction("pair_wait"));
      if (!active) return;
      error = "";
      if (result.unlocked) await refreshSync();
    } catch (e) {
      if (!active) return;
      error = String(e);
      // Back off after a failure; waiting continues on its own.
      resumeAt = Date.now() + 20000;
    } finally {
      working = false;
    }
  }
  onMount(() => {
    active = true;
    void poll();
    const timer = setInterval(() => void poll(), 3000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });
</script>

<div class="approval" aria-label="Getting your key">
  <h2 data-text-surface="emboss">Getting your key from your other computer</h2>
  <p data-text-surface="flat">
    Keep SonoJot open on one of your other computers. This happens on its own.
  </p>
  <p class="waiting" role="status">
    <span class="spinner" aria-hidden="true"></span>Waiting…
  </p>
  {#if error}<p class="field-help" data-text-surface="flat">{error}</p>{/if}
</div>

<style>
  .approval {
    display: grid;
    gap: 18px;
  }
  h2,
  p {
    margin: 0;
  }
  .waiting {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--line);
    border-top-color: var(--ink);
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
