import { createTypographyLighting } from "./typography-lighting";
const TEXT =
  "h1,h2,h3,h4,h5,h6,p,span,a,label,small,strong,em,li,dt,dd,td,th,pre,code,summary,figcaption,[data-text-surface]";
const REPLACED =
  "svg,img,canvas,video,input,textarea,select,button,[contenteditable=true]";
// One shared density plate derived offline from the existing stock photograph.
// No runtime pixel processing or per-node image allocation.
const DENSITY_PLATE = new URL("./assets/paper-density.webp", import.meta.url)
  .href;
/** Native text owns all shaping, layout, selection and accessibility. Shared
 * observers align the plate to its stock; there is no idle rendering loop. */
export function mountTypographyMaterial(root: HTMLElement): () => void {
  if (!CSS.supports("background-clip", "text")) return () => {};
  let disposed = false,
    frame = 0,
    discover = true;
  let nodes: HTMLElement[] = [];
  const observed = new Set<Element>();
  let plateReady = false;
  const plateImage = new Image();
  plateImage.onload = () => {
    if (!disposed) {
      plateReady = true;
      schedule();
    }
  };
  plateImage.src = DENSITY_PLATE;
  const lighting = createTypographyLighting();
  const resize = new ResizeObserver(() => schedule());
  function schedule(collect = false) {
    discover ||= collect;
    if (!disposed && !frame) frame = requestAnimationFrame(register);
  }
  function collect() {
    const candidates = [...root.querySelectorAll<HTMLElement>(TEXT)].filter(
      (el) =>
        !el.closest(
          "svg,.type-lighting-defs,[data-native-type],.sr-only,[aria-hidden=true]",
        ) &&
        !el.matches(REPLACED) &&
        !el.querySelector(REPLACED) &&
        !el.querySelector("strong,b,a,[data-text-surface]") &&
        [...el.childNodes].some(
          (n) => n.nodeType === Node.TEXT_NODE && n.textContent?.trim(),
        ),
    );
    const chosen = new Set(candidates);
    const nextNodes = candidates.filter((el) => {
      for (let p = el.parentElement; p && p !== root; p = p.parentElement)
        if (chosen.has(p)) return false;
      return true;
    });
    for (const old of nodes)
      if (!nextNodes.includes(old)) old.removeAttribute("data-grain-ready");
    nodes = nextNodes;
    const next = new Set<Element>([root, ...nodes]);
    for (const el of nodes) {
      const stock = el.closest(".stock");
      if (stock) next.add(stock);
    }
    for (const el of observed)
      if (!next.has(el)) {
        resize.unobserve(el);
        observed.delete(el);
      }
    for (const el of next)
      if (!observed.has(el)) {
        resize.observe(el);
        observed.add(el);
      }
  }
  function register() {
    frame = 0;
    if (discover) {
      discover = false;
      collect();
    }
    const entries = nodes.map((el) => {
      const style = getComputedStyle(el),
        rect = el.getBoundingClientRect();
      const ownBackground =
        style.backgroundImage !== "none" &&
        !el.hasAttribute("data-grain-ready");
      const ownFill =
        style.backgroundColor !== "rgba(0, 0, 0, 0)" &&
        !el.hasAttribute("data-grain-ready");
      if (
        !rect.width ||
        !rect.height ||
        ownBackground ||
        ownFill ||
        style.display === "none"
      )
        return { el };
      const surface =
        el.closest<HTMLElement>(".stock") ??
        root.querySelector<HTMLElement>(".desk-plane") ??
        root;
      const paperStyle = getComputedStyle(surface),
        paperBox = surface.getBoundingClientRect();
      if (!plateReady || !paperStyle.getPropertyValue("--paper-image").trim())
        return { el };
      const plate = `url("${DENSITY_PLATE}")`;
      const positions =
        paperStyle.backgroundPosition.split(",").at(-1)?.trim().split(/\s+/) ??
        [];
      const tile =
        paperStyle.backgroundSize.split(",").at(-1)?.trim() || "620px 620px";
      const size = parseFloat(style.fontSize);
      const theme =
        el.closest<HTMLElement>("[data-theme]")?.dataset.theme ?? "fog";
      const mode =
        style.getPropertyValue("--type-treatment").trim() || "imprint";
      return {
        el,
        plate,
        tile,
        x:
          paperBox.left +
          surface.clientLeft +
          (parseFloat(positions[0]) || 0) -
          rect.left,
        y:
          paperBox.top +
          surface.clientTop +
          (parseFloat(positions[1]) || 0) -
          rect.top,
        strength: Math.min(1, Math.max(0.1, (size - 10) / 40)),
        filter: lighting.filterFor(mode, size, style, theme),
      };
    });
    for (const entry of entries) {
      const { el } = entry;
      if (!entry.plate) {
        el.removeAttribute("data-grain-ready");
        continue;
      }
      for (const [name, value] of Object.entries({
        "--type-stock-image": entry.plate,
        "--type-stock-size": entry.tile,
        "--type-stock-x": `${entry.x}px`,
        "--type-stock-y": `${entry.y}px`,
        "--type-grain-scale": String(entry.strength),
        "--type-relief-filter": entry.filter,
      }))
        if (el.style.getPropertyValue(name) !== value)
          el.style.setProperty(name, value);
      if (!el.hasAttribute("data-grain-ready"))
        el.setAttribute("data-grain-ready", "");
    }
  }
  const mutations = new MutationObserver((records) => {
    if (
      records.some(
        (r) =>
          !(
            r.target instanceof Element &&
            r.target.closest(".type-lighting-defs")
          ),
      )
    )
      schedule(true);
  });
  mutations.observe(root, {
    childList: true,
    characterData: true,
    subtree: true,
  });
  const themes = new MutationObserver(() => schedule());
  themes.observe(document.documentElement, {
    attributes: true,
    subtree: true,
    attributeFilter: [
      "data-theme",
      "data-material",
      "data-type-material",
      "data-text-surface",
      "aria-current",
      "aria-selected",
      "aria-pressed",
    ],
  });
  const tuning = () => schedule();
  document.addEventListener("typography-tune", tuning);
  document.addEventListener("scroll", tuning, { capture: true, passive: true });
  window.addEventListener("resize", tuning, { passive: true });
  root.addEventListener("transitionend", tuning);
  void document.fonts.ready.then(() => schedule());
  schedule(true);
  return () => {
    disposed = true;
    cancelAnimationFrame(frame);
    resize.disconnect();
    mutations.disconnect();
    themes.disconnect();
    lighting.dispose();
    document.removeEventListener("typography-tune", tuning);
    document.removeEventListener("scroll", tuning, true);
    window.removeEventListener("resize", tuning);
    root.removeEventListener("transitionend", tuning);
    for (const el of nodes) {
      el.removeAttribute("data-grain-ready");
      for (const name of ["image", "size", "x", "y"])
        el.style.removeProperty(`--type-stock-${name}`);
      el.style.removeProperty("--type-grain-scale");
      el.style.removeProperty("--type-relief-filter");
    }
  };
}
