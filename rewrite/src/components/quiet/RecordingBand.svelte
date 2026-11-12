<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../../lib/app.svelte";
  import type { MeetingSession } from "../../lib/meeting-session.svelte";
  import { inkTrace, type InkSample } from "../../lib/meeting-display";
  import { meetingClock } from "../../lib/meeting-notices";
  import Sheet from "./Sheet.svelte";
  let {
    session,
    onsources,
  }: { session: MeetingSession; onsources: () => void } = $props();
  let samples = $state<InkSample[]>([]);
  let more = $state(false);
  let anchor = $state<HTMLButtonElement | null>(null);
  let now = $state(Date.now());
  const current = $derived(
    session.meetings.find((row) => row.id === app.snapshot.activeMeetingId) ??
      session.selected,
  );
  const source = $derived(
    current?.source ?? app.snapshot.settings.meetingSource,
  );
  const elapsed = $derived(
    current
      ? Math.max(
          current.durationSeconds,
          (now - Date.parse(current.createdAt)) / 1000,
        )
      : 0,
  );
  const lostMic = $derived(
    app.snapshot.meetingSourceStatus.some((text) =>
      /microphone.*(lost|disconnect|unavailable)/i.test(text),
    ),
  );
  const lostSystem = $derived(
    !!app.snapshot.meetingSystemAudio ||
      app.snapshot.meetingSourceStatus.some((text) =>
        /system.*(lost|disconnect|unavailable)/i.test(text),
      ),
  );
  const channels = [
    { id: "microphone", label: "You", detail: "microphone" },
    { id: "system", label: "Others", detail: "system audio" },
  ] as const;
  onMount(() => {
    const timer = window.setInterval(() => {
      now = Date.now();
      const levels = app.snapshot.meetingLevels;
      samples = [
        ...samples.slice(-119),
        {
          at: now,
          microphone:
            source === "system" || lostMic
              ? null
              : (levels?.microphone ?? null),
          system:
            source === "microphone" || lostSystem
              ? null
              : (levels?.system ?? null),
        },
      ];
    }, 250);
    return () => clearInterval(timer);
  });
</script>

<section
  class="stock paper-card recording-band"
  aria-label="Recording this meeting"
>
  <div class="recording-head">
    <span class="record-dot"></span>
    <h1 data-text-surface="emboss">Recording</h1>
    <time>{meetingClock(elapsed)}</time><span class="spacer"></span><button
      class="button stop"
      disabled={app.busy("meeting-record")}
      onclick={session.stop}
      >■ {app.busy("meeting-record") ? "Saving…" : "Stop"}</button
    ><button
      class="button small"
      bind:this={anchor}
      aria-label="Recording options"
      onclick={() => (more = true)}>⋯</button
    >
  </div>
  {#each channels as channel}
    {@const off = source !== "both" && source !== channel.id}
    {@const lost = channel.id === "microphone" ? lostMic : lostSystem}
    <div class="source-line">
      <div class="source-label">
        {channel.label}<small>{channel.detail}</small>
      </div>
      <svg
        viewBox="0 0 600 34"
        preserveAspectRatio="none"
        aria-label={`${channel.label} input trace`}
        role="img"><path d={inkTrace(samples, channel.id)} /></svg
      ><small class:lost
        >{off
          ? "NOT RECORDED"
          : lost
            ? "LOST · RECONNECTING"
            : !app.snapshot.meetingLevels
              ? "Waiting for input"
              : ""}</small
      >
    </div>
  {/each}
  {#if session.systemNotice}<div class="system-notice" role="alert">
      <span>{session.systemNotice.text}</span
      >{#if session.systemNotice.fix}<button
          class="button small"
          onclick={session.fixSystem}>{session.systemNotice.fix}</button
        >{/if}
    </div>{/if}
  {#if app.snapshot.meetingSourceStatus.length}<p
      class="field-help"
      role="status"
      aria-label="Recording source status"
    >
      {app.snapshot.meetingSourceStatus.join(" · ")}
    </p>{/if}
  {#if session.systemFixError}<p role="alert">{session.systemFixError}</p>{/if}
</section>
{#if more}<Sheet title="Recording" {anchor} onclose={() => (more = false)}
    ><p class="field-help">{session.sourceName(source)}</p>
    <button
      class="quiet-option"
      onclick={() => {
        more = false;
        onsources();
      }}>Sources</button
    ><button
      class="quiet-option danger-text"
      disabled={app.busy("meeting-record")}
      onclick={async () => {
        more = false;
        await session.discardRecording();
      }}>Discard recording…</button
    ></Sheet
  >{/if}

<style>
  .recording-band {
    padding: 20px 26px;
    border-radius: 16px;
    margin: 0 0 22px;
  }
  .recording-head {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 18px;
  }
  h1 {
    font-size: 24px;
    margin: 0;
  }
  time {
    font-size: 20px;
    font-variant-numeric: tabular-nums;
    color: var(--soft-ink);
  }
  .spacer {
    flex: 1;
  }
  .record-dot {
    width: 10px;
    height: 10px;
    background: var(--danger);
    border-radius: 50%;
  }
  .stop {
    color: var(--danger);
  }
  .source-line {
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 12px 0;
    min-height: 42px;
  }
  .source-label {
    width: 110px;
    flex: none;
    font-size: 16px;
  }
  .source-label small {
    display: block;
    color: var(--soft-ink);
    font-size: 12px;
  }
  svg {
    height: 34px;
    flex: 1;
    min-width: 60px;
    color: var(--dye);
  }
  path {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    vector-effect: non-scaling-stroke;
  }
  .source-line > small {
    font-size: 12px;
    color: var(--soft-ink);
    flex: 0 0 150px;
    min-height: 16px;
    box-sizing: border-box;
  }
  .source-line > small.lost {
    color: var(--danger);
    border-left: 2px dashed currentColor;
    padding-left: 12px;
  }
  .system-notice {
    display: flex;
    gap: 12px;
    align-items: center;
    font-size: 14px;
  }
</style>
