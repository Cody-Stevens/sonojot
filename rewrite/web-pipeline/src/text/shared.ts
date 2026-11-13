export type Range = [number, number];
export interface Edit {
  start: number;
  end: number;
  replacement: string;
}

// Rust cleanup.rs::apply_edits_with_budget (UTF-16 offsets remain internal).
export function applyEdits(text: string, edits: Edit[]): string {
  let out = "",
    cursor = 0;
  for (const e of edits.sort((a, b) => a.start - b.start || a.end - b.end)) {
    if (e.start < cursor || e.end < e.start || e.end > text.length) throw new Error("Overlapping text edits");
    out += text.slice(cursor, e.start) + e.replacement;
    cursor = e.end;
  }
  return out + text.slice(cursor);
}

// Rust char::is_whitespace; JS \s also includes BOM and omits NEL.
export const SPACE = /\p{White_Space}/u;
export const splitWords = (s: string): string[] => s.split(/\p{White_Space}+/u).filter(Boolean);
export const trimSpace = (s: string): string => s.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
export const isAlpha = (s: string): boolean => /^\p{Alphabetic}+$/u.test(s);
export const isNumeric = (s: string): boolean => /^\p{N}+$/u.test(s);
export const isUpper = (s: string): boolean => /^\p{Uppercase}$/u.test(s);
export const asciiLower = (s: string): string => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

// Rust punctuation.rs::uppercase_first.
export function capitalize(s: string): string {
  const first = [...s][0];
  return first ? first.toUpperCase() + s.slice(first.length) : s;
}

// Rust punctuation.rs::check; callers must never publish an aborted result.
export function check(signal?: AbortSignal): void {
  signal?.throwIfAborted();
}

// Rust str::split_whitespace with positions for immutable-source edits.
export function wordBounds(s: string): { text: string; start: number; end: number }[] {
  return [...s.matchAll(/[^\p{White_Space}]+/gu)].map((m) => ({
    text: m[0],
    start: m.index,
    end: m.index + m[0].length,
  }));
}

// Rust structured_clean.rs::protected_ranges.
export function quoteRanges(source: string): Range[] {
  const chars = [...source];
  const offsets: number[] = [];
  let pos = 0;
  for (const c of chars) {
    offsets.push(pos);
    pos += c.length;
  }
  const ranges: Range[] = [];
  let previous: string | undefined;
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i]!;
    const closer =
      c === '"' || c === "`"
        ? c
        : c === "“"
          ? "”"
          : c === "‘"
            ? "’"
            : c === "'" && !(previous && /^[\p{Alphabetic}\p{N}]$/u.test(previous))
              ? "'"
              : undefined;
    if (!closer) {
      previous = c;
      continue;
    }
    const start = offsets[i]!;
    let count = 1;
    if (c === "`")
      while (chars[i + 1] === "`") {
        i++;
        count++;
      }
    let end = source.length,
      escaped = false;
    while (++i < chars.length) {
      const next = chars[i]!;
      if (c !== "`" && !escaped && next === "\\") {
        escaped = true;
        continue;
      }
      if (next === closer && !escaped) {
        const closeStart = offsets[i]!;
        let closing = 1;
        if (c === "`")
          while (chars[i + 1] === "`") {
            i++;
            closing++;
          }
        if (closing === count) {
          end = closeStart + closer.length * closing;
          break;
        }
      }
      escaped = false;
    }
    ranges.push([start, end]);
    previous = closer;
  }
  return ranges;
}
