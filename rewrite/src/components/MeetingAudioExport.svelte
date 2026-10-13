<script lang="ts">
  import { call, chooseAudioExport, native } from "../lib/api";
  import {
    audioExportErrorMessage,
    audioExportFilename,
    canExportAudio,
    shouldInvokeAudioExport,
  } from "../lib/meeting-audio-export";

  let {
    meetingId,
    title,
    status,
    hasAudio,
    disabled = false,
  }: {
    meetingId: string;
    title: string;
    status: string;
    hasAudio: boolean;
    disabled?: boolean;
  } = $props();

  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  const gate = $derived(
    canExportAudio({
      status,
      hasAudio,
      busy,
      native,
    }),
  );
  const effectiveDisabled = $derived(disabled || !meetingId || !gate.enabled);

  async function exportAudio(): Promise<void> {
    if (busy || !meetingId) return;
    error = "";
    notice = "";
    // Snapshot the target meeting before awaiting the native save dialog so
    // a selection change while the dialog is open cannot redirect the export.
    const snapId = meetingId;
    const snapTitle = title;
    const snapStatus = status;
    void snapStatus;
    let destination: string | null;
    try {
      destination = await chooseAudioExport(audioExportFilename(snapTitle));
    } catch (failure: unknown) {
      error = audioExportErrorMessage(failure);
      return;
    }
    // Cancel is a no-op: no invoke, no error, no notice.
    if (!shouldInvokeAudioExport(destination)) return;
    busy = true;
    try {
      await call("export_meeting_audio", { id: snapId, path: destination });
      notice = "Original recording saved.";
    } catch (failure: unknown) {
      error = audioExportErrorMessage(failure);
    } finally {
      busy = false;
    }
  }
</script>

<div class="audio-export">
  <button
    type="button"
    class="button"
    disabled={effectiveDisabled}
    onclick={exportAudio}
  >
    {busy ? "Saving…" : "Save recording (.wav)"}
  </button>
  {#if gate.reason && !busy}
    <p class="field-help" data-text-surface="flat">{gate.reason}</p>
  {:else}
    <p class="field-help" data-text-surface="flat">
      Saves a copy of the original recording. Available while transcription or
      notes are processing.
    </p>
  {/if}
  {#if error}
    <p class="field-help audio-error" role="alert" data-text-surface="flat">
      {error}
    </p>
  {/if}
  {#if notice}
    <p class="field-help" role="status" data-text-surface="flat">{notice}</p>
  {/if}
</div>

<style>
  .audio-export {
    margin-top: 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  .audio-error {
    color: var(--danger, #8c2f2f);
  }
</style>
