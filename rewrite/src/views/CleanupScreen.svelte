<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icon.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import SegmentedControl from "../components/quiet/SegmentedControl.svelte";
  import Disclosure from "../components/quiet/Disclosure.svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import LocalCleanupSheet from "../components/quiet/LocalCleanupSheet.svelte";
  import MemoryBar from "../components/quiet/MemoryBar.svelte";
  import Toggle from "../components/Toggle.svelte";
  import PunctuationDetails from "../components/PunctuationDetails.svelte";
  import InfoTip from "../components/quiet/InfoTip.svelte";
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
  const onlineCostText = $derived.by(() => {
    const cost = textCost(online);
    return cost === "Price unavailable" ? "Price unknown" : cost;
  });
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

<div class="cleanup-page" aria-label="Cleanup settings">
  <section class="stock paper-card ui-card row-card">
    <SettingRow label="Level" layout="choice">
      <div class="level-control">
        <SegmentedControl
          label="Cleanup level"
          options={levels}
          value={settings.cleanupMode}
          disabled={locked}
          onchange={(cleanupMode) => void app.saveSettings({ cleanupMode })}
        />
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
    </SettingRow>
    {#if app.snapshot.capabilities.platform !== "linux"}
      <SettingRow
        label="Punctuation"
        layout="action"
        help={!settings.sonojotPunctuation
          ? "Off · basic rules only"
          : (punctuation?.name ?? "Basic rules until installed")}
      >
        {#snippet info()}
          <p>
            Adds punctuation, capitals and sentence breaks on this computer,
            right after you stop. In Clean, SonoJot Cleanup v4 also removes
            fillers, repeated words and restarts.
          </p>
          <p>
            It does not fix misheard words or grammar; that is what a
            proofreader does.
          </p>
        {/snippet}
        {#if !punctuation && settings.sonojotPunctuation}<button
            class="button small"
            aria-label="Install punctuation model"
            onclick={() => onmanage("punctuation")}>Install</button
          >{/if}
        <Toggle
          label="Punctuation model"
          checked={settings.sonojotPunctuation}
          disabled={locked || settings.cleanupMode === "original"}
          onchange={(sonojotPunctuation) =>
            void app.saveSettings({ sonojotPunctuation })}
        />
      </SettingRow>
    {/if}
  </section>

  <div class="section-heading">
    <span class="label-tail"
      >Proofreader<InfoTip label="About Proofreader"
        ><p>
          An optional writing model that reads your words after punctuation. It
          can fix misheard words and grammar, and it adds a wait after you stop.
        </p>
        <p>
          Protected numbers, links and wording are checked; if a change looks
          unsafe, the text from before proofreading is kept.
        </p></InfoTip
      ></span
    >
  </div>
  <section
    class="stock paper-card ui-card option-card option-list"
    role="group"
    aria-label="Proofreader"
  >
    <button
      class="quiet-option"
      class:chosen={choice.kind === "off"}
      aria-pressed={choice.kind === "off"}
      disabled={locked}
      onclick={() => void app.saveSettings(cleanupPatch({ kind: "off" }))}
      ><span class="choice-dot" aria-hidden="true"></span><span
        class="option-copy"
        ><span class="option-title"
          ><strong data-text-surface="emboss">None</strong><small
            class="quiet-tag">RECOMMENDED</small
          ></span
        ><small>Fastest · nothing extra to load</small></span
      ><MemoryBar
        bytes={0}
        total={library.data?.hardware.totalRamBytes ?? null}
      /><span class="option-chevron spacer" aria-hidden="true"></span></button
    >
    <button
      class="quiet-option"
      class:chosen={choice.kind === "local" || choice.kind === "sonojot"}
      aria-pressed={choice.kind === "local" || choice.kind === "sonojot"}
      disabled={locked}
      aria-label="Choose a proofreader on this computer"
      onclick={() => (sheet = "local")}
      ><span class="choice-dot" aria-hidden="true"></span><span
        class="option-copy"
        ><span class="option-title"
          ><strong data-text-surface="emboss"
            >{choice.kind === "sonojot"
              ? "SonoJot proofreader"
              : (local?.name ?? "On this computer")}</strong
          >{#if choice.kind === "sonojot" || local?.id === SMART_CLEANUP_MODEL}<small
              class="quiet-tag amber">Experiment</small
            >{/if}</span
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
      /><span class="option-chevron" aria-hidden="true"
        ><Icon name="chevron-right" size={15} /></span
      ></button
    >
    <button
      class="quiet-option"
      class:chosen={choice.kind === "online"}
      aria-pressed={choice.kind === "online"}
      disabled={locked}
      aria-label="Choose an online proofreader"
      onclick={() => openRouter.open("cleanup", onlineId)}
      ><span class="choice-dot" aria-hidden="true"></span><span
        class="option-copy"
        ><span class="option-title"
          ><strong data-text-surface="emboss"
            >{online?.name ?? onlineCleanupName(onlineId)}</strong
          >{#if ai.data?.connected}<small class="quiet-tag">Connected</small
            >{/if}</span
        ><small>Online · OpenRouter</small></span
      ><span class="option-meta">{onlineCostText}</span><span
        class="option-chevron"
        aria-hidden="true"><Icon name="chevron-right" size={15} /></span
      ></button
    >
  </section>

  <section class="stock paper-card ui-card row-card">
    <button class="settings-link" onclick={() => (sheet = "more")}
      ><strong data-text-surface="emboss">Advanced</strong><span
        class="door-detail"
        data-text-surface="flat">Written form · shape by type</span
      ><span class="door-chevron" aria-hidden="true"
        ><Icon name="chevron-right" size={15} /></span
      ></button
    >
  </section>
</div>

{#if sheet === "local"}<LocalCleanupSheet onclose={() => (sheet = null)} />{/if}
{#if sheet === "more"}<Sheet title="Advanced" onclose={() => (sheet = null)}>
    <p class="sheet-context" data-text-surface="flat">Cleanup</p>
    <SettingRow
      label="Written form"
      help="three → 3 · gpt six → GPT-6"
      layout="action"
    >
      <Toggle
        label="Written form"
        checked={settings.writtenForm}
        disabled={locked || settings.cleanupMode !== "clean"}
        onchange={(writtenForm) => void app.saveSettings({ writtenForm })}
      />
    </SettingRow>
    <SettingRow
      label="Shape by type"
      help={layoutUnavailable ?? "Emails, lists and steps"}
      layout="action"
    >
      {#snippet labelExtra()}<small class="quiet-tag amber">Experiment</small
        >{/snippet}
      <Toggle
        label="Shape by type"
        checked={settings.smartLayout}
        disabled={locked || !!layoutUnavailable}
        onchange={(smartLayout) => void app.saveSettings({ smartLayout })}
      />
    </SettingRow>
    {#if settings.smartLayout}
      <SettingRow
        label="Use the app I'm in"
        help="Outlook → email"
        layout="action"
      >
        {#snippet labelExtra()}<small class="quiet-tag amber">Experiment</small
          >{/snippet}
        <Toggle
          label="Use the app I'm in"
          checked={settings.smartLayoutAppHint}
          disabled={locked || !!layoutUnavailable}
          onchange={(smartLayoutAppHint) =>
            void app.saveSettings({ smartLayoutAppHint })}
        />
      </SettingRow>
    {/if}
    {#if app.snapshot.capabilities.platform !== "linux"}
      <div class="sheet-fold">
        <Disclosure title="Punctuation details">
          <PunctuationDetails id={punctuation?.id ?? PUNCTUATION_MODEL} inline />
        </Disclosure>
      </div>
    {/if}
  </Sheet>{/if}

<style>
  .level-control {
    display: grid;
    justify-items: start;
    gap: 12px;
    width: 100%;
  }
  .cleanup-example {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 10px;
    font-size: 15px;
    line-height: 1.45;
  }
  .cleanup-example > span:first-child {
    color: var(--soft-ink);
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
    width: 16px;
    color: var(--soft-ink);
    flex: 0 0 16px;
  }
  .option-chevron.spacer {
    visibility: hidden;
  }
  .sheet-context {
    margin: -8px 0 8px;
    font-size: 14px;
    color: var(--soft-ink);
  }
  .sheet-fold {
    border-top: 1px solid var(--line);
    margin-top: 4px;
  }
</style>
