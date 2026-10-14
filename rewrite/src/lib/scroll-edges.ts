/**
 * Scroll-edge cues for every scrolling region (design bible: paper-pocket
 * openings). A lip shadow shows at the top while content exists above and at
 * the bottom while content exists below; each fades in over the last 28px of
 * remaining scroll. The paint lives in styles.css (`[data-scroll-edges]`,
 * `.scroll-edges-frame` and the main `.page-frame`); this file only measures.
 *
 * Two placements share one measurement and one recipe:
 * - default: the scroller draws sticky `::before`/`::after` lips inside itself.
 *   Use it on block scrollers (lists, wells, popover bodies). A flex or grid
 *   scroller with `gap` sets `--edge-gap` to that gap so the lips add no space.
 * - `frame`: the parent draws the lips (the main page frame, and wrappers
 *   around textareas, which cannot hold pseudo-elements).
 */

/** Remaining scroll, in pixels, over which an edge fades fully in. */
export const EDGE_FADE_PX = 28;

export interface EdgeState {
  /** 0 at the top, 1 once 28px or more of content is above. */
  start: number;
  /** 0 at the bottom, 1 while 28px or more of content is below. */
  end: number;
}

/** Pure edge opacity from scroll metrics; sub-pixel ranges count as none. */
export function edgeState(metrics: {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}): EdgeState {
  const range = metrics.scrollHeight - metrics.clientHeight;
  if (range < 1) return { start: 0, end: 0 };
  const top = Math.min(range, Math.max(0, metrics.scrollTop));
  const fraction = (pixels: number) =>
    pixels < 1 ? 0 : Math.min(1, pixels / EDGE_FADE_PX);
  return { start: fraction(top), end: fraction(range - top) };
}

/** Write an edge state as the CSS variables and test hooks the paint uses. */
export function paintEdges(
  node: HTMLElement,
  target: HTMLElement,
  state: EdgeState,
): void {
  // Inside a closed <details> a fade would never run or finish; set the lips
  // without one so they are already right when the content opens.
  target.toggleAttribute(
    "data-edge-instant",
    !(node.checkVisibility?.() ?? true),
  );
  target.style.setProperty("--edge-start", String(state.start));
  target.style.setProperty("--edge-end", String(state.end));
  node.toggleAttribute("data-edge-start", state.start > 0);
  node.toggleAttribute("data-edge-end", state.end > 0);
}

/**
 * Re-run `measure` (once per frame at most) when the scroller scrolls, resizes,
 * gains or loses content, or a textarea's text changes.
 */
export function watchScroll(
  node: HTMLElement,
  measure: () => void,
): () => void {
  let pending = 0;
  const run = () => {
    pending = 0;
    measure();
  };
  const schedule = () => {
    if (!pending) pending = requestAnimationFrame(run);
  };
  const resize = new ResizeObserver(schedule);
  const observeChildren = () => {
    resize.disconnect();
    resize.observe(node);
    for (const child of Array.from(node.children)) resize.observe(child);
  };
  observeChildren();
  // Direct children are enough: a descendant that grows grows its child, which
  // the resize observer reports. New children need observing too.
  const mutation = new MutationObserver(() => {
    observeChildren();
    schedule();
  });
  mutation.observe(node, { childList: true });
  node.addEventListener("scroll", schedule, { passive: true });
  node.addEventListener("input", schedule);
  schedule();
  return () => {
    resize.disconnect();
    mutation.disconnect();
    node.removeEventListener("scroll", schedule);
    node.removeEventListener("input", schedule);
    cancelAnimationFrame(pending);
  };
}

export interface ScrollEdgesOptions {
  /** Paint on the parent element instead of inside the scroller. */
  frame?: boolean;
}

/** Svelte action: `use:scrollEdges` or `use:scrollEdges={{ frame: true }}`. */
export function scrollEdges(
  node: HTMLElement,
  options: ScrollEdgesOptions = {},
): { destroy(): void } {
  const target = options.frame ? (node.parentElement ?? node) : node;
  // A data attribute, not a class: Svelte rewrites `class` on some elements.
  node.dataset.scrollEdges = options.frame ? "frame" : "inside";
  const stop = watchScroll(node, () => {
    // A frame's lips follow the scroller's box, whatever margins it has.
    const set = (name: string, px: number) =>
      target.style.setProperty(name, `${px}px`);
    if (target !== node) {
      set("--edge-frame-top", Math.max(0, node.offsetTop));
      set("--edge-frame-left", Math.max(0, node.offsetLeft));
      set(
        "--edge-frame-bottom",
        Math.max(0, target.clientHeight - node.offsetTop - node.offsetHeight),
      );
      set(
        "--edge-frame-right",
        Math.max(0, target.clientWidth - node.offsetLeft - node.offsetWidth),
      );
    } else {
      // Sticky offsets count from the padding edge; pull the lips out to the
      // visible edge of the scroller.
      const style = getComputedStyle(node);
      set("--edge-stick-top", -(parseFloat(style.paddingTop) || 0));
      set("--edge-stick-bottom", -(parseFloat(style.paddingBottom) || 0));
    }
    paintEdges(node, target, edgeState(node));
  });
  return {
    destroy() {
      stop();
      delete node.dataset.scrollEdges;
    },
  };
}
