import measurements from "./speech-measurements.json";
import { discoverable, memoryRange, type ModelEntry } from "./model-library";

// Copied from the November 6 product measurements. No leaderboard family scores.
// Memory observations use decimal GB; byte capacities are converted the same way.
type Source = string | string[];
type Score = { value: number; source: Source; note: string };
type Wait = {
  seconds: number;
  label: string;
  basis: string;
  source: Source;
};
/** Kinds of speech the Compare chart can show, each from one test run. */
export type SpeechView = "everyday" | "accented" | "talks";
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
  wait: Wait | null;
  /** One run per kind of speech (docs/research/ParakeetFineTuneDesktop.md). */
  views?: Partial<Record<SpeechView, Score>>;
  /** A wait measured through the Mac app, when the shared one is not. */
  macWait?: Wait;
  /** Meetings through the app's meeting path, pooled over public sets. */
  meetings?: Score;
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
export function waitText(id: string, mac?: boolean): string {
  const wait =
    mac === undefined ? observations[id]?.wait : platformWait(id, mac);
  return wait?.label ?? "not measured";
}
/** A wait measured through the Mac app counts only on a Mac, and the reverse. */
export function platformWait(id: string, mac: boolean): Wait | null {
  const data = observations[id];
  if (mac && data?.macWait) return data.macWait;
  const wait = data?.wait;
  if (!wait) return null;
  const fromMac = [wait.source]
    .flat()
    .some((source) => /BENCHMARK-MAC-/.test(source));
  return fromMac === mac ? wait : null;
}

/** The Mac app run that measured the main choices together (November 13). */
const MAC_RUN = "BENCHMARK-MAC-M4-2026-11-13";
const VIEW_NAMES: Record<SpeechView, string> = {
  everyday: "Everyday speech",
  accented: "Accented English",
  talks: "Talks",
};
export function viewName(view: SpeechView): string {
  return VIEW_NAMES[view];
}
/**
 * Kinds of speech with one comparable run on this platform. The Mac app has
 * measured talks only; the other views come from a Windows and Linux run.
 */
export function speechViews(mac: boolean): SpeechView[] {
  return mac ? ["talks"] : ["everyday", "accented", "talks"];
}
/** The test behind each view, named the way the footnote and details say it. */
export function viewTest(view: SpeechView, mac: boolean): string {
  if (view === "everyday")
    return "People's Speech and Common Voice spontaneous clips (5,282 words)";
  if (view === "accented") return "VoxPopuli accented English (10,675 words)";
  return mac
    ? "12 TED talks through the Mac app on an M4 (1,233 words)"
    : "12 TED talks (1,233 words)";
}
/** Pooled meeting-path word errors; Windows runs, so never shown on a Mac. */
export function meetingPathScore(id: string, mac: boolean): Score | null {
  return mac ? null : (observations[id]?.meetings ?? null);
}
/** The test's short name, for a card's details. */
export function viewTestShort(view: SpeechView, mac: boolean): string {
  if (view === "everyday") return "People's Speech, Common Voice";
  if (view === "accented") return "VoxPopuli";
  return mac ? "12 TED talks, Mac app" : "12 TED talks";
}
/** Word errors for a view, only from that view's single run. */
export function viewScore(
  id: string,
  mac: boolean,
  view: SpeechView,
): Score | null {
  const data = observations[id];
  if (mac)
    return view === "talks" &&
      [data?.mac?.source ?? []].flat().some((source) => source.includes(MAC_RUN))
      ? (data?.mac ?? null)
      : null;
  return data?.views?.[view] ?? null;
}
/**
 * A talks score from another run (another computer or test path). It is real,
 * but not comparable with the chart's run, so it is listed, not plotted.
 */
export function otherRunScore(
  id: string,
  mac: boolean,
  view: SpeechView,
): Score | null {
  if (view !== "talks" || viewScore(id, mac, view)) return null;
  return speechScore(id, mac, false);
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

/** One model on the Compare chart: measured word errors against wait. */
export type ParetoPoint = {
  id: string;
  name: string;
  errors: number;
  wait: number;
  gpu: boolean;
  downloadBytes: number | null;
};

/**
 * Models with both a word-error score for this platform and a measured wait.
 * Anything else stays off the chart and reads "not measured" in the table.
 */
export function paretoPoints(
  models: ModelEntry[],
  mac: boolean,
  view: SpeechView = mac ? "talks" : "everyday",
): ParetoPoint[] {
  return models.flatMap((model) => {
    const score = viewScore(model.id, mac, view);
    const wait = platformWait(model.id, mac)?.seconds;
    if (!score || wait == null) return [];
    return [
      {
        id: model.id,
        name: shortSpeechName(model.id, model.name),
        errors: score.value,
        wait,
        gpu: gpuModel(model),
        downloadBytes: model.diskBytes,
      },
    ];
  });
}

/**
 * Ids no other point beats on both axes (lower errors and lower wait, at
 * least one strictly), in order of increasing wait.
 */
export function paretoFrontier(points: ParetoPoint[]): string[] {
  return points
    .filter(
      (point) =>
        !points.some(
          (other) =>
            other !== point &&
            other.errors <= point.errors &&
            other.wait <= point.wait &&
            (other.errors < point.errors || other.wait < point.wait),
        ),
    )
    .sort((a, b) => a.wait - b.wait || a.errors - b.errors)
    .map((point) => point.id);
}

/** Round axis ticks for a log wait axis between the given seconds. */
export function waitTicks(min: number, max: number): number[] {
  const coarse = [0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10].filter(
    (tick) => tick >= min && tick <= max,
  );
  if (coarse.length >= 3) return coarse;
  return [0.05, 0.1, 0.15, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1, 1.5, 2, 3, 5]
    .filter((tick) => tick >= min && tick <= max)
    .filter((_, index, all) => all.length <= 6 || index % 2 === 0);
}

/** Pad a log axis so close values still separate (at least a 1.6× span). */
export function waitRange(waits: number[]): [number, number] {
  if (!waits.length) return [0.1, 1];
  let min = Math.min(...waits) / 1.15;
  let max = Math.max(...waits) * 1.15;
  const ratio = max / min;
  if (ratio < 1.6) {
    const grow = Math.sqrt(1.6 / ratio);
    min /= grow;
    max *= grow;
  }
  return [min, max];
}

function multilingual(model: Pick<ModelEntry, "id" | "languages">): boolean {
  if (model.id.endsWith(".en") || model.id.startsWith("moonshine"))
    return false;
  if (model.id.startsWith("parakeet")) return false;
  return (
    model.id.startsWith("qwen3-asr") ||
    model.languages.some((language) => /multi/i.test(language)) ||
    model.languages.length > 1
  );
}

export function languageText(
  model: Pick<ModelEntry, "id" | "languages">,
): string {
  return multilingual(model) ? "Many languages" : "English only";
}

/**
 * The one short line on a speech card (eight words or fewer). Claims follow
 * the measurements: SonoJot Parakeet is faster and smaller; Qwen3-ASR is
 * multilingual with the fewest measured word errors on prepared talks.
 */
export function speechLine(
  model: ModelEntry,
  context: { mac: boolean },
): string {
  if (model.id === SONOJOT_PARAKEET) return "English only. Fast and small.";
  if (model.id.startsWith("qwen3-asr")) {
    if (context.mac)
      return model.id.includes("q4")
        ? "Many languages. Less memory than Q8."
        : "Many languages.";
    if (model.id === "qwen3-asr-0.6b-q8-cpu")
      return "Many languages. Smaller, more word errors.";
    if (gpuModel(model))
      return "Many languages; fewer errors on prepared talks.";
    return "Many languages. Fewer errors on talks, longer wait.";
  }
  if (model.id.startsWith("parakeet")) return "English only. Quick and light.";
  if (model.id.startsWith("moonshine"))
    return "English only. Very small, more word errors.";
  return `${languageText(model)}.`;
}

/** The longer detail behind a card's information button. */
export function speechDetail(model: ModelEntry): string {
  if (model.id === SONOJOT_PARAKEET)
    return "Parakeet, tuned by SonoJot for English dictation and meetings. Faster and smaller than Qwen3-ASR.";
  if (model.id.startsWith("qwen3-asr"))
    return gpuModel(model)
      ? "Qwen3-ASR understands many languages. It runs on the graphics card."
      : "Qwen3-ASR understands many languages. It runs on the processor, so it waits longer after you stop.";
  if (model.id.startsWith("parakeet"))
    return "NVIDIA Parakeet, English only. It runs on the processor and leaves the graphics card free.";
  if (model.id.startsWith("moonshine"))
    return "A very small English model with more measured word errors.";
  if (model.id.startsWith("whisper"))
    return "OpenAI Whisper, an older model.";
  return model.guidance || model.name;
}

/** "rewrite/evaluation/x/REPORT.md" → "x/REPORT.md": enough to find it. */
export function sourceName(source: string): string {
  return source.replace(/^rewrite\//, "").replace(/^evaluation\//, "");
}

export function speechSources(id: string, mac: boolean): string[] {
  const data = observations[id];
  return [
    ...speechViews(mac).map((view) => viewScore(id, mac, view)?.source),
    speechScore(id, mac, false)?.source,
    meetingPathScore(id, mac)?.source,
    platformWait(id, mac)?.source,
    data?.ram?.source,
    data?.vram?.source,
  ]
    .flat()
    .filter((value): value is string => !!value && value.endsWith(".md"))
    .map(sourceName)
    .filter((value, index, all) => all.indexOf(value) === index);
}

/** Plain memory words for a model's details: measured peaks, else the plan. */
export function memoryText(model: ModelEntry): string {
  const part = (pool: "ram" | "vram", words: string) => {
    const memory = observations[model.id]?.[pool];
    if (memory?.measuredPeak != null)
      return `${memory.measuredPeak} GB ${words}`;
    const plan = memory?.planningRange
      ? `${memory.planningRange.join("–")} GB`
      : memoryRange(
          pool === "ram" ? model.ramMinBytes : model.vramMinBytes,
          pool === "ram" ? model.ramMaxBytes : model.vramMaxBytes,
        );
    return `about ${plan} ${words} (estimate)`;
  };
  if (/metal/i.test(model.backend))
    return `${part("ram", "of shared memory")}, measured peak`.replace(
      "(estimate), measured peak",
      "(estimate)",
    );
  if (gpuModel(model))
    return `${part("vram", "graphics memory")} and ${part("ram", "system memory")}`;
  return part("ram", "system memory");
}
