import words from "./data/words.json";
import { applyEdits, asciiLower, quoteRanges, wordBounds } from "./shared.ts";
import type { Range, Edit } from "./shared.ts";
interface Token {
  start: number;
  end: number;
  rawStart: number;
  rawEnd: number;
  word: string;
  protected: boolean;
}
const has = (list: string[], word: string) => list.includes(asciiLower(word));
const horizontal = (s: string) => /^[ \t]+$/.test(s);
const external = /^[.,;:!?()[\]{}]+|[.,;:!?()[\]{}]+$/g;
// Rust structured_clean.rs::tokenize.
function tokenize(source: string): Token[] {
  const ranges = quoteRanges(source);
  return wordBounds(source).map(({ text: raw, start: rawStart, end: rawEnd }) => {
    const word = raw.replace(external, "");
    const start = rawStart + (raw.match(/^[.,;:!?()[\]{}]+/)?.[0].length ?? 0);
    const prefix = source.slice(source.lastIndexOf("\n", rawStart - 1) + 1, rawStart);
    return {
      start,
      end: start + word.length,
      rawStart,
      rawEnd,
      word,
      protected:
        ranges.some(([a, b]) => a < rawEnd && b > rawStart) ||
        prefix.startsWith("    ") ||
        prefix.startsWith("\t") ||
        prefix.trimStart().startsWith(">") ||
        raw.startsWith(">"),
    };
  });
}
// Rust structured_clean.rs::adjacent.
const adjacent = (s: string, t: Token[], a: number, b: number) =>
  b === a + 1 && horizontal(s.slice(t[a]!.end, t[b]!.start));
// Rust structured_clean.rs::sentence_boundary / segment_boundary.
function boundary(s: string, t: Token[], i: number, segment = false): boolean {
  return (
    /[.!?;]/.test(s.slice(t[i]!.end, t[i]!.rawEnd)) ||
    i + 1 === t.length ||
    /[\n\r]/.test(s.slice(t[i]!.rawEnd, t[i + 1]!.rawStart)) ||
    (segment && /[,;]/.test(s.slice(t[i]!.end, t[i + 1]!.start)))
  );
}
// Rust structured_clean.rs::sentence_span / segment_span.
function enclosing(s: string, t: Token[], at: number, segment = false): Range {
  let a = at,
    b = at + 1;
  while (a > 0 && !boundary(s, t, a - 1, segment)) a--;
  while (b < t.length && !boundary(s, t, b - 1, segment)) b++;
  return [a, b];
}
// Rust structured_clean.rs::words_match.
function matches(s: string, t: Token[], start: number, ws: string[]): boolean {
  return (
    start + ws.length <= t.length &&
    ws.every(
      (w, i) =>
        !t[start + i]!.protected &&
        asciiLower(t[start + i]!.word) === w &&
        (i === 0 || adjacent(s, t, start + i - 1, start + i)),
    )
  );
}
// Rust structured_clean.rs::independent_clause.
function independent(s: string, t: Token[], start: number): boolean {
  if (t[start]!.protected || !has(["but", "and", "then"], t[start]!.word)) return false;
  let next = start + 1;
  if (matches(s, t, next, ["then"])) next++;
  if (matches(s, t, next, ["please"])) next++;
  for (const p of [["do", "not"], ["don't"], ["don’t"], ["never"]])
    if (matches(s, t, next, p)) {
      next += p.length;
      break;
    }
  return ["deploy", "compile", "build", "test", "call"].some((w) => matches(s, t, next, [w]));
}
// Rust structured_clean.rs::literal_reference.
function literalReference(t: Token[], forward: boolean): boolean {
  const any = (ws: string[]) => t.some((x) => !x.protected && has(ws, x.word));
  return (
    any(words.instruction_word) &&
    any(words.literal_word) &&
    any(
      forward
        ? ["following", "next", "below"]
        : ["those", "these", "that", "previous", "above", "it", "its", "them", "address", "addresses", "text"],
    )
  );
}
// Rust structured_clean.rs::local_literal_referent.
function localReferent(s: string, t: Token[], start: number, end: number): boolean {
  const naming = "spelled spelt spelling written pronounced named called capitalized capitalised".split(" ");
  const generic =
    "it its that this those these them the a an address addresses email emails word words phrase phrases string strings text name names spelling pronunciation sentence sentences line lines part parts one version versions everything all".split(
      " ",
    );
  for (let i = start; i + 1 < end; i++)
    if (has(["that", "this", "those", "these", "the", "previous"], t[i]!.word) && has(generic, t[i + 1]!.word))
      return false;
  for (let i = start; i + 1 < end; i++) {
    const a = t[i]!,
      b = t[i + 1]!;
    if (
      !a.protected &&
      !b.protected &&
      adjacent(s, t, i, i + 1) &&
      a.word &&
      has(naming, b.word) &&
      !has(generic, a.word) &&
      !has(words.instruction_word, a.word) &&
      !has(words.literal_word, a.word)
    )
      return true;
  }
  return false;
}
// Rust structured_clean.rs::candidate_guarded, literal_consumed, cross_sentence_literal_reference.
function guarded(s: string, t: Token[], first: number, end: number): boolean {
  const [sentenceStart, sentenceEnd] = enclosing(s, t, first);
  let left = sentenceStart,
    right = sentenceEnd;
  if (end > right) right = end;
  else {
    for (let i = left; i < first; i++) if (independent(s, t, i)) left = i;
    for (let i = end; i < right; i++)
      if (independent(s, t, i)) {
        right = i;
        break;
      }
  }
  if (t.slice(left, right).some((x) => !x.protected && has(words.uncertainty_word, x.word))) return true;
  const [segmentStart, segmentEnd] = enclosing(s, t, first, true);
  const literalToken = (i: number) => t[i]!.protected || /[^a-z]/i.test(t[i]!.word);
  for (let i = sentenceStart; i < sentenceEnd; i++) {
    if (
      t[i]!.protected ||
      !has(words.literal_word, t[i]!.word) ||
      (i > 0 && adjacent(s, t, i - 1, i) && literalToken(i - 1)) ||
      (i + 1 < t.length && adjacent(s, t, i, i + 1) && literalToken(i + 1))
    )
      continue;
    if (i >= segmentStart && i < segmentEnd) return true;
    const [a, b] = enclosing(s, t, i, true);
    if (t.slice(a, b).some((x) => !x.protected && has(words.instruction_word, x.word)) && !localReferent(s, t, a, b))
      return true;
  }
  if (sentenceStart > 0) {
    const [a] = enclosing(s, t, sentenceStart - 1);
    if (literalReference(t.slice(a, sentenceStart), true) && !localReferent(s, t, a, sentenceStart)) return true;
  }
  if (sentenceEnd < t.length) {
    const [, b] = enclosing(s, t, sentenceEnd);
    if (literalReference(t.slice(sentenceEnd, b), false) && !localReferent(s, t, sentenceEnd, b)) return true;
  }
  return false;
}
const EMAIL_CUES = [
  "email address is",
  "email address",
  "email me at",
  "email us at",
  "email him at",
  "email her at",
  "email them at",
  "email is",
  "email to",
  "email",
  "contact",
  "send it to",
];
const URL_CUES = ["go to", "visit", "open", "url is", "url", "website is", "website", "web address is"];
// Rust structured_clean.rs::cue.
function cue(s: string, t: Token[], start: number): ["email" | "url", number] | undefined {
  for (const [kind, patterns] of [
    ["email", EMAIL_CUES],
    ["url", URL_CUES],
  ] as const)
    for (const p of patterns) {
      const pattern = p.split(" "),
        next = start + pattern.length;
      if (next >= t.length || !matches(s, t, start, pattern)) continue;
      if (
        p === "contact" &&
        (has("lens lenses details information form list page number tracing".split(" "), t[next]!.word) ||
          (start > 0 &&
            adjacent(s, t, start - 1, start) &&
            has("a the my our your his her their".split(" "), t[start - 1]!.word)))
      )
        continue;
      const gap = s.slice(t[next - 1]!.end, t[next]!.start);
      if (horizontal(gap) || (gap.startsWith(":") && horizontal(gap.slice(1)))) return [kind, next];
    }
  return undefined;
}
// Rust structured_clean.rs::reserved.
function reserved(w: string): boolean {
  return [
    words.prose_boundary,
    words.literal_word,
    words.uncertainty_word,
    words.negation_word,
    words.STRUCTURAL_WORDS,
  ].some((ws) => has(ws, w));
}
// Rust structured_clean.rs::Parser (component accounting is implicit in immutable edits).
class Parser {
  readonly source: string;
  readonly tokens: Token[];
  readonly first: number;
  next: number;
  rendered = "";
  constructor(source: string, tokens: Token[], first: number) {
    this.source = source;
    this.tokens = tokens;
    this.first = first;
    this.next = first;
  }
  // Rust structured_clean.rs::Parser::peek.
  peek(): string | undefined {
    const t = this.tokens[this.next];
    return !t ||
      t.protected ||
      !t.word ||
      this.next - this.first >= 256 ||
      t.word.length > 128 ||
      !/^[a-z0-9]+$/i.test(t.word) ||
      (this.next > this.first && !adjacent(this.source, this.tokens, this.next - 1, this.next))
      ? undefined
      : t.word;
  }
  // Rust structured_clean.rs::Parser::is / take / symbol.
  is(w: string): boolean {
    const p = this.peek();
    return p !== undefined && asciiLower(p) === w;
  }
  take(replacement: string): boolean {
    if (this.peek() === undefined) return false;
    this.rendered += replacement;
    this.next++;
    return true;
  }
  symbol(spoken: string, replacement: string): boolean {
    return this.is(spoken) && this.take(replacement);
  }
  // Rust structured_clean.rs::Parser::digit.
  digit(): boolean {
    if (!this.symbol("digit", "")) return false;
    const w = this.peek();
    if (w === undefined) return false;
    const n = words.SMALL.slice(0, 10).indexOf(asciiLower(w));
    return n >= 0 ? this.take(String(n)) : /^\d$/.test(w) && this.take(w);
  }
  // Rust structured_clean.rs::Parser::atom.
  atom(lowercase: boolean): boolean {
    if (this.is("digit")) {
      if (!this.digit()) return false;
    } else {
      const w = this.peek();
      if (w === undefined || reserved(w) || !this.take(lowercase ? asciiLower(w) : w)) return false;
    }
    while (this.is("digit")) if (!this.digit()) return false;
    return true;
  }
  // Rust structured_clean.rs::Parser::separated_atom.
  separated(lowercase: boolean, separators: [string, string][]): boolean {
    if (!this.atom(lowercase)) return false;
    for (;;) {
      const sep = separators.find(([w]) => this.is(w));
      if (!sep) return true;
      this.take(sep[1]);
      if (!this.atom(lowercase)) return false;
    }
  }
  // Rust structured_clean.rs::Parser::domain.
  domain(): boolean {
    const begin = this.rendered.length;
    let labels = 0,
      last = 0;
    for (;;) {
      last = this.rendered.length;
      if (
        !this.separated(true, [
          ["hyphen", "-"],
          ["dash", "-"],
        ]) ||
        this.rendered.length - last > 63
      )
        return false;
      labels++;
      if (!this.is("dot")) break;
      this.take(".");
    }
    return labels >= 2 && this.rendered.length - begin <= 253 && words.TLDS.includes(this.rendered.slice(last));
  }
  // Rust structured_clean.rs::Parser::email.
  email(): boolean {
    const sep: [string, string][] = [
      ["dot", "."],
      ["hyphen", "-"],
      ["dash", "-"],
      ["underscore", "_"],
    ];
    if (!this.separated(false, sep)) return false;
    if (this.is("plus")) {
      this.take("+");
      if (!this.separated(false, sep)) return false;
    }
    return this.rendered.length <= 64 && this.symbol("at", "@") && this.domain();
  }
  // Rust structured_clean.rs::Parser::url_atom / url.
  urlAtom(): boolean {
    return this.separated(false, [
      ["dot", "."],
      ["hyphen", "-"],
      ["dash", "-"],
      ["underscore", "_"],
      ["plus", "+"],
    ]);
  }
  url(): boolean {
    if (this.is("http") || this.is("https")) {
      this.take(asciiLower(this.peek()!));
      if (!this.symbol("colon", ":") || !this.symbol("slash", "/") || !this.symbol("slash", "/")) return false;
    }
    if (!this.domain()) return false;
    while (this.is("slash")) {
      this.take("/");
      const w = this.peek();
      if (w !== undefined && (!reserved(w) || asciiLower(w) === "digit") && !this.urlAtom()) return false;
    }
    if (this.is("question")) {
      this.take("?");
      if (!this.symbol("mark", "")) return false;
      for (;;) {
        if (!this.urlAtom() || !this.symbol("equals", "=") || !this.urlAtom()) return false;
        if (!this.is("ampersand")) break;
        this.take("&");
      }
    }
    if (this.is("hash") || this.is("hashtag")) {
      this.take("#");
      if (!this.urlAtom()) return false;
    }
    return true;
  }
  // Rust structured_clean.rs::Parser::complete_boundary.
  complete(): boolean {
    const last = this.tokens[this.next - 1]!;
    const nextNonempty = this.tokens.slice(this.next).find((t) => t.word);
    if (nextNonempty && has(words.STRUCTURAL_WORDS, nextNonempty.word)) return false;
    if (last.end < last.rawEnd) return true;
    const next = this.tokens[this.next];
    return (
      !next ||
      /[\n\r]/.test(this.source.slice(last.rawEnd, next.rawStart)) ||
      next.rawStart < next.start ||
      has(words.prose_boundary, next.word) ||
      has(words.CONTINUATION_WORDS, next.word)
    );
  }
}
// Rust structured_clean.rs::discover.
export function normalizeAddresses(source: string): string {
  const t = tokenize(source),
    edits: Edit[] = [];
  for (let i = 0; i < t.length; ) {
    const c = cue(source, t, i);
    if (!c) {
      i++;
      continue;
    }
    const [kind, first] = c,
      p = new Parser(source, t, first);
    if (!p[kind]() || !p.complete() || guarded(source, t, i, p.next)) {
      i++;
      continue;
    }
    edits.push({
      start: t[first]!.start,
      end: t[p.next - 1]!.end,
      replacement: p.rendered,
    });
    i = p.next;
  }
  return applyEdits(source, edits);
}
// Rust structured_clean.rs::preserved_spans_checked.
export function preservedCandidates(source: string): Range[] {
  const t = tokenize(source),
    spans: Range[] = [];
  for (let i = 0; i < t.length; ) {
    const c = cue(source, t, i);
    if (!c) {
      i++;
      continue;
    }
    const [kind, first] = c,
      p = new Parser(source, t, first),
      parsed = p[kind]();
    if (parsed && (!p.complete() || guarded(source, t, i, p.next))) {
      spans.push([t[i]!.start, t[p.next - 1]!.end]);
      i = Math.max(p.next, i + 1);
      continue;
    }
    if (kind === "email" && !parsed && p.rendered.includes("@")) {
      spans.push([t[first]!.start, t[p.next - 1]!.end]);
      i = Math.max(p.next, i + 1);
      continue;
    }
    i++;
  }
  return spans;
}
