<script lang="ts">
  import type { Snippet } from "svelte";
  let { children }: { children: Snippet } = $props();
</script>

<div class="quiet-well">{@render children()}</div>

<style>
  .quiet-well {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 14px;
    background-color: var(--well);
    box-shadow: var(--press);
    border-radius: 13px 12px 14px 12px;
  }
  .quiet-well :global(input),
  .quiet-well :global(textarea) {
    flex: 1;
    border: 0;
    background: transparent;
    box-shadow: none;
    min-width: 0;
    padding: 0;
  }
</style>
