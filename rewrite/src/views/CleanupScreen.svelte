<script lang="ts">
  import { onMount } from "svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import LocalCleanupSheet from "../components/quiet/LocalCleanupSheet.svelte";
  import MemoryBar from "../components/quiet/MemoryBar.svelte";
  import Toggle from "../components/Toggle.svelte";
  import PunctuationDetails from "../components/PunctuationDetails.svelte";
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import { ai } from "../lib/ai.svelte";
  import { call } from "../lib/api";
  import { openRouter } from "../lib/openrouter.svelte";
  import {
    cleanupChoice,
    cleanupPatch,
    onlineCleanupName,
  } from "../lib/cleanup-choice";
  import { smartLayoutUnavailableReason } from "../lib/smart-layout";
  import { activePunctuation, PUNCTUATION_MODEL } from "../lib/punctuation";
  import { textCost, type AiModel } from "../lib/ai";
  import { SMART_CLEANUP_MODEL } from "../lib/smart-cleanup";
  import {
    formatBytes,
    installed,
    type ModelsTarget,
  } from "../lib/model-library";
  import type { Settings } from "../lib/protocol";
  let { onmanage }: { onmanage: (target: ModelsTarget) => void } = $props();
  let sheet = $state<"local" | "more" | null>(null);
  let catalog = $state<AiModel[]>([]);
  let disposed = false;
  const settings = $derived(app.snapshot.settings);
  const models = $derived(library.data?.models ?? []);
  const choice = $derived(
    cleanupChoice(settings, app.snapshot.localProofreaderAvailable),
  );
  const local = $derived(
    models.find(
      (model) =>
        model.id ===
        (choice.kind === "local" ? choice.model : SMART_CLEANUP_MODEL),
    ) ?? models.find((model) => model.id === "qwen3-4b-instruct-2507"),
  );
  const onlineId = $derived(
    choice.kind === "online" ? choice.model : settings.proofreaderModel,
  );
  const online = $derived(catalog.find((model) => model.id === onlineId));
  const punctuation = $derived(activePunctuation(models));
  const layoutUnavailable = $derived(
    smartLayoutUnavailableReason(
      settings,
      models,
      app.snapshot.localProofreaderAvailable,
    ),
  );
  const locked = $derived(
    !app.native ||
      app.busy("settings") ||
      app.busy("setup-profile") ||
      ai.busy ||
      app.snapshot.meetingRecording ||
      ["loading", "listening", "finishing", "cleaning"].includes(
        app.snapshot.state,
      ),
  );
  const levels: {
    value: Settings["cleanupMode"];
    label: string;
    after: string;
  }[] = [
    {
      value: "original",
      label: "As heard",
      after: "um so the the meeting is at three",
    },
    {
      value: "format",
      label: "Format",
      after: "Um, so the, the meeting is at three.",
    },
    { value: "clean", label: "Clean", after: "The meeting is at 3." },
  ];
  onMount(() => {
    const release = library.acquire();
    void ai.refresh();
    void call("list_ai_models")
      .then((models) => {
        if (!disposed) catalog = models;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      release();
    };
  });
</script>

<section class="stock paper-card quiet-panel" aria-label="Cleanup settings">
  <div class="stage-setting">
    <span>Level</span>
    <div class="stage-control level-control">
      <div class="segments" role="group" aria-label="Cleanup level">
        {#each levels as level}<button
            disabled={locked}
            aria-pressed={settings.cleanupMode === level.value}
            onclick={() => void app.saveSettings({ cleanupMode: level.value })}
            >{level.label}</button
          >{/each}
      </div>
      <div class="cleanup-example" aria-label="Cleanup example">
        <span
          >{#if settings.cleanupMode === "clean"}<s>um so</s> the <s>the</s> meeting
            is at three{:else}um so the the meeting is at three{/if}</span
        ><span aria-hidden="true">→</span><span
          >{levels.find((level) => level.value === settings.cleanupMode)
            ?.after}</span
        >
      </div>
    </div>
  </div>
  <div class="stage-setting cleaner-setting">
    <span>Done by</span>
    <div class="cleaners">
      <button
        class="quiet-option"
        class:chosen={choice.kind === "off"}
        aria-pressed={choice.kind === "off"}
        disabled={locked}
        onclick={() => void app.saveSettings(cleanupPatch({ kind: "off" }))}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >Rules <small class="quiet-tag">RECOMMENDED</small><small
            >Instant · nothing extra to load</small
          ></span
        ><MemoryBar
          bytes={0}
          total={library.data?.hardware.totalRamBytes ?? null}
        /></button
      >
      <button
        class="quiet-option"
        class:chosen={choice.kind === "local" || choice.kind === "sonojot"}
        aria-pressed={choice.kind === "local" || choice.kind === "sonojot"}
        disabled={locked}
        aria-label="Choose local cleanup model"
        onclick={() => (sheet = "local")}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >{choice.kind === "sonojot"
            ? "SonoJot proofreader"
            : (local?.name ?? "Local cleanup model")}
          <small class="quiet-tag amber"
            >{choice.kind === "sonojot" || local?.id === SMART_CLEANUP_MODEL
              ? "EXPERIMENT"
              : "NOT RECOMMENDED"}</small
          ><small
            >This computer{local && choice.kind !== "sonojot"
              ? ` · ${formatBytes(local.diskBytes)} ${installed(local) ? "on disk" : "download"}`
              : ""}</small
          ></span
        ><MemoryBar
          bytes={choice.kind === "sonojot"
            ? null
            : (local?.ramMaxBytes ?? null)}
          total={library.data?.hardware.totalRamBytes ?? null}
        /><span aria-hidden="true">›</span></button
      >
      <button
        class="quiet-option"
        class:chosen={choice.kind === "online"}
        aria-pressed={choice.kind === "online"}
        disabled={locked}
        aria-label="Choose online cleanup model"
        onclick={() => openRouter.open("cleanup", onlineId)}
        ><span class="choice-dot" aria-hidden="true"></span><span
          class="option-copy"
          >{online?.name ?? onlineCleanupName(onlineId)}
          {#if ai.data?.connected}<small class="quiet-tag">CONNECTED</small
            >{/if}<small>Online · OpenRouter · {textCost(online)}</small></span
        ><small class="online-memory">Online · no model in RAM</small><span
          aria-hidden="true">›</span
        ></button
      >
    </div>
  </div>
  <button class="more-door" onclick={() => (sheet = "more")}
    ><span>More</span><small>Punctuation · written form · shape by type</small
    ><span aria-hidden="true">›</span></button
  >
</section>

{#if sheet === "local"}<LocalCleanupSheet onclose={() => (sheet = null)} />{/if}
{#if sheet === "more"}<Sheet title="More" onclose={() => (sheet = null)}>
    <p class="sheet-context" data-text-surface="flat">Cleanup</p>
    {#if app.snapshot.capabilities.platform !== "linux"}
      <div class="stage-setting wide-label">
        <span
          >Punctuation model<small
            >{punctuation?.name ?? "Basic punctuation until installed"}</small
          ></span
        ><Toggle
          label="Punctuation model"
          checked={settings.sonojotPunctuation}
          disabled={locked || settings.cleanupMode === "original"}
          onchange={(sonojotPunctuation) =>
            void app.saveSettings({ sonojotPunctuation })}
        />
      </div>
      {#if !punctuation && settings.sonojotPunctuation}<button
          class="button small"
          onclick={() => {
            sheet = null;
            onmanage("punctuation");
          }}>Install punctuation model</button
        >{/if}
    {/if}
    <div class="stage-setting wide-label">
      <span>Written form<small>three → 3 · gpt six → GPT-6</small></span><Toggle
        label="Written form"
        checked={settings.writtenForm}
        disabled={locked || settings.cleanupMode !== "clean"}
        onchange={(writtenForm) => void app.saveSettings({ writtenForm })}
      />
    </div>
    <div class="stage-setting wide-label">
      <span
        >Shape by type <small class="quiet-tag amber">EXPERIMENT</small><small
          >{layoutUnavailable ?? "Emails, lists and steps"}</small
        ></span
      ><Toggle
        label="Shape by type"
        checked={settings.smartLayout}
        disabled={locked || !!layoutUnavailable}
        onchange={(smartLayout) => void app.saveSettings({ smartLayout })}
      />
    </div>
    {#if settings.smartLayout}<div class="stage-setting wide-label">
        <span
          >Use the app I'm in <small class="quiet-tag amber">EXPERIMENT</small
          ><small>Outlook → email</small></span
        ><Toggle
          label="Use the app I'm in"
          checked={settings.smartLayoutAppHint}
          disabled={locked || !!layoutUnavailable}
          onchange={(smartLayoutAppHint) =>
            void app.saveSettings({ smartLayoutAppHint })}
        />
      </div>{/if}
    {#if app.snapshot.capabilities.platform !== "linux"}<details
        class="license spaced"
      >
        <summary>Punctuation details</summary><PunctuationDetails
          id={punctuation?.id ?? PUNCTUATION_MODEL}
          inline
        />
      </details>{/if}
  </Sheet>{/if}

<style>
  .level-control {
    display: block;
  }
  .level-control :global(.segments) {
    width: fit-content;
  }
  .cleanup-example {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    font-size: 15px;
    margin: 16px 0 4px;
  }
  .cleanup-example > span:first-child {
    color: var(--soft-ink);
  }
  .cleaner-setting {
    align-items: start;
  }
  .cleaner-setting > span {
    padding-top: 14px;
  }
  .cleaners {
    min-width: 0;
  }
  .online-memory {
    width: 155px;
    flex: 0 0 155px;
    font-size: 13px;
  }
  .more-door {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 20px 0;
    color: var(--ink);
    border: 0;
    border-top: 1px solid var(--line);
    background: transparent;
    text-align: left;
    font-size: 18px;
  }
  .more-door small {
    flex: 1;
    color: var(--soft-ink);
  }
  .sheet-context {
    margin-top: -8px;
    color: var(--soft-ink);
  }
  .wide-label > span > small:not(.quiet-tag) {
    display: block;
    margin-top: 4px;
  }
</style>
