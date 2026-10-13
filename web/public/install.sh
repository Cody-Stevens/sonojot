#!/bin/sh
# Download and open the verified SonoJot disk image for macOS on Apple Silicon.
set -eu
die() { printf 'sonojot: %s\n' "$*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] ||
  die "This installer supports only macOS on Apple Silicon. Windows: https://sonojot.com/install.ps1"
for tool in curl plutil shasum open; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is required."
done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
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
mkdir -p "$HOME/Downloads"
mv "$work/$name" "$HOME/Downloads/$name"
open "$HOME/Downloads/$name"
printf '%s\n' 'Drag SonoJot to Applications in the disk image window. Run this command again to update.'
