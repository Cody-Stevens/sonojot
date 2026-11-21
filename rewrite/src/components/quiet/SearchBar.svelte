<script lang="ts">
  import Icon from "../Icon.svelte";

  let {
    label,
    placeholder = label,
    value = $bindable(""),
    disabled = false,
  }: {
    label: string;
    placeholder?: string;
    value?: string;
    disabled?: boolean;
  } = $props();

  let inputEl: HTMLInputElement;
</script>

<div class="search-bar" class:disabled>
  <span class="search-icon" aria-hidden="true"><Icon name="search" size={16} /></span>
  <input
    bind:this={inputEl}
    type="search"
    aria-label={label}
    {placeholder}
    bind:value
    {disabled}
  />
  {#if value}
    <button
      type="button"
      class="search-clear"
      aria-label="Clear search"
      {disabled}
      onclick={() => {
        value = "";
        inputEl?.focus();
      }}
    >
      <Icon name="close" size={14} />
    </button>
  {/if}
</div>

<style>
  .search-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 44px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background-color: var(--well);
    box-shadow: var(--press);
    color: var(--ink);
  }
  .search-bar:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .search-bar.disabled {
    opacity: 0.6;
  }
  .search-icon {
    display: grid;
    place-items: center;
    color: var(--soft-ink);
    flex: none;
  }
  input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    color: var(--ink);
    font-size: 16px;
    line-height: 1.3;
  }
  input:focus-visible {
    outline: none;
  }
  input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }
  .search-clear {
    display: grid;
    place-items: center;
    flex: none;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--soft-ink);
    transition:
      color var(--motion-control) linear,
      background-color var(--motion-control) linear;
  }
  .search-clear:hover:not(:disabled) {
    color: var(--ink);
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    box-shadow: var(--raise-sm);
  }
  .search-clear:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  @media (prefers-reduced-motion: reduce) {
    .search-clear {
      transition: none;
    }
  }
</style>
