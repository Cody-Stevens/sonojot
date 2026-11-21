<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    status = "",
    live = false,
    icon,
    actions,
    stages,
    children,
  }: {
    label: string;
    status?: string;
    live?: boolean;
    icon?: Snippet;
    actions?: Snippet;
    stages?: Snippet;
    children?: Snippet;
  } = $props();
</script>

<section
  class="stock paper-card ui-card pipeline-band"
  class:live
  aria-label={label}
>
  {#if status || icon || actions}
    <div class="band-head">
      {#if icon}{@render icon()}{/if}
      {#if status}
        <span class="band-status" role="status" aria-live="polite" data-text-surface="flat"
          >{status}</span
        >
      {/if}
      {#if actions}
        <div class="band-actions">{@render actions()}</div>
      {/if}
    </div>
  {/if}
  {#if stages}
    <div class="band-stages">{@render stages()}</div>
  {/if}
  {@render children?.()}
</section>

<style>
  .pipeline-band {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px;
    margin: 0 0 18px;
    border-radius: var(--r-card-die);
    transition: box-shadow var(--motion-sheet) var(--ease-settle);
  }
  .pipeline-band.live {
    box-shadow: var(--press);
  }
  .band-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 12px;
    min-height: 36px;
  }
  .band-status {
    font-size: 18px;
    line-height: 24px;
    font-weight: 600;
    color: var(--ink);
  }
  .band-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .band-stages {
    min-width: 0;
  }
  .band-stages:empty {
    display: none;
  }
  .band-stages :global(.stage-row) {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  /* Pair each arrow with its following chip so an arrow never dangles at line end. */
  .band-stages :global(.stage-step) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    white-space: nowrap;
  }
  .band-stages :global(.stage-arrow),
  .band-stages :global(.stage-dot) {
    color: var(--soft-ink);
    flex: none;
  }
  .band-stages :global(.quiet-chip) {
    min-width: 0;
    min-height: 32px;
    padding: 5px 11px;
    border-radius: var(--r-inner);
    font-size: 14px;
    line-height: 1.2;
    gap: 6px;
  }
  @media (prefers-reduced-motion: reduce) {
    .pipeline-band {
      transition: none;
    }
  }
</style>
