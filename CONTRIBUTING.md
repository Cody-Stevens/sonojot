# Contributing to SonoJot

Keep changes focused. Describe the problem, the resulting behavior, and how you checked it. Preserve raw recognition separately from published text, conservative cleanup, and cancellation and clipboard safeguards. Use synthetic text in tests and issue reports.

Contributions intentionally submitted for inclusion are licensed under Apache-2.0, the same license as the project (inbound equals outbound), unless you explicitly state otherwise. We recommend a [Developer Certificate of Origin](https://developercertificate.org/) sign-off using `git commit -s` when you can make that certification. A sign-off is recommended, not required.

## Validate a change

Install dependencies and platform build tools as described in the [README](README.md). In `rewrite/`, run `npm run check` and `npm test`. In `rewrite/src-tauri/`, run `cargo check -j 6` and `cargo test -j 6 --lib`. For website changes, run `npm run build` in `web/`. Include a short visual check when changing UI. Do not run hardware-dependent ignored tests unless you have the required models and runtimes.

Do not commit credentials, personal dictations, recordings, local configuration, model weights, or build output. Preserve third-party notices and license files. Follow the [Code of Conduct](CODE_OF_CONDUCT.md).
