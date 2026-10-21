<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from "svelte";
  import SettingRow from "./SettingRow.svelte";
  import Toggle from "./Toggle.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { call } from "../lib/api";
  import { priceLine, type AiModel } from "../lib/ai";
  import { scrollEdges } from "../lib/scroll-edges";
  import {
    PROOFREADER_DEFAULT_MODEL,
    PROOFREADER_HELP,
    PROOFREADER_LABEL,
    CLOUD_NOTICE_KEY,
    CLOUD_NOTICE_VERSION,
    needsCloudNotice,
    needsTrainingAcceptance,
    proofreaderModels,
    trainingAcceptance,
    trainingNotice,
  } from "../lib/proofreader";

  let { active = false }: { active?: boolean } = $props();
  let catalog = $state<AiModel[]>([]);
  let loading = $state(false);
  let failed = $state(false);
  let choosing = $state(false);
  let search = $state("");
  let shown = $state(25);
  let confirming = $state(false);
  let modelTrigger: HTMLButtonElement | undefined;
  let searchField: HTMLInputElement | undefined = $state();
  let sequence = 0;
  let disposed = false;
  const settings = $derived(app.snapshot.settings);
  const locked = $derived(
    !app.native ||
      active ||
      app.snapshot.meetingRecording ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      ai.busy,
  );
  const selected = $derived(
    proofreaderModels(catalog, "").find(
      (item) => item.id === settings.proofreaderModel,
    ),
  );
  const modelName = $derived(
    selected?.name ??
      (settings.proofreaderModel === PROOFREADER_DEFAULT_MODEL
        ? "GPT-6 Luna"
        : settings.proofreaderModel),
  );
  const needsTraining = $derived(
    !!selected && needsTrainingAcceptance(selected, settings),
  );
  const needsCloud = $derived(needsCloudNotice(settings));
  const matches = $derived(proofreaderModels(catalog, search));
  const help = $derived(
    !ai.data
      ? ai.busy
        ? "Checking OpenRouter connection…"
        : "Could not check OpenRouter. Try again."
      : !ai.data.supported
        ? ai.data.help
        : app.busy("ai-connect")
          ? "Finish connecting in your browser."
          : !ai.data.connected
            ? "Connect OpenRouter to proofread dictations."
            : needsCloud
              ? "Review the OpenRouter notice before enabling proofreading."
              : loading
                ? "Loading proofreading models…"
                : failed
                  ? "Could not load models. Your saved choice is unchanged."
                  : !selected
                    ? "Choose an available proofreading model."
                    : needsTraining
                      ? "Review this model's training notice before using it."
                      : PROOFREADER_HELP,
  );

  async function loadCatalog(): Promise<void> {
    if (!ai.data?.connected || loading) return;
    const current = ++sequence;
    loading = true;
    failed = false;
    try {
      const models = await call("list_ai_catalog");
      if (current === sequence) catalog = models;
    } catch {
      if (current === sequence) failed = true;
    } finally {
      if (current === sequence) loading = false;
    }
  }

  async function setChoosing(open: boolean): Promise<void> {
    choosing = open;
    await tick();
    if (!disposed) (open ? searchField : modelTrigger)?.focus();
  }

  async function acceptTraining(model: AiModel): Promise<boolean> {
    if (!needsTrainingAcceptance(model, app.snapshot.settings)) return true;
    return (
      (await app.chooseConfirmation({
        title: "Allow this model to train on your text?",
        detail: trainingNotice(model),
        label: "Allow for this model",
        cancelLabel: "Cancel",
      })) === true
    );
  }

  async function selectModel(model: AiModel): Promise<void> {
    if (locked || confirming || loading || !ai.data?.connected) return;
    confirming = true;
    try {
      if (
        !(await acceptTraining(model)) ||
        disposed ||
        locked ||
        !ai.data?.connected
      )
        return;
      await app.saveSettings({
        proofreaderModel: model.id,
        ...(model.trainingRequired
          ? {
              proofreaderTrainingModels: trainingAcceptance(
                model,
                app.snapshot.settings,
              ),
            }
          : {}),
      });
      if (app.snapshot.settings.proofreaderModel === model.id)
        await setChoosing(false);
    } finally {
      confirming = false;
    }
  }

  async function toggle(enabled: boolean): Promise<void> {
    if (locked || confirming) return;
    if (!enabled) return app.saveSettings({ proofreader: false });
    if (!selected || !ai.data?.connected || loading || failed || needsCloud)
      return;
    const model = selected;
    confirming = true;
    try {
      if (
        !(await acceptTraining(model)) ||
        disposed ||
        locked ||
        !ai.data?.connected ||
        app.snapshot.settings.proofreaderModel !== model.id
      )
        return;
      await app.saveSettings({
        proofreader: true,
        ...(model.trainingRequired
          ? {
              proofreaderTrainingModels: trainingAcceptance(
                model,
                app.snapshot.settings,
              ),
            }
          : {}),
      });
    } finally {
      confirming = false;
    }
  }

  async function reviewCloudNotice(): Promise<void> {
    if (locked || confirming || !ai.data?.connected) return;
    confirming = true;
    try {
      const accepted = await app.chooseConfirmation({
        title: "Use online proofreading?",
        detail: ai.data.help,
        label: "Accept online notice",
        cancelLabel: "Cancel",
      });
      if (accepted !== true || disposed || locked || !ai.data?.connected)
        return;
      await app.saveSettings({
        componentAcknowledgments: {
          ...app.snapshot.settings.componentAcknowledgments,
          [CLOUD_NOTICE_KEY]: CLOUD_NOTICE_VERSION,
        },
      });
    } finally {
      confirming = false;
    }
  }

  $effect(() => {
    const connected = ai.data?.connected;
    untrack(() => {
      if (connected) void loadCatalog();
      else {
        sequence++;
        loading = false;
        failed = false;
        catalog = [];
        choosing = false;
      }
    });
  });
  onMount(() => {
    void ai.refresh();
  });
  onDestroy(() => {
    disposed = true;
    sequence++;
  });
</script>

<div class="proofreader-settings">
  <SettingRow label={PROOFREADER_LABEL} {help}>
    {#if !ai.data}
      <button
        class="button quiet small"
        disabled={locked}
        onclick={() => ai.refresh()}>Try again</button
      >
    {:else if app.busy("ai-connect")}
      <button
        class="button quiet small"
        disabled={app.busy("ai-cancel")}
        onclick={() =>
          app.perform("ai-cancel", () => call("cancel_ai_connect"))}
        >Cancel</button
      >
    {:else if !ai.data.connected}
      <button
        class="button quiet small"
        disabled={locked || !ai.data.supported}
        onclick={() => ai.connect()}>Connect OpenRouter</button
      >
    {:else if needsCloud}
      <button
        class="button quiet small"
        disabled={locked || confirming}
        onclick={() => reviewCloudNotice()}>Review online notice</button
      >
    {:else if failed}
      <button
        class="button quiet small"
        disabled={locked || loading}
        onclick={() => loadCatalog()}>Try again</button
      >
    {:else if needsTraining}
      <button
        class="button quiet small"
        disabled={locked || confirming}
        onclick={() => selected && selectModel(selected)}
        >Review model notice</button
      >
    {:else if !selected && !loading}
      <button
        class="button quiet small"
        disabled={locked}
        onclick={() => setChoosing(true)}>Choose model</button
      >
    {:else}
      <Toggle
        label={PROOFREADER_LABEL}
        checked={settings.proofreader}
        disabled={locked || confirming || loading || !selected}
        onchange={(enabled) => void toggle(enabled)}
      />
    {/if}
  </SettingRow>
  <SettingRow
    label="Proofreading model"
    help={selected?.trainingRequired
      ? needsTraining
        ? "Provider may train on your text. Acceptance is required."
        : "Training allowed for this model only. Account privacy settings still apply."
      : "Used only for proofreading dictations."}
  >
    <button
      bind:this={modelTrigger}
      class="button quiet small model-choice"
      aria-label={`Proofreading model: ${modelName}`}
      aria-expanded={choosing}
      aria-controls="proofreader-catalog"
      disabled={locked || confirming || !ai.data?.connected || loading}
      onclick={() => {
        search = "";
        shown = 25;
        void setChoosing(!choosing);
      }}
    >
      {modelName}
    </button>
  </SettingRow>
  {#if choosing}
    <section
      id="proofreader-catalog"
      aria-label="Proofreading models"
      class="model-catalog"
    >
      <label class="field-label" for="proofreader-search"
        >Search OpenRouter models</label
      >
      <input
        bind:this={searchField}
        id="proofreader-search"
        type="search"
        bind:value={search}
        oninput={() => (shown = 25)}
        onkeydown={(event) => {
          if (event.key === "Escape") void setChoosing(false);
        }}
        disabled={locked || confirming}
        placeholder="Model name or provider"
      />
      {#if failed}
        <p class="field-help" data-text-surface="flat" role="status">
          The catalog could not be refreshed. Try again above.
        </p>
      {:else if !matches.length}
        <p class="field-help" data-text-surface="flat" role="status">
          No text model matches. Try another name.
        </p>
      {/if}
      <ul
        class="catalog-models"
        aria-label="Proofreading model results"
        use:scrollEdges
      >
        {#each matches.slice(0, shown) as model (model.id)}
          <li>
            <div class="model-copy">
              <span>{model.name}</span>
              <small data-text-surface="flat">{priceLine(model)}</small>
              {#if model.trainingRequired}<small data-text-surface="flat"
                  >Provider may train on your text</small
                >{/if}
              {#if model.id === PROOFREADER_DEFAULT_MODEL}<small
                  data-text-surface="flat">Default</small
                >{/if}
            </div>
            <button
              class="button quiet small"
              aria-label={`Use ${model.name} for proofreading`}
              aria-pressed={model.id === settings.proofreaderModel}
              disabled={locked || confirming || loading}
              onclick={() => selectModel(model)}
              >{model.id === settings.proofreaderModel
                ? "Selected"
                : "Use"}</button
            >
          </li>
        {/each}
      </ul>
      <div class="actions">
        {#if matches.length > shown}<button
            class="button quiet small"
            onclick={() => (shown += 25)}
            >Show more ({matches.length - shown})</button
          >{/if}
        <button class="button quiet small" onclick={() => setChoosing(false)}
          >Close model list</button
        >
      </div>
    </section>
  {/if}
</div>

<style>
  /* The wrapper ends its own rows, so it carries the divider the next card row expects. */
  .proofreader-settings {
    padding-bottom: var(--space-2);
    border-bottom: 1px solid var(--line);
    box-shadow: 0 1px #ffffff1c;
  }
  .model-choice {
    max-width: 100%;
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .model-catalog {
    padding: 9px 0 18px;
  }
  input {
    width: 100%;
  }
  .catalog-models {
    list-style: none;
    padding: 0;
    margin: 9px 0;
    max-height: 360px;
    overflow-y: auto;
    scrollbar-gutter: stable;
  }
  li {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 9px;
  }
  li + li {
    border-top: 1px solid var(--line);
  }
  .model-copy {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  small {
    display: block;
    color: var(--soft-ink);
    font-size: 0.85rem;
  }
</style>
