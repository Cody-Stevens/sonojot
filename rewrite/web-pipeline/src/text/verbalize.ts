import { AMP_NAMES, Decimal, MONTHS, ORDINALS, SCALES, SMALL, TENS, number, ordinalWord, suffix } from "./numbers.ts";
import { trimSpace } from "./shared.ts";

// Rust written_form/verbalize.rs::integer_speech.
export function integerSpeech(n: bigint): string {
  if (n < 0n) return `minus ${integerSpeech(-n)}`;
  if (n < 20n) return SMALL[Number(n)]!;
  if (n < 100n) return TENS[Number(n / 10n)]! + (n % 10n ? ` ${SMALL[Number(n % 10n)]}` : "");
  if (n < 1000n) return `${SMALL[Number(n / 100n)]} hundred${n % 100n ? ` ${integerSpeech(n % 100n)}` : ""}`;
  for (const [word, scale] of [...SCALES].reverse()) {
    if (n >= scale) return `${integerSpeech(n / scale)} ${word}${n % scale ? ` ${integerSpeech(n % scale)}` : ""}`;
  }
  throw new Error("Unreachable integer speech");
}

// Rust written_form/verbalize.rs::ordinal_speech.
function ordinalSpeech(n: bigint): string {
  const s = integerSpeech(n),
    at = s.lastIndexOf(" ");
  const prefix = at < 0 ? "" : s.slice(0, at),
    last = s.slice(at + 1);
  const ordinal = ORDINALS.find((o) => ordinalWord(o) === last) ?? `${last}th`;
  return prefix ? `${prefix} ${ordinal}` : ordinal;
}

// Rust written_form/verbalize.rs::dedup / article.
const dedup = (items: string[]): string[] => [...new Set(items)];
const article = (s: string): string => s.replace(/^one (hundred\b|thousand\b|million\b|billion\b|trillion\b)/, "a $1");

// Rust written_form/verbalize.rs::numeric_speech.
export function numericSpeech(input: string): string[] {
  const value = input.replaceAll(",", ""),
    negative = value.startsWith("-");
  const unsigned = value.replace(/^-+/, ""),
    [whole, fraction] = unsigned.split(".");
  if (!whole || !/^\d+$/.test(whole) || (fraction !== undefined && !/^\d+$/.test(fraction))) return [];
  const n = BigInt(whole);
  if (n > (1n << 127n) - 1n) return [];
  const spoken = integerSpeech(n);
  let options = [spoken];
  if (fraction !== undefined) {
    const tail = [...fraction].map((c) => SMALL[Number(c)]!).join(" ");
    options = [`${spoken} point ${tail}`];
    if (n === 0n) options.push(`point ${tail}`);
    if (fraction.length === 2 && !fraction.startsWith("0")) {
      const rest = integerSpeech(BigInt(fraction));
      options.push(`${spoken} point ${rest}`);
      if (n === 0n) options.push(`point ${rest}`);
    }
    if (tail.includes("zero")) {
      options.push(...options.map((s) => s.replaceAll("zero", "oh")));
      options.push(...options.map((s) => s.replaceAll("zero", "o")));
    }
  } else {
    if (n >= 1000n && n < 1000000n && n % 1000n !== 0n) {
      options.push(...numericSpeech(new Decimal(n, 3).trimmedDigits()).map((s) => `${s} thousand`));
    }
    if (n >= 1000n && n <= 2999n) {
      const century = integerSpeech(n / 100n),
        rem = n % 100n;
      if (rem > 0n) {
        options.push(`${century} ${rem < 10n ? "oh " : ""}${integerSpeech(rem)}`);
        if (rem < 10n) options.push(`${century} o ${integerSpeech(rem)}`);
      } else options.push(`${century} hundred`);
    }
  }
  options.push(...options.map(article));
  options.push(...options.map((s) => s.replace(/hundred (\w)/g, "hundred and $1")));
  if (negative) options = options.flatMap((s) => [`minus ${s}`, `negative ${s}`]);
  options.push(value);
  return dedup(options);
}

// Rust written_form/verbalize.rs::combine (the reference caps each Cartesian expansion).
function combine(parts: string[][]): string[] {
  let output = [""];
  for (const part of parts) {
    const next: string[] = [];
    outer: for (const a of output)
      for (const b of part) {
        next.push(a ? (b ? `${a} ${b}` : a) : b);
        if (next.length === 512) break outer;
      }
    output = next;
  }
  return dedup(output);
}

// Rust written_form/verbalize.rs::verbalize.
export function verbalize(written: string): string[] {
  if (new TextEncoder().encode(written).length > 2048) return [];
  const phrases = verbalizeInner(trimSpace(written));
  return dedup([...phrases, ...phrases.map(article)]);
}

// Rust written_form/verbalize.rs::verbalize_inner.
function verbalizeInner(v: string): string[] {
  if (!v) return [];
  if (AMP_NAMES.includes(v)) return [[...v].map((c) => (c === "&" ? "and" : c)).join(" ")];
  let c = /^([$€])(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?$/.exec(v);
  if (c) {
    const currency = c[1] === "$" ? ["dollar", "dollars"] : ["euro", "euros"];
    const result = combine([numericSpeech(c[2]!), [c[3] ?? ""], currency]);
    if (c[3] === undefined) {
      const [whole, frac] = c[2]!.split(".");
      if (frac !== undefined && frac.length <= 2) {
        const cents = String(BigInt(frac.padEnd(2, "0")));
        result.push(
          ...combine([numericSpeech(whole!), currency, ["and", ""], numericSpeech(cents), ["cent", "cents"]]),
        );
      }
    }
    return dedup(result);
  }
  c = /^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?%$/.exec(v);
  if (c) return combine([numericSpeech(c[1]!), [c[2] ?? ""], ["percent"]]);
  c = /^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)([KMx])$/.exec(v);
  if (c) return combine([numericSpeech(c[1]!), [c[2]!]]);
  c =
    /^(January|February|March|April|May|June|July|August|September|October|November|December) (\d{1,2})(?:, (\d{4}))?$/.exec(
      v,
    );
  if (c) {
    const day = BigInt(c[2]!);
    return combine([[c[1]!], [ordinalSpeech(day), integerSpeech(day), c[2]!], c[3] ? numericSpeech(c[3]) : [""]]);
  }
  c = /^(the )?(\d{1,2})(?:st|nd|rd|th) of (\w+)(?:, (\d{4}))?$/.exec(v);
  if (c && MONTHS.some((m) => m.toLowerCase() === c![3]!.toLowerCase())) {
    return combine([
      [(c[1] ?? "").trim()],
      [ordinalSpeech(BigInt(c[2]!))],
      ["of"],
      [c[3]!],
      c[4] ? numericSpeech(c[4]) : [""],
    ]);
  }
  c = /^(at )?(\d{1,2})(?::(\d{2}))?(?: (AM|PM))?$/.exec(v);
  if (c && (c[3] !== undefined || c[4] !== undefined)) {
    let minutes = [""];
    if (c[3] !== undefined) {
      const n = BigInt(c[3]);
      minutes = [integerSpeech(n)];
      if (n < 10n) minutes.push(`oh ${integerSpeech(n)}`, `o ${integerSpeech(n)}`);
      minutes.push(String(n));
    }
    return combine([[(c[1] ?? "").trim()], numericSpeech(c[2]!), minutes, [c[4] ?? ""]]);
  }
  c = /^([Vv]|version |Python |\.NET )(\d+(?:\.\d+)*)$/.exec(v);
  if (c) {
    const digits = c[2]!.split(".");
    if (digits.some((p) => BigInt(p) > (1n << 127n) - 1n)) return [v];
    const parts = digits.map((p) => integerSpeech(BigInt(p))),
      speech = [parts.join(" point ")];
    const choices: string[][] = [];
    for (const p of digits) {
      if (choices.length) choices.push(["point"]);
      choices.push([integerSpeech(BigInt(p)), [...p].map((b) => SMALL[Number(b)]!).join(" "), p]);
    }
    speech.push(...combine(choices));
    if (["Python", ".NET"].includes(c[1]!.trim()) && parts.length > 1) speech.push(parts.join(" "));
    speech.push(c[2]!);
    return combine([[c[1]!.trim()], speech]);
  }
  c = /^'(\d{2})$/.exec(v);
  if (c) {
    const n = BigInt(c[1]!);
    return c[1]!.startsWith("0") ? [`oh ${integerSpeech(n)}`, `o ${integerSpeech(n)}`] : [integerSpeech(n)];
  }
  c = /^(\d+)(st|nd|rd|th)$/.exec(v);
  if (c) {
    const n = BigInt(c[1]!);
    if (n <= (1n << 127n) - 1n && c[2] === suffix(n)) return [ordinalSpeech(n), v];
  }
  const at = v.indexOf(" to ");
  if (at >= 0) {
    const a = v.slice(0, at),
      b = v.slice(at + 4);
    const endpoint = /^([$€]?)(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?(%?)$/;
    const ac = endpoint.exec(a),
      bc = endpoint.exec(b);
    if (ac && bc && ac[1] === bc[1]) {
      const result =
        ac[1] && ac[3] === undefined && bc[3] !== undefined
          ? []
          : combine([verbalizeInner(a), ["to"], verbalizeInner(b)]);
      const ending =
        bc[1] === "$" ? ["dollar", "dollars"] : bc[1] === "€" ? ["euro", "euros"] : bc[4] === "%" ? ["percent"] : [""];
      if (!ac[4] && (!bc[4] || !bc[1])) {
        const amount = (m: RegExpExecArray) => m[2]! + (m[3] === undefined ? "" : ` ${m[3]}`);
        result.push(...combine([verbalizeInner(amount(ac)), ["to"], verbalizeInner(amount(bc)), ending]));
        if (ac[3] === undefined && bc[3] === undefined) {
          const av = number([ac[2]!]),
            bv = number([bc[2]!]);
          const thousands = (n: Decimal) => {
            const v = n.coefficient / 10n ** BigInt(n.places);
            return v >= 1000n && v < 1000000n;
          };
          if (av && bv && thousands(av) && thousands(bv) && av.places <= 33 && bv.places <= 33) {
            av.places += 3;
            bv.places += 3;
            result.push(
              ...combine([
                numericSpeech(av.trimmedDigits()),
                ["to"],
                numericSpeech(bv.trimmedDigits()),
                ["thousand"],
                ending,
              ]),
            );
          }
        }
      }
      return dedup(result);
    }
  }
  c = /^(-?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?)(?: (million|billion|trillion))?(?: ([A-Za-z]+))?$/.exec(v);
  if (c) return combine([numericSpeech(c[1]!), [c[2] ?? ""], [c[3] ?? ""]]);
  return [v];
}
