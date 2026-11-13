import * as numbers from "./numbers.ts";
import { guardModel, renderModel } from "./model-names.ts";
import { protectedTerm, renderTerms } from "./terms.ts";
import { numericSpeech, verbalize } from "./verbalize.ts";
import { tokens } from "./wf-shared.ts";
import type { BioLabel, Class, Span } from "./wf-shared.ts";
import { proposeSpans } from "./proposals.ts";
import { applyEdits, isUpper, splitWords, wordBounds } from "./shared.ts";
import type { Edit, Range } from "./shared.ts";
import { protectedRanges } from "./wf-protection.ts";
import { Document, wordRegex } from "./punctuation.ts";
import { preservedCandidates } from "./structured.ts";
export type { BioLabel, Class, Span } from "./wf-shared.ts";
export { proposeSpans, renderTerms };

// Rust written_form.rs::match_key.
function matchKey(text: string): string {
  return numbers
    .groupedDigits(tokens(text))
    .filter((t) => t !== "and")
    .map((t) => (/^-?(?:\d+|\d{1,3}(?:,\d{3})+)(?:\.\d+)?$/.test(t) ? (numericSpeech(t)[0] ?? t) : t))
    .join(" ");
}

// Rust written_form.rs::crosses_mark.
export function crossesMark(core: string): boolean {
  const digit = (w: string) => numbers.value(w) !== undefined || ["oh", "o"].includes(w) || /^\d+$/.test(w);
  const words = splitWords(core);
  for (let i = 0; i + 1 < words.length; i++) {
    const left = words[i]!;
    if (!/[,;:!?]$/.test(left)) continue;
    const a = left
      .replace(/[.,;:!?]+$/, "")
      .toLowerCase()
      .split("-")
      .at(-1)!;
    const b = words[i + 1]!.replace(/^["([“]+/, "")
      .replace(/[.,;:!?"\)\]”]+$/, "")
      .toLowerCase()
      .split("-")[0]!;
    if (digit(a) && digit(b)) return true;
  }
  return false;
}

// Rust written_form.rs::sentence_follows.
function sentenceFollows(after: string): boolean {
  const next = splitWords(after)[0];
  if (!next) return true;
  const word = next.replace(/^[^\p{Alphabetic}\p{N}]+/u, "").match(/^\p{Alphabetic}*/u)![0];
  if (!isUpper([...word][0] ?? "")) return false;
  const lower = word.toLowerCase();
  return (
    ![
      "monday",
      "tuesday",
      "wednesday",
      "thursday",
      "friday",
      "saturday",
      "sunday",
      "eastern",
      "pacific",
      "central",
      "mountain",
    ].includes(lower) &&
    numbers.month(lower) === undefined &&
    !/^[A-Z]{2,}$/.test(word)
  );
}

// Rust written_form.rs::render_span / render_with_guard.
export function renderSpan(text: string, cls: Class, before = "", after = ""): string | undefined {
  if (protectedTerm(text, before, after, cls === "MODEL")) return;
  if (!text || new TextEncoder().encode(text).length > 2048) return;
  const leading = text.match(/^[\p{White_Space}"([“]*/u)![0];
  const trailing = text.match(/[\p{White_Space}.,;:!?"\)\]”]*$/u)![0];
  const start = leading.length,
    end = text.length - trailing.length;
  if (start >= end) return;
  const core = text.slice(start, end),
    t = tokens(core);
  if (cls === "MODEL" && t.at(-1) === "extra" && tokens(after)[0] === "high") return;
  if (!t.length) return;
  if (["CARDINAL", "DECIMAL", "MONEY", "PERCENT", "MAGNITUDE", "MEASURE"].includes(cls)) {
    const bt = tokens(before),
      at = tokens(after);
    if (
      bt.at(-1) === "to" &&
      bt.length > 1 &&
      (numbers.scale(bt.at(-2)!) !== undefined || numbers.number([bt.at(-2)!]) !== undefined)
    )
      return;
    if (
      at[0] === "to" &&
      at.length > 1 &&
      numbers.number([at[1]!]) !== undefined &&
      (numbers.scale(t.at(-1)!) !== undefined || numbers.number([t.at(-1)!]) !== undefined)
    )
      return;
  }
  if (cls !== "DATE" && crossesMark(core)) return;
  const contextAfter = cls === "YEAR" && /[.,;:!?]/.test(trailing) ? "" : after;
  const candidate =
    cls === "MODEL" ? renderModel(core, before, contextAfter) : numbers.candidate(t, cls, before, contextAfter, core);
  if (candidate === undefined) return;
  const accepted =
    cls === "MODEL"
      ? guardModel(core, candidate, before, contextAfter)
      : verbalize(candidate).some((spoken) => matchKey(spoken) === matchKey(core));
  if (!accepted) return;
  let suffix = trailing;
  if (
    cls === "TIME" &&
    suffix.startsWith(".") &&
    ["a.m", "p.m"].includes(splitWords(core).at(-1)?.toLowerCase() ?? "") &&
    !sentenceFollows(after)
  ) {
    suffix = suffix.slice(1);
  }
  return leading + candidate + suffix;
}

// Rust written_form.rs::render_spans.
export function applySpans(text: string, spans: Span[], extra: Range[] = []): string {
  const words = wordBounds(text),
    edits: Edit[] = [];
  let previous = 0;
  for (const span of spans) {
    if (span.start < previous || span.start >= span.end || span.end > words.length) return text;
    previous = span.end;
  }
  const protectedSpans = [...protectedRanges(text), ...extra];
  for (const span of spans) {
    const start = words[span.start]!.start,
      end = words[span.end - 1]!.end;
    if (protectedSpans.some(([a, b]) => a < end && b > start) || /[\n\r]/.test(text.slice(start, end))) continue;
    const before = words
      .slice(Math.max(0, span.start - 3), span.start)
      .map((w) => w.text)
      .join(" ");
    const after = words
      .slice(span.end, span.end + 3)
      .map((w) => w.text)
      .join(" ");
    const replacement = renderSpan(text.slice(start, end), span.class, before, after);
    if (replacement !== undefined && replacement !== text.slice(start, end)) edits.push({ start, end, replacement });
  }
  return applyEdits(text, edits);
}

// Rust written_form.rs::apply.
export function applyWrittenForm(text: string, labels?: BioLabel[]): string {
  return applySpans(text, proposeSpans(splitWords(text), labels));
}

// Rust punctuation.rs::project_span_labels.
export function projectSpanLabels(text: string, labels: BioLabel[]): BioLabel[] | undefined {
  const result: BioLabel[] = [];
  let offset = 0;
  for (const token of splitWords(text)) {
    const count = [...token.matchAll(wordRegex())].length;
    if (!count) {
      result.push("O");
      continue;
    }
    const parts = labels.slice(offset, offset + count);
    if (parts.length !== count) return;
    const first = parts[0]!;
    const compatible = parts.slice(1).every((part) => (first === "O" ? part === "O" : part === `I-${first.slice(2)}`));
    result.push(compatible ? first : "O");
    offset += count;
  }
  return offset === labels.length ? result : undefined;
}

// Rust punctuation.rs::written_form_finish (empty vocabulary).
export function finishWrittenForm(text: string, labels?: BioLabel[]): string {
  const protectedSpans = preservedCandidates(text);
  const converted = applySpans(text, proposeSpans(splitWords(text), labels), protectedSpans);
  const terms = renderTerms(converted, preservedCandidates(converted));
  const doc = new Document(terms);
  return doc.rebuildGaps(
    doc.words.map((_, i) => doc.gap(i)),
    undefined,
    true,
  );
}
