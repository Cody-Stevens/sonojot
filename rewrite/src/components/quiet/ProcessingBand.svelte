<script lang="ts">
  import { app } from "../../lib/app.svelte";
  import { meetingStatus } from "../../lib/meeting-display";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  let { session }: { session: MeetingSession } = $props();
  const status = $derived(
    app.busy("meeting-clean")
      ? "cleaning"
      : (session.selected?.status ?? "ready"),
  );
</script>

<section class="stock paper-card processing-band" aria-label="Meeting progress">
  <div class="progress-head">
    <span class="progress-ink" aria-hidden="true"></span><strong role="status"
      >{meetingStatus[status] ?? status}</strong
    ><span class="spacer"
    ></span>{#if session.processing && status !== "finalizing"}<button
        class="button"
        disabled={app.busy("meeting-cancel")}
        onclick={session.cancel}
        >{status === "transcribing"
          ? "Stop transcribing"
          : "Stop processing"}</button
      >{/if}
  </div>
  <div class="progress-stages">
    <span>{status === "finalizing" ? "Saving audio…" : "✓ Audio saved"}</span
    ><span class:current={status === "transcribing"}>Transcript</span><span
      class:current={status === "processingNotes"}
      >{app.snapshot.settings.meetingNotesPreset === null &&
      status !== "processingNotes"
        ? "No automatic notes"
        : "Notes"}</span
    >
  </div>
  <p class="field-help">
    {status === "finalizing"
      ? "Finishing the recording. Audio actions are available when saving completes."
      : "Your recording stays saved. You can return to the meeting list while this finishes."}
  </p>
</section>

<style>
  .processing-band {
    padding: 22px 26px;
    border-radius: 14px;
    margin: 18px 0;
  }
  .progress-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .spacer {
    flex: 1;
  }
  .progress-ink {
    height: 9px;
    width: 9px;
    background: var(--dye);
    border-radius: 50%;
  }
  .progress-stages {
    display: flex;
    gap: 32px;
    margin: 22px 0 10px;
    color: var(--soft-ink);
    font-size: 15px;
  }
  .current {
    color: var(--dye);
    text-decoration: underline;
    text-underline-offset: 6px;
  }
  .field-help {
    margin-bottom: 0;
  }
</style>
