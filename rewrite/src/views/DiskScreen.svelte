<script lang="ts">
  import { onMount } from "svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import { speechChoice } from "../lib/speech-choice.svelte";
  import { PUNCTUATION_MODEL, activePunctuation } from "../lib/punctuation";
  import SettingRow from "../components/SettingRow.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { modelStorage } from "../lib/model-storage.svelte";
  import {
    installed,
    installing,
    checking,
    formatBytes,
    modelUnavailableReason,
  } from "../lib/model-library";
  import {
    call,
    chooseModelsFolder,
    chooseConverter,
    subscribeMediaSupport,
  } from "../lib/api";
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import type { SetupStatus } from "../lib/setup";
  import type { RouteTarget } from "../lib/route.svelte";
  let { onnavigate }: { onnavigate: (target: RouteTarget) => void } = $props();
  let tab = $state("installed");
  let selected = $state("");
  let query = $state("");
  let setup = $state<SetupStatus | null>(null);
  let pending = $state(false);
  const models = $derived(library.data?.models ?? []);
  const frontier = $derived(
    new Set([
      ...(profiles.hardware?.frontier.dictation ?? []),
      ...(profiles.hardware?.frontier.meetings ?? []),
      profiles.hardware?.frontier.otherLanguages,
      profiles.hardware?.recommendations.balanced.cleanupModel,
      profiles.hardware?.recommendations.balanced.notesModel,
    ]),
  );
  const rows = $derived(
    models.filter(
      (model) =>
        (tab === "installed"
          ? installed(model) || checking(model)
          : !installed(model) &&
            (model.deprecated ||
              !!modelUnavailableReason(model) ||
              (model.kind === "asr" && !frontier.has(model.id)))) &&
        model.name.toLowerCase().includes(query.toLowerCase()),
    ),
  );
  const used = $derived(
    models
      .filter(installed)
      .reduce((total, model) => total + model.diskBytes, 0),
  );
  const punctuation = $derived(
    models.find((model) => model.id === PUNCTUATION_MODEL),
  );
  const punctuationReady = $derived(activePunctuation(models));
  const punctuationDownload = $derived(
    punctuation &&
      (installing(punctuation) || speechChoice.failure?.id === punctuation.id)
      ? punctuation
      : null,
  );
  const locked = $derived(modelStorage.locked || pending);
  async function refresh() {
    const result = await app.perform("storage-setup", () => call("get_setup"));
    if (result) setup = result;
  }
  onMount(() => {
    const release = library.acquire();
    void profiles.check();
    void ai.refresh();
    void refresh();
    let disposed = false;
    let stop: (() => void) | undefined;
    void subscribeMediaSupport(() => void refresh()).then((value) => {
      if (disposed) value();
      else stop = value;
    });
    return () => {
      disposed = true;
      stop?.();
      release();
    };
  });
  async function locate(converter = false) {
    if (locked) return;
    pending = true;
    try {
      await app.perform("models-location", async () => {
        const path = converter
          ? await chooseConverter()
          : await chooseModelsFolder();
        if (!path) return;
        setup = converter
          ? await call("locate_converter", { path })
          : await call("locate_models", { path });
        app.apply(await call("get_snapshot"));
        await library.refresh();
        await profiles.check();
      });
    } finally {
      pending = false;
    }
  }
  async function installMedia() {
    if (locked || !(await ensureDownloadConsent(["ffmpeg"])) || locked) return;
    pending = true;
    try {
      await app.perform("install-media", async () => {
        setup = await call("install_media_support");
      });
    } finally {
      pending = false;
    }
  }
</script>

<div class="disk-toolbar">
  <div class="segments" aria-label="On disk views">
    <button
      aria-pressed={tab === "installed"}
      onclick={() => (tab = "installed")}>Installed</button
    ><button
      aria-pressed={tab === "not-offered"}
      onclick={() => (tab = "not-offered")}>Not offered</button
    >
  </div>
  <input
    type="search"
    placeholder="Search models"
    aria-label="Search models on disk"
    bind:value={query}
  />
</div>
<section class="stock paper-card quiet-settings disk-list">
  {#if library.error}<p role="status">{library.error}</p>
    <button class="button" onclick={() => library.refresh()}>Try again</button
    >{/if}
  {#if !rows.length}<p class="field-help">
      {library.data ? "No matching models." : "Reading models…"}
    </p>{/if}
  {#each rows as model (model.id)}
    {@const plan = modelStorage.removal(model)}
    <div
      class="disk-entry"
      class:selected={selected === model.id}
      data-model-id={model.id}
    >
      <button
        class="disk-row"
        aria-expanded={selected === model.id}
        onclick={() => (selected = selected === model.id ? "" : model.id)}
        ><strong data-text-surface="emboss">{model.name}</strong><span
          data-text-surface="flat"
          >{tab === "installed"
            ? model.id === "local-rules"
              ? "Built in"
              : checking(model)
                ? "Checking…"
                : formatBytes(model.diskBytes)
            : modelUnavailableReason(model) ||
              (model.deprecated
                ? "Replaced by a newer package"
                : "Outside the recommended speech frontier")}</span
        ></button
      >
      {#if selected === model.id}<div class="disk-detail">
          <p class="field-help">{model.guidance}</p>
          {#if tab === "installed"}
            {#if model.removable}<div class="actions">
                <button
                  class="button"
                  disabled={locked || !!plan.blocked}
                  onclick={() => modelStorage.remove(model)}>Remove</button
                >{#if plan.blocked}<span class="field-help" role="status"
                    >{plan.blocked}</span
                  >{/if}
              </div>{:else}<p class="field-help">
                {model.id === "local-rules"
                  ? "Built into SonoJot."
                  : "This package cannot be removed here."}
              </p>{/if}
            {#if plan.needs}<button
                class="button"
                onclick={() =>
                  onnavigate(
                    plan.needs === "asr"
                      ? { tab: "dictation", stage: "speech", from: "settings" }
                      : { tab: "meetings", stage: "notes", from: "settings" },
                  )}
                >Choose another {plan.needs === "asr" ? "speech" : "notes"} model</button
              >{/if}
          {:else if !modelUnavailableReason(model)}<button
              class="button"
              onclick={() =>
                onnavigate({
                  tab: "dictation",
                  stage: "speech",
                  from: "settings",
                })}>Open speech models</button
            >{/if}
        </div>{/if}
    </div>
  {/each}
</section>
<section class="stock paper-card quiet-settings">
  <SettingRow label="Folder"
    ><div class="folder">
      <code>{setup?.modelsPath ?? "Reading folder…"}</code><small
        data-text-surface="flat">{formatBytes(used)} in listed packages</small
      >
    </div>
    <button class="button" disabled={locked} onclick={() => locate()}
      >Change</button
    ><button
      class="button"
      disabled={!app.native}
      onclick={() =>
        app.perform("folder", () => call("open_path", { kind: "models" }))}
      >Open</button
    ></SettingRow
  >
  {#if punctuation && !punctuationReady && !modelUnavailableReason(punctuation)}
    <SettingRow
      label="Punctuation"
      help="Optional local punctuation and vocabulary support"
    >
      <button
        class="button"
        disabled={locked || speechChoice.locked}
        onclick={() => punctuation && speechChoice.use(punctuation, "helper")}
        >Install punctuation model</button
      >
    </SettingRow>
  {/if}
  {#if punctuationDownload}<ModelDownload
      model={punctuationDownload}
      pending={false}
      retryDisabled={locked}
      onretry={(model) => speechChoice.use(model, "helper")}
      oncancel={(model) => speechChoice.cancel(model.id)}
    />{/if}
  {#if speechChoice.failure?.id === PUNCTUATION_MODEL}<p role="alert">
      {speechChoice.failure.message}
    </p>{/if}
  <details>
    <summary>Audio and video import</summary>
    <div class="actions spaced">
      <span class="field-help"
        >{setup?.mediaImport.detail ||
          setup?.mediaImport.state ||
          "Checking…"}</span
      >{#if setup && ["missing", "failed"].includes(setup.mediaImport.state)}<button
          class="button"
          disabled={locked}
          onclick={installMedia}>Install media support</button
        >{/if}<button
        class="button"
        disabled={locked}
        onclick={() => locate(true)}>Locate converter</button
      >
    </div>
  </details>
</section>
{#if locked}<p class="field-help">
    Model and folder changes wait for capture, downloads and model processing to
    finish.
  </p>{/if}

<style>
  .disk-toolbar {
    display: flex;
    justify-content: space-between;
    gap: 18px;
    margin-bottom: 18px;
  }
  .disk-toolbar input {
    width: 240px;
  }
  .disk-entry + .disk-entry {
    border-top: 1px solid var(--line);
  }
  .disk-row {
    background: transparent;
    border: 0;
    display: flex;
    align-items: baseline;
    gap: 20px;
    width: 100%;
    text-align: left;
    padding: 16px 8px;
  }
  .disk-row strong {
    flex: 1;
    font-weight: 500;
  }
  .disk-row span {
    font-size: 14px;
    color: var(--soft-ink);
    max-width: 45%;
  }
  .selected {
    box-shadow: var(--raise-sm);
    border-radius: 10px;
    margin: 4px -8px;
    padding: 0 8px;
  }
  .disk-detail {
    padding: 0 8px 16px;
  }
  .disk-detail .field-help {
    margin: 0 0 8px;
  }
  .folder {
    display: grid;
    gap: 5px;
    flex: 1;
    overflow-wrap: anywhere;
  }
  .folder code {
    font-size: 13px;
  }
  summary {
    cursor: pointer;
  }
  .disk-list {
    padding-block: 4px;
  }
</style>
