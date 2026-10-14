import { edgeState, paintEdges, watchScroll } from "./scroll-edges";

/**
 * Whole-page scrollport: the shared scroll-edge lips, painted by the parent
 * frame, plus trailing pocket clearance that never scrolls into blank margin.
 */
export function paperScroll(node: HTMLElement): { destroy(): void } {
  const frame = node.parentElement ?? node;
  const flow = node.firstElementChild as HTMLElement | null;
  /* The scrollport's trailing padding is pocket clearance, not content. When
     the real content end plus a few pixels of contour/focus room fits inside
     the viewport, only the surplus padding is removed so the page cannot
     scroll into blank margin; when content truly overflows the full clearance
     stays. A last card needs room below it for its contour shadow and focus
     ring, so the reduction only applies while that room survives. */
  const MIN_CLEARANCE = 6;
  const priorPad = node.style.getPropertyValue("--pocket-pad");
  const fullPad = parseFloat(getComputedStyle(node).paddingBlockEnd) || 0;
  node.dataset.scrollEdges = "frame";
  const measure = () => {
    /* The flow child's box bottom is the true end of content, independent of
       scroll position and of the current trailing padding. */
    const contentEnd = flow
      ? flow.getBoundingClientRect().bottom -
        node.getBoundingClientRect().top +
        node.scrollTop
      : node.scrollHeight -
        (parseFloat(getComputedStyle(node).paddingBlockEnd) || 0);
    const room = node.clientHeight - contentEnd;
    if (room >= MIN_CLEARANCE) {
      /* Half a pixel under the exact fit keeps fractional rounding from
         leaving a 1px scroll range. */
      const pad = Math.min(fullPad, Math.max(MIN_CLEARANCE, room - 0.5));
      node.style.setProperty("--pocket-pad", `${pad}px`);
    } else {
      node.style.removeProperty("--pocket-pad");
    }
    paintEdges(node, frame, edgeState(node));
  };
  const stop = watchScroll(node, measure);
  return {
    destroy() {
      stop();
      delete node.dataset.scrollEdges;
      if (priorPad) node.style.setProperty("--pocket-pad", priorPad);
      else node.style.removeProperty("--pocket-pad");
    },
  };
}
