// Rust punctuation/casing.rs: per-word casing from a package with a casing label.
// Same as punctuation.ts::wordRegex (Rust word_regex); local to avoid an import cycle.
const wordRegex = () => /[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*/gu;

export const CASE_LABELS = ["lower", "Capitalised", "UPPER"];
export const CASE_HINT_ENCODING =
  "Every subword token of a word carries 1 + 2 * (hint - 1) + piece_start, where hint is the recognizer's case of the word (1 unknown, 2 lower, 3 Capitalised, 4 UPPER, 5 mixed) and piece_start is 1 when the word begins a separately decoded audio piece. CLS, SEP and recognizer-mark tokens carry 0. Read case_logits at the same last-subword position as logits.";

export interface CaseThresholds {
  raise: number;
  upper: number;
  lower_positional: number;
  lower: number;
}
export interface Casing {
  probs: [number, number, number][];
  bounds: boolean[];
  thresholds: CaseThresholds;
}

// Rust casing.rs::CaseThresholds::valid.
export function validThresholds(t: unknown): t is CaseThresholds {
  if (!t || typeof t !== "object") return false;
  const r = t as Record<string, unknown>;
  const v = [r.raise, r.upper, r.lower_positional, r.lower];
  return (
    v.every((x) => typeof x === "number" && Number.isFinite(x) && x > 0.5 && x <= 1) &&
    (r.lower as number) >= (r.raise as number) &&
    Object.keys(r).length === 4
  );
}

const isLetter = (c: string) => /\p{Alphabetic}/u.test(c);
const isUpperChar = (c: string) => c !== c.toLowerCase() && c === c.toUpperCase();
const isLowerChar = (c: string) => c !== c.toUpperCase() && c === c.toLowerCase();

// 0 lower, 1 Capitalised, 2 UPPER, 3 mixed; undefined without letters.
function caseClass(word: string): number | undefined {
  const letters = [...word].filter(isLetter);
  if (!letters.length) return undefined;
  if (letters.every(isLowerChar)) return 0;
  if (letters.length >= 2 && letters.every(isUpperChar)) return 2;
  if (isUpperChar(letters[0]!) && letters.slice(1).every(isLowerChar)) return 1;
  return 3;
}

export const wordHint = (word: string) => {
  const c = caseClass(word);
  return c === undefined ? 1 : c + 2;
};
export const hintId = (word: string, pieceStart: boolean) => 1 + 2 * (wordHint(word) - 1) + (pieceStart ? 1 : 0);

// Keep in step with rewrite/src-tauri/src/punctuation_mixed_case.txt.
const MIXED = `AirDrop AirPlay AirPods ChatGPT CodePen DeepMind DeepSeek DevOps DigitalOcean DoorDash eBay ElevenLabs FaceTime
FastAPI FedEx GitHub GitLab GraphQL HubSpot iCloud iMac iMessage IntelliJ iOS iPad iPadOS iPhone iPod iTunes JavaScript
JetBrains LinkedIn MacBook macOS MongoDB MySQL NumPy OAuth OneDrive OneNote OpenAI OpenRouter PayPal PhD PlayStation
PocketBase PostgreSQL PowerPoint PowerShell PyCharm PyPI PyTorch QuickTime SharePoint SonoJot SoundCloud SpaceX SQLite
TensorFlow TikTok tvOS TypeScript visionOS watchOS WebAssembly WebGPU WebRTC WebSocket WebStorm WhatsApp WordPress WorkOS
YouTube`
  .split(/\s+/)
  .filter(Boolean);
const FORMS = new Map(MIXED.map((f) => [f.toLowerCase(), f]));
export const mixedCase = (lower: string) => FORMS.get(lower);
export const sentenceStartForm = (word: string) => FORMS.get(word.toLowerCase().replaceAll("’", "'"));

// Rust casing.rs::function_word. Keep in step with rewrite/src-tauri/src/punctuation_function_words.txt.
import functionWords from "./data/function-words.json";
const FUNCTION = new Set<string>(functionWords);
export const functionWord = (lower: string) => FUNCTION.has(lower.replaceAll("’", "'"));

const upperFirst = (w: string) => {
  const [first = "", ...rest] = [...w];
  return first.toUpperCase() + rest.join("");
};

const CONNECTORS = new Set(["a", "an", "and", "at", "de", "for", "in", "of", "on", "the", "to", "&"]);
const I_FORMS = new Set(["i", "i'm", "i'll", "i've", "i'd"]);
// Rust casing.rs::capital_run.
export function capitalRun(words: readonly string[], breaks: readonly boolean[], index: number): boolean {
  const broken = (a: number, b: number) => {
    for (let k = Math.min(a, b) + 1; k <= Math.max(a, b); k++) if (breaks[k]) return true;
    return false;
  };
  const capital = (w: string) => isUpperChar([...w][0] ?? "") && !I_FORMS.has(w.toLowerCase().replaceAll("’", "'"));
  for (const step of [-1, 1]) {
    let j = index + step,
      skipped = 0;
    while (j >= 0 && j < words.length && CONNECTORS.has(words[j]!.toLowerCase()) && skipped < 2) {
      j += step;
      skipped++;
    }
    if (j >= 0 && j < words.length && capital(words[j]!) && !broken(index, j)) return true;
  }
  return false;
}
// Rust casing.rs::model_case.
export function modelCase(word: string, p: readonly number[], positional: boolean, run: boolean, t: CaseThresholds): string | undefined {
  if (/[0-9]/.test(word)) return undefined;
  const lower = word.toLowerCase();
  const form = mixedCase(lower.replaceAll("’", "'"));
  if (form !== undefined) return form !== word && form.toLowerCase() === lower ? form : undefined;
  if (!run && caseClass(word) === 1 && [...word].length > 1 && functionWord(lower)) return lower;
  const letters = [...word].filter(isLetter).length;
  const acronym = letters >= 2 && letters <= 6 && p[2]! >= t.upper;
  switch (caseClass(word)) {
    case 0:
      return acronym ? word.toUpperCase() : p[1]! >= t.raise ? upperFirst(word) : undefined;
    case 1:
      // Rewriting a capital the recognizer chose is as strict as lowering one.
      if (acronym && p[2]! >= t.lower) return word.toUpperCase();
      // A single capital letter mid-sentence is usually a label ("option A").
      if (letters === 1 && !positional) return undefined;
      return !run && p[0]! >= (positional ? t.lower_positional : t.lower) ? lower : undefined;
    case 2:
      if (run) return undefined;
      return p[0]! >= t.lower ? lower : p[1]! >= t.lower ? upperFirst(lower) : undefined;
    default:
      return undefined;
  }
}

// Rust casing.rs::bounds_for.
export function boundsFor(words: readonly string[], pieces: readonly string[]): boolean[] {
  const source: [string, boolean][] = [];
  pieces.forEach((piece, index) =>
    [...piece.matchAll(wordRegex())].forEach((m, k) => source.push([m[0].toLowerCase(), k === 0 && index > 0])),
  );
  const out = words.map(() => false);
  let next = 0;
  words.forEach((word, i) => {
    const lower = word.toLowerCase();
    const offset = source.slice(next, next + 8).findIndex(([w]) => w === lower);
    if (offset >= 0) {
      out[i] = source[next + offset]![1];
      next += offset + 1;
    }
  });
  return out;
}

export function softmax3(row: ArrayLike<number>): [number, number, number] {
  const m = Math.max(row[0]!, row[1]!, row[2]!);
  const e = [Math.exp(row[0]! - m), Math.exp(row[1]! - m), Math.exp(row[2]! - m)];
  const s = e[0]! + e[1]! + e[2]!;
  return [e[0]! / s, e[1]! / s, e[2]! / s];
}
