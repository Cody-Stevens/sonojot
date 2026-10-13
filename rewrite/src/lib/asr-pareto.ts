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
  const labels: {
    point: AsrPoint;
    x: number;
    y: number;
    width: number;
    height: number;
  }[] = [];
  const required = (point: AsrPoint) =>
    point.offered || point.name.startsWith("Whisper");
  const notable = (point: AsrPoint) =>
    /^(Canary|Voxtral|Granite 3\.3|Phi-4)/.test(point.name);
  const priority = (point: AsrPoint) =>
    required(point) ? 0 : notable(point) ? 1 : 2;
  for (const p of [...plotted].sort(
    (a, b) => priority(a.point) - priority(b.point),
  )) {
    if (!required(p.point) && !notable(p.point)) continue;
    if (width < 500 && !required(p.point)) continue;
    const w = p.point.name.length * 6.5 + 4;
    const h = 16;
    const candidates = [
      { x: p.x + 10, y: p.y - h / 2 },
      { x: p.x - w - 10, y: p.y - h / 2 },
      { x: p.x - w / 2, y: p.y - h - 10 },
      { x: p.x - w / 2, y: p.y + 10 },
    ];
    // Dense clusters can use a nearby free row, connected back to their point.
    for (let offset = 18; offset <= 234; offset += 18) {
      for (const direction of [-1, 1]) {
        for (const left of [p.x + 10, p.x - w - 10, p.x - w / 2]) {
          candidates.push({
            x: Math.max(28, Math.min(width - w - 4, left)),
            y: p.y + direction * offset - h / 2,
          });
        }
      }
    }
    if (required(p.point)) {
      for (let top = 32; top <= 283 - h; top += 2) {
        for (let left = 28; left <= width - w - 4; left += 8) {
          candidates.push({ x: left, y: top });
        }
      }
    }
    const position = candidates.find(
      (c) =>
        c.x >= 28 &&
        c.x + w <= width - 4 &&
        c.y >= 32 &&
        c.y + h <= 283 &&
        !labels.some(
          (l) =>
            c.x < l.x + l.width + 3 &&
            c.x + w + 3 > l.x &&
            c.y < l.y + l.height + 3 &&
            c.y + h + 3 > l.y,
        ) &&
        !plotted.some(
          (q) =>
            q.x + 7 > c.x &&
            q.x - 7 < c.x + w &&
            q.y + 7 > c.y &&
            q.y - 7 < c.y + h,
        ),
    );
    // Required labels remain visible even if a future dataset exhausts the space.
    if (position || required(p.point)) {
      labels.push({
        point: p.point,
        ...(position ?? {
          x: Math.max(28, Math.min(width - w - 4, p.x - w / 2)),
          y: p.y + 10,
        }),
        width: w,
        height: h,
      });
    }
  }
  return { x, y, plotted, labels };
}
