#!/bin/sh
# Builds Hearth.app: the Rust core as a static library, then the SwiftUI app,
# then the bundle around it.
#
#   ./build.sh            debug build of the current architecture
#   ./build.sh release    universal (arm64 + x86_64) release build
#   ./build.sh --run      build, then open the app
#
# Output: macos/build/<config>/Hearth.app
set -eu

cd "$(dirname "$0")"
here=$(pwd)
root=$(cd .. && pwd)

config=debug
run=0
for arg in "$@"; do
  case $arg in
    release) config=release ;;
    debug) config=debug ;;
    --run) run=1 ;;
    *) echo "usage: $0 [debug|release] [--run]" >&2; exit 2 ;;
  esac
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)

if [ $config = release ]; then
  export MACOSX_DEPLOYMENT_TARGET=14.0
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --manifest-path "$root/Cargo.toml" -p hearth-ffi --release --target $t
  done
  mkdir -p "$root/target/universal/release"
  lipo -create -output "$root/target/universal/release/libhearth_ffi.a" \
    "$root/target/aarch64-apple-darwin/release/libhearth_ffi.a" \
    "$root/target/x86_64-apple-darwin/release/libhearth_ffi.a"
  swift build -c release --arch arm64 --arch x86_64
  bin=$(swift build -c release --arch arm64 --arch x86_64 --show-bin-path)/Hearth
else
  cargo build --manifest-path "$root/Cargo.toml" -p hearth-ffi
  swift build
  bin=$(swift build --show-bin-path)/Hearth
fi

app="$here/build/$config/Hearth.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/Hearth"
cp "$root/assets/logo/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
cp "$root/assets/logo/tray-template.png" "$app/Contents/Resources/MenuBarIcon.png"
cp "$root/assets/logo/tray-template@2x.png" "$app/Contents/Resources/MenuBarIcon@2x.png"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>app.hearth.irc</string>
  <key>CFBundleName</key><string>Hearth</string>
  <key>CFBundleDisplayName</key><string>Hearth</string>
  <key>CFBundleExecutable</key><string>Hearth</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
EOF

# Ad-hoc by default; set HEARTH_SIGN_IDENTITY to sign for distribution.
if [ -n "${HEARTH_SIGN_IDENTITY:-}" ]; then
  codesign --force --options runtime --timestamp --sign "$HEARTH_SIGN_IDENTITY" "$app"
else
  codesign --force --sign - "$app"
fi

echo "Built $app"
if [ $run = 1 ]; then open "$app"; fi
