import { splitWords } from "./shared.ts";
export const CLASSES = [
  "CARDINAL",
  "DECIMAL",
  "MONEY",
  "PERCENT",
  "MAGNITUDE",
  "YEAR",
  "DATE",
  "TIME",
  "ORDINAL",
  "MEASURE",
  "VERSION",
  "MODEL",
  "AMP",
] as const;
export type Class = (typeof CLASSES)[number];
export type BioLabel = "O" | `B-${Class}` | `I-${Class}`;
export interface Span {
  start: number;
  end: number;
  class: Class;
}
// Rust written_form.rs::tokens.
export function tokens(text: string): string[] {
  text = text
    .toLowerCase()
    .replaceAll("’", "'")
    .replace(/(?<![\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d])([ap])\p{White_Space}*\.\p{White_Space}*m\.?/gu, "$1m")
    .replace(
      /(?<![\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d])([ap])\p{White_Space}+m(?![\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d])/gu,
      "$1m",
    );
  const hyphen = /([\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d])-([\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\u200c\u200d])/gu;
  text = text.replace(hyphen, "$1 $2").replace(hyphen, "$1 $2");
  return splitWords(text)
    .map((p) => p.replace(/^[,;:!?"()[\]“”]+|[,;:!?"()[\]“”]+$/g, "").replace(/\.+$/g, ""))
    .filter(Boolean);
}
