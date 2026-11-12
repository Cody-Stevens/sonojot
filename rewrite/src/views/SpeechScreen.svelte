<script lang="ts">
  import { onMount } from "svelte";
  import RecommendedSetup from "../components/quiet/RecommendedSetup.svelte";
  import SpeechChart from "../components/quiet/SpeechChart.svelte";
  import Sheet from "../components/quiet/Sheet.svelte";
  import Toggle from "../components/Toggle.svelte";
  import ModelDownload from "../components/ModelDownload.svelte";
  import { app } from "../lib/app.svelte";
  import { ai } from "../lib/ai.svelte";
  import { library } from "../lib/model-library.svelte";
  import { profiles } from "../lib/setup-profiles.svelte";
  import { profileChanges } from "../lib/setup-profiles";
  import { speechChoice } from "../lib/speech-choice.svelte";
  import {
    speechChoices,
    speechScore,
    speechMeasurement,
    waitText,
    memoryObservation,
    secondaryMemory,
    gpuModel,
    SONOJOT_PARAKEET,
  } from "../lib/speech-chart";
  import {
    checking,
    installed,
    installing,
    modelUnavailableReason,
    formatBytes,
    GRANITE_SPEECH_MODEL,
    type ModelEntry,
  } from "../lib/model-library";
  import { openRouter } from "../lib/openrouter.svelte";
  import { call, chooseModelsFolder } from "../lib/api";
  import {
    LIVE_PREVIEW_MODEL,
    LIVE_PREVIEW_SUMMARY,
  } from "../lib/live-preview";
  import {
    systemAudioState,
    systemAudioFixable,
    meetingSourceWarning,
  } from "../lib/meeting-source";
  import { fixSystemAudio, watchSystemAudio } from "../lib/system-audio.svelte";
  let { meeting = false }: { meeting?: boolean } = $props();
  let all = $state(false);
  let force = $state<ModelEntry | null>(null);
  let details = $state(false);
  let reset = $state(false);
  let diskDismissed = $state("");
  const settings = $derived(app.snapshot.settings);
  const models = $derived(library.data?.models ?? []);
  const hardware = $derived(profiles.hardware);
  const mac = $derived(hardware?.hardware.appleSilicon ?? false);
  const hardwareText = $derived(
    hardware
      ? [
          hardware.hardware.appleSilicon
            ? "Apple Silicon"
            : hardware.hardware.gpuName || "CPU speech",
          !mac && hardware.hardware.totalVramBytes
            ? `${(hardware.hardware.totalVramBytes / 1e9).toFixed(1)} GB graphics memory`
            : "",
          hardware.hardware.totalRamBytes
            ? `${(hardware.hardware.totalRamBytes / 1e9).toFixed(1)} GB ${mac ? "unified memory" : "RAM"}`
            : "Memory unknown",
        ]
          .filter(Boolean)
          .join(" · ")
      : "",
  );
  const recommended = $derived(
    hardware?.recommendations.balanced.asrModel ?? "",
  );
  const frontier = $derived(
    meeting
      ? (hardware?.frontier.meetings ?? [])
      : (hardware?.frontier.dictation ?? []),
  );
  const current = $derived(
    meeting
      ? settings.meetingAsrModel ||
          (ai.data?.connected && ai.data.speechModel
            ? "openrouter"
            : settings.asrModel)
      : settings.asrModel,
  );
  const rows = $derived(
    speechChoices(
      models,
      frontier,
      current,
      hardware?.frontier.otherLanguages ?? "",
      all,
    ).sort(
      (a, b) => Number(b.id === recommended) - Number(a.id === recommended),
    ),
  );
  const granite = $derived(
    models.find((model) => model.id === GRANITE_SPEECH_MODEL),
  );
  const dictation = $derived(
    models.find((model) => model.id === settings.asrModel),
  );
  const chartModels = $derived(
    meeting
      ? [dictation, granite].filter(
          (model): model is ModelEntry =>
            !!model && !modelUnavailableReason(model),
        )
      : rows,
  );
  const helper = $derived(
    models.find(
      (model) =>
        model.id === (meeting ? "nemotron-3-diarization" : LIVE_PREVIEW_MODEL),
    ),
  );
  const locked = $derived(speechChoice.locked);
  const changes = $derived(
    hardware
      ? profileChanges(
          settings,
          hardware.recommendations.balanced,
          ai.data,
          app.snapshot.localProofreaderAvailable,
        )
      : [],
  );
  const download = $derived(
    models.find(
      (model) => model.id === speechChoice.pending || installing(model),
    ),
  );
  const system = $derived(
    systemAudioState(
      app.snapshot.capabilities.systemAudioStatus,
      app.snapshot.capabilities.platform,
    ),
  );
  const diskFailure = $derived(
    speechChoice.failure &&
      /disk space|disk full|not enough.*space/i.test(
        speechChoice.failure.message,
      ) &&
      speechChoice.failure.message !== diskDismissed
      ? speechChoice.failure
      : null,
  );
  onMount(() => {
    const release = library.acquire();
    void profiles.check();
    void ai.refresh();
    const stop = meeting ? watchSystemAudio() : () => {};
    return () => {
      release();
      stop();
    };
  });
  async function changeFolder() {
    await app.perform("models-location", async () => {
      const path = await chooseModelsFolder();
      if (!path) return;
      await call("locate_models", { path });
      app.apply(await call("get_snapshot"));
      await library.refresh();
      await profiles.check();
    });
  }
  function choose(model: ModelEntry) {
    if (locked || model.id === current) return;
    if (
      !frontier.includes(model.id) &&
      model.id !== hardware?.frontier.otherLanguages
    )
      force = model;
    else void speechChoice.use(model, meeting ? "meeting" : "dictation");
  }
  function caption(model: ModelEntry) {
    // Before the recommended captions, which describe Qwen and stock Parakeet.
    if (model.id === SONOJOT_PARAKEET)
      return "English only · tuned by SonoJot for dictation · leaves the GPU free.";
    if (model.id === recommended)
      return mac
        ? "The same measured word errors as Q8, with less memory and a shorter wait."
        : gpuModel(model)
          ? "Fewest measured TED-12 word errors among the recommended choices."
          : "Quick English transcription; leaves the graphics card free.";
    if (model.id.startsWith("parakeet"))
      return "English only · leaves the GPU free.";
    if (model.id.startsWith("moonshine"))
      return "Smaller memory footprint; more measured word errors.";
    return `Wait after speech: ${waitText(model.id)}.`;
  }
  async function source(value: "both" | "microphone" | "system") {
    if (locked) return;
    await app.saveSettings({ meetingSource: value });
    if (
      value !== "microphone" &&
      systemAudioFixable(app.snapshot.capabilities.systemAudioStatus)
    )
      await app.perform("system-audio", fixSystemAudio);
  }
</script>

<div class="speech-screen">
  {#if hardware}<p class="hardware" data-text-surface="flat">
      {hardwareText}
    </p>{:else}<p>{profiles.hardwareError || "Reading this computer…"}</p>
    <button
      class="button small"
      disabled={profiles.checking}
      onclick={() => profiles.check()}>Try again</button
    >{/if}
  {#if meeting}<div class="stock paper-card source-row">
      <span>Records</span>
      <div class="segments" role="group" aria-label="Meeting sources">
        {#each [{ id: "both", label: "Both" }, { id: "microphone", label: "You" }, { id: "system", label: "Others" }] as item}<button
            aria-pressed={settings.meetingSource === item.id}
            disabled={locked}
            onclick={() => source(item.id as "both" | "microphone" | "system")}
            >{item.label}</button
          >{/each}
      </div>
      <small
        >{meetingSourceWarning(settings.meetingSource) ||
          "You and the people you hear"}</small
      >
    </div>
    {#if settings.meetingSource !== "microphone" && system && app.snapshot.capabilities.systemAudioStatus !== "ready"}<p
        class="permission"
      >
        {system.label} · {system.detail}{#if system.action}<button
            class="button small"
            disabled={locked}
            onclick={() => app.perform("system-audio", fixSystemAudio)}
            >{system.action}</button
          >{/if}
      </p>{/if}
  {/if}
  <div class="subheading">
    <span
      >{meeting
        ? "Tested on AMI room microphones"
        : mac
          ? "Tested on TED holdout · through the Mac app"
          : "Tested on 12 TED talks"}</span
    >{#if !meeting}<button
        class="text-action"
        aria-expanded={all}
        onclick={() => (all = !all)}
        >{all ? "Best here only" : "All models"}</button
      >{/if}
  </div>
  <div class="speech-layout">
    <div>
      <SpeechChart
        models={chartModels}
        {frontier}
        recommended={meeting ? settings.asrModel : recommended}
        selected={current}
        {meeting}
      /><button
        class="text-action measurements"
        onclick={() => (details = true)}>Measurements and sources</button
      >
    </div>
    <div class="model-list" aria-label="Speech models">
      {#if meeting}
        <button
          class="stock paper-card model-card"
          class:chosen={current === settings.asrModel &&
            !settings.meetingAsrModel}
          aria-pressed={current === settings.asrModel &&
            !settings.meetingAsrModel}
          disabled={locked}
          onclick={() => speechChoice.sameAsDictation()}
          ><span class="choice-dot"></span><span class="model-copy"
            ><strong data-text-surface="emboss">Same as dictation</strong><small
              class="stamp">Recommended</small
            ><small
              >{dictation?.name ?? settings.asrModel} · nothing extra to download</small
            ></span
          ><span>{current === settings.asrModel ? "✓" : "Use"}</span></button
        >
        <button
          class="stock paper-card model-card"
          class:chosen={current === GRANITE_SPEECH_MODEL}
          aria-pressed={current === GRANITE_SPEECH_MODEL}
          disabled={locked ||
            !granite ||
            !!modelUnavailableReason(granite) ||
            checking(granite)}
          onclick={() => granite && choose(granite)}
          ><span class="choice-dot"></span><span class="model-copy"
            ><strong data-text-surface="emboss">Granite Speech 4.1</strong
            ><small class="stamp">Meetings only</small><small
              >{!granite
                ? "Not available on this computer"
                : modelUnavailableReason(granite) ||
                  "Fewer errors on AMI room microphones; personal speech may favor Qwen."}</small
            >{#if granite}<small>{secondaryMemory(granite)}</small>{/if}
            ></span
          ><span class="cost"
            >{granite && !modelUnavailableReason(granite)
              ? installed(granite)
                ? current === granite.id
                  ? "✓ In use"
                  : "Use"
                : `↓ ${formatBytes(granite.diskBytes)}`
              : "Unavailable"}</span
          ></button
        >
        <button
          class="stock paper-card model-card"
          class:chosen={current === "openrouter"}
          aria-pressed={current === "openrouter"}
          disabled={locked}
          onclick={() => openRouter.open("meeting")}
          ><span class="choice-dot"></span><span class="model-copy"
            ><strong data-text-surface="emboss">Online · OpenRouter</strong
            ><small
              >{ai.data?.speechModel || "Choose an online speech model"}</small
            ><small
              >Audio leaves this computer · errors and wait not measured</small
            ></span
          ><span>›</span></button
        >
      {:else}
        {#each rows as model (model.id)}{@const edge =
            frontier.includes(model.id) ||
            model.id === hardware?.frontier.otherLanguages}
          <button
            class="stock paper-card model-card"
            class:chosen={model.id === current}
            class:dimmed={!edge}
            aria-pressed={model.id === current}
            disabled={locked ||
              !!modelUnavailableReason(model) ||
              checking(model)}
            onclick={() => choose(model)}
            data-model-id={model.id}
          >
            <span class="choice-dot"></span><span class="model-copy"
              ><strong data-text-surface="emboss">{model.name}</strong><span
                class="tags"
                >{#if model.id === recommended}<small class="stamp"
                    >Recommended</small
                  >{/if}{#if model.id === hardware?.frontier.otherLanguages && !frontier.includes(model.id)}<small
                    class="stamp">Other languages</small
                  >{/if}{#if current === model.id && current !== recommended}<small
                    class="stamp custom">Custom</small
                  >{/if}{#if !edge}<small class="stamp custom"
                    >Not recommended here</small
                  >{/if}</span
              ><small>{modelUnavailableReason(model) || caption(model)}</small
              ><small>{secondaryMemory(model)}</small
              >{#if !speechScore(model.id, mac, false)}<small
                  >Word errors not measured</small
                >{/if}{#if speechMeasurement(model.id)?.ram?.measuredPeak == null}<small
                  >{memoryObservation(model, "ram")}</small
                >{/if}<small class="sr-only">
                {speechScore(model.id, mac, false)?.value ?? "Unmeasured"} word errors;
                wait {waitText(model.id)}
              </small></span
            ><span class="cost"
              >{checking(model)
                ? "Checking…"
                : installing(model)
                  ? "Downloading"
                  : installed(model)
                    ? model.id === current
                      ? "✓ In use"
                      : "Use"
                    : model.installable
                      ? `↓ ${formatBytes(model.diskBytes)}`
                      : "Unavailable"}</span
            >
          </button>
        {/each}
        {#if current !== recommended && recommended}<button
            class="text-action back-recommended"
            disabled={locked}
            onclick={() => {
              const model = models.find((m) => m.id === recommended);
              if (model) void speechChoice.use(model, "dictation");
            }}
            >Back to {models.find((m) => m.id === recommended)?.name ??
              "recommended speech"}</button
          >{/if}
      {/if}
      {#if download && (installing(download) || checking(download))}<p
          class="field-help"
        >
          {speechChoice.pending === download.id &&
          speechChoice.pendingRole !== "helper"
            ? "Downloads here; switches when ready."
            : "Downloading model files."}
        </p>
        <ModelDownload
          model={download}
          pending={app.busy("model-cancel")}
          retryDisabled={locked}
          oncancel={(model) => speechChoice.cancel(model.id)}
          onretry={(model) =>
            speechChoice.use(model, meeting ? "meeting" : "dictation")}
        />{/if}
      {#if speechChoice.failure}<p role="alert">
          {speechChoice.failure.message}
        </p>
        <button
          class="button small"
          disabled={locked}
          onclick={() => {
            const m = models.find(
              (model) => model.id === speechChoice.failure?.id,
            );
            if (m && speechChoice.failure)
              void speechChoice.use(m, speechChoice.failure.role);
          }}>Retry</button
        >{/if}
      <div class="stock paper-card helper-row">
        <span
          >{meeting ? "Speaker names" : "Live preview"}<small
            >{meeting
              ? "Labels voices in the transcript"
              : LIVE_PREVIEW_SUMMARY}</small
          ></span
        >{#if helper && installed(helper)}<Toggle
            label={meeting ? "Speaker names" : "Live preview"}
            checked={meeting
              ? settings.meetingSpeakerLabels
              : settings.livePreview}
            disabled={locked}
            onchange={(value) =>
              app.saveSettings(
                meeting
                  ? { meetingSpeakerLabels: value }
                  : { livePreview: value },
              )}
          />{:else if helper && !modelUnavailableReason(helper)}<button
            class="button small"
            disabled={locked || checking(helper)}
            onclick={() => speechChoice.use(helper, "helper")}
            >{checking(helper)
              ? "Checking…"
              : `Install · ${formatBytes(helper.diskBytes)}`}</button
          >{:else}<small>Not available on this computer</small>{/if}
      </div>
    </div>
  </div>
  {#if changes.length}<div class="reset-row">
      <span
        >Custom choices in {changes
          .map(
            (c) =>
              ({
                asr: "dictation speech",
                cleanup: "Cleanup",
                notes: "Notes",
                meetingSpeech: "meeting speech",
              })[c.role],
          )
          .join(", ")}</span
      ><button
        class="text-action"
        disabled={profiles.applyLocked || !!speechChoice.pending}
        onclick={() => (reset = true)}>Back to recommended setup…</button
      >
    </div>{/if}
  {#if profiles.failure}<p role="alert">{profiles.failure.message}</p>{/if}
  {#if profiles.busy}<p role="status">Preparing the recommended setup…</p>{/if}
</div>

{#if force}<Sheet title={force.name} onclose={() => (force = null)}
    ><p class="warning">Not recommended here</p>
    <p>
      This model is outside the tested choices for this computer. It may use
      more memory, take longer or have unmeasured accuracy.
    </p>
    <p>{memoryObservation(force, "ram")}</p>
    <p>{memoryObservation(force, "vram")}</p>
    <p>Wait after speech: {waitText(force.id)}.</p>
    <div class="sheet-actions">
      <button class="button" onclick={() => (force = null)}>Cancel</button
      ><button
        class="button"
        disabled={locked}
        onclick={() => {
          const model = force;
          force = null;
          if (model)
            void speechChoice.use(model, meeting ? "meeting" : "dictation");
        }}>Use anyway</button
      >
    </div></Sheet
  >{/if}
{#if diskFailure}<Sheet
    title="Not enough disk space"
    onclose={() => (diskDismissed = diskFailure?.message ?? "")}
    ><p>{diskFailure.message}</p>
    <p>
      Download: {formatBytes(
        models.find((model) => model.id === diskFailure.id)?.diskBytes ?? null,
      )}. Choose a folder with more free space, then retry.
    </p>
    <div class="sheet-actions">
      <button
        class="button"
        onclick={() => (diskDismissed = diskFailure.message)}>Cancel</button
      ><button
        class="button"
        onclick={() => {
          diskDismissed = diskFailure.message;
          void changeFolder();
        }}>Change folder</button
      >
    </div></Sheet
  >{/if}
{#if reset}<RecommendedSetup onclose={() => (reset = false)} />{/if}
{#if details}<Sheet
    title="Measurements and sources"
    onclose={() => (details = false)}
    ><p>
      {meeting
        ? "AMI single distant microphone recordings"
        : mac
          ? "TED holdout through the Mac app"
          : "TED-12, normalized English word errors"}. Scores apply to these
      exact packages and test paths. Different corpora are not comparable.
    </p>
    <p>
      GB here means 1,000,000,000 bytes. GPU models also need system RAM. Apple
      Silicon has one shared pool. Planning ranges are estimates; dots use
      measured peaks only.
    </p>
    {#each chartModels as model}<section class="measurement">
        <strong data-text-surface="emboss">{model.name}</strong>
        <p>
          {speechScore(model.id, mac, meeting)?.value ??
            "Not measured"}{speechScore(model.id, mac, meeting)
            ? "% word errors"
            : " word errors"} · wait {waitText(model.id)}
        </p>
        <p>{memoryObservation(model, "ram")} system RAM</p>
        <p>{memoryObservation(model, "vram")} dedicated GPU memory</p>
        <small
          >{speechMeasurement(model.id)?.wait?.basis ??
            "Wait after speech not measured"}</small
        ><small
          >{[
            speechScore(model.id, mac, meeting)?.source,
            speechMeasurement(model.id)?.ram?.source,
            speechMeasurement(model.id)?.vram?.source,
            speechMeasurement(model.id)?.wait?.source,
          ]
            .flat()
            .filter(Boolean)
            .filter((value, index, all) => all.indexOf(value) === index)
            .join(" · ") || "No SonoJot measurement available"}</small
        >
      </section>{/each}</Sheet
  >{/if}

<style>
  .speech-screen {
    max-width: 1020px;
    margin: 0 auto;
  }
  .hardware {
    font-size: 14px;
    color: var(--soft-ink);
    margin: 0 0 16px;
  }
  .subheading {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 14px;
    color: var(--soft-ink);
    margin-bottom: 12px;
  }
  .speech-layout {
    display: grid;
    grid-template-columns: minmax(350px, 1.15fr) minmax(300px, 1fr);
    gap: 20px;
    align-items: start;
  }
  .model-list {
    display: grid;
    gap: 12px;
  }
  .model-card {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 13px;
    width: 100%;
    min-height: 100px;
    padding: 16px;
    border: 1px solid transparent;
    border-radius: 14px;
    text-align: left;
    background-color: var(--card);
    color: var(--ink);
    box-shadow: var(--raise-sm);
  }
  .model-card.chosen {
    border-color: var(--dye);
    outline: 1px solid var(--dye);
  }
  .model-card.chosen .choice-dot {
    background: var(--dye);
  }
  .model-card.dimmed {
    opacity: 0.65;
  }
  .model-copy {
    display: grid;
    gap: 5px;
    flex: 1;
  }
  .model-copy strong {
    font-size: 16px;
    font-weight: 600;
  }
  .model-copy small {
    font-size: 13px;
    line-height: 1.4;
    color: var(--soft-ink);
  }
  .tags {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
  }
  .tags:empty {
    display: none;
  }
  .model-copy .stamp {
    display: inline-block;
    justify-self: start;
    border: 1px solid var(--line);
    padding: 0 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--dye);
  }
  .model-copy .stamp.custom {
    color: var(--soft-ink);
  }
  .cost {
    font-size: 12px;
    max-width: 72px;
    flex-shrink: 0;
    text-align: right;
  }
  .helper-row {
    border-radius: 16px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 18px 22px;
    margin-top: 2px;
    gap: 18px;
  }
  .helper-row small {
    display: block;
    color: var(--soft-ink);
    font-size: 13px;
    margin-top: 3px;
  }
  .source-row {
    display: flex;
    gap: 20px;
    align-items: center;
    padding: 14px 20px;
    margin: 0 0 12px;
  }
  .source-row .segments {
    margin: 0;
    padding: 3px;
    gap: 3px;
  }
  .source-row .segments button {
    padding: 8px 15px;
  }
  .source-row small {
    color: var(--soft-ink);
    font-size: 13px;
  }
  .permission {
    font-size: 14px;
  }
  .permission button {
    margin-left: 12px;
  }
  .measurements {
    margin-top: 8px;
    font-size: 12px;
  }
  .back-recommended {
    justify-self: start;
    font-size: 14px;
  }
  .reset-row {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    margin-top: 18px;
    color: var(--soft-ink);
    font-size: 12px;
  }
  .sheet-actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
    margin-top: 22px;
  }
  .warning {
    color: var(--danger);
  }
  .measurement {
    border-top: 1px solid var(--line);
    padding: 16px 0;
  }
  .measurement p {
    margin: 4px 0;
    font-size: 14px;
  }
  .measurement small {
    display: block;
    overflow-wrap: anywhere;
    color: var(--soft-ink);
    font-size: 12px;
    margin-top: 5px;
  }
  @media (max-width: 1000px) {
    .speech-layout {
      grid-template-columns: 1fr;
    }
  }
</style>
