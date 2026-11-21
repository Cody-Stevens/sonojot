// The doubtful-word overlay's view model and its pure helpers. The native
// trust layer sends a `TrustView` whenever something on screen changes; the
// overlay only draws inline marks and the popover, and reports what the user did.

export type RectF = { x: number; y: number; w: number; h: number };

export type TrustAlternative = { text: string; label: string };

export type TrustMark = {
  id: number;
  kind: "word" | "gap";
  heard: string;
  alternatives: TrustAlternative[];
  hasAudio: boolean;
  rects: RectF[];
};

export type TrustNotice = { text: string; seq: number };

export type TrustView = {
  session: number;
  scale: number;
  area: RectF;
  hostDark: boolean;
  markStyle: "band" | "dots";
  popoverStyle: "card" | "strip";
  playOn: "hover" | "click";
  dictationKey: string;
  hold: boolean;
  marks: TrustMark[];
  focus: TrustNotice | null;
  voice: number | null;
  message: TrustNotice | null;
  announce: TrustNotice | null;
  /** A dictation or meeting is recording: nothing plays by itself. */
  busy: boolean;
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
          ? "Possible missing word after"
          : "Possible missing word at the start",
        word: mark.heard ? `“${mark.heard}”` : "",
      }
    : { lead: "Unsure about", word: mark.heard };
}

/** What a screen reader hears when a popover opens from the keyboard. */
export function describe(mark: TrustMark, index: number, total: number) {
  const where = `Spot ${index + 1} of ${total}`;
  if (mark.kind === "gap")
    return `${where}: possible missing word after "${mark.heard}". Down to choose, Enter to add, Escape to close.`;
  const could = mark.alternatives.length
    ? ` Could be ${mark.alternatives.map((a) => `${a.text}, ${a.label.toLowerCase()}`).join(" or ")}.`
    : "";
  return `${where}: ${mark.heard}.${could} Up and down to choose, Enter to replace, Escape to close, P to play.`;
}

/**
 * Whether opening a popover plays the mark's audio once by itself:
 * only with "Play the audio: On hover" (the default), never while a dictation
 * or meeting records or a spoken fix is listening.
 */
export function autoplays(
  view: Pick<TrustView, "playOn" | "busy" | "voice">,
  mark: Pick<TrustMark, "hasAudio">,
): boolean {
  return (
    mark.hasAudio && view.playOn === "hover" && !view.busy && view.voice == null
  );
}

export function voiceHint(key: string, hold: boolean, hasAlternatives = true) {
  if (hasAlternatives) {
    return `${hold ? "Or hold" : "Or press"} ${key} and say the word`;
  }
  return `${hold ? "Hold" : "Press"} ${key} and say the right word, or type over it`;
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
