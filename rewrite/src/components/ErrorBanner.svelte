<script lang="ts">
  import { presentError, type ErrorAction } from "../lib/errors";
  import { scrollEdges } from "../lib/scroll-edges";
  let {
    error,
    onaction,
    ondismiss,
    canCopyText = false,
    compact = false,
    savedMeetingAudio = false,
  }: {
    error: unknown;
    onaction?: (action: ErrorAction) => void;
    ondismiss?: () => void;
    canCopyText?: boolean;
    compact?: boolean;
    savedMeetingAudio?: boolean;
  } = $props();
  const message = $derived(presentError(error, { savedMeetingAudio }));
</script>

<div class="error-banner" class:compact role="alert">
  <div class="error-heading">
    <strong>{message.title}</strong>
    {#if ondismiss}<button
        class="button quiet small"
        aria-label="Dismiss error"
        title="Dismiss error"
        data-help="Dismiss error"
        onclick={ondismiss}>Dismiss</button
      >{/if}
  </div>
  <p>{message.body}</p>
  {#if onaction}<div class="error-actions">
      {#if message.action.id !== "copy-text" || canCopyText}
        <button
          class="button small"
          onclick={() => onaction?.(message.action.id)}
          >{message.action.label}</button
        >
      {/if}
      {#if canCopyText && message.action.id !== "copy-text"}
        <button class="button small" onclick={() => onaction?.("copy-text")}
          >Copy available text</button
        >
      {/if}
    </div>{/if}
  <details data-text-surface="flat">
    <summary>Details</summary>
    <pre use:scrollEdges>{message.details}</pre>
  </details>
</div>

<style>
  .error-banner {
    border: 1px solid var(--danger, #963e35);
    border-radius: 12px;
    padding: 16px;
    margin-block: 12px;
    color: var(--text, inherit);
    background: var(--surface, transparent);
    overflow-wrap: anywhere;
  }
  .error-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .error-heading strong {
    font-weight: 650;
  }
  p {
    margin: 8px 0 12px;
  }
  .error-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 12px;
  }
  details {
    font-size: 0.875rem;
  }
  summary {
    cursor: pointer;
    width: fit-content;
  }
  summary:focus-visible {
    outline: 2px solid currentColor;
    outline-offset: 4px;
  }
  pre {
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 14rem;
    overflow: auto;
    margin: 8px 0 0;
    user-select: text;
  }
  .compact {
    padding: 10px 12px;
    font-size: 0.875rem;
  }
</style>
