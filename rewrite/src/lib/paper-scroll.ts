/** Fixed paper lips show only the content that remains beyond the scrollport. */
export function paperScroll(node: HTMLElement): { destroy(): void } {
  const frame = node.parentElement;
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
  let pending = 0;
  const measure = () => {
    pending = 0;
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
    const remaining = Math.max(0, node.scrollHeight - node.clientHeight);
    frame?.style.setProperty(
      "--pocket-top",
      String(Math.min(1, Math.max(0, node.scrollTop) / 28)),
    );
    frame?.style.setProperty(
      "--pocket-bottom",
      String(Math.min(1, Math.max(0, remaining - node.scrollTop) / 28)),
    );
  };
  const schedule = () => {
    if (!pending) pending = requestAnimationFrame(measure);
  };
  const observer = new ResizeObserver(schedule);
  observer.observe(node);
  if (flow) observer.observe(flow);
  node.addEventListener("scroll", schedule, { passive: true });
  schedule();
  return {
    destroy() {
      observer.disconnect();
      node.removeEventListener("scroll", schedule);
      cancelAnimationFrame(pending);
      if (priorPad) node.style.setProperty("--pocket-pad", priorPad);
      else node.style.removeProperty("--pocket-pad");
    },
  };
}
