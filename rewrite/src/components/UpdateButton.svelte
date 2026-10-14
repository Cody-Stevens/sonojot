<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../lib/app.svelte";
  import {
    updateActionDisabled,
    updateButtonLabel,
    updateTooltip,
  } from "../lib/updates";
  import type { UpdateStatus } from "../lib/protocol";

  // The setup tour pictures the button in its "available" state.
  let { preview = false }: { preview?: boolean } = $props();
  const uid = $props.id();
  const status = $derived<UpdateStatus>(
    preview
      ? { state: "available", version: "", notes: null, size: null }
      : app.snapshot.update.status,
  );
  const installing = $derived(!preview && app.snapshot.update.installing);
  const reason = $derived(app.updateRestartBlockedReason);
  const label = $derived(updateButtonLabel(status, installing));
  const description = $derived(updateTooltip(status, installing, reason));
  const disabled = $derived(
    preview ||
      updateActionDisabled(status, installing, reason, app.native, (key) =>
        app.busy(key),
      ),
  );
  const progress = $derived(
    status.state === "downloading" ? status.percent : 100,
  );
  let tip = $state<HTMLSpanElement>();

  async function activate(): Promise<void> {
    if (disabled) return;
    if (status.state === "available") await app.downloadUpdate();
    else if (status.state === "ready") await app.restartToUpdate();
    else if (status.state === "error") await app.checkForUpdates();
  }

  // Top-layer positioning keeps the tooltip outside the scrolling sidebar.
  function tooltip(button: HTMLButtonElement, _description: string) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    function hide() {
      clearTimeout(timer);
      tip?.hidePopover();
    }
    function show() {
      clearTimeout(timer);
      if (!tip) return;
      tip.showPopover();
      const anchor = button.getBoundingClientRect();
      const rect = tip.getBoundingClientRect();
      tip.style.left = `${Math.max(9, Math.min(anchor.left, window.innerWidth - rect.width - 9))}px`;
      const top =
        anchor.top > rect.height + 18
          ? anchor.top - rect.height - 9
          : anchor.bottom + 9;
      tip.style.top = `${Math.max(9, Math.min(top, window.innerHeight - rect.height - 9))}px`;
    }
    function leave() {
      timer = setTimeout(hide, 100);
    }
    function key(event: KeyboardEvent) {
      if (event.key === "Escape") hide();
    }
    button.addEventListener("mouseenter", show);
    button.addEventListener("mouseleave", leave);
    button.addEventListener("focus", show);
    button.addEventListener("blur", hide);
    button.addEventListener("keydown", key);
    const popup = tip;
    popup?.addEventListener("mouseenter", show);
    popup?.addEventListener("mouseleave", leave);
    window.addEventListener("resize", hide);
    document.addEventListener("scroll", hide, true);
    return {
      update() {
        // Measure after the new text and theme have reached the DOM.
        void tick().then(() => {
          if (!disposed && tip?.matches(":popover-open")) show();
        });
      },
      destroy() {
        disposed = true;
        hide();
        button.removeEventListener("mouseenter", show);
        button.removeEventListener("mouseleave", leave);
        button.removeEventListener("focus", show);
        button.removeEventListener("blur", hide);
        button.removeEventListener("keydown", key);
        popup?.removeEventListener("mouseenter", show);
        popup?.removeEventListener("mouseleave", leave);
        window.removeEventListener("resize", hide);
        document.removeEventListener("scroll", hide, true);
      },
    };
  }
</script>

{#if status.state !== "idle"}
  <div class="sidebar-update">
    <span
      bind:this={tip}
      id={uid}
      class="update-tooltip stock paper-popover"
      role="tooltip"
      popover="manual"
      data-text-surface="flat">{description}</span
    >
    <button
      type="button"
      class="button update-button"
      class:primary={status.state === "ready"}
      class:quiet={status.state !== "ready"}
      class:installing
      aria-label={label}
      aria-disabled={disabled}
      aria-describedby={uid}
      onclick={activate}
      use:tooltip={description}
    >
      {#if status.state === "available"}
        <span class="update-dot" aria-hidden="true"></span>
      {:else if status.state === "downloading" || status.state === "ready"}
        <svg
          class="progress-ring"
          class:indeterminate={progress === null}
          viewBox="0 0 18 18"
          aria-hidden="true"
        >
          <circle
            cx="9"
            cy="9"
            r="7"
            fill="none"
            stroke="currentColor"
            stroke-opacity=".25"
            stroke-width="2"
          />
          <circle
            cx="9"
            cy="9"
            r="7"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            pathLength="100"
            stroke-dasharray={`${progress ?? 25} 100`}
            transform="rotate(-90 9 9)"
          />
          {#if status.state === "ready"}<path
              d="m6 9 2 2 4-4"
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
            />{/if}
        </svg>
      {/if}
      <span role="status">{label}</span>
    </button>
  </div>
{/if}

<style>
  .sidebar-update {
    flex: none;
  }
  .update-button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1);
    border-radius: 10px;
    font-size: 0.78rem;
    line-height: 1.35;
    text-align: left;
    justify-content: flex-start;
    min-height: 2.5rem;
  }
  .update-button.quiet {
    background-color: var(--well);
  }
  .update-button[aria-disabled="true"] {
    cursor: default;
  }
  .installing {
    opacity: 0.7;
  }
  .update-dot {
    width: 9px;
    height: 9px;
    flex: none;
    margin-inline: 4px;
    border-radius: 50%;
    background: var(--dye);
    animation: update-pulse 1.8s ease-out infinite;
  }
  .progress-ring {
    width: 18px;
    height: 18px;
    flex: none;
  }
  .indeterminate {
    animation: update-spin 1.8s linear infinite;
  }
  @keyframes update-pulse {
    50% {
      opacity: 0.45;
    }
  }
  @keyframes update-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .update-dot,
    .indeterminate {
      animation: none;
    }
  }
  @media (max-width: 640px) {
    .sidebar-update {
      min-width: 9rem;
    }
  }
  .update-tooltip {
    position: fixed;
    inset: auto;
    margin: 0;
    width: max-content;
    max-width: min(300px, calc(100vw - 36px));
    max-height: calc(100dvh - 18px);
    overflow-y: auto;
    padding: 9px 12px;
    font-size: 0.85rem;
    line-height: 1.4;
    text-align: left;
    white-space: normal;
    overflow-wrap: anywhere;
  }
</style>
