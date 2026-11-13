import measurements from "./speech-measurements.json";
import { discoverable, memoryRange, type ModelEntry } from "./model-library";

// Copied from the November 6 product measurements. No leaderboard family scores.
// Memory observations use decimal GB; byte capacities are converted the same way.
type Source = string | string[];
type Score = { value: number; source: Source; note: string };
type Memory = {
  planningRange: number[] | null;
  measuredPeak: number | null;
  source: Source;
  note?: string;
};
export type SpeechMeasurement = {
  ted: Score | null;
  mac: Score | null;
  ami: Score | null;
  ram: Memory | null;
  vram: Memory | null;
  wait: {
    seconds: number;
    label: string;
    basis: string;
    source: Source;
  } | null;
};
const observations: Record<string, SpeechMeasurement> = measurements;
/** SonoJot's fine-tune of Parakeet TDT 0.6B v2 (`ft2`). */
export const SONOJOT_PARAKEET = "parakeet-tdt-0.6b-v2-sonojot-ft2";
export function speechMeasurement(id: string): SpeechMeasurement | undefined {
  return observations[id];
}
export function speechScore(
  id: string,
  mac: boolean,
  meeting: boolean,
): Score | null {
  const data = observations[id];
  // AMI measurements were CUDA runs; never present those as Metal/CPU results.
  return (
    (meeting
      ? data?.ami
      : mac
        ? macMeasured(data?.mac)
          ? data?.mac
          : null
        : data?.ted) ?? null
  );
}
/** A Mac score counts only when a Mac benchmark report measured it (Metal or CPU). */
function macMeasured(score: Score | null | undefined): boolean {
  return [score?.source ?? []]
    .flat()
    .some((source) => /evaluation\/BENCHMARK-MAC-/.test(source));
}
export function gpuModel(model: Pick<ModelEntry, "backend">): boolean {
  return /cuda|metal|nvidia/i.test(model.backend);
}
export function primarySpeechPool(
  model: Pick<ModelEntry, "backend">,
): "ram" | "vram" {
  return gpuModel(model) ? "vram" : "ram";
}
export function secondaryMemory(model: ModelEntry): string {
  if (/metal/i.test(model.backend)) return "GPU and CPU share unified memory.";
  if (!gpuModel(model))
    return "Uses system RAM; no graphics memory allocation.";
  const peak = observations[model.id]?.ram?.measuredPeak;
  return peak == null
    ? `Also uses system RAM · ${memoryObservation(model, "ram")}`
    : `Also uses system RAM · ${peak} GB measured peak.`;
}
export function shortSpeechName(id: string, fallback: string): string {
  if (id.startsWith("qwen3-asr"))
    return `Qwen ${id.includes("0.6b") ? "0.6B" : "1.7B"} ${id.includes("q4") ? "Q4" : "Q8"}`;
  if (id === SONOJOT_PARAKEET) return "SonoJot Parakeet";
  if (id.startsWith("parakeet")) return "Parakeet";
  if (id.startsWith("granite")) return "Granite";
  if (id === "moonshine-medium") return "Moonshine";
  if (id === "whisper-small-metal") return "Whisper Small";
  return fallback;
}
export function speechChoices(
  models: ModelEntry[],
  frontier: string[],
  selected: string,
  other: string,
  all: boolean,
): ModelEntry[] {
  const visible = new Set([...frontier, selected, other]);
  return models.filter(
    (model) =>
      model.kind === "asr" &&
      model.roles.includes("asr") &&
      (visible.has(model.id) || (all && discoverable(model))),
  );
}
export function memoryObservation(
  model: ModelEntry,
  pool: "ram" | "vram",
): string {
  if (pool === "vram" && /metal/i.test(model.backend))
    return "Shared memory; counted in system RAM";
  if (pool === "vram" && !gpuModel(model))
    return "Runs on CPU; no graphics allocation";
  const memory = observations[model.id]?.[pool];
  const plan = memory?.planningRange
    ? `${memory.planningRange.join("–")} GB`
    : memoryRange(
        pool === "ram" ? model.ramMinBytes : model.vramMinBytes,
        pool === "ram" ? model.ramMaxBytes : model.vramMaxBytes,
      );
  return `${memory?.measuredPeak == null ? "Peak not measured" : `${memory.measuredPeak} GB measured peak`} · plan ${plan}`;
}
export function waitText(id: string): string {
  return observations[id]?.wait?.label ?? "not measured";
}
export function pointRadius(id: string): number {
  const seconds = observations[id]?.wait?.seconds;
  return seconds == null
    ? 4
    : Math.max(4, Math.min(9, 9 - Math.log2(1 + seconds * 4) * 2));
}
export function labelPositions(
  values: number[],
  top: number,
  bottom: number,
  gap: number,
): number[] {
  const result: number[] = [];
  const sorted = values
    .map((value, i) => ({ value, i }))
    .sort((a, b) => a.value - b.value);
  sorted.forEach((item, index) => {
    result[item.i] = Math.max(
      top,
      item.value,
      index ? result[sorted[index - 1].i] + gap : top,
    );
  });
  for (let i = sorted.length - 1; i >= 0; i--) {
    const item = sorted[i];
    result[item.i] = Math.min(
      result[item.i],
      i === sorted.length - 1 ? bottom : result[sorted[i + 1].i] - gap,
    );
  }
  return result;
}
