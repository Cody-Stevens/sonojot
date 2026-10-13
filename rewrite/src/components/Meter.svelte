<script lang="ts">
  import { untrack } from "svelte";
  import { meterDescription, normalizeRms, smoothMeter } from "../lib/meter";
  let {
    level,
    active = false,
    compact = false,
  }: { level: number; active?: boolean; compact?: boolean } = $props();
  const bars = Array.from({ length: 28 }, (_, index) => index);
  let displayed = $state(0);
  $effect(() => {
    const target = active ? normalizeRms(level) : 0;
    if (
      !active ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) {
      displayed = target;
      return;
    }
    let current = untrack(() => displayed);
    if (current === target) return;
    let previousTime = performance.now();
    let frame = 0;
    const update = (time: number) => {
      current = smoothMeter(current, target, time - previousTime);
      previousTime = time;
      displayed = current;
      if (Math.abs(target - current) > 0.001)
        frame = requestAnimationFrame(update);
      else displayed = target;
    };
    frame = requestAnimationFrame(update);
    return () => cancelAnimationFrame(frame);
  });
</script>

<div
  class:compact
  class="meter"
  role="meter"
  aria-label="Audio input level"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={Math.round(displayed * 100)}
  aria-valuetext={meterDescription(level, active)}
>
  {#each bars as bar}<span
      class:lit={active && displayed > bar / bars.length}
      class:high={bar > 23}
    ></span>{/each}
</div>
