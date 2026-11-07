#!/bin/sh
# Installs what GPUI, the keychain (libdbus) and the packaging tools need on a
# Debian/Ubuntu machine. CI uses it; developers can too.
#
#   scripts/install-linux-deps.sh            build and test dependencies
#   scripts/install-linux-deps.sh packaging  also what cargo-packager needs for an AppImage
set -eu

sudo=""
[ "$(id -u)" -eq 0 ] || sudo="sudo"

packages="
  build-essential clang pkg-config
  libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev
  libssl-dev libzstd-dev libasound2-dev libdbus-1-dev libvulkan1
"
if [ "${1:-}" = "packaging" ]; then
  # The AppImage tools are themselves AppImages and need FUSE (or extraction).
  packages="$packages libfuse2 file xdg-utils squashfs-tools"
fi

$sudo apt-get update
# shellcheck disable=SC2086
$sudo apt-get install -y --no-install-recommends $packages
