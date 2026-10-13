<script lang="ts">
  import { onMount } from "svelte";
  import Card from "./Card.svelte";
  import SettingRow from "./SettingRow.svelte";
  import Toggle from "./Toggle.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import type { LearnedEntry } from "../lib/protocol";
  let { ondirty }: { ondirty: (dirty: boolean) => void } = $props();
  let entries = $state<LearnedEntry[]>([]);
  let saved = $state("[]");
  let loaded = $state(false);
  let expandedId = $state<string | null>(null);
  const dirty = $derived(JSON.stringify(entries) !== saved);
  const busy = $derived(
    app.busy("learning-save") || app.busy("learning-reset"),
  );
  $effect(() => {
    ondirty(dirty);
  });
  async function load(): Promise<void> {
    const result = await app.perform("learning-load", () =>
      call("get_learning"),
    );
    if (result) {
      entries = result;
      saved = JSON.stringify(result);
      loaded = true;
    }
  }
  onMount(() => {
    if (app.native) void load();
  });
  async function save(): Promise<void> {
    const result = await app.perform(
      "learning-save",
      () =>
        call("save_learning", {
          entries: entries.map((entry) => ({
            ...entry,
            sourceIds: [...entry.sourceIds],
            text: entry.text.trim(),
            heard: entry.heard?.trim() || null,
            scope: entry.scope?.trim() || null,
          })),
        }),
      "Local learning saved.",
    );
    if (result) {
      entries = result;
      saved = JSON.stringify(result);
    }
  }
  async function remove(entry: LearnedEntry): Promise<void> {
    if (
      !(await app.confirm(
        "Delete this learned entry?",
        `Remove “${entry.text}” from local learning? Save learning to apply this deletion. Your saved dictations and manual vocabulary remain.`,
        "Delete entry",
        true,
      ))
    )
      return;
    entries = entries.filter((item) => item.id !== entry.id);
    if (expandedId === entry.id) expandedId = null;
  }
  async function reset(): Promise<void> {
    if (
      !(await app.confirm(
        "Reset local learning?",
        "This removes all learned entries, including pinned entries, and discards unsaved learning edits. Manual vocabulary, saved dictations and meetings remain. Learning can resume if the opt-in stays on.",
        "Reset learning",
        true,
      ))
    )
      return;
    await app.perform(
      "learning-reset",
      async () => {
        await call("reset_learning", { confirmed: true });
        entries = [];
        saved = "[]";
        loaded = true;
        expandedId = null;
      },
      "Local learning reset.",
    );
  }
</script>

<Card title="Learning from your fixes">
  <SettingRow
    label="Learn from my corrections on this device"
    help="Remember clear word corrections you make in SonoJot. You can inspect, disable or delete learned entries."
  >
    <Toggle
      label="Learn from my corrections on this device"
      checked={app.snapshot.settings.learnFromCorrections}
      disabled={!app.native || app.busy("settings")}
      onchange={(learnFromCorrections) =>
        app.saveSettings({ learnFromCorrections })}
    />
  </SettingRow>
  <p class="field-help" data-text-surface="flat">
    {app.snapshot.settings.doNotSaveDictations
      ? "Automatic learning is paused while Save dictation history is off. Existing vocabulary can still be used."
      : "Learning uses explicit corrections saved in SonoJot. Disabling it stops new learning and keeps existing entries."}
  </p>
  {#if !loaded}<p class="field-help" data-text-surface="flat">
      {app.native
        ? "Local learning has not been loaded."
        : "Open the desktop app to inspect local learning."}
    </p>
    <button
      class="button small"
      disabled={!app.native || app.busy("learning-load")}
      onclick={load}
      >{app.busy("learning-load") ? "Loading…" : "Load learning"}</button
    >
  {:else if !entries.length}<p class="empty-copy" data-text-surface="flat">
      No learned entries.
    </p>
  {:else}<ul class="learning-list" aria-label="Learned entries">
      {#each entries as entry (entry.id)}
        <li class="learned-entry" data-learning-id={entry.id}>
          <button
            class="button quiet learned-row"
            aria-expanded={expandedId === entry.id}
            aria-controls={`learning-editor-${entry.id}`}
            onclick={() =>
              (expandedId = expandedId === entry.id ? null : entry.id)}
          >
            <span class="entry-text"
              >{#if entry.heard}<span>{entry.heard}</span><span
                  aria-hidden="true"
                >
                  →
                </span><span class="sr-only">becomes </span>{/if}<strong
                >{entry.text || "Saved text"}</strong
              ></span
            >
            {#if !entry.enabled}<span
                class="field-help row-status"
                data-text-surface="flat">Off</span
              >{/if}
            <span aria-hidden="true">{expandedId === entry.id ? "▾" : "›"}</span
            >
          </button>
          {#if expandedId === entry.id}
            <div class="learning-editor" id={`learning-editor-${entry.id}`}>
              <div class="two-fields">
                <label
                  >Saved text<input
                    aria-label={`Saved text for ${entry.id}`}
                    bind:value={entry.text}
                    disabled={busy}
                    maxlength="1000"
                  /></label
                >
                <label
                  >Heard form<input
                    aria-label={`Heard form for ${entry.id}`}
                    value={entry.heard ?? ""}
                    oninput={(event) =>
                      (entry.heard = event.currentTarget.value || null)}
                    disabled={busy}
                    maxlength="250"
                  /></label
                >
              </div>
              <details class="topic-fold">
                <summary>Project or topic (optional)</summary>
                <label>
                  <span class="sr-only"
                    >Project or topic for learned entry {entry.id}</span
                  >
                  <input
                    aria-label={`Project or topic for learned entry ${entry.id}`}
                    value={entry.scope ?? ""}
                    oninput={(event) =>
                      (entry.scope = event.currentTarget.value || null)}
                    placeholder="Optional project or topic"
                    disabled={busy}
                    maxlength="250"
                  />
                </label>
              </details>
              <div class="entry-actions">
                <span>Use this entry</span>
                <Toggle
                  label={`Use learned entry ${entry.text}`}
                  checked={entry.enabled}
                  disabled={busy}
                  onchange={(enabled) => (entry.enabled = enabled)}
                />
                <button
                  class="button quiet small remove-entry"
                  disabled={busy}
                  onclick={() => remove(entry)}>Remove from learning</button
                >
              </div>
              <details>
                <summary>Source references</summary>
                <p class="field-help" data-text-surface="flat">
                  Kind: {entry.kind}. Source: {entry.sourceType}. {entry.userPinned
                    ? "User-pinned entry."
                    : "Source-linked entry; deleting its source removes dependent learning."}
                </p>
                <p class="field-help" data-text-surface="flat">
                  {entry.sourceIds.join(", ") || "No source records"}
                </p>
              </details>
            </div>
          {/if}
        </li>
      {/each}
    </ul>{/if}
  <div class="save-bar">
    <span class="field-help" data-text-surface="flat" role="status"
      >{dirty
        ? "You have unsaved learning changes."
        : "Learned entries stay on this device."}</span
    ><button
      class="button primary"
      disabled={!app.native ||
        !dirty ||
        busy ||
        entries.some((entry) => !entry.text.trim())}
      onclick={save}
      >{app.busy("learning-save") ? "Saving…" : "Save learning"}</button
    >
  </div>
  <details class="learning-options">
    <summary>More learning options</summary>
    <p class="field-help" data-text-surface="flat">
      Meeting learning and cross-application monitoring are not enabled.
      Reviewing recent dictations uses the AI selected in Models and may send
      review text to OpenRouter.
    </p>
    <button
      class="button quiet small"
      disabled={!app.native || !loaded || busy}
      onclick={reset}>Reset learning…</button
    >
  </details>
</Card>

<style>
  .learning-list {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
  }
  .learned-entry + .learned-entry {
    border-top: 1px solid var(--line);
  }
  .learned-row {
    width: 100%;
    justify-content: flex-start;
    text-align: left;
    margin-block: var(--space-1);
  }
  .entry-text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .row-status {
    margin: 0;
  }
  .learning-editor {
    padding: var(--space-1) var(--space-2) var(--space-2);
  }
  .entry-actions {
    flex-wrap: wrap;
  }
  .remove-entry {
    margin-left: auto;
  }
  details {
    margin-block: var(--space-2);
  }
  summary {
    cursor: pointer;
  }
  summary:focus-visible {
    outline: 3px solid var(--focus);
    outline-offset: 3px;
  }
  .topic-fold > label {
    display: block;
    margin-top: var(--space-1);
  }
  .learning-options {
    margin-bottom: 0;
  }
</style>
