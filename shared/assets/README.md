# Typography density plate

`paper-density.webp` is derived from SonoJot's existing
`rewrite/src/brand/material/paper-stock-seamless.png`, not a new paper pattern.
The build script centers luminance at 234, expands fiber contrast around neutral
gray, then downsamples to 940 pixels and encodes WebP at 0.92 quality. Its repeat
remains 620 CSS pixels so the original fiber coordinates match the stock.

Regenerate from the repository root with `node shared/assets/build-density.mjs`.
This is an offline developer operation using the existing Playwright dependency;
no canvas processing runs in the app or website. All text styles share the
resulting asset and their pigment modulation stays in the shared CSS.

Source SHA-256: `f1acf4bf41a6eb0c5ddcb5a7f8e572860ed7cf0839dc1d5584abd8ad6171728f`.
Plate SHA-256: `63889cfffcf2f4b3f0ccaddca47d4e86ffcd874a90552f0d492da34a4ee8d29c`.
