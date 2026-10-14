<script lang="ts">
  // Self-contained setup section for recording the other side of calls. The
  // only hook in a host view is `bind:resolved`, which turns true once system
  // audio is ready, impossible on this OS, or the user deliberately chose Not now.
  import { onMount } from "svelte";
  import { app } from "../lib/app.svelte";
  import { errorMessage } from "../lib/protocol";
  import { systemAudioFixable, systemAudioState } from "../lib/meeting-source";
  import { fixSystemAudio, watchSystemAudio } from "../lib/system-audio.svelte";

  let { resolved = $bindable(false) }: { resolved?: boolean } = $props();
  const status = $derived(app.snapshot.capabilities.systemAudioStatus);
  const view = $derived(
    systemAudioState(status, app.snapshot.capabilities.platform),
  );
  const fixable = $derived(systemAudioFixable(status));
  let deferred = $state(false);
  let busy = $state(false);
  let error = $state("");
  let assisting = $state(false);
  // One line in place: the step must fit the smallest window without scrolling.
  const detail = $derived(
    fixable && assisting
      ? "Switch SonoJot on under System Audio Recording Only, or drag in the helper's icon."
      : fixable && deferred
        ? "For now, meetings record only your microphone."
        : view.detail,
  );
  let wasFixable = false;

  $effect(() => {
    resolved = !app.native || !fixable || deferred;
  });
  $effect(() => {
    // Granting it here means the user wants both sides recorded.
    if (fixable) wasFixable = true;
    else if (
      wasFixable &&
      status === "ready" &&
      app.snapshot.settings.meetingSource === "microphone"
    ) {
      wasFixable = false;
      void app.saveSettings({ meetingSource: "both" });
    }
  });
  onMount(() =>
    watchSystemAudio(1000, (permission) => {
      assisting = permission.assistantOpen && !permission.granted;
    }),
  );

  async function fix(): Promise<void> {
    busy = true;
    error = "";
    try {
      const permission = await fixSystemAudio();
      assisting = !!permission?.assistantOpen && !permission.granted;
    } catch (failure: unknown) {
      error = errorMessage(failure);
    } finally {
      busy = false;
    }
  }
</script>

<div class="system-audio-step">
  <div
    class="paper-well system-audio-status"
    class:granted={status === "ready"}
    role="status"
    aria-label="System audio permission"
  >
    <p data-text-surface="flat">
      <strong data-text-surface="emboss">System audio: {view.label}.</strong>
      {detail}
    </p>
    {#if fixable}<div class="system-audio-actions">
        <button
          class="button primary"
          disabled={busy || !app.native}
          onclick={fix}>{view.action}</button
        >
        {#if !deferred}<button
            class="button quiet"
            disabled={busy}
            onclick={() => {
              deferred = true;
            }}>Not now</button
          >{/if}
      </div>{/if}
  </div>
  {#if error}<p class="flow-error" role="alert">{error}</p>{/if}
</div>

<style>
  .system-audio-step {
    margin-top: 18px;
  }
  .system-audio-status {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 9px 12px;
  }
  .system-audio-status p {
    flex: 1;
    margin: 0;
    line-height: 1.45;
  }
  .system-audio-status strong {
    font-weight: 620;
  }
  .system-audio-status:not(.granted) strong {
    color: var(--danger);
  }
  .system-audio-actions {
    display: flex;
    flex: none;
    gap: 9px;
  }
  .flow-error {
    color: var(--danger);
  }
</style>
