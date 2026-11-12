<script lang="ts">
  import Icon from "./Icon.svelte";
  import { app } from "../lib/app.svelte";
  import { call } from "../lib/api";
  import { duration, type MeetingDetail } from "../lib/protocol";
  import { tabs, type Route, type RouteTarget } from "../lib/route.svelte";
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
  <nav aria-label="Main navigation">
    {#each tabs as tab}
      <button
        type="button"
        class="rail-item"
        class:selected={route.tab === tab.id}
        aria-current={route.tab === tab.id ? "page" : undefined}
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
          >{update.status.state === "error" || update.installProblem
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
    padding: 12px 8px 18px;
    min-height: 0;
    overflow-y: auto;
    --edge-gap: 8px;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rail-item {
    border: 0;
    background: transparent;
    width: 74px;
    min-height: 70px;
    padding: 10px 2px 8px;
    border-radius: 16px 15px 17px 15px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    font-size: 0.75rem;
    transition:
      background-color var(--motion-control),
      box-shadow var(--motion-control);
  }
  .rail-item:hover:not(.selected) {
    background-color: var(--well);
  }
  .rail-item.selected {
    background-color: var(--dye);
    color: var(--dye-ink);
    background-image: var(--paper-image);
    background-size: var(--paper-tile-size);
    background-position: top left;
    background-blend-mode: multiply;
    box-shadow: var(--raise-sm);
    font-weight: 600;
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
    .rail-item {
      width: 70px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rail-item {
      transition: none;
    }
  }
</style>
