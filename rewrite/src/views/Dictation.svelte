<script lang="ts">
  import { onMount } from "svelte";
  import Card from "../components/Card.svelte";
  import SettingRow from "../components/SettingRow.svelte";
  import Toggle from "../components/Toggle.svelte";
  import ProofreaderSettings from "../components/ProofreaderSettings.svelte";
  import DictationHistory from "../components/DictationHistory.svelte";
  import TransientDictation from "../components/TransientDictation.svelte";
  import ModelSummary from "../components/ModelSummary.svelte";
  import MacPasteAccess from "../components/MacPasteAccess.svelte";
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import {
    SMART_LAYOUT_HELP,
    SMART_LAYOUT_LABEL,
    SMART_LAYOUT_APP_HINT_LABEL,
    SMART_LAYOUT_APP_HINT_HELP,
    smartLayoutUnavailableReason,
  } from "../lib/smart-layout";
  import type { ModelRole } from "../lib/model-library";
  import { LIVE_PREVIEW_MODEL, livePreviewHelpText } from "../lib/live-preview";
  import type { Settings } from "../lib/protocol";
  let {
    onmanage,
    initialView = "preferences",
    reviewSessionId = null,
  }: {
    onmanage?: (role: ModelRole) => void;
    initialView?: "preferences" | "recent";
    reviewSessionId?: number | null;
  } = $props();
  let view = $state<"preferences" | "recent">("preferences");
  $effect(() => {
    view = initialView;
  });
  async function changeView(next: typeof view): Promise<void> {
    if (view === next) return;
    if (
      app.dirty &&
      !(await app.confirm(
        "Leave these changes?",
        "This dictation has changes that have not been saved.",
      ))
    )
      return;
    view = next;
  }
  const modes: {
    value: Settings["cleanupMode"];
    title: string;
    subtitle: string;
    sample: string;
  }[] = [
    {
      value: "original",
      title: "As heard",
      subtitle: "Keep recognition unchanged",
      sample: "um please keep teh backup. i need it tomorrow. send it to me",
    },
    {
      value: "format",
      title: "Format only",
      subtitle: "Keep wording, improve layout",
      sample: "Um please keep teh backup. I need it tomorrow. Send it to me.",
    },
    {
      value: "clean",
      title: "Clean",
      subtitle: "Remove clutter, keep intent",
      sample: "Please keep the backup. I need it tomorrow. Send it to me.",
    },
  ];
  const mode = $derived(
    modes.find((item) => item.value === app.snapshot.settings.cleanupMode) ??
      modes[2],
  );
  const active = $derived(
    ["listening", "loading", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const layoutUnavailable = $derived(
    smartLayoutUnavailableReason(
      app.snapshot.settings,
      library.data?.models ?? [],
    ),
  );
  const livePreviewModel = $derived(
    library.data?.models.find((model) => model.id === LIVE_PREVIEW_MODEL),
  );
  const livePreviewHelp = $derived(
    livePreviewHelpText(livePreviewModel, app.snapshot.settings.asrModel),
  );
  onMount(() => library.acquire());
</script>

<header class="page-heading dictation-guide">
  <h1 class="text-grain" data-text-surface="emboss">Dictation</h1>
  <p data-text-surface="flat">
    {app.snapshot.settings.hotkeyMode === "hold" ? "Hold" : "Press"}
    <kbd>{app.snapshot.settings.hotkey.replaceAll("+", " + ")}</kbd>
    {app.snapshot.settings.hotkeyMode === "hold"
      ? "to dictate; release to finish."
      : "to start or finish."}
    {app.snapshot.capabilities.platform === "macos"
      ? "You can also record from the menu bar."
      : "You can also record from the screen tab or tray menu."}
  </p>
</header>
<TransientDictation {reviewSessionId} />
<div class="segments dictation-views" role="group" aria-label="Dictation views">
  <button
    aria-pressed={view === "preferences"}
    onclick={() => void changeView("preferences")}>Preferences</button
  >
  <button
    aria-pressed={view === "recent"}
    onclick={() => void changeView("recent")}>Recent</button
  >
</div>
{#if view === "recent"}
  <DictationHistory />
{:else}
  <Card title="Text cleanup">
    <div class="segments" role="group" aria-label="Dictation cleanup mode">
      {#each modes as item}<button
          type="button"
          aria-pressed={mode.value === item.value}
          disabled={app.busy("settings") || active}
          onclick={() => app.saveSettings({ cleanupMode: item.value })}
          >{item.title}<small data-text-surface="flat">{item.subtitle}</small
          ></button
        >{/each}
    </div>
    <div class="shape-options">
      <ProofreaderSettings {active} />
      <SettingRow
        label={SMART_LAYOUT_LABEL}
        help={layoutUnavailable ?? SMART_LAYOUT_HELP}
      >
        {#if layoutUnavailable}
          <button
            class="button quiet small"
            disabled={!app.native ||
              app.busy("settings") ||
              active ||
              (mode.value !== "original" && !onmanage)}
            onclick={() =>
              mode.value === "original"
                ? app.saveSettings({ cleanupMode: "format" })
                : onmanage?.("notes")}
            >{mode.value === "original"
              ? "Use Format only"
              : "Choose text model"}</button
          >
        {:else}
          <Toggle
            label={SMART_LAYOUT_LABEL}
            checked={app.snapshot.settings.smartLayout}
            disabled={!app.native || app.busy("settings") || active}
            onchange={(smartLayout) => app.saveSettings({ smartLayout })}
          />
        {/if}
      </SettingRow>
      <SettingRow
        label={SMART_LAYOUT_APP_HINT_LABEL}
        help={SMART_LAYOUT_APP_HINT_HELP}
      >
        <Toggle
          label={SMART_LAYOUT_APP_HINT_LABEL}
          checked={app.snapshot.settings.smartLayoutAppHint}
          disabled={!app.native ||
            app.busy("settings") ||
            active ||
            !app.snapshot.settings.smartLayout ||
            !!layoutUnavailable}
          onchange={(smartLayoutAppHint) =>
            app.saveSettings({ smartLayoutAppHint })}
        />
      </SettingRow>
    </div>
    <div class="paper-well">
      <div class="well-label" data-text-surface="flat">Example</div>
      <div class="example-output">{mode.sample}</div>
    </div>
    <SettingRow
      label="Paste into the app I was using"
      help={app.snapshot.capabilities.autoPaste
        ? "Otherwise the text is copied and waits for you."
        : "Automatic paste is unavailable in this environment."}
      ><Toggle
        label="Paste into the app I was using"
        checked={app.snapshot.settings.autoPaste}
        disabled={!app.native ||
          !app.snapshot.capabilities.autoPaste ||
          app.busy("settings")}
        onchange={(autoPaste) => app.saveSettings({ autoPaste })}
      /></SettingRow
    >
    {#if app.native && app.snapshot.capabilities.platform === "macos"}<MacPasteAccess
        variant="settings"
      />{/if}
    <details class="more-options license">
      <summary>More options</summary>
      <SettingRow
        label="Sound cues"
        help="Hear a short cue when dictation starts and stops."
      >
        <Toggle
          label="Sound cues"
          checked={app.snapshot.settings.audioCues}
          disabled={!app.native || app.busy("settings")}
          onchange={(audioCues) => app.saveSettings({ audioCues })}
        />
      </SettingRow>
      <SettingRow label="Live preview" help={livePreviewHelp}
        ><Toggle
          label="Live preview"
          checked={app.snapshot.settings.livePreview}
          disabled={!app.native ||
            app.busy("settings") ||
            livePreviewModel?.supported === false}
          onchange={(livePreview) => app.saveSettings({ livePreview })}
        /></SettingRow
      >
      <SettingRow
        label="Written form"
        help="Write numbers, symbols and AI model names in their usual form. Applies to Clean."
        ><Toggle
          label="Written form"
          checked={app.snapshot.settings.writtenForm}
          disabled={!app.native ||
            app.busy("settings") ||
            active ||
            mode.value !== "clean"}
          onchange={(writtenForm) => app.saveSettings({ writtenForm })}
        /></SettingRow
      >
      <div class="section-toolbar">
        <h3 data-text-surface="emboss">Models in use</h3>
        <button
          class="button quiet small"
          onclick={() => onmanage?.("asr")}
          disabled={!onmanage}>Change</button
        >
      </div>
      <ModelSummary role="asr" {onmanage} />
      <ModelSummary role="cleanup" {onmanage} />
      <ModelSummary role="notes" {onmanage} />
    </details>
  </Card>
{/if}

<style>
  .shape-options {
    margin-bottom: 18px;
    padding-bottom: 18px;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .dictation-views {
    max-width: 360px;
    margin-bottom: 18px;
  }
  .more-options {
    margin-top: 18px;
    border-top: 1px solid var(--line);
    padding-top: 18px;
  }
</style>
