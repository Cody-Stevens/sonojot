<script lang="ts">
  import Icon from "../Icon.svelte";
  let {
    title,
    parent,
    onback,
    heading = false,
  }: {
    title: string;
    parent: string;
    onback: () => void;
    heading?: boolean;
  } = $props();
</script>

<header class="stage-header">
  <button
    type="button"
    class="button small"
    aria-label={`Back to ${parent}`}
    onclick={onback}
  >
    <Icon name="back" size={18} /><span>Back</span>
  </button>
  {#if heading}<div>
      <p data-text-surface="flat">{parent}</p>
      <h1 data-text-surface="emboss">{title}</h1>
    </div>{:else}<p data-text-surface="flat">
      {parent} <span aria-hidden="true">›</span>
      <strong data-text-surface="emboss">{title}</strong>
    </p>{/if}
</header>

<style>
  .stage-header {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-bottom: 27px;
  }
  p {
    margin: 0;
    color: var(--soft-ink);
    font-size: 0.9rem;
  }
  p span {
    margin-inline: 8px;
  }
  strong {
    color: var(--ink);
    font-weight: 600;
  }
  h1 {
    margin: 3px 0 0;
    font-size: 30px;
    line-height: 1.2;
  }
</style>
