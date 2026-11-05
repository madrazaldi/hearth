#!/bin/sh
# Regenerates every raster icon from the SVGs in this directory into
# crates/hearth-app/assets (the in-app logo, the tray icons and the installer icons).
# Needs macOS 14+ (AppKit SVG loading), swift and python3.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
assets="${HEARTH_ASSETS_DIR:-$root/crates/hearth-app/assets}"
icons="$assets/icons"
tray="$assets/tray"
tmp=$(mktemp -d)
iconset="$tmp/icon.iconset"
trap 'rm -rf "$tmp"' EXIT

render() { swift "$here/render.swift" "$@"; }

mkdir -p "$icons" "$tray" "$iconset"

set --
for s in 16 32 128 256 512; do
  set -- "$@" "$s:$iconset/icon_${s}x${s}.png" "$((s * 2)):$iconset/icon_${s}x${s}@2x.png"
done
for s in 24 48 64; do set -- "$@" "$s:$tmp/ico-$s.png"; done
render "$here/hearth-icon.svg" "$@" 256:"$assets/logo.png"

render "$here/hearth-template.svg" \
  18:"$tray/tray-template.png" \
  36:"$tray/tray-template@2x.png"
# The tray icon on Windows and Linux, where there are no template images.
cp "$tmp/ico-64.png" "$tray/tray-icon.png"

# The macOS bundle icon.
iconutil -c icns "$iconset" -o "$icons/icon.icns"

python3 "$here/make_ico.py" "$icons/icon.ico" \
  "$iconset/icon_16x16.png" "$tmp/ico-24.png" "$iconset/icon_32x32.png" \
  "$tmp/ico-48.png" "$tmp/ico-64.png" "$iconset/icon_256x256.png"

# The PNG sizes the Linux packages install (cargo-packager reads @2x as high density).
cp "$iconset/icon_32x32.png" "$icons/32x32.png"
cp "$iconset/icon_128x128.png" "$icons/128x128.png"
cp "$iconset/icon_128x128@2x.png" "$icons/128x128@2x.png"
cp "$iconset/icon_512x512.png" "$icons/512x512.png"

echo "icons written to $assets"
