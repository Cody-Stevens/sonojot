<script lang="ts">
  import type { Snippet } from "svelte";
  let {
    title,
    actions,
    children,
    class: className = "",
  }: {
    title?: string;
    actions?: Snippet;
    children: Snippet;
    class?: string;
  } = $props();
  const id = $props.id();
</script>

<section
  class={`stock paper-card ${className}`}
  aria-labelledby={title ? `${id}-heading` : undefined}
>
  {#if title}<div class="card-heading">
      <h2 data-text-surface="emboss" id={`${id}-heading`}>{title}</h2>
      {#if actions}{@render actions()}{/if}
    </div>{/if}
  {@render children()}
</section>

<style>
  .card-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
  }
  .card-heading h2 {
    min-width: 0;
  }
</style>
