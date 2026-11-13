// Design bible, "No flat white": no surface, fill, highlight or focus ring is
// flat white (or, in Slate, flat black). Colours come from the theme tokens
// in styles.css (and the overlay's mirror of them in TrustOverlay.svelte);
// component styles use those tokens, never a literal near-white or near-black,
// and every outline names a token so no browser-default focus ring returns.
import { strict as assert } from "node:assert";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const src = fileURLToPath(new URL("..", import.meta.url));

/** Files that define the palette. Custom properties declared here may hold
 * the light stock colours that sit under the paper texture. */
const TOKEN_FILES = new Set(["styles.css", "views/TrustOverlay.svelte"]);

/** Justified exceptions: `file|property|value` and why. Keep this short. */
const ALLOWED = new Map<string, string>([
  [
    "components/CursorIndicator.svelte|stroke|#ffffffb3",
    "Halo round the cursor indicator, drawn over any app's text; it must read on dark and light hosts alike.",
  ],
  [
    "styles.css|box-shadow|inset calc(1px * var(--material-light-x, 1)) calc(1px * var(--material-light-y, 1)) 1px #fff9, 0 3px 8px #1a261e80",
    "Top-edge light on the Fog theme swatch, which previews Fog in either theme, so it cannot follow the current theme's --light.",
  ],
]);

/** Masks use black and white as alpha, not as paint. */
const MASK = /^(-webkit-)?mask(-image)?$/;

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "tests" ? [] : files(path);
    return /\.(css|svelte)$/.test(entry.name) ? [path] : [];
  });
}

function styles(path: string): string {
  const text = readFileSync(path, "utf8");
  if (path.endsWith(".css")) return text;
  return [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)]
    .map((m) => m[1])
    .join("\n");
}

type Declaration = { selector: string; prop: string; value: string };

/** Every `property: value` declaration with its rule's selector, comments removed. */
export function declarations(css: string): Declaration[] {
  const out: Declaration[] = [];
  const selectors: string[] = [];
  let segment = "";
  for (const ch of css.replace(/\/\*[\s\S]*?\*\//g, "")) {
    if (ch === "{") {
      selectors.push(segment.trim().replace(/\s+/g, " "));
      segment = "";
    } else if (ch === ";" || ch === "}") {
      const colon = segment.indexOf(":");
      if (colon > 0) {
        const prop = segment.slice(0, colon).trim();
        if (/^-?-?[\w-]+$/.test(prop))
          out.push({
            selector: selectors.at(-1) ?? "",
            prop,
            value: segment
              .slice(colon + 1)
              .trim()
              .replace(/\s+/g, " "),
          });
      }
      if (ch === "}") selectors.pop();
      segment = "";
    } else segment += ch;
  }
  return out;
}

/** Hover, focus and chosen states. */
const STATE =
  /:hover|:focus|:active|:checked|\[aria-(selected|pressed|checked|current)|\.(selected|chosen|current|active)\b/;
/** The light stock colours: under the paper texture they read as paper;
 * alone they are a flat near-white patch. */
const STOCK = /^var\(--(sj-)?(card|sheet)\)$/;

/** A state that fills with a stock colour but drops or never sets the paper
 * texture: the "flat white highlight" fault. */
export function flatStates(css: string): string[] {
  const rules = new Map<string, Declaration[]>();
  for (const d of declarations(css)) {
    if (!STATE.test(d.selector)) continue;
    rules.set(d.selector, [...(rules.get(d.selector) ?? []), d]);
  }
  const out: string[] = [];
  for (const [selector, list] of rules) {
    const fill = list.find(
      (d) =>
        (d.prop === "background" || d.prop === "background-color") &&
        STOCK.test(d.value),
    );
    const textured = list.some(
      (d) => d.prop === "background-image" && /paper/.test(d.value),
    );
    if (fill && (fill.prop === "background" || !textured))
      out.push(`${selector} { ${fill.prop}: ${fill.value} }`);
  }
  return out;
}

type Rgba = { r: number; g: number; b: number; a: number };

/** Colour literals in a value, resolved to RGBA. */
export function colours(value: string): { text: string; rgba: Rgba }[] {
  const found: { text: string; rgba: Rgba }[] = [];
  for (const m of value.matchAll(/#([0-9a-f]{3,8})\b/gi)) {
    let h = m[1];
    if (![3, 4, 6, 8].includes(h.length)) continue;
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    found.push({
      text: m[0],
      rgba: { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? n(6) / 255 : 1 },
    });
  }
  for (const m of value.matchAll(/rgba?\(([^)]*)\)/gi)) {
    const parts = m[1].split(/[\s,/]+/).filter(Boolean);
    const num = (s: string | undefined, max: number) =>
      s === undefined
        ? 1
        : s.endsWith("%")
          ? (parseFloat(s) / 100) * max
          : parseFloat(s);
    if (parts.length < 3 || parts.some((p) => p.startsWith("var"))) continue;
    found.push({
      text: m[0],
      rgba: {
        r: num(parts[0], 255),
        g: num(parts[1], 255),
        b: num(parts[2], 255),
        a: num(parts[3], 1),
      },
    });
  }
  for (const m of value.matchAll(/hsla?\(([^)]*)\)/gi)) {
    const parts = m[1].split(/[\s,/]+/).filter(Boolean);
    const l = parseFloat(parts[2] ?? "0");
    const a = parts[3]
      ? parseFloat(parts[3]) / (parts[3].endsWith("%") ? 100 : 1)
      : 1;
    const v = (l / 100) * 255; // Lightness bounds every channel at the extremes.
    found.push({ text: m[0], rgba: { r: v, g: v, b: v, a } });
  }
  if (/(^|[\s,(])white\b/i.test(value))
    found.push({ text: "white", rgba: { r: 255, g: 255, b: 255, a: 1 } });
  if (/(^|[\s,(])black\b/i.test(value))
    found.push({ text: "black", rgba: { r: 0, g: 0, b: 0, a: 1 } });
  return found;
}

const nearWhite = (c: Rgba) => c.a >= 0.5 && Math.min(c.r, c.g, c.b) >= 240;
const nearBlack = (c: Rgba) => c.a >= 0.5 && Math.max(c.r, c.g, c.b) <= 12;
const SYSTEM =
  /(^|[\s,(])(Canvas|CanvasText|ButtonFace|ButtonText|Field|FieldText|Window|-webkit-focus-ring-color|Highlight)\b/;

function problems(): string[] {
  const out: string[] = [];
  for (const path of files(src)) {
    const file = relative(src, path).replaceAll("\\", "/");
    for (const flat of flatStates(styles(path)))
      out.push(
        `${file}|${flat}: flat stock fill in a state; raise it on the paper texture`,
      );
    for (const { prop, value } of declarations(styles(path))) {
      const key = `${file}|${prop}|${value}`;
      if (ALLOWED.has(key)) continue;
      const token = prop.startsWith("--");
      if ((token && TOKEN_FILES.has(file)) || MASK.test(prop)) continue;
      for (const c of colours(value))
        if (nearWhite(c.rgba) || nearBlack(c.rgba))
          out.push(
            `${key}: ${c.text} is flat white or black; use a theme token`,
          );
      if (SYSTEM.test(value))
        out.push(`${key}: system colour; use a theme token`);
      if (
        (prop === "outline" || prop === "outline-color") &&
        !/^(none|0|0px)$/.test(value) &&
        !/var\(--/.test(value)
      )
        out.push(`${key}: outline without a token; use var(--focus)`);
    }
  }
  return out;
}

test("component styles never use flat white, flat black or default outlines", () => {
  assert.deepEqual(problems(), []);
});

test("the guard catches what it is meant to catch", () => {
  const values = (css: string) =>
    declarations(css).flatMap(({ value }) => colours(value));
  assert.equal(
    values("a:hover{background:#fff}").filter((c) => nearWhite(c.rgba)).length,
    1,
  );
  assert.equal(
    values("a{background: rgb(255 253 247)}").filter((c) => nearWhite(c.rgba))
      .length,
    1,
  );
  assert.equal(values("a{color: white}").length, 1);
  assert.equal(
    values("a{box-shadow: inset 0 1px #ffffff30}").filter((c) =>
      nearWhite(c.rgba),
    ).length,
    0,
  );
  assert.equal(
    values("a{background: hsl(40 30% 98%)}").filter((c) => nearWhite(c.rgba))
      .length,
    1,
  );
  assert.equal(
    values("a{background:#0a0a0a}").filter((c) => nearBlack(c.rgba)).length,
    1,
  );
  assert.deepEqual(
    declarations("a:focus-visible{outline:2px solid}").map((d) => d.prop),
    ["outline"],
  );
  // The November 13 fault: a highlighted choice swapped to the flat card colour.
  assert.equal(
    flatStates(".opt:hover, .opt:focus-visible { background: var(--sj-card); }")
      .length,
    1,
  );
  assert.equal(
    flatStates(".opt:hover { background-color: var(--sj-card); }").length,
    1,
  );
  assert.equal(
    flatStates(
      ".opt:hover { background-color: var(--card); background-image: var(--paper-image); }",
    ).length,
    0,
  );
});

test("the overlay's focus ring is the design system's focus token", () => {
  const app = readFileSync(join(src, "styles.css"), "utf8");
  const overlay = styles(join(src, "views/TrustOverlay.svelte"));
  const token = (css: string, selector: string, name: string) => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start >= 0, `${selector} is missing`);
    const block = css.slice(start, css.indexOf("}", start));
    return new RegExp(`${name}:\\s*([^;]+);`).exec(block)?.[1].trim();
  };
  assert.equal(
    token(overlay, ".trust", "--sj-focus"),
    token(app, ".studio", "--focus"),
  );
  assert.equal(
    token(overlay, ".trust.sj-dark", "--sj-focus"),
    token(app, '.studio[data-theme="slate"]', "--focus"),
  );
});
