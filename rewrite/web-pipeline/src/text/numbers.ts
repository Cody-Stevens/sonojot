import data from "./data/words.json";
import { tokens } from "./wf-shared.ts";
import type { Class } from "./wf-shared.ts";
export const { SMALL, TENS, ORDINALS, MONTHS, AMP_NAMES } = data;
export const SCALES: [string, bigint][] = [
  ["thousand", 1000n],
  ["million", 1000000n],
  ["billion", 1000000000n],
  ["trillion", 1000000000000n],
];
const MAX = (1n << 127n) - 1n,
  MIN = -(1n << 127n);
const checked = (v: bigint): bigint | undefined => (v <= MAX && v >= MIN ? v : undefined);
const parse = (s: string): bigint | undefined => (/^-?\d+$/.test(s) ? checked(BigInt(s)) : undefined);
// Rust written_form/numbers.rs::unit.
export const unit = (s: string) => data.UNITS.includes(s);
// Rust written_form/numbers.rs::duration.
export const duration = (s: string) => data.DURATIONS.includes(s);
// Rust written_form/numbers.rs::month.
export const month = (s: string): number | undefined => {
  const i = MONTHS.findIndex((m) => m.toLowerCase() === s.toLowerCase());
  return i < 0 ? undefined : i;
};
// Rust written_form/numbers.rs::scale.
export const scale = (s: string): bigint | undefined => SCALES.find((x) => x[0] === s)?.[1];
// Rust written_form/numbers.rs::value.
export function value(s: string): bigint | undefined {
  const i = SMALL.indexOf(s);
  if (i >= 0) return BigInt(i);
  const j = TENS.indexOf(s);
  return j < 0 ? undefined : BigInt(j * 10);
}
// Rust written_form/numbers.rs::ordinal_word.
export function ordinalWord(s: string): string | undefined {
  const i = ORDINALS.indexOf(s);
  return i < 0
    ? undefined
    : i < 20
      ? SMALL[i]
      : i < 28
        ? TENS[i - 18]
        : ["hundred", "thousand", "million", "billion"][i - 28];
}
// Rust written_form/numbers.rs::numeric_word.
export const numericWord = (s: string) =>
  value(s) !== undefined ||
  scale(s) !== undefined ||
  ordinalWord(s) !== undefined ||
  ["hundred", "point", "oh", "o", "and", "a", "minus", "negative"].includes(s) ||
  /^\p{Nd}[\p{Nd},.]*(?:st|nd|rd|th)?$/u.test(s);
// Rust written_form/numbers.rs::small_int.
function smallInt(t: string[]): bigint | undefined {
  if (t.length >= 3 && /^[1-9]00$/.test(t[0]!) && t[1] === "and") {
    const tail = smallInt(t.slice(2));
    return tail !== undefined && tail > 0n && tail < 100n ? BigInt(t[0]!) + tail : undefined;
  }
  if (t.includes("hundred")) {
    if (t.length < 2 || t[1] !== "hundred" || t.filter((w) => w === "hundred").length !== 1) return;
    const lead = smallInt(t.slice(0, 1));
    if (lead === undefined || lead < 1n || lead > 9n) return;
    let rest = t.slice(2);
    if (rest[0] === "and") {
      rest = rest.slice(1);
      if (!rest.length) return;
    }
    const tail = rest.length ? smallInt(rest) : 0n;
    return tail !== undefined && tail < 100n ? lead * 100n + tail : undefined;
  }
  if (t.length === 1) return /^\d{1,3}$/.test(t[0]!) ? BigInt(t[0]!) : value(t[0]!);
  if (t.length === 2 && TENS.slice(2).includes(t[0]!) && SMALL.slice(1, 10).includes(t[1]!))
    return value(t[0]!)! + value(t[1]!)!;
  return undefined;
}
// Rust written_form/numbers.rs::grouped_digits.
export function groupedDigits(t: string[]): string[] {
  const result: string[] = [];
  let i = 0;
  while (i < t.length) {
    let end = i;
    while (end < t.length && /^\d+$/.test(t[end]!)) end++;
    if (end - i >= 2 && /^[1-9]\d{0,2}$/.test(t[i]!) && t.slice(i + 1, end).every((s) => s.length === 3))
      result.push(t.slice(i, end).join(""));
    else result.push(...t.slice(i, Math.max(end, i + 1)));
    i = Math.max(end, i + 1);
  }
  return result;
}
// Rust written_form/numbers.rs::integer.
export function integer(input: string[], article = false): bigint | undefined {
  if (!input.length) return;
  const t = [...input];
  if (article && t.length > 1 && t[0] === "a" && (t[1] === "hundred" || scale(t[1]!) !== undefined)) t[0] = "one";
  if (t.length === 1 && /^(?:\d+|\d{1,3}(?:,\d{3})+)$/.test(t[0]!)) return parse(t[0]!.replaceAll(",", ""));
  let total = 0n,
    start = 0,
    last = 1000000000000000n;
  for (let i = 0; i < t.length; i++) {
    const sc = scale(t[i]!);
    if (sc !== undefined) {
      if (sc >= last) return;
      const part = smallInt(t.slice(start, i)) ?? (i === start + 1 ? parse(t[start]!) : undefined);
      if (part === undefined || part <= 0n || part >= 1000n) return;
      const sum = checked(total + part * sc);
      if (sum === undefined) return;
      total = sum;
      last = sc;
      start = i + 1;
    }
  }
  let rest = t.slice(start);
  if (start > 0 && rest[0] === "and") {
    rest = rest.slice(1);
    if (!rest.length) return;
  }
  const tail = !rest.length && start > 0 ? 0n : smallInt(rest);
  return tail === undefined ? undefined : checked(total + tail);
}
// Rust written_form/numbers.rs::Number; decimal arithmetic never uses floating point.
export class Decimal {
  readonly coefficient: bigint;
  places: number;
  constructor(coefficient: bigint, places: number) {
    this.coefficient = coefficient;
    this.places = places;
  }
  // Rust written_form/numbers.rs::Number::whole.
  whole(): bigint | undefined {
    const p = 10n ** BigInt(this.places);
    return this.coefficient % p === 0n ? this.coefficient / p : undefined;
  }
  // Rust written_form/numbers.rs::Number::below_ten.
  belowTen(): boolean {
    return (this.coefficient < 0n ? -this.coefficient : this.coefficient) / 10n ** BigInt(this.places) < 10n;
  }
  // Rust written_form/numbers.rs::Number::digits.
  digits(commas = true): string {
    let s = (this.coefficient < 0n ? -this.coefficient : this.coefficient).toString();
    if (this.places > 0) {
      s = s.padStart(this.places + 1, "0");
      s = s.slice(0, -this.places) + "." + s.slice(-this.places);
    }
    const [whole, frac] = s.split(".");
    return (
      (this.coefficient < 0n ? "-" : "") +
      (commas ? whole!.replace(/\B(?=(\d{3})+(?!\d))/g, ",") : whole) +
      (frac !== undefined ? "." + frac : "")
    );
  }
  // Rust written_form/numbers.rs::Number::trimmed_digits.
  trimmedDigits(): string {
    const s = this.digits();
    return s.includes(".") ? s.replace(/0+$/, "").replace(/\.$/, "") : s;
  }
}
// Rust written_form/numbers.rs::number.
export function number(input: string[], article = false): Decimal | undefined {
  let t = groupedDigits(input),
    sign = 1n;
  if (["minus", "negative"].includes(t[0] ?? "")) {
    sign = -1n;
    t = t.slice(1);
  }
  if (!t.length) return;
  if (t.length === 1 && /^-?(?:\d+|\d{1,3}(?:,\d{3})+)(?:\.\d+)?$/.test(t[0]!)) {
    const s = t[0]!.replaceAll(",", ""),
      places = s.split(".")[1]?.length ?? 0,
      n = parse(s.replace(".", ""));
    if (places > 36 || n === undefined) return;
    const coeff = checked(n * sign);
    return coeff === undefined ? undefined : new Decimal(coeff, places);
  }
  if (scale(t.at(-1)!) !== undefined && (t.includes("point") || (t.length === 2 && /^\d+(?:\.\d+)?$/.test(t[0]!)))) {
    const n = number(t.slice(0, -1), article);
    if (n) {
      const c = checked(n.coefficient * scale(t.at(-1)!)! * sign);
      return c === undefined ? undefined : new Decimal(c, n.places);
    }
  }
  const i = t.indexOf("point");
  if (i >= 0) {
    if (t.filter((w) => w === "point").length !== 1) return;
    const whole = i === 0 ? 0n : integer(t.slice(0, i), article),
      tail = t.slice(i + 1);
    if (whole === undefined || !tail.length) return;
    let frac: string;
    if (tail.every((w) => SMALL.slice(0, 10).includes(w) || ["oh", "o"].includes(w) || /^\d$/.test(w)))
      frac = tail.map((w) => value(w)?.toString() ?? (["oh", "o"].includes(w) ? "0" : w)).join("");
    else {
      const n = integer(tail);
      if (n === undefined || n < 10n || n > 99n) return;
      frac = n.toString();
    }
    if (frac.length > 36) return;
    const c = parse(`${whole}${frac}`);
    if (c === undefined) return;
    const signed = checked(c * sign);
    return signed === undefined ? undefined : new Decimal(signed, frac.length);
  }
  const n = integer(t, article);
  if (n === undefined) return;
  const c = checked(n * sign);
  return c === undefined ? undefined : new Decimal(c, 0);
}
// Rust written_form/numbers.rs::format_number.
export function formatNumber(t: string[], force = false, article = false): string | undefined {
  const n = number(t, article);
  if (!n || (!force && !t.includes("point") && !t.some((w) => w.includes(".")) && n.belowTen())) return;
  if (["million", "billion", "trillion"].includes(t.at(-1)!)) {
    let prefix = t.slice(0, -1);
    if (article && prefix.length === 1 && prefix[0] === "a") prefix = ["one"];
    const p = number(prefix, article);
    if (p) return `${p.digits()} ${t.at(-1)}`;
  }
  return scale(t.at(-1)!) !== undefined ? n.trimmedDigits() : n.digits();
}
// Rust written_form/numbers.rs::suffix / ordinal.
export const suffix = (n: bigint) =>
  n % 100n >= 10n && n % 100n <= 20n
    ? "th"
    : n % 10n === 1n
      ? "st"
      : n % 10n === 2n
        ? "nd"
        : n % 10n === 3n
          ? "rd"
          : "th";
export function ordinal(t: string[]): bigint | undefined {
  if (t.length === 1) {
    const m = /^(\d+)(st|nd|rd|th)$/.exec(t[0]!);
    if (m) {
      const n = parse(m[1]!);
      return n !== undefined && suffix(n) === m[2] ? n : undefined;
    }
  }
  const last = ordinalWord(t.at(-1) ?? "");
  return last === undefined ? undefined : integer([...t.slice(0, -1), last]);
}
// Rust written_form/numbers.rs::year.
export function year(t: string[]): bigint | undefined {
  const n = integer(t);
  if (n !== undefined && n >= 1000n && n <= 2999n) return n;
  if (t.at(-1) === "hundred") {
    const c = integer(t.slice(0, -1));
    if (c !== undefined && c >= 10n && c <= 29n) return c * 100n;
  }
  for (let i = 1; i < t.length; i++) {
    let tail = t.slice(i);
    const zero = ["oh", "o"].includes(tail[0]!);
    if (zero) tail = tail.slice(1);
    const a = integer(t.slice(0, i)),
      b = integer(tail);
    if (
      a !== undefined &&
      b !== undefined &&
      a >= 10n &&
      a <= 29n &&
      ((b >= 10n && b <= 99n) || (zero && b >= 0n && b <= 9n))
    )
      return a * 100n + b;
  }
  return undefined;
}
// Rust written_form/numbers.rs::year_cue / short_year.
export function yearCue(before: string): "named" | "loose" | undefined {
  const t = tokens(before),
    last = t.at(-1),
    prev = t.at(-2);
  if (
    last === "of" &&
    prev &&
    (month(prev) !== undefined || ["spring", "summer", "fall", "autumn", "winter"].includes(prev))
  )
    return "named";
  if (
    ["back in", "class of"].includes(t.slice(-2).join(" ")) ||
    t.slice(-3).join(" ") === "in the year" ||
    ["since", "circa"].includes(last ?? "")
  )
    return "loose";
  return undefined;
}
function shortYear(t: string[], before: string, after: string): string | undefined {
  const cue = yearCue(before);
  if (!cue) return;
  const next = tokens(after)[0];
  if (
    next &&
    (value(next) !== undefined ||
      scale(next) !== undefined ||
      ["hundred", "oh", "o"].includes(next) ||
      /^\d*$/.test(next) ||
      (cue === "loose" && !data.CLAUSE_WORDS.includes(next)))
  )
    return;
  if (t.length === 2 && ["oh", "o"].includes(t[0]!)) {
    const v = value(t[1]!);
    return v !== undefined && v >= 1n && v <= 9n ? `'0${v}` : undefined;
  }
  if (t.length >= 1 && t.length <= 2 && t.every((w) => value(w) !== undefined)) {
    const n = integer(t);
    return n !== undefined && n >= 10n && n <= 99n && !(cue === "loose" && n < 20n) ? `'${n}` : undefined;
  }
  return undefined;
}
// Rust written_form/numbers.rs::range.
function range(t: string[], force: boolean): [string, string] | undefined {
  if (t.filter((w) => w === "to").length !== 1) return;
  const i = t.indexOf("to"),
    left = t.slice(0, i),
    right = t.slice(i + 1);
  if (!left.length || !right.length || t.some((w) => ["a", "minus", "negative"].includes(w) || w.startsWith("-")))
    return;
  const a = number(left),
    b = number(right);
  if (!a || !b) return;
  if (
    left.includes("hundred") &&
    !left.some((w) => scale(w) !== undefined) &&
    right.some((w) => scale(w) !== undefined)
  )
    return;
  const ls = left.filter((w) => scale(w) !== undefined || w === "hundred").length,
    rs = right.filter((w) => scale(w) !== undefined || w === "hundred").length;
  if (rs > 0 && ls === 0) {
    const magnitude = scale(right.at(-1)!),
      coefficient = number(right.slice(0, -1)),
      bounded = (n: Decimal) => n.coefficient > 0n && n.coefficient / 10n ** BigInt(n.places) < 1000n;
    if (magnitude === undefined || !coefficient || rs !== 1 || !bounded(a) || !bounded(coefficient)) return;
    const ac = checked(a.coefficient * 10n ** BigInt(coefficient.places)),
      bc = checked(coefficient.coefficient * 10n ** BigInt(a.places));
    if (ac === undefined || bc === undefined || ac > bc) return;
    if (magnitude === 1000n) {
      const c = checked(a.coefficient * 1000n);
      return c === undefined ? undefined : [new Decimal(c, a.places).trimmedDigits(), b.trimmedDigits()];
    }
  } else if (ls > 0 && rs === 0) return;
  else if (!force && !t.includes("point") && !t.some((w) => /^\d+\.\d+$/.test(w)) && a.belowTen() && b.belowTen())
    return;
  const av = formatNumber(left, true),
    bv = formatNumber(right, true);
  return av === undefined || bv === undefined ? undefined : [av, bv];
}
const currency = (w: string | undefined) =>
  ["dollar", "dollars"].includes(w ?? "") ? "$" : ["euro", "euros"].includes(w ?? "") ? "€" : undefined;
// Rust written_form/numbers.rs::money.
function money(t: string[]): string | undefined {
  if (t.includes("to")) {
    if (t.filter((w) => w === "to").length !== 1) return;
    const symbol = currency(t.at(-1)),
      i = t.indexOf("to");
    if (!symbol) return;
    if (i > 0 && currency(t[i - 1])) {
      if (currency(t[i - 1]) !== symbol) return;
      const a = money(t.slice(0, i)),
        b = money(t.slice(i + 1));
      return a === undefined || b === undefined ? undefined : `${a} to ${b}`;
    }
    const r = range(t.slice(0, -1), true);
    return r ? `${symbol}${r[0]} to ${symbol}${r[1]}` : undefined;
  }
  const last = t.at(-1);
  if (!last) return;
  if (["bucks", "buck", "pounds", "pound"].includes(last)) {
    const n = formatNumber(t.slice(0, -1));
    return n === undefined ? undefined : `${n} ${last}`;
  }
  const indices = t.flatMap((w, i) => (currency(w) ? [i] : []));
  if (indices.length === 1) {
    const i = indices[0]!;
    let amount = formatNumber(t.slice(0, i), true, true);
    if (amount === undefined) return;
    let rest = t.slice(i + 1);
    if (rest.length) {
      if (rest[0] === "and") rest = rest.slice(1);
      if (!["cent", "cents"].includes(rest.at(-1)!)) return;
      const cents = integer(rest.slice(0, -1)),
        dollars = number(t.slice(0, i), true)?.whole();
      if (cents === undefined || cents < 0n || cents >= 100n || dollars === undefined) return;
      amount = `${new Decimal(dollars, 0).digits()}.${String(cents).padStart(2, "0")}`;
    }
    return `${currency(t[i])}${amount}`;
  }
  if (["cent", "cents"].includes(last)) {
    const n = formatNumber(t.slice(0, -1), true);
    return n === undefined ? undefined : `${n} ${last}`;
  }
  return undefined;
}
// Rust written_form/numbers.rs::day_valid / date.
function dayValid(d: bigint, m: number, y = 2000n): boolean {
  const days =
    m === 1
      ? y % 4n === 0n && (y % 100n !== 0n || y % 400n === 0n)
        ? 29n
        : 28n
      : [3, 5, 8, 10].includes(m)
        ? 30n
        : 31n;
  return d >= 1n && d <= days;
}
function date(t: string[]): string | undefined {
  const indices = t.flatMap((w, i) => (month(w) === undefined ? [] : [[i, month(w)!]]));
  if (indices.length !== 1) return;
  const [mi, m] = indices[0]!;
  if (mi === undefined || m === undefined) return;
  if (mi === 0) {
    for (let split = 1; split < t.length; split++) {
      const day = ordinal(t.slice(1, split + 1)) ?? integer(t.slice(1, split + 1)),
        yt = t.slice(split + 1),
        y = year(yt);
      if (day === undefined || (yt.length && y === undefined) || !dayValid(day, m, y)) continue;
      return `${MONTHS[m]} ${day}${y === undefined ? "" : `, ${y}`}`;
    }
  } else if (mi >= 2 && t[mi - 1] === "of") {
    const start = t[0] === "the" ? 1 : 0,
      day = ordinal(t.slice(start, mi - 1)),
      yt = t.slice(mi + 1),
      y = year(yt);
    if (day === undefined || (yt.length && y === undefined) || !dayValid(day, m, y)) return;
    return `${start ? "the " : ""}${day}${suffix(day)} of ${MONTHS[m]}${y === undefined ? "" : `, ${y}`}`;
  }
  return undefined;
}
// Rust written_form/numbers.rs::time.
function time(input: string[], before: string): string | undefined {
  let t = input;
  if (t.some((w) => w.includes("clock"))) return;
  const prefix = t[0] === "at" ? "at " : "";
  if (prefix) t = t.slice(1);
  let period: string | undefined;
  if (["am", "pm"].includes(t.at(-1)!)) {
    period = t.at(-1)!.toUpperCase();
    t = t.slice(0, -1);
  }
  const cue = tokens(before).at(-1) ?? "",
    clockCue = ["until", "till", "by", "before", "after", "around"].includes(cue);
  if (!period && !prefix && cue !== "at" && !clockCue) return;
  const limit = period || (!prefix && cue !== "at") ? 12n : 23n,
    hour = integer(t);
  if (hour !== undefined) return period && hour >= 1n && hour <= limit ? `${prefix}${hour} ${period}` : undefined;
  for (let i = 1; i < t.length; i++) {
    let mt = t.slice(i);
    if (["oh", "o"].includes(mt[0]!)) mt = mt.slice(1);
    const h = integer(t.slice(0, i)),
      m = integer(mt);
    if (h !== undefined && m !== undefined && h >= BigInt(!!period) && h <= limit && m >= 0n && m < 60n)
      return `${prefix}${h}:${String(m).padStart(2, "0")}${period ? " " + period : ""}`;
  }
  return undefined;
}
// Rust written_form/numbers.rs::version.
function version(t: string[]): string | undefined {
  if (t.length < 2 || !["version", "v", "python", ".net"].includes(t[0]!)) return;
  const values = t.slice(1);
  const component = (g: string[]): string | undefined =>
    g.length === 1 && /^\d+$/.test(g[0]!)
      ? g[0]
      : g.length && g.every((w) => SMALL.slice(0, 10).includes(w) || ["oh", "o"].includes(w))
        ? g.map((w) => (value(w) ?? 0n).toString()).join("")
        : integer(g)?.toString();
  let parts: (string | undefined)[];
  if (values.includes("point")) {
    const groups: string[][] = [[]];
    for (const w of values)
      if (w === "point") groups.push([]);
      else groups.at(-1)!.push(w);
    parts = groups.map(component);
  } else if (values.length === 1 && /^\d+(?:\.\d+)*$/.test(values[0]!)) parts = values[0]!.split(".");
  else {
    const v = integer(values);
    if (v !== undefined) parts = [String(v)];
    else if (["python", ".net"].includes(t[0]!) && values.length === 2) parts = values.map((w) => component([w]));
    else return;
  }
  if (parts.some((p) => p === undefined || p.length > 4 || !/^\d+$/.test(p))) return;
  return `${t[0] === "v" ? "v" : t[0] === "python" ? "Python " : t[0] === ".net" ? ".NET " : "version "}${parts.join(".")}`;
}
// Rust written_form/numbers.rs::candidate.
export function candidate(
  t: string[],
  cls: Class,
  before: string,
  after: string,
  original: string,
): string | undefined {
  const last = t.at(-1);
  if (!last) return;
  switch (cls) {
    case "MODEL":
      return;
    case "AMP": {
      if (t.filter((w) => w === "and").length !== 1 || t.some((w) => w !== "and" && !/^[a-z]$/i.test(w))) return;
      const name = t.map((w) => (w === "and" ? "&" : w.toUpperCase())).join("");
      return AMP_NAMES.includes(name) ? name : undefined;
    }
    case "CARDINAL": {
      if (["number", "step"].includes(tokens(before).at(-1) ?? "") || t[0] === "minus") return;
      if (t.includes("to")) return range(t, false)?.join(" to ");
      return formatNumber(
        t,
        false,
        t.join(" ") === "a billion" &&
          ["parameters", "tokens", "shares", "bytes", "records", "requests"].includes(tokens(after)[0] ?? ""),
      );
    }
    case "DECIMAL":
      if (!t.includes("point") && !t.some((w) => /^\d+\.\d+$/.test(w))) return;
      return t.includes("to") ? range(t, true)?.join(" to ") : formatNumber(t, true);
    case "MONEY":
      return money(t);
    case "PERCENT": {
      if (last !== "percent") return;
      if (t.includes("to")) {
        if (t.filter((w) => w === "to").length !== 1) return;
        const i = t.indexOf("to");
        if (i > 0 && t[i - 1] === "percent") {
          const a = formatNumber(t.slice(0, i - 1), true, true),
            b = formatNumber(t.slice(i + 1, -1), true, true);
          return a === undefined || b === undefined ? undefined : `${a}% to ${b}%`;
        }
        const r = range(t.slice(0, -1), true);
        return r ? `${r[0]} to ${r[1]}%` : undefined;
      }
      const n = formatNumber(t.slice(0, -1), true, true);
      return n === undefined ? undefined : n + "%";
    }
    case "MAGNITUDE": {
      if (!["k", "m", "x"].includes(last)) return;
      const n = formatNumber(t.slice(0, -1), true);
      return n === undefined ? undefined : n + (last === "x" ? "x" : last.toUpperCase());
    }
    case "YEAR":
      return year(t)?.toString() ?? shortYear(t, before, after);
    case "DATE":
      return date(t);
    case "TIME":
      return time(t, before);
    case "ORDINAL": {
      const n = ordinal(t);
      return n !== undefined && n >= 10n ? `${n}${suffix(n)}` : undefined;
    }
    case "MEASURE": {
      if (unit(last) || duration(last)) {
        const n = formatNumber(t.slice(0, -1), unit(last));
        return n === undefined
          ? undefined
          : `${n} ${original
              .split(/\p{White_Space}+/u)
              .at(-1)!
              .replace(/[.,;:!?]+$/, "")}`;
      }
      const u = tokens(after)[0];
      return u && (unit(u) || duration(u)) ? formatNumber(t, unit(u)) : undefined;
    }
    case "VERSION": {
      const v = version(t);
      return v !== undefined && t[0] === "v" && original.startsWith("V") ? "V" + v.slice(1) : v;
    }
  }
}
