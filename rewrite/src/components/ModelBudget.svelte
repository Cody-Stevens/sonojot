<script lang="ts">
  import {
    formatBytes,
    type ModelEntry,
    type ModelLibrary,
  } from "../lib/model-library";
  import { modelBudget } from "../lib/model-budget";
  let {
    selected,
    hardware,
    sequential,
    cpu,
  }: {
    selected: (ModelEntry | undefined)[];
    hardware: ModelLibrary["hardware"] | undefined;
    sequential: boolean;
    cpu: boolean;
  } = $props();
  const budgets = $derived(
    (hardware?.unifiedMemory
      ? (["ram"] as const)
      : (["ram", "vram"] as const)
    ).map((resource) => ({
      label: hardware?.unifiedMemory
        ? "UNIFIED MEMORY"
        : resource === "ram"
          ? "HOST RAM"
          : "GPU VRAM",
      ...modelBudget({ selected, hardware, sequential, cpu, resource }),
    })),
  );
</script>

<div class="budgets" class:unified={hardware?.unifiedMemory}>
  {#each budgets as budget}
    {@const scale = Math.max(
      budget.total ?? 0,
      budget.segments.reduce((sum, segment) => sum + segment.bytes, 0),
      1,
    )}
    <div
      class="budget"
      data-budget={budget.label}
      data-unknown={budget.unknown}
    >
      <div class="caption">
        <span>{budget.label}</span><span data-text-surface="flat"
          >{budget.total === null
            ? "Capacity unknown"
            : hardware?.unifiedMemory
              ? `${(budget.total / 2 ** 30).toFixed(0)} GiB`
              : formatBytes(budget.total)}</span
        >
      </div>
      <div
        class="bar"
        role="img"
        aria-label={`${budget.label}: ${budget.estimate}; ${budget.unknown ? "headroom unknown" : `${formatBytes(budget.headroom)} planned headroom`}.`}
      >
        {#each budget.segments.filter((segment) => segment.bytes > 0) as segment}
          <span
            class="segment"
            data-segment={segment.label}
            style:width={`${(segment.bytes / scale) * (budget.unknown ? 75 : 100)}%`}
            title={`${segment.label}: ${formatBytes(segment.bytes)} projected; ${formatBytes(segment.loadedBytes)} loaded`}
            ><span
              class="loaded"
              style:width={`${(segment.loadedBytes / Math.max(segment.bytes, 1)) * 100}%`}
            ></span></span
          >
        {/each}
        {#if budget.unknown}<span
            class="unknown"
            title="Unknown; width is not a byte count"
          ></span>{/if}
      </div>
      <div class="caption secondary" data-text-surface="flat">
        <span>{budget.estimate}</span><span
          >{budget.free === null
            ? hardware?.unifiedMemory
              ? "Available now unknown"
              : "Free now unknown"
            : `${formatBytes(budget.free)} ${hardware?.unifiedMemory ? "available" : "free"} now`}</span
        >
      </div>
      {#if budget.over}<p class="over" data-text-surface="flat">
          Upper estimate + allowance exceeds capacity.
        </p>{/if}
    </div>
  {/each}
  <div class="legend" data-text-surface="flat">
    {#if sequential}<span
        ><i data-segment="Largest stage"></i>Projected / loaded</span
      >{:else}<span><i data-segment="Speech"></i>Projected speech</span><span
        ><i data-segment="Text"></i>Projected text</span
      >{/if}
    <span><i data-segment="System allowance"></i>System allowance</span><span
      ><i data-segment="Headroom"></i>Headroom</span
    ><span><i class="unknown"></i>Unknown</span>
  </div>
</div>

<style>
  .budgets {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 18px 27px;
    margin-top: 27px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
    font-size: 1rem;
  }
  .budgets.unified {
    grid-template-columns: minmax(0, 1fr);
  }
  .budget {
    min-width: 0;
  }
  .caption {
    display: flex;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0 9px;
  }
  .bar {
    display: flex;
    height: 18px;
    overflow: hidden;
    margin: 9px 0;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--well);
  }
  .segment {
    flex: none;
    position: relative;
    background: repeating-linear-gradient(
      135deg,
      color-mix(in srgb, var(--soft-ink) 24%, transparent) 0 4px,
      transparent 4px 8px
    );
  }
  .loaded {
    display: block;
    height: 100%;
    background: var(--ink);
  }
  [data-segment="Speech"],
  [data-segment="Largest stage"] {
    background: var(--ink);
  }
  [data-segment="Text"] {
    background: var(--soft-ink);
  }
  [data-segment="System allowance"] {
    background: repeating-linear-gradient(
      135deg,
      var(--edge) 0 4px,
      color-mix(in srgb, var(--edge) 45%, transparent) 4px 8px
    );
  }
  [data-segment="Headroom"] {
    background: var(--sheet);
  }
  .unknown {
    flex: 1;
    min-width: 25%;
    background: repeating-linear-gradient(
      135deg,
      transparent 0 4px,
      var(--soft-ink) 4px 5px
    );
  }
  .secondary,
  .legend {
    color: var(--soft-ink);
  }
  .legend {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: 9px 18px;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 9px;
  }
  .legend i {
    width: 9px;
    height: 9px;
    min-width: 9px;
    border: 1px solid var(--line);
  }
  .over {
    margin: 9px 0 0;
    color: var(--danger);
  }
  @media (max-width: 850px) {
    .budgets {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
