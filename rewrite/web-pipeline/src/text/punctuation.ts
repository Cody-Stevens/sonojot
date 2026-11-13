import lowercase from "./data/lowercase.json";
import { capitalize, isAlpha, isNumeric, isUpper, SPACE, trimSpace, wordBounds } from "./shared.ts";
import type { Range } from "./shared.ts";
import { preservedCandidates } from "./structured.ts";
export type Mark = "" | "." | "," | "?" | ";";
export interface Word {
  text: string;
  start: number;
  end: number;
  protected: boolean;
}
const LOWERCASE = new Set(lowercase);
// Rust punctuation.rs::common_lowercase.
export const commonLowercase = (w: string) => LOWERCASE.has(w.replaceAll("’", "'"));
// Rust punctuation.rs::word_regex.
export const wordRegex = () => /[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*/gu;
// Rust punctuation.rs::same_words.
export function sameWords(a: string, b: string): boolean {
  return (
    [...a.matchAll(wordRegex())].map((x) => x[0].toLowerCase()).join("\0") ===
    [...b.matchAll(wordRegex())].map((x) => x[0].toLowerCase()).join("\0")
  );
}
// Rust punctuation.rs::trim_token.
const trimToken = (s: string) =>
  s.replace(/^[\p{White_Space}.,;:?!…()[\]{}"'“”‘’—–-]+|[\p{White_Space}.,;:?!…()[\]{}"'“”‘’—–-]+$/gu, "");
// Rust punctuation.rs::prose_dash.
export const proseDash = (s: string) => /[—–]/.test(s) || s.includes("--") || (s.includes("-") && SPACE.test(s));
// Rust punctuation.rs::recognizer_cue.
export function recognizerCue(gap: string, last: boolean): string {
  if (!last && ["", ".", "-", "/", ":"].includes(gap)) return "";
  return gap.includes("?")
    ? "?"
    : /[.!]/.test(gap)
      ? "."
      : /[,;:—–]/.test(gap) || gap.includes("--") || gap.includes(" - ")
        ? ","
        : last
          ? "."
          : "";
}
// Rust punctuation.rs::literal_spans.
function literalSpans(text: string): Range[] {
  const spans: Range[] = [];
  let open: number | undefined;
  for (let i = 0; i < text.length; i++)
    if (text[i] === "`") {
      if (open !== undefined) {
        spans.push([open, i + 1]);
        open = undefined;
      } else open = i;
    }
  return spans;
}
// Rust punctuation.rs::render_gap.
export function renderGap(gap: string, prediction: Mark | undefined, terminal: boolean): string {
  if (/[^\p{White_Space}.,?!;:…\-–—"'“”‘’()[\]{}]/u.test(gap)) return gap;
  const dash = /[-–—]/.test(gap),
    ellipsis = gap.includes("..") || gap.includes("…");
  if (dash) prediction = terminal ? "." : ",";
  else if (ellipsis) prediction = prediction || (terminal ? "." : ",");
  if (prediction === undefined) return gap;
  const split = gap.search(SPACE),
    at = split < 0 ? gap.length : split;
  let before = "",
    after = "",
    rest = "",
    saw = false;
  const editable = (c: string) => ". , ? ! ; : … - – —".split(" ").includes(c);
  for (const c of gap.slice(0, at)) {
    if (editable(c)) saw = true;
    else if (saw) after += c;
    else before += c;
  }
  for (const c of gap.slice(at)) {
    if (editable(c)) saw = true;
    else if (")]}”".includes(c)) {
      if (saw) after += c;
      else before += c;
    } else if (!SPACE.test(c)) rest += c;
  }
  return `${before}${prediction}${after}${terminal ? "" : " "}${rest}`;
}
// Rust punctuation.rs::Document.
export class Document {
  readonly text: string;
  readonly words: Word[];
  readonly preserved: Range[];
  readonly proper: Set<string>;
  // Rust punctuation.rs::Document::new.
  constructor(text: string) {
    this.text = text;
    this.words = [...text.matchAll(wordRegex())].map((m) => ({
      text: m[0],
      start: m.index,
      end: m.index + m[0].length,
      protected: false,
    }));
    const ranges = [...preservedCandidates(text), ...literalSpans(text)];
    for (const t of wordBounds(text)) {
      const core = trimToken(t.text),
        offset = t.start + t.text.indexOf(core);
      const literal =
        /[/\\@_]/.test(core) ||
        core.startsWith("www.") ||
        (!!core && [...core.matchAll(wordRegex())].every((m) => isNumeric(m[0])));
      if (literal) {
        if (core) ranges.push([offset, offset + core.length]);
      } else {
        let start = 0;
        const dash = [...core.matchAll(/[—–]|--/g)].map((m) => [m.index, m.index + m[0].length] as const);
        dash.push([core.length, core.length]);
        for (const [end, next] of dash) {
          const chunk = core.slice(start, end),
            part = trimToken(chunk);
          if (wordRegex().test(part) && (!/^[\p{Alphabetic}'’]*$/u.test(part) || [...part].slice(1).some(isUpper))) {
            const begin = offset + start + chunk.indexOf(part);
            ranges.push([begin, begin + part.length]);
          }
          start = next;
        }
      }
    }
    for (let i = 1; i < this.words.length; i++) {
      const a = this.words[i - 1]!,
        b = this.words[i]!;
      if (isNumeric(a.text) && isNumeric(b.text) && /^[\p{White_Space},.:/\-–]*$/u.test(text.slice(a.end, b.start)))
        ranges.push([a.start, b.end]);
    }
    ranges.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    this.preserved = [];
    for (const [a, b] of ranges) {
      const last = this.preserved.at(-1);
      if (last && a < last[1]) last[1] = Math.max(last[1], b);
      else this.preserved.push([a, b]);
    }
    for (const w of this.words) w.protected = this.preserved.some(([a, b]) => a < w.end && b > w.start);
    for (let i = 1; i < this.words.length; i++)
      if (!SPACE.test(this.gap(i - 1)) && !proseDash(this.gap(i - 1))) {
        this.words[i - 1]!.protected = true;
        this.words[i]!.protected = true;
      }
    this.proper = new Set(
      this.words
        .slice(1)
        .filter((w, i) => isUpper([...w.text][0] ?? "") && !/[.?!:"“]/.test(text.slice(this.words[i]!.end, w.start)))
        .map((w) => w.text.toLowerCase()),
    );
  }
  // Rust punctuation.rs::Document::gap.
  gap(i: number): string {
    return this.text.slice(this.words[i]!.end, this.words[i + 1]?.start ?? this.text.length);
  }
  // Rust punctuation.rs::Document::meridiem.
  meridiem(i: number): boolean {
    return (
      i > 0 &&
      this.words[i]!.text.toLowerCase() === "m" &&
      this.gap(i - 1) === "." &&
      ["a", "p", "A", "P"].includes(this.words[i - 1]!.text)
    );
  }
  // Rust punctuation.rs::Document::abbreviation.
  abbreviation(i: number): boolean {
    const w = this.words[i]!.text;
    if (!this.gap(i).startsWith(".")) return false;
    return (
      ["etc", "vs"].includes(w.toLowerCase()) ||
      ([...w].length === 1 && w !== "I" && isUpper(w)) ||
      this.meridiem(i) ||
      (i > 0 &&
        this.gap(i - 1) === "." &&
        ["e g", "i e"].includes(`${this.words[i - 1]!.text.toLowerCase()} ${w.toLowerCase()}`))
    );
  }
  // Rust punctuation.rs::Document::next_starts_sentence.
  nextStartsSentence(i: number): boolean {
    const next = this.words[i + 1];
    return (
      !!next &&
      new TextEncoder().encode(next.text).length > 1 &&
      isUpper([...next.text][0] ?? "") &&
      !next.protected &&
      !this.proper.has(next.text.toLowerCase()) &&
      commonLowercase(next.text.toLowerCase())
    );
  }
  // Rust punctuation.rs::Document::abbreviation_ends.
  abbreviationEnds(i: number, gap: string, prediction?: Mark): boolean {
    if (this.meridiem(i)) return this.nextStartsSentence(i);
    const w = this.words[i]!.text;
    if (!/^[A-Z]$/.test(w) || !SPACE.test(gap) || (i > 0 && this.gap(i - 1) === ".")) return false;
    return prediction === "." || prediction === "?" || this.nextStartsSentence(i);
  }
  // Rust punctuation.rs::Document::end_in_gap.
  endInGap(i: number, gap: string, prediction?: Mark): boolean {
    return (
      /[?!]/.test(gap) || (gap.includes(".") && (!this.abbreviation(i) || this.abbreviationEnds(i, gap, prediction)))
    );
  }
  // Rust punctuation.rs::Document::rebuilt_gap.
  rebuiltGap(i: number, prediction?: Mark): string {
    const w = this.words[i]!,
      last = i + 1 === this.words.length,
      end = this.words[i + 1]?.start ?? this.text.length,
      gap = this.gap(i);
    if (!last && ((!SPACE.test(gap) && !proseDash(gap)) || this.abbreviation(i))) return gap;
    let beginEdit = w.end,
      endEdit = end;
    for (const [a, b] of this.preserved) {
      if (a <= w.end && b > w.end) beginEdit = Math.max(beginEdit, Math.min(b, end));
      if (a < end && b >= end) endEdit = Math.min(endEdit, Math.max(a, w.end));
    }
    if (beginEdit > endEdit) return gap;
    const editable = this.text.slice(beginEdit, endEdit);
    if (
      (!last && !SPACE.test(editable) && !proseDash(editable)) ||
      this.preserved.some(([a, b]) => a >= beginEdit && b <= endEdit && a < b)
    )
      return gap;
    const prefix = this.text.slice(w.end, beginEdit),
      suffix = this.text.slice(endEdit, end);
    let mark = last ? (prediction === "." || prediction === "?" ? prediction : undefined) : prediction;
    if (
      prediction === undefined &&
      !last &&
      !w.protected &&
      "the a an your my our their its every and or but because if than whether nor of onto"
        .split(" ")
        .includes(w.text.toLowerCase()) &&
      [...editable].filter((c) => c === ".").length === 1 &&
      !/[?!,;:]/.test(editable) &&
      recognizerCue(editable, false) === "."
    )
      mark = "";
    if (/[.?!]/.test(prefix)) mark = "";
    return prefix + renderGap(editable, mark, last) + suffix;
  }
  // Rust punctuation.rs::Document::rebuild_with_case.
  rebuild(predictions?: Mark[], houseStyle = true): string {
    if (predictions && predictions.length !== this.words.length) throw new Error("Punctuation label alignment");
    return this.rebuildGaps(
      this.words.map((_, i) => this.rebuiltGap(i, predictions?.[i])),
      predictions,
      houseStyle,
    );
  }
  // Rust punctuation.rs::Document::rebuild_gaps.
  rebuildGaps(gaps: string[], predictions?: Mark[], houseStyle = true): string {
    if (!this.words.length) return this.text;
    let out = this.text.slice(0, this.words[0]!.start);
    for (let i = 0; i < this.words.length; i++) {
      const w = this.words[i]!,
        original = w.text,
        lower = original.toLowerCase();
      const iForm = ["i", "i'm", "i'll", "i've", "i'd"].includes(lower.replaceAll("’", "'"));
      const capital = i === 0 || this.endInGap(i - 1, gaps[i - 1]!, predictions?.[i - 1]);
      const removed = i > 0 && this.endInGap(i - 1, this.gap(i - 1)) && !capital;
      const replacement = w.protected
        ? original
        : (houseStyle || predictions !== undefined) && (iForm || capital)
          ? capitalize(original)
          : removed && commonLowercase(lower) && !this.proper.has(lower) && ![...original].slice(1).some(isUpper)
            ? lower
            : original;
      out += (sameWords(original, replacement) ? replacement : original) + gaps[i]!;
    }
    return out;
  }
}
