<script lang="ts" module>
  // Remember On disk tab, search and open row when stepping into Speech/Notes and Back.
  const remembered = {
    tab: "installed" as "installed" | "not-offered",
    selected: "",
    query: "",
  };
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icon.svelte";
  import PageHeader from "../components/quiet/PageHeader.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
  import SearchBar from "../components/quiet/SearchBar.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import ModelUpdateNotice from "../components/ModelUpdateNotice.svelte";
  import { modelUpdates } from "../lib/model-updates.svelte";
  import {
    updateFor,
    updateLabel,
    updateRunning,
    updateStatus,
  } from "../lib/model-updates";
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
  let {
    onnavigate,
    onback,
    parentTitle = "Settings",
  }: {
    onnavigate: (target: RouteTarget) => void;
    onback: () => void;
    parentTitle?: string;
  } = $props();
  let tab = $state<"installed" | "not-offered">(remembered.tab);
  let selected = $state(remembered.selected);
  let query = $state(remembered.query);
  $effect(() => {
    remembered.tab = tab;
    remembered.selected = selected;
    remembered.query = query;
  });
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
  const viewOptions = [
    { value: "installed" as const, label: "Installed" },
    { value: "not-offered" as const, label: "Not offered" },
  ];
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

<PageHeader title="On disk" back={{ label: parentTitle, onclick: onback }}>
  {#snippet actions()}
    <SegmentedControl
      label="On disk views"
      options={viewOptions}
      value={tab}
      onchange={(next) => (tab = next)}
    />
  {/snippet}
</PageHeader>

<div class="list-toolbar">
  <SearchBar
    label="Search models on disk"
    placeholder="Search models"
    bind:value={query}
  />
</div>
<ModelUpdateNotice place="page" />
<section
  class="stock paper-card ui-card flush quiet-settings disk-list"
  aria-label="Models on disk"
>
  {#if library.error}<div class="disk-empty">
      <p role="status">{library.error}</p>
      <button class="button small" onclick={() => library.refresh()}
        >Try again</button
      >
    </div>{/if}
  {#if !rows.length}<p class="field-help disk-empty">
      {library.data ? "No matching models." : "Reading models…"}
    </p>{/if}
  {#each rows as model (model.id)}
    {@const plan = modelStorage.removal(model)}
    {@const update =
      tab === "installed" ? updateFor(modelUpdates.list, model.id) : undefined}
    <div
      class="day-rule disk-entry"
      class:selected={selected === model.id}
      data-model-id={model.id}
    >
      <button
        class="disk-row"
        aria-expanded={selected === model.id}
        onclick={() => (selected = selected === model.id ? "" : model.id)}
        ><strong data-text-surface="emboss">{model.name}</strong><span
          class="disk-size"
          data-text-surface="flat"
          >{tab === "installed"
            ? model.id === "local-rules"
              ? "Built in"
              : checking(model)
                ? "Checking…"
                : update && !update.automatic
                  ? `${formatBytes(model.diskBytes)} · Update available`
                  : formatBytes(model.diskBytes)
            : modelUnavailableReason(model) ||
              (model.deprecated
                ? "Replaced by a newer package"
                : "Outside the recommended speech frontier")}</span
        ><span
          class="row-chevron"
          class:open={selected === model.id}
          aria-hidden="true"><Icon name="chevron-right" size={14} /></span
        ></button
      >
      {#if selected === model.id}<div class="row-pocket disk-detail">
          <p class="field-help">{model.guidance}</p>
          {#if update}<div class="actions update-row">
              <span class="field-help" role="status">{updateStatus(update)}</span
              >{#if update.state === "available" && !update.automatic}<button
                  class="button small"
                  onclick={() => modelUpdates.start(update.id)}
                  >{updateLabel(update)}</button
                >{:else if update.state === "failed"}<button
                  class="button small"
                  onclick={() => modelUpdates.start(update.id)}
                  >Try again</button
                >{:else if updateRunning(update) && ["queued", "downloading"].includes(update.state)}<button
                  class="button small"
                  onclick={() => modelUpdates.cancel(update.id)}>Cancel</button
                >{/if}
            </div>{/if}
          {#if tab === "installed"}
            <div class="actions">
              {#if model.removable}
                <button
                  class="button small"
                  disabled={locked || !!plan.blocked}
                  onclick={() => modelStorage.remove(model)}>Remove</button
                >{#if plan.blocked}<span class="field-help" role="status"
                    >{plan.blocked}</span
                  >{/if}
              {:else}<p class="field-help inline-note">
                  {model.id === "local-rules"
                    ? "Built into SonoJot."
                    : "This package cannot be removed here."}
                </p>{/if}
              {#if plan.needs}<button
                  class="button small"
                  onclick={() =>
                    onnavigate(
                      plan.needs === "asr"
                        ? {
                            tab: "dictation",
                            stage: "speech",
                            from: "settings-disk",
                          }
                        : {
                            tab: "meetings",
                            stage: "notes",
                            from: "settings-disk",
                          },
                    )}
                  >Choose another {plan.needs === "asr" ? "speech" : "notes"} model</button
                >{/if}
            </div>
          {:else if !modelUnavailableReason(model)}<button
              class="button small"
              onclick={() =>
                onnavigate({
                  tab: "dictation",
                  stage: "speech",
                  from: "settings-disk",
                })}>Open speech models</button
            >{/if}
        </div>{/if}
    </div>
  {/each}
</section>
<section class="stock paper-card ui-card row-card quiet-settings">
  <SettingRow label="Folder" layout="choice">
    <div class="folder-row">
      <div class="folder">
        <code>{setup?.modelsPath ?? "Reading folder…"}</code>
        <small data-text-surface="flat"
          >{formatBytes(used)} in listed packages</small
        >
      </div>
      <div class="actions">
        <button class="button small" disabled={locked} onclick={() => locate()}
          >Change</button
        ><button
          class="button small"
          disabled={!app.native}
          onclick={() =>
            app.perform("folder", () => call("open_path", { kind: "models" }))}
          >Open</button
        >
      </div>
    </div>
  </SettingRow>
  {#if punctuation && !punctuationReady && !modelUnavailableReason(punctuation)}
    <SettingRow
      label="Punctuation"
      help="Optional local punctuation and vocabulary support"
      layout="action"
    >
      <button
        class="button small"
        disabled={locked || speechChoice.locked}
        onclick={() => punctuation && speechChoice.use(punctuation, "helper")}
        >Install punctuation model</button
      >
    </SettingRow>
  {/if}
  {#if punctuationDownload}<div class="download-pad">
      <ModelDownload
        model={punctuationDownload}
        pending={false}
        retryDisabled={locked}
        onretry={(model) => speechChoice.use(model, "helper")}
        oncancel={(model) => speechChoice.cancel(model.id)}
      />
    </div>{/if}
  {#if speechChoice.failure?.id === PUNCTUATION_MODEL}<p role="alert">
      {speechChoice.failure.message}
    </p>{/if}
  <Disclosure title="Audio and video import">
    <div class="actions media-actions">
      <span class="field-help"
        >{setup?.mediaImport.detail ||
          setup?.mediaImport.state ||
          "Checking…"}</span
      >{#if setup && ["missing", "failed"].includes(setup.mediaImport.state)}<button
          class="button small"
          disabled={locked}
          onclick={installMedia}>Install media support</button
        >{/if}<button
        class="button small"
        disabled={locked}
        onclick={() => locate(true)}>Locate converter</button
      >
    </div>
  </Disclosure>
</section>
{#if locked}<p class="field-help">
    Model and folder changes wait for capture, downloads and model processing to
    finish.
  </p>{/if}

<style>
  .disk-empty {
    padding: 20px 18px;
    margin: 0;
  }
  .disk-row {
    background: transparent;
    border: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    min-height: 54px;
    text-align: left;
    padding: 13px 18px;
    color: var(--ink);
    font-size: 16px;
    transition: background-color var(--motion-control) linear;
  }
  .disk-row:hover:not([aria-expanded="true"]) {
    background-color: var(--well);
  }
  .disk-row:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .disk-row strong {
    flex: 1 1 auto;
    min-width: 0;
    font-weight: 550;
  }
  .disk-size {
    font-size: 13px;
    color: var(--soft-ink);
    font-variant-numeric: tabular-nums;
    max-width: 45%;
    text-align: right;
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
  .disk-detail > .field-help:first-child {
    margin: 0 0 12px;
  }
  .inline-note {
    margin: 0;
  }
  .folder-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 14px;
    width: 100%;
  }
  .folder {
    display: grid;
    gap: 4px;
    flex: 1 1 200px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .folder code {
    font-size: 13px;
  }
  .folder small {
    font-size: 13px;
    color: var(--soft-ink);
  }
  .download-pad {
    padding: 8px 0 14px;
  }
  .media-actions {
    padding-bottom: 12px;
  }
  .update-row {
    align-items: center;
    margin-bottom: 10px;
  }
  .update-row .field-help {
    flex: 1;
    margin: 0;
  }
  @media (prefers-reduced-motion: reduce) {
    .disk-row,
    .row-chevron {
      transition: none;
    }
  }
</style>
