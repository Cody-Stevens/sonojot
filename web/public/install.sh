#!/bin/sh
# Install SonoJot for macOS on Apple Silicon from its verified disk image.
# With a writable /Applications, the signed app is copied there and opened.
# Otherwise the disk image opens in Downloads for a drag to Applications.
set -eu
die() { printf 'sonojot: %s\n' "$*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] ||
  die "This installer supports only macOS on Apple Silicon. Windows: https://sonojot.com/install.ps1"
for tool in curl plutil shasum open hdiutil ditto codesign spctl pgrep; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required."
done
team=YJ76KNY97F
apps=/Applications
dest="$apps/SonoJot.app"
work=$(mktemp -d)
mnt="$work/mount"
mounted=
staged=
previous=
cleanup() {
  # Put a moved-aside copy back if the new one never arrived.
  if [ -n "$previous" ] && [ -e "$previous" ] && [ ! -e "$dest" ]; then
    mv "$previous" "$dest" || true
  fi
  if [ -n "$staged" ] && [ -e "$staged" ]; then rm -rf "$staged"; fi
  if [ -n "$mounted" ]; then
    hdiutil detach "$mnt" -quiet 2>/dev/null || hdiutil detach "$mnt" -force -quiet 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 1' INT TERM
fetch() { curl --proto '=https' --proto-redir '=https' -fSsL "$1" -o "$2"; }
host=https://releases.sonojot.com
fetch "$host/latest.json" "$work/latest.json"
version=$(plutil -extract version raw -o - "$work/latest.json") || die "Invalid release metadata."
printf '%s\n' "$version" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' ||
  die "Invalid release version."
plutil -extract platforms.darwin-aarch64 json -o /dev/null "$work/latest.json" ||
  die "This release has no macOS build."
name="SonoJot_${version}_aarch64.dmg"
url="$host/v$version/$name"
fetch "$url.sha256" "$work/checksum"
checksum=$(cat "$work/checksum")
printf '%s\n' "$checksum" | grep -Eq '^[a-fA-F0-9]{64}$' || die "Invalid release checksum."
printf 'Downloading SonoJot %s...\n' "$version"
fetch "$url" "$work/$name"
actual=$(shasum -a 256 "$work/$name")
actual=${actual%% *}
[ "$actual" = "$checksum" ] || die "The disk image checksum does not match."

# Open the disk image in Downloads and let the user drag the app.
open_image() {
  if [ -n "$mounted" ]; then
    hdiutil detach "$mnt" -quiet 2>/dev/null || die "Cannot detach the disk image."
    mounted=
  fi
  mkdir -p "$HOME/Downloads"
  mv "$work/$name" "$HOME/Downloads/$name"
  open "$HOME/Downloads/$name"
  printf '%s\n' "$1" 'Drag SonoJot onto Applications in the disk image window. Run this command again to update.'
  exit 0
}

mkdir "$mnt"
hdiutil attach -readonly -nobrowse -noautoopen -noverify -mountpoint "$mnt" "$work/$name" >/dev/null 2>&1 ||
  die "Cannot open the disk image."
mounted=1
[ -d "$mnt/SonoJot.app" ] || die "The disk image does not contain SonoJot."

# Install directly only a Developer ID build from Cody Stevens Studio that
# Gatekeeper accepts; anything else takes the manual path.
signature=$(codesign -dv "$mnt/SonoJot.app" 2>&1) || signature=
if ! codesign --verify --deep --strict "$mnt/SonoJot.app" 2>/dev/null ||
  ! printf '%s\n' "$signature" | grep -qx "TeamIdentifier=$team" ||
  ! spctl --assess --type exec "$mnt/SonoJot.app" 2>/dev/null; then
  open_image "This SonoJot build is not notarized for direct install."
fi
if [ ! -w "$apps" ] || { [ -e "$dest" ] && [ ! -w "$dest" ]; }; then
  open_image "Your account cannot write to $apps."
fi

# Never stop a running copy: ask the user to quit it, and wait.
running() { pgrep -f "^$dest/Contents/MacOS/" >/dev/null 2>&1; }
if running; then
  printf '%s\n' 'SonoJot is open. Quit it from its menu bar icon (Quit SonoJot) to continue...'
  waited=0
  while running; do
    [ "$waited" -lt 120 ] || die "SonoJot is still open. Quit it, then run this command again."
    sleep 2
    waited=$((waited + 2))
  done
fi

printf 'Installing SonoJot %s in %s...\n' "$version" "$apps"
staged="$apps/.SonoJot.app.installing.$$"
ditto "$mnt/SonoJot.app" "$staged" || die "Cannot copy SonoJot to $apps."
if [ -e "$dest" ]; then
  previous="$apps/.SonoJot.app.previous.$$"
  mv "$dest" "$previous" || die "Cannot replace the existing SonoJot."
fi
mv "$staged" "$dest" || die "Cannot move SonoJot into place."
staged=
if [ -n "$previous" ]; then
  rm -rf "$previous"
  previous=
fi
open "$dest"
printf 'SonoJot %s is installed in %s and opening. Run this command again to update.\n' "$version" "$apps"
