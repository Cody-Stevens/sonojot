<script lang="ts">
  import Icon from "./Icon.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { duration, type MeetingDetail } from "../lib/protocol";
  import {
    railTab,
    tabs,
    type Route,
    type RouteTarget,
  } from "../lib/route.svelte";
  import { updateButtonLabel, updateTooltip } from "../lib/updates";
  import { scrollEdges } from "../lib/scroll-edges";
  let {
    route,
    onnavigate = () => {},
    preview = false,
  }: {
    route: Route;
    onnavigate?: (target: RouteTarget) => void;
    preview?: boolean;
  } = $props();
  const activeTab = $derived(railTab(route));
  const recording = $derived(!preview && app.snapshot.meetingRecording);
  const activeId = $derived(recording ? app.snapshot.activeMeetingId : null);
  let meeting = $state<MeetingDetail | null>(null);
  let now = $state(Date.now());
  let readAt = $state(Date.now());
  $effect(() => {
    const id = activeId;
    meeting = null;
    if (!id || !app.native) return;
    let disposed = false;
    void call("get_meeting", { id })
      .then((value) => {
        if (!disposed) {
          meeting = value;
          readAt = Date.now();
          now = readAt;
        }
      })
      .catch(() => {
        /* Recording remains visible when its detail is unavailable. */
      });
    const timer = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  });
  const elapsed = $derived(
    meeting
      ? Math.max(
          meeting.durationSeconds,
          (now -
            (Number.isFinite(Date.parse(meeting.createdAt))
              ? Date.parse(meeting.createdAt)
              : readAt)) /
            1000,
        )
      : null,
  );
  const update = $derived(app.snapshot.update);
  const updateLabel = $derived(
    updateButtonLabel(update.status, update.installing) ||
      (update.installProblem ? "Repair needed" : ""),
  );
  const updateHelp = $derived(
    updateTooltip(
      update.status,
      update.installing,
      app.updateRestartBlockedReason,
    ) ||
      update.installProblem ||
      "",
  );

  let navEl = $state<HTMLElement | null>(null);
  let thumbY = $state(0);
  let thumbH = $state(0);
  let thumbReady = $state(false);
  let thumbAnimate = $state(false);
  let lastTab = "";

  function measureThumb(animate: boolean) {
    if (!navEl) return;
    const active = navEl.querySelector<HTMLButtonElement>(
      `[data-nav="${CSS.escape(activeTab)}"]`,
    );
    if (!active) return;
    const nextY = active.offsetTop;
    const nextH = active.offsetHeight;
    if (!nextH) return;
    thumbAnimate = animate && thumbReady;
    thumbY = nextY;
    thumbH = nextH;
    thumbReady = true;
  }

  $effect(() => {
    const tab = activeTab;
    const el = navEl;
    if (!el) return;
    const changed = lastTab !== "" && lastTab !== tab;
    lastTab = tab;
    measureThumb(changed);
    const observer = new ResizeObserver(() => measureThumb(false));
    observer.observe(el);
    let disposed = false;
    void document.fonts?.ready.then(() => {
      if (!disposed) measureThumb(false);
    });
    return () => {
      disposed = true;
      observer.disconnect();
    };
  });

  function navKeys(event: KeyboardEvent) {
    if (!(event.currentTarget instanceof HTMLElement)) return;
    const buttons = [
      ...(event.currentTarget
        .closest("nav")
        ?.querySelectorAll<HTMLButtonElement>("[data-nav]") ?? []),
    ];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : event.key === "ArrowDown"
            ? (index + 1) % buttons.length
            : event.key === "ArrowUp"
              ? (index + buttons.length - 1) % buttons.length
              : -1;
    if (next < 0) return;
    event.preventDefault();
    buttons[next]?.focus();
  }
</script>

<aside class="quiet-rail" use:scrollEdges>
  <nav
    bind:this={navEl}
    class="rail-track"
    class:thumb-ready={thumbReady}
    class:thumb-animate={thumbAnimate}
    aria-label="Main navigation"
  >
    <span
      class="rail-thumb"
      aria-hidden="true"
      style:transform={`translate3d(0, ${thumbY}px, 0)`}
      style:height={`${thumbH}px`}
    ></span>
    {#each tabs as tab}
      <button
        type="button"
        class="rail-item"
        class:selected={activeTab === tab.id}
        aria-current={activeTab === tab.id ? "page" : undefined}
        data-nav={tab.id}
        onkeydown={navKeys}
        onclick={() => onnavigate({ tab: tab.id })}
      >
        <span class="rail-icon"
          ><Icon
            name={tab.id === "dictation"
              ? "microphone"
              : tab.id === "meetings"
                ? "people"
                : tab.id}
            size={22}
          />
          {#if tab.id === "meetings" && recording}<span
              class="recording-dot"
              aria-label="Recording"
            ></span>{/if}
        </span><span>{tab.label}</span>
      </button>
    {/each}
  </nav>
  <div class="rail-tickets">
    {#if recording}
      <button
        type="button"
        class="stock rail-ticket recording-ticket"
        aria-label="Open recording meeting"
        onclick={() => onnavigate("meetings")}
      >
        <span class="recording-dot" aria-hidden="true"></span>
        {#if elapsed !== null}<strong data-text-surface="emboss"
            >{duration(elapsed)}</strong
          >{/if}
        <span data-text-surface="flat">Recording</span>
      </button>
    {/if}
    {#if updateLabel && !preview}
      <button
        type="button"
        class="stock rail-ticket update-ticket"
        aria-label={updateLabel}
        title={updateHelp}
        onclick={() =>
          onnavigate({ tab: "settings", stage: "updates", sheet: "update" })}
      >
        <span class="update-mark" aria-hidden="true"
          >{update.status.state === "error" ||
          (update.status.state === "ready" && update.status.installError) ||
          update.installProblem
            ? "!"
            : "↓"}</span
        >
        {#if update.status.state === "downloading" && !update.installing}
          <!-- The rail is too narrow for "Downloading"; the arrow, figure
               and channel say it, the label and tooltip spell it out. -->
          {#if update.status.percent === null}<span aria-hidden="true"
              >Starting download</span
            >{:else}<strong aria-hidden="true"
              >{Math.floor(update.status.percent)}%</strong
            >{/if}
          <span class="ticket-progress" aria-hidden="true"
            ><span style:width={`${update.status.percent ?? 0}%`}></span></span
          >
        {:else}
          <span>{updateLabel}</span>
        {/if}
      </button>
    {/if}
  </div>
</aside>

<style>
  .quiet-rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
    padding: 0 7px 18px;
    min-height: 0;
    overflow-y: auto;
    --edge-gap: 18px;
  }
  .rail-track {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    padding: 4px;
    border-radius: 19px;
    background-color: var(--well);
    box-shadow: var(--press);
  }
  .rail-thumb {
    position: absolute;
    top: 0;
    left: 4px;
    right: 4px;
    z-index: 0;
    border-radius: 15px;
    pointer-events: none;
    opacity: 0;
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    border: 1px solid var(--surface-border-color);
    box-shadow: var(--raise-sm);
  }
  .rail-track.thumb-ready .rail-thumb {
    opacity: 1;
  }
  .rail-track.thumb-animate .rail-thumb {
    transition:
      transform 220ms cubic-bezier(0.22, 1, 0.36, 1),
      height 220ms cubic-bezier(0.22, 1, 0.36, 1);
  }
  .rail-item {
    position: relative;
    z-index: 1;
    border: 1px solid transparent;
    background: transparent;
    color: var(--soft-ink);
    width: 100%;
    min-height: 68px;
    padding: 10px 2px 8px;
    border-radius: 15px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    font-size: 0.75rem;
    transition:
      background-color var(--motion-control) linear,
      color var(--motion-control) linear,
      transform var(--motion-control) var(--ease-settle);
  }
  .rail-item:hover:not(.selected) {
    background-color: color-mix(in srgb, var(--card) 45%, transparent);
    color: var(--ink);
  }
  .rail-item:active {
    transform: translateY(0.5px);
  }
  .rail-item.selected {
    color: var(--ink);
    font-weight: 600;
  }
  .rail-track:not(.thumb-ready) .rail-item.selected {
    background-color: var(--card);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    border-color: var(--surface-border-color);
    box-shadow: var(--raise-sm);
  }
  :global(.studio[data-material="off"]) .rail-track {
    box-shadow: none;
    border: 1px solid var(--line);
  }
  :global(.studio[data-material="off"]) .rail-thumb,
  :global(.studio[data-material="off"])
    .rail-track:not(.thumb-ready)
    .rail-item.selected {
    background-image: none;
    box-shadow: none;
    border-color: var(--line);
  }
  @media (prefers-contrast: more) {
    .rail-track {
      box-shadow: none;
      border: 1px solid currentColor;
    }
    .rail-thumb,
    .rail-track:not(.thumb-ready) .rail-item.selected {
      background-image: none;
      box-shadow: none;
      border-color: currentColor;
    }
  }
  .rail-icon {
    position: relative;
  }
  .rail-icon .recording-dot {
    position: absolute;
    top: -3px;
    right: -10px;
  }
  .recording-dot {
    display: block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--danger);
    box-shadow: 0 0 0 4px color-mix(in srgb, var(--danger) 14%, transparent);
  }
  .rail-tickets {
    margin-top: auto;
    display: grid;
    /* Never let a ticket's longest word widen the rail. */
    grid-template-columns: minmax(0, 1fr);
    gap: 12px;
    width: 100%;
  }
  .rail-ticket {
    border: 1px solid var(--surface-border-color);
    border-radius: 13px 12px 14px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 10px 4px;
    width: 100%;
    min-width: 0;
    font-size: 0.7rem;
    line-height: 1.25;
    text-align: center;
    overflow-wrap: anywhere;
    box-shadow: var(--raise-sm);
  }
  .rail-ticket:active {
    box-shadow: var(--press);
  }
  .rail-ticket strong {
    font-variant-numeric: tabular-nums;
    font-size: 0.85rem;
  }
  /* The first-run step channel: pressed well, ink fill. */
  .ticket-progress {
    display: block;
    width: calc(100% - 12px);
    height: 6px;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: var(--well);
    box-shadow: var(--press);
  }
  .ticket-progress > span {
    display: block;
    height: 100%;
    background: var(--ink);
    border-radius: 5px;
  }
  .update-mark {
    color: var(--dye);
    font-size: 1.1rem;
    font-weight: 650;
  }
  @media (max-width: 640px) {
    .quiet-rail {
      padding-inline: 2px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rail-item,
    .rail-thumb {
      transition: none;
    }
    .rail-item:active {
      transform: none;
    }
  }
</style>
