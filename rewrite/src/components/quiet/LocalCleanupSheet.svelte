<script lang="ts">
  import Sheet from "./Sheet.svelte";
  import MemoryBar from "./MemoryBar.svelte";
  import ModelDownload from "../ModelDownload.svelte";
  import { app } from "../../lib/app.svelte";
  import { library } from "../../lib/model-library.svelte";
  import { call } from "../../lib/api";
  import { ensureDownloadConsent } from "../../lib/download-consent.svelte";
  import { cleanupChoice, cleanupPatch } from "../../lib/cleanup-choice";
  import {
    checking,
    discoverable,
    installed,
    installing,
    selectable,
    formatBytes,
    type ModelEntry,
  } from "../../lib/model-library";
  import { SMART_CLEANUP_MODEL } from "../../lib/smart-cleanup";
  let { onclose }: { onclose: () => void } = $props();
  let expanded = $state(false);
  let warning = $state("");
  let pending = $state(false);
  let error = $state("");
  const approvedIds = new Set([
    SMART_CLEANUP_MODEL,
    "qwen3-4b-instruct-2507",
    "lfm2.5-1.2b",
    "lfm2.5-2.6b",
  ]);
  const models = $derived(
    (library.data?.models ?? []).filter(
      (model) =>
        model.kind === "cleanup" &&
        model.id !== "local-rules" &&
        model.roles.includes("cleanup") &&
        (approvedIds.has(model.id) ||
          model.id === app.snapshot.settings.cleanupModel) &&
        discoverable(model),
    ),
  );
  const firstIds = [SMART_CLEANUP_MODEL, "qwen3-4b-instruct-2507"];
  const leading = $derived(
    firstIds
      .map((id) => models.find((model) => model.id === id))
      .filter((model): model is ModelEntry => !!model),
  );
  const others = $derived(
    models.filter((model) => !firstIds.includes(model.id)),
  );
  const choice = $derived(
    cleanupChoice(
      app.snapshot.settings,
      app.snapshot.localProofreaderAvailable,
    ),
  );
  const rows = $derived(
    expanded
      ? [...leading, ...others]
      : [
          ...leading,
          ...others.filter(
            (model) => choice.kind === "local" && choice.model === model.id,
          ),
        ],
  );
  const blocked = $derived(
    !app.native ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      app.snapshot.meetingRecording ||
      ["loading", "listening", "finishing", "cleaning"].includes(
        app.snapshot.state,
      ) ||
      models.some((model) =>
        ["loading", "busy", "unloading"].includes(model.state),
      ),
  );
  const locked = $derived(blocked || pending);
  const downloading = $derived(
    models.find(
      (model) => installing(model) || model.id === library.downloadId,
    ),
  );
  async function use(model: ModelEntry) {
    if (locked || !model.supported || checking(model)) return;
    error = "";
    pending = true;
    try {
      if (!installed(model)) {
        if (!model.installable || models.some(installing)) return;
        await app.perform("model-install", async () => {
          if (!(await ensureDownloadConsent([model.id])) || blocked) return;
          library.downloadId = model.id;
          try {
            library.apply(await call("start_model_install", { id: model.id }));
          } finally {
            if (!library.data?.models.some(installing)) library.downloadId = "";
          }
        });
        error = app.error;
        warning = "";
        return;
      }
      if (!selectable(model, "cleanup")) return;
      await app.saveSettings({
        ...cleanupPatch({ kind: "local", model: model.id }),
        ...(app.snapshot.settings.setupProfile !== null
          ? { setupProfile: "custom" as const }
          : {}),
      });
      error = app.error;
      if (!error) onclose();
    } finally {
      pending = false;
    }
  }
  async function cancel(model: ModelEntry) {
    await app.perform("model-cancel", async () =>
      library.apply(await call("cancel_model_install", { id: model.id })),
    );
  }
</script>

<Sheet
  title="Local cleanup model"
  onclose={() => {
    if (!pending) onclose();
  }}
>
  <p class="field-help">Cleanup · estimated system RAM</p>
  {#each rows as model (model.id)}
    <div class="local-row">
      <button
        class="quiet-option"
        class:chosen={choice.kind === "local" && choice.model === model.id}
        aria-pressed={choice.kind === "local" && choice.model === model.id}
        disabled={locked ||
          !model.supported ||
          checking(model) ||
          installing(model)}
        onclick={() =>
          model.id === SMART_CLEANUP_MODEL
            ? void use(model)
            : (warning = model.id)}
      >
        <span class="choice-dot" aria-hidden="true"></span>
        <span class="option-copy"
          >{model.name}
          <small class="quiet-tag amber"
            >{model.id === SMART_CLEANUP_MODEL
              ? "EXPERIMENT"
              : "NOT RECOMMENDED"}</small
          ><small
            >{model.id === SMART_CLEANUP_MODEL
              ? "Specialized cleanup · Clean only"
              : model.id === "qwen3-4b-instruct-2507"
                ? "Not tested as a cleanup model"
                : model.guidance}</small
          ><small
            >{!model.supported
              ? model.compatibility
              : installed(model)
                ? "Installed"
                : `${formatBytes(model.diskBytes)} download`}</small
          ></span
        >
        <MemoryBar
          bytes={model.ramMaxBytes}
          total={library.data?.hardware.totalRamBytes ?? null}
        />
      </button>
      {#if warning === model.id}<div class="local-warning">
          <p>
            Changes to protected numbers, links or wording are rejected; the
            text from before cleanup is kept.
          </p>
          <div class="actions">
            <button class="button small" onclick={() => (warning = "")}
              >Cancel</button
            ><button
              class="button small"
              disabled={locked}
              onclick={() => void use(model)}
              >{installed(model) ? "Use anyway" : "Download anyway"}</button
            >
          </div>
        </div>{/if}
    </div>
  {/each}
  {#if !expanded && others.some((model) => !rows.includes(model))}<button
      class="text-action extras"
      onclick={() => (expanded = true)}
      >{others.filter((model) => !rows.includes(model)).length} untested models</button
    >{/if}
  {#if expanded || app.snapshot.localProofreaderAvailable}<button
      class="quiet-option"
      disabled={locked || !app.snapshot.localProofreaderAvailable}
      aria-pressed={choice.kind === "sonojot"}
      onclick={async () => {
        await app.saveSettings(cleanupPatch({ kind: "sonojot" }));
        if (!app.error) onclose();
      }}
      ><span class="choice-dot" aria-hidden="true"></span><span
        class="option-copy"
        >SonoJot proofreader <small class="quiet-tag amber">EXPERIMENT</small
        ><small
          >{app.snapshot.localProofreaderAvailable
            ? "Runs on this computer"
            : "Appears when its file is present · not found"}</small
        ></span
      ><MemoryBar bytes={null} total={null} /></button
    >{/if}
  {#if library.loading && !library.data}<p role="status">
      Loading local models…
    </p>{/if}
  {#if library.error}<p role="alert">{library.error}</p>
    <button class="button" onclick={() => void library.refresh()}
      >Try again</button
    >{/if}
  {#if downloading}<ModelDownload
      model={downloading}
      pending={pending || app.busy("model-cancel")}
      retryDisabled={locked}
      oncancel={cancel}
      onretry={use}
    />{/if}
  {#if error}<p role="alert">{error}</p>{/if}
</Sheet>

<style>
  .local-warning {
    margin: 0 12px 14px 32px;
    padding: 12px;
    border-left: 2px solid var(--warning-ink, #835d24);
    font-size: 15px;
  }
  .local-warning p {
    margin: 0 0 10px;
  }
  .extras {
    margin: 14px 12px;
  }
</style>
