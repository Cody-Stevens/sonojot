/** Keeps the right edge visible without slicing text or adding punctuation. */
export function suffixPreviewOffset(
  contentWidth: number,
  viewportWidth: number,
): number {
  if (!Number.isFinite(contentWidth) || !Number.isFinite(viewportWidth))
    return 0;
  return Math.max(0, contentWidth - Math.max(0, viewportWidth));
}
