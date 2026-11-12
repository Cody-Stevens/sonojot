<script lang="ts">
  import { app } from "../../lib/app.svelte";
  import { ai } from "../../lib/ai.svelte";
  import { cleanupChoice } from "../../lib/cleanup-choice";
  import { library } from "../../lib/model-library.svelte";
  import { modelBudget } from "../../lib/model-budget";
  import {
    gpuModel,
    primarySpeechPool,
    labelPositions,
    pointRadius,
    shortSpeechName,
    speechMeasurement,
    speechScore,
    waitText,
  } from "../../lib/speech-chart";
  import type { ModelEntry } from "../../lib/model-library";
  let {
    models,
    frontier,
    recommended,
    selected,
    meeting = false,
  }: {
    models: ModelEntry[];
    frontier: string[];
    recommended: string;
    selected: string;
    meeting?: boolean;
  } = $props();
  const hardware = $derived(library.data?.hardware);
  const mac = $derived(hardware?.unifiedMemory ?? false);
  const measured = $derived(
    models.filter((model) => speechScore(model.id, mac, meeting)),
  );
  const maxWer = $derived(
    Math.max(
      5,
      ...measured.map((model) =>
        Math.ceil(speechScore(model.id, mac, meeting)!.value + 1),
      ),
    ),
  );
  const plotBottom = $derived(
    Math.max(
      102,
      50 +
        Math.max(
          ...["ram", "vram"].map(
            (pool) =>
              measured.filter(
                (model) =>
                  primarySpeechPool(model) === pool &&
                  frontier.includes(model.id),
              ).length,
          ),
        ) *
          34,
    ),
  );
  const y = (value: number) => 35 + (value / maxWer) * (plotBottom - 35);
  const cleanup = $derived(
    cleanupChoice(
      app.snapshot.settings,
      app.snapshot.localProofreaderAvailable,
    ),
  );
  const setup = $derived(
    [
      selected,
      cleanup.kind === "local"
        ? cleanup.model
        : cleanup.kind === "sonojot"
          ? undefined
          : "local-rules",
      ai.data?.backend === "openrouter"
        ? "openrouter"
        : app.snapshot.settings.notesModel,
    ]
      .filter((id) => id !== "openrouter")
      .map((id) => library.data?.models.find((model) => model.id === id)),
  );
  function panel(pool: "ram" | "vram") {
    const resource = mac ? "ram" : pool;
    const budget = modelBudget({
      selected: setup,
      hardware,
      resource,
      sequential: app.snapshot.settings.memoryMode === "sequential",
      cpu: app.snapshot.settings.textAcceleration === "cpu",
    });
    const total = budget.total == null ? null : budget.total / 1e9;
    const rows = measured.filter((model) => primarySpeechPool(model) === pool);
    const peak = (model: ModelEntry) =>
      speechMeasurement(model.id)?.[resource]?.measuredPeak ?? null;
    const points = rows.filter((model) => peak(model) !== null);
    const x = (gb: number) => 43 + Math.min(1, gb / (total || 1)) * 365;
    const labelled = points.filter((model) => frontier.includes(model.id));
    const labels = labelPositions(
      labelled.map((model) => y(speechScore(model.id, mac, meeting)!.value)),
      43,
      plotBottom - 8,
      34,
    );
    return {
      total,
      points,
      x,
      peak,
      labelled,
      labels,
      share:
        total === null || budget.demand.unknown
          ? null
          : budget.demand.max / 1e9,
    };
  }
</script>

<div class="speech-chart">
  <div class="legend">
    <span class="gpu">● GPU</span><span class="cpu">● CPU</span><span
      >◎ recommended</span
    ><span>✓ in use</span>
  </div>
  {#each ["vram", "ram"] as pool}
    {@const p = panel(pool as "ram" | "vram")}
    <section
      class="plot"
      aria-label={pool === "vram"
        ? "GPU memory chart"
        : "CPU system memory chart"}
    >
      <div class="plot-title">
        <strong class:gpu={pool === "vram"} class:cpu={pool === "ram"}
          >{pool === "vram" ? "GPU" : "CPU"}</strong
        ><span
          >{pool === "vram"
            ? mac
              ? "Apple GPU · unified memory"
              : "Graphics memory · whole card"
            : mac
              ? "System memory · the same pool"
              : "System RAM · separate pool"}</span
        >
      </div>
      {#if pool === "vram" && !mac && !p.total}
        <div class="empty-plot">
          No compatible dedicated GPU memory reported. These models use system
          RAM.
        </div>
      {:else if !p.total}<div class="empty-plot">
          Memory capacity unavailable. No budget line can be calculated.
        </div>
      {:else}
        <svg
          viewBox={`0 0 440 ${plotBottom + 26}`}
          role="img"
          aria-label={`${pool === "vram" ? "Graphics" : "System"} memory versus word errors; fewer errors toward the top`}
        >
          {#if p.share !== null}<rect
              class="reserved"
              x={p.x(p.share)}
              y="30"
              width={408 - p.x(p.share)}
              height={plotBottom - 30}
            /><line
              class="budget"
              x1={p.x(p.share)}
              x2={p.x(p.share)}
              y1="30"
              y2={plotBottom}
            />{/if}
          <text x="43" y="20">↑ fewer word errors</text>
          {#each [0, maxWer / 2, maxWer] as tick}<line
              class="grid"
              x1="43"
              x2="408"
              y1={y(tick)}
              y2={y(tick)}
            /><text x="36" y={y(tick) + 4} text-anchor="end">{tick}%</text
            >{/each}
          {#each [0, p.total / 2, p.total] as tick}<text
              x={p.x(tick)}
              y={plotBottom + 19}
              text-anchor="middle">{Number(tick.toFixed(1))} GB</text
            >{/each}
          {#each p.points as model}
            {@const px = p.x(p.peak(model)!)}{@const py = y(
              speechScore(model.id, mac, meeting)!.value,
            )}
            <g
              data-chart-model={model.id}
              class:gpu={gpuModel(model)}
              class:cpu={!gpuModel(model)}
            >
              <title
                >{model.name}: {speechScore(model.id, mac, meeting)!.value}%
                word errors; wait {waitText(model.id)}; {p.peak(model)} GB measured
                peak</title
              >
              {#if model.id === recommended}<circle
                  class="ring"
                  cx={px}
                  cy={py}
                  r={pointRadius(model.id) + 4}
                />{/if}
              <circle
                class="point"
                class:hollow={!frontier.includes(model.id)}
                cx={px}
                cy={py}
                r={pointRadius(model.id)}
              />
              {#if model.id === selected}<text
                  class="check"
                  x={px}
                  y={py + 3}
                  text-anchor="middle">✓</text
                >{/if}
            </g>
          {/each}
          {#each p.labelled as model, i}
            {@const px = p.x(p.peak(model)!)}{@const py = y(
              speechScore(model.id, mac, meeting)!.value,
            )}
            {@const lx = 250}
            <g
              data-chart-label={model.id}
              class:gpu={gpuModel(model)}
              class:cpu={!gpuModel(model)}
              ><path
                class="leader"
                d={`M${px + 7},${py} L${lx - 5},${p.labels[i] - 5}`}
              /><text class="model-label" x={lx} y={p.labels[i] - 6}
                >{shortSpeechName(model.id, model.name)}</text
              ><text x={lx} y={p.labels[i] + 8}>{waitText(model.id)} wait</text
              ></g
            >
          {/each}
          {#if !p.points.length}<text x="225" y="75" text-anchor="middle"
              >{mac && pool === "ram"
                ? "No CPU speech models in this selection"
                : "Word errors or peak memory not measured"}</text
            >{/if}
        </svg>
        <div class="budget-note">
          {p.share === null
            ? "SonoJot’s share is unknown"
            : `SonoJot’s planned share · ${p.share.toFixed(1)} of ${p.total.toFixed(1)} GB${mac ? " unified memory" : pool === "vram" ? " on this card" : " system memory"}`}<span
            >Shaded: left for your apps</span
          >
        </div>
      {/if}
    </section>
  {/each}
  <p class="chart-note">
    One dot per model. Larger dots answer sooner. Test computers differ; the
    share is an estimate, not a hard limit.
  </p>
</div>

<style>
  .speech-chart {
    --gpu-ink: var(--dye, #39563d);
    --cpu-ink: #926424;
  }
  :global([data-theme="slate"]) .speech-chart {
    --cpu-ink: #d4b573;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 13px;
    font-size: 12px;
    margin: 0 0 8px;
  }
  .gpu {
    color: var(--gpu-ink);
  }
  .cpu {
    color: var(--cpu-ink);
  }
  .plot {
    border: 1px solid var(--line);
    border-radius: 5px;
    padding: 10px 10px 6px;
    margin-bottom: 9px;
    background:
      repeating-linear-gradient(
        0deg,
        transparent 0 11px,
        color-mix(in srgb, var(--ink) 4%, transparent) 11px 12px
      ),
      repeating-linear-gradient(
        90deg,
        transparent 0 11px,
        color-mix(in srgb, var(--ink) 4%, transparent) 11px 12px
      );
  }
  .plot-title {
    display: flex;
    gap: 10px;
    align-items: baseline;
    font-size: 12px;
  }
  .plot-title strong {
    font-size: 15px;
  }
  svg {
    display: block;
    width: 100%;
    overflow: visible;
  }
  text {
    fill: var(--soft-ink);
    font:
      12px Stationery,
      Georgia,
      serif;
  }
  g text {
    fill: currentColor;
  }
  .model-label {
    font-weight: 600;
    font-size: 13px;
  }
  .grid {
    stroke: var(--line);
  }
  .reserved {
    fill: color-mix(in srgb, var(--ink) 4%, transparent);
  }
  .budget {
    stroke: var(--soft-ink);
    stroke-dasharray: 4 4;
  }
  .point {
    fill: currentColor;
    stroke: currentColor;
  }
  .point.hollow,
  .ring {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
  }
  .check {
    fill: var(--dye-ink);
    font-size: 10px;
    font-weight: bold;
  }
  .leader {
    stroke: currentColor;
    opacity: 0.45;
    fill: none;
  }
  .budget-note {
    display: grid;
    gap: 2px;
    font-size: 11px;
    color: var(--soft-ink);
  }
  .budget-note span {
    font-size: 10px;
  }
  .chart-note {
    font-size: 12px;
    line-height: 1.4;
    color: var(--soft-ink);
    margin: 9px 0 0;
  }
  .empty-plot {
    min-height: 82px;
    display: flex;
    align-items: center;
    padding: 16px 20px;
    font-size: 14px;
    color: var(--soft-ink);
  }
</style>
