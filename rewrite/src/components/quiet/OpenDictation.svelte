<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import Icon from "../Icon.svelte";
  import FixWord from "./FixWord.svelte";
  import SegmentedControl from "./SegmentedControl.svelte";
  import Disclosure from "./Disclosure.svelte";
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
  let phraseOffset = $state<number | null>(null);
  let editing = $state(false);
  let corrected = $state(untrack(() => record.published));
  let scope = $state("");
  let notice = $state("");
  let transcript: HTMLDivElement;
  const text = $derived(versionText(record, version));
  const dirty = $derived(editing && corrected !== record.published);
  const proofreading = $derived(proofreaderMetadata(record.cleanupStatus));
  const incomplete = $derived(record.recognitionCompleteness === "incomplete");
  const versionOptions = [
    { value: "published" as const, label: "Final" },
    { value: "pre_model" as const, label: "Before cleanup" },
    { value: "raw" as const, label: "As heard" },
  ];
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
    if (
      selection?.anchorNode &&
      transcript?.contains(selection.anchorNode) &&
      selection.rangeCount > 0
    ) {
      const selected = selection.toString().trim().slice(0, 250);
      phrase = selected;
      if (selected) {
        const range = selection.getRangeAt(0);
        const prefix = document.createRange();
        prefix.selectNodeContents(transcript);
        prefix.setEnd(range.startContainer, range.startOffset);
        phraseOffset = prefix.toString().length;
      } else {
        phraseOffset = null;
      }
    } else {
      phrase = "";
      phraseOffset = null;
    }
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
    if (result.outcome === "activated") {
      notice = "Saved correction. SonoJot will use this spelling from now on.";
    } else if (result.outcome === "suggested" || result.learnedCount > 0) {
      notice =
        "Saved correction. Added to Words → Review so you can keep or dismiss it.";
    } else {
      notice = "Saved correction to this dictation.";
    }
  }
</script>

<section class="open-dictation" aria-label="Open dictation">
  <div class="record-meta" data-text-surface="flat">
    <span class="record-time">{dictationTime(record.createdAt)}</span>
    {#if record.targetApp}<span>· {record.targetApp}</span>{/if}
    {#if incomplete}<span class="quiet-tag amber">INCOMPLETE · NOT PASTED</span
      >{/if}
    <button
      type="button"
      class="icon-button close-pocket"
      aria-label="Close dictation"
      title="Close dictation"
      onclick={() => void close()}><Icon name="close" size={16} /></button
    >
  </div>
  <div class="row-pocket">
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
      {text ?? "This dictation has no saved version before cleanup."}{text ===
      ""
        ? "No text was recovered."
        : ""}
    </div>
    <div class="record-actions">
      <div class="version-switch">
        <SegmentedControl
          label="Dictation versions"
          options={versionOptions}
          value={version}
          onchange={(next) => (version = next)}
        />
      </div>
      <div class="pocket-buttons">
        <button
          class="button primary small"
          disabled={!app.native || !text || app.busy("history-copy")}
          onclick={() => void copyDictation(record, version)}
          ><Icon name="copy" size={14} />Copy</button
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
    </div>
    {#if proofreaderAccountBlocked(record.guardReason) || proofreaderAccountBlocked(record.cleanupStatus)}
      <ProofreaderPrivacyLink />
    {/if}
    <div class="pocket-more">
      <Disclosure title="More">
        <div class="actions">
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
            }}><Icon name="delete" size={14} />Delete this dictation</button
          >
        </div>
        {#if editing}
          <section class="spaced" aria-label="Edit full text">
            <label class="field-label" for="history-corrected"
              >Corrected text</label
            >
            <textarea
              id="history-corrected"
              rows="4"
              bind:value={corrected}
              maxlength="2000"
              disabled={app.busy("history-correct")}></textarea>
            <div class="nested-fold">
              <Disclosure title="Project or topic (optional)">
                <label
                  >Project or topic<input
                    bind:value={scope}
                    maxlength="160"
                  /></label
                >
              </Disclosure>
            </div>
            <button
              class="button small primary spaced"
              disabled={!app.native ||
                !corrected.trim() ||
                app.busy("history-correct")}
              onclick={() => void save()}>Save correction</button
            >
            {#if notice}<p class="field-help" role="status">{notice}</p>{/if}
          </section>
        {/if}
        <div class="nested-fold">
          <Disclosure title="Processing details">
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
          </Disclosure>
        </div>
      </Disclosure>
    </div>
  </div>
</section>
{#if fixing}<FixWord
    {record}
    {phrase}
    {phraseOffset}
    {onchanged}
    onclose={() => (fixing = false)}
  />{/if}

<style>
  .open-dictation {
    padding: 0;
  }
  .record-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 10px;
    padding: 12px 18px 8px;
    font-size: 13px;
    color: var(--soft-ink);
  }
  .record-time {
    font-variant-numeric: tabular-nums;
  }
  .close-pocket {
    margin-left: auto;
    width: 30px;
    height: 30px;
    padding: 0;
    border-radius: 8px;
    background: transparent;
    box-shadow: none;
    color: var(--soft-ink);
  }
  .close-pocket:hover {
    color: var(--ink);
  }
  .full-text {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    margin: 0 0 16px;
    font-size: 16px;
    line-height: 1.6;
  }
  .record-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 12px;
  }
  .version-switch {
    margin-right: auto;
    max-width: 100%;
  }
  .pocket-buttons {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .pocket-more {
    margin-top: 12px;
    border-top: 1px solid var(--line);
  }
  .nested-fold {
    margin-top: 8px;
    border-top: 1px solid var(--line);
  }
  textarea {
    width: 100%;
  }
</style>
