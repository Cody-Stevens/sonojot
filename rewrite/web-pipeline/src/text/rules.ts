import data from "./data/words.json";
import type { CleanupMode } from "./contract.ts";
import { applyEdits, asciiLower, capitalize, isAlpha, isUpper, SPACE, wordBounds } from "./shared.ts";
import type { Edit, Range } from "./shared.ts";
import { normalizeAddresses, preservedCandidates } from "./structured.ts";
interface Token {
  start: number;
  end: number;
  coreStart: number;
  coreEnd: number;
  protected: boolean;
  literal: boolean;
}
// Rust cleanup.rs::tokens_with_budget.
function tokenize(text: string): Token[] {
  let quoted = false,
    single = false,
    code = false;
  return wordBounds(text).map(({ text: raw, start, end }) => {
    let literal = quoted || single || code,
      offset = start;
    for (const c of raw) {
      if ('"“”'.includes(c)) {
        quoted = !quoted;
        literal = true;
      }
      if ("'‘’".includes(c)) {
        const previous = [...text.slice(0, offset)].at(-1),
          next = [...text.slice(offset + c.length)][0];
        if (single && c !== "‘" && (!next || SPACE.test(next) || ".,;:!?".includes(next))) {
          single = false;
          literal = true;
        } else if (!single && c !== "’" && (!previous || SPACE.test(previous) || "(:".includes(previous))) {
          single = true;
          literal = true;
        }
      }
      if (c === "`") {
        code = !code;
        literal = true;
      }
      offset += c.length;
    }
    const core = raw.replace(/^[.…,!?;:()]+|[.…,!?;:()]+$/g, ""),
      coreStart = start + (raw.match(/^[.…,!?;:()]+/)?.[0].length ?? 0);
    return {
      start,
      end,
      coreStart,
      coreEnd: coreStart + core.length,
      literal,
      protected:
        literal ||
        !core ||
        !/^[\p{Alphabetic}'’]+$/u.test(core) ||
        core.startsWith("'") ||
        core.endsWith("'") ||
        [...core].slice(1).some(isUpper),
    };
  });
}
const word = (s: string, t: Token) => s.slice(t.coreStart, t.coreEnd);
const lower = (s: string, t: Token) => word(s, t).toLowerCase();
// Rust cleanup.rs::abbreviation, pause_suffix, pause_prose.
const abbreviation = (w: string) =>
  "mr mrs ms dr prof sr jr st vs etc".split(" ").includes(asciiLower(w)) ||
  ([...w].length === 1 && !["a", "A", "I", "i"].includes(w));
const pauseSuffix = (s: string) => /^[.…]+$/.test(s) && (s.includes("…") || s.length >= 2);
const pauseProse = (s: string, t: Token) =>
  !t.literal &&
  !!word(s, t) &&
  (/^[\p{Alphabetic}'’]+$/u.test(word(s, t)) ||
    /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$|^[+-]?(?:inf|infinity|nan)$/i.test(word(s, t)));
// Rust cleanup.rs::sentence_boundary.
function sentenceBoundary(s: string, t: Token): "none" | "external" | "quotation" {
  const raw = s.slice(t.start, t.end),
    quotation = /["”'’]$/.test(raw.replace(/[)\]}]+$/g, ""));
  const ending = raw.replace(/["”'’)\]}]+$/g, ""),
    content = ending.replace(/[.?!]+$/g, ""),
    punctuation = ending.slice(content.length);
  return content && punctuation && !pauseSuffix(punctuation) && !abbreviation(word(s, t))
    ? quotation
      ? "quotation"
      : "external"
    : "none";
}
// Rust cleanup.rs::sentence_break_between.
function sentenceBreak(s: string, t: Token[], from: number, to: number): boolean {
  for (let i = from; i < to; i++)
    if (
      /[.?!]/.test(s.slice(t[i]!.coreEnd, t[i]!.end)) ||
      (i + 1 < t.length && /[.?!\n\r]/.test(s.slice(t[i]!.end, t[i + 1]!.start)))
    )
      return true;
  return false;
}
// Rust cleanup_targeted.rs::possible_address.
function possibleAddress(s: string, t: Token[], first: number, next: number): boolean {
  let cue = false;
  for (let i = Math.max(0, first - 6); i < first; i++)
    if (["email", "address", "website", "url", "visit"].includes(lower(s, t[i]!)) && !sentenceBreak(s, t, i, first))
      cue = true;
  if (!cue) return false;
  for (let i = next; i < Math.min(next + 12, t.length); i++)
    if (["at", "dot", "slash"].includes(lower(s, t[i]!)) && !sentenceBreak(s, t, first, i)) return true;
  return false;
}
// Rust cleanup_targeted.rs::mark_repetitions.
function repetitions(s: string, t: Token[], removed: boolean[]): void {
  for (let first = 0; first < t.length; first++)
    for (const width of [2, 1]) {
      const next = first + width * 2;
      if (next >= t.length || possibleAddress(s, t, first, next)) continue;
      const span = t.slice(first, next + 1),
        ws = span.map((x) => lower(s, x));
      if (
        span.some((x) => x.literal || x.protected || !pauseProse(s, x)) ||
        span.slice(0, -1).some((x) => x.coreEnd !== x.end) ||
        span.some((x, i) => i > 0 && !/^[ \t]*$/.test(s.slice(span[i - 1]!.end, x.start)))
      )
        continue;
      if (
        span
          .slice(0, width * 2)
          .some((x, i) => word(s, x) !== ws[i] && !(first === 0 && i === 0 && word(s, x) === capitalize(ws[i]!)))
      )
        continue;
      if (
        (width === 1
          ? ws[0] === "the"
          : ["in", "on", "at", "to", "for", "of", "with", "from", "by"].includes(ws[0]!) && ws[1] === "the") &&
        ws.slice(0, width).every((w, i) => w === ws[i + width])
      ) {
        removed.fill(true, first, first + width);
        break;
      }
    }
}
// Rust cleanup_targeted.rs::numeric_phrase, ambiguous_numeric_neighbor.
const numericPhrase = (ws: string[]) =>
  (ws.length === 1 &&
    (data.SMALL.includes(ws[0]!) || data.TENS.slice(2).includes(ws[0]!) || /^\d{1,12}$/.test(ws[0]!))) ||
  (ws.length === 2 && data.TENS.slice(2).includes(ws[0]!) && data.SMALL.slice(1, 10).includes(ws[1]!));
const ambiguous = (w: string) =>
  data.SMALL.includes(w) ||
  data.TENS.slice(2).includes(w) ||
  /\d/.test(w) ||
  "minus negative plus positive - + not no never neither nor and or hundred thousand million billion point decimal between"
    .split(" ")
    .includes(w);
// Rust cleanup_targeted.rs::numeric_groups / mark_numeric_retirements.
function numericRetirements(s: string, t: Token[], preserved: Range[], removed: boolean[]): void {
  const ws = t.map((x) => lower(s, x));
  for (let marker = 1; marker < t.length; marker++) {
    const end = ["actually", "sorry", "no"].includes(ws[marker]!)
      ? marker + 1
      : ws[marker] === "or" && ws[marker + 1] === "rather"
        ? marker + 2
        : 0;
    if (
      !end ||
      end >= t.length ||
      !s.slice(t[marker - 1]!.coreEnd, t[marker]!.start).includes(",") ||
      !s.slice(t[end - 1]!.coreEnd, t[end]!.start).includes(",")
    )
      continue;
    let start = -1,
      revised = -1;
    for (let i = Math.max(0, marker - 2); i < marker; i++)
      if (numericPhrase(ws.slice(i, marker))) {
        start = i;
        break;
      }
    for (let i = Math.min(end + 2, t.length); i > end; i--)
      if (numericPhrase(ws.slice(end, i))) {
        revised = i;
        break;
      }
    if (
      start < 0 ||
      revised < 0 ||
      t.slice(start, revised).some((x) => x.literal || x.start !== x.coreStart) ||
      sentenceBreak(s, t, start, revised - 1) ||
      t.slice(start, marker - 1).some((x) => x.end !== x.coreEnd) ||
      t.slice(end, revised - 1).some((x) => x.end !== x.coreEnd)
    )
      continue;
    if (
      (start > 0 && !sentenceBreak(s, t, start - 1, start) && ambiguous(ws[start - 1]!)) ||
      (revised < t.length && !sentenceBreak(s, t, revised - 1, revised) && ambiguous(ws[revised]!))
    )
      continue;
    if (
      preserved.some(([a, b]) => t[start]!.start < b && a < t[revised - 1]!.end) ||
      possibleAddress(s, t, start, revised)
    )
      continue;
    removed.fill(true, start, end);
  }
}
// Rust cleanup.rs::removable_tokens.
function removable(s: string, t: Token[], mode: CleanupMode): boolean[] {
  const removed = t.map(
    (x, i) =>
      pauseSuffix(s.slice(x.start, x.end)) &&
      (i > 0 || i + 1 < t.length) &&
      (i === 0 || pauseProse(s, t[i - 1]!)) &&
      (i + 1 === t.length || pauseProse(s, t[i + 1]!)),
  );
  if (mode !== "clean") return removed;
  for (let i = 0; i + 2 < t.length; i++)
    if (
      !t[i]!.protected &&
      !t[i + 1]!.protected &&
      asciiLower(word(s, t[i]!)) === "the" &&
      asciiLower(word(s, t[i + 1]!)) === "the" &&
      s.slice(t[i]!.coreEnd, t[i]!.end) === "." &&
      s.slice(t[i + 1]!.coreEnd, t[i + 1]!.end) === "." &&
      !/[\r\n]/.test(s.slice(t[i]!.start, t[i + 2]!.start))
    )
      removed[i] = true;
  for (let i = 0; i < t.length; i++) {
    const x = t[i]!,
      nextFiller = i + 1 < t.length && ["um", "uh"].includes(asciiLower(word(s, t[i + 1]!)));
    const follows = i > 0 && !nextFiller && x.coreEnd === x.end && /\d/.test(word(s, t[i - 1]!));
    if (!x.protected && !follows && ["um", "uh"].includes(asciiLower(word(s, x)))) removed[i] = true;
  }
  for (let i = 0; i + 3 < t.length; i++) {
    const span = t.slice(i, i + 4);
    if (span.some((x) => x.protected) || s.slice(span[0]!.start, span[3]!.end).includes("\n")) continue;
    const ws = span.map((x) => asciiLower(word(s, x)));
    if (
      ws[0] === ws[2] &&
      ws[1] === ws[3] &&
      ["you know", "i mean", "sort of", "kind of"].includes(ws.slice(0, 2).join(" "))
    )
      removed.fill(true, i, i + 2);
  }
  return removed;
}
const TYPOS: Record<string, string> = {
  teh: "the",
  recieve: "receive",
  recieved: "received",
  seperate: "separate",
  definately: "definitely",
  occured: "occurred",
  becuase: "because",
  thier: "their",
};
// Rust cleanup.rs::safe_format_with_policy, conservative-v1; speech.rs::CaptureOwner default.
export function localRules(raw: string, mode: CleanupMode): string {
  if (mode === "original") return raw;
  const source = mode === "clean" ? normalizeAddresses(raw) : raw;
  const all = tokenize(source),
    incomplete = preservedCandidates(source),
    removed = removable(source, all, mode),
    edits: Edit[] = [];
  if (mode === "clean") {
    repetitions(source, all, removed);
    numericRetirements(source, all, incomplete, removed);
  }
  for (let i = 0; i < all.length; i++)
    if (incomplete.some(([a, b]) => all[i]!.coreStart >= a && all[i]!.coreEnd <= b)) removed[i] = false;
  const retained = all.map((_, i) => i).filter((i) => !removed[i]);
  for (let i = 0; i < all.length; ) {
    if (!removed[i]) {
      i++;
      continue;
    }
    const first = i;
    while (i < all.length && removed[i]) i++;
    edits.push({
      start: i === all.length && first > 0 ? all[first - 1]!.end : all[first]!.start,
      end: i < all.length ? all[i]!.start : all[i - 1]!.end,
      replacement: "",
    });
  }
  let boundary: "none" | "external" | "quotation" = "external";
  for (let i = 0; i < all.length; i++) {
    if (removed[i]) continue;
    const x = all[i]!,
      w = word(source, x),
      suffix = source.slice(x.coreEnd, x.end),
      protectedIncomplete = incomplete.some(([a, b]) => x.coreStart >= a && x.coreEnd <= b);
    const pause = !protectedIncomplete && pauseProse(source, x) && pauseSuffix(suffix);
    if (pause)
      edits.push({
        start: x.coreEnd,
        end: x.end,
        replacement: retained.at(-1) === i ? "." : "",
      });
    const atStart = boundary === "external",
      afterQuote = boundary === "quotation";
    boundary = pause ? "none" : sentenceBoundary(source, x);
    if (x.protected || protectedIncomplete) continue;
    let replacement = mode === "clean" ? TYPOS[w] : undefined;
    const current = replacement ?? w;
    if (atStart || ["i", "i'm", "i'll", "i've", "i'd"].includes(asciiLower(current))) replacement = capitalize(current);
    else if (
      !afterQuote &&
      data.function_word.includes(asciiLower(current)) &&
      !(i > 0 && /^[A-Z]\.$/.test(source.slice(all[i - 1]!.start, all[i - 1]!.end)))
    )
      replacement = current.toLowerCase();
    if (replacement !== undefined && replacement !== w) edits.push({ start: x.coreStart, end: x.coreEnd, replacement });
  }
  const last = retained.at(-1);
  if (last !== undefined) {
    const x = all[last]!;
    if (!x.protected && isAlpha([...source.slice(x.start, x.end)].at(-1) ?? ""))
      edits.push({ start: x.end, end: x.end, replacement: "." });
  }
  return applyEdits(source, edits);
}
