// The doubtful-word overlay's view model and its pure helpers. The native
// trust layer sends a `TrustView` whenever something on screen changes; the
// overlay only draws it and reports what the user did.

export type RectF = { x: number; y: number; w: number; h: number };

export type TrustAlternative = { text: string; label: string };

export type TrustMark = {
  id: number;
  kind: "word" | "gap";
  heard: string;
  alternatives: TrustAlternative[];
  hasAudio: boolean;
  rects: RectF[];
  from: number;
  to: number;
};

export type TrustNotice = { text: string; seq: number };

export type TrustView = {
  session: number;
  mode: "inline" | "chip";
  canReplace: boolean;
  retype: boolean;
  scale: number;
  area: RectF;
  hostDark: boolean;
  markStyle: "band" | "dots";
  popoverStyle: "card" | "strip";
  playOn: "hover" | "click";
  dictationKey: string;
  hold: boolean;
  text: string;
  marks: TrustMark[];
  chip: RectF | null;
  focus: TrustNotice | null;
  voice: number | null;
  message: TrustNotice | null;
  announce: TrustNotice | null;
};

export type TrustOption =
  | { keep: false; text: string; label: string; source: string }
  | { keep: true; label: string };

/** Choices in popover order: alternatives, then keep. */
export function optionsFor(mark: TrustMark): TrustOption[] {
  const alternatives: TrustOption[] = mark.alternatives
    .slice(0, 3)
    .map((alt) => ({
      keep: false,
      text: alt.text,
      label: mark.kind === "gap" ? `Add "${alt.text}"` : alt.text,
      source: alt.label,
    }));
  return [
    ...alternatives,
    {
      keep: true,
      label: mark.kind === "gap" ? "Nothing missing" : `Keep "${mark.heard}"`,
    },
  ];
}

export function heading(mark: TrustMark): { lead: string; word: string } {
  return mark.kind === "gap"
    ? {
        lead: mark.heard
          ? "Something may be missing after"
          : "Something may be missing before the first word",
        word: mark.heard ? `“${mark.heard}”` : "",
      }
    : { lead: "Heard", word: mark.heard };
}

/** What a screen reader hears when a popover opens from the keyboard. */
export function describe(mark: TrustMark, index: number, total: number) {
  const where = `Spot ${index + 1} of ${total}`;
  if (mark.kind === "gap")
    return `${where}: something may be missing after "${mark.heard}". Down to choose, Enter to add, Escape to close.`;
  const could = mark.alternatives.length
    ? ` Could be ${mark.alternatives.map((a) => `${a.text}, ${a.label.toLowerCase()}`).join(" or ")}.`
    : "";
  return `${where}: ${mark.heard}.${could} Up and down to choose, Enter to replace, Escape to close, P to play.`;
}

export function voiceHint(key: string, hold: boolean) {
  return `${hold ? "Or hold" : "Or press"} ${key} and say it`;
}

export type Placement = { x: number; y: number; above: boolean; notch: number };

/**
 * Popover position under (or, near the bottom edge, above) the doubtful
 * word, kept inside the usable screen area. The card's notch stays on the
 * word; the strip centres on it.
 */
export function placePopover(
  anchor: RectF,
  size: { w: number; h: number },
  area: RectF,
  style: "card" | "strip",
  gap = 10,
): Placement {
  const margin = 8;
  const center = anchor.x + anchor.w / 2;
  let x = style === "card" ? center - 36 : center - size.w / 2;
  let y = anchor.y + anchor.h + gap;
  let above = false;
  if (y + size.h > area.y + area.h - margin) {
    y = anchor.y - gap - size.h;
    above = true;
  }
  if (y < area.y + margin) y = area.y + margin;
  x = Math.max(area.x + margin, Math.min(x, area.x + area.w - size.w - margin));
  const notch = Math.max(10, Math.min(size.w - 24, center - x - 7));
  return { x, y, above, notch };
}

/** The fallback chip sits just right of its anchor, or inside the area. */
export function placeChip(
  anchor: RectF,
  size: { w: number; h: number },
  area: RectF,
): { x: number; y: number } {
  const margin = 8;
  let x = anchor.x + anchor.w + 10;
  let y = anchor.y + anchor.h / 2 - size.h / 2;
  if (x + size.w > area.x + area.w - margin) x = anchor.x - size.w - 10;
  x = Math.max(area.x + margin, Math.min(x, area.x + area.w - size.w - margin));
  y = Math.max(area.y + margin, Math.min(y, area.y + area.h - size.h - margin));
  return { x, y };
}

/** The fallback panel opens above the chip, or below it when there is no room. */
export function placePanel(
  chip: RectF,
  size: { w: number; h: number },
  area: RectF,
): { x: number; y: number } {
  const margin = 8;
  let y = chip.y - size.h - 8;
  if (y < area.y + margin)
    y = Math.min(chip.y + chip.h + 8, area.y + area.h - size.h - margin);
  const x = Math.max(
    area.x + margin,
    Math.min(chip.x + chip.w - size.w, area.x + area.w - size.w - margin),
  );
  return { x, y };
}

/** Split the pasted text into plain runs and marked runs for the panel. */
export function segments(
  text: string,
  marks: TrustMark[],
): { text: string; mark: TrustMark | null }[] {
  const sorted = [...marks].sort((a, b) => a.from - b.from || a.to - b.to);
  const out: { text: string; mark: TrustMark | null }[] = [];
  let at = 0;
  for (const mark of sorted) {
    if (mark.from < at) continue;
    if (mark.from > at)
      out.push({ text: text.slice(at, mark.from), mark: null });
    out.push({ text: text.slice(mark.from, mark.to), mark });
    at = mark.to;
  }
  if (at < text.length) out.push({ text: text.slice(at), mark: null });
  return out;
}

/** Decode the native 16-bit PCM snippet into samples for Web Audio. */
export function decodePcm(base64: string): Float32Array {
  const binary = atob(base64);
  const samples = new Float32Array(binary.length >> 1);
  for (let i = 0; i < samples.length; i++) {
    const lo = binary.charCodeAt(2 * i);
    const hi = binary.charCodeAt(2 * i + 1);
    let v = (hi << 8) | lo;
    if (v >= 0x8000) v -= 0x10000;
    samples[i] = v / 32768;
  }
  return samples;
}
