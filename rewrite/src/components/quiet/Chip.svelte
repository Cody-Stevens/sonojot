<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    children,
    onclick,
    label,
    selected,
    disabled = false,
  }: {
    children: Snippet;
    onclick: () => void;
    label?: string;
    selected?: boolean;
    disabled?: boolean;
  } = $props();
</script>

<button
  type="button"
  class="button quiet-chip"
  class:primary={selected === true}
  aria-label={label}
  aria-pressed={selected}
  {disabled}
  {onclick}
>
  {@render children()}
</button>

<style>
  .quiet-chip {
    min-height: 38px;
    padding: 6px 12px;
    gap: 8px;
    font-size: 0.85rem;
  }
</style>
