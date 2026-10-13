# Windows installer identity

SonoJot uses `SonoJot.exe` and keeps the existing installation directory, app data and startup registration identity. The
NSIS template is Tauri CLI 2.11.4's upstream template, with four changes: retain
the uninstall key, retain the manufacturer/product key, use the stable default
installation directory, and remove the actual stable startup key on uninstall.
The hooks wait for the previous installed executable to exit, update an existing `EavesdropperNext` Run command only when it points to this installation's `eavesdropper-next.exe` or interim `sonojot.exe`, and remove old display shortcuts only when their target matches this app. The template migrates existing SonoJot shortcuts to the renamed executable and removes the old binary. An absent or unrelated startup value stays unchanged.

The CLI is pinned because this template tracks its bundler. When upgrading it,
refresh the upstream template and reapply these four changes; do not silently
return to a product-name-derived installation identity.

Source: https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
