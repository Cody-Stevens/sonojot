<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import Card from "./Card.svelte";
  import Icon from "./Icon.svelte";
  import { scrollEdges } from "../lib/scroll-edges";
  import SettingRow from "./SettingRow.svelte";
  import Toggle from "./Toggle.svelte";
  import ProofreaderPrivacyLink from "./ProofreaderPrivacyLink.svelte";
  import {
    proofreaderAccountBlocked,
    proofreaderMetadata,
    proofreaderOutcomeLabel,
    proofreaderGuardLabel,
  } from "../lib/proofreader";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import type { DictationRecord } from "../lib/protocol";
  import {
    guardReasonLabel,
    historyVersions,
    publicationLabel,
  } from "../lib/smart-cleanup";
  let records = $state<DictationRecord[]>([]);
  let selected = $state<DictationRecord | null>(null);
  let loaded = $state(!app.native);
  let heard = $state("");
  let written = $state("");
  let corrected = $state("");
  let correctionScope = $state("");
  let correctionNotice = $state("");
  let editing = $state(false);
  let comparing = $state(false);
  let addingWord = $state(false);
  let processingDetails = $state(false);
  const proofreading = $derived(
    proofreaderMetadata(selected?.cleanupStatus ?? null),
  );
  const dirty = $derived(
    selected !== null &&
      (corrected !== selected.published ||
        !!correctionScope.trim() ||
        !!heard.trim() ||
        !!written.trim()),
  );
  $effect(() => {
    app.dirty = dirty;
  });
  onDestroy(() => {
    app.dirty = false;
  });
  async function back(): Promise<void> {
    if (
      dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This dictation has changes that have not been saved.",
      ))
    )
      return;
    selected = null;
  }
  let lastSelectedId = $state<string | null>(null);
  let lastRevision = $state(0);
  const validCorrection = $derived(
    selected !== null &&
      heard.trim().length > 0 &&
      written.trim().length > 0 &&
      selected.raw
        .toLocaleLowerCase()
        .includes(heard.trim().toLocaleLowerCase()),
  );
  const captureActive = $derived(
    ["loading", "listening", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const newestId = $derived(records.length ? records[0].id : null);
  function sortNewest(rows: DictationRecord[]): DictationRecord[] {
    return [...rows].sort(
      (left, right) =>
        right.createdAt.localeCompare(left.createdAt) ||
        (left.id < right.id ? -1 : left.id > right.id ? 1 : 0),
    );
  }
  async function refreshList(): Promise<void> {
    const result = await app.perform("history-load", () =>
      call("list_dictations", { limit: 100 }),
    );
    if (result) {
      records = sortNewest(result);
      loaded = true;
      // Rebind the open detail to the fresh row; never drop a review in progress.
      const id = selected?.id;
      if (id) selected = records.find((item) => item.id === id) ?? selected;
    }
  }
  onMount(() => {
    if (app.native) void refreshList();
  });
  // Refresh only when a result is committed, never on raw interims.
  $effect(() => {
    const revision = app.dictationRevision;
    if (app.native && loaded && revision !== lastRevision)
      untrack(() => {
        lastRevision = revision;
        void refreshList();
      });
  });
  $effect(() => {
    const id = selected?.id ?? null;
    if (id !== lastSelectedId)
      untrack(() => {
        lastSelectedId = id;
        heard = "";
        written = "";
        corrected = selected?.published ?? "";
        correctionScope = "";
        correctionNotice = "";
        editing = false;
        comparing = false;
        addingWord = false;
        processingDetails = false;
      });
  });
  async function learn(): Promise<void> {
    // Legacy heard/written vocabulary path. Requires the phrase in the saved
    // original; backend enforces source existence. Never used as a silent
    // fallback for full-text corrections below.
    const id = selected?.id;
    if (!id || !validCorrection) return;
    const result = await app.perform(
      "history-learn",
      () =>
        call("learn_correction", {
          id,
          heard: heard.trim(),
          written: written.trim(),
        }),
      "Correction added to your vocabulary.",
    );
    if (result) {
      heard = "";
      written = "";
    }
  }
  async function saveCorrection(): Promise<void> {
    // Canonical full-text correction path. Uses save_dictation_correction only;
    // a native failure surfaces via app.error and never falls back to the
    // legacy learn_correction command.
    const id = selected?.id;
    const text = corrected.trim();
    if (!id || !text) return;
    const result = await app.perform("history-correct", () =>
      call("save_dictation_correction", {
        id,
        text,
        scope: correctionScope.trim() || null,
      }),
    );
    if (result) {
      records = records.map((item) => (item.id === id ? result.record : item));
      selected = result.record;
      corrected = result.record.published;
      correctionScope = "";
      correctionNotice =
        result.learnedCount > 0
          ? `Saved correction. ${result.learnedCount} learned ${result.learnedCount === 1 ? "entry" : "entries"} will be reused for matching words.`
          : "Saved correction to this dictation. Nothing was added to learning.";
    }
  }
  async function copyRecord(
    record: DictationRecord,
    original: boolean,
  ): Promise<void> {
    await app.perform(
      "history-copy",
      () => call("copy_history", { id: record.id, original }),
      record.recognitionCompleteness === "incomplete"
        ? "Reviewed text copied."
        : original
          ? "Original text copied."
          : "Finished text copied.",
    );
  }
  async function removeRecord(record: DictationRecord): Promise<void> {
    let scopeNote = "";
    if (app.native) {
      try {
        const learning = await call("get_learning");
        const dependents = learning.filter((entry) =>
          entry.sourceIds.includes(record.id),
        );
        if (dependents.length) {
          const scopes = Array.from(
            new Set(
              dependents
                .map((entry) => entry.scope)
                .filter((scope): scope is string => !!scope),
            ),
          );
          scopeNote = ` Derived learning (${dependents.length} ${dependents.length === 1 ? "entry" : "entries"}${scopes.length ? `, projects or topics: ${scopes.join(", ")}` : ""}) loses this source and entries without other support are removed.`;
        }
      } catch {
        // Learning state is unavailable; the delete dialog keeps the core warning.
      }
    }
    const confirmed = await app.confirm(
      "Delete this dictation?",
      `This removes the saved original and finished text from SonoJot.${scopeNote}`,
      "Delete",
      true,
    );
    if (!confirmed) return;
    const next = await app.perform("history-delete", () =>
      call("delete_dictation", { id: record.id, confirmed: true }),
    );
    if (next) {
      app.apply(next);
      records = records.filter((item) => item.id !== record.id);
      if (selected?.id === record.id) {
        selected = null;
        lastSelectedId = null;
      }
      app.notice = "Dictation deleted.";
    }
  }
  async function removeAll(): Promise<void> {
    const count = await app.perform("history-count", () =>
      call("count_dictations"),
    );
    if (count === undefined) return;
    if (count === 0) {
      app.notice = "There are no saved dictations to delete.";
      return;
    }
    const confirmed = await app.confirm(
      `Delete all ${count} dictations?`,
      `This removes all ${count} saved ${count === 1 ? "dictation" : "dictations"} (original and finished text) from SonoJot. Saved vocabulary and meetings are kept.`,
      "Delete all",
      true,
    );
    if (!confirmed) return;
    const result = await app.perform("history-delete-all", () =>
      call("delete_all_dictations", { confirmed: true }),
    );
    if (result) {
      app.apply(result.snapshot);
      records = [];
      selected = null;
      lastSelectedId = null;
      app.notice =
        result.deletedCount === 1
          ? "1 dictation deleted."
          : `${result.deletedCount} dictations deleted.`;
    }
  }
  function date(value: string): string {
    const parsed = new Date(value);
    return Number.isNaN(parsed.getTime())
      ? value
      : parsed.toLocaleString(undefined, {
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        });
  }
</script>

<Card title={selected ? undefined : "Recent"}>
  {#if selected}
    <button class="back-button" onclick={() => void back()}
      ><Icon name="back" size={18} />All recent dictations</button
    >
    <p class="field-help" data-text-surface="flat">
      {date(selected.createdAt)}
    </p>
    {#if selected.recognitionCompleteness === "incomplete"}
      <p class="field-help" data-text-surface="flat" role="status">
        Recognition was incomplete. Review any available text before copying.
      </p>
    {/if}
    <div class="paper-well transcript" use:scrollEdges>
      {selected.published ||
        (selected.recognitionCompleteness === "incomplete"
          ? "No text was recovered."
          : "No text was published.")}
    </div>
    {#if proofreaderAccountBlocked(selected.guardReason) || proofreaderAccountBlocked(selected.cleanupStatus)}
      <ProofreaderPrivacyLink />
    {/if}
    <div class="actions">
      <button
        class="button primary"
        disabled={!app.native ||
          app.busy("history-copy") ||
          !selected.published}
        onclick={() => {
          if (selected) void copyRecord(selected, false);
        }}
      >
        <Icon name="copy" size={16} />Copy
      </button>
      <button
        class="button"
        aria-expanded={editing}
        aria-controls="dictation-correction"
        onclick={() => (editing = !editing)}
        >{editing ? "Close editor" : "Fix a mistake"}</button
      >
      <button
        class="button"
        aria-expanded={comparing}
        aria-controls="dictation-versions"
        onclick={() => (comparing = !comparing)}
        >{comparing ? "Hide versions" : "Compare versions"}</button
      >
      <details class="license record-more">
        <summary>More</summary>
        <div class="more-actions stock paper-popover paper-menu">
          <button
            class="button quiet small"
            aria-expanded={addingWord}
            aria-controls="dictation-add-word"
            onclick={() => (addingWord = !addingWord)}
            >Add a word to Vocabulary…</button
          >
          <button
            class="button quiet small"
            aria-expanded={processingDetails}
            aria-controls="dictation-processing"
            onclick={() => (processingDetails = !processingDetails)}
            >Processing details</button
          >
          <button
            class="button quiet small"
            disabled={!app.native ||
              app.busy("history-delete") ||
              app.busy("history-correct") ||
              app.busy("history-learn")}
            onclick={() => {
              if (selected) void removeRecord(selected);
            }}>Delete this dictation</button
          >
        </div>
      </details>
    </div>
    {#if editing}
      <section
        class="review-section"
        id="dictation-correction"
        aria-label="Fix a mistake"
      >
        <h3 data-text-surface="emboss">Fix a mistake</h3>
        <p class="body-copy" data-text-surface="flat">
          Fix the finished text and save it{app.snapshot.settings
            .doNotSaveDictations
            ? " as the only saved trace of this dictation; history stays off for new dictations"
            : ""}. Clear word corrections can be reused for matching words.
        </p>
        <label class="field-label" for="history-corrected">Corrected text</label
        >
        <div class="scroll-edges-frame">
          <textarea
            id="history-corrected"
            bind:value={corrected}
            rows="4"
            maxlength="2000"
            disabled={app.busy("history-correct")}
            use:scrollEdges={{ frame: true }}></textarea>
        </div>
        <details class="license scope-fold">
          <summary>Project or topic (optional)</summary>
          <label class="field-label" for="history-scope">Project or topic</label
          >
          <input
            id="history-scope"
            bind:value={correctionScope}
            maxlength="250"
            placeholder="For example, Spring launch"
            disabled={app.busy("history-correct")}
          />
        </details>
        <div class="actions">
          <button
            class="button"
            disabled={!app.native ||
              !corrected.trim() ||
              app.busy("history-correct")}
            onclick={() => void saveCorrection()}
            >{app.busy("history-correct")
              ? "Saving…"
              : "Save correction"}</button
          >
        </div>
        {#if correctionNotice}<p
            class="field-help"
            data-text-surface="flat"
            role="status"
          >
            {correctionNotice}
          </p>{/if}
      </section>
    {/if}
    {#if comparing}
      <section
        class="review-section history-comparison"
        id="dictation-versions"
        aria-label="Compare versions"
      >
        {#each historyVersions(selected) as version}
          <div>
            <div class="section-toolbar">
              <h3 data-text-surface="emboss">{version.title}</h3>
              <button
                class="button small"
                disabled={!app.native ||
                  app.busy("history-copy") ||
                  !version.text}
                onclick={() => {
                  const record = selected;
                  if (!record) return;
                  if (version.kind === "pre_model") {
                    void app.perform(
                      "history-copy",
                      () => call("copy_history_pre_model", { id: record.id }),
                      "Text before AI changes copied.",
                    );
                  } else void copyRecord(record, version.kind === "raw");
                }}><Icon name="copy" size={16} />Copy</button
              >
            </div>
            <div class="paper-well transcript" use:scrollEdges>
              {version.text ||
                (selected.recognitionCompleteness === "incomplete"
                  ? "No text was recovered."
                  : "No text was published.")}
            </div>
          </div>
        {/each}
      </section>
    {/if}
    {#if addingWord}
      <section
        class="review-section"
        id="dictation-add-word"
        aria-label="Add a word to Vocabulary"
      >
        <h3 data-text-surface="emboss">Add a word to Vocabulary</h3>
        <p class="body-copy" data-text-surface="flat">
          Choose a word or phrase the recognizer heard and the spelling you
          meant. It becomes a vocabulary correction for Clean mode.
        </p>
        <form
          onsubmit={(event) => {
            event.preventDefault();
            void learn();
          }}
        >
          <div class="two-fields">
            <label
              >When it hears<input
                bind:value={heard}
                maxlength="250"
                placeholder="The recognized phrase"
                disabled={app.busy("history-learn")}
              /></label
            >
            <label
              >Write it as<input
                bind:value={written}
                maxlength="250"
                placeholder="Your correction"
                disabled={app.busy("history-learn")}
              /></label
            >
          </div>
          {#if heard.trim() && !selected.raw
              .toLocaleLowerCase()
              .includes(heard.trim().toLocaleLowerCase())}
            <p class="field-help" data-text-surface="flat">
              Use a phrase that appears in the As recognized version.
            </p>
          {/if}
          <button
            type="submit"
            class="button"
            disabled={!app.native ||
              !validCorrection ||
              app.busy("history-learn")}
          >
            {app.busy("history-learn") ? "Adding…" : "Add to vocabulary"}
          </button>
        </form>
      </section>
    {/if}
    {#if processingDetails}
      <section
        class="review-section"
        id="dictation-processing"
        aria-label="Processing details"
      >
        <h3 data-text-surface="emboss">Processing details</h3>
        <dl class="details-list">
          <div>
            <dt>Recognition</dt>
            <dd>
              {selected.recognitionCompleteness === "incomplete"
                ? "Incomplete"
                : "Complete"}
            </dd>
          </div>
          {#if selected.publicationStage}<div>
              <dt>Finished text</dt>
              <dd>{publicationLabel(selected.publicationStage)}</dd>
            </div>{/if}
          {#if selected.guardReason}<div>
              <dt>Guard reason</dt>
              <dd>{guardReasonLabel(selected.guardReason)}</dd>
            </div>{/if}
          {#if proofreading}
            <div>
              <dt>Proofreading model</dt>
              <dd class="diagnostic-code">{proofreading.model}</dd>
            </div>
            <div>
              <dt>Proofreading outcome</dt>
              <dd>{proofreaderOutcomeLabel(proofreading.outcome)}</dd>
            </div>
            {#if !["accepted", "unchanged"].includes(proofreading.outcome)}
              <div>
                <dt>Proofreading reason</dt>
                <dd>{proofreaderGuardLabel(proofreading.reason)}</dd>
              </div>
            {/if}
          {/if}
        </dl>
        {#if selected.guardReason}<p
            class="field-help diagnostic-code"
            data-text-surface="flat"
          >
            {selected.guardReason}
          </p>{/if}
        {#each selected.recognitionWarnings as warning}<p
            class="field-help"
            data-text-surface="flat"
          >
            {warning}
          </p>{/each}
      </section>
    {/if}
  {:else}
    <div class="section-toolbar">
      <p class="field-help" data-text-surface="flat">
        Newest first · Last 100 dictations
      </p>
      <button
        class="button primary small"
        disabled={!app.native ||
          app.busy("history-copy") ||
          !records[0]?.published}
        onclick={() => {
          const record = records[0];
          if (!record) return;
          if (record.recognitionCompleteness === "incomplete")
            selected = record;
          else void copyRecord(record, false);
        }}><Icon name="copy" size={16} />Copy latest</button
      >
    </div>
    <details class="license history-options">
      <summary>History options</summary>
      <div class="history-options-panel stock paper-popover">
        <SettingRow
          label="Save dictation history"
          help="Save new dictations for review and automatic learning. Turning this off keeps existing dictations until you delete them."
        >
          <Toggle
            label="Save dictation history"
            checked={!app.snapshot.settings.doNotSaveDictations}
            disabled={!app.native ||
              app.busy("settings") ||
              app.busy("history-delete-all") ||
              captureActive}
            onchange={(saveHistory) =>
              app.saveSettings({ doNotSaveDictations: !saveHistory })}
          />
        </SettingRow>
        {#if captureActive}<p class="field-help" data-text-surface="flat">
            Finish the current dictation to change retention.
          </p>{/if}
        <div class="actions">
          <button
            class="button quiet small"
            disabled={!app.native || app.busy("history-load")}
            onclick={() => void refreshList()}
            >{app.busy("history-load") ? "Loading…" : "Refresh list"}</button
          >
          <button
            class="button quiet small"
            disabled={!app.native ||
              app.busy("history-delete-all") ||
              app.busy("history-count") ||
              captureActive}
            onclick={() => void removeAll()}
            >{app.busy("history-delete-all") || app.busy("history-count")
              ? "Working…"
              : "Delete all dictations…"}</button
          >
        </div>
      </div>
    </details>
    {#if app.snapshot.settings.doNotSaveDictations}
      <p class="field-help" data-text-surface="flat">
        New dictations are not saved. You can still paste them or use Copy last
        on the screen tab until the next dictation or app exit. Existing
        dictations stay until you delete them. Meeting recordings use their own
        retention.
      </p>
    {/if}
    {#if !loaded}
      <p class="empty-copy" data-text-surface="flat">
        {app.busy("history-load")
          ? "Loading recent dictations…"
          : "Recent dictations could not be loaded. Try refreshing."}
      </p>
    {:else if !records.length}
      <p class="empty-copy" data-text-surface="flat">
        {app.snapshot.settings.doNotSaveDictations
          ? "There are no saved dictations."
          : "Your completed dictations will appear here."}
      </p>
    {:else}
      <ul class="history-list" aria-label="Recent dictations" use:scrollEdges>
        {#each records as record (record.id)}
          <li class="history-row" class:latest-row={record.id === newestId}>
            <button
              class="history-item"
              title={record.published || record.raw || "Empty dictation"}
              onclick={() => (selected = record)}
            >
              <span>{record.published || record.raw || "Empty dictation"}</span>
              <small data-text-surface="flat"
                >{#if record.id === newestId}<span class="latest-marker"
                    >Latest</span
                  >{/if}{date(record.createdAt)}</small
              >
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</Card>

<style>
  .transcript {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .review-section {
    margin-top: 27px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
  }
  .scope-fold,
  .history-options {
    margin: 18px 0;
  }
  .record-more {
    align-self: center;
    position: relative;
  }
  .history-options {
    position: relative;
  }
  .more-actions,
  .history-options-panel {
    position: absolute;
    top: calc(100% + 9px);
    left: 0;
    z-index: 5;
    width: min(380px, 65vw);
  }
  .history-options-panel {
    width: min(560px, 100%);
    padding: 9px 18px 18px;
  }
  .diagnostic-code {
    overflow-wrap: anywhere;
  }
  .history-row {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .history-row .history-item {
    flex: 1;
    min-width: 0;
    background: transparent;
  }
  .history-row .history-item:hover,
  .history-row .history-item:focus-visible {
    background: color-mix(in srgb, var(--dye) 7%, transparent);
  }
  .latest-marker {
    display: inline-block;
    margin-right: 9px;
    padding: 1px 9px;
    border: 1px solid color-mix(in srgb, var(--dye) 35%, transparent);
    border-radius: 7px;
    background: color-mix(in srgb, var(--dye) 8%, transparent);
    color: var(--dye);
    font-size: 0.78rem;
    font-weight: 600;
  }
</style>
