<script lang="ts">
  import type { MeetingSummary } from "../../lib/protocol";
  import { duration } from "../../lib/protocol";
  import { meetingBusy, meetingStatus } from "../../lib/meeting-display";
  import { app } from "../../lib/app.svelte";
  let {
    meeting,
    match = "",
    onopen,
    onexport,
  }: {
    meeting: MeetingSummary;
    match?: string;
    onopen: () => void;
    onexport: () => void;
  } = $props();
  const time = $derived(new Date(meeting.createdAt));
</script>

<article class="meeting-slip">
  <button class="slip-open" data-meeting-id={meeting.id} onclick={onopen}>
    <time datetime={meeting.createdAt} data-text-surface="flat"
      >{Number.isFinite(time.getTime())
        ? time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : "—"}</time
    >
    <span class="slip-copy"
      ><strong data-text-surface="emboss">{meeting.title}</strong><small
        data-text-surface="flat"
        >{duration(meeting.durationSeconds)} <span aria-hidden="true">·</span>
        {meetingStatus[meeting.status] ?? meeting.status}
        {#if meeting.speakerCount}<span
            class="speaker-dots"
            aria-label={`${meeting.speakerCount} speakers`}
            >{#each Array(Math.min(meeting.speakerCount, 4)) as _, i}<i
                style:--speaker-index={i}
              ></i>{/each}{#if meeting.speakerCount > 4}<span
                class="speaker-more">+{meeting.speakerCount - 4}</span
              >{/if}</span
          >{/if}
      </small>{#if match}<small class="search-match" data-text-surface="flat"
          >{match}</small
        >{/if}</span
    >
  </button>
  <div class="slip-actions">
    <button
      class="button small"
      disabled={!app.native ||
        app.dirty ||
        meetingBusy(meeting.status) ||
        app.busy("meeting-export")}
      title="Export as rich text (.rtf)"
      onclick={onexport}>Export</button
    >
  </div>
</article>

<style>
  .meeting-slip {
    position: relative;
    display: flex;
    align-items: center;
    margin: 0;
    padding: 0;
    background: transparent;
  }
  .slip-open {
    display: flex;
    align-items: center;
    gap: 18px;
    text-align: left;
    padding: 14px 18px;
    width: 100%;
    min-height: 72px;
    color: var(--ink);
    background: transparent;
    border: 0;
    box-shadow: none;
    transition: background-color var(--motion-control) linear;
  }
  .slip-open:hover,
  .meeting-slip:focus-within .slip-open {
    background-color: var(--well);
  }
  .slip-open:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  time {
    color: var(--soft-ink);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    width: 68px;
    flex: 0 0 68px;
  }
  .slip-copy {
    min-width: 0;
    flex: 1 1 auto;
    padding-right: 96px;
  }
  strong {
    font-size: 17px;
    line-height: 22px;
    font-weight: 600;
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  small {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 7px;
    margin-top: 4px;
    color: var(--soft-ink);
    font-size: 13px;
    line-height: 18px;
    font-variant-numeric: tabular-nums;
  }
  .search-match {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .slip-actions {
    position: absolute;
    right: 16px;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--motion-control) linear;
  }
  .meeting-slip:hover .slip-actions,
  .meeting-slip:focus-within .slip-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .speaker-dots {
    display: inline-flex;
    align-items: center;
    margin-left: 2px;
  }
  .speaker-dots i {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--dye);
    opacity: calc(1 - var(--speaker-index) * 0.14);
    border: 1px solid var(--card);
    margin-left: -2px;
  }
  .speaker-dots i:first-child {
    margin-left: 0;
  }
  .speaker-more {
    margin-left: 4px;
    font-size: 12px;
  }
  @media (max-width: 760px) {
    time {
      width: 56px;
      flex-basis: 56px;
    }
    .slip-copy {
      padding-right: 84px;
    }
  }
  @media (hover: none) {
    .slip-actions {
      opacity: 1;
      pointer-events: auto;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .slip-open,
    .slip-actions {
      transition: none;
    }
  }
</style>
