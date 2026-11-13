import table from "./data/terms.json";
import { applyEdits, isAlpha, isUpper, splitWords, wordBounds } from "./shared.ts";
import type { Edit, Range } from "./shared.ts";
import { protectedRanges } from "./wf-protection.ts";

interface Token {
  lower: string;
  original: string;
  word: number;
}
interface Term {
  written: string;
  forms: string[][];
  case: string;
  blocksModels: boolean;
}
// Rust written_form/terms.rs::tokens.
function tokens(text: string): Token[] {
  return splitWords(text).flatMap((w, word) =>
    w.split("-").flatMap((piece) => {
      const core = piece.replaceAll("’", "'").replace(/^[.,;:!?"()[\]{}“”']+|[.,;:!?"()[\]{}“”']+$/g, "");
      return core ? [{ lower: core.toLowerCase(), original: core, word }] : [];
    }),
  );
}
const terms: Term[] = table.terms.map((raw) => ({
  written: raw.written,
  forms: raw.spoken.map((s) => tokens(s).map((t) => t.lower)),
  case: raw.case ?? "name",
  blocksModels: raw.blocks_models ?? false,
}));

// Rust written_form/terms.rs::fold.
const fold = (text: string) => text.toLowerCase().replace(/[^a-z0-9]/g, "");

// Rust written_form/terms.rs::cued.
function cued(term: Term, matched: Token[], form: string[]): boolean {
  if (term.case === "any") return true;
  if (term.case === "lower") return !matched.some((t) => [...t.original].some(isUpper));
  if (form.length === 1 && form[0] === fold(term.written)) return true;
  const named = matched.filter(
    (t) =>
      isAlpha([...t.original][0] ?? "") &&
      !["and", "of", "the", "to", "in", "on", "for", "a", "an", "at", "by"].includes(t.lower),
  );
  return !!named.length && named.every((t) => isUpper([...t.original][0] ?? ""));
}

// Rust written_form/terms.rs::matches_at.
function matchesAt(seq: Token[], p: number): { n: number; term: Term }[] {
  const found: { n: number; term: Term }[] = [];
  for (const term of terms)
    for (const form of term.forms) {
      const n = form.length;
      if (
        p + n <= seq.length &&
        form.every((w, i) => seq[p + i]!.lower === w) &&
        cued(term, seq.slice(p, p + n), form)
      ) {
        found.push({ n, term });
      }
    }
  return found;
}

// Rust written_form/terms.rs::protected.
export function protectedTerm(text: string, before: string, after: string, model: boolean): boolean {
  const span = tokens(text);
  if (!span.length) return false;
  const left = tokens(before),
    low = left.length,
    high = low + span.length;
  const seq = [...left, ...span, ...tokens(after)];
  return seq.some((_, p) =>
    matchesAt(seq, p).some(({ n, term }) =>
      model ? term.blocksModels && p <= low && p + n >= high : p < high && p + n > low,
    ),
  );
}

// Rust written_form/terms.rs::render.
export function renderTerms(text: string, extra: Range[] = []): string {
  const seq = tokens(text),
    words = wordBounds(text),
    ranges = [...protectedRanges(text), ...extra];
  const edits: Edit[] = [];
  for (let p = 0; p < seq.length; ) {
    if (p > 0 && seq[p - 1]!.word === seq[p]!.word) {
      p++;
      continue;
    }
    let best: { n: number; term: Term } | undefined;
    for (const match of matchesAt(seq, p)) {
      if (seq[p + match.n]?.word === seq[p + match.n - 1]!.word) continue;
      if (!best || match.n > best.n) best = match;
    }
    if (!best) {
      p++;
      continue;
    }
    const start = words[seq[p]!.word]!.start,
      end = words[seq[p + best.n - 1]!.word]!.end;
    p += best.n;
    const region = text.slice(start, end),
      afterLead = region.replace(/^["'([{“‘]+/g, "");
    const core = afterLead.replace(/[.,;:!?"')\]}”’]+$/g, "");
    const begin = start + region.length - afterLead.length;
    if (
      !core ||
      core === best.term.written ||
      ranges.some(([a, b]) => a < end && b > start) ||
      !/^[a-z0-9 \t\-.&+']+$/i.test(core) ||
      fold(core) !== fold(best.term.written)
    )
      continue;
    edits.push({
      start: begin,
      end: begin + core.length,
      replacement: best.term.written,
    });
  }
  return applyEdits(text, edits);
}
