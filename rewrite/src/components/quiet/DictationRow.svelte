<script lang="ts">
  import RowActions from "./RowActions.svelte";
  import Icon from "../Icon.svelte";
  import { app } from "../../lib/app.svelte";
  import { copyDictation, exportDictation } from "../../lib/dictation-actions";
  import { dictationTime } from "../../lib/dictation-history";
  import type { DictationRecord } from "../../lib/protocol";
  let {
    record,
    onopen,
  }: { record: DictationRecord; onopen: (fix?: boolean) => void } = $props();
</script>

<div class="dictation-row" data-quiet-row>
  <button
    class="row-open"
    data-dictation-id={record.id}
    aria-label={`Open dictation: ${record.published || record.raw || "No text recovered"}`}
    onclick={() => onopen()}
  >
    <time datetime={record.createdAt} data-text-surface="flat"
      >{dictationTime(record.createdAt)}</time
    >
    <span class="row-text"
      >{#if record.recognitionCompleteness === "incomplete"}<span
          class="quiet-tag amber">NOT PASTED</span
        >
      {/if}{record.published || record.raw || "No text recovered"}</span
    >
    {#if record.targetApp}<span class="target-app" data-text-surface="flat"
        >{record.targetApp}</span
      >{/if}
  </button>
  <div class="row-tools">
    <RowActions>
      <button
        class="button small"
        disabled={!app.native || app.busy("history-copy") || !record.published}
        onclick={() =>
          record.recognitionCompleteness === "incomplete"
            ? onopen()
            : void copyDictation(record)}
        ><Icon name="copy" size={14} />Copy</button
      >
      <button class="button small" onclick={() => onopen(true)}>Fix</button>
      <button
        class="button small"
        disabled={!app.native || app.busy("history-export")}
        onclick={() =>
          record.recognitionCompleteness === "incomplete"
            ? onopen()
            : void exportDictation(record)}>Export .md</button
      >
    </RowActions>
  </div>
</div>

<style>
  .dictation-row {
    position: relative;
  }
  .row-open {
    display: flex;
    align-items: center;
    width: 100%;
    min-height: 54px;
    gap: 18px;
    padding: 14px 18px;
    border: 0;
    background: transparent;
    color: var(--ink);
    text-align: left;
    border-radius: 0;
    font-size: 16px;
    transition: background-color var(--motion-control) linear;
  }
  .row-open:hover,
  .dictation-row:focus-within .row-open {
    background-color: var(--well);
  }
  .row-open:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  time {
    width: 68px;
    flex: 0 0 68px;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--soft-ink);
  }
  .row-text {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .row-text .quiet-tag {
    margin-inline-end: 8px;
  }
  .target-app {
    font-size: 13px;
    color: var(--soft-ink);
    max-width: 120px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .row-tools {
    position: absolute;
    right: 12px;
    top: 50%;
    transform: translateY(-50%);
    pointer-events: none;
  }
  .row-tools :global(.quiet-row-actions) {
    padding: 4px;
  }
  .dictation-row:hover .row-open,
  .dictation-row:focus-within .row-open {
    padding-right: 255px;
  }
  .dictation-row:hover .target-app,
  .dictation-row:focus-within .target-app {
    display: none;
  }
  @media (max-width: 760px) {
    time {
      width: 56px;
      flex-basis: 56px;
    }
  }
  @media (hover: none) {
    .row-tools {
      position: static;
      transform: none;
      padding: 0 18px 9px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .row-open {
      transition: none;
    }
  }
</style>
