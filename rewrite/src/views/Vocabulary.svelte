<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import PageHeader from "../components/quiet/PageHeader.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
  import SearchBar from "../components/quiet/SearchBar.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SyncWords from "../components/SyncWords.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Icon from "../components/Icon.svelte";
  import { cleanupChoice } from "../lib/cleanup-choice";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { call } from "../lib/api";
  import {
    errorMessage,
    type ConfirmedWordMeta,
    type LearnedItemView,
    type SpellingClusterView,
    type VocabularyEntry,
    type WordsOverview,
  } from "../lib/protocol";
  import type { ModelsTarget } from "../lib/model-library";
  import { parseWord, validHeardForm, wordKey } from "../lib/words";

  let { onmanage }: { onmanage?: (target: ModelsTarget) => void } = $props();

  let tab = $state<"words" | "review">("words");
  let entries = $state<VocabularyEntry[]>([]);
  let wordMeta = $state<Record<string, ConfirmedWordMeta>>({});
  let activeLearned = $state<LearnedItemView[]>([]);
  let reviewItems = $state<LearnedItemView[]>([]);
  let clusters = $state<SpellingClusterView[]>([]);
  let suppressedCount = $state(0);
  let loaded = $state(false);
  let search = $state("");
  let addText = $state("");
  let draft = $state<VocabularyEntry | null>(null);
  let aliases = $state("");
  let near = $state("");
  let pendingSyncPull = $state(false);

  // Expanded learned item (either active in Words or pending in Review)
  let openLearnedId = $state<string | null>(null);
  let learnedHeard = $state("");
  let learnedWritten = $state("");
  let learnedNear = $state("");
  let learnedKind = $state<"replace" | "hint">("replace");

  // Immediate Undo notice after Confirm / Ignore
  let undoToken = $state<string | null>(null);
  let undoMessage = $state("");

  // On-demand recent dictations review
  let reviewing = $state(false);
  let cancelling = $state(false);
  let reviewNotice = $state("");
  let disposed = false;
  let generation = 0;

  const busy = $derived(
    app.busy("vocabulary-save") ||
      app.busy("lexicon-action") ||
      app.busy("learning-reset"),
  );
  const editorDirty = $derived(
    !!draft &&
      (JSON.stringify(draft) !==
        JSON.stringify(entries.find((item) => item.id === draft?.id)) ||
        aliases !== draft.aliases.join(", ") ||
        near !== draft.context.join(", ")),
  );
  $effect(() => {
    app.dirty = editorDirty || !!addText.trim();
  });

  const inertLegacyWords = $derived(
    entries.filter((entry) => wordMeta[entry.id]?.inertLegacy),
  );
  const totalWordsCount = $derived(entries.length + activeLearned.length);
  const reviewCount = $derived(
    reviewItems.length + clusters.length + inertLegacyWords.length,
  );
  const viewOptions = $derived([
    { value: "words" as const, label: `Words ${totalWordsCount}` },
    {
      value: "review" as const,
      label: reviewCount > 0 ? `Review ${reviewCount}` : "Review",
    },
  ]);

  const query = $derived(search.trim().toLowerCase());
  const filteredActiveLearned = $derived(
    activeLearned.filter((item) =>
      `${item.heard ?? ""} ${item.written} ${item.reason}`
        .toLowerCase()
        .includes(query),
    ),
  );
  const filteredEntries = $derived(
    entries.filter((item) =>
      `${item.heard} ${item.written} ${item.aliases.join(" ")} ${item.scope ?? ""}`
        .toLowerCase()
        .includes(query),
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

  function applyOverview(overview: WordsOverview) {
    entries = overview.words;
    wordMeta = overview.wordMeta ?? {};
    activeLearned = overview.activeLearned ?? [];
    reviewItems = overview.reviewItems ?? [];
    clusters = overview.clusters ?? [];
    suppressedCount = overview.suppressedCount ?? 0;
    loaded = true;
  }

  async function load() {
    const result = await app.perform("vocabulary-load", async () => {
      try {
        return await call("get_words_overview");
      } catch {
        const words = await call("get_vocabulary");
        return {
          words,
          wordMeta: {},
          activeLearned: [],
          reviewItems: [],
          clusters: [],
          suppressedCount: 0,
          lexiconRevision: 0,
        } satisfies WordsOverview;
      }
    });
    if (result) applyOverview(result);
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
    await load();
    return true;
  }

  async function add(input: string = addText): Promise<boolean> {
    try {
      const parsed = parseWord(input);
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
      addText = "";
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
    openLearnedId = null;
    draft =
      draft?.id === entry.id ? null : structuredClone($state.snapshot(entry));
    aliases = draft?.aliases.join(", ") ?? "";
    near = draft?.context.join(", ") ?? "";
    if (!draft && pendingSyncPull) {
      pendingSyncPull = false;
      void load();
    }
  }

  function toggleLearnedPocket(item: LearnedItemView) {
    if (busy) return;
    draft = null;
    if (openLearnedId === item.id) {
      openLearnedId = null;
      return;
    }
    openLearnedId = item.id;
    learnedHeard = item.heard ?? "";
    learnedWritten = item.written;
    learnedNear = item.nearCues.join(", ");
    learnedKind = item.kind === "hint" || !item.heard ? "hint" : "replace";
  }

  async function done() {
    if (!draft) return;
    const previous = entries.find((item) => item.id === draft?.id);
    let heardForms = aliases
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean);
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
    ) {
      draft = null;
      if (pendingSyncPull) {
        pendingSyncPull = false;
        void load();
      }
    }
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

  async function confirmLearned(
    item: LearnedItemView,
    fromPocket = false,
  ): Promise<void> {
    if (busy) return;
    const kind = fromPocket
      ? learnedKind
      : item.kind === "hint" || !item.heard
        ? "hint"
        : "replace";
    const written = (fromPocket ? learnedWritten : item.written).trim();
    const heard =
      kind === "hint"
        ? null
        : (fromPocket ? learnedHeard : (item.heard ?? "")).trim() || null;
    const context = (fromPocket ? learnedNear.split(/[,\s]+/) : item.nearCues)
      .map((w) => w.trim())
      .filter(Boolean);
    const result = await app.perform("lexicon-action", () =>
      call("confirm_learned_word", {
        id: item.id,
        expectedRevision: item.revision,
        heard,
        written,
        context,
        kind,
      }),
    );
    if (!result) return;
    openLearnedId = null;
    applyOverview(result.overview);
    undoToken = result.undoToken;
    undoMessage = result.message;
  }

  async function ignoreLearned(
    item: LearnedItemView,
    target: "mapping" | "term" = item.heard ? "mapping" : "term",
  ): Promise<void> {
    if (busy) return;
    const result = await app.perform("lexicon-action", () =>
      call("ignore_learned_word", {
        id: item.id,
        expectedRevision: item.revision,
        target,
      }),
    );
    if (!result) return;
    if (openLearnedId === item.id) openLearnedId = null;
    applyOverview(result.overview);
    undoToken = result.undoToken;
    undoMessage = result.message;
  }

  async function confirmCluster(
    cluster: SpellingClusterView,
    chosen: string,
  ): Promise<void> {
    if (busy) return;
    const result = await app.perform("lexicon-action", () =>
      call("confirm_spelling_cluster", {
        chosen,
        variants: cluster.variants,
      }),
    );
    if (!result) return;
    applyOverview(result.overview);
    undoToken = result.undoToken;
    undoMessage = result.message;
  }

  async function ignoreCluster(cluster: SpellingClusterView): Promise<void> {
    if (busy) return;
    const result = await app.perform("lexicon-action", () =>
      call("ignore_spelling_cluster", { variants: cluster.variants }),
    );
    if (!result) return;
    applyOverview(result.overview);
    undoToken = result.undoToken;
    undoMessage = result.message;
  }

  async function activateLegacyWord(
    entry: VocabularyEntry,
    asHint: boolean,
  ): Promise<void> {
    const next: VocabularyEntry = asHint
      ? {
          ...entry,
          kind: "hint",
          heard: entry.written,
          aliases: [],
          context: [],
          scope: null,
        }
      : {
          ...entry,
          kind: "replace",
          aliases: [entry.heard],
        };
    await persist(entries.map((item) => (item.id === next.id ? next : item)));
  }

  async function undoLast(): Promise<void> {
    if (!undoToken || busy) return;
    const token = undoToken;
    const overview = await app.perform("lexicon-action", () =>
      call("undo_lexicon_action", { token }),
    );
    if (!overview) return;
    applyOverview(overview);
    undoToken = null;
    undoMessage = "";
  }

  async function resetLearning(): Promise<void> {
    if (
      !(await app.confirm(
        "Reset local learning?",
        "This clears learned suggestions, active local hints, and ignored-word memories on this computer. Your saved Words, dictations, and meetings stay unchanged.",
        "Reset learning",
        true,
      ))
    )
      return;
    await app.perform(
      "learning-reset",
      async () => {
        await call("reset_learning", { confirmed: true });
        undoToken = null;
        undoMessage = "";
        openLearnedId = null;
        await load();
      },
      "Local learning reset.",
    );
  }

  async function reviewRecent() {
    if (!reviewReady || reviewing) return;
    const current = ++generation;
    reviewing = true;
    reviewNotice = "";
    try {
      const result = await call("suggest_vocabulary");
      if (disposed || current !== generation) return;
      await load();
      reviewNotice = result.length
        ? `Added ${result.length} suggestion${result.length === 1 ? "" : "s"} to Review.`
        : "No new suggestions found in recent dictations.";
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

<PageHeader title="Words">
  {#snippet actions()}
    <SegmentedControl
      label="Word views"
      options={viewOptions}
      value={tab}
      onchange={(next) => (tab = next)}
    />
  {/snippet}
</PageHeader>

{#if undoMessage}
  <div class="stock paper-card ui-card undo-strip" role="status">
    <span data-text-surface="flat">{undoMessage}</span>
    {#if undoToken}
      <button
        type="button"
        class="button small"
        disabled={busy}
        onclick={undoLast}>Undo</button
      >
    {/if}
    <button
      type="button"
      class="icon-button"
      aria-label="Dismiss notice"
      onclick={() => {
        undoMessage = "";
        undoToken = null;
      }}><Icon name="close" size={14} /></button
    >
  </div>
{/if}

{#if tab === "words"}
  <div class="word-toolbar">
    <div class="word-search">
      <SearchBar
        label="Search words"
        placeholder="Search words"
        bind:value={search}
      />
    </div>
    <form
      class="word-add-form"
      onsubmit={(event) => {
        event.preventDefault();
        void add();
      }}
    >
      <label class="add-well"
        ><span class="add-icon" aria-hidden="true"
          ><Icon name="plus" size={16} /></span
        ><input
          aria-label="Add a word"
          placeholder="Add: a word, or heard → written"
          bind:value={addText}
          disabled={!loaded || busy}
        /></label
      >
    </form>
  </div>
  {#if cleanupChoice(app.snapshot.settings, app.snapshot.localProofreaderAvailable).kind === "off"}
    <p class="field-help hints-note" data-text-surface="flat">
      Hints help when a model does the cleanup.
    </p>
  {/if}
  <section
    class="stock paper-card ui-card flush words-paper"
    aria-label="Your words"
  >
    {#if !loaded}
      <div class="words-empty">
        <p>Loading words…</p>
        <button class="button small" onclick={load}>Try again</button>
      </div>
    {:else if !filteredEntries.length && !filteredActiveLearned.length}
      <p class="words-empty" data-text-surface="flat">
        {search
          ? "No matching words."
          : "Add a word or replacement above. Enter saves it."}
      </p>
    {:else}
      {#each filteredActiveLearned as item (item.id)}
        <div class="day-rule word-entry" data-learned-id={item.id}>
          <button
            type="button"
            class="word-row"
            class:open={openLearnedId === item.id}
            aria-label={item.heard
              ? `${item.heard} becomes ${item.written} (learned)`
              : `${item.written} learned hint`}
            aria-expanded={openLearnedId === item.id}
            onclick={() => toggleLearnedPocket(item)}
            >{#if item.heard}<span class="word-heard">{item.heard}</span><span
                class="word-arrow"
                aria-hidden="true">→</span
              >{:else}<span class="word-heard"
                ><small class="quiet-tag">HINT</small></span
              ><span class="word-arrow" aria-hidden="true"></span>{/if}<strong
              class="word-written"
              data-text-surface="emboss">{item.written}</strong
            ><span class="word-tags"><small class="stamp">LEARNED</small></span
            ><span
              class="row-chevron"
              class:open={openLearnedId === item.id}
              aria-hidden="true"><Icon name="chevron-right" size={14} /></span
            ></button
          >
          {#if openLearnedId === item.id}
            <form
              class="row-pocket word-editor"
              onsubmit={(event) => {
                event.preventDefault();
                void confirmLearned(item, true);
              }}
            >
              <p class="field-help provenance-line" data-text-surface="flat">
                {item.reason}{#if item.probationLeft > 0}
                  · Highlighted on its next {item.probationLeft} use{item.probationLeft ===
                  1
                    ? ""
                    : "s"}{/if}
              </p>
              {#if item.sources.length}
                <div class="source-pills" data-text-surface="flat">
                  {#each item.sources as src (src.id)}
                    <small class="stamp"
                      >{src.date ? `${src.date} · ` : ""}{src.label}</small
                    >
                  {/each}
                </div>
              {/if}
              <div class="editor-pair">
                {#if learnedKind !== "hint"}<label
                    ><span class="sr-only">Heard</span><input
                      aria-label="Heard"
                      bind:value={learnedHeard}
                      disabled={busy}
                    /></label
                  ><span class="word-arrow" aria-hidden="true">→</span
                  >{:else}<span class="quiet-tag">HINT</span>{/if}<label
                  ><span class="sr-only">Written</span><input
                    aria-label="Written"
                    bind:value={learnedWritten}
                    disabled={busy}
                  /></label
                >
              </div>
              {#if learnedKind !== "hint"}
                <label class="editor-line"
                  ><span>Only near</span><input
                    aria-label="Only near"
                    placeholder="Optional nearby context words"
                    bind:value={learnedNear}
                    disabled={busy}
                  /></label
                >
              {/if}
              <div class="editor-actions">
                <button
                  class="button small"
                  type="button"
                  disabled={busy}
                  onclick={() => void ignoreLearned(item)}>Don’t learn</button
                ><button
                  class="button small"
                  type="button"
                  disabled={busy}
                  onclick={() => (openLearnedId = null)}>Cancel</button
                ><button
                  class="button small primary"
                  type="submit"
                  disabled={busy || !learnedWritten.trim()}
                  >Keep in Words</button
                >
              </div>
            </form>
          {/if}
        </div>
      {/each}

      {#each filteredEntries as entry (entry.id)}
        {@const meta = wordMeta[entry.id]}
        <div class="day-rule word-entry" data-word-id={entry.id}>
          <button
            class="word-row"
            class:open={draft?.id === entry.id}
            aria-label={entry.kind === "hint"
              ? `${entry.written} hint`
              : `${entry.heard} becomes ${entry.written}`}
            aria-expanded={draft?.id === entry.id}
            onclick={() => select(entry)}
            >{#if entry.kind !== "hint"}<span class="word-heard"
                >{entry.heard}</span
              ><span class="word-arrow" aria-hidden="true">→</span>{:else}<span
                class="word-heard"><small class="quiet-tag">HINT</small></span
              ><span class="word-arrow" aria-hidden="true"></span>{/if}<strong
              class="word-written"
              data-text-surface="emboss">{entry.written}</strong
            ><span class="word-tags"
              >{#if meta?.needsCheck}<small class="stamp">CHECK</small
                >{/if}{#if entry.scope}<small class="stamp">{entry.scope}</small
                >{/if}{#if !entry.enabled}<small class="stamp">Off</small
                >{/if}</span
            ><span
              class="row-chevron"
              class:open={draft?.id === entry.id}
              aria-hidden="true"><Icon name="chevron-right" size={14} /></span
            ></button
          >
          {#if draft?.id === entry.id}
            <form
              class="row-pocket word-editor"
              onsubmit={(event) => {
                event.preventDefault();
                void done();
              }}
            >
              {#if meta && meta.applied > 0}
                <p class="field-help provenance-line" data-text-surface="flat">
                  Used {meta.applied} time{meta.applied === 1
                    ? ""
                    : "s"}{#if meta.needsCheck}
                    · Reverted recently{/if}
                </p>
              {/if}
              <div class="editor-pair">
                {#if draft.kind !== "hint"}<label
                    ><span class="sr-only">Heard</span><input
                      aria-label="Heard"
                      bind:value={draft.heard}
                      disabled={busy}
                    /></label
                  ><span class="word-arrow" aria-hidden="true">→</span
                  >{:else}<span class="quiet-tag">HINT</span>{/if}<label
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
                  class="button small"
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
                      if (draft)
                        draft.scope = event.currentTarget.value || null;
                    }}
                    disabled={busy}
                  /><input
                    aria-label="Near words"
                    placeholder="Near words (optional)"
                    bind:value={near}
                    disabled={busy}
                  />
                </div>
                {#if meta?.extraAliases?.length}
                  <div class="extra-aliases">
                    <span class="field-help" data-text-surface="flat"
                      >Also heard as:</span
                    >
                    {#each meta.extraAliases as extra (extra.id)}
                      <span class="alias-chip">
                        <span>{extra.heard ?? extra.written}</span>
                        <button
                          type="button"
                          class="text-action"
                          disabled={busy}
                          onclick={() => void confirmLearned(extra, false)}
                          >Add</button
                        >
                        <button
                          type="button"
                          class="text-action"
                          disabled={busy}
                          onclick={() => void ignoreLearned(extra)}
                          >Ignore</button
                        >
                      </span>
                    {/each}
                  </div>
                {/if}
                <p class="field-help" data-text-surface="flat">
                  Confirmed heard forms apply in Format and Clean. Common
                  English words need a nearby context word.
                </p>{:else}<p class="field-help" data-text-surface="flat">
                  A spelling the cleanup model and doubtful-word choices can
                  consider. It is not an automatic replacement.
                </p>{/if}
              <div class="editor-actions">
                <button
                  class="button small"
                  type="button"
                  disabled={busy}
                  onclick={() => (draft = null)}>Cancel</button
                ><button
                  class="button small primary"
                  type="submit"
                  disabled={busy}>Done</button
                >
              </div>
            </form>
          {/if}
        </div>
      {/each}
    {/if}
  </section>
{:else}
  <section
    class="stock paper-card ui-card flush learning-settings"
    aria-label="Learning settings"
  >
    <div class="learning-head">
      <SettingRow
        label="Learn words on this device"
        help={app.snapshot.settings.doNotSaveDictations
          ? "Automatic learning is paused while Save dictation history is off. Your saved Words still apply."
          : "Notice recurring names and remember corrections you make on this device. Turning it off stops learning and pauses unconfirmed entries; saved Words stay active."}
      >
        <Toggle
          label="Learn words on this device"
          checked={app.snapshot.settings.learnFromCorrections}
          disabled={!app.native || app.busy("settings")}
          onchange={(learnFromCorrections) =>
            app.saveSettings({ learnFromCorrections })}
        />
      </SettingRow>
      {#if app.snapshot.settings.learnFromCorrections}
        <div class="day-rule">
          <SettingRow
            label="Include saved meetings"
            help="Also notice recurring terms and transcript edits in saved meetings (counted once per meeting)."
          >
            <Toggle
              label="Include saved meetings"
              checked={app.snapshot.settings.learnFromMeetings}
              disabled={!app.native || app.busy("settings")}
              onchange={(learnFromMeetings) =>
                app.saveSettings({ learnFromMeetings })}
            />
          </SettingRow>
        </div>
      {/if}
    </div>
  </section>

  <section
    class="stock paper-card ui-card flush review-paper"
    aria-label="Words to review"
  >
    {#if !loaded}
      <p class="words-empty" data-text-surface="flat">Loading review…</p>
    {:else if !clusters.length && !reviewItems.length && !inertLegacyWords.length}
      <p class="words-empty" data-text-surface="flat">
        Nothing waiting to review. As you dictate and fix words, suggestions
        appear here.
      </p>
    {:else}
      {#each clusters as cluster (cluster.key)}
        <div class="day-rule review-row cluster-row">
          <div class="review-main">
            <strong class="word-written" data-text-surface="emboss"
              >Which spelling is right?</strong
            >
            <span class="field-help" data-text-surface="flat"
              >{cluster.reason}</span
            >
            <div class="cluster-chips">
              {#each cluster.variants as variant (variant)}
                <button
                  type="button"
                  class="button small"
                  disabled={busy}
                  onclick={() => void confirmCluster(cluster, variant)}
                  >{variant}</button
                >
              {/each}
              <button
                type="button"
                class="text-action"
                disabled={busy}
                onclick={() => void ignoreCluster(cluster)}>Ignore all</button
              >
            </div>
          </div>
        </div>
      {/each}

      {#each reviewItems as item (item.id)}
        <div class="day-rule review-entry" data-review-id={item.id}>
          <div class="review-row">
            <div class="review-main">
              <div class="review-pair">
                {#if item.heard}
                  <span class="word-heard">{item.heard}</span>
                  <span class="word-arrow" aria-hidden="true">→</span>
                {:else}
                  <small class="quiet-tag">HINT</small>
                {/if}
                <strong class="word-written" data-text-surface="emboss"
                  >{item.written}</strong
                >
              </div>
              <span class="field-help" data-text-surface="flat"
                >{item.reason}</span
              >
            </div>
            <div class="review-actions">
              <button
                type="button"
                class="button small primary"
                disabled={busy}
                onclick={() => void confirmLearned(item, false)}>Keep</button
              ><button
                type="button"
                class="button small"
                disabled={busy}
                aria-expanded={openLearnedId === item.id}
                onclick={() => toggleLearnedPocket(item)}>Edit</button
              ><button
                type="button"
                class="button small"
                disabled={busy}
                onclick={() => void ignoreLearned(item)}>Ignore</button
              >
            </div>
          </div>
          {#if openLearnedId === item.id}
            <form
              class="row-pocket word-editor"
              onsubmit={(event) => {
                event.preventDefault();
                void confirmLearned(item, true);
              }}
            >
              <div class="editor-pair">
                {#if learnedKind !== "hint"}<label
                    ><span class="sr-only">Heard</span><input
                      aria-label="Heard"
                      bind:value={learnedHeard}
                      disabled={busy}
                    /></label
                  ><span class="word-arrow" aria-hidden="true">→</span
                  >{:else}<span class="quiet-tag">HINT</span>{/if}<label
                  ><span class="sr-only">Written</span><input
                    aria-label="Written"
                    bind:value={learnedWritten}
                    disabled={busy}
                  /></label
                >
              </div>
              {#if learnedKind !== "hint"}
                <label class="editor-line"
                  ><span>Only near</span><input
                    aria-label="Only near"
                    placeholder={item.risk === "ordinary"
                      ? "Required for common words (e.g. project, app)"
                      : "Optional nearby context words"}
                    bind:value={learnedNear}
                    disabled={busy}
                  /></label
                >
              {/if}
              {#if item.sources.length}
                <div class="source-pills" data-text-surface="flat">
                  {#each item.sources as src (src.id)}
                    <small class="stamp"
                      >{src.date ? `${src.date} · ` : ""}{src.label}</small
                    >
                  {/each}
                </div>
              {/if}
              <div class="editor-actions">
                <button
                  class="button small"
                  type="button"
                  disabled={busy}
                  onclick={() => (openLearnedId = null)}>Cancel</button
                ><button
                  class="button small primary"
                  type="submit"
                  disabled={busy || !learnedWritten.trim()}
                  >Save to Words</button
                >
              </div>
            </form>
          {/if}
        </div>
      {/each}

      {#each inertLegacyWords as entry (entry.id)}
        <div class="day-rule review-row">
          <div class="review-main">
            <div class="review-pair">
              <span class="word-heard">{entry.heard}</span>
              <span class="word-arrow" aria-hidden="true">→</span>
              <strong class="word-written" data-text-surface="emboss"
                >{entry.written}</strong
              >
            </div>
            <span class="field-help" data-text-surface="flat"
              >Saved earlier, not currently replacing automatically.</span
            >
          </div>
          <div class="review-actions">
            <button
              type="button"
              class="button small primary"
              disabled={busy}
              onclick={() => void activateLegacyWord(entry, false)}
              >Replace when heard</button
            ><button
              type="button"
              class="button small"
              disabled={busy}
              onclick={() => void activateLegacyWord(entry, true)}
              >Keep as hint</button
            >
          </div>
        </div>
      {/each}
    {/if}
  </section>

  <div class="review-footer">
    <div class="list-footer">
      <span data-text-surface="flat"
        >From recent dictations · {ai.data?.backend === "openrouter"
          ? "sent to OpenRouter for review"
          : "reviewed on this computer"}</span
      >
      <div class="review-footer-actions">
        {#if !reviewReady}
          <button
            type="button"
            class="text-action"
            onclick={() => onmanage?.("notes")}>Choose a review model</button
          >
        {:else}
          <button
            type="button"
            class="text-action"
            disabled={!loaded || reviewing}
            onclick={reviewRecent}
            >{reviewing ? "Reviewing…" : "Review recent dictations"}</button
          >
        {/if}
        {#if reviewing}
          <button
            type="button"
            class="text-action"
            disabled={cancelling}
            onclick={cancelReview}>Cancel</button
          >
        {/if}
      </div>
    </div>
    {#if reviewNotice}
      <p role="status" class="field-help" data-text-surface="flat">
        {reviewNotice}
      </p>
    {/if}
    <div class="review-more">
      <Disclosure title="More learning options">
        <p class="field-help" data-text-surface="flat">
          Learned entries stay on this computer and are removed when their
          source dictations or meetings are deleted.{#if suppressedCount > 0}
            {` ${suppressedCount} ignored item${suppressedCount === 1 ? "" : "s"} won’t be suggested again.`}{/if}
        </p>
        <button
          class="button small"
          disabled={!app.native || !loaded || busy}
          onclick={resetLearning}>Reset learning…</button
        >
      </Disclosure>
    </div>
  </div>
{/if}

{#if app.snapshot.capabilities.sync}<SyncWords
    onpull={() => {
      if (editorDirty) pendingSyncPull = true;
      else void load();
    }}
  />{/if}
{#if tab !== "words" && editorDirty}<button
    class="button small spaced"
    onclick={() => (tab = "words")}>Review unsaved word</button
  >{/if}

<style>
  .undo-strip {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    margin: 0 0 16px;
  }
  .undo-strip > span {
    flex: 1 1 auto;
    font-size: 15px;
    color: var(--soft-ink);
  }
  .word-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 0 18px;
  }
  .word-search {
    flex: 1 1 220px;
    min-width: 0;
  }
  .word-add-form {
    flex: 1.25 1 260px;
    min-width: 0;
  }
  .add-well {
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
  }
  .add-well:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .add-icon {
    display: grid;
    place-items: center;
    color: var(--soft-ink);
    flex: none;
  }
  .add-well input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
    box-shadow: none;
    color: var(--ink);
    font-size: 16px;
  }
  .add-well input:focus-visible {
    outline: none;
  }
  .hints-note {
    margin: -8px 0 14px 4px;
  }
  .words-empty {
    color: var(--soft-ink);
    padding: 24px 18px;
    margin: 0;
  }
  .word-row {
    display: grid;
    grid-template-columns: 190px 18px minmax(0, 1fr) auto 16px;
    align-items: center;
    width: 100%;
    min-height: 54px;
    gap: 12px;
    border: 0;
    padding: 13px 18px;
    background: transparent;
    color: var(--ink);
    text-align: left;
    font-size: 16px;
    transition: background-color var(--motion-control) linear;
  }
  .word-row:hover:not(.open) {
    background-color: var(--well);
  }
  .word-row:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .word-heard {
    min-width: 0;
    color: var(--soft-ink);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .word-arrow {
    color: var(--soft-ink);
    text-align: center;
  }
  .word-written {
    min-width: 0;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .word-tags {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    justify-self: end;
  }
  .row-chevron {
    display: grid;
    place-items: center;
    color: var(--soft-ink);
    flex: none;
    transition: transform var(--motion-control) var(--ease-settle);
  }
  .row-chevron.open {
    transform: rotate(90deg);
  }
  .provenance-line {
    margin: 0 0 10px;
  }
  .source-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0 0 12px;
  }
  .extra-aliases {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: 0 0 12px;
  }
  .alias-chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background-color: var(--well);
    font-size: 14px;
  }
  .editor-pair,
  .editor-line {
    display: flex;
    gap: 12px;
    align-items: center;
    margin-bottom: 14px;
  }
  .editor-pair label {
    flex: 1 1 120px;
    min-width: 0;
  }
  .editor-pair input {
    width: 100%;
    margin-top: 0;
  }
  .editor-line > span {
    width: 110px;
    flex: none;
    font-size: 15px;
    color: var(--soft-ink);
  }
  .editor-line input {
    flex: 1 1 120px;
    min-width: 0;
    margin-top: 0;
  }
  .editor-actions {
    display: flex;
    gap: 9px;
    justify-content: flex-end;
    margin-top: 12px;
  }
  .learning-settings {
    margin-bottom: 18px;
  }
  .learning-head {
    padding: 0 18px;
  }
  .review-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 15px 18px;
  }
  .review-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .review-pair {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .review-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .cluster-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }
  .review-footer {
    margin-top: 14px;
  }
  .review-footer-actions {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .review-more {
    margin-top: 8px;
  }
  @media (max-width: 760px) {
    .word-toolbar,
    .editor-pair,
    .editor-line,
    .review-row {
      flex-wrap: wrap;
    }
    .word-row {
      grid-template-columns: 124px 16px minmax(0, 1fr) auto 16px;
      gap: 8px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .word-row,
    .row-chevron {
      transition: none;
    }
  }
</style>
