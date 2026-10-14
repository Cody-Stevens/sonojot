# Windows installer identity

SonoJot uses `SonoJot.exe`. Upgrades keep the existing installation directory;
new installations go to `%LOCALAPPDATA%\Programs\SonoJot`. The NSIS template is
Tauri CLI 2.11.4's upstream template, with four changes: retain the
`Eavesdropper Next` uninstall key, retain the `Eavesdropper Next`
manufacturer/product key (upgrades find the existing installation through both;
users only see the `SonoJot` display name), default new installations to
`%LOCALAPPDATA%\Programs\SonoJot`, and remove both the `SonoJot` and the older
`EavesdropperNext` startup values on uninstall.

The hooks wait for the previous installed executable to exit, update an existing `EavesdropperNext` Run command only when it points to this installation's `eavesdropper-next.exe` or interim `sonojot.exe`, and remove old display shortcuts only when their target matches this app. The template migrates existing SonoJot shortcuts to the renamed executable and removes the old binary. An absent or unrelated startup value stays unchanged. At its next launch the app renames this installation's `EavesdropperNext` Run value to `SonoJot`, carrying Task Manager's enabled/disabled state, and moves its data folder from `%LOCALAPPDATA%\EavesdropperNext` to `%LOCALAPPDATA%\SonoJot`.

The CLI is pinned because this template tracks its bundler. When upgrading it,
refresh the upstream template and reapply these four changes; do not rename the
registry keys, which would break upgrade detection.

Source: https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
