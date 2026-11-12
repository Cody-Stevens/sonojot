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

<article class="stock paper-card meeting-slip">
  <button class="slip-open" data-meeting-id={meeting.id} onclick={onopen}>
    <time datetime={meeting.createdAt} data-text-surface="flat"
      >{Number.isFinite(time.getTime())
        ? time.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : "—"}</time
    >
    <span class="record-mark" aria-hidden="true">◉</span>
    <span class="slip-copy"
      ><strong data-text-surface="emboss">{meeting.title}</strong><small
        data-text-surface="flat"
        >{duration(meeting.durationSeconds)} <span>·</span>
        {meetingStatus[meeting.status] ?? meeting.status}
        {#if meeting.speakerCount}<span
            class="speaker-dots"
            aria-label={`${meeting.speakerCount} speakers`}
            >{#each Array(Math.min(meeting.speakerCount, 5)) as _, i}<i
                style:--speaker-index={i}
              ></i>{/each}</span
          >{/if}
        {#if meeting.autoNamed}<span class="quiet-tag">AUTO-NAMED</span>{/if}
        {#if meeting.missingSeconds}<span class="quiet-tag amber"
            >MISSING {Math.round(meeting.missingSeconds)} S</span
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
      onclick={onexport}>Export .md</button
    >
  </div>
</article>

<style>
  .meeting-slip {
    position: relative;
    display: flex;
    align-items: center;
    margin: 10px 0;
    padding: 0;
    border-radius: 8px 13px 11px 9px;
    box-shadow: var(--raise);
  }
  .slip-open {
    display: flex;
    align-items: center;
    gap: 20px;
    text-align: left;
    padding: 16px 24px;
    width: 100%;
    color: var(--ink);
    background: none;
    border: 0;
    box-shadow: none;
    min-height: 80px;
  }
  time {
    color: var(--soft-ink);
    font-size: 14px;
    min-width: 60px;
  }
  .record-mark {
    font-size: 30px;
    color: var(--soft-ink);
    width: 44px;
  }
  .slip-copy {
    min-width: 0;
    flex: 1;
    padding-right: 110px;
  }
  strong {
    font-size: 20px;
    font-weight: 600;
    display: block;
  }
  small {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 9px;
    margin-top: 6px;
    color: var(--soft-ink);
    font-size: 14px;
  }
  .slip-actions {
    position: absolute;
    right: 20px;
    opacity: 0;
    pointer-events: none;
    transition: opacity 140ms;
  }
  .meeting-slip:hover .slip-actions,
  .meeting-slip:focus-within .slip-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .speaker-dots {
    display: inline-flex;
    gap: 3px;
  }
  .speaker-dots i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--dye);
    opacity: calc(1 - var(--speaker-index) * 0.12);
  }
  @media (max-width: 760px) {
    .record-mark {
      display: none;
    }
    .slip-open {
      gap: 12px;
      padding: 16px;
    }
    .slip-copy {
      padding-right: 90px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .slip-actions {
      transition: none;
    }
  }
</style>
