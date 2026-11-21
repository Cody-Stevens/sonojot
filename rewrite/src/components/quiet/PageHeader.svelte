<script lang="ts" module>
  export interface BackAction {
    label: string;
    ariaLabel?: string;
    disabled?: boolean;
    onclick: () => void;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../Icon.svelte";

  let {
    title,
    back,
    actions,
    titleSlot,
    meta,
    class: className = "",
    align = "center",
  }: {
    title: string;
    back?: BackAction;
    actions?: Snippet;
    titleSlot?: Snippet;
    meta?: Snippet;
    class?: string;
    align?: "center" | "start";
  } = $props();
</script>

<header
  class={`page-header page-heading ${className}`}
  class:has-back={!!back}
  class:align-start={align === "start"}
>
  <div class="header-line">
    {#if back}
      <button
        type="button"
        class="button small header-back"
        aria-label={back.ariaLabel ?? `Back to ${back.label}`}
        disabled={back.disabled}
        onclick={back.onclick}
      >
        <Icon name="back" size={15} /><span>{back.label}</span>
      </button>
    {/if}
    <div class="header-title">
      {#if titleSlot}
        {@render titleSlot()}
      {:else}
        <h1 class="page-title" data-text-surface="emboss">{title}</h1>
      {/if}
    </div>
    {#if actions}
      <div class="header-actions">{@render actions()}</div>
    {/if}
  </div>
  {#if meta}
    <div class="header-meta" data-text-surface="flat">{@render meta()}</div>
  {/if}
</header>

<style>
  .page-header {
    margin: 0 0 18px;
  }
  .header-line {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 36px;
  }
  .align-start .header-line {
    align-items: flex-start;
  }
  .header-back {
    flex: none;
    height: 36px;
    min-height: 36px;
    padding: 0 12px 0 9px;
    gap: 6px;
    border-radius: var(--r-control);
    font-size: 15px;
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
  }
  .header-title {
    flex: 1 1 auto;
    min-width: 0;
  }
  .page-title {
    margin: 0;
    font-size: 32px;
    line-height: 36px;
    font-weight: 650;
    letter-spacing: -0.015em;
    max-width: none;
    text-wrap: balance;
    overflow-wrap: anywhere;
  }
  .header-actions {
    flex: none;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 9px;
  }
  .header-actions :global(.button) {
    min-height: 36px;
    height: 36px;
    padding: 0 13px;
    border-radius: var(--r-control);
    font-size: 15px;
    line-height: 1;
  }
  .header-actions :global(.icon-button) {
    width: 36px;
    height: 36px;
    padding: 0;
    border-radius: var(--r-control);
  }
  .header-meta {
    margin-top: 8px;
    font-size: 14px;
    line-height: 18px;
    color: var(--soft-ink);
  }
  @media (max-width: 680px) {
    .header-line {
      flex-wrap: wrap;
      gap: 10px 14px;
    }
    .header-actions {
      margin-left: auto;
    }
  }
</style>
