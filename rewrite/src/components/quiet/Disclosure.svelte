<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../Icon.svelte";

  let {
    title,
    detail = "",
    open = $bindable(false),
    class: className = "",
    ontoggle,
    children,
  }: {
    title: string;
    detail?: string;
    open?: boolean;
    class?: string;
    ontoggle?: (open: boolean) => void;
    children: Snippet;
  } = $props();
</script>

<details
  class={`ui-disclosure ${className}`}
  bind:open
  ontoggle={(event) => ontoggle?.(event.currentTarget.open)}
>
  <summary class="disclosure-summary">
    <span class="disclosure-title" data-text-surface="emboss">{title}</span>
    {#if detail}
      <span class="disclosure-detail" data-text-surface="flat">{detail}</span>
    {/if}
    <span class="disclosure-chevron" aria-hidden="true">
      <Icon name="chevron-right" size={15} />
    </span>
  </summary>
  <div class="disclosure-body">
    {@render children()}
  </div>
</details>

<style>
  .ui-disclosure {
    margin: 0;
  }
  .disclosure-summary {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 15px 0;
    cursor: pointer;
    list-style: none;
    user-select: none;
    border-radius: var(--r-inner);
  }
  .disclosure-summary::-webkit-details-marker {
    display: none;
  }
  .disclosure-summary:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .disclosure-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--ink);
  }
  .disclosure-detail {
    font-size: 14px;
    color: var(--soft-ink);
  }
  .disclosure-chevron {
    margin-left: auto;
    display: grid;
    place-items: center;
    color: var(--soft-ink);
    transition: transform var(--motion-control) var(--ease-settle);
  }
  .ui-disclosure[open] > .disclosure-summary .disclosure-chevron {
    transform: rotate(90deg);
  }
  .disclosure-body {
    border-top: 1px solid var(--line);
    padding: 14px 0 4px;
  }
  @media (prefers-reduced-motion: reduce) {
    .disclosure-chevron {
      transition: none;
    }
  }
</style>
