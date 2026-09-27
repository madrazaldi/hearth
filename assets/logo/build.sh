#!/bin/sh
# Regenerates every raster icon from the SVGs in this directory.
# Needs macOS 14+ (AppKit SVG loading), swift, iconutil and python3.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
icons="$root/src-tauri/icons"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

render() { swift "$here/render.swift" "$@"; }

set -- 
for s in 16 32 128 256 512; do
  set -- "$@" "$s:$tmp/icon_${s}x${s}.png" "$((s * 2)):$tmp/icon_${s}x${s}@2x.png"
done
for s in 24 48 64; do set -- "$@" "$s:$tmp/ico-$s.png"; done
render "$here/hearth-icon.svg" "$@" \
  32:"$icons/32x32.png" \
  64:"$icons/64x64.png" \
  128:"$icons/128x128.png" \
  256:"$icons/128x128@2x.png" \
  512:"$icons/icon.png" \
  30:"$icons/Square30x30Logo.png" \
  44:"$icons/Square44x44Logo.png" \
  71:"$icons/Square71x71Logo.png" \
  89:"$icons/Square89x89Logo.png" \
  107:"$icons/Square107x107Logo.png" \
  142:"$icons/Square142x142Logo.png" \
  150:"$icons/Square150x150Logo.png" \
  284:"$icons/Square284x284Logo.png" \
  310:"$icons/Square310x310Logo.png" \
  50:"$icons/StoreLogo.png" \
  256:"$root/src/assets/logo.png"

render "$here/hearth-template.svg" \
  18:"$here/tray-template.png" \
  36:"$here/tray-template@2x.png"

mkdir "$tmp/icon.iconset"
mv "$tmp"/icon_*.png "$tmp/icon.iconset/"
iconutil -c icns "$tmp/icon.iconset" -o "$icons/icon.icns"
cp "$icons/icon.icns" "$here/AppIcon.icns"

i="$tmp/icon.iconset"
python3 "$here/make_ico.py" "$icons/icon.ico" \
  "$i/icon_16x16.png" "$tmp/ico-24.png" "$i/icon_32x32.png" \
  "$tmp/ico-48.png" "$tmp/ico-64.png" "$i/icon_256x256.png"

echo "icons written"
