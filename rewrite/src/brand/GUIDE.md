# SonoJot logo package

Version 1.0 | 11 September 2026

## Start here

- `01_Vector_SVG/sonojot-symbol-compact-pine.svg`: recommended standalone symbol on a square canvas.
- `01_Vector_SVG/sonojot-symbol-wide-pine.svg`: reference-style proportions, closest to the selected concept.
- `01_Vector_SVG/sonojot-horizontal-pine.svg`: symbol and outlined wordmark without a tagline.
- `01_Vector_SVG/sonojot-horizontal-tagline-pine.svg`: the full signature with “Your voice, in writing.”
- `05_App_Icons/`: light and dark application icons, including Windows ICO and macOS ICNS containers.
- `08_Guide/SonoJot_Logo_Guide.pdf`: visual comparison and usage notes.

## What was rebuilt

The supplied concept was a raster image. The symbol has been rebuilt as clean, filled Bezier paths. Its three audio bars are regularized capsules; its curved jot stroke and open pen-nib slit are separate vector shapes. The reference-style wide mark retains the selected direction. The compact alternative raises the pen, tightens spacing, and lengthens the central audio bar rather than simply squeezing the original.

The wide symbol's artwork bounds are approximately 1.77:1. The compact symbol's artwork bounds are approximately 1.19:1 and sit on a square 1024 x 1024 master canvas with intentional clear space. These are two explicit variants; the compact version does not replace the wide original-style option.

The SonoJot wordmark was reconstructed from the supplied concept. The tagline was reset in FreeSerif with tracking and converted to outlines. No claim is made that a particular commercial font was identified in the generated reference. Every delivered logo SVG contains outlined paths, with no live text, embedded raster image, external asset, or required font installation. No font files are included.

The paper grain, lighting, shadows, and embossed treatment in the concept are presentation effects, not part of the logo's core silhouette. These masters intentionally omit those effects. Place the transparent logo over the existing paper material in the app rather than scaling a baked-in texture with the logo.

## Package structure

| Folder | Contents |
|---|---|
| `01_Vector_SVG` | Compact and wide symbols; outlined wordmark; horizontal, tagline and stacked signatures; app icons; tray symbols; favicon. Pine, black, white and selected currentColor variants. |
| `02_Vector_PDF` | Vector PDFs for artwork exchange and print placement. Two black EPS exports for legacy workflows. |
| `03_PNG_Transparent` | Transparent sRGB exports. Standalone compact symbol up to 8192 x 8192; wordmark and horizontal signatures 8192 pixels wide; stacked marks 4096 x 4096. |
| `04_JPEG_Backgrounds` | High-resolution, opaque versions on cream, white and charcoal. JPEG does not retain transparency. |
| `05_App_Icons` | Light and dark tiles at 256, 512, 1024 and 4096 pixels, plus multi-resolution ICO/ICNS files and favicon assets. |
| `06_Tray_Icons` | Black and white transparent symbols at 16, 20, 24, 32, 40, 48 and 64 pixels, plus tray ICO containers. |
| `07_Source` | Normalized vector geometry, color tokens, asset inventory, raster export manifest and a reusable Python export script. |
| `08_Guide` | Visual reference and implementation guidance. |

## Choosing a version

Use the **compact symbol** in square slots, favicons, avatars and desktop icons. Use the **wide symbol** in horizontal lockups or where it should closely resemble the chosen concept. The **wordmark-only** asset is useful when the symbol already appears elsewhere in the interface.

Use the **micro** tray version at 16-24 pixels. It has a wider nib slit for small rendering. Use the standard compact tray version from 32 pixels upward. Check the final image at the intended OS scale, on both light and dark taskbars. An asset container being valid does not constitute an on-device installation test.

Keep the supplied aspect ratios. Preserve the open pen slit and the gap between the nib and jot stroke. Do not fill those gaps with an outline, heavy shadow, or texture. Omit the tagline when it becomes difficult to read.

The master symbol files already include clear space. Keep at least one audio-bar width of space around the artwork when composing a new lockup. Do not crop to the colored pixels for routine app use; tight cropping changes alignment and optical size.

## Color tokens

| Token | sRGB hex | Intended role |
|---|---|---|
| Pine | `#34432E` | Primary logo ink and dark application tile |
| Cream | `#F7F4EA` | Light surface and reversed application-tile mark |
| Sage | `#89957E` | Secondary interface accent |
| Parchment | `#D0C6B3` | Supporting surface or divider |
| Charcoal | `#252D25` | Dark background |
| Black | `#000000` | Single-ink reproduction and light-taskbar symbols |
| White | `#FFFFFF` | Reversed marks and dark-taskbar symbols |

These are selected flat production colors, not a claim of recovering exact ink values from the textured mockup. The PDFs are vector RGB artwork, not CMYK/PDF-X press files. A printer can apply the required output profile for the actual substrate and print process.

## Export another size

The included PNGs and JPEGs were rendered programmatically from the SVG masters, not enlarged from the concept image. Raster files include an sRGB profile and 300-DPI print metadata. Pixel dimensions determine raster detail; changing DPI metadata alone does not add detail.

From `07_Source`, install the export dependencies and run:

```sh
python -m pip install -r requirements.txt
python export_logo.py ../01_Vector_SVG/sonojot-symbol-compact-pine.svg custom-logo.png --width 8192
python export_logo.py ../01_Vector_SVG/sonojot-horizontal-pine.svg custom-logo.jpg --width 10000 --background '#F7F4EA'
python export_logo.py ../01_Vector_SVG/sonojot-horizontal-black.svg custom-logo.pdf
```

The script preserves the SVG aspect ratio, validates its inputs, and renders PNG with transparency unless a background is requested. JPEG defaults to a white background. CairoSVG requires its Cairo runtime; platform-specific dependency setup may be needed.

The `currentColor` SVG can be embedded inline and tinted with CSS. For a standalone file or an `<img>` element, use an explicit pine, black or white SVG unless the application handles color replacement itself.

## Production notes

This is a production artwork package for the selected visual direction, not a new trademark or name-availability review. Raster exports, SVG structure, icon containers and vector PDF structure were checked during preparation. The included asset geometry and export script remain editable for later refinements.
