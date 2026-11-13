<script lang="ts">
  import { onMount } from "svelte";
  import RecommendedSetup from "../components/quiet/RecommendedSetup.svelte";
  import CompareModels from "../components/quiet/CompareModels.svelte";
  import InfoTip from "../components/quiet/InfoTip.svelte";
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
    waitText,
    memoryObservation,
    memoryText,
    speechLine,
    speechDetail,
    speechSources,
    languageText,
    speechViews,
    viewName,
    viewScore,
    viewTestShort,
    otherRunScore,
    meetingPathScore,
  } from "../lib/speech-chart";
  import {
    checking,
    discoverable,
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
  let compare = $state(false);
  let reset = $state(false);
  let diskDismissed = $state("");
  const settings = $derived(app.snapshot.settings);
  const models = $derived(library.data?.models ?? []);
  const hardware = $derived(profiles.hardware);
  const mac = $derived(hardware?.hardware.appleSilicon ?? false);
  // Shown in Compare models, where it explains which models can run here.
  const hardwareText = $derived(
    hardware
      ? [
          hardware.hardware.appleSilicon
            ? "Apple Silicon"
            : hardware.hardware.gpuName || "no supported graphics card",
          !mac && hardware.hardware.totalVramBytes
            ? `${(hardware.hardware.totalVramBytes / 1e9).toFixed(1)} GB graphics memory`
            : "",
          hardware.hardware.totalRamBytes
            ? `${(hardware.hardware.totalRamBytes / 1e9).toFixed(1)} GB ${mac ? "unified memory" : "RAM"}`
            : "memory unknown",
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
  // Every speech model this computer can run, for Compare models.
  const runnable = $derived(
    models.filter(
      (model) =>
        model.kind === "asr" &&
        model.roles.includes("asr") &&
        discoverable(model) &&
        !modelUnavailableReason(model),
    ),
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
  function status(model: ModelEntry): string {
    return checking(model)
      ? "Checking…"
      : installing(model)
        ? "Downloading"
        : installed(model)
          ? model.id === current
            ? "✓ In use"
            : "Use"
          : model.installable
            ? `↓ ${formatBytes(model.diskBytes)}`
            : "Unavailable";
  }
  function errorsText(model: ModelEntry, meetingScore: boolean): string {
    const score = speechScore(model.id, mac, meetingScore);
    if (!score) return "Not measured on this kind of computer";
    return `${score.value} per 100 words on ${
      meetingScore
        ? "AMI meeting room recordings"
        : mac
          ? "a TED holdout through the Mac app"
          : "12 TED talks"
    }`;
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

{#snippet details(model: ModelEntry, meetingScore: boolean)}
  <p>{speechDetail(model)}</p>
  <dl>
    <dt>Languages</dt>
    <dd>{languageText(model)}</dd>
    {#if meetingScore}<dt>Word errors</dt>
      <dd>{errorsText(model, true)}</dd>{:else}{#each speechViews(mac) as view}{@const score =
          viewScore(model.id, mac, view)}{@const other = otherRunScore(
          model.id,
          mac,
          view,
        )}<dt>{viewName(view)}</dt>
        <dd>
          {score
            ? `${score.value} per 100 words (${viewTestShort(view, mac)})`
            : other
              ? `${other.value} per 100 words (12 TED talks, another run)`
              : "Not measured"}
        </dd>{/each}{/if}
    {#if meetingPathScore(model.id, mac)}<dt>Meetings</dt>
      <dd>
        {meetingPathScore(model.id, mac)?.value} per 100 words (AMI, NOTSOFAR, DiPCo,
        pooled)
      </dd>{/if}
    <dt>Wait</dt>
    <dd>
      {waitText(model.id, mac) === "not measured"
        ? "Not measured on this kind of computer"
        : `${waitText(model.id, mac)} after you stop`}
    </dd>
    <dt>Memory</dt>
    <dd>{memoryText(model)}</dd>
    <dt>Download</dt>
    <dd>{formatBytes(model.diskBytes)}</dd>
  </dl>
  {#if speechSources(model.id, mac).length}<small
      >Source: {speechSources(model.id, mac).join(" · ")}</small
    >{/if}
{/snippet}

<div class="speech-screen">
  {#if !hardware}<p>{profiles.hardwareError || "Reading this computer…"}</p>
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
  <div class="model-list" role="group" aria-label="Speech models">
    {#if meeting}
      {@const same =
        current === settings.asrModel && !settings.meetingAsrModel}
      <div class="stock paper-card model-card" class:chosen={same}>
        <button
          class="card-choose"
          aria-pressed={same}
          disabled={locked}
          onclick={() => speechChoice.sameAsDictation()}
          ><span class="choice-dot"></span><span class="model-copy"
            ><span class="title-line"
              ><strong data-text-surface="emboss">Same as dictation</strong
              ><small class="stamp">Recommended</small></span
            ><small>{dictation?.name ?? settings.asrModel}</small></span
          ><span class="cost">{same ? "✓ In use" : "Use"}</span></button
        >{#if dictation}<InfoTip label="About Same as dictation"
            ><p>
              Meetings use your dictation model, so there is nothing extra to
              download.
            </p>
            {@render details(dictation, true)}</InfoTip
          >{/if}
      </div>
      <div
        class="stock paper-card model-card"
        class:chosen={current === GRANITE_SPEECH_MODEL}
        data-model-id={GRANITE_SPEECH_MODEL}
      >
        <button
          class="card-choose"
          aria-pressed={current === GRANITE_SPEECH_MODEL}
          disabled={locked ||
            !granite ||
            !!modelUnavailableReason(granite) ||
            checking(granite)}
          onclick={() => granite && choose(granite)}
          ><span class="choice-dot"></span><span class="model-copy"
            ><span class="title-line"
              ><strong data-text-surface="emboss">Granite Speech 4.1</strong
              ><small class="stamp">Meetings only</small></span
            ><small
              >{!granite
                ? "Not available on this computer"
                : modelUnavailableReason(granite) ||
                  "Fewer errors on meeting room microphones."}</small
            ></span
          ><span class="cost"
            >{granite && !modelUnavailableReason(granite)
              ? status(granite)
              : "Unavailable"}</span
          ></button
        >{#if granite}<InfoTip label="About Granite Speech 4.1"
            ><p>
              Measured on AMI meeting room recordings. Your own voice may still
              favor Qwen.
            </p>
            {@render details(granite, true)}</InfoTip
          >{/if}
      </div>
      <div
        class="stock paper-card model-card"
        class:chosen={current === "openrouter"}
      >
        <button
          class="card-choose"
          aria-pressed={current === "openrouter"}
          disabled={locked}
          onclick={() => openRouter.open("meeting")}
          ><span class="choice-dot"></span><span class="model-copy"
            ><strong data-text-surface="emboss">Online · OpenRouter</strong
            ><small>{ai.data?.speechModel || "Audio leaves this computer."}</small
            ></span
          ><span class="cost" aria-hidden="true">›</span></button
        ><InfoTip label="About online speech"
          ><p>
            Sends meeting audio to the online model you choose through
            OpenRouter. Word errors and wait are not measured.
          </p></InfoTip
        >
      </div>
    {:else}
      {#each rows as model (model.id)}{@const edge =
          frontier.includes(model.id) ||
          model.id === hardware?.frontier.otherLanguages}
        <div
          class="stock paper-card model-card"
          class:chosen={model.id === current}
          class:dimmed={!edge}
          data-model-id={model.id}
        >
          <button
            class="card-choose"
            aria-pressed={model.id === current}
            disabled={locked ||
              !!modelUnavailableReason(model) ||
              checking(model)}
            onclick={() => choose(model)}
          >
            <span class="choice-dot"></span><span class="model-copy"
              ><span class="title-line"
                ><strong data-text-surface="emboss">{model.name}</strong
                >{#if model.id === recommended}<small class="stamp"
                    >Recommended</small
                  >{/if}{#if current === model.id && current !== recommended}<small
                    class="stamp custom">Custom</small
                  >{/if}{#if !edge}<small class="stamp custom"
                    >Not recommended here</small
                  >{/if}</span
              ><small
                >{modelUnavailableReason(model) ||
                  speechLine(model, { mac })}</small
              ></span
            ><span class="cost">{status(model)}</span>
          </button>
          <InfoTip label="About {model.name}"
            >{@render details(model, false)}</InfoTip
          >
        </div>
      {/each}
    {/if}
  </div>
  {#if !meeting}<div class="list-actions">
      <button class="button small" onclick={() => (compare = true)}
        >Compare models</button
      ><button
        class="text-action"
        aria-expanded={all}
        onclick={() => (all = !all)}
        >{all ? "Best here only" : "All models"}</button
      >{#if current !== recommended && recommended}<button
          class="text-action back-recommended"
          disabled={locked}
          onclick={() => {
            const model = models.find((m) => m.id === recommended);
            if (model) void speechChoice.use(model, "dictation");
          }}
          >Back to {models.find((m) => m.id === recommended)?.name ??
            "recommended speech"}</button
        >{/if}
    </div>{/if}
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
        const m = models.find((model) => model.id === speechChoice.failure?.id);
        if (m && speechChoice.failure)
          void speechChoice.use(m, speechChoice.failure.role);
      }}>Retry</button
    >{/if}
  <div class="stock paper-card helper-row">
    <span class="helper-label"
      >{meeting ? "Speaker names" : "Live preview"}<InfoTip
        label={meeting ? "About Speaker names" : "About Live preview"}
        ><p>
          {meeting ? "Labels voices in the transcript." : LIVE_PREVIEW_SUMMARY}
        </p>
        {#if helper}<p>
            Uses {helper.name}, a {formatBytes(helper.diskBytes)} download.
          </p>{/if}</InfoTip
      ></span
    >{#if helper && installed(helper)}<Toggle
        label={meeting ? "Speaker names" : "Live preview"}
        checked={meeting ? settings.meetingSpeakerLabels : settings.livePreview}
        disabled={locked}
        onchange={(value) =>
          app.saveSettings(
            meeting ? { meetingSpeakerLabels: value } : { livePreview: value },
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
    <p>Wait after speech: {waitText(force.id, mac)}.</p>
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
{#if compare}<CompareModels
    models={runnable}
    {mac}
    {current}
    {recommended}
    {hardwareText}
    onclose={() => (compare = false)}
  />{/if}

<style>
  .speech-screen {
    max-width: 760px;
    margin: 0 auto;
  }
  .model-list {
    display: grid;
    gap: 12px;
  }
  .model-card {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 16px 0 0;
    border: 1px solid transparent;
    border-radius: 14px;
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
  .card-choose {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 13px;
    min-height: 76px;
    margin: 0;
    padding: 14px 6px 14px 16px;
    border: 0;
    border-radius: 14px;
    background: transparent;
    box-shadow: none;
    color: inherit;
    text-align: left;
    font: inherit;
  }
  /* The focus ring goes round the whole card, info button included. */
  .card-choose:focus-visible {
    outline: none;
  }
  .model-card:has(.card-choose:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 3px;
  }
  .model-copy {
    display: grid;
    gap: 4px;
    flex: 1;
    min-width: 0;
  }
  .title-line {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
  }
  .model-copy strong {
    font-size: 16px;
    font-weight: 600;
  }
  .model-copy small {
    font-size: 14px;
    line-height: 1.4;
    color: var(--soft-ink);
  }
  .model-copy .stamp {
    display: inline-block;
    border: 1px solid var(--line);
    padding: 0 4px;
    font-size: 10px;
    line-height: 1.5;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--dye);
  }
  .model-copy .stamp.custom {
    color: var(--soft-ink);
  }
  .cost {
    font-size: 13px;
    flex-shrink: 0;
    text-align: right;
    white-space: nowrap;
  }
  .list-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 22px;
    margin: 16px 0 0;
  }
  .list-actions .text-action {
    font-size: 14px;
  }
  .helper-row {
    border-radius: 16px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 16px 20px;
    margin-top: 22px;
    gap: 18px;
  }
  .helper-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .helper-row > small {
    color: var(--soft-ink);
    font-size: 13px;
  }
  .source-row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px 20px;
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
  .reset-row {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 8px 16px;
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
</style>
