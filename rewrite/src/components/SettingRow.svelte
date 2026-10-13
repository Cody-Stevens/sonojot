<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    label,
    help,
    children,
    detail,
  }: { label: string; help?: string; children: Snippet; detail?: Snippet } =
    $props();
  const id = $props.id();
</script>

<div
  class="paper-row"
  role="group"
  aria-labelledby={`${id}-label`}
  aria-describedby={help ? `${id}-help` : undefined}
>
  <div class="row-copy">
    <span id={`${id}-label`}>{label}</span>{#if help}<small
        id={`${id}-help`}
        data-text-surface="flat">{help}</small
      >{/if}
    {@render detail?.()}
  </div>
  <div class="row-control">{@render children()}</div>
</div>
