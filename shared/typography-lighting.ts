/** One shared native-glyph height filter per treatment, size band and stock.
 * DOM text owns shaping/selection. Only height and contact maps are blurred.
 * No duplicated glyphs, specular light, JS text rasters, or animation loop.
 */
const NS = "http://www.w3.org/2000/svg";
let owner = 0;
const number = (style: CSSStyleDeclaration, name: string, fallback: number) => {
  const value = parseFloat(style.getPropertyValue(name));
  return Number.isFinite(value) ? value : fallback;
};
export function createTypographyLighting() {
  const svg = document.createElementNS(NS, "svg");
  svg.classList.add("type-lighting-defs");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  const defs = document.createElementNS(NS, "defs");
  svg.append(defs);
  document.body.append(svg);
  const prefix = `sj-type-${++owner}`;
  const cache = new Map<
    string,
    { id: string; signature: string; filter: SVGFilterElement }
  >();
  function filterFor(
    mode: string,
    size: number,
    style: CSSStyleDeclaration,
    theme: string,
  ): string {
    if (mode === "flat" || size < 20) return "none";
    const raised = mode === "emboss" || mode === "emboss-strong";
    const band = size >= 52 ? 2 : size >= 32 ? 1 : 0;
    const amount = [0.22, 0.55, 1][band];
    const key = `${raised ? "raised" : "pressed"}-${band}-${theme}`;
    const depth =
      number(style, "--type-depth-scale", 1) *
      amount *
      number(
        style,
        raised ? "--type-emboss-height" : "--type-imprint-depth",
        raised ? 2.5 : 2,
      );
    const shoulder = Math.max(
      0.12,
      number(style, "--type-edge-softness", 0.65) * Math.sqrt(amount),
    );
    const angle = number(style, "--material-light-angle", 45);
    const elevation = number(style, "--type-light-elevation", 32);
    const ambient = Math.min(
      0.95,
      Math.max(0.1, number(style, "--type-ambient", 0.32)),
    );
    const highlight = number(style, "--type-highlight-scale", 1),
      shadow = number(style, "--type-shadow-scale", 1);
    const contact = Math.min(
      0.8,
      number(
        style,
        raised ? "--type-emboss-shadow" : "--type-imprint-shadow",
        0.42,
      ) * shadow,
    );
    const highlightAmount =
      number(
        style,
        raised ? "--type-emboss-highlight" : "--type-imprint-highlight",
        0.38,
      ) * highlight;
    const signature = JSON.stringify([
      depth,
      shoulder,
      angle,
      elevation,
      ambient,
      contact,
      highlightAmount,
    ]);
    let entry = cache.get(key);
    if (entry?.signature === signature) return `url("#${entry.id}")`;
    if (!entry) {
      const filter = document.createElementNS(NS, "filter"),
        id = `${prefix}-${key}`;
      filter.id = id;
      for (const [k, v] of Object.entries({
        x: "-4%",
        y: "-15%",
        width: "108%",
        height: "130%",
        "color-interpolation-filters": "linearRGB",
      }))
        filter.setAttribute(k, v);
      defs.append(filter);
      entry = { id, filter, signature: "" };
      cache.set(key, entry);
    }
    const rad = (angle * Math.PI) / 180,
      offset = Math.min(1.1, depth * 0.45);
    const dx = Math.cos(rad) * offset,
      dy = Math.sin(rad) * offset;
    const diffuse =
      (1 - ambient) / Math.max(0.2, Math.sin((elevation * Math.PI) / 180));
    // Negative height is a recess. Light is opposite the paper shadow vector.
    entry.filter.innerHTML = `
      <feGaussianBlur in="SourceAlpha" stdDeviation="${shoulder}" result="shoulder"/>
      <feDiffuseLighting in="shoulder" surfaceScale="${depth * (raised ? 1 : -1)}" diffuseConstant="${diffuse}" lighting-color="white" result="directional">
        <feDistantLight azimuth="${angle + 180}" elevation="${elevation}"/>
      </feDiffuseLighting>
      <feComponentTransfer in="directional" result="light"><feFuncR type="linear" intercept="${ambient}"/><feFuncG type="linear" intercept="${ambient}"/><feFuncB type="linear" intercept="${ambient}"/></feComponentTransfer>
      <feBlend in="SourceGraphic" in2="light" mode="multiply" result="lit"/>
      <feComposite in="lit" in2="SourceAlpha" operator="in" result="face"/>
      <feOffset in="SourceAlpha" dx="${raised ? dx : -dx}" dy="${raised ? dy : -dy}" result="offset"/>
      <feGaussianBlur in="offset" stdDeviation="${Math.min(0.42, shoulder * 0.55)}" result="contact"/>
      <feComposite in="${raised ? "contact" : "SourceAlpha"}" in2="${raised ? "SourceAlpha" : "contact"}" operator="out" result="contactMask"/>
      <feFlood flood-color="#11190f" flood-opacity="${contact}"/><feComposite in2="contactMask" operator="in" result="shadow"/>
      <feOffset in="SourceAlpha" dx="${raised ? -dx : dx}" dy="${raised ? -dy : dy}" result="litOffset"/>
      <feComposite in="litOffset" in2="SourceAlpha" operator="out" result="litEdge"/>
      <feFlood flood-color="#fff5d8" flood-opacity="${Math.min(0.65, highlightAmount)}"/><feComposite in2="litEdge" operator="in" result="edge"/>
      <feMerge><feMergeNode in="edge"/><feMergeNode in="face"/><feMergeNode in="shadow"/></feMerge>`;
    entry.signature = signature;
    return `url("#${entry.id}")`;
  }
  return { filterFor, dispose: () => svg.remove() };
}
