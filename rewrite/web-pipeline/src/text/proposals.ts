import * as numbers from "./numbers.ts";
import { modelAnchor } from "./model-names.ts";
import { tokens } from "./wf-shared.ts";
import type { BioLabel, Class, Span } from "./wf-shared.ts";
import { crossesMark, renderSpan } from "./written-form.ts";

// Rust written_form/proposals.rs::clean / numeric_start / plain_number.
const clean = (w: string) => tokens(w).join(" ");
function numericStart(w: string): boolean {
  const first = tokens(w)[0];
  return (
    first !== undefined &&
    numbers.numericWord(first) &&
    !["and", "hundred", "thousand", "million", "billion", "trillion"].includes(first)
  );
}
function plainNumber(w: string): boolean {
  const t = tokens(w);
  return !!t.length && t.every(numbers.numericWord);
}

// Rust written_form/proposals.rs::cents_end.
function centsEnd(words: string[], start: number): number | undefined {
  let numberSeen = false;
  for (let i = start; i < Math.min(words.length, start + 20); i++) {
    const word = clean(words[i]!);
    if (["cent", "cents"].includes(word)) return numberSeen ? i + 1 : undefined;
    if (!plainNumber(word)) return;
    numberSeen ||= word !== "and";
  }
  return undefined;
}

// Rust written_form/proposals.rs::suppressed.
function suppressed(words: string[], i: number): boolean {
  if (!i) return false;
  const prev = clean(words[i - 1]!);
  return (
    ["several", "few", "couple", "many", "hundreds", "thousands", "millions"].includes(prev) ||
    (plainNumber(words[i - 1]!) && !["and", "a"].includes(prev)) ||
    (prev === "and" && i >= 2 && plainNumber(words[i - 2]!))
  );
}

const MONEY = ["dollar", "dollars", "euro", "euros", "cent", "cents", "buck", "bucks", "pound", "pounds"];
// Rust written_form/proposals.rs::range_end.
function rangeEnd(words: string[], start: number): number | undefined {
  let end = start,
    range = false;
  while (end < words.length && end < start + 28) {
    const w = clean(words[end]!);
    if (w === "to") {
      if (end + 1 >= words.length || !numericStart(words[end + 1]!)) break;
      range = true;
    } else if (!plainNumber(words[end]!) && ![...MONEY, "percent"].includes(w)) break;
    end++;
  }
  return range ? end : undefined;
}

// Rust written_form/proposals.rs::RulesProposer::spans.
export function rulesSpans(words: string[]): Span[] {
  const result: Span[] = [];
  let i = 0;
  while (i < words.length) {
    const first = clean(words[i]!),
      prefix = words.slice(i, i + 5).join(" "),
      normalized = clean(prefix);
    if (
      ["cloud nine", "eleventh hour", "the fourth of july", "fourth of july"].some((s) => normalized.startsWith(s)) ||
      (first === "codex" &&
        words
          .slice(Math.max(0, i - 3), i + 12)
          .join(" ")
          .toLowerCase()
          .includes("museum"))
    ) {
      i++;
      continue;
    }
    const before = words.slice(Math.max(0, i - 3), i).join(" ");
    const model =
      modelAnchor(prefix) &&
      !(
        ["oh", "o", "0"].includes(first) &&
        i > 0 &&
        (plainNumber(words[i - 1]!) || numbers.yearCue(before) !== undefined)
      );
    const version = ["version", "v", ".net", "python"].includes(first);
    const date =
      numbers.month(first) !== undefined ||
      first === "the" ||
      numbers.ordinalWord(tokens(first).at(-1) ?? "") !== undefined;
    const numeric = numericStart(words[i]!) && !suppressed(words, i);
    const completeRange = numeric ? rangeEnd(words, i) : undefined;
    if (numeric) {
      let failedMoney: number | undefined;
      for (let ci = i + 1; ci < Math.min(words.length, i + 20); ci++) {
        if (["dollar", "dollars", "euro", "euros"].includes(clean(words[ci]!))) {
          const end = centsEnd(words, ci + 1);
          if (end !== undefined && renderSpan(words.slice(i, end).join(" "), "MONEY") === undefined) failedMoney = end;
          break;
        }
        if (!plainNumber(words[ci]!)) break;
      }
      if (failedMoney !== undefined) {
        i = failedMoney;
        continue;
      }
    }
    const amp = /^[a-z]$/i.test(first);
    if (!model && !version && !date && !numeric && !amp) {
      i++;
      continue;
    }
    const prev = i ? clean(words[i - 1]!) : "";
    const cuedYear =
      ["in", "since", "fiscal", "year"].includes(prev) ||
      numbers.month(prev) !== undefined ||
      numbers.yearCue(before) !== undefined;
    let best: Span | undefined;
    for (let end = Math.min(words.length, i + 28); end >= i + 1; end--) {
      if (completeRange !== undefined && end < completeRange) continue;
      const text = words.slice(i, end).join(" ");
      if (new TextEncoder().encode(text).length > 2048) continue;
      const last = clean(words[end - 1]!),
        after = words.slice(end, end + 3).join(" ");
      const classes: Class[] = [];
      if (model) classes.push("MODEL");
      if (version) classes.push("VERSION");
      if (date) classes.push("DATE");
      if (amp) classes.push("AMP");
      if (numeric) {
        if (MONEY.includes(last)) classes.push("MONEY");
        if (last === "percent") classes.push("PERCENT");
        if (["k", "m", "x"].includes(last)) classes.push("MAGNITUDE");
        if (numbers.unit(last) || numbers.duration(last)) classes.push("MEASURE");
        if (
          ["am", "pm"].includes(last) ||
          prev === "at" ||
          (end >= i + 2 && ["a", "p"].includes(clean(words[end - 2]!)) && last === "m")
        )
          classes.push("TIME");
        if (cuedYear) classes.push("YEAR");
        classes.push("DECIMAL", "ORDINAL", "CARDINAL");
      }
      for (const cls of classes) {
        if (numbers.month(prev) !== undefined && ["CARDINAL", "ORDINAL", "DECIMAL"].includes(cls)) continue;
        if (cls === "ORDINAL" && prev === "a" && tokens(after)[0] === "of") continue;
        if (
          cls === "MONEY" &&
          end < words.length &&
          ["dollar", "dollars", "euro", "euros"].includes(last) &&
          centsEnd(words, end) !== undefined
        )
          continue;
        if (
          end < words.length &&
          clean(words[end]!) === "and" &&
          (end + 1 === words.length || clean(words[end + 1]!) === "and") &&
          ["CARDINAL", "DECIMAL", "ORDINAL"].includes(cls)
        )
          continue;
        if (
          cls === "MEASURE" &&
          ["gig", "gigs"].includes(last) &&
          tokens(after).some((w) => ["weekend", "tonight", "band", "concert", "venue"].includes(w))
        )
          continue;
        if (
          end < words.length &&
          !crossesMark(`${words[end - 1]} ${words[end]}`) &&
          plainNumber(words[end]!) &&
          (clean(words[end]!) !== "and" || (end + 1 < words.length && numericStart(words[end + 1]!))) &&
          ["CARDINAL", "DECIMAL", "YEAR", "ORDINAL", "VERSION", "MODEL", "DATE"].includes(cls) &&
          !(["MODEL", "VERSION", "DATE"].includes(cls) && numbers.ordinalWord(clean(words[end]!)) !== undefined)
        )
          continue;
        if (renderSpan(text, cls, before, after) !== undefined) {
          best = { start: i, end, class: cls };
          break;
        }
      }
      if (best) break;
    }
    if (best) {
      i = best.end;
      result.push(best);
    } else i = completeRange ?? i + 1;
  }
  return result;
}

// Rust written_form/proposals.rs::complete_bio_spans.
export function completeBioSpans(labels: BioLabel[]): Span[] {
  const spans: Span[] = [];
  for (let i = 0; i < labels.length; ) {
    const label = labels[i]!;
    if (!label.startsWith("B-")) {
      i++;
      continue;
    }
    const cls = label.slice(2) as Class;
    let end = i + 1;
    while (labels[end] === `I-${cls}`) end++;
    spans.push({ start: i, end, class: cls });
    i = end;
  }
  return spans;
}

// Rust written_form/proposals.rs::cued_years.
function cuedYears(words: string[], spans: Span[]): void {
  for (let i = 0; i < spans.length; i++) {
    const span = spans[i]!;
    if (!["CARDINAL", "YEAR"].includes(span.class)) continue;
    const before = words.slice(Math.max(0, span.start - 3), span.start).join(" ");
    if (numbers.yearCue(before) === undefined) continue;
    const limit = spans[i + 1]?.start ?? words.length;
    for (let end = Math.min(limit, span.end + 3); end >= span.end; end--) {
      const after = words.slice(end, end + 3).join(" ");
      if (renderSpan(words.slice(span.start, end).join(" "), "YEAR", before, after) !== undefined) {
        spans[i] = { start: span.start, end, class: "YEAR" };
        break;
      }
    }
  }
}

// Rust written_form/proposals.rs::ModelProposer::spans.
export function proposeSpans(words: string[], labels?: BioLabel[]): Span[] {
  if (labels === undefined) return rulesSpans(words);
  if (words.length !== labels.length) return [];
  const model = completeBioSpans(labels),
    rules = rulesSpans(words);
  const modelCovered = Array<boolean>(words.length).fill(false);
  const modelClasses = Array<Class | undefined>(words.length).fill(undefined);
  for (const span of model) {
    modelClasses.fill(span.class, span.start, span.end);
    const before = words.slice(Math.max(0, span.start - 3), span.start).join(" ");
    const after = words.slice(span.end, span.end + 3).join(" ");
    if (renderSpan(words.slice(span.start, span.end).join(" "), span.class, before, after) !== undefined) {
      modelCovered.fill(true, span.start, span.end);
    }
  }
  const special = (cls: Class) => ["MODEL", "VERSION", "AMP"].includes(cls);
  const specialCovered = Array<boolean>(words.length).fill(false);
  let routed = rules.filter((span) => special(span.class));
  for (const span of routed) specialCovered.fill(true, span.start, span.end);
  routed.push(
    ...model.filter((span) => !special(span.class) && !specialCovered.slice(span.start, span.end).some(Boolean)),
  );
  const fallback = rules.filter(
    (span) =>
      !special(span.class) &&
      !modelCovered.slice(span.start, span.end).some(Boolean) &&
      modelClasses.slice(span.start, span.end).every((cls) => cls === undefined || cls === span.class) &&
      labels.slice(span.start, span.end).some((label) => label !== "O"),
  );
  routed = routed.filter((span) => !fallback.some((rule) => span.start < rule.end && rule.start < span.end));
  routed.push(...fallback);
  routed.sort((a, b) => a.start - b.start);
  cuedYears(words, routed);
  return routed;
}
