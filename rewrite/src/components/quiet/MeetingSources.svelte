<script lang="ts">
  import Sheet from "./Sheet.svelte";
  import Toggle from "../Toggle.svelte";
  import ModelDownload from "../ModelDownload.svelte";
  import { call } from "../../lib/api";
  import { library } from "../../lib/model-library.svelte";
  import { app } from "../../lib/app.svelte";
  import { installed, checking, formatBytes } from "../../lib/model-library";
  import { speakerEchoRemoval } from "../../lib/meeting-models";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  let {
    session,
    onclose,
    onspeech,
  }: { session: MeetingSession; onclose: () => void; onspeech: () => void } =
    $props();
  const locked = $derived(
    !app.native ||
      app.busy("settings") ||
      session.dictating ||
      app.snapshot.meetingRecording,
  );
  const echo = $derived(
    speakerEchoRemoval(
      app.snapshot.settings.meetingRemoveEcho,
      session.transcriptionModel,
    ),
  );
  const sources = [
    { id: "both", label: "Both", detail: "You and the people you hear" },
    { id: "microphone", label: "Only you", detail: "Microphone" },
    { id: "system", label: "Only others", detail: "System audio" },
  ] as const;
</script>

<Sheet title="Sources and speakers" {onclose}>
  {#if app.error}<p role="alert">{app.error}</p>{/if}
  {#each sources as source}<button
      class="quiet-option"
      aria-pressed={session.source === source.id}
      disabled={locked}
      onclick={() => session.changeSource(source.id)}
      ><span class="choice-dot" aria-hidden="true"></span><span
        class="option-copy">{source.label}<small>{source.detail}</small></span
      ></button
    >{/each}
  {#if app.snapshot.meetingRecording}<p class="field-help">
      Sources are fixed for this recording. Stop before changing them.
    </p>{/if}
  {#if session.sourceWarning}<p class="field-help">
      {session.sourceWarning}
    </p>{/if}
  {#if session.source !== "microphone" && session.systemState}<p
      class="field-help"
    >
      {session.systemState.label} · {session.systemState.detail}
    </p>
    {#if session.systemState.action}<button
        class="button"
        onclick={session.fixSystem}>{session.systemState.action}</button
      >{/if}{/if}
  {#if session.systemFixError}<p role="alert">{session.systemFixError}</p>{/if}
  {#if session.source === "both"}
    <div class="speaker-row">
      <span>Remove speaker echo</span><Toggle
        label="Remove speaker echo"
        checked={echo.on}
        disabled={!app.native || app.busy("settings")}
        onchange={(meetingRemoveEcho) => app.saveSettings({ meetingRemoveEcho })}
      />
    </div>
    <p class="field-help">
      For calls on speakers: your microphone also hears the others. This keeps
      their words out of your side of the transcript.{#if echo.automatic}
        {echo.on
          ? "On automatically for this speech model."
          : "Off automatically for this speech model, which could also remove words you repeat."}{/if}
    </p>
  {/if}
  {#if app.snapshot.capabilities.platform !== "linux" && session.speakerLabelsAvailable && session.speakerModel}
    <div class="speaker-row">
      <span>Label speakers</span>{#if installed(session.speakerModel)}<Toggle
          label="Label speakers"
          checked={app.snapshot.settings.meetingSpeakerLabels}
          disabled={locked}
          onchange={(meetingSpeakerLabels) =>
            app.saveSettings({ meetingSpeakerLabels })}
        />{:else}<button
          class="button"
          disabled={session.speakerInstallDisabled}
          onclick={session.installSpeakerLabels}
          >{checking(session.speakerModel)
            ? "Checking…"
            : session.speakerInstallPending
              ? "Installing…"
              : `Install · ${formatBytes(session.speakerModel.diskBytes)}`}</button
        >{/if}
    </div>
    {#if session.speakerInstallPending || checking(session.speakerModel) || ["failed", "cancelled", "canceled"].includes(session.speakerModel.state)}<ModelDownload
        model={session.speakerModel}
        pending={app.busy("model-install") || app.busy("model-cancel")}
        retryDisabled={session.speakerInstallDisabled}
        onretry={session.installSpeakerLabels}
        oncancel={async (model) => {
          await app.perform("model-cancel", async () =>
            library.apply(await call("cancel_model_install", { id: model.id })),
          );
        }}
      />{/if}
  {/if}
  <button
    class="quiet-option"
    disabled={app.snapshot.meetingRecording || session.dictating}
    onclick={() => {
      onclose();
      onspeech();
    }}>Speech model <span class="spacer"></span>›</button
  >
</Sheet>

<style>
  .speaker-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 20px 0;
  }
  .spacer {
    flex: 1;
  }
</style>
