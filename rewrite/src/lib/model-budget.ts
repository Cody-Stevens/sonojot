import {
  formatBytes,
  memoryRange,
  type ModelEntry,
  type ModelLibrary,
} from "./model-library";

type Range = { min: number; max: number; unknown: boolean };
export type BudgetSegment = {
  label: string;
  bytes: number;
  loadedBytes: number;
};
export type MemoryBudget = {
  total: number | null;
  free: number | null;
  demand: Range;
  allowance: number | null;
  headroom: number | null;
  unknown: boolean;
  over: boolean;
  segments: BudgetSegment[];
  estimate: string;
  detail: string;
};
const zero = (): Range => ({ min: 0, max: 0, unknown: false });

function range(
  model: ModelEntry | undefined,
  resource: "ram" | "vram",
  cpu: boolean,
): Range {
  if (!model) return { ...zero(), unknown: true };
  if (model.id === "local-rules") return zero();
  if (resource === "vram") {
    if (model.kind === "cleanup" && cpu) return zero();
    if (
      model.kind === "asr" &&
      /cpu/i.test(model.backend) &&
      !/cuda|nvidia/i.test(model.backend)
    )
      return zero();
  }
  const min = resource === "ram" ? model.ramMinBytes : model.vramMinBytes;
  const max = resource === "ram" ? model.ramMaxBytes : model.vramMaxBytes;
  // Earlier hosts reported no text VRAM estimate as null (or zero). Auto can
  // offload these same weights, so zero must not masquerade as known demand.
  const missingTextVram =
    resource === "vram" && model.kind === "cleanup" && !cpu && max === 0;
  return {
    min: min ?? 0,
    max: max ?? min ?? 0,
    unknown:
      min === null || max === null || max < (min ?? 0) || missingTextVram,
  };
}

function peak(ranges: Range[]): Range {
  return {
    min: Math.max(0, ...ranges.map((r) => r.min)),
    max: Math.max(0, ...ranges.map((r) => r.max)),
    unknown: ranges.some((r) => r.unknown),
  };
}

function isLoaded(model: ModelEntry | undefined): boolean {
  return (
    !!model && ["loading", "ready", "busy", "unloading"].includes(model.state)
  );
}

export function modelBudget({
  selected,
  hardware,
  sequential,
  cpu,
  resource,
}: {
  selected: readonly (ModelEntry | undefined)[];
  hardware: ModelLibrary["hardware"] | undefined;
  sequential: boolean;
  cpu: boolean;
  resource: "ram" | "vram";
}): MemoryBudget {
  const distinct = [
    ...new Map(
      selected
        .filter(
          (model) =>
            model?.kind !== "diarization" && model?.kind !== "punctuation",
        )
        .map((model, index) => [model?.id ?? `missing-${index}`, model]),
    ).values(),
  ];
  const speech = peak(
    distinct
      .filter((m) => m?.kind === "asr")
      .map((m) => range(m, resource, cpu)),
  );
  const text = peak(
    distinct
      .filter((m) => m?.kind === "cleanup")
      .map((m) => range(m, resource, cpu)),
  );
  const demand = sequential
    ? peak([speech, text])
    : {
        min: speech.min + text.min,
        max: speech.max + text.max,
        unknown: speech.unknown || text.unknown,
      };
  demand.unknown ||= distinct.some((m) => !m);
  const loadedModels = distinct.map((model) =>
    isLoaded(model) ? model : undefined,
  );
  const loadedSpeech = peak(
    loadedModels
      .filter((m) => m?.kind === "asr")
      .map((m) => range(m, resource, cpu)),
  );
  const loadedText = peak(
    loadedModels
      .filter((m) => m?.kind === "cleanup")
      .map((m) => range(m, resource, cpu)),
  );
  const loadedDemand = sequential
    ? peak([loadedSpeech, loadedText])
    : {
        min: loadedSpeech.min + loadedText.min,
        max: loadedSpeech.max + loadedText.max,
        unknown: loadedSpeech.unknown || loadedText.unknown,
      };
  const total =
    (resource === "ram"
      ? hardware?.totalRamBytes
      : hardware?.dedicatedVramBytes) ?? null;
  const free =
    (resource === "ram"
      ? hardware?.availableRamBytes
      : hardware?.availableVramBytes) ?? null;
  // A disclosed planning allowance, not an estimate of current OS allocations.
  const fraction = resource === "ram" ? 0.15 : 0.1;
  const allowance = total === null ? null : total * fraction;
  const unknown = demand.unknown || total === null;
  const planned = demand.max + (allowance ?? 0);
  const headroom =
    unknown || total === null ? null : Math.max(0, total - planned);
  const over = total !== null && planned > total;
  const segments = sequential
    ? [
        {
          label: "Largest stage",
          bytes: demand.max,
          loadedBytes: loadedDemand.max,
        },
      ]
    : [
        { label: "Speech", bytes: speech.max, loadedBytes: loadedSpeech.max },
        { label: "Text", bytes: text.max, loadedBytes: loadedText.max },
      ];
  if (allowance !== null)
    segments.push({
      label: "System allowance",
      bytes: allowance,
      loadedBytes: 0,
    });
  if (headroom !== null)
    segments.push({ label: "Headroom", bytes: headroom, loadedBytes: 0 });
  const estimate = demand.unknown
    ? "Estimate unknown"
    : `${memoryRange(demand.min, demand.max)} models`;
  const detail = [
    hardware?.unifiedMemory
      ? "Apple Silicon shares memory between CPU and GPU. Available now estimates free plus inactive pages; it is not a separate GPU pool."
      : "",
    "Hatched sections are projected model demand; solid sections are models currently loaded. Free now is live capacity.",
    resource === "vram" && !cpu
      ? "Auto proposes GPU offload. Text may fall back to CPU before allocation when the combination exceeds VRAM; this plan still shows the proposed GPU demand. The device label reports the initialized runtime separately."
      : "",
    sequential
      ? "Stages load separately; the largest stage sets the peak."
      : "Speech may overlap one text model; Cleanup and Notes use the larger estimate, never their sum.",
    "Shared weights count once. Host estimates remain conservative when text uses GPU offload.",
    `System allowance: ${fraction * 100}% of total capacity for ${resource === "ram" ? "the OS and other apps" : "display and other GPU work"}; this is a planning reserve, not measured usage.`,
    "Projected demand is not a claim that memory is allocated. Free now is separate and already includes current resident allocations.",
    `Free now: ${free === null ? "unknown" : formatBytes(free)}.`,
    unknown
      ? "Hatching means an unknown estimate or capacity; its width is not a byte count. Headroom cannot be determined."
      : `Planned headroom: ${formatBytes(headroom)}.`,
    ...distinct
      .filter((m): m is ModelEntry => !!m)
      .map((m) => `${m.name}: ${m.memoryBasis || "Estimate basis unknown"}`),
  ].join(" ");
  return {
    total,
    free,
    demand,
    allowance,
    headroom,
    unknown,
    over,
    segments,
    estimate,
    detail,
  };
}
