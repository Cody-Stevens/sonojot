<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    children,
    selected = false,
  }: { children: Snippet; selected?: boolean } = $props();
</script>

<div class="quiet-row-actions" class:selected>{@render children()}</div>

<style>
  /* The owner row uses data-quiet-row; opacity leaves actions in the tab order. */
  .quiet-row-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--motion-control);
  }
  .quiet-row-actions.selected,
  .quiet-row-actions:focus-within,
  :global([data-quiet-row]:hover) .quiet-row-actions,
  :global([data-quiet-row]:focus-within) .quiet-row-actions,
  :global([data-quiet-row][aria-selected="true"]) .quiet-row-actions {
    opacity: 1;
    pointer-events: auto;
  }
  @media (hover: none) {
    .quiet-row-actions {
      opacity: 1;
      pointer-events: auto;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .quiet-row-actions {
      transition: none;
    }
  }
</style>
