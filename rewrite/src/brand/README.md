# Current brand assets

This directory contains the Source Serif font binaries, paper stock and the SonoJot marks.

`brand.css` loads the variable UI font and stock image through relative CSS imports, which Vite fingerprints into the build. `BrandMark.svelte` is the shared live text mark. `marks/favicon.ico` is referenced by the Vite HTML entry and `marks/favicon.svg` by the test fixtures. The app icon family in `marks/app` is consumed by Tauri at build time. Every raster in it is a resize of the 1024px `ic10` master inside `icon.icns`: after replacing that master, run `python scripts/refresh_app_icon.py` to rebuild the PNGs and ICOs and refresh this manifest. The tray uses `marks/tray-black.png` and `marks/tray-white.png`.

The regular and bold TrueType files remain the PDF's compile-time `include_bytes!` inputs. About imports `fonts/SourceSerif-LICENSE.md?raw` from the source tree so the license works in both development and production. No asset is fetched from a remote server or chosen by a failed network request.

`manifest.json` records the original file bytes and their SHA-256 hashes. Replace assets here when a later branding task is authorized, then update their hashes. The static material provenance is in `material/PROVENANCE.md`.
