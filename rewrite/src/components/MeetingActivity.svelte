<script lang="ts">
  let {
    status,
    recording = false,
    announce = false,
    plain = false,
  }: {
    status: string;
    recording?: boolean;
    announce?: boolean;
    plain?: boolean;
  } = $props();

  const labels: Record<string, string> = {
    recorded: "Recorded",
    imported: "Imported",
    recording: "Recording",
    finalizing: "Saving recording",
    transcribing: "Transcribing",
    processing: "Processing",
    cleaning: "Cleaning transcript",
    processingNotes: "Writing notes",
    generatingTitle: "Naming meeting",
    importing: "Importing",
    ready: "Ready",
    transcribed: "Transcribed",
    processed: "Notes ready",
    complete: "Ready",
    completed: "Ready",
    error: "Needs attention",
    failed: "Needs attention",
    interrupted: "Transcription stopped",
    cancelled: "Cancelled",
    canceled: "Cancelled",
    generating_notes: "Writing notes",
    "generating-notes": "Writing notes",
  };
  const isRecording = $derived(recording || status === "recording");
  const active = $derived(
    !plain &&
      !isRecording &&
      [
        "finalizing",
        "transcribing",
        "processing",
        "cleaning",
        "processingNotes",
        "generatingTitle",
        "importing",
        "generating_notes",
        "generating-notes",
      ].includes(status),
  );
  const label = $derived(
    isRecording ? "Recording" : (labels[status] ?? status.replaceAll("_", " ")),
  );
</script>

<span
  class="status-label meeting-activity"
  class:active
  class:recording={isRecording}
  role={announce ? "status" : undefined}
  aria-live={announce ? "polite" : undefined}
  aria-atomic={announce ? true : undefined}
  title={label}
>
  {#if active}
    <svg
      class="writing-mark"
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.55"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M3 21h13" />
      <g class="writing-pen">
        <path
          d="m5 17 1-4L16 3a1.4 1.4 0 0 1 2 0l1 1a1.4 1.4 0 0 1 0 2L9 16l-4 1ZM14 5l3 3M6 13l3 3"
        />
      </g>
    </svg>
  {/if}
  <span data-text-surface={active ? "imprint" : "flat"}>{label}</span>
</span>

<style>
  .meeting-activity {
    display: inline-flex;
    align-items: center;
    flex: none;
    gap: 7px;
    vertical-align: middle;
    max-width: 100%;
  }
  .meeting-activity.active {
    padding: 4px 9px;
    border: 1px solid color-mix(in srgb, var(--dye) 35%, transparent);
    border-radius: 7px;
    background: color-mix(in srgb, var(--dye) 8%, transparent);
    color: var(--dye);
    font-size: 16px;
    font-weight: 600;
    line-height: 1.4;
  }
  .writing-pen {
    transform-origin: 6px 16px;
    animation: meeting-writing 1800ms ease-in-out infinite;
  }
  @keyframes meeting-writing {
    0%,
    100% {
      transform: translate(0, 0) rotate(-4deg);
    }
    18% {
      transform: translate(1px, -1px) rotate(1deg);
    }
    36% {
      transform: translate(2px, 0) rotate(-3deg);
    }
    54%,
    68% {
      transform: translate(3px, -1px) rotate(1deg);
    }
    84% {
      transform: translate(0, -2px) rotate(-4deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .writing-pen {
      animation: none;
      transform: none;
    }
  }
</style>
