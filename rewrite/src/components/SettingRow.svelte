<script lang="ts">
  import type { Snippet } from "svelte";
  import InfoTip from "./quiet/InfoTip.svelte";

  let {
    label,
    help,
    layout = "action",
    forId,
    children,
    detail,
    info,
    labelExtra,
  }: {
    label: string;
    help?: string;
    /** "action": binary toggle or single button right-aligned (1fr auto).
     *  "choice": pickers, segments, shortcut wells, textareas (200px 1fr, stacks <760px). */
    layout?: "action" | "choice";
    forId?: string;
    children: Snippet;
    detail?: Snippet;
    /** Longer explanation behind an information button beside the last word of the label. */
    info?: Snippet;
    /** Optional inline badge/stamp after the label. */
    labelExtra?: Snippet;
  } = $props();

  const id = $props.id();
  // Keep the info button bound to the final word so it never orphans onto a line alone.
  const split = $derived.by(() => {
    const trimmed = label.trim();
    const index = trimmed.lastIndexOf(" ");
    return index < 0
      ? { head: "", tail: trimmed }
      : { head: trimmed.slice(0, index + 1), tail: trimmed.slice(index + 1) };
  });
</script>

<div
  class="paper-row setting-row"
  class:setting-choice={layout === "choice"}
  class:setting-action={layout === "action"}
  role={forId ? undefined : "group"}
  aria-labelledby={forId ? undefined : `${id}-label`}
  aria-describedby={forId || !help ? undefined : `${id}-help`}
>
  <div class="row-copy">
    {#if forId}
      <label class="row-label" id={`${id}-label`} for={forId}>
        {#if info}{split.head}<span class="label-tail"
            >{split.tail}<InfoTip label={`About ${label}`}
              >{@render info()}</InfoTip
            ></span
          >{:else}{label}{/if}{#if labelExtra}<span class="label-extra"
            >{@render labelExtra()}</span
          >{/if}
      </label>
    {:else}
      <span class="row-label" id={`${id}-label`}>
        {#if info}{split.head}<span class="label-tail"
            >{split.tail}<InfoTip label={`About ${label}`}
              >{@render info()}</InfoTip
            ></span
          >{:else}{label}{/if}{#if labelExtra}<span class="label-extra"
            >{@render labelExtra()}</span
          >{/if}
      </span>
    {/if}
    {#if help}<small id={`${id}-help`} class="row-help" data-text-surface="flat"
        >{help}</small
      >{/if}
    {@render detail?.()}
  </div>
  <div class="row-control">{@render children()}</div>
</div>

<style>
  :global(.ui-card) .setting-row {
    display: grid;
    align-items: center;
    gap: 12px 18px;
    padding: 18px 0;
    border-bottom: 0;
    box-shadow: none;
  }
  :global(.ui-card) .setting-action {
    grid-template-columns: minmax(0, 1fr) auto;
  }
  :global(.ui-card) .setting-choice {
    grid-template-columns: 200px minmax(0, 1fr);
    align-items: start;
  }
  :global(.ui-card) .setting-row .row-copy {
    min-width: 0;
    flex: unset;
    font-size: 17px;
    line-height: 1.35;
  }
  :global(.ui-card) .setting-choice .row-copy {
    padding-top: 6px;
  }
  :global(.ui-card) .row-label {
    font-size: 17px;
    line-height: 1.35;
    font-weight: 500;
    color: var(--ink);
  }
  .label-tail {
    white-space: nowrap;
  }
  .label-extra {
    margin-left: 6px;
    vertical-align: middle;
  }
  :global(.ui-card) .row-help {
    display: block;
    font-size: 14px;
    line-height: 1.45;
    color: var(--soft-ink);
    margin-top: 4px;
  }
  :global(.ui-card) .setting-row .row-control {
    min-width: 0;
    max-width: none;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 12px;
  }
  :global(.ui-card) .setting-action .row-control {
    justify-content: flex-end;
  }
  :global(.ui-card) .setting-choice .row-control {
    justify-content: flex-start;
  }
  @media (max-width: 760px) {
    :global(.ui-card) .setting-choice {
      grid-template-columns: minmax(0, 1fr);
      gap: 10px;
    }
    :global(.ui-card) .setting-choice .row-copy {
      padding-top: 0;
    }
  }
</style>
