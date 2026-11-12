// Version 1: selected rows from artifacts/asr-pareto/models.json, 2026-10-12.
// Keep one leaderboard cohort. Never mix the newer leaderboard average here.
export const source = {
  date: "2026-10-12",
  url: "https://huggingface.co/datasets/hf-audio/open-asr-leaderboard-results/blob/f82da8589575706245e606f6c845bb3df6c93c71/english_short_latest.csv",
  description:
    "Open ASR Leaderboard: classic English average across seven public sets, excluding TED-LIUM",
};
export type Device = "CPU" | "GPU";
export interface AsrPoint {
  name: string;
  gb: number;
  wer: number;
  device: Device | "both";
  offered: boolean;
  reason: string;
}
export const points: readonly AsrPoint[] = [
  {
    name: "Qwen 1.7B Q8",
    gb: 2.52,
    wer: 6.25,
    device: "GPU",
    offered: true,
    reason: "Our GPU dictation choice, informed by tests on real dictation.",
  },
  {
    name: "Qwen 1.7B Q4",
    gb: 1.64,
    wer: 6.25,
    device: "CPU",
    offered: true,
    reason: "Our most accurate measured CPU dictation option.",
  },
  {
    name: "Qwen 0.6B Q8",
    gb: 1.019141728,
    wer: 6.93,
    device: "CPU",
    offered: true,
    reason: "Smaller multilingual option for less powerful computers.",
  },
  {
    name: "Qwen 1.7B sherpa",
    gb: 2.4,
    wer: 6.25,
    device: "CPU",
    offered: false,
    reason: "Larger and slower than Qwen Q4 in SonoJot's tests.",
  },
  {
    name: "Qwen 1.7B FP16",
    gb: 4.73,
    wer: 6.25,
    device: "GPU",
    offered: false,
    reason: "Larger download and slower response than Qwen Q8.",
  },
  {
    name: "Granite 4.1 NAR",
    gb: 2.5,
    wer: 5.7,
    device: "GPU",
    offered: true,
    reason: "For meetings: fewer errors on room microphones in our tests.",
  },
  {
    name: "Granite 4.1 AR",
    gb: 2.56,
    wer: 5.65,
    device: "GPU",
    offered: false,
    reason: "Similar accuracy to NAR, much slower in our measured runtime.",
  },
  {
    name: "Granite 4.0",
    gb: 2.56,
    wer: 5.87,
    device: "GPU",
    offered: false,
    reason: "Superseded by 4.1; repeated output on some test clips.",
  },
  {
    name: "Granite 3.3 2B",
    gb: 3.2,
    wer: 6.35,
    device: "GPU",
    offered: false,
    reason: "Needs a runtime SonoJot doesn't ship.",
  },
  {
    name: "Granite 3.3 8B",
    gb: 9.5,
    wer: 6.07,
    device: "GPU",
    offered: false,
    reason: "Much larger download for a small accuracy gain.",
  },
  {
    name: "Cohere Transcribe",
    gb: 2.41,
    wer: 5.84,
    device: "GPU",
    offered: false,
    reason: "Did not improve dictation over Qwen in our tests.",
  },
  {
    name: "Canary-Qwen",
    gb: 2.8,
    wer: 6.05,
    device: "GPU",
    offered: false,
    reason: "More errors and slower than Qwen on our test recordings.",
  },
  {
    name: "Parakeet v3",
    gb: 0.67,
    wer: 6.82,
    device: "both",
    offered: false,
    reason:
      "More holdout errors than v2 in our sherpa CPU full-clip comparison.",
  },
  {
    name: "Parakeet v2",
    gb: 0.66,
    wer: 6.44,
    device: "CPU",
    offered: true,
    reason: "Compact English speech recognition on CPU.",
  },
  {
    name: "Parakeet 110M",
    gb: 0.135,
    wer: 7.98,
    device: "CPU",
    offered: false,
    reason: "Not yet tested through SonoJot's recognition path.",
  },
  {
    name: "Canary 180M",
    gb: 0.21,
    wer: 7.48,
    device: "CPU",
    offered: false,
    reason: "Needs a speech adapter SonoJot doesn't ship.",
  },
  {
    name: "Canary 1B Flash",
    gb: 1.05,
    wer: 6.81,
    device: "both",
    offered: false,
    reason: "Needs a speech adapter SonoJot doesn't ship.",
  },
  {
    name: "Moonshine Medium",
    gb: 0.27,
    wer: 7.18,
    device: "CPU",
    offered: true,
    reason: "Fast English streaming with live text.",
  },
  {
    name: "Moonshine Small",
    gb: 0.14,
    wer: 8.42,
    device: "CPU",
    offered: true,
    reason: "Small English streaming option.",
  },
  {
    name: "Whisper Turbo",
    gb: 1.04,
    wer: 8.44,
    device: "both",
    offered: true,
    reason:
      "Kept for languages Qwen does not cover; GPU support requires Apple Metal.",
  },
  {
    name: "Whisper Large v3",
    gb: 1.78,
    wer: 7.95,
    device: "GPU",
    offered: false,
    reason: "More word errors than Qwen; slow on CPU.",
  },
  {
    name: "Whisper Medium.en",
    gb: 0.95,
    wer: 8.66,
    device: "CPU",
    offered: false,
    reason: "More errors than smaller CPU options.",
  },
  {
    name: "Whisper Small.en",
    gb: 0.375,
    wer: 9.24,
    device: "CPU",
    offered: true,
    reason:
      "Multilingual sibling kept for languages Qwen lacks; plotted WER is Small.en.",
  },
  {
    name: "Voxtral Mini 3B",
    gb: 3.19,
    wer: 7.53,
    device: "GPU",
    offered: false,
    reason: "Less accurate than Qwen at a larger size.",
  },
  {
    name: "Voxtral Realtime",
    gb: 2.6,
    wer: 8.24,
    device: "GPU",
    offered: false,
    reason: "More errors than Qwen on the published English average.",
  },
  {
    name: "Voxtral Small 24B",
    gb: 14,
    wer: 7.06,
    device: "GPU",
    offered: false,
    reason: "Much larger than Qwen without an accuracy gain.",
  },
  {
    name: "Phi-4 multimodal",
    gb: 4,
    wer: 6.46,
    device: "GPU",
    offered: false,
    reason: "Needs a runtime SonoJot doesn't ship.",
  },
];

export function frontier(rows: readonly AsrPoint[]): AsrPoint[] {
  return rows
    .filter(
      (point) =>
        !rows.some(
          (other) =>
            other.gb <= point.gb &&
            other.wer <= point.wer &&
            (other.gb < point.gb || other.wer < point.wer),
        ),
    )
    .sort((a, b) => a.gb - b.gb || a.wer - b.wer);
}

export function devicePoints(device: Device): AsrPoint[] {
  return points.filter(
    (point) => point.device === device || point.device === "both",
  );
}

/** A direct label, or a number keyed to the list under the chart. */
export interface ChartLabel {
  point: AsrPoint;
  text: string;
  x: number;
  y: number;
  width: number;
  height: number;
  /** Drawn when the label sits away from its point. */
  leader: { x1: number; y1: number; x2: number; y2: number } | null;
  key?: number;
  /** Text is anchored on the side facing its point, so an estimated width
      never opens a gap between a label and its point or leader. */
  anchor?: "start" | "middle" | "end";
  textX?: number;
}

type Box = { x: number; y: number; width: number; height: number };

const LABEL_HEIGHT = 16;
const PLOT = { top: 32, bottom: 288, left: 28 };

export function labelWidth(text: string, bold: boolean): number {
  return text.length * (bold ? 7.3 : 6.6) + 4;
}

function boxDistance(px: number, py: number, b: Box): number {
  const dx = Math.max(b.x - px, 0, px - (b.x + b.width));
  const dy = Math.max(b.y - py, 0, py - (b.y + b.height));
  return Math.hypot(dx, dy);
}

function segmentDistance(
  px: number,
  py: number,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
): number {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const t =
    dx || dy
      ? Math.max(
          0,
          Math.min(1, ((px - x1) * dx + (py - y1) * dy) / (dx * dx + dy * dy)),
        )
      : 0;
  return Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy));
}

function segmentsCross(
  a: { x1: number; y1: number; x2: number; y2: number },
  b: { x1: number; y1: number; x2: number; y2: number },
): boolean {
  const side = (x: number, y: number, s: typeof a) =>
    Math.sign((s.x2 - s.x1) * (y - s.y1) - (s.y2 - s.y1) * (x - s.x1));
  return (
    side(b.x1, b.y1, a) * side(b.x2, b.y2, a) < 0 &&
    side(a.x1, a.y1, b) * side(a.x2, a.y2, b) < 0
  );
}

/**
 * Places a label for every offered model and Whisper, beside its point when
 * that is unambiguous and otherwise at the end of a leader line. Left-out
 * models get a name where one fits near the point, or else a number keyed to
 * the list under the chart, so every point can be identified.
 */
export function chartLayout(rows: readonly AsrPoint[], width: number) {
  const x = (gb: number) =>
    30 + ((Math.log10(gb) + 1) / Math.log10(200)) * (width - 48);
  // Smaller error rates sit lower on the page, matching the explanatory copy.
  const y = (wer: number) => 278 - ((wer - 5.4) / 4.2) * 230;
  const plotted = rows.map((point) => ({
    point,
    x: x(point.gb),
    y: y(point.wer),
  }));
  type Plotted = (typeof plotted)[number];
  const right = width - 4;
  const labels: ChartLabel[] = [];
  const leaders = () => labels.flatMap((l) => (l.leader ? [l.leader] : []));
  // Labels also keep off the frontier line; leaders may cross it.
  const edge = frontier(rows).map((p) => ({ x: x(p.gb), y: y(p.wer) }));
  const frontierSegments = edge.slice(1).map((b, i) => ({
    x1: edge[i].x,
    y1: edge[i].y,
    x2: b.x,
    y2: b.y,
  }));

  const inside = (b: Box) =>
    b.x >= PLOT.left &&
    b.x + b.width <= right &&
    b.y >= PLOT.top &&
    b.y + b.height <= PLOT.bottom;
  const hitsLabel = (b: Box) =>
    labels.some(
      (l) =>
        b.x < l.x + l.width + 3 &&
        b.x + b.width + 3 > l.x &&
        b.y < l.y + l.height + 3 &&
        b.y + b.height + 3 > l.y,
    );
  const hitsPoint = (b: Box, own: Plotted | null) =>
    plotted.some((q) => q !== own && boxDistance(q.x, q.y, b) < 7);
  const hitsLeader = (b: Box) =>
    [...leaders(), ...frontierSegments].some((s) => {
      for (let t = 0; t <= 1; t += 0.05) {
        const px = s.x1 + (s.x2 - s.x1) * t;
        const py = s.y1 + (s.y2 - s.y1) * t;
        if (boxDistance(px, py, b) < 2) return true;
      }
      return false;
    });
  // A label beside its point must be clearly nearer to it than to any other.
  const owns = (b: Box, p: Plotted) => {
    const own = boxDistance(p.x, p.y, b);
    return plotted.every((q) => q === p || boxDistance(q.x, q.y, b) > own + 9);
  };
  const free = (b: Box, own: Plotted | null) =>
    inside(b) && !hitsLabel(b) && !hitsPoint(b, own) && !hitsLeader(b);

  function beside(p: Plotted, w: number): Box | null {
    const h = LABEL_HEIGHT;
    const spots = [
      { x: p.x + 9, y: p.y - h / 2 },
      { x: p.x - 9 - w, y: p.y - h / 2 },
      { x: p.x + 5, y: p.y - 7 - h },
      { x: p.x + 5, y: p.y + 7 },
      { x: p.x - 5 - w, y: p.y - 7 - h },
      { x: p.x - 5 - w, y: p.y + 7 },
      { x: p.x - w / 2, y: p.y - 9 - h },
      { x: p.x - w / 2, y: p.y + 9 },
    ];
    for (const s of spots) {
      const b = { ...s, width: w, height: h };
      if (free(b, p) && owns(b, p)) return b;
    }
    return null;
  }

  function leadered(
    p: Plotted,
    w: number,
    maxReach: number,
    strict = true,
  ): { box: Box; leader: NonNullable<ChartLabel["leader"]> } | null {
    const h = LABEL_HEIGHT;
    let best: {
      box: Box;
      leader: NonNullable<ChartLabel["leader"]>;
      cost: number;
    } | null = null;
    for (let reach = 22; reach <= maxReach; reach += 6) {
      // Every candidate costs at least its reach, so nothing longer can win.
      if (best && reach >= best.cost) break;
      for (let step = 0; step < 24; step++) {
        const angle = (step / 24) * Math.PI * 2;
        const ax = p.x + Math.cos(angle) * reach;
        const ay = p.y + Math.sin(angle) * reach;
        const c = Math.cos(angle);
        const s = Math.sin(angle);
        const bx = c > 0.35 ? ax : c < -0.35 ? ax - w : ax - w / 2;
        const by = s > 0.35 ? ay : s < -0.35 ? ay - h : ay - h / 2;
        const box = {
          x: Math.max(PLOT.left, Math.min(right - w, bx)),
          y: Math.max(PLOT.top, Math.min(PLOT.bottom - h, by)),
          width: w,
          height: h,
        };
        if (!free(box, null)) continue;
        // Away from its point, a label must not read as another point's:
        // no other point may sit nearly as close to it as its own leader.
        const reachToBox = boxDistance(p.x, p.y, box);
        const margin = strict ? Math.max(12, reachToBox * 0.8) : 9;
        if (plotted.some((q) => q !== p && boxDistance(q.x, q.y, box) < margin))
          continue;
        const leader = {
          x1: p.x,
          y1: p.y,
          x2: Math.max(box.x, Math.min(box.x + w, p.x)),
          y2: Math.max(box.y, Math.min(box.y + h, p.y)),
        };
        // Points stacked on this one are skipped: the leader starts clear of them.
        const length = Math.hypot(leader.x2 - leader.x1, leader.y2 - leader.y1);
        const sx = leader.x1 + ((leader.x2 - leader.x1) / length) * 8;
        const sy = leader.y1 + ((leader.y2 - leader.y1) / length) * 8;
        const crossings = leaders().filter((other) =>
          segmentsCross(leader, other),
        ).length;
        if (strict && crossings) continue;
        const clear =
          plotted.every(
            (q) =>
              q === p ||
              Math.hypot(q.x - p.x, q.y - p.y) < 8 ||
              segmentDistance(q.x, q.y, sx, sy, leader.x2, leader.y2) >
                (strict ? 6 : 4),
          ) &&
          labels.every((l) => {
            for (let t = 0.1; t <= 1; t += 0.05) {
              const px = leader.x1 + (leader.x2 - leader.x1) * t;
              const py = leader.y1 + (leader.y2 - leader.y1) * t;
              if (boxDistance(px, py, l) < 1) return false;
            }
            return true;
          });
        if (!clear) continue;
        const cost = reach + crossings * 40;
        if (!best || cost < best.cost) best = { box, leader, cost };
      }
    }
    return best;
  }

  function place(
    p: Plotted,
    text: string,
    bold: boolean,
    maxReach: number,
    key?: number,
  ) {
    const w = labelWidth(text, bold);
    const near = beside(p, w);
    if (near) {
      labels.push({ point: p.point, text, ...near, leader: null, key });
      return true;
    }
    // Crossing another leader is better than leaving a point unidentified.
    const far =
      leadered(p, w, maxReach) ??
      (maxReach > 100 ? leadered(p, w, 320, false) : null);
    if (far) {
      labels.push({
        point: p.point,
        text,
        ...far.box,
        leader: far.leader,
        key,
      });
      return true;
    }
    return false;
  }

  const required = (point: AsrPoint) =>
    point.offered || point.name.startsWith("Whisper");
  const crowding = (p: Plotted) =>
    plotted.filter((q) => q !== p && Math.hypot(q.x - p.x, q.y - p.y) < 40)
      .length;
  const byCrowding = (a: Plotted, b: Plotted) =>
    crowding(a) - crowding(b) || a.point.gb - b.point.gb;

  function attempt(
    first: readonly AsrPoint[],
    order: (a: Plotted, b: Plotted) => number,
  ) {
    labels.length = 0;
    const missed: AsrPoint[] = [];
    const early = (p: Plotted) => (first.includes(p.point) ? 0 : 1);

    // Offered models and Whisper always carry their name.
    const named = plotted
      .filter((p) => required(p.point))
      .sort((a, b) => early(a) - early(b) || order(a, b));
    for (const p of named) {
      if (place(p, p.point.name, p.point.offered, 200)) continue;
      missed.push(p.point);
      // A future dataset could exhaust the space; keep the name visible anyway.
      const w = labelWidth(p.point.name, p.point.offered);
      labels.push({
        point: p.point,
        text: p.point.name,
        x: Math.max(PLOT.left, Math.min(right - w, p.x - w / 2)),
        y: Math.min(PLOT.bottom - LABEL_HEIGHT, p.y + 9),
        width: w,
        height: LABEL_HEIGHT,
        leader: null,
      });
    }

    // Left-out models: a name where it fits nearby on wider charts.
    const unnamed: Plotted[] = [];
    for (const p of plotted.filter((p) => !required(p.point)).sort(order)) {
      if (width < 500 || !place(p, p.point.name, false, 48)) unnamed.push(p);
    }

    // Otherwise a number, keyed in order of size.
    unnamed.sort(
      (a, b) => a.point.gb - b.point.gb || a.point.wer - b.point.wer,
    );
    const key = unnamed.map((p, i) => ({ key: i + 1, point: p.point }));
    const numbered = [...unnamed].sort(
      (a, b) => early(a) - early(b) || order(a, b),
    );
    for (const p of numbered) {
      const n = unnamed.indexOf(p) + 1;
      if (place(p, String(n), false, 200, n)) continue;
      missed.push(p.point);
      const w = labelWidth(String(n), false);
      labels.push({
        point: p.point,
        text: String(n),
        x: Math.max(PLOT.left, Math.min(right - w, p.x + 7)),
        y: Math.max(PLOT.top, Math.min(PLOT.bottom - LABEL_HEIGHT, p.y - 8)),
        width: w,
        height: LABEL_HEIGHT,
        leader: null,
        key: n,
      });
    }
    return { missed, key, placed: [...labels] };
  }

  // Try a few placement orders and keep the clearest result: nothing
  // unplaced, then the shortest and least tangled leaders.
  const score = (r: ReturnType<typeof attempt>) => {
    const lines = r.placed.flatMap((l) => (l.leader ? [l.leader] : []));
    const length = lines.reduce(
      (sum, l) => sum + Math.hypot(l.x2 - l.x1, l.y2 - l.y1),
      0,
    );
    let crossings = 0;
    lines.forEach((a, i) =>
      lines
        .slice(i + 1)
        .forEach((b) => (crossings += segmentsCross(a, b) ? 1 : 0)),
    );
    return r.missed.length * 1000 + length + crossings * 40;
  };
  const orders: ((a: Plotted, b: Plotted) => number)[] = [
    byCrowding,
    (a, b) => byCrowding(b, a),
    (a, b) => a.point.gb - b.point.gb,
    (a, b) => a.point.wer - b.point.wer,
  ];
  let best: ReturnType<typeof attempt> | null = null;
  for (const order of orders) {
    // A point that found no space is placed first on the next try.
    let result = attempt([], order);
    let first: AsrPoint[] = [];
    for (let tries = 0; tries < 3 && result.missed.length; tries++) {
      first = [...new Set([...result.missed, ...first])];
      const next = attempt(first, order);
      if (next.missed.length < result.missed.length) result = next;
    }
    if (!best || score(result) < score(best)) best = result;
  }
  if (!best) throw new Error("unreachable");
  labels.length = 0;
  labels.push(
    ...best.placed.map((l) => {
      const px = l.leader ? l.leader.x2 : x(l.point.gb);
      const anchor: ChartLabel["anchor"] =
        px <= l.x + 1 ? "start" : px >= l.x + l.width - 1 ? "end" : "middle";
      const textX =
        anchor === "start"
          ? l.x + 2
          : anchor === "end"
            ? l.x + l.width - 2
            : l.x + l.width / 2;
      return { ...l, anchor, textX };
    }),
  );
  const key = best.key;

  return { x, y, plotted, labels, key, unplaced: best.missed };
}
