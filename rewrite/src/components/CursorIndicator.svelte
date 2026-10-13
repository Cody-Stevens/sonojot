<script lang="ts">
  import { app } from "../lib/app.svelte";
  import { normalizeRms } from "../lib/meter";
  const state = $derived(app.snapshot.state);
  const busy = $derived(
    state === "loading" || state === "finishing" || state === "cleaning",
  );
  const done = $derived(state === "idle");
  const failed = $derived(
    state === "error" || (done && Boolean(app.snapshot.error)),
  );
  const level = $derived(normalizeRms(app.snapshot.audioLevel));
  const label = $derived(
    failed
      ? "Dictation needs attention"
      : done
        ? "Dictation ready"
        : state === "loading"
          ? "Preparing dictation"
          : busy
            ? "Finishing dictation"
            : "Listening",
  );
</script>

<div
  class="cursor-feedback"
  class:busy
  class:done
  class:failed
  data-theme={app.snapshot.settings.theme}
  aria-label={label}
>
  <svg viewBox="0 0 48 48" aria-hidden="true">
    <circle class="stock" cx="24" cy="24" r="17" />
    <circle class="ring" cx="24" cy="24" r="12" />
    {#if failed}
      <path class="mark" d="M24 16v10m0 5v.1" />
    {:else if done}
      <path class="mark" d="m17 24 5 5 10-11" />
    {:else if !busy}
      <circle class="level-dot" cx="24" cy="24" r={3 + 3 * level} />
    {/if}
  </svg>
</div>

<style>
  .cursor-feedback {
    width: 48px;
    height: 48px;
    display: grid;
    place-items: center;
    color: #40553e;
    pointer-events: none;
  }
  svg {
    width: 48px;
    height: 48px;
    overflow: visible;
  }
  .stock {
    fill: #ede9da;
    stroke: #ffffffb3;
    stroke-width: 1;
    filter: drop-shadow(0 1px 2px #18261a55);
  }
  .ring {
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    opacity: 0.8;
    transform-origin: 24px 24px;
    animation: breathe 1.4s ease-in-out infinite alternate;
  }
  .level-dot {
    fill: currentColor;
    transition: r 80ms ease-out;
  }
  .mark {
    fill: none;
    stroke: currentColor;
    stroke-width: 2.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .busy .ring {
    stroke-dasharray: 22 54;
    opacity: 1;
    animation: turn 0.85s linear infinite;
  }
  .done .ring,
  .failed .ring {
    animation: none;
    opacity: 0.3;
  }
  .done svg {
    animation: arrive 160ms ease-out both;
  }
  .failed {
    color: #9a652d;
  }
  [data-theme="slate"] {
    color: #d8ddc4;
  }
  [data-theme="slate"] .stock {
    fill: #394139;
    stroke: #8b928280;
  }
  [data-theme="slate"].failed {
    color: #e5b973;
  }
  @keyframes breathe {
    from {
      opacity: 0.45;
      transform: scale(0.94);
    }
    to {
      opacity: 0.9;
      transform: scale(1.04);
    }
  }
  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes arrive {
    from {
      transform: scale(0.85);
      opacity: 0.5;
    }
    to {
      transform: scale(1);
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ring,
    .busy .ring,
    .done svg {
      animation: none;
    }
    .level-dot {
      transition: none;
    }
  }
</style>
