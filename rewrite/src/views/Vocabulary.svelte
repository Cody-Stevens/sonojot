<script lang="ts">
  import { onMount, onDestroy, tick } from "svelte";
  import LocalLearning from "../components/LocalLearning.svelte";
  import ModelSummary from "../components/ModelSummary.svelte";
  import RemoteModelBadge from "../components/RemoteModelBadge.svelte";
  import type { ModelRole } from "../lib/model-library";
  import Card from "../components/Card.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Icon from "../components/Icon.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { errorMessage, type VocabularyEntry } from "../lib/protocol";
  import type { AiSettings, VocabularySuggestion } from "../lib/ai";
  let { onmanage }: { onmanage?: (role: ModelRole) => void } = $props();
  let learningDirty = $state(false);
  let entries = $state<VocabularyEntry[]>([]);
  let saved = $state("[]");
  let loaded = $state(!app.native);
  let heard = $state("");
  let written = $state("");
  let scope = $state("");
  let search = $state("");
  let view = $state<"words" | "suggestions" | "learning">("words");
  let moreOpen = $state(false);
  let adding = $state(false);
  let expandedId = $state<string | null>(null);
  let searchInput = $state<HTMLInputElement>();
  let backButton = $state<HTMLButtonElement>();
  let addButton = $state<HTMLButtonElement>();
  let heardInput = $state<HTMLInputElement>();
  let aiSettings = $state<AiSettings | null>(null);
  let loadingAi = $state(false);
  let aiError = $state("");
  let suggestions = $state<VocabularySuggestion[]>([]);
  let reviewState = $state<"idle" | "reviewing" | "cancelling">("idle");
  let reviewNotice = $state("");
  let reviewError = $state("");
  let reviewGeneration = 0;
  let disposed = false;
  const reviewing = $derived(reviewState !== "idle");
  const reviewReady = $derived(
    aiSettings?.backend === "local" ||
      (aiSettings?.supported && aiSettings.connected && !!aiSettings.model),
  );
  const dirty = $derived(JSON.stringify(entries) !== saved);
  const draftDirty = $derived(!!(heard || written || scope));
  $effect(() => {
    app.dirty = dirty || draftDirty || learningDirty;
  });
  const filtered = $derived(
    entries.filter((item) =>
      `${item.heard} ${item.written} ${item.scope ?? ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
    ),
  );
  onDestroy(() => {
    disposed = true;
    reviewGeneration++;
    if (reviewState === "reviewing")
      void call("cancel_vocabulary_suggestions").catch(() => {
        // The view is gone; its late result is discarded even if cancellation fails.
      });
    app.dirty = false;
  });
  function entryStatus(entry: VocabularyEntry): string {
    if (!entry.enabled) return "Disabled";
    const heard = entry.heard.trim().toLowerCase();
    const written = entry.written.trim().toLowerCase();
    if (heard === written && /^\p{L}+$/u.test(written))
      return "Casing correction in Clean mode; no project or topic needed.";
    if (
      ["grock:grok", "moonshien:moonshine", "nemotorn:nemotron"].includes(
        `${heard}:${written}`,
      )
    )
      return "Spelling correction only near model-related words";
    if (!entry.scope?.trim())
      return "This correction needs a supported project or topic to apply in Clean mode.";
    if (
      entry.scope.trim().toLowerCase().replace(/\s+/g, " ") !==
      "ai model discussion"
    )
      return "This project or topic is not supported yet. Currently supported: AI model discussion.";
    const supportedName = (value: string): boolean =>
      new TextEncoder().encode(value).length <= 128 &&
      /^[\p{L}\p{N}](?:[\p{L}\p{N}_-]*[\p{L}\p{N}])?(?: [\p{L}\p{N}](?:[\p{L}\p{N}_-]*[\p{L}\p{N}])?)*$/u.test(
        value,
      );
    if (
      !supportedName(entry.heard.trim()) ||
      !supportedName(entry.written.trim())
    )
      return "Inactive: use short words or phrases with letters, numbers, hyphens or underscores; start and end each word with a letter or number.";
    return "Used in Clean mode only when supported model-related words appear beside the correction.";
  }
  async function showView(next: typeof view): Promise<void> {
    view = next;
    moreOpen = false;
    await tick();
    if (next === "words") searchInput?.focus();
    else backButton?.focus();
  }
  async function toggleAdd(): Promise<void> {
    adding = !adding;
    await tick();
    if (adding) heardInput?.focus();
    else addButton?.focus();
  }
  function cancelAdd(): void {
    heard = "";
    written = "";
    scope = "";
    adding = false;
    addButton?.focus();
  }
  async function load(): Promise<void> {
    const result = await app.perform("vocabulary-load", () =>
      call("get_vocabulary"),
    );
    if (result) {
      entries = result;
      saved = JSON.stringify(result);
      loaded = true;
    }
  }
  onMount(() => {
    if (app.native) {
      void load();
      void loadAi();
    }
  });
  const heardKey = (text: string): string => text.trim().toLowerCase();
  function hasEntry(value: string): boolean {
    return entries.some((entry) => heardKey(entry.heard) === heardKey(value));
  }
  async function loadAi(): Promise<void> {
    loadingAi = true;
    aiError = "";
    try {
      const settings = await call("get_ai_settings");
      if (!disposed) aiSettings = settings;
    } catch (error: unknown) {
      if (!disposed) aiError = errorMessage(error);
    } finally {
      if (!disposed) loadingAi = false;
    }
  }
  async function review(): Promise<void> {
    if (!loaded || !reviewReady || reviewing) return;
    const generation = ++reviewGeneration;
    reviewState = "reviewing";
    reviewNotice = "";
    reviewError = "";
    suggestions = [];
    try {
      const result = await call("suggest_vocabulary");
      if (disposed || generation !== reviewGeneration) return;
      const seen = new Set(entries.map((entry) => heardKey(entry.heard)));
      suggestions = result.filter((suggestion) => {
        const key = heardKey(suggestion.heard);
        if (!key || !suggestion.written.trim() || seen.has(key)) return false;
        seen.add(key);
        return true;
      });
      reviewNotice = suggestions.length
        ? "Add corrections you want to keep, then choose Save words."
        : "No new supported suggestions found in recent dictations.";
    } catch (error: unknown) {
      if (!disposed && generation === reviewGeneration)
        reviewError = errorMessage(error);
    } finally {
      if (!disposed && generation === reviewGeneration) reviewState = "idle";
    }
  }
  async function cancelReview(): Promise<void> {
    if (reviewState !== "reviewing") return;
    reviewGeneration++;
    reviewState = "cancelling";
    reviewError = "";
    try {
      await call("cancel_vocabulary_suggestions");
      if (!disposed) reviewNotice = "Review canceled.";
    } catch (error: unknown) {
      if (!disposed) reviewError = errorMessage(error);
    } finally {
      if (!disposed) reviewState = "idle";
    }
  }
  function addSuggestion(suggestion: VocabularySuggestion): void {
    if (!loaded || app.busy("vocabulary-save") || hasEntry(suggestion.heard))
      return;
    entries = [
      ...entries,
      {
        id: crypto.randomUUID(),
        heard: suggestion.heard.trim(),
        written: suggestion.written.trim(),
        enabled: true,
        scope: null,
      },
    ];
    reviewNotice =
      "Added to unsaved vocabulary. Choose Save words to apply it.";
  }
  function add(): void {
    if (
      !loaded ||
      app.busy("vocabulary-save") ||
      !heard.trim() ||
      !written.trim()
    )
      return;
    if (hasEntry(heard)) {
      app.error =
        "This heard phrase is already in your vocabulary. Edit its existing entry.";
      return;
    }
    const id = crypto.randomUUID();
    entries = [
      ...entries,
      {
        id,
        heard: heard.trim(),
        written: written.trim(),
        enabled: true,
        scope: scope.trim() || null,
      },
    ];
    expandedId = id;
    search = "";
    cancelAdd();
  }
  async function save(): Promise<void> {
    if (entries.some((entry) => !entry.heard.trim() || !entry.written.trim())) {
      app.error = "Every entry needs a heard phrase and its written form.";
      return;
    }
    const cleaned = entries.map((entry) => ({
      ...entry,
      heard: entry.heard.trim(),
      written: entry.written.trim(),
      scope: entry.scope?.trim() || null,
    }));
    if (
      new Set(cleaned.map((entry) => entry.heard.toLowerCase())).size !==
      cleaned.length
    ) {
      app.error =
        "Use each heard phrase only once so its correction is unambiguous.";
      return;
    }
    const result = await app.perform(
      "vocabulary-save",
      () => call("save_vocabulary", { entries: cleaned }),
      "Vocabulary saved.",
    );
    if (result) {
      entries = result;
      saved = JSON.stringify(result);
    }
  }
</script>

<header class="page-heading">
  <h1 class="text-grain" data-text-surface="emboss">Vocabulary</h1>
</header>

{#if view !== "words"}
  <button
    class="button quiet back-button"
    bind:this={backButton}
    onclick={() => showView("words")}
  >
    <Icon name="back" size={18} />Your words
  </button>
{/if}

{#if view === "words"}
  <Card>
    <div class="section-toolbar vocabulary-toolbar">
      <label class="search-field">
        <span class="sr-only">Find a word</span>
        <input
          type="search"
          bind:this={searchInput}
          bind:value={search}
          placeholder="Find a word"
        />
      </label>
      <button
        class="button"
        bind:this={addButton}
        aria-expanded={adding}
        aria-controls="add-word"
        disabled={!loaded || app.busy("vocabulary-save")}
        onclick={toggleAdd}><Icon name="plus" size={19} />Add a word</button
      >
      <details class="more-menu" bind:open={moreOpen}>
        <summary class="button quiet">More</summary>
        <div class="more-actions stock paper-popover paper-menu">
          <button
            class="button quiet small"
            onclick={() => showView("suggestions")}
          >
            Suggestions from recent dictations{reviewing ? " (reviewing…)" : ""}
          </button>
          <button
            class="button quiet small"
            onclick={() => showView("learning")}
          >
            Learning from your fixes{learningDirty ? " (unsaved changes)" : ""}
          </button>
        </div>
      </details>
    </div>

    {#if adding}
      <form
        id="add-word"
        class="add-word"
        onsubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <div class="two-fields">
          <label
            >When it hears<input
              bind:this={heardInput}
              bind:value={heard}
              placeholder="For example, nemotron"
              maxlength="250"
              disabled={!loaded || app.busy("vocabulary-save")}
            /></label
          >
          <label
            >Write it as<input
              bind:value={written}
              placeholder="Nemotron"
              maxlength="250"
              disabled={!loaded || app.busy("vocabulary-save")}
            /></label
          >
        </div>
        <p class="field-help" data-text-surface="flat">
          Single-word casing corrections work without a project or topic.
          Phrases and other spelling changes need supported context.
        </p>
        <details class="topic-fold">
          <summary>Project or topic (optional)</summary>
          <label>
            <span class="sr-only">Project or topic for the new word</span>
            <input
              bind:value={scope}
              placeholder="AI model discussion"
              maxlength="250"
              disabled={!loaded || app.busy("vocabulary-save")}
            />
          </label>
          <p class="field-help" data-text-surface="flat">
            Currently, “AI model discussion” is supported when model-related
            words appear beside the correction. Other topics can be saved but do
            not activate a correction.
          </p>
        </details>
        <div class="actions">
          <button
            class="button primary"
            type="submit"
            disabled={!loaded ||
              !heard.trim() ||
              !written.trim() ||
              app.busy("vocabulary-save")}>Add</button
          >
          <button class="button quiet" type="button" onclick={cancelAdd}
            >Cancel</button
          >
        </div>
        <p class="field-help" data-text-surface="flat">
          Add the word, then choose Save words. Corrections apply in Clean mode;
          original recognition stays separate.
        </p>
      </form>
    {/if}

    {#if !loaded}
      <p class="empty-copy" data-text-surface="flat">
        {app.busy("vocabulary-load")
          ? "Loading your vocabulary…"
          : "Vocabulary could not be loaded."}
      </p>
      {#if !app.busy("vocabulary-load")}<button class="button" onclick={load}
          >Try again</button
        >{/if}
    {:else if !entries.length}
      <p class="empty-copy" data-text-surface="flat">
        No words saved. Choose Add a word to save a correction.
      </p>
    {:else if !filtered.length}
      <p class="empty-copy" data-text-surface="flat">
        No entries match “{search}”.
      </p>
    {:else}
      <ul class="word-list" aria-label="Your vocabulary">
        {#each filtered as entry (entry.id)}
          <li>
            <button
              class="button quiet word-row"
              aria-expanded={expandedId === entry.id}
              aria-controls={`word-editor-${entry.id}`}
              onclick={() =>
                (expandedId = expandedId === entry.id ? null : entry.id)}
            >
              <span class="word-pair"
                ><span>{entry.heard || "When it hears"}</span><span
                  aria-hidden="true"
                >
                  →
                </span><span class="sr-only">becomes </span><strong
                  >{entry.written || "Write it as"}</strong
                ></span
              >
              {#if !entry.enabled}<span
                  class="field-help row-status"
                  data-text-surface="flat">Off</span
                >{/if}
              <span aria-hidden="true"
                >{expandedId === entry.id ? "▾" : "›"}</span
              >
            </button>
            {#if expandedId === entry.id}
              <div class="word-editor" id={`word-editor-${entry.id}`}>
                <div class="two-fields">
                  <label
                    >When it hears<input
                      aria-label={`Heard phrase for ${entry.written}`}
                      bind:value={entry.heard}
                      disabled={app.busy("vocabulary-save")}
                    /></label
                  >
                  <label
                    >Write it as<input
                      aria-label={`Written form for ${entry.heard}`}
                      bind:value={entry.written}
                      disabled={app.busy("vocabulary-save")}
                    /></label
                  >
                </div>
                <details class="topic-fold">
                  <summary>Project or topic (optional)</summary>
                  <label>
                    <span class="sr-only"
                      >Project or topic for {entry.written}</span
                    >
                    <input
                      aria-label={`Project or topic for ${entry.written}`}
                      placeholder="AI model discussion"
                      maxlength="250"
                      value={entry.scope ?? ""}
                      disabled={app.busy("vocabulary-save")}
                      oninput={(event) =>
                        (entry.scope = event.currentTarget.value || null)}
                    />
                  </label>
                  <p class="field-help" data-text-surface="flat">
                    The correction needs nearby words that support this topic.
                    Currently supported: AI model discussion.
                  </p>
                </details>
                <p class="field-help" data-text-surface="flat">
                  {entryStatus(entry)}
                </p>
                <div class="entry-actions">
                  <span>Use this word</span>
                  <Toggle
                    label={`Use correction ${entry.heard}`}
                    checked={entry.enabled}
                    disabled={app.busy("vocabulary-save")}
                    onchange={(enabled) => (entry.enabled = enabled)}
                  />
                  <button
                    class="button quiet small remove-word"
                    disabled={app.busy("vocabulary-save")}
                    onclick={() => {
                      entries = entries.filter((item) => item.id !== entry.id);
                      expandedId = null;
                    }}>Remove</button
                  >
                </div>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if loaded && entries.length}
      <p class="field-help word-count" data-text-surface="flat">
        {entries.length}
        {entries.length === 1 ? "entry" : "entries"}. Eligible saved corrections
        apply in Clean mode.
      </p>
    {/if}
  </Card>
{/if}

{#if view === "suggestions"}
  <Card title="Suggestions from recent dictations">
    <p class="body-copy" data-text-surface="flat">
      {#if aiSettings?.backend === "openrouter"}
        Review sends recent dictation text to OpenRouter{aiSettings.model
          ? ` using ${aiSettings.model}`
          : ""}. <RemoteModelBadge /> Choose the AI in Models.
      {:else if aiSettings?.backend === "local"}
        Uses the notes and text processing model selected in Models. Review text
        stays on this computer.
      {:else}
        Uses the AI selected in Models. Check Models to see whether review stays
        local or sends to OpenRouter.
      {/if}
    </p>
    <div class="actions">
      <button
        class="button"
        disabled={!app.native ||
          !loaded ||
          !reviewReady ||
          reviewing ||
          loadingAi}
        onclick={review}
      >
        {reviewState === "reviewing"
          ? "Reviewing…"
          : "Review recent dictations"}
      </button>
      {#if reviewing}
        <button
          class="button quiet"
          disabled={reviewState === "cancelling"}
          onclick={cancelReview}
          >{reviewState === "cancelling" ? "Canceling…" : "Cancel"}</button
        >
      {/if}
      {#if aiError}<button
          class="button quiet"
          disabled={loadingAi}
          onclick={loadAi}>Retry AI settings</button
        >{/if}
    </div>
    <ModelSummary role="notes" {onmanage} />
    {#if loadingAi}
      <p class="field-help" data-text-surface="flat">Reading AI settings…</p>
    {:else if aiSettings && !reviewReady}
      <p class="field-help" data-text-surface="flat">
        {aiSettings.help ||
          "Choose an available AI in Models to review dictations."}
      </p>
    {/if}
    {#if aiError || reviewError}
      <ErrorBanner
        error={aiError || reviewError}
        onaction={(action) => {
          if (action === "setup") onmanage?.("notes");
          else {
            aiError = "";
            reviewError = "";
          }
        }}
      />
    {/if}
    {#if reviewNotice}<p
        class="field-help"
        data-text-surface="flat"
        role="status"
      >
        {reviewNotice}
      </p>{/if}
    {#if suggestions.length}
      <ul class="suggestions-list" aria-label="Suggested corrections">
        {#each suggestions as suggestion (heardKey(suggestion.heard))}
          <li>
            <div>
              <p class="suggested-words">
                <span>{suggestion.heard}</span><span aria-hidden="true">
                  →
                </span><span class="sr-only">becomes </span><strong
                  >{suggestion.written}</strong
                >
              </p>
              <p class="field-help" data-text-surface="flat">
                {suggestion.reason}
              </p>
            </div>
            <button
              class="button small"
              disabled={!loaded ||
                app.busy("vocabulary-save") ||
                hasEntry(suggestion.heard)}
              aria-label={hasEntry(suggestion.heard)
                ? `${suggestion.written} is in your vocabulary`
                : `Add correction ${suggestion.heard} to ${suggestion.written}`}
              onclick={() => addSuggestion(suggestion)}
              >{hasEntry(suggestion.heard) ? "Added" : "Add"}</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
{/if}

<!-- Keep learning mounted so subview navigation preserves its draft and dirty guard. -->
<div hidden={view !== "learning"}>
  <LocalLearning ondirty={(value) => (learningDirty = value)} />
</div>

{#if dirty}
  <div class="save-bar">
    <span class="field-help" data-text-surface="flat" role="status"
      >You have unsaved word changes.</span
    >
    <button
      class="button primary"
      disabled={!app.native || app.busy("vocabulary-save")}
      onclick={save}
    >
      {app.busy("vocabulary-save") ? "Saving…" : "Save words"}
    </button>
  </div>
{/if}
{#if learningDirty && view !== "learning"}
  <p class="field-help" data-text-surface="flat">
    You have unsaved learning changes.
    <button class="button quiet small" onclick={() => showView("learning")}
      >Review learning</button
    >
  </p>
{/if}

<style>
  .back-button {
    margin-bottom: var(--space-2);
  }
  .vocabulary-toolbar {
    flex-wrap: wrap;
    align-items: flex-start;
  }
  .search-field {
    min-width: min(100%, 180px);
    max-width: none;
  }
  .more-menu {
    flex: none;
    position: relative;
  }
  .more-menu summary {
    display: list-item;
    list-style-position: inside;
  }
  .more-actions {
    position: absolute;
    right: 0;
    top: calc(100% + 9px);
    z-index: 5;
    width: min(340px, 70vw);
  }
  .more-actions .button {
    justify-content: flex-start;
    text-align: left;
  }
  summary {
    cursor: pointer;
  }
  summary:focus-visible {
    outline: 3px solid var(--focus);
    outline-offset: 3px;
  }
  .add-word {
    padding-block: var(--space-2);
    margin-bottom: var(--space-2);
    border-block: 1px solid var(--line);
  }
  .add-word > .field-help:last-child {
    margin-bottom: 0;
  }
  .topic-fold {
    margin-block: var(--space-2);
  }
  .topic-fold > label {
    display: block;
    margin-top: var(--space-1);
  }
  .topic-fold .field-help {
    margin-bottom: 0;
  }
  .word-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .word-list > li + li {
    border-top: 1px solid var(--line);
  }
  .word-row {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    margin-block: var(--space-1);
  }
  .word-pair {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .row-status {
    margin: 0;
  }
  .word-editor {
    padding: var(--space-1) var(--space-2) var(--space-2);
  }
  .entry-actions {
    flex-wrap: wrap;
  }
  .remove-word {
    margin-left: auto;
  }
  .word-count {
    margin-bottom: 0;
  }
  .suggestions-list {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
  }
  .suggestions-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding-block: var(--space-2);
    border-top: 1px solid var(--line);
  }
  .suggestions-list li > div {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .suggestions-list button {
    flex: none;
  }
  .suggested-words {
    margin-bottom: var(--space-1);
  }
  .suggestions-list .field-help {
    margin-bottom: 0;
  }
</style>
