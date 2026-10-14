<script lang="ts">
  import {
    checking,
    downloadPhaseLabel,
    formatBytes,
    installed,
    installing,
    type ModelEntry,
  } from "../lib/model-library";
  let {
    model,
    pending,
    retryDisabled,
    oncancel,
    onretry,
    idleLabel = "Cancelled",
    compact = false,
    errorShownElsewhere = false,
  }: {
    model: ModelEntry | undefined;
    pending: boolean;
    retryDisabled: boolean;
    oncancel: (model: ModelEntry) => Promise<void>;
    onretry: (model: ModelEntry) => Promise<void>;
    idleLabel?: string;
    /** A tighter row without the byte counts, for the setup tour. */
    compact?: boolean;
    /** The profile card above already prints this error beside its action. */
    errorShownElsewhere?: boolean;
  } = $props();
  const phase = $derived(
    !model
      ? ""
      : installed(model)
        ? "Installed"
        : ({
            queued: "Queued",
            checking: "Checking…",
            downloading: downloadPhaseLabel(model),
            verifying: "Verifying",
            installing: "Installing",
            cancelling: "Cancelling",
            canceling: "Cancelling",
            failed: "Failed",
            cancelled: "Cancelled",
            canceled: "Cancelled",
            available: idleLabel,
          }[model.state] ?? model.state),
  );
</script>

{#if model}
  <section
    class="download"
    class:compact
    aria-label="Model download"
    data-model-id={model.id}
  >
    <div class="download-copy">
      <span>{model.name}</span><span
        class="phase"
        data-text-surface="flat"
        role="status"
        >{phase}{#if installing(model) && model.totalBytes > 0}
          {" "}· {Math.min(
            100,
            Math.floor((model.downloadedBytes / model.totalBytes) * 100),
          )}%{/if}</span
      >
    </div>
    {#if installing(model)}
      <!-- Compact rows leave Cancel to the Models page. -->
      {#if !compact}<button
          class="button quiet"
          disabled={pending ||
            ["cancelling", "canceling"].includes(model.state)}
          onclick={() => model && oncancel(model)}>Cancel</button
        >{/if}
      <progress
        aria-label={`Installation progress for ${model.name}`}
        max={model.totalBytes || 1}
        value={model.totalBytes > 0
          ? Math.min(model.downloadedBytes, model.totalBytes)
          : undefined}
      ></progress>
      <span class="bytes" class:sr-only={compact} data-text-surface="flat"
        >{formatBytes(model.downloadedBytes)} / {model.totalBytes
          ? formatBytes(model.totalBytes)
          : "size unknown"}</span
      >
    {:else if !installed(model) && !checking(model) && model.installable}
      <button
        class="button"
        disabled={retryDisabled}
        onclick={() => model && onretry(model)}>Retry</button
      >
    {/if}
    {#if model.error && !errorShownElsewhere}<p
        role="alert"
        data-text-surface="flat"
      >
        {model.error}
      </p>{/if}
  </section>
{/if}

<style>
  .download {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 9px 18px;
    margin-top: 27px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
    font-size: 1rem;
  }
  .download-copy {
    display: flex;
    flex-wrap: wrap;
    gap: 0 18px;
    overflow-wrap: anywhere;
  }
  .phase,
  .bytes,
  p {
    color: var(--soft-ink);
  }
  .button {
    font-size: 1rem;
  }
  progress {
    grid-column: 1 / -1;
    width: 100%;
    height: 9px;
    accent-color: var(--ink);
    background: var(--sheet);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    border: 1px solid var(--line);
    border-radius: 5px;
    appearance: none;
  }
  progress::-webkit-progress-bar {
    background: var(--sheet);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    border-radius: 5px;
  }
  progress::-webkit-progress-value {
    background: var(--ink);
    border-radius: 5px;
  }
  progress::-moz-progress-bar {
    background: var(--ink);
    border-radius: 5px;
  }
  .bytes,
  p {
    grid-column: 1 / -1;
    margin: 0;
  }
  .compact {
    gap: 6px 12px;
    margin-top: 0;
    padding: 9px 0;
    line-height: 1.35;
  }
  .compact .download-copy {
    gap: 0 12px;
  }
  .compact .button {
    min-height: 35px;
    padding: 6px 11px;
    font-size: 15px;
  }
  .compact progress {
    height: 6px;
  }
</style>
