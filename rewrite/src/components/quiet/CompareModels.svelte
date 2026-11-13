<script lang="ts">
  import Sheet from "./Sheet.svelte";
  import {
    paretoFrontier,
    paretoPoints,
    speechViews,
    viewName,
    viewScore,
    viewTest,
    otherRunScore,
    waitText,
    type SpeechView,
    waitTicks,
    waitRange,
    gpuModel,
    shortSpeechName,
    labelPositions,
    type ParetoPoint,
  } from "../../lib/speech-chart";
  import { formatBytes, type ModelEntry } from "../../lib/model-library";

  // One Pareto chart for every speech model this computer can run: measured
  // word errors against the wait after you stop. Models without both numbers
  // stay off the chart and read "Not measured" in the table.
  let {
    models,
    mac,
    current,
    recommended,
    hardwareText,
    onclose,
  }: {
    models: ModelEntry[];
    mac: boolean;
    current: string;
    recommended: string;
    hardwareText: string;
    onclose: () => void;
  } = $props();
  let layout = $state<"chart" | "table">("chart");
  // Default: everyday speech, the public test closest to dictation. A Mac has
  // measured talks only, so it starts (and stays) there.
  const views = $derived(speechViews(mac));
  let chosen = $state<SpeechView | null>(null);
  const speech = $derived<SpeechView>(
    chosen && views.includes(chosen) ? chosen : views[0],
  );
  let active = $state("");
  const points = $derived(paretoPoints(models, mac, speech));
  const frontier = $derived(paretoFrontier(points));
  const unmeasured = $derived(
    models.filter((model) => !points.some((point) => point.id === model.id)),
  );
  const test = $derived(viewTest(speech, mac));

  // Plot geometry in viewBox units; the SVG scales to the sheet width.
  const W = 600,
    H = 330,
    left = 52,
    right = 150,
    top = 18,
    bottom = 272;
  const waits = $derived(points.map((point) => point.wait));
  const range = $derived(waitRange(waits));
  const xMin = $derived(range[0]);
  const xMax = $derived(range[1]);
  const errors = $derived(points.map((point) => point.errors));
  const yMin = $derived(Math.max(0, Math.floor(Math.min(...errors) - 0.5)));
  const yMax = $derived(Math.ceil(Math.max(...errors) + 0.5));
  const x = (wait: number) =>
    left +
    ((Math.log(wait) - Math.log(xMin)) / (Math.log(xMax) - Math.log(xMin))) *
      (W - left - right);
  const y = (value: number) =>
    top + ((value - yMin) / (yMax - yMin || 1)) * (bottom - top);
  const yTicks = $derived(
    Array.from(
      { length: yMax - yMin + 1 },
      (_, index) => yMin + index,
    ).filter(
      (_, index, all) => all.length <= 7 || index % 2 === 0 || index === 0,
    ),
  );
  const largest = $derived(
    Math.max(...points.map((point) => point.downloadBytes ?? 0), 1),
  );
  // Bigger mark, bigger download: area follows size, kept between 4 and 10.
  const radius = (point: ParetoPoint) =>
    4 + 6 * Math.sqrt((point.downloadBytes ?? 0) / largest);
  const labelled = $derived(
    points.filter(
      (point) =>
        frontier.includes(point.id) ||
        point.id === current ||
        point.id === recommended,
    ),
  );
  const labelY = $derived(
    labelPositions(
      labelled.map((point) => y(point.errors)),
      top + 6,
      bottom - 6,
      26,
    ),
  );
  const line = $derived(
    frontier
      .map((id) => points.find((point) => point.id === id)!)
      .map((point) => `${x(point.wait).toFixed(1)},${y(point.errors).toFixed(1)}`)
      .join(" "),
  );
  const shown = $derived(
    points.find((point) => point.id === active) ??
      points.find((point) => point.id === current) ??
      points.find((point) => point.id === recommended),
  );
  const percent = (value: number) =>
    Number.isInteger(value) ? value.toFixed(1) : String(value);
  function runsOn(model: Pick<ModelEntry, "backend">): string {
    return gpuModel(model) ? "Graphics" : "Processor";
  }
  function marks(id: string): string {
    return [
      id === recommended ? "Recommended" : "",
      id === current ? "In use" : "",
    ]
      .filter(Boolean)
      .join(", ");
  }
  function readout(point: ParetoPoint): string {
    return [
      `${point.name}${marks(point.id) ? ` (${marks(point.id)})` : ""}`,
      `${percent(point.errors)} word errors per 100 words`,
      `waits ${waitText(point.id, mac)}`,
      point.downloadBytes ? `${formatBytes(point.downloadBytes)} download` : "",
      point.gpu ? "uses the graphics chip" : "uses the processor",
      frontier.includes(point.id) ? "on the line" : "beaten on both by another",
    ]
      .filter(Boolean)
      .join(" · ");
  }
  const rows = $derived(
    [...models].sort((a, b) => {
      const score = (id: string) =>
        viewScore(id, mac, speech)?.value ??
        1000 + (otherRunScore(id, mac, speech)?.value ?? 1000);
      const sa = score(a.id);
      const sb = score(b.id);
      return sa - sb || a.name.localeCompare(b.name);
    }),
  );
</script>

<Sheet title="Compare models" {onclose}>
  {#if hardwareText}<p class="context" data-text-surface="flat">
      This computer: {hardwareText}
    </p>{/if}
  <div class="switches">
    {#if views.length > 1}<div
        class="segments view-switch"
        role="group"
        aria-label="Kind of speech"
      >
        {#each views as item}<button
            aria-pressed={speech === item}
            onclick={() => {
              chosen = item;
              active = "";
            }}>{viewName(item)}</button
          >{/each}
      </div>{/if}
    <div class="segments view-switch" role="group" aria-label="Show as">
      <button
        aria-pressed={layout === "chart"}
        onclick={() => (layout = "chart")}>Chart</button
      ><button
        aria-pressed={layout === "table"}
        onclick={() => (layout = "table")}>Table</button
      >
    </div>
  </div>
  {#if layout === "chart"}
    <p class="how" data-text-surface="flat">
      Up and left is better (fewer word errors, shorter wait); the line joins
      models nothing else beats on both.
    </p>
    {#if points.length}
      <figure class="pareto">
        <svg
          viewBox="0 0 {W} {H}"
          role="group"
          aria-label="Word errors against wait for {points.length} models. Use Tab to read each model, or switch to Table."
        >
          {#each yTicks as tick}<g class="grid"
              ><line x1={left} x2={W - right + 10} y1={y(tick)} y2={y(tick)} /><text
                x={left - 8}
                y={y(tick) + 4}
                text-anchor="end">{tick}%</text
              ></g
            >{/each}
          {#each waitTicks(xMin, xMax) as tick}<g class="grid"
              ><line x1={x(tick)} x2={x(tick)} y1={top} y2={bottom} /><text
                x={x(tick)}
                y={bottom + 16}
                text-anchor="middle">{tick} s</text
              ></g
            >{/each}
          <text class="axis" x={left} y={H - 14}
            >Wait after you stop (seconds, log scale)</text
          >
          <text
            class="axis"
            x={14}
            y={top}
            transform="rotate(-90 14 {top})"
            text-anchor="end">Fewer word errors →</text
          >
          {#if yMin > 0}<text class="axis-note" x={4} y={bottom + 30}
              >Axis starts at {yMin}%</text
            >{/if}
          {#if frontier.length > 1}<polyline class="frontier" points={line} />{/if}
          {#each points as point (point.id)}{@const cx = x(point.wait)}{@const cy =
              y(point.errors)}{@const r = radius(point)}
            <g
              class="mark"
              class:gpu={point.gpu}
              class:faded={!frontier.includes(point.id) &&
                point.id !== current &&
                point.id !== recommended}
              class:active={active === point.id}
              tabindex="0"
              role="button"
              aria-pressed={active === point.id}
              aria-label={readout(point)}
              data-chart-model={point.id}
              onfocus={() => (active = point.id)}
              onmouseenter={() => (active = point.id)}
              onclick={() => (active = point.id)}
              onkeydown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  active = point.id;
                }
              }}
            >
              {#if point.id === current}<circle
                  class="in-use"
                  {cx}
                  {cy}
                  r={r + 4}
                />{/if}
              {#if point.gpu}<rect
                  class="dot"
                  x={cx - r * 0.9}
                  y={cy - r * 0.9}
                  width={r * 1.8}
                  height={r * 1.8}
                  transform="rotate(45 {cx} {cy})"
                />{:else}<circle class="dot" {cx} {cy} {r} />{/if}
              <circle class="hit" {cx} {cy} r={Math.max(14, r + 6)} />
            </g>
          {/each}
          {#each labelled as point, index (point.id)}{@const cx = x(point.wait)}{@const ly =
              labelY[index]}
            <g class="label" class:gpu={point.gpu} aria-hidden="true">
              {#if Math.abs(ly - y(point.errors)) > 4}<line
                  class="leader"
                  x1={cx + radius(point) + 2}
                  y1={y(point.errors)}
                  x2={cx + radius(point) + 10}
                  y2={ly - 4}
                />{/if}
              <text x={cx + radius(point) + 12} y={ly - 2}
                ><tspan class="name">{point.name}</tspan></text
              >
              {#if marks(point.id)}<text
                  class="mark-word"
                  x={cx + radius(point) + 12}
                  y={ly + 11}>{marks(point.id)}</text
                >{/if}
            </g>
          {/each}
        </svg>
        <figcaption>
          <p class="readout" aria-live="polite" data-text-surface="flat">
            {shown ? readout(shown) : "Choose a mark to see its numbers."}
          </p>
          <p class="legend" data-text-surface="flat">
            <span class="key cpu">●</span> Processor
            <span class="key gpu">◆</span> Graphics
            <span class="key ring">◯</span> In use · Bigger mark, bigger download
          </p>
        </figcaption>
      </figure>
    {:else}<p class="how">
        No model here has both numbers measured for {viewName(
          speech,
        ).toLowerCase()} yet. The other views or the table may have them.
      </p>{/if}
    <p class="small" data-text-surface="flat">
      {mac
        ? "Only talks are measured in the Mac app so far."
        : "Everyday speech is shown first because it is the public test closest to dictation."}
      Word errors on {test}, all from one run{speech === "accented"
        ? "; SonoJot Parakeet was trained on other VoxPopuli speakers"
        : ""}. Waits were measured on SonoJot test computers; yours may
      differ.{#if unmeasured.length}{` ${unmeasured.length} ${unmeasured.length === 1 ? "model is" : "models are"} not measured here; see Table.`}{/if}
    </p>
  {:else}
    <div class="table-wrap">
      <table>
        <caption class="small" data-text-surface="flat"
          >Word errors per 100 words on {test}; lower is better.</caption
        >
        <thead
          ><tr
            ><th scope="col">Model</th><th scope="col">Runs on</th><th
              scope="col">Word errors</th
            ><th scope="col">Wait</th><th scope="col">Download</th></tr
          ></thead
        >
        <tbody>
          {#each rows as model (model.id)}{@const score = viewScore(
              model.id,
              mac,
              speech,
            )}{@const other = otherRunScore(model.id, mac, speech)}
            <tr data-table-model={model.id}>
              <th scope="row"
                >{shortSpeechName(model.id, model.name)}{#if marks(model.id)}<small
                    >{marks(model.id)}</small
                  >{/if}</th
              >
              <td>{runsOn(model)}</td>
              <td
                >{score
                  ? `${percent(score.value)}%`
                  : other
                    ? `${percent(other.value)}% (another run)`
                    : "Not measured"}</td
              >
              <td
                >{waitText(model.id, mac) === "not measured"
                  ? "Not measured"
                  : waitText(model.id, mac)}</td
              >
              <td>{model.diskBytes ? formatBytes(model.diskBytes) : "—"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <p class="small" data-text-surface="flat">
      "Another run" was measured on a different computer or test path, so it
      is listed but not compared on the chart. Sources are in each model's
      details.
    </p>
  {/if}
</Sheet>

<style>
  .context {
    margin: -8px 0 12px;
    color: var(--soft-ink);
    font-size: 14px;
  }
  .switches {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 14px;
    margin: 0 0 12px;
  }
  .view-switch {
    margin: 0;
    width: fit-content;
    padding: 3px;
    gap: 3px;
  }
  .view-switch button {
    flex: 0 0 auto;
    padding: 6px 14px;
    font-size: 14px;
    line-height: 1.2;
    white-space: nowrap;
  }
  .how {
    margin: 0 0 8px;
    font-size: 14px;
  }
  .pareto {
    --gpu-ink: var(--dye, #39563d);
    --cpu-ink: #926424;
    margin: 0;
  }
  :global([data-theme="slate"]) .pareto {
    --cpu-ink: #d4b573;
  }
  svg {
    display: block;
    width: 100%;
    height: auto;
    overflow: visible;
  }
  .grid line {
    stroke: var(--line);
  }
  .grid text,
  .axis,
  .axis-note {
    fill: var(--soft-ink);
    font-size: 11px;
  }
  .axis {
    font-size: 12px;
  }
  .frontier {
    fill: none;
    stroke: var(--soft-ink);
    stroke-width: 1.5;
  }
  .mark {
    color: var(--cpu-ink);
    cursor: pointer;
    outline: none;
  }
  .mark.gpu {
    color: var(--gpu-ink);
  }
  .dot {
    fill: currentColor;
    stroke: var(--card);
    stroke-width: 1;
  }
  .mark.faded .dot {
    opacity: 0.38;
  }
  .mark.active .dot,
  .mark:focus-visible .dot {
    opacity: 1;
    stroke: var(--ink);
    stroke-width: 2;
  }
  .mark:focus-visible .hit {
    stroke: var(--focus);
    stroke-width: 2;
  }
  .hit {
    fill: transparent;
  }
  .in-use {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
  }
  .label text {
    fill: var(--ink);
    font-size: 12px;
  }
  .label .name {
    font-weight: 600;
  }
  .label .mark-word {
    fill: var(--soft-ink);
    font-size: 11px;
  }
  .leader {
    stroke: var(--soft-ink);
    stroke-width: 0.8;
  }
  figcaption {
    margin-top: 6px;
  }
  .readout {
    min-height: 2.8em;
    margin: 0 0 6px;
    font-size: 14px;
  }
  .legend {
    margin: 0;
    font-size: 12px;
    color: var(--soft-ink);
  }
  .key {
    margin-left: 8px;
  }
  .key:first-child {
    margin-left: 0;
  }
  .key.cpu {
    color: #926424;
  }
  :global([data-theme="slate"]) .key.cpu {
    color: #d4b573;
  }
  .key.gpu {
    color: var(--dye);
  }
  .small {
    font-size: 12px;
    color: var(--soft-ink);
    margin: 10px 0 0;
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
  }
  caption {
    caption-side: top;
    text-align: left;
    margin: 0 0 8px;
  }
  th,
  td {
    text-align: left;
    padding: 7px 10px 7px 0;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
  }
  thead th {
    font-weight: 600;
    color: var(--soft-ink);
    font-size: 12px;
  }
  tbody th {
    font-weight: 600;
  }
  tbody th small {
    display: block;
    font-weight: 400;
    font-size: 12px;
    color: var(--dye);
  }
</style>
