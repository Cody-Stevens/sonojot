<script lang="ts">
  import { call, native } from "../lib/api";
  import { app } from "../lib/app.svelte";
  import { modelLicense } from "../lib/model-licenses";

  let { id }: { id: string } = $props();
  const license = $derived(modelLicense(id));

  function openLicense(event: MouseEvent): void {
    if (!native) return;
    event.preventDefault();
    void app.perform("model-license", () => call("open_model_license", { id }));
  }
</script>

{#if id !== "local-rules"}
  <p class="model-license" data-text-surface="flat">
    {#if license}
      {license.name} ·
      <a
        href={license.url}
        onclick={openLicense}
        target="_blank"
        rel="noopener noreferrer">License details ↗</a
      >
    {:else}
      License unverified
    {/if}
  </p>
  {#if id === "lfm2.5-1.2b" || id === "lfm2.5-2.6b"}
    <p class="model-license" data-text-surface="flat">
      Optional. Commercial use depends on your legal entity's revenue under the
      LFM Open License, with a US$10 million threshold. Confirm eligibility with
      LiquidAI at or above that amount. Review and acknowledge the terms before
      downloading.
    </p>
  {/if}
{/if}

<style>
  .model-license {
    margin: 9px 0 0;
    color: var(--soft-ink);
  }
  a {
    color: inherit;
    text-decoration: underline;
    text-underline-offset: 4px;
  }
</style>
