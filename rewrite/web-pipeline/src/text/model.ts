import { Tokenizer } from "@huggingface/tokenizers";
import type { PunctuationPackage, PunctuationRunner } from "./contract.ts";
import { check } from "./shared.ts";
import { Document, recognizerCue } from "./punctuation.ts";
import type { Mark } from "./punctuation.ts";
import type { BioLabel } from "./written-form.ts";
export interface Config {
  labels: Mark[];
  spanLabels?: BioLabel[];
  windowWords: number;
  hopWords: number;
  clsId: number;
  sepId: number;
  padId: number;
  wordPrefix: string;
  exceptions: Set<string>;
}
interface Loaded {
  config: Config;
  tokenizer: Tokenizer;
}
const loaded = new WeakMap<PunctuationPackage, Loaded>();
const MEANINGS: Record<string, Mark> = {
  none: "",
  period: ".",
  comma: ",",
  question: "?",
  semicolon: ";",
};
const CLASSES = "CARDINAL DECIMAL MONEY PERCENT MAGNITUDE YEAR DATE TIME ORDINAL MEASURE VERSION MODEL AMP".split(" ");
const LEGACY =
  "CLS, then for each word: tokens of word_prefix+word (no special tokens) and, if the recognizer text has a sentence/comma mark right after the word, the tokens of that mark; then SEP. The label of a word is read at its last word token (before any mark tokens).";
const ENCODING =
  "CLS, then for each word: tokens of word_prefix+word (no special tokens), followed by the recognizer cue from the gap after it; then SEP. Non-final gaps exactly equal to empty, dot, hyphen, slash or colon have no cue. Otherwise question wins over period/exclamation, then semicolon/comma/colon/em dash/en dash/double hyphen/spaced hyphen map to comma. A final word with no cue receives a period cue. Read each label at its last word token, before cue tokens.";
function record(v: unknown): Record<string, unknown> {
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("Invalid punctuation config");
  return v as Record<string, unknown>;
}
function ordered(v: unknown): string[] {
  const a = Array.isArray(v) ? v : Object.keys(record(v)).map((_, i) => record(v)[String(i)]);
  if (!a.every((x) => typeof x === "string")) throw new Error("Invalid label list");
  return a;
}
function positive(v: unknown): number {
  if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) throw new Error("Invalid punctuation integer");
  return v;
}
// Rust punctuation.rs::Config::parse / ConfigFile::parse_span_labels.
export function parseConfig(value: unknown): Config {
  const c = record(value),
    names = ordered(c.labels),
    meaning = record(c.label_meaning);
  const labels = names.map((n) => {
    const m = meaning[n];
    if (typeof m !== "string" || MEANINGS[m] === undefined) throw new Error("Unknown punctuation label");
    return MEANINGS[m]!;
  });
  const windowWords = positive(c.window_words),
    hopWords = positive(c.hop_words),
    clsId = positive(c.cls_id),
    sepId = positive(c.sep_id),
    padId = positive(c.pad_id);
  if (
    c.schema !== 1 ||
    typeof c.encoding !== "string" ||
    ![LEGACY, ENCODING].includes(c.encoding.trim()) ||
    c.word_prefix !== " " ||
    windowWords < 1 ||
    windowWords > 1024 ||
    hopWords < 1 ||
    hopWords > windowWords ||
    new Set([clsId, sepId, padId]).size !== 3 ||
    !labels.length ||
    labels.length > 32 ||
    new Set(names).size !== names.length ||
    !labels.includes("")
  )
    throw new Error("Unsupported punctuation configuration");
  if ("punct_output" in c && c.punct_output !== "logits") throw new Error("Invalid punctuation output");
  const exceptions = ordered(c.lowercase_words_except);
  if (exceptions.some((w) => !w || w.toLowerCase() !== w || !/^[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*$/u.test(w)))
    throw new Error("Invalid lowercase exception");
  let spanLabels: BioLabel[] | undefined;
  if ("span_labels" in c) {
    const values = ordered(c.span_labels),
      required = ["O", ...CLASSES.flatMap((c) => [`B-${c}`, `I-${c}`])];
    if (
      values.length !== required.length ||
      required.some((v) => !values.includes(v)) ||
      ("span_label_scheme" in c && c.span_label_scheme !== "BIO") ||
      ("span_output" in c && c.span_output !== "span_logits") ||
      ("span_encoding" in c &&
        c.span_encoding !== "Read span_logits at the same last-subword position as logits; independent argmax.")
    )
      throw new Error("Invalid written-form head");
    if ("span_classes" in c) {
      const classes = ordered(c.span_classes);
      if (
        classes.length !== CLASSES.length ||
        new Set(classes).size !== CLASSES.length ||
        classes.some((x) => !CLASSES.includes(x))
      )
        throw new Error("Invalid span classes");
    }
    spanLabels = values as BioLabel[];
  } else if (["span_label_scheme", "span_output", "span_encoding", "span_classes"].some((k) => k in c))
    throw new Error("Missing span labels");
  return {
    labels,
    spanLabels,
    windowWords,
    hopWords,
    clsId,
    sepId,
    padId,
    wordPrefix: " ",
    exceptions: new Set(exceptions),
  };
}
// Rust punctuation.rs::Engine::load. Runtime ownership remains with the caller.
export function loadPunctuationPackage(
  configJson: unknown,
  tokenizerJson: unknown,
  runner: PunctuationRunner,
): PunctuationPackage {
  const pkg: PunctuationPackage = {
    config: typeof configJson === "string" ? JSON.parse(configJson) : configJson,
    tokenizer: typeof tokenizerJson === "string" ? JSON.parse(tokenizerJson) : tokenizerJson,
    runner,
  };
  initialize(pkg);
  return pkg;
}
// Rust punctuation.rs::Engine::load (per-word encoding has no padding/truncation).
function initialize(pkg: PunctuationPackage): Loaded {
  const prior = loaded.get(pkg);
  if (prior) return prior;
  const config = parseConfig(pkg.config),
    raw = record(pkg.tokenizer);
  const tokenizer = new Tokenizer({ ...raw, truncation: null, padding: null }, {});
  for (const id of [config.clsId, config.sepId, config.padId])
    if (tokenizer.id_to_token(id) === undefined) throw new Error("Missing punctuation special token");
  const marks = record(pkg.config).recognizer_marks;
  if (marks !== undefined)
    for (const [mark, ids] of Object.entries(record(marks))) {
      if (
        ![".", ",", "?"].includes(mark) ||
        !Array.isArray(ids) ||
        !ids.length ||
        JSON.stringify(tokenizer.encode(mark, { add_special_tokens: false }).ids) !== JSON.stringify(ids)
      )
        throw new Error("Recognizer cue tokenizer mismatch");
    }
  const result = { config, tokenizer };
  loaded.set(pkg, result);
  return result;
}
export interface EncodedWindow {
  start: number;
  end: number;
  inputIds: number[];
  positions: number[];
}
// Rust punctuation.rs::Engine::predict / windows / encode_pieces.
export function encodeWindows(document: Document, pkg: PunctuationPackage): EncodedWindow[] {
  const { config: c, tokenizer } = initialize(pkg);
  const pieces = document.words.map((word, i) => {
    const lower = word.text.toLowerCase(),
      value = c.wordPrefix + (c.exceptions.has(lower) ? word.text : lower),
      cue = recognizerCue(document.gap(i), i + 1 === document.words.length);
    const ids = tokenizer.encode(value, { add_special_tokens: false }).ids,
      marks = cue ? tokenizer.encode(cue, { add_special_tokens: false }).ids : [];
    if (!ids.length || (cue && !marks.length)) throw new Error("Empty punctuation tokens");
    return { ids, marks };
  });
  const out: EncodedWindow[] = [];
  for (let start = 0; start < pieces.length; start += c.hopWords) {
    const end = Math.min(start + c.windowWords, pieces.length),
      ids = [c.clsId],
      positions: number[] = [];
    for (const p of pieces.slice(start, end)) {
      ids.push(...p.ids);
      positions.push(ids.length - 1);
      ids.push(...p.marks);
      if (ids.length >= 4096) throw new Error("Punctuation window exceeds token bound");
    }
    ids.push(c.sepId);
    out.push({ start, end, inputIds: ids, positions });
    if (end === pieces.length) break;
  }
  return out;
}
// Rust punctuation.rs::decode_logits.
export function decodeLogits<T>(logits: Float32Array, tokens: number, positions: number[], labels: T[]): T[] {
  if (
    !labels.length ||
    logits.length !== tokens * labels.length ||
    logits.some((x) => !Number.isFinite(x)) ||
    positions.some((p, i) => p === 0 || p >= tokens - 1 || (i > 0 && positions[i - 1]! >= p))
  )
    throw new Error("Invalid punctuation output");
  return positions.map((p) => {
    let best = 0;
    for (let i = 1; i < labels.length; i++)
      if (logits[p * labels.length + i]! > logits[p * labels.length + best]!) best = i;
    return labels[best]!;
  });
}
// Rust punctuation.rs::Engine::predict / merge_window; both heads share centrality.
export async function predict(
  document: Document,
  pkg: PunctuationPackage,
  signal?: AbortSignal,
): Promise<{ labels: Mark[]; spans?: BioLabel[] }> {
  const { config } = initialize(pkg),
    windows = encodeWindows(document, pkg),
    labels: Mark[] = [],
    spans: BioLabel[] | undefined = config.spanLabels ? [] : undefined,
    best: number[] = [];
  for (const w of windows) {
    check(signal);
    const out = await pkg.runner.run({ inputIds: w.inputIds });
    check(signal);
    const marks = decodeLogits(out.logits, w.inputIds.length, w.positions, config.labels);
    const span = config.spanLabels
      ? decodeLogits(out.spanLogits ?? new Float32Array(), w.inputIds.length, w.positions, config.spanLabels)
      : undefined;
    for (let i = 0; i < marks.length; i++) {
      const distance = Math.min(i, marks.length - 1 - i),
        at = w.start + i;
      if (best[at] === undefined || distance > best[at]!) {
        best[at] = distance;
        labels[at] = marks[i]!;
        if (spans && span) spans[at] = span[i]!;
      }
    }
  }
  return { labels, spans };
}
