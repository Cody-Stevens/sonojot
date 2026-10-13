# Release license inventory

From the repository root, regenerate and check the notices with Python 3.11+:

```powershell
python rewrite/scripts/generate_notices.py --offline
python rewrite/scripts/generate_notices.py --offline --check
python -m unittest discover -s rewrite/scripts -p test_generate_notices.py -v
```

Omit `--offline` when Cargo needs to fetch a locked crate. The generator runs
`cargo metadata --locked --format-version 1 --filter-platform` for Windows x64,
macOS ARM64 and macOS x64. It does not compile the application, install npm
packages, or start a server. The checked-out `node_modules` must match the npm
lockfile. Set `CARGO_TARGET_DIR` to the project's shared target directory when
running alongside other Cargo work; metadata itself produces no build artifacts.

Generation writes the inventory and notices even when coverage is incomplete,
then returns 1 and lists the errors. `--check` compares all expected outputs
without changing them and also fails on missing mandatory evidence. Use
`--strict --check` as the public-release gate: it also fails on unresolved owner
review issues, including incomplete attribution and unverified toolchain notices.
Operational or malformed-input failures return 2. Do not waive an error by
deleting a component or inventing a license.

## Files and evidence

- `inventory.json` records declarations, repositories, exact versions, targets,
  citations, checksums, original text hashes, input hashes, and review issues.
  `components` covers the bundled application and installer. `buildTools`
  separates crates reachable only through build or proc-macro edges. `downloads`
  preserves every field from the reviewed download manifest; `services` stays out of
  the notices document.
- `assets.json` assigns provenance rules to fonts, artwork, sounds, dictionaries,
  embedded tables and existing notice resources. Newly discovered assets without
  a rule fail generation. Source artwork is counted conservatively, including
  unused icon-family members.
- `supplements.json` identifies embedded libraries and installer components that
  package manifests alone do not describe. Its version-specific overrides retain
  copyright comments and their byte offsets where no separate notice exists.
- `native-rules.json` records which native files are relevant to the configured
  targets, and why other native files do not ship. An unfamiliar native crate or
  an incompatible feature causes an error.
- `upstream-evidence.json` stores upstream bytes as base64 with source URLs and
  SHA-256 hashes. This preserves original encodings and line endings. Registry
  crates that omit a notice are supplemented from their recorded Git revision
  where available. Explicit `standard-license` records reproduce standard terms;
  they do not fabricate missing component-specific copyright notices. Those gaps
  remain owner review issues.
- `texts/` contains content-addressed original bytes. Identical files deduplicate;
  different copyright notices remain distinct even when their license names
  match. Markdown renders these texts verbatim inside fences. A declared legacy
  encoding is converted to UTF-8 for display only; the evidence bytes and hash
  remain unchanged. Regeneration removes retired generated text files; `--check`
  reports them without deleting them.
- `evidence/` contains portable Cargo graphs and native/asset coverage reports.
  Cargo's absolute machine paths are not written into the reports.
- `download-components.json` and `downloads/` retain the reviewed download
  inventory and upstream evidence. Missing files, empty text coverage, unknown licenses, duplicate
  component IDs and paths escaping this directory cause errors. Update that
  inventory when any model/runtime source or pin changes, then regenerate.
- `downloads/evidence-manifest.json` identifies each saved document and its exact
  source. Generation checks every saved hash/size and restores preserved original
  bytes before emitting legal texts. Changed wording, corrupted original bytes,
  or a notice missing from the evidence manifest fails generation.
- `model-links.json` is generated from the reviewed models. It contains only the
  catalog IDs, license names and URLs needed by the frontend and native browser
  command, keeping the full audit out of the startup bundle.

## Scope and release review

This is a source inventory, not a binary scan. The Rust normal closure excludes
dev-only crates, build-only crates and proc macros; it retains crates that also
have a normal runtime path. Cargo feature unification and frontend tree shaking
can make the inventory conservative. npm's `devOptional` entries are retained
because they can be optional production dependencies, as described in the
[npm lockfile reference](https://docs.npmjs.com/cli/v11/configuring-npm/package-lock-json/#packages).

Before public release, review `validation.reviewIssues`. In particular, resolve
the missing upstream copyright notices, source availability for MPL-covered
crates and the NSIS LZMA component, the prebuilt installer helper's dependency
versions, artwork/data ownership, and the exact release compiler runtime notices.
The WebView2 SDK loader and Microsoft's separately downloaded Evergreen runtime
have separate licensing records. The SDK archive and embedded loader were matched
byte for byte; that comparison does not identify the downloaded Evergreen version.

Keep Cargo/npm lockfiles, native feature selections, bundle configuration,
installer templates, asset bytes, model catalogs and evidence current together.
`--check` detects changes to those recorded inputs. Regeneration updates hashes;
it does not replace the owner's review of new dependencies, catalog entries or
changed license terms. No release binary was built to produce this inventory.
