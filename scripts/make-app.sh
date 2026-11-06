#!/bin/sh
# Builds Tessella.app, so that it opens from the Dock, Spotlight or the Finder and can
# post Mac notifications. It is signed for this Mac only (ad hoc), which needs no Apple
# account. Built on this Mac, macOS does not mark it as downloaded.
#
#   scripts/make-app.sh            # makes target/release/Tessella.app
#   scripts/make-app.sh --install  # and moves it into /Applications
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

cargo build --release
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/Tessella.app
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/tessella "$APP/Contents/MacOS/tessella"

# Every size macOS asks for, from the one icon.
ICONSET="$WORK/Tessella.iconset"
mkdir "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" docs/assets/icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" docs/assets/icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Tessella.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Tessella</string>
  <key>CFBundleDisplayName</key><string>Tessella</string>
  <key>CFBundleIdentifier</key><string>com.github.audiarmadhani.tessella</string>
  <key>CFBundleExecutable</key><string>tessella</string>
  <key>CFBundleIconFile</key><string>Tessella</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign - "$APP"
echo "Made $APP"

if [ "${1:-}" = --install ]; then
  rm -rf /Applications/Tessella.app
  cp -R "$APP" /Applications/Tessella.app
  echo "Installed /Applications/Tessella.app"
fi
