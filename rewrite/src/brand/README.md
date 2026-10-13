# Current brand assets

This directory contains the existing Source Serif font binaries, paper stock and placeholder marks. Consolidation changes their location only. Fog, Slate, green ink, the name and the live lowercase `e` mark are unchanged.

`brand.css` loads the variable UI font and stock image through relative CSS imports, which Vite fingerprints into the build. `BrandMark.svelte` is the shared live text mark. `marks/favicon.svg` is referenced by the Vite HTML entry. The app icon family in `marks/app` is consumed by Tauri at build time; its current default window icon also supplies the tray.

The regular and bold TrueType files remain the PDF's compile-time `include_bytes!` inputs. About imports `fonts/SourceSerif-LICENSE.md?raw` from the source tree so the license works in both development and production. No asset is fetched from a remote server or chosen by a failed network request.

`manifest.json` records the original file bytes and their SHA-256 hashes. Replace assets here when a later branding task is authorized, then update their hashes. The static material provenance is in `material/PROVENANCE.md`.
