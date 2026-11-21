<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import DictationRow from "./DictationRow.svelte";
  import OpenDictation from "./OpenDictation.svelte";
  import SearchBar from "./SearchBar.svelte";
  import { app } from "../../lib/app.svelte";
  import { call } from "../../lib/api";
  import { dayPages } from "../../lib/dictation-history";
  import type { DictationRecord } from "../../lib/protocol";
  let { onselection }: { onselection?: (open: boolean) => void } = $props();
  let records = $state<DictationRecord[]>([]);
  let loaded = $state(!app.native);
  let query = $state("");
  let selected = $state<string | null>(null);
  let startFix = $state(false);
  let lastRevision = app.dictationRevision;
  let list: HTMLDivElement;
  const groups = $derived(dayPages(records, query));
  $effect(() => onselection?.(selected !== null));
  async function refresh() {
    const rows = await app.perform("history-load", () =>
      call("list_dictations", { limit: 100 }),
    );
    if (!rows) return;
    const reviewing = records.find((row) => row.id === selected);
    records =
      reviewing && !rows.some((row) => row.id === selected)
        ? [reviewing, ...rows]
        : rows;
    loaded = true;
  }
  onMount(() => {
    if (app.native) void refresh();
  });
  $effect(() => {
    const revision = app.dictationRevision;
    if (app.native && loaded && revision !== lastRevision)
      untrack(() => {
        lastRevision = revision;
        void refresh();
      });
  });
  async function open(id: string, fix = false) {
    if (
      app.dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This dictation has changes that have not been saved.",
      ))
    )
      return;
    selected = id;
    startFix = fix;
    await tick();
    if (!fix)
      list
        .querySelector<HTMLButtonElement>('[aria-label="Close dictation"]')
        ?.focus({ preventScroll: true });
  }
  async function close() {
    const id = selected;
    selected = null;
    await tick();
    [...list.querySelectorAll<HTMLButtonElement>("[data-dictation-id]")]
      .find((button) => button.dataset.dictationId === id)
      ?.focus({ preventScroll: true });
  }
  function changed(record: DictationRecord) {
    records = records.map((row) => (row.id === record.id ? record : row));
  }
  function keyboard(event: KeyboardEvent) {
    if (
      !(event.target instanceof HTMLElement) ||
      !list?.contains(event.target) ||
      !event.target.matches("[data-dictation-id]")
    )
      return;
    const rows = [
      ...list.querySelectorAll<HTMLButtonElement>("[data-dictation-id]"),
    ];
    const index = rows.indexOf(event.target as HTMLButtonElement);
    const next =
      event.key === "ArrowDown"
        ? index + 1
        : event.key === "ArrowUp"
          ? index - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? rows.length - 1
              : null;
    if (next === null) return;
    event.preventDefault();
    rows[Math.max(0, Math.min(rows.length - 1, next))]?.focus();
  }
</script>

<svelte:window onkeydown={keyboard} />

<div class="list-toolbar">
  <SearchBar
    label="Search dictations"
    placeholder="Search dictations"
    bind:value={query}
    disabled={app.dirty}
  />
</div>
{#if app.snapshot.settings.doNotSaveDictations}<p
    class="field-help"
    data-text-surface="flat"
  >
    History is off for new dictations. Existing text stays until you delete it.
  </p>{/if}
<div bind:this={list} role="region" aria-label="Recent dictations">
  {#each groups as group (group.key)}
    <section class="day-group" aria-label={group.label}>
      <h2 class="day-heading" data-text-surface="flat">
        {group.label}{#if group.date}<span>{group.date}</span>{/if}
      </h2>
      <div class="stock paper-card ui-card day-card notebook">
        {#each group.records as record (record.id)}
          <div class="day-rule">
            {#if selected === record.id}<OpenDictation
                {record}
                {startFix}
                onchanged={changed}
                onclose={() => void close()}
                ondeleted={() => {
                  records = records.filter((row) => row.id !== record.id);
                  selected = null;
                }}
              />
            {:else}<DictationRow
                {record}
                onopen={(fix) => void open(record.id, fix)}
              />{/if}
          </div>
        {/each}
      </div>
    </section>
  {:else}
    <div class="history-empty" data-text-surface="flat">
      {#if !loaded}<p>
          {app.busy("history-load")
            ? "Loading dictations…"
            : "Dictations could not be loaded."}
        </p>{:else if query}<p>Nothing matches “{query}”.</p>{:else}<p>
          Your completed dictations will appear here.
        </p>
        <p>
          {app.snapshot.settings.hotkeyMode === "hold" ? "Hold" : "Press"}
          <kbd>{app.snapshot.settings.hotkey.replaceAll("+", " + ")}</kbd> anywhere
          and speak.
        </p>{/if}
    </div>
  {/each}
</div>
<div class="list-footer" data-text-surface="flat">
  <span>Last 100 dictations</span>
  <button
    class="text-action"
    disabled={!app.native || app.busy("history-load")}
    onclick={() => void refresh()}>Refresh</button
  >
</div>

<style>
  .history-empty {
    padding: 48px 18px;
    text-align: center;
    color: var(--soft-ink);
  }
</style>
