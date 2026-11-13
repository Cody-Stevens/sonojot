<script lang="ts">
  import { onMount, onDestroy } from "svelte";
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

<section class="stock paper-card quiet-panel">
  <div class="stage-setting">
    <span class="label-with-info"
      ><label for="notes-style">Style</label><InfoTip label="About notes style"
        ><p>
          Notes are written after you press Stop and the transcript is ready.
          Notes you already have are kept.
        </p></InfoTip
      ></span
    >
    <div class="stage-control">
      <select
        id="notes-style"
        value={preset ?? ""}
        disabled={locked}
        onchange={(event) => {
          preset =
            notesStyles.find((style) => style.id === event.currentTarget.value)
              ?.id ?? null;
          void saveDefaults();
        }}
        ><option value="">No notes</option>{#each notesStyles as style}<option
            value={style.id}>{style.label}</option
          >{/each}</select
      >
    </div>
  </div>
  {#if preset === "custom"}<div class="stage-setting">
      <label for="notes-default-instructions">Instructions</label><textarea
        id="notes-default-instructions"
        rows="3"
        bind:value={instructions}
        disabled={locked}></textarea>
      <small class="field-help"
        >{instructionLength(instructions)} / {NOTES_INSTRUCTIONS_LIMIT} characters</small
      >
      {#if instructionsError}<p class="field-help" role="alert">
          {instructionsError}
        </p>{/if}
      {#if draft}<button
          class="button"
          disabled={locked || !!instructionsError}
          onclick={saveDefaults}>Save instructions</button
        >{/if}
    </div>{/if}
  <div class="stage-setting writers">
    <span class="label-with-info"
      >Written by<InfoTip label="About Written by"
        ><p>
          {app.snapshot.settings.meetingNotesPreset === null
            ? "With No notes, nothing is written automatically. You can still choose Write notes in a meeting."
            : "Notes use this model after transcription. You can cancel while they are being written."}
        </p></InfoTip
      ></span
    >
    <div>
      <button
        class="quiet-option"
        aria-pressed={!remote}
        disabled={locked}
        onclick={() => (localSheet = true)}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >{local?.name ?? "Local notes model"}<small
            >This computer · {local
              ? `${formatBytes(local.diskBytes)} ${installed(local) ? "on disk" : "download"}`
              : "Choose a model"}</small
          ></span
        ><MemoryBar
          bytes={local?.ramMaxBytes ?? null}
          total={library.data?.hardware.totalRamBytes ?? null}
        /><span aria-hidden="true">›</span></button
      >
      <button
        class="quiet-option"
        aria-pressed={remote}
        disabled={locked}
        onclick={() => openRouter.open("notes")}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >{online?.name || ai.data?.model || "OpenRouter"}<small
            >Online · {textCost(online)}</small
          ></span
        >{#if ai.data?.connected}<span class="quiet-tag">CONNECTED</span
          >{/if}<span aria-hidden="true">›</span></button
      >
    </div>
  </div>
  <div class="stage-setting">
    <span class="label-with-info"
      >Name meetings automatically<InfoTip label="About naming meetings"
        ><p>Uses a model on this computer after transcription.</p></InfoTip
      ></span
    ><Toggle
      label="Name meetings automatically"
      checked={app.snapshot.settings.autoMeetingTitles}
      disabled={locked}
      onchange={(autoMeetingTitles) => app.saveSettings({ autoMeetingTitles })}
    />
  </div>
</section>
{#if localSheet}<Sheet
    title="Local notes model"
    onclose={() => {
      if (!pending) localSheet = false;
    }}
  >
    {#if app.error}<p role="alert">{app.error}</p>{/if}
    {#each models.filter((model) => allLocal || model.id === local?.id || installed(model)) as model}<button
        class="quiet-option"
        aria-pressed={!remote && model.id === app.snapshot.settings.notesModel}
        disabled={locked || !model.supported || installing(model)}
        onclick={() => choose(model)}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >{model.name}<small
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
    <button class="text-action" onclick={() => (allLocal = !allLocal)}
      >{allLocal ? "Show installed models" : "Other local models"}</button
    >
  </Sheet>{/if}

<style>
  .stage-control {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 9px;
  }
  .writers {
    align-items: flex-start;
  }
  .writers > div {
    flex: 1;
  }
  .writers > span {
    padding-top: 18px;
  }
  .writers .quiet-option {
    width: 100%;
  }
  select,
  textarea {
    min-width: 200px;
  }
  .quiet-option {
    gap: 12px;
  }
  .quiet-panel {
    padding: 18px 24px;
  }
</style>
