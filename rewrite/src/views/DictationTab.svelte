<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../components/Icon.svelte";
  import Meter from "../components/Meter.svelte";
  import Chip from "../components/quiet/Chip.svelte";
  import DayPages from "../components/quiet/DayPages.svelte";
  import DictationControl from "../components/DictationControl.svelte";
  import TransientDictation from "../components/TransientDictation.svelte";
  import TranscriptPreview from "../components/TranscriptPreview.svelte";
  import MacPasteAccess from "../components/MacPasteAccess.svelte";
  import { app } from "../lib/app.svelte";
  import { library } from "../lib/model-library.svelte";
  import { ai } from "../lib/ai.svelte";
  import { installed } from "../lib/model-library";
  import { isGpuAsrModel, isMetalAsrModel } from "../lib/acceleration";
  import { cleanupChoice, onlineCleanupName } from "../lib/cleanup-choice";
  import { dictationReady } from "../lib/dictation-control";
  import { presentError } from "../lib/errors";
  import type { RouteTarget } from "../lib/route.svelte";
  let {
    onnavigate,
    reviewSessionId = null,
  }: {
    onnavigate: (route: RouteTarget) => unknown;
    reviewSessionId?: number | null;
  } = $props();
  const settings = $derived(app.snapshot.settings);
  let reviewing = $state(false);
  const active = $derived(
    ["loading", "listening", "finishing", "cleaning"].includes(
      app.snapshot.state,
    ),
  );
  const mic = $derived(
    app.snapshot.devices.find((device) =>
      settings.microphone === null
        ? device.isDefault
        : device.id === settings.microphone,
    ),
  );
  const speech = $derived(
    library.data?.models.find((model) => model.id === settings.asrModel),
  );
  const speechName = $derived(
    speech?.name ??
      app.snapshot.models.find((model) => model.id === settings.asrModel)
        ?.name ??
      "Choose speech model",
  );
  const speechReady = $derived(
    app.snapshot.models.some(
      (model) => model.id === settings.asrModel && model.ready,
    ),
  );
  const choice = $derived(
    cleanupChoice(settings, app.snapshot.localProofreaderAvailable),
  );
  const cleanupModel = $derived(
    choice.kind === "local"
      ? library.data?.models.find((model) => model.id === choice.model)
      : null,
  );
  const cleaner = $derived(
    choice.kind === "off"
      ? ""
      : choice.kind === "sonojot"
        ? "SonoJot proofreader"
        : choice.kind === "online"
          ? onlineCleanupName(choice.model)
          : (cleanupModel?.name ?? choice.model),
  );
  const cleanupHealth = $derived(
    settings.cleanupMode !== "original" &&
      (choice.kind === "online"
        ? ai.data !== null && !ai.data.connected
        : choice.kind === "local"
          ? !!cleanupModel && !installed(cleanupModel)
          : false),
  );
  const shortcutConflict = $derived(
    !!(app.error || app.snapshot.error) &&
      presentError(app.error || app.snapshot.error).family === "shortcut",
  );
  const status = $derived(
    app.snapshot.state === "cleaning"
      ? "Cleaning"
      : app.snapshot.state === "finishing"
        ? "Finishing"
        : app.snapshot.state === "loading"
          ? "Listening · loading speech"
          : app.snapshot.state === "listening"
            ? "Listening"
            : app.snapshot.meetingRecording
              ? "Meeting recording"
              : dictationReady(app.snapshot)
                ? "Ready"
                : "Setup needed",
  );
  onMount(() => {
    void ai.refresh();
    return library.acquire();
  });
</script>

<section class="stock paper-card dictation-band" aria-label="Dictation status">
  <div class="band-state">
    <span
      class="state-ring"
      class:listening={app.snapshot.state === "listening" ||
        app.snapshot.state === "loading"}
      ><Icon name="microphone" size={23} /></span
    >
    <h1 data-text-surface="emboss" aria-live="polite">{status}</h1>
    {#if active}<div class="band-controls">
        <DictationControl compact />
      </div>{/if}
  </div>
  {#if active}
    {#if ["listening", "loading"].includes(app.snapshot.state)}<div
        class="band-meter"
      >
        <Meter level={app.snapshot.audioLevel} active />
      </div>{/if}
    {#if app.snapshot.interimText || app.snapshot.previewText}<TranscriptPreview
        text={[app.snapshot.interimText, app.snapshot.previewText]
          .filter(Boolean)
          .join(" ")}
      />{/if}
  {:else if !reviewing}
    <div class="dictation-stages" role="group" aria-label="Dictation settings">
      <Chip
        label="Shortcut and mic"
        onclick={() => onnavigate({ tab: "dictation", stage: "input" })}
      >
        <span class="keycaps"
          >{#each settings.hotkey.split("+") as key}<kbd>{key}</kbd
            >{/each}</span
        >
        {#if shortcutConflict}<span class="quiet-tag amber">TAKEN</span
          >{/if}<span class="stage-dot">·</span><Icon
          name="microphone"
          size={15}
        /><span class="chip-name">{mic?.name ?? "Choose mic"}</span
        >{#if !mic}<span class="health-dot" aria-label="Microphone unavailable"
          ></span>{/if}
      </Chip>
      <span class="stage-arrow" aria-hidden="true">→</span>
      <Chip
        label="Speech"
        onclick={() => onnavigate({ tab: "dictation", stage: "speech" })}
        ><span class="chip-name">{speechName}</span><span class="quiet-tag"
          >{isGpuAsrModel(settings.asrModel) ||
          isMetalAsrModel(settings.asrModel)
            ? "GPU"
            : "CPU"}</span
        >{#if !speechReady}<span
            class="health-dot"
            aria-label="Speech model needs attention"
          ></span>{/if}</Chip
      >
      <span class="stage-arrow" aria-hidden="true">→</span>
      <Chip
        label="Cleanup"
        onclick={() => onnavigate({ tab: "dictation", stage: "cleanup" })}
        ><span
          >{settings.cleanupMode === "original"
            ? "As heard"
            : settings.cleanupMode === "format"
              ? "Format"
              : "Clean"}</span
        >{#if cleaner}<span class="stage-dot">·</span><span class="chip-name"
            >{cleaner}</span
          >{/if}{#if cleanupHealth}<span
            class="health-dot"
            aria-label="Cleanup needs attention"
          ></span>{/if}</Chip
      >
      <span class="stage-arrow" aria-hidden="true">→</span>
      <Chip
        label="Paste and history"
        onclick={() => onnavigate({ tab: "dictation", stage: "output" })}
        ><span>{settings.autoPaste ? "Paste" : "Clipboard"}</span><span
          class="stage-dot">·</span
        ><span
          >{settings.doNotSaveDictations
            ? "No history"
            : settings.keepDictationAudio
              ? "Text + audio"
              : "History"}</span
        ></Chip
      >
    </div>
  {/if}
  <TransientDictation {reviewSessionId} />
  {#if app.native && app.snapshot.capabilities.platform === "macos"}<MacPasteAccess
      variant="settings"
    />{/if}
</section>
<DayPages onselection={(open) => (reviewing = open)} />

<style>
  .dictation-band {
    padding: 22px 24px;
    margin-bottom: 0;
  }
  .band-state {
    display: flex;
    align-items: center;
    gap: 13px;
    margin-bottom: 18px;
  }
  h1 {
    font-size: 28px;
    line-height: 1.2;
    margin: 0;
  }
  .state-ring {
    width: 38px;
    height: 38px;
    display: grid;
    place-items: center;
    border: 2px solid color-mix(in srgb, var(--dye) 32%, transparent);
    border-radius: 50%;
    color: var(--dye);
    box-shadow: var(--press);
  }
  .state-ring.listening {
    color: var(--danger);
    border-color: var(--danger);
  }
  .band-controls {
    margin-left: auto;
  }
  .band-meter {
    width: 180px;
    margin-left: 51px;
  }
  .dictation-stages {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-wrap: wrap;
  }
  .dictation-stages :global(.quiet-chip) {
    min-width: 0;
    font-size: 14px;
    padding: 9px 10px;
    gap: 6px;
  }
  .chip-name {
    max-width: 150px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keycaps {
    display: flex;
    gap: 3px;
  }
  kbd {
    font-size: 12px;
    padding: 1px 3px;
  }
  .stage-arrow,
  .stage-dot {
    color: var(--soft-ink);
  }
  .health-dot {
    width: 7px;
    height: 7px;
    flex: 0 0 7px;
    background: var(--amber, #99732e);
    border-radius: 50%;
  }
</style>
