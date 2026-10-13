<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import Card from "./Card.svelte";
  import SettingRow from "./SettingRow.svelte";
  import RemoteModelBadge from "./RemoteModelBadge.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { ai } from "../lib/ai.svelte";
  import {
    modelRoles,
    priceLine,
    quickModelRemoval,
    routeNotes,
    type AiModel,
    type AiModelCheck,
    type AiQuickModel,
  } from "../lib/ai";

  let recommended = $state<AiModel[] | null>(null);
  let catalog = $state<AiModel[] | null>(null);
  let catalogLoading = $state(false);
  let catalogFailed = $state(false);
  let recommendedFailed = $state(false);
  let search = $state("");
  let mode = $state<"recommended" | "all">("recommended");
  let shown = $state(25);
  let notice = $state("");
  let checks = $state<
    Record<
      string,
      { loading: boolean; check: AiModelCheck | null; failed: boolean }
    >
  >({});
  let catalogRequested = false;
  let catalogSeq = 0;
  let checkSeq = 0;
  const query = $derived(search.trim().toLocaleLowerCase());
  const browsingAll = $derived(!!query || mode === "all");
  const modelById = $derived(
    new Map(
      [...(catalog ?? []), ...(recommended ?? [])].map((item) => [
        item.id,
        item,
      ]),
    ),
  );
  const matches = $derived(
    (browsingAll
      ? (catalog ?? recommended ?? [])
      : (recommended ?? catalog ?? [])
    )
      .flatMap((item) => modelRoles(item).map((role) => ({ item, role })))
      .filter(
        ({ item, role }) =>
          !query ||
          `${item.name} ${item.id} ${role} ${roleLabel(role)} ${item.inputModalities.join(" ")}`
            .toLocaleLowerCase()
            .includes(query),
      ),
  );
  const results = $derived(matches.slice(0, shown));
  const locked = $derived(
    ai.busy || !app.native || app.busy("settings") || app.busy("setup-profile"),
  );

  function roleLabel(role: AiQuickModel["role"]): string {
    return role === "speech" ? "Meeting transcription" : "Text and notes";
  }

  function hasModel(id: string, role: AiQuickModel["role"]): boolean {
    return !!ai.data?.quickModels.some(
      (entry) => entry.id === id && entry.role === role,
    );
  }

  async function loadModels(force = false): Promise<void> {
    if (!ai.data?.connected || catalogLoading || (catalogRequested && !force))
      return;
    const seq = ++catalogSeq;
    catalogRequested = true;
    catalogLoading = true;
    catalogFailed = false;
    recommendedFailed = false;
    const [full, curated] = await Promise.allSettled([
      call("list_ai_catalog"),
      call("list_ai_models"),
    ]);
    if (seq !== catalogSeq) return;
    if (full.status === "fulfilled") catalog = full.value;
    else catalogFailed = true;
    if (curated.status === "fulfilled") recommended = curated.value;
    else recommendedFailed = true;
    catalogLoading = false;
  }

  async function checkModel(id: string, force = false): Promise<void> {
    if (!ai.data?.connected || checks[id]?.loading || (checks[id] && !force))
      return;
    const seq = checkSeq;
    checks[id] = { loading: true, check: null, failed: false };
    try {
      const check = await call("check_ai_model", { model: id });
      if (seq === checkSeq)
        checks[id] = { loading: false, check, failed: false };
    } catch {
      if (seq === checkSeq)
        checks[id] = { loading: false, check: null, failed: true };
    }
  }

  async function add(item: AiModel, role: AiQuickModel["role"]): Promise<void> {
    if (locked || !ai.data?.connected || hasModel(item.id, role)) return;
    notice = "";
    // Adding makes the model available to pickers without selecting a route.
    if (
      await ai.save({
        quickModels: [...ai.data.quickModels, { id: item.id, role }],
      })
    )
      notice = `${item.name} added for ${roleLabel(role).toLocaleLowerCase()}.`;
  }

  async function remove(entry: AiQuickModel): Promise<void> {
    const current = ai.data;
    if (locked || !current) return;
    const resetText =
      entry.role === "text" &&
      current.model === entry.id &&
      (current.backend === "openrouter" ||
        app.snapshot.settings.notesModel === "openrouter");
    const resetSpeech =
      entry.role === "speech" && current.speechModel === entry.id;
    notice = "";
    const saved = await ai.save(quickModelRemoval(current, entry));
    if (saved)
      notice = resetText
        ? "Model removed. Notes now use the local model."
        : resetSpeech
          ? "Model removed. Meeting transcription now uses the local Speech model."
          : "Model removed from your list.";
  }

  $effect(() => {
    const connected = ai.data?.connected;
    untrack(() => {
      if (connected) void loadModels();
      else {
        catalogSeq++;
        checkSeq++;
        catalogRequested = false;
        catalogLoading = false;
        catalog = null;
        recommended = null;
        checks = {};
      }
    });
  });
  onMount(() => {
    void ai.refresh();
  });
  onDestroy(() => {
    catalogSeq++;
    checkSeq++;
  });
</script>

{#snippet details(id: string, item: AiModel | undefined)}
  <details
    class="model-details"
    ontoggle={(event) => {
      if (event.currentTarget.open) void checkModel(id);
    }}
  >
    <summary>Details</summary>
    <div class="field-help" data-text-surface="flat">
      <p>{id}</p>
      {#if item?.description}<p>{item.description}</p>{/if}
      {#if item?.contextLength}<p>
          {item.contextLength.toLocaleString("en-US")} token context
        </p>{/if}
      {#if checks[id]?.loading}<p role="status">Checking model routes…</p>
      {:else}
        {#each routeNotes(checks[id]?.check ?? null, checks[id]?.failed ?? false) as note}<p
          >
            {note}
          </p>{/each}
        {#if checks[id]?.failed}<button
            class="button small"
            onclick={() => checkModel(id, true)}>Try again</button
          >{/if}
      {/if}
    </div>
  </details>
{/snippet}

<div id="openrouter-models">
  <Card title="Online models (OpenRouter)">
    <p class="field-help" data-text-surface="flat">
      Optional. Audio and text sent to OpenRouter leave this computer and are
      billed to your account.
    </p>
    {#if !ai.data}
      <p class="field-help" data-text-surface="flat" role="status">
        {ai.busy
          ? "Checking OpenRouter settings…"
          : "OpenRouter settings could not be checked."}
      </p>
      {#if !ai.busy}<button
          class="button"
          disabled={!app.native}
          onclick={() => ai.refresh()}>Try again</button
        >{/if}
    {:else if !ai.data.connected}
      <div class="actions">
        {#if app.busy("ai-connect")}
          <span role="status">Finish connecting in your browser…</span>
          <button
            class="button"
            disabled={app.busy("ai-cancel")}
            onclick={() =>
              app.perform("ai-cancel", () => call("cancel_ai_connect"))}
            >Cancel</button
          >
        {:else}
          <button
            id="ai-connect"
            class="button"
            disabled={locked || !ai.data.supported}
            onclick={() => ai.connect()}>Connect OpenRouter</button
          >
        {/if}
      </div>
      {#if !ai.data.supported}<p class="field-help" data-text-surface="flat">
          {ai.data.help}
        </p>{/if}
    {:else}
      <SettingRow label="Connected">
        <button
          class="button quiet"
          disabled={locked}
          onclick={() => ai.disconnect()}>Disconnect</button
        >
      </SettingRow>

      <h3 data-text-surface="emboss">Your OpenRouter models</h3>
      <p class="field-help" data-text-surface="flat">
        Available in the Notes and Meeting transcription pickers. Adding a model
        does not select it.
      </p>
      {#if !ai.data.quickModels.length}
        <p class="field-help" data-text-surface="flat">
          No models yet. Add one below.
        </p>
      {:else}
        <ul class="quick-models" aria-label="Your OpenRouter models">
          {#each ai.data.quickModels as entry (`${entry.role}:${entry.id}`)}
            {@const item = modelById.get(entry.id)}
            <li class="model-row">
              <div class="model-copy">
                <span>{item?.name ?? entry.id}<RemoteModelBadge /></span>
                <small data-text-surface="flat"
                  >{roleLabel(entry.role)} · {item
                    ? priceLine(item)
                    : "Price unavailable."}</small
                >
                {@render details(entry.id, item)}
              </div>
              <button
                class="button small"
                disabled={locked}
                aria-label={`Remove ${item?.name ?? entry.id} for ${roleLabel(entry.role)}`}
                onclick={() => remove(entry)}>Remove</button
              >
            </li>
          {/each}
        </ul>
      {/if}
      {#if notice}<p class="field-help" data-text-surface="flat" role="status">
          {notice}
        </p>{/if}

      <h3 data-text-surface="emboss">Add a model</h3>
      <label class="field-label" for="ai-model-search"
        >Search OpenRouter models</label
      >
      <input
        id="ai-model-search"
        type="search"
        bind:value={search}
        autocomplete="off"
        spellcheck="false"
        placeholder="Search by model, provider or role"
        oninput={() => {
          shown = 25;
        }}
      />
      <div class="catalog-toolbar">
        <div class="actions" role="group" aria-label="Catalog list">
          <button
            class="button small"
            aria-pressed={!browsingAll}
            onclick={() => {
              mode = "recommended";
              search = "";
              shown = 25;
            }}>Recommended</button
          >
          <button
            class="button small"
            aria-pressed={browsingAll}
            onclick={() => {
              mode = "all";
              shown = 25;
            }}>All models</button
          >
        </div>
        <button
          class="button small"
          disabled={catalogLoading}
          onclick={() => loadModels(true)}>Refresh</button
        >
      </div>
      {#if catalogLoading}<p
          class="field-help"
          data-text-surface="flat"
          role="status"
        >
          Loading models…
        </p>{/if}
      {#if catalogFailed || recommendedFailed}
        <div class="catalog-notice" role="status">
          <span class="field-help" data-text-surface="flat">
            {catalogFailed && recommendedFailed
              ? "Model lists could not be refreshed."
              : catalogFailed
                ? "The full model catalog could not be loaded."
                : "The recommended list could not be refreshed."}
          </span>
          <button
            class="button small"
            disabled={catalogLoading}
            onclick={() => loadModels(true)}>Try again</button
          >
        </div>
      {/if}
      {#if !browsingAll && recommended?.length}
        <p class="field-help" data-text-surface="flat">
          Recommended models, least expensive first. Use All models for the full
          text and speech catalog.
        </p>
      {/if}
      {#if results.length}
        <ul class="catalog-models" aria-label="OpenRouter catalog results">
          {#each results as { item, role } (`${role}:${item.id}`)}
            <li class="model-row">
              <div class="model-copy">
                <span>{item.name}<RemoteModelBadge /></span>
                <small data-text-surface="flat"
                  >{roleLabel(role)} · {priceLine(item)}</small
                >
                {@render details(item.id, item)}
              </div>
              {#if hasModel(item.id, role)}
                <span class="in-list" data-text-surface="flat"
                  >In your list</span
                >
              {:else}
                <button
                  class="button small"
                  disabled={locked}
                  aria-label={`Add ${item.name} for ${roleLabel(role)}`}
                  onclick={() => add(item, role)}>Add</button
                >
              {/if}
            </li>
          {/each}
        </ul>
        {#if matches.length > results.length}
          <div class="catalog-toolbar">
            <button
              class="button small"
              onclick={() => {
                shown += 25;
              }}>Show more</button
            >
            <span class="field-help" data-text-surface="flat"
              >{matches.length - results.length} more results</span
            >
          </div>
        {/if}
      {:else if !catalogLoading && (catalog !== null || recommended !== null)}
        <p class="field-help" data-text-surface="flat" role="status">
          {query ? "No catalog model matches." : "No models in this list."}
        </p>
      {/if}
    {/if}
  </Card>
</div>

<style>
  #openrouter-models {
    scroll-margin-top: 18px;
  }
  h3 {
    margin: 27px 0 9px;
    font-size: 1rem;
  }
  .quick-models,
  .catalog-models {
    list-style: none;
    margin: 9px 0;
    padding: 0;
  }
  .catalog-models {
    max-height: 360px;
    overflow-y: auto;
    scrollbar-gutter: stable;
  }
  .model-row {
    display: flex;
    align-items: flex-start;
    gap: 18px;
    padding: 12px 0;
  }
  .model-row + .model-row {
    border-top: 1px solid var(--line);
  }
  .model-row > .button,
  .in-list {
    flex: none;
  }
  .model-copy {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .model-copy small {
    display: block;
    margin-top: 3px;
    color: var(--soft-ink);
    font-size: 0.85rem;
  }
  .in-list {
    color: var(--soft-ink);
    font-size: 0.85rem;
    padding: 6px 0;
  }
  .model-details {
    margin-top: 6px;
    font-size: 0.85rem;
  }
  .model-details summary {
    cursor: pointer;
    width: fit-content;
  }
  .model-details p {
    margin: 6px 0;
  }
  .catalog-toolbar,
  .catalog-notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 9px;
    flex-wrap: wrap;
    margin: 9px 0;
  }
  .catalog-toolbar .field-help,
  .catalog-notice .field-help {
    margin: 0;
  }
  #ai-model-search {
    width: 100%;
  }
</style>
