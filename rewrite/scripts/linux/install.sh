#!/usr/bin/env bash
# Build and install SonoJot on Arch Linux (Omarchy included) from this checkout.
# Installs build tools with pacman, builds a pacman package with makepkg and
# installs it. Run again after pulling to upgrade.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
package_dir="$(realpath "$here/../../packaging/arch")"

if ! command -v pacman >/dev/null; then
  echo "This installer is for Arch Linux. Other distributions can install the .deb or AppImage that 'npm run tauri build' makes." >&2
  exit 1
fi

# Runtime and build dependencies. --needed skips anything already installed.
sudo pacman -S --needed --noconfirm \
  base-devel rustup nodejs npm libarchive \
  webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg alsa-lib \
  pipewire-alsa wl-clipboard hicolor-icon-theme

if ! cargo --version >/dev/null 2>&1; then
  rustup default stable
fi

cd "$package_dir"
makepkg -sif --noconfirm
rm -rf "$package_dir/src" "$package_dir/pkg"

cat <<'EOF'

Installed. Start SonoJot from the launcher or run `SonoJot`.
Setup downloads a speech model. On Hyprland, SonoJot binds the dictation
shortcut itself (see ~/.config/hypr/sonojot.lua).
EOF
