#!/usr/bin/env bash
# Builds release binaries and assembles target/release/Dove.app
# (desktop app + dove-mesh-host sidecar for remote control).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
APP="$ROOT/target/release/Dove.app"

cargo build --release --locked -p dove --manifest-path "$ROOT/Cargo.toml"
cargo build --release --locked -p dove-mesh-host --manifest-path "$ROOT/mesh/Cargo.toml"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/target/release/dove" "$APP/Contents/MacOS/dove"
cp "$ROOT/mesh/target/release/dove-mesh-host" "$APP/Contents/MacOS/dove-mesh-host"

ICON="$(find "$ROOT/apps/ios" -name '*1024*.png' -print -quit 2>/dev/null || true)"
if [[ -n "$ICON" ]] && command -v iconutil >/dev/null; then
  SET="$(mktemp -d)/Dove.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    sips -z $s $s "$ICON" --out "$SET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) "$ICON" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$SET" -o "$APP/Contents/Resources/Dove.icns"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Dove</string>
  <key>CFBundleDisplayName</key><string>Dove</string>
  <key>CFBundleIdentifier</key><string>sh.dove.desktop</string>
  <key>CFBundleExecutable</key><string>dove</string>
  <key>CFBundleIconFile</key><string>Dove</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Dove uses the microphone for voice input.</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" 2>/dev/null || true
du -sh "$APP"
