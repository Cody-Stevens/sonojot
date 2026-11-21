<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import Icon from "../components/Icon.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import Picker from "../components/quiet/Picker.svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import MemoryBar from "../components/quiet/MemoryBar.svelte";
  import InfoTip from "../components/quiet/InfoTip.svelte";
  import Toggle from "../components/Toggle.svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { call } from "../lib/api";
  import { library } from "../lib/model-library.svelte";
  import {
    installed,
    installing,
    selectable,
    formatBytes,
    type ModelEntry,
    type ModelsTarget,
  } from "../lib/model-library";
  import { notesStyles } from "../lib/meeting-notes.svelte";
  import type { NotesPreset } from "../lib/protocol";
  import {
    instructionLength,
    notesInstructionsError,
    NOTES_INSTRUCTIONS_LIMIT,
  } from "../lib/notes-instructions";
  import { ensureDownloadConsent } from "../lib/download-consent.svelte";
  import { openRouter } from "../lib/openrouter.svelte";
  import { textCost, type AiModel } from "../lib/ai";
  let { onmanage }: { onmanage: (target: ModelsTarget) => void } = $props();
  let instructions = $state(app.snapshot.settings.meetingNotesInstructions);
  let instructionsTouched = $state(false);
  let preset = $state<NotesPreset | null>(
    app.snapshot.settings.meetingNotesPreset,
  );
  const draft = $derived(
    instructions !== app.snapshot.settings.meetingNotesInstructions ||
      preset !== app.snapshot.settings.meetingNotesPreset,
  );
  const instructionsError = $derived(
    notesInstructionsError(preset, instructions),
  );
  const showErrorTone = $derived(
    !!instructionsError &&
      (instructionsTouched ||
        instructionLength(instructions) > NOTES_INSTRUCTIONS_LIMIT),
  );
  $effect(() => {
    app.dirty = draft;
  });
  onDestroy(() => {
    app.dirty = false;
  });
  let allLocal = $state(false);
  let localSheet = $state(false),
    pending = $state(false);
  let catalog = $state<AiModel[]>([]);
  let disposed = false;
  const models = $derived(
    (library.data?.models ?? []).filter(
      (model) =>
        model.roles.includes("notes") &&
        !["local-rules", "openrouter"].includes(model.id),
    ),
  );
  const local = $derived(
    models.find((model) => model.id === app.snapshot.settings.notesModel) ??
      models.find((model) => model.id === "qwen3-4b-instruct-2507"),
  );
  const remote = $derived(ai.data?.backend === "openrouter");
  const online = $derived(catalog.find((model) => model.id === ai.data?.model));
  const locked = $derived(
    !app.native ||
      pending ||
      ai.busy ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      app.snapshot.meetingRecording ||
      !["idle", "error"].includes(app.snapshot.state),
  );
  const downloading = $derived(models.find(installing));
  const styleOptions = $derived([
    { value: "", label: "No notes" },
    ...notesStyles.map((style) => ({ value: style.id, label: style.label })),
  ]);
  const onlineCostText = $derived.by(() => {
    const cost = textCost(online);
    return cost === "Price unavailable" ? "Price unknown" : cost;
  });
  onMount(() => {
    const release = library.acquire();
    void ai.refresh();
    void call("list_ai_models")
      .then((next) => {
        if (!disposed) catalog = next;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      release();
    };
  });
  async function choose(model: ModelEntry) {
    if (locked || !ai.data) return;
    pending = true;
    try {
      if (!installed(model)) {
        if (!model.installable || library.data?.models.some(installing)) return;
        if (!(await ensureDownloadConsent([model.id]))) return;
        if (
          disposed ||
          app.snapshot.meetingRecording ||
          !["idle", "error"].includes(app.snapshot.state)
        )
          return;
        await app.perform("model-install", async () => {
          library.downloadId = model.id;
          library.apply(await call("start_model_install", { id: model.id }));
        });
      } else if (selectable(model, "notes")) {
        const previous = app.snapshot.settings.notesModel;
        await app.saveSettings({
          notesModel: model.id,
          ...(app.snapshot.settings.setupProfile !== null
            ? { setupProfile: "custom" as const }
            : {}),
        });
        if (app.error) return;
        if (remote && !(await ai.save({ backend: "local" }))) {
          const failure = app.error;
          await app.saveSettings({ notesModel: previous });
          app.error = failure;
          return;
        }
        localSheet = false;
      }
    } finally {
      pending = false;
    }
  }
  async function saveDefaults() {
    if (locked || instructionsError) return;
    await app.saveSettings({
      meetingNotesPreset: preset,
      meetingNotesInstructions: instructions,
    });
  }
</script>

<section class="stock paper-card ui-card row-card">
  <SettingRow label="Style" layout="choice">
    {#snippet info()}
      <p>
        Notes are written after you press Stop and the transcript is ready.
        Notes you already have are kept.
      </p>
    {/snippet}
    <Picker
      label="Style"
      value={preset ?? ""}
      options={styleOptions}
      disabled={locked}
      onchange={(value) => {
        preset =
          notesStyles.find((style) => style.id === value)?.id ?? null;
        instructionsTouched = false;
        void saveDefaults();
      }}
    />
  </SettingRow>
  {#if preset === "custom"}
    <SettingRow
      label="Instructions"
      layout="choice"
      forId="notes-default-instructions"
    >
      <div class="custom-instructions">
        <textarea
          id="notes-default-instructions"
          rows="3"
          bind:value={instructions}
          oninput={() => (instructionsTouched = true)}
          onblur={() => (instructionsTouched = true)}
          disabled={locked}></textarea>
        <div class="instructions-foot">
          {#if instructionsError}<p
              class="field-help"
              class:error-text={showErrorTone}
              role={showErrorTone ? "alert" : undefined}
            >
              {instructionsError}
            </p>{:else}<span></span>{/if}
          <small class="field-help char-count"
            >{instructionLength(instructions)} / {NOTES_INSTRUCTIONS_LIMIT} characters</small
          >
        </div>
        {#if draft}
          <div class="instructions-actions">
            <button
              class="button small primary"
              disabled={locked || !!instructionsError}
              onclick={saveDefaults}>Save instructions</button
            >
          </div>
        {/if}
      </div>
    </SettingRow>
  {/if}
  <SettingRow label="Name meetings automatically" layout="action">
    {#snippet info()}
      <p>Uses a model on this computer after transcription.</p>
    {/snippet}
    <Toggle
      label="Name meetings automatically"
      checked={app.snapshot.settings.autoMeetingTitles}
      disabled={locked}
      onchange={(autoMeetingTitles) => app.saveSettings({ autoMeetingTitles })}
    />
  </SettingRow>
</section>

<div class="section-heading">
  <span class="label-tail"
    >Written by<InfoTip label="About Written by"
      ><p>
        {app.snapshot.settings.meetingNotesPreset === null
          ? "With No notes, nothing is written automatically. You can still choose Write notes in a meeting."
          : "Notes use this model after transcription. You can cancel while they are being written."}
      </p></InfoTip
    ></span
  >
</div>
<section
  class="stock paper-card ui-card option-card option-list writers"
  role="group"
  aria-label="Written by"
>
  <button
    class="quiet-option"
    class:chosen={!remote}
    aria-pressed={!remote}
    disabled={locked}
    onclick={() => (localSheet = true)}
    ><span class="choice-dot" aria-hidden="true"></span><span
      class="option-copy"
      ><span class="option-title"
        ><strong data-text-surface="emboss"
          >{local?.name ?? "Local notes model"}</strong
        ></span
      ><small
        >This computer · {local
          ? `${formatBytes(local.diskBytes)} ${installed(local) ? "on disk" : "download"}`
          : "Choose a model"}</small
      ></span
    ><MemoryBar
      bytes={local?.ramMaxBytes ?? null}
      total={library.data?.hardware.totalRamBytes ?? null}
    /><span class="option-chevron" aria-hidden="true"
      ><Icon name="chevron-right" size={15} /></span
    ></button
  >
  <button
    class="quiet-option"
    class:chosen={remote}
    aria-pressed={remote}
    disabled={locked}
    onclick={() => openRouter.open("notes")}
    ><span class="choice-dot" aria-hidden="true"></span><span
      class="option-copy"
      ><span class="option-title"
        ><strong data-text-surface="emboss"
          >{online?.name || ai.data?.model || "OpenRouter"}</strong
        >{#if ai.data?.connected}<span class="quiet-tag">Connected</span>{/if}</span
      ><small>Online · OpenRouter</small></span
    ><span class="option-meta">{onlineCostText}</span><span
      class="option-chevron"
      aria-hidden="true"><Icon name="chevron-right" size={15} /></span
    ></button
  >
</section>

{#if localSheet}<Sheet
    title="Local notes model"
    onclose={() => {
      if (!pending) localSheet = false;
    }}
  >
    {#if app.error}<p role="alert">{app.error}</p>{/if}
    {#each models.filter((model) => allLocal || model.id === local?.id || installed(model)) as model (model.id)}<button
        class="quiet-option"
        aria-pressed={!remote && model.id === app.snapshot.settings.notesModel}
        disabled={locked || !model.supported || installing(model)}
        onclick={() => choose(model)}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          ><strong data-text-surface="emboss">{model.name}</strong><small
            >{installed(model)
              ? "Installed"
              : `${formatBytes(model.diskBytes)} download`}</small
          ></span
        ><MemoryBar
          bytes={model.ramMaxBytes}
          total={library.data?.hardware.totalRamBytes ?? null}
        /></button
      >{/each}
    {#if downloading}<ModelDownload
        model={downloading}
        {pending}
        retryDisabled={locked}
        onretry={choose}
        oncancel={async (model) => {
          await app.perform("model-cancel", async () =>
            library.apply(await call("cancel_model_install", { id: model.id })),
          );
        }}
      />{/if}
    <button class="text-action spaced" onclick={() => (allLocal = !allLocal)}
      >{allLocal ? "Show installed models" : "Other local models"}</button
    >
  </Sheet>{/if}

<style>
  .custom-instructions {
    display: grid;
    gap: 8px;
    width: 100%;
  }
  .custom-instructions textarea {
    width: 100%;
    min-width: 0;
  }
  .instructions-foot {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .char-count {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .error-text {
    color: var(--danger);
    margin: 0;
  }
  .instructions-actions {
    display: flex;
    justify-content: flex-start;
  }
  .section-heading {
    margin: 22px 0 8px 18px;
    font-size: 15px;
    font-weight: 600;
    color: var(--soft-ink);
  }
  .label-tail {
    white-space: nowrap;
  }
  .option-chevron {
    display: grid;
    place-items: center;
    color: var(--soft-ink);
    flex: none;
  }
</style>
