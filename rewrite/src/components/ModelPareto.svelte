<script lang="ts">
  import {
    devicePoints,
    chartLayout,
    frontier,
    source,
    type AsrPoint,
    type Device,
  } from "../lib/asr-pareto";
  let device = $state<Device>("CPU");
  let selected = $state<AsrPoint | null>(null);
  const rows = $derived(devicePoints(device));
  const edge = $derived(frontier(rows));
  let width = $state(590);
  const layout = $derived(chartLayout(rows, width));
  const x = (gb: number) => layout.x(gb);
  const y = (wer: number) => layout.y(wer);
  const line = $derived(edge.map((p) => `${x(p.gb)},${y(p.wer)}`).join(" "));
  function describe(p: AsrPoint): string {
    return `${p.name}: ${p.gb.toFixed(2)} GB, ${p.wer.toFixed(2)}% word error rate. ${p.reason}`;
  }
</script>

<details class="why">
  <summary>Why these models</summary>
  <p class="takeaway">
    Qwen makes fewer English word errors than Whisper at a similar size, which
    is why SonoJot recommends it.
  </p>
  <div class="views" aria-label="Chart device">
    {#each ["CPU", "GPU"] as view}
      <button
        class="button"
        aria-pressed={device === view}
        onclick={() => {
          device = view as Device;
          selected = null;
        }}>{view}</button
      >
    {/each}
  </div>
  <p class="meta" data-text-surface="flat">
    Smaller downloads to the left, fewer word errors toward the bottom. Select a
    point for the reason behind each choice.
  </p>
  <div class="legend" data-text-surface="flat">
    <span><i class="offered-key"></i>Offered in SonoJot</span>
    <span><i class="left-out-key"></i>Left out</span>
    <span><i class="frontier-key"></i>Best accuracy for its size</span>
  </div>
  <div class="plot" bind:clientWidth={width}>
    <svg
      viewBox={`0 0 ${width} 330`}
      role="group"
      aria-label={`${device} model download size versus published English word error rate`}
    >
      <text x="0" y="18">Word errors (%), lower is better</text>
      {#each [6, 7, 8, 9] as tick}
        <line x1="30" x2={width - 18} y1={y(tick)} y2={y(tick)} class="grid" />
        <text x="20" y={y(tick) + 4} text-anchor="end">{tick}</text>
      {/each}
      {#each [0.1, 0.3, 1, 3, 10] as tick}
        <text x={x(tick)} y="301" text-anchor="middle">{tick}</text>
      {/each}
      <text x={width / 2} y="324" text-anchor="middle"
        >Download GB (log scale)</text
      >
      <polyline points={line} class="frontier" />
      {#each layout.labels as label (label.point.name)}
        <line
          class="leader"
          x1={x(label.point.gb)}
          y1={y(label.point.wer)}
          x2={Math.max(
            label.x,
            Math.min(label.x + label.width, x(label.point.gb)),
          )}
          y2={label.y + label.height / 2}
        />
        <text
          class="model-label"
          class:offered-label={label.point.offered}
          x={label.x + 2}
          y={label.y + 12}>{label.point.name}</text
        >
      {/each}
      {#each rows as point (point.name)}
        <circle
          cx={x(point.gb)}
          cy={y(point.wer)}
          r={selected?.name === point.name ? 7 : 5}
          class:offered={point.offered}
          class:selected={selected?.name === point.name}
          tabindex="0"
          role="button"
          aria-label={describe(point)}
          aria-pressed={selected?.name === point.name}
          onfocus={() => (selected = point)}
          onmouseenter={() => (selected = point)}
          onclick={() => (selected = point)}
          onkeydown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              selected = point;
            }
          }}
        >
          <title>{describe(point)}</title>
        </circle>
      {/each}
    </svg>
  </div>
  <p class="description" aria-live="polite" data-text-surface="flat">
    {selected
      ? describe(selected)
      : "Select a point to see the model and why it is included or left out."}
  </p>
  <details class="values">
    <summary>Model values</summary>
    <ul>
      {#each rows as point}<li>
          <button onclick={() => (selected = point)}>{point.name}</button> · {point.gb.toFixed(
            2,
          )} GB · {point.wer}%{point.offered ? " · Offered" : ""}<span
            >{point.reason}</span
          >
        </li>{/each}
    </ul>
  </details>
  <p class="meta" data-text-surface="flat">
    {source.description}. Snapshot {source.date}, revision f82da858. Family
    scores are not measurements of these quantized downloads. Sizes exclude
    runtimes; some unshipped sizes are estimates. SonoJot's own tests on real
    dictation also informed the defaults.
  </p>
  <p class="meta" data-text-surface="flat">
    Whisper Small and Medium use their English-only sibling's scores. Models
    without a comparable score are omitted, including Apple Speech.
    Non-commercial models, including Canary 1B and Granite TurboCTC-NC, are
    excluded from the frontier.
  </p>
  <a href={source.url} target="_blank" rel="noreferrer">Leaderboard source</a>
</details>

<style>
  .why {
    margin-block: 18px;
  }
  summary {
    cursor: pointer;
  }
  .views {
    display: flex;
    gap: 9px;
    margin-block: 18px 9px;
  }
  .takeaway {
    margin-block: 18px 9px;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 9px 18px;
    font-size: 0.75rem;
    margin-bottom: 9px;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .legend i {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .offered-key {
    background: var(--dye);
    border: 2px solid var(--dye);
  }
  .left-out-key {
    border: 1.5px solid var(--soft-ink);
  }
  .legend .frontier-key {
    width: 22px;
    height: 0;
    border-top: 1.5px dashed var(--dye);
    border-radius: 0;
  }
  .plot {
    width: 100%;
    min-width: 0;
  }
  button[aria-pressed="true"] {
    outline: 2px solid var(--dye);
  }
  svg {
    width: 100%;
    height: 330px;
    display: block;
    overflow: visible;
    color: var(--ink);
  }
  svg text {
    fill: currentColor;
    font-family: inherit;
    font-size: 12px;
  }
  .grid {
    stroke: currentColor;
    opacity: 0.15;
  }
  .frontier {
    fill: none;
    stroke: var(--dye);
    stroke-width: 1.5;
    stroke-dasharray: 4 3;
  }
  .leader {
    stroke: var(--soft-ink);
    stroke-width: 0.7;
    opacity: 0.6;
  }
  .model-label {
    paint-order: stroke;
    stroke: var(--card);
    stroke-width: 3px;
    stroke-linejoin: round;
    pointer-events: none;
  }
  .offered-label {
    fill: var(--dye);
    font-weight: 650;
  }
  circle {
    fill: var(--card);
    stroke: var(--soft-ink);
    stroke-width: 1.5;
    cursor: pointer;
  }
  circle.offered {
    fill: var(--dye);
    stroke: var(--dye);
  }
  circle:focus,
  circle.selected {
    outline: none;
    stroke: currentColor;
    stroke-width: 2.5;
  }
  .meta {
    font-size: 0.8rem;
    line-height: 1.45;
  }
  .description {
    min-height: 3em;
    font-size: 0.9rem;
  }
  .values ul {
    padding-left: 18px;
    font-size: 0.85rem;
  }
  .values li {
    margin-block: 9px;
  }
  .values span {
    display: block;
  }
  .values button {
    color: inherit;
    font: inherit;
    background: none;
    border: none;
    padding: 0;
    text-decoration: underline;
    cursor: pointer;
  }
  a {
    font-size: 0.8rem;
  }
</style>
