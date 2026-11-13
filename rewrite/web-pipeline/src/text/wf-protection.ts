import { wordBounds } from "./shared.ts";
import type { Range } from "./shared.ts";

// Rust written_form/proposals.rs::protected_ranges (only unclosed code protects to EOF).
export function protectedRanges(text: string): Range[] {
  const ranges: Range[] = [];
  const chars: { char: string; start: number }[] = [];
  let offset = 0;
  for (const char of text) {
    chars.push({ char, start: offset });
    offset += char.length;
  }
  const alphanumeric = (s: string) => /^[\p{Alphabetic}\p{N}]$/u.test(s);
  for (let i = 0; i < chars.length; ) {
    const { char: c, start } = chars[i]!;
    const closer =
      c === "`" || c === '"'
        ? c
        : c === "“"
          ? "”"
          : c === "‘"
            ? "’"
            : c === "'" && (i === 0 || !alphanumeric(chars[i - 1]!.char))
              ? "'"
              : undefined;
    if (!closer) {
      i++;
      continue;
    }
    const fence = c === "`" && text.slice(start).startsWith("```");
    const openLength = fence ? 3 : c.length,
      tail = text.slice(start + openLength);
    let end: number | undefined;
    if (fence) {
      const at = tail.indexOf("```");
      if (at >= 0) end = start + openLength + at + 3;
    } else {
      let at = 0;
      for (const ch of tail) {
        if (ch === closer && (closer !== "'" || !alphanumeric([...tail.slice(at + 1)][0] ?? ""))) {
          end = start + openLength + at + ch.length;
          break;
        }
        at += ch.length;
      }
    }
    if (end === undefined && c === "`") end = text.length;
    if (end !== undefined) {
      ranges.push([start, end]);
      while (i < chars.length && chars[i]!.start < end) i++;
    } else i++;
  }
  for (const w of wordBounds(text)) {
    const literal =
      w.text.includes("://") ||
      w.text.startsWith("www.") ||
      /[@\\/_]/.test(w.text) ||
      /\b[a-z0-9-]+\.(?:com|org|net|io|ai|dev|edu|gov|co|app|md|rs|py|js|ts)\b/i.test(w.text);
    if (literal) ranges.push([w.start, w.end]);
  }
  return ranges;
}
