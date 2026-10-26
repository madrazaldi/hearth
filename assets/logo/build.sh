#!/bin/sh
# Regenerates every raster icon from the SVGs in this directory.
# Needs macOS 14+ (AppKit SVG loading), swift and python3.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
icons="$root/assets/icons"
iconset="$root/assets/icon.iconset"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

render() { swift "$here/render.swift" "$@"; }

mkdir -p "$icons"
rm -rf "$iconset"
mkdir -p "$iconset"

set -- 
for s in 16 32 128 256 512; do
  set -- "$@" "$s:$iconset/icon_${s}x${s}.png" "$((s * 2)):$iconset/icon_${s}x${s}@2x.png"
done
for s in 24 48 64; do set -- "$@" "$s:$tmp/ico-$s.png"; done
render "$here/hearth-icon.svg" "$@" \
  512:"$icons/icon.png" \
  256:"$root/src/assets/logo.png"

render "$here/hearth-template.svg" \
  18:"$here/tray-template.png" \
  36:"$here/tray-template@2x.png"

# The macOS bundle icon, for tools that want a ready-made .icns.
iconutil -c icns "$iconset" -o "$here/AppIcon.icns"

python3 "$here/make_ico.py" "$icons/icon.ico" \
  "$iconset/icon_16x16.png" "$tmp/ico-24.png" "$iconset/icon_32x32.png" \
  "$tmp/ico-48.png" "$tmp/ico-64.png" "$iconset/icon_256x256.png"

echo "icons written"
