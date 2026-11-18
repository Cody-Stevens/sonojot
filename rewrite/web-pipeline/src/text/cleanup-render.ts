// Rust punctuation/cleanup_render.rs: Clean mode with a package that has a delete head
// (punct-v4). Words the model marks for deletion are removed, their punctuation moves to
// the kept neighbour, and the text is rebuilt from the kept words. A gap with literal
// content ("%", emoji, backticks, a combining accent) is kept as render_gap keeps it.
import { capitalize, isUpper, SPACE, trimSpace } from "./shared.ts";
import { commonLowercase, literalGap } from "./punctuation.ts";
import type { Document, Mark } from "./punctuation.ts";
import { capitalRun, modelCase, sentenceStartForm } from "./casing.ts";

const DELETE_INTRO = new Set(
  "again alright also anyway finally first hello hey hi honestly however no nope now oh ok okay please right second so sure thanks third well yeah yep yes".split(" "),
);
const DELETE_CONTINUES = new Set(
  "a about also am an and any are as at be because been being both but by can could each either every for from get going gonna he he's her his i i'll i'm if in into is it's its just let's like make may might more most must my need neither no nor not of on onto or our really shall she she's should so some such than that that's the their then there's they they're to trying unless until very wanna want was we we'll we're were when where whether which while who whose will with would you'll you're your".split(" "),
);
const DELETE_CORRECTION_MARKERS = new Set("actually mean no oops rather sorry wait".split(" "));
const DELETE_OPENERS = new Set(
  "alright also anyway anyways besides btw hey meanwhile oh ok okay separately thanks".split(" "),
);
const DELETE_CLAUSE_START = new Set(
  "and because but he he's i i'd i'll i'm i've if it it'll it's or she she's so that's then there there's these they they'll they're they've this those we we'll we're we've when where which who you you'll you're you've".split(" "),
);
/** The word lists, for the test that compares them with the Rust source. */
export const RENDER_WORD_LISTS = { DELETE_INTRO, DELETE_CONTINUES, DELETE_CORRECTION_MARKERS, DELETE_OPENERS, DELETE_CLAUSE_START };
const I_FORMS = new Set(["i", "i'm", "i'll", "i've", "i'd"]);
const PAIRS = new Set(["you know", "i mean", "sort of", "kind of"]);

const terminal = (m: Mark) => m === "." || m === "?";
// Rust punctuation.rs::editable_punctuation.
const editable = (c: string) => ".,?!;:…-–—".includes(c);

/** Rust cleanup_render.rs::render_clean. Returns the text and which words were kept. */
export function renderClean(doc: Document, marks: readonly Mark[], deleteIn: readonly boolean[]): { text: string; kept: boolean[] } {
  const words = doc.words,
    n = words.length,
    text = doc.text;
  if (marks.length !== n || deleteIn.length !== n) throw new Error("Invalid delete output");
  // Protected literals and vocabulary are retained, then glued words are kept together.
  const del = words.map((w, i) => !w.protected && deleteIn[i]!);
  let start = 0;
  for (let end = 1; end <= n; end++) {
    if (end === n || SPACE.test(text.slice(words[end - 1]!.end, words[end]!.start))) {
      const full = del.slice(start, end).every(Boolean);
      del.fill(full, start, end);
      start = end;
    }
  }
  const lower = words.map((w) => w.text.toLowerCase().replaceAll("’", "'"));
  for (let i = 0; i + 1 < n; i++)
    if (PAIRS.has(`${lower[i]} ${lower[i + 1]}`) && del[i] !== del[i + 1]) {
      del[i] = false;
      del[i + 1] = false;
    }
  const kept = del.map((d) => !d);
  const indices = kept.flatMap((k, i) => (k ? [i] : []));
  if (!indices.length) return { text: "", kept };
  const labels = [...marks];
  for (let i = 0; i < n; ) {
    if (!del[i]) {
      i++;
      continue;
    }
    let j = i;
    while (j + 1 < n && del[j + 1]) j++;
    if (i > 0) {
      const p = i - 1,
        next = j + 1,
        m = labels[j]!;
      if (next === n) {
        if (m === "?") labels[p] = m;
      } else if (!terminal(labels[p]!)) {
        if (terminal(m)) {
          const keepBreak =
            !lower.slice(i, j + 1).some((w) => DELETE_CORRECTION_MARKERS.has(w)) &&
            lower[next] !== lower[i] &&
            DELETE_OPENERS.has(lower[next]!) &&
            !DELETE_CONTINUES.has(lower[p]!);
          if (labels[p] === "," || labels[p] === ";" || (labels[p] === "" && keepBreak)) labels[p] = m;
        } else if (labels[p] === "," && !DELETE_INTRO.has(lower[p]!) && !DELETE_CLAUSE_START.has(lower[next]!)) {
          labels[p] = "";
        }
      }
    }
    i = j + 1;
  }
  const names = new Set(
    words
      .filter((w, i) => i > 0 && isUpper([...w.text][0] ?? "") && !/[.?!:"“]/.test(text.slice(words[i - 1]!.end, w.start)))
      .map((w) => w.text),
  );
  const texts = words.map((w) => w.text);
  let out = text.slice(0, words[0]!.start);
  let cap = true;
  // After an abbreviation's own period ("etc.", "U.S.", "p.m.") the next word keeps
  // the recognizer's casing, as in rebuild_gaps.
  let keepCase = false;
  for (let k = 0; k < indices.length; k++) {
    const i = indices[k]!,
      word = words[i]!;
    let w = word.text;
    const iForm = I_FORMS.has(lower[i]!);
    if (!word.protected && doc.casing && !iForm && cap) {
      w = sentenceStartForm(w) ?? capitalize(w);
    } else if (!word.protected && !iForm && doc.casing) {
      // The casing label decides; the word lists below are the fallback for packages without one.
      const positional =
        !keepCase && ((doc.casing.bounds[i] ?? false) || (i > 0 && /[.?!]/.test(text.slice(words[i - 1]!.end, word.start))));
      const breaks = texts.map((_, x) => x > 0 && /[.?!,;:]/.test(doc.gap(x - 1)));
      const run = capitalRun(texts, breaks, i);
      w = modelCase(w, doc.casing.probs[i]!, positional, run, doc.casing.thresholds) ?? w;
    } else if (!word.protected) {
      if (iForm || cap) w = capitalize(w);
      // Same rule as rebuild_gaps: never lowercase an unfamiliar title-cased word
      // just because a sentence end was removed.
      else if (
        !keepCase &&
        !names.has(word.text) &&
        ![...w].slice(1).some(isUpper) &&
        commonLowercase(lower[i]!) &&
        !doc.proper.has(lower[i]!)
      )
        w = w.toLowerCase();
    }
    const gap = i + 1 < n ? text.slice(word.end, words[i + 1]!.start) : text.slice(word.end);
    const last = k + 1 === indices.length;
    const previous = i > 0 ? text.slice(words[i - 1]!.end, word.start) : undefined;
    // Literal content after a deleted word: the pieces standing on their own and the piece
    // glued to this word stay; the deleted word's own piece ("%" after a deleted "20") went with it.
    if (previous !== undefined && literalGap(previous) && !(k > 0 && indices[k - 1] === i - 1)) {
      const [, free, glued] = literalPieces(previous, false);
      for (const piece of free) out += piece + " ";
      out += glued;
    }
    const literal = literalGap(gap),
      spaced = SPACE.test(gap);
    // A gap inside a protected span (`keep as spoken`) is never edited, as rebuilt_gap leaves it.
    const insideSpan =
      i + 1 < n && doc.preserved.some(([a, b]) => a <= word.end && b > word.end && b >= words[i + 1]!.start);
    if (!last && indices[k + 1] === i + 1 && (literal || insideSpan || !spaced)) {
      // Glued words stay together, and literal content between two kept words is written
      // as it was, as render_gap leaves it on the rules path.
      out += w + gap;
      cap = (literal || insideSpan) && spaced && /[.?!]/.test(gap);
      keepCase = false;
      continue;
    }
    // An opening quote, bracket or leading apostrophe ("'cause") before this word stays in front of it.
    if (previous !== undefined && !literalGap(previous)) out += gapSides(previous)[2];
    const sides = gapSides(gap);
    const [before, after, free] = literal ? literalSides(gap, i + 1 === n) : [sides[0], sides[1], [] as string[]];
    out += w;
    // An abbreviation keeps its own period whatever the label says, as rebuilt_gap keeps it.
    const abbreviation = !last && doc.abbreviation(i);
    const finalGap = trimSpace(text.slice(words[n - 1]!.end));
    if (last && !terminal(labels[i]!) && i + 1 === n && finalGap) out += finalGap;
    else {
      out += before + (abbreviation || (last && !terminal(labels[i]!)) ? "." : labels[i]!) + after;
      for (const piece of free) out += " " + piece;
    }
    if (!last) out += " ";
    // The models label an abbreviation's own period "." too; it ends the sentence
    // only on the same evidence the rules path uses.
    cap = abbreviation ? doc.abbreviationEnds(i, gap, labels[i]) : terminal(labels[i]!);
    keepCase = abbreviation && !cap;
  }
  return { text: houseStyle(out), kept };
}

/**
 * Rust cleanup_render.rs::gap_sides. Quotes, brackets and apostrophes in a gap, by the
 * side they attach to: before the mark, after it, and in front of the next word.
 */
function gapSides(gap: string): [string, string, string] {
  const at = gap.search(SPACE),
    split = at < 0 ? gap.length : at;
  let before = "",
    after = "",
    opening = "",
    saw = false;
  for (const c of gap.slice(0, split)) {
    if (editable(c)) saw = true;
    else if (`"'“”‘’()[]{}`.includes(c)) {
      if (saw) after += c;
      else before += c;
    }
  }
  for (const c of gap.slice(split)) {
    if (editable(c)) saw = true;
    else if (")]}”".includes(c)) {
      if (saw) after += c;
      else before += c;
    } else if (`"'“‘’([{`.includes(c)) opening += c;
  }
  return [before, after, opening];
}

/**
 * Rust cleanup_render.rs::literal_pieces. A literal gap's pieces between whitespace: the piece
 * glued to the word before it, the pieces standing on their own, and the piece glued to the
 * word after it (none at the end of the text).
 */
function literalPieces(gap: string, end: boolean): [string, string[], string] {
  const pieces = gap.split(/\p{White_Space}/u).filter(Boolean);
  const before = !/^\p{White_Space}/u.test(gap) && pieces.length ? pieces.shift()! : "";
  const after = !end && !/\p{White_Space}$/u.test(gap) ? (pieces.pop() ?? "") : "";
  return [before, pieces, after];
}

/**
 * Rust cleanup_render.rs::literal_sides. A literal gap after a kept word whose next word is
 * deleted (or after the last word): the piece glued to the kept word stays, split around its
 * editable punctuation as render_gap splits it, and the free-standing pieces follow.
 */
function literalSides(gap: string, end: boolean): [string, string, string[]] {
  const [glued, free] = literalPieces(gap, end);
  let before = "",
    after = "",
    saw = false;
  for (const c of glued) {
    if (editable(c)) saw = true;
    else if (saw) after += c;
    else before += c;
  }
  return [before, after, free];
}

/** Rust cleanup_render.rs::cleanup_house_style. */
function houseStyle(text: string): string {
  const chars = [...text];
  const dashes = chars
    .map((c, i) =>
      c === "–" && !(i > 0 && i + 1 < chars.length && /^[0-9]$/.test(chars[i - 1]!) && /^[0-9]$/.test(chars[i + 1]!))
        ? "—"
        : c,
    )
    .join("");
  return dashes
    .replace(/\p{White_Space}*(?:—|--)\p{White_Space}*/gu, ", ")
    .replace(/,\p{White_Space}*,/gu, ",")
    .replace(/,\p{White_Space}*([.?!;:])/gu, "$1")
    .replace(/([.?!;:])\p{White_Space}*,/gu, "$1")
    .replace(/^\p{White_Space}*,\p{White_Space}*/u, "");
}
