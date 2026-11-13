import lexicon from "./data/models.json";
import { integer, value } from "./numbers.ts";
import { splitWords } from "./shared.ts";
import { tokens } from "./wf-shared.ts";

interface Name {
  canonical: string;
  spoken: string[];
  misheard?: string[];
  ambiguous?: boolean;
  join?: string | null;
}
interface Family extends Name {
  id: string;
  version_join: string;
  version_format: string;
  tiers: Name[];
  tier_alone_anchor?: boolean;
}
type Part =
  | { kind: "version"; value: string }
  | { kind: "tier"; index: number }
  | { kind: "size"; prefix: string; value: string };
interface Parsed {
  family: number;
  short: boolean;
  parts: Part[];
  cue: string;
  effort: string;
}
const families: Family[] = lexicon.families;
const products: Name[] = lexicon.products;

// Rust written_form/models.rs::words.
function words(text: string): string[] {
  return tokens(
    text
      .replace(/[-_]/g, " ")
      .replace(/([a-zA-Z])(\p{Nd})/gu, "$1 $2")
      .replace(/(\p{Nd})([a-zA-Z])/gu, "$1 $2"),
  );
}

const aliases = new WeakMap<Name, string[][]>();
// Rust written_form/models.rs::alias_len, family_len, name_len.
function nameLength(input: string[], name: Name): number | undefined {
  let forms = aliases.get(name);
  if (!forms) {
    forms = [...name.spoken, ...(name.misheard ?? []), name.canonical].map(words);
    aliases.set(name, forms);
  }
  let best: number | undefined;
  for (const form of forms) {
    if (form.length && form.every((w, i) => input[i] === w)) best = Math.max(best ?? 0, form.length);
  }
  return best;
}

// Rust written_form/models.rs::version_number.
function versionNumber(t: string[], paired: boolean): string | undefined {
  if (!t.length || t.length > 12) return;
  if (t.length === 1 && /^\p{Nd}{1,4}(?:\.\p{Nd}{1,4})?$/u.test(t[0]!)) return t[0];
  if (t.some((w) => ["a", "minus", "negative", "thousand", "million", "billion", "trillion"].includes(w))) return;
  if (t.includes("point")) {
    if (t.filter((w) => w === "point").length !== 1) return;
    const at = t.indexOf("point");
    const groups = [t.slice(0, at), t.slice(at + 1)];
    const result: string[] = [];
    for (const [i, group] of groups.entries()) {
      if (!group.length) return;
      if (i > 0 && group.every((w) => (value(w) ?? 10n) < 10n || ["oh", "o"].includes(w) || /^\p{Nd}$/u.test(w))) {
        const digits = group.map((w) => value(w)?.toString() ?? (["oh", "o"].includes(w) ? "0" : w)).join("");
        if (digits.length > 4) return;
        result.push(digits);
      } else {
        const n = integer(group);
        if (n === undefined || n < 0n || n > 9999n) return;
        result.push(String(n));
      }
    }
    return result.join(".");
  }
  const n = integer(t);
  if (n !== undefined && n >= 0n && n <= 9999n) return String(n);
  if (paired && t.length === 2) {
    const a = integer([t[0]!]),
      b = integer([t[1]!]);
    if (a !== undefined && b !== undefined && a >= 0n && a <= 9n && b >= 0n && b <= 9n) return `${a}.${b}`;
  }
  return undefined;
}

// Rust written_form/models.rs::size_number.
function sizeNumber(t: string[]): string | undefined {
  let n = versionNumber(t, false);
  if (n === undefined && t.length >= 2) {
    const a = integer([t[0]!]),
      b = integer(t.slice(1));
    if (a !== undefined && b !== undefined && a >= 1n && a <= 9n && b >= 10n && b <= 99n) {
      n = String(a * 100n + b);
    }
  }
  return n !== undefined && n.split(".").length <= 2 && /[1-9]/.test(n) ? n : undefined;
}

// Rust written_form/models.rs::parse_parts.
function parseParts(f: Family, t: string[], parts: Part[]): boolean {
  if (!t.length) return true;
  if (parts.length >= 8) return false;
  const tiers = f.tiers
    .flatMap((tier, i) => {
      const length = tier.canonical.includes("{") ? undefined : nameLength(t, tier);
      return length === undefined ? [] : [{ length, i }];
    })
    .sort((a, b) => b.length - a.length || b.i - a.i);
  for (const { length, i } of tiers) {
    if (parts.some((p) => p.kind === "tier" && p.index === i)) continue;
    if (f.id === "gpt" && t[0] === "extra" && t[1] === "high") continue;
    if (f.id === "whisper" && f.tiers[i]!.canonical === "1") continue;
    if (!["qwen", "lfm", "whisper", "glm"].includes(f.id) && parts.some((p) => p.kind === "tier")) continue;
    if (
      !parts.some((p) => p.kind === "version") &&
      t.length > length &&
      ![
        "claude",
        "grok",
        "muse",
        "mistral",
        "devstral",
        "magistral",
        "voxtral",
        "mercury",
        "granite",
        "whisper",
        "parakeet",
      ].includes(f.id)
    )
      continue;
    parts.push({ kind: "tier", index: i });
    if (parseParts(f, t.slice(length), parts)) return true;
    parts.pop();
  }
  const acceptsSizes = f.tiers.some((tier) => tier.canonical.includes("{n}")) || ["gpt-oss", "parakeet"].includes(f.id);
  if (acceptsSizes && (parts.some((p) => p.kind === "version") || ["gpt-oss", "parakeet"].includes(f.id))) {
    const prefix = ["a", "e"].includes(t[0]!) ? t[0]!.toUpperCase() : "";
    const start = prefix ? 1 : 0;
    if (
      (!prefix || f.tiers.some((n) => n.canonical.startsWith(`${prefix}{n}`))) &&
      !parts.some((p) => p.kind === "size" && p.prefix === prefix)
    ) {
      for (let end = Math.min(t.length, start + 13) - 1; end >= start + 1; end--) {
        if (t[end] !== "b") continue;
        const n = sizeNumber(t.slice(start, end));
        if (n === undefined) continue;
        parts.push({ kind: "size", prefix, value: n });
        if (parseParts(f, t.slice(end + 1), parts)) return true;
        parts.pop();
      }
    }
  }
  if (!parts.some((p) => p.kind === "version") && f.version_format && f.id !== "gpt-oss") {
    let start = 0,
      prefix = "";
    switch (f.id) {
      case "kimi":
        if (!["k", "kay"].includes(t[0]!)) return false;
        start = 1;
        prefix = "K";
        break;
      case "deepseek":
        if (["v", "vee"].includes(t[0]!)) prefix = "V";
        else if (["r", "are"].includes(t[0]!)) prefix = "R";
        else return false;
        start = 1;
        break;
      case "minimax":
        if (!["m", "em"].includes(t[0]!)) return false;
        start = 1;
        prefix = "M";
        break;
      case "whisper":
      case "parakeet":
        if (t[0] === "v") {
          start = 1;
          prefix = "v";
        } else if (!(f.id === "whisper" && t.length === 1 && ["one", "1"].includes(t[0]!))) return false;
        break;
    }
    const paired = ["gpt", "claude", "gemini", "grok", "glm"].includes(f.id);
    for (let end = Math.min(t.length, start + 12); end >= start + 1; end--) {
      const v = versionNumber(t.slice(start, end), paired);
      if (v === undefined || (v.includes(".") && !f.version_format.includes("."))) continue;
      const n = f.id === "gemma" && t[end] === "n";
      parts.push({ kind: "version", value: `${prefix}${v}${n ? "n" : ""}` });
      if (parseParts(f, t.slice(end + Number(n)), parts)) return true;
      parts.pop();
    }
  }
  return false;
}

// Rust written_form/models.rs::effort.
function isEffort(t: string[]): boolean {
  return [
    "low",
    "medium",
    "high",
    "x high",
    "ex high",
    "xhigh",
    "extra high",
    "max",
    "maximum",
    "ultra",
    "minimal",
    "none",
    "no thinking",
  ].includes(t.join(" "));
}

// Rust written_form/models.rs::parse.
function parse(input: string): Parsed | undefined {
  let all = words(input);
  if (!all.length || all.length > 36) return;
  let company = "";
  if (all[0] === "typesafe") {
    all = all.slice(1);
    company = "TypeSafe ";
  } else if (all[0] === "type" && all[1] === "safe") {
    all = all.slice(2);
    company = "TypeSafe ";
  }
  for (const [index, f] of families.entries()) {
    if (company && f.id !== "jev") continue;
    const anchors: { length: number; short: boolean; initial?: number }[] = [];
    const n = nameLength(all, f);
    if (n !== undefined) anchors.push({ length: n, short: false });
    if (f.tier_alone_anchor)
      for (const [i, tier] of f.tiers.entries()) {
        const length = nameLength(all, tier);
        if (length !== undefined) anchors.push({ length, short: true, initial: i });
      }
    for (const { length, short, initial } of anchors) {
      const rest = all.slice(length);
      const endings = [{ end: rest.length, cue: "", effort: "" }];
      if (rest.at(-1) === "model")
        endings.push({
          end: rest.length - 1,
          cue: ` ${splitWords(input).at(-1)!}`,
          effort: "",
        });
      for (let e = 0; e < rest.length; e++) {
        const offset = Number(["on", "at", "with", "using"].includes(rest[e]!));
        if (isEffort(rest.slice(e + offset))) {
          const source = splitWords(input);
          const tail = source
            .slice(Math.max(0, source.length - (rest.length - e)))
            .join(" ")
            .replace(/\bx\s+high\b/gi, "xhigh");
          endings.push({ end: e, cue: "", effort: ` ${tail}` });
        }
      }
      for (const { end, cue, effort } of endings) {
        const parts: Part[] = initial === undefined ? [] : [{ kind: "tier", index: initial }];
        if (!parseParts(f, rest.slice(0, end), parts)) continue;
        const version = parts.some((p) => p.kind === "version");
        const tier = parts.some((p) => p.kind === "tier" || p.kind === "size");
        if (f.ambiguous && !version && !tier) continue;
        if (
          !parts.length &&
          ![...f.spoken, f.canonical].some((alias) => words(alias).join("\0") === all.slice(0, length).join("\0"))
        )
          continue;
        if (["kimi", "deepseek", "minimax"].includes(f.id) && !version) continue;
        if (f.id === "gpt-oss" && !parts.some((p) => p.kind === "size")) continue;
        if (short && !version) continue;
        if (parts.some((p) => p.kind === "size") && !version && !["gpt-oss", "parakeet"].includes(f.id)) continue;
        return { family: index, short, parts, cue: company + cue, effort };
      }
    }
  }
  return undefined;
}

// Rust written_form/models.rs::display.
function display(p: Parsed): string {
  const f = families[p.family]!;
  let out = p.short ? "" : f.canonical;
  const granular =
    f.id === "granite" &&
    p.parts.some(
      (part) => part.kind === "size" || (part.kind === "tier" && f.tiers[part.index]!.canonical.startsWith("H-")),
    );
  const fullParakeet = f.id === "parakeet" && p.parts.some((part) => part.kind === "size" || part.kind === "version");
  for (const [i, part] of p.parts.entries()) {
    let join: string, text: string;
    switch (part.kind) {
      case "version":
        join =
          (f.id === "whisper" && part.value.startsWith("v")) || granular || fullParakeet
            ? "-"
            : i > 0
              ? " "
              : f.version_join;
        if (f.id === "whisper" && i === 0 && !part.value.startsWith("v")) out = "whisper";
        text = part.value;
        break;
      case "tier": {
        const tier = f.tiers[part.index]!;
        join =
          f.id === "whisper" && out === "Whisper"
            ? " "
            : granular || fullParakeet || (f.id === "whisper" && out !== "Whisper")
              ? "-"
              : (tier.join ?? " ");
        text = tier.canonical;
        break;
      }
      case "size":
        join = ["qwen", "lfm", "gpt-oss"].includes(f.id) || granular || fullParakeet ? "-" : " ";
        text = `${part.prefix}${part.value}${f.id === "gpt-oss" ? "b" : "B"}`;
    }
    out += (out ? join : "") + text;
  }
  if (p.cue.startsWith("TypeSafe ")) out = `TypeSafe ${out}`;
  const cue = splitWords(p.cue).at(-1);
  if (cue?.toLowerCase() === "model") out += ` ${cue}`;
  return out + p.effort;
}

// Rust written_form/models.rs::product.
function product(input: string, context: string): string | undefined {
  const w = words(input);
  for (const name of products) {
    if (name.ambiguous) continue;
    if (
      name.canonical === "Claude Code" &&
      w[0] === "cloud" &&
      !words(context).some((s) => ["agent", "cli", "coding"].includes(s))
    )
      continue;
    if (nameLength(w, name) === w.length) return name.canonical;
  }
  return undefined;
}

// Rust written_form/models.rs::render.
export function renderModel(input: string, before: string, after: string): string | undefined {
  const exact = product(input, `${before} ${after}`);
  if (exact !== undefined) return exact;
  const parsed = parse(input);
  return parsed ? display(parsed) : undefined;
}

// Rust written_form/models.rs::guard.
export function guardModel(original: string, candidate: string, before: string, after: string): boolean {
  const context = `${before} ${after}`,
    a = product(original, context),
    b = product(candidate, context);
  if (a !== undefined && b !== undefined) return a === b;
  const left = parse(original),
    right = parse(candidate);
  if (!left || !right) return false;
  left.effort = left.effort.toLowerCase();
  right.effort = right.effort.toLowerCase();
  return JSON.stringify(left) === JSON.stringify(right);
}

// Rust written_form/models.rs::anchor.
export function modelAnchor(input: string): boolean {
  const w = words(input);
  return (
    families.some(
      (f) =>
        nameLength(w, f) !== undefined || (f.tier_alone_anchor && f.tiers.some((t) => nameLength(w, t) !== undefined)),
    ) ||
    products.some((p) => nameLength(w, p) !== undefined) ||
    w[0] === "typesafe"
  );
}
