<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Icon from "../Icon.svelte";
  import FixWord from "./FixWord.svelte";
  import ProofreaderPrivacyLink from "../ProofreaderPrivacyLink.svelte";
  import { app } from "../../lib/app.svelte";
  import { call } from "../../lib/api";
  import {
    dictationTime,
    versionText,
    type DictationVersion,
  } from "../../lib/dictation-history";
  import {
    copyDictation,
    exportDictation,
    deleteDictation,
  } from "../../lib/dictation-actions";
  import {
    proofreaderAccountBlocked,
    proofreaderMetadata,
    proofreaderOutcomeLabel,
    proofreaderGuardLabel,
  } from "../../lib/proofreader";
  import { guardReasonLabel, publicationLabel } from "../../lib/smart-cleanup";
  import type { DictationRecord } from "../../lib/protocol";
  let {
    record,
    startFix = false,
    onchanged,
    onclose,
    ondeleted,
  }: {
    record: DictationRecord;
    startFix?: boolean;
    onchanged: (record: DictationRecord) => void;
    onclose: () => void;
    ondeleted: () => void;
  } = $props();
  let version = $state<DictationVersion>("published");
  let fixing = $state(untrack(() => startFix));
  let phrase = $state("");
  let editing = $state(false);
  let corrected = $state(untrack(() => record.published));
  let scope = $state("");
  let notice = $state("");
  let transcript: HTMLDivElement;
  const text = $derived(versionText(record, version));
  const dirty = $derived(
    editing && (corrected !== record.published || !!scope.trim()),
  );
  const proofreading = $derived(proofreaderMetadata(record.cleanupStatus));
  const incomplete = $derived(record.recognitionCompleteness === "incomplete");
  $effect(() => {
    if (!fixing) app.dirty = dirty;
  });
  $effect(() => {
    const latest = record.published;
    if (!editing) untrack(() => (corrected = latest));
  });
  onDestroy(() => {
    app.dirty = false;
  });
  async function close() {
    if (
      dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This dictation has changes that have not been saved.",
      ))
    )
      return;
    onclose();
  }
  function fix() {
    const selection = window.getSelection();
    phrase =
      selection?.anchorNode && transcript?.contains(selection.anchorNode)
        ? selection.toString().trim().slice(0, 250)
        : "";
    fixing = true;
  }
  async function save() {
    const result = await app.perform("history-correct", () =>
      call("save_dictation_correction", {
        id: record.id,
        text: corrected.trim(),
        scope: scope.trim() || null,
      }),
    );
    if (!result) return;
    onchanged(result.record);
    corrected = result.record.published;
    scope = "";
    notice = result.learnedCount
      ? `Saved correction. ${result.learnedCount} learned entries will be reused for matching words.`
      : "Saved correction to this dictation.";
  }
</script>

<section class="open-dictation" aria-label="Open dictation">
  <div class="record-meta" data-text-surface="flat">
    <span>{dictationTime(record.createdAt)}</span>
    {#if record.targetApp}<span>· {record.targetApp}</span>{/if}
    {#if incomplete}<span class="quiet-tag amber">INCOMPLETE · NOT PASTED</span
      >{/if}
    <button
      class="button small"
      aria-label="Close dictation"
      onclick={() => void close()}><Icon name="close" size={16} /></button
    >
  </div>
  {#if incomplete}<p class="field-help">
      Recognition was incomplete. Review the available text before copying.
    </p>{/if}
  <div
    bind:this={transcript}
    class="full-text"
    aria-label={version === "raw"
      ? "As heard"
      : version === "pre_model"
        ? "Before cleanup"
        : "Final text"}
  >
    {text ?? "This dictation has no saved version before cleanup."}{text === ""
      ? "No text was recovered."
      : ""}
  </div>
  <div class="record-actions">
    <div class="segments" role="group" aria-label="Dictation versions">
      {#each [{ key: "published", label: "Final" }, { key: "pre_model", label: "Before cleanup" }, { key: "raw", label: "As heard" }] as item}
        <button
          aria-pressed={version === item.key}
          onclick={() => (version = item.key as DictationVersion)}
          >{item.label}</button
        >
      {/each}
    </div>
    <button
      class="button primary small"
      disabled={!app.native || !text || app.busy("history-copy")}
      onclick={() => void copyDictation(record, version)}
      ><Icon name="copy" size={15} />Copy</button
    >
    <button class="button small" disabled={editing} onclick={fix}
      >Fix a word</button
    >
    <button
      class="button small"
      disabled={!app.native || dirty || app.busy("history-export")}
      onclick={() => void exportDictation(record)}>Export .md</button
    >
  </div>
  {#if proofreaderAccountBlocked(record.guardReason) || proofreaderAccountBlocked(record.cleanupStatus)}<ProofreaderPrivacyLink
    />{/if}
  <details class="license spaced">
    <summary>More</summary>
    <div class="actions spaced">
      <button
        class="button small"
        aria-expanded={editing}
        onclick={async () => {
          if (
            dirty &&
            !(await app.confirm(
              "Leave these changes?",
              "This dictation has changes that have not been saved.",
            ))
          )
            return;
          corrected = record.published;
          scope = "";
          editing = !editing;
        }}>{editing ? "Close editor" : "Edit full text"}</button
      >
      <button
        class="button small danger"
        disabled={!app.native ||
          app.busy("history-delete") ||
          app.busy("history-correct")}
        onclick={async () => {
          if (await deleteDictation(record)) ondeleted();
        }}><Icon name="delete" size={15} />Delete this dictation</button
      >
    </div>
    {#if editing}
      <section class="spaced" aria-label="Edit full text">
        <label class="field-label" for="history-corrected">Corrected text</label
        >
        <textarea
          id="history-corrected"
          rows="4"
          bind:value={corrected}
          maxlength="2000"
          disabled={app.busy("history-correct")}></textarea>
        <details class="license spaced">
          <summary>Project or topic (optional)</summary><label
            >Project or topic<input bind:value={scope} maxlength="160" /></label
          >
        </details>
        <button
          class="button"
          disabled={!app.native ||
            !corrected.trim() ||
            app.busy("history-correct")}
          onclick={() => void save()}>Save correction</button
        >
        {#if notice}<p role="status">{notice}</p>{/if}
      </section>
    {/if}
    <details class="license spaced">
      <summary>Processing details</summary>
      <dl class="details-list">
        <div>
          <dt>Recognition</dt>
          <dd>{incomplete ? "Incomplete" : "Complete"}</dd>
        </div>
        {#if record.publicationStage}<div>
            <dt>Finished text</dt>
            <dd>{publicationLabel(record.publicationStage)}</dd>
          </div>{/if}
        {#if record.guardReason}<div>
            <dt>Guard reason</dt>
            <dd>{guardReasonLabel(record.guardReason)}</dd>
          </div>{/if}
        {#if proofreading}<div>
            <dt>Proofreading model</dt>
            <dd>{proofreading.model}</dd>
          </div>
          <div>
            <dt>Proofreading outcome</dt>
            <dd>{proofreaderOutcomeLabel(proofreading.outcome)}</dd>
          </div>
          <div>
            <dt>Proofreading reason</dt>
            <dd>{proofreaderGuardLabel(proofreading.reason)}</dd>
          </div>{/if}
      </dl>
      {#each record.recognitionWarnings as warning}<p class="field-help">
          {warning}
        </p>{/each}
    </details>
  </details>
</section>
{#if fixing}<FixWord
    {record}
    {phrase}
    {onchanged}
    onclose={() => (fixing = false)}
  />{/if}

<style>
  .open-dictation {
    padding: 18px 22px;
  }
  .record-meta {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 14px;
    color: var(--soft-ink);
  }
  .record-meta button {
    margin-left: auto;
  }
  .full-text {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 20px 0;
    line-height: 1.65;
  }
  .record-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .segments {
    margin-right: auto;
    margin-bottom: 0;
    flex: 0 0 auto;
  }
  .segments button {
    font-size: 14px;
    padding: 7px 10px;
    white-space: nowrap;
    flex: 0 0 auto;
  }
  textarea {
    width: 100%;
  }
</style>
