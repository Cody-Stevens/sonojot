<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import LocalLearning from "../components/LocalLearning.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Icon from "../components/Icon.svelte";
  import { cleanupChoice } from "../lib/cleanup-choice";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { call } from "../lib/api";
  import { errorMessage, type VocabularyEntry } from "../lib/protocol";
  import type { VocabularySuggestion } from "../lib/ai";
  import type { ModelsTarget } from "../lib/model-library";
  import {
    parseWord,
    replacementWord,
    validHeardForm,
    wordKey,
  } from "../lib/words";
  let { onmanage }: { onmanage?: (target: ModelsTarget) => void } = $props();
  let tab = $state("all");
  let entries = $state<VocabularyEntry[]>([]);
  let loaded = $state(false);
  let search = $state("");
  let addText = $state("");
  let draft = $state<VocabularyEntry | null>(null);
  let aliases = $state("");
  let near = $state("");
  let learningDirty = $state(false);
  let suggestions = $state<VocabularySuggestion[]>([]);
  let reviewing = $state(false);
  let cancelling = $state(false);
  let reviewNotice = $state("");
  let disposed = false;
  let generation = 0;
  const busy = $derived(app.busy("vocabulary-save"));
  const editorDirty = $derived(
    !!draft &&
      (JSON.stringify(draft) !==
        JSON.stringify(entries.find((item) => item.id === draft?.id)) ||
        aliases !== draft.aliases.join(", ") ||
        near !== draft.context.join(", ")),
  );
  $effect(() => {
    app.dirty = editorDirty || !!addText.trim() || learningDirty;
  });
  const filtered = $derived(
    entries.filter((item) =>
      `${item.heard} ${item.written} ${item.aliases.join(" ")} ${item.scope ?? ""}`
        .toLowerCase()
        .includes(search.toLowerCase()),
    ),
  );
  const reviewReady = $derived(
    ai.data?.backend === "local" ||
      (ai.data?.supported && ai.data.connected && !!ai.data.model),
  );
  onMount(() => {
    void load();
    void ai.refresh();
  });
  onDestroy(() => {
    disposed = true;
    generation++;
    if (reviewing) void call("cancel_vocabulary_suggestions").catch(() => {});
    app.dirty = false;
  });
  async function load() {
    const result = await app.perform("vocabulary-load", () =>
      call("get_vocabulary"),
    );
    if (result) {
      entries = result;
      loaded = true;
    }
  }
  async function persist(next: VocabularyEntry[]): Promise<boolean> {
    if (busy || !loaded) return false;
    if (
      next.some(
        (item) =>
          !item.heard.trim() ||
          !item.written.trim() ||
          item.aliases.some((alias) => !validHeardForm(alias)),
      )
    ) {
      app.error =
        "Each entry needs both forms. Heard aliases must contain 1–4 words.";
      return false;
    }
    const result = await app.perform("vocabulary-save", () =>
      call("save_vocabulary", { entries: next }),
    );
    if (!result) return false;
    entries = result;
    return true;
  }
  async function add(
    input: string | Pick<VocabularySuggestion, "heard" | "written"> = addText,
    clearInput = true,
  ): Promise<boolean> {
    try {
      const parsed =
        typeof input === "string"
          ? parseWord(input)
          : replacementWord(input.heard, input.written);
      if (
        entries.some((entry) => wordKey(entry.heard) === wordKey(parsed.heard))
      )
        throw new Error(
          "This heard phrase is already saved. Open its row to edit it.",
        );
      if (
        !(await persist([...entries, { ...parsed, id: crypto.randomUUID() }]))
      )
        return false;
      if (clearInput) addText = "";
      return true;
    } catch (error) {
      app.error = errorMessage(error);
      return false;
    }
  }
  async function select(entry: VocabularyEntry) {
    if (
      editorDirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This word has unsaved changes.",
        "Leave changes",
      ))
    )
      return;
    if (busy) return;
    draft =
      draft?.id === entry.id ? null : structuredClone($state.snapshot(entry));
    aliases = draft?.aliases.join(", ") ?? "";
    near = draft?.context.join(", ") ?? "";
  }
  async function done() {
    if (!draft) return;
    const previous = entries.find((item) => item.id === draft?.id);
    let heardForms = aliases
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
    // Changing an explicit replacement changes its confirmed primary alias too.
    if (
      previous?.aliases.includes(previous.heard) &&
      draft.heard !== previous.heard
    )
      heardForms = [
        draft.heard.trim(),
        ...heardForms.filter((value) => value !== previous.heard),
      ];
    const next = {
      ...$state.snapshot(draft),
      heard: draft.kind === "hint" ? draft.written.trim() : draft.heard.trim(),
      written: draft.written.trim(),
      aliases: draft.kind === "hint" ? [] : [...new Set(heardForms)],
      context:
        draft.kind === "hint" ? [] : near.split(/[,\s]+/).filter(Boolean),
      scope: draft.kind === "hint" ? null : draft.scope?.trim() || null,
    };
    if (
      await persist(entries.map((item) => (item.id === next.id ? next : item)))
    )
      draft = null;
  }
  async function remove() {
    if (!draft || busy) return;
    const id = draft.id;
    if (
      !(await app.confirm(
        "Delete this word?",
        `Remove “${draft.written}” from your words? Saved dictations stay unchanged.`,
        "Delete word",
        true,
      ))
    )
      return;
    if (await persist(entries.filter((item) => item.id !== id))) draft = null;
  }
  async function review() {
    if (!reviewReady || reviewing) return;
    const current = ++generation;
    reviewing = true;
    reviewNotice = "";
    try {
      const result = await call("suggest_vocabulary");
      if (disposed || current !== generation) return;
      const seen = new Set(entries.map((item) => wordKey(item.heard)));
      suggestions = result.filter((item) => {
        const key = wordKey(item.heard);
        if (!key || !item.written.trim() || seen.has(key)) return false;
        seen.add(key);
        return true;
      });
      reviewNotice = suggestions.length
        ? "Choose the corrections you want to keep."
        : "No new supported suggestions found.";
    } catch (error) {
      if (!disposed && current === generation) app.error = errorMessage(error);
    } finally {
      if (current === generation) reviewing = false;
    }
  }
  async function cancelReview() {
    generation++;
    cancelling = true;
    await app.perform("vocabulary-cancel", () =>
      call("cancel_vocabulary_suggestions"),
    );
    cancelling = false;
    reviewing = false;
    reviewNotice = "Review canceled.";
  }
</script>

<header class="page-heading words-heading">
  <h1 data-text-surface="emboss">Words</h1>
  <div class="segments" aria-label="Word views">
    {#each [["all", `All ${entries.length}`], ["learned", "Learned"], ["suggested", "Suggested"]] as [value, label]}
      <button aria-pressed={tab === value} onclick={() => (tab = value)}
        >{label}</button
      >
    {/each}
  </div>
</header>
{#if tab === "all"}
  <section class="stock paper-card words-paper" aria-label="Your words">
    <div class="word-wells">
      <label class="paper-well search-well"
        ><Icon name="search" size={18} /><input
          aria-label="Search words"
          type="search"
          placeholder="Search words"
          bind:value={search}
        /></label
      >
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void add();
        }}
      >
        <label class="paper-well search-well"
          ><Icon name="plus" size={18} /><input
            aria-label="Add a word"
            placeholder="Add: a word, or heard → written"
            bind:value={addText}
            disabled={!loaded || busy}
          /></label
        >
      </form>
    </div>
    {#if cleanupChoice(app.snapshot.settings, app.snapshot.localProofreaderAvailable).kind === "off"}<p
        class="field-help"
      >
        Hints help when a model does the cleanup.
      </p>{/if}
    {#if !loaded}<p>Loading words…</p>
      <button class="button" onclick={load}>Try again</button>
    {:else if !filtered.length}<p class="empty">
        {search
          ? "No matching words."
          : "Add a word or replacement above. Enter saves it."}
      </p>{/if}
    {#each filtered as entry (entry.id)}
      {#if draft?.id === entry.id}
        <form
          class="word-editor"
          onsubmit={(event) => {
            event.preventDefault();
            void done();
          }}
        >
          <div class="editor-pair">
            {#if draft.kind !== "hint"}<label
                ><span class="sr-only">Heard</span><input
                  aria-label="Heard"
                  bind:value={draft.heard}
                  disabled={busy}
                /></label
              ><span>→</span>{:else}<span class="quiet-tag">HINT</span
              >{/if}<label
              ><span class="sr-only">Written</span><input
                aria-label="Written"
                bind:value={draft.written}
                disabled={busy}
              /></label
            ><Toggle
              label="Use this word"
              checked={draft.enabled}
              disabled={busy}
              onchange={(value) => {
                if (draft) draft.enabled = value;
              }}
            /><button
              class="button"
              type="button"
              onclick={remove}
              disabled={busy}>Delete</button
            >
          </div>
          {#if draft.kind !== "hint"}<label class="editor-line"
              ><span>Also hears</span><input
                aria-label="Also hears"
                placeholder="Comma-separated heard forms"
                bind:value={aliases}
                disabled={busy}
              /></label
            >
            <div class="editor-line">
              <span>Only for</span><input
                aria-label="Topic"
                placeholder="Topic (optional)"
                value={draft.scope ?? ""}
                oninput={(event) => {
                  if (draft) draft.scope = event.currentTarget.value || null;
                }}
                disabled={busy}
              /><input
                aria-label="Near words"
                placeholder="Near words (optional)"
                bind:value={near}
                disabled={busy}
              />
            </div>
            <p class="field-help">
              Confirmed heard forms apply in Format and Clean. Common words need
              a nearby context word. Topic rules currently support “AI model
              discussion”.
            </p>{:else}<p class="field-help">
              A spelling the cleanup model can consider when recognition sounds
              wrong. It is not an automatic replacement.
            </p>{/if}
          <div class="editor-actions">
            <button
              class="button"
              type="button"
              disabled={busy}
              onclick={() => (draft = null)}>Cancel</button
            ><button class="button primary" type="submit" disabled={busy}
              >Done</button
            >
          </div>
        </form>
      {:else}
        <button
          class="word-row"
          aria-label={entry.kind === "hint"
            ? `${entry.written} hint`
            : `${entry.heard} becomes ${entry.written}`}
          aria-expanded="false"
          onclick={() => select(entry)}
          >{#if entry.kind !== "hint"}<span>{entry.heard}</span><span
              aria-hidden="true">→</span
            >{:else}<span><small class="quiet-tag">HINT</small></span
            >{/if}<strong data-text-surface="emboss">{entry.written}</strong
          >{#if entry.scope}<small class="stamp">{entry.scope}</small
            >{/if}{#if !entry.enabled}<small class="stamp">Off</small
            >{/if}</button
        >
      {/if}
    {/each}
  </section>
{:else if tab === "suggested"}
  <section class="stock paper-card words-paper">
    <div class="review-head">
      <span data-text-surface="flat"
        >From recent dictations · {ai.data?.backend === "openrouter"
          ? "sent to OpenRouter for review"
          : "reviewed on this computer"}</span
      ><button
        class="button"
        disabled={!loaded || !reviewReady || reviewing}
        onclick={review}
        >{reviewing ? "Reviewing…" : "Review recent dictations"}</button
      >{#if reviewing}<button
          class="button"
          disabled={cancelling}
          onclick={cancelReview}>Cancel</button
        >{/if}
    </div>
    {#if !reviewReady}<button class="button" onclick={() => onmanage?.("notes")}
        >Choose a review model</button
      >{/if}
    <p role="status" class="field-help">{reviewNotice}</p>
    {#each suggestions as suggestion}
      <div class="suggestion-row">
        <span>{suggestion.heard}</span><span>→</span><strong
          >{suggestion.written}</strong
        ><button
          class="button"
          disabled={busy}
          onclick={async () => {
            if (await add(suggestion, false))
              suggestions = suggestions.filter((item) => item !== suggestion);
          }}>Add</button
        ><button
          class="button"
          onclick={() =>
            (suggestions = suggestions.filter((item) => item !== suggestion))}
          >Skip</button
        >
      </div>
    {/each}
  </section>
{/if}
<div hidden={tab !== "learned"}>
  <LocalLearning ondirty={(value) => (learningDirty = value)} />
</div>
{#if tab !== "all" && editorDirty}<button
    class="button"
    onclick={() => (tab = "all")}>Review unsaved word</button
  >{/if}
{#if tab !== "learned" && learningDirty}<button
    class="button"
    onclick={() => (tab = "learned")}>Review unsaved learning</button
  >{/if}

<style>
  .words-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
  }
  .words-paper {
    padding: 18px 22px;
  }
  .word-wells {
    display: flex;
    gap: 12px;
    margin-bottom: 16px;
  }
  .word-wells > label {
    flex: 1;
  }
  .word-wells form {
    flex: 1.25;
  }
  .search-well {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 12px;
    height: 46px;
  }
  .search-well input {
    margin: 0;
    width: 100%;
    background: none;
    box-shadow: none;
    border: 0;
    padding: 0;
    min-width: 0;
  }
  .word-row,
  .suggestion-row {
    display: flex;
    width: 100%;
    gap: 18px;
    align-items: center;
    border: 0;
    border-top: 1px solid var(--line);
    padding: 16px 8px;
    background: transparent;
    text-align: left;
  }
  .word-row > span:first-child,
  .suggestion-row > span:first-child {
    width: 200px;
    color: var(--soft-ink);
  }
  .word-row strong,
  .suggestion-row strong {
    flex: 1;
  }
  .word-row:hover {
    background: var(--well);
    border-radius: 6px;
  }
  .word-editor {
    padding: 16px;
    border-radius: 10px;
    box-shadow: var(--raise-sm);
    margin: 9px -6px;
  }
  .editor-pair,
  .editor-line {
    display: flex;
    gap: 12px;
    align-items: center;
    margin-bottom: 15px;
  }
  .editor-pair label {
    flex: 1;
  }
  .editor-pair input {
    width: 100%;
  }
  .editor-line > span {
    width: 120px;
    flex: none;
  }
  .editor-line input {
    flex: 1;
    min-width: 0;
  }
  .editor-actions {
    display: flex;
    gap: 9px;
    justify-content: flex-end;
  }
  .review-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .review-head > span {
    flex: 1;
    font-size: 14px;
  }
  .empty {
    color: var(--soft-ink);
    padding: 18px 8px;
  }
  @media (max-width: 850px) {
    .word-wells,
    .editor-pair {
      flex-wrap: wrap;
    }
    .word-row > span:first-child {
      width: 130px;
    }
    .word-wells form {
      min-width: 250px;
    }
  }
</style>
