#!/usr/bin/env bash
# macOS packaging: build the release binaries for the host arch and produce
#   target/package/Keplorayx-<version>-macos-<arch>.dmg          (user download)
#   target/package/Keplorayx-<version>-macos-<arch>-app.tar.gz   (the updater's)
# containing Keplorayx.app (ad-hoc signed unless CODESIGN_IDENTITY is set).
#
# Two binaries ship, side by side in Contents/MacOS, because the app starts the
# engine from its own directory (apps/zgx/src/engine/manager.rs):
#
#   keplorayx         zr/apps/zgx — the interface, and `zeron`'s command line
#   keplorayx-engine  zr           — the background engine it starts and attaches to
#
# The interface is a workspace of its own (`zr/apps/zgx`, its own lock and
# patches); the engine is `-p zeron --bin keplorayx-engine` in `zr`'s.
#
# Usage: scripts/package-macos.sh
# Env:   CODESIGN_IDENTITY="Developer ID Application: …" to sign the bundle.
#        NOTARY_KEY_PATH + NOTARY_KEY_ID + NOTARY_ISSUER_ID — App Store Connect
#        API key (.p8) for notarization; all three set → notarize + staple the
#        app and the dmg, which removes the Gatekeeper warning entirely.
#        DMG=0 — the bundle and its tarball only (CI's dry run).
#        CARGO_FLAGS=--locked in CI; CARGO_TARGET_DIR to build outside both
#        workspaces' own target dirs, or APP_TARGET_DIR and ENGINE_TARGET_DIR
#        to point the two builds at them separately.
#
# What is NOT here yet: the in-app updater. crates/update looks for
# `Zeron.app`, its launchd label `sh.zeron.app` and a `Contents/MacOS/zeron`
# bundle (src/lib.rs:1054, src/lib.rs:992, src/lib.rs:1907), all of which are
# the Zeron bundle's names. A Keplorayx.app therefore installs and runs but
# does not self-update until those names are parameters. The dmg is the
# install path until then.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
ARCH="$(uname -m)" # arm64 on Apple silicon runners
read -r -a CARGO_EXTRA <<<"${CARGO_FLAGS:-}"

# Each workspace has its own target dir (they set different profiles), and
# CARGO_TARGET_DIR overrides both. CI points the two at the dirs its build job
# filled, so a dry run is incremental rather than a second cold compile.
APP_TARGET="${APP_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/apps/zgx/target}}"
ENGINE_TARGET="${ENGINE_TARGET_DIR:-${CARGO_TARGET_DIR:-$ROOT/target}}"
APP_BIN="$APP_TARGET/release/keplorayx"
ENGINE_BIN="$ENGINE_TARGET/release/keplorayx-engine"

OUT_DIR="$ROOT/target/package"

echo "building the engine"
(cd "$ROOT" && cargo build --release ${CARGO_EXTRA[@]+"${CARGO_EXTRA[@]}"} -p zeron --bin keplorayx-engine --target-dir "$ENGINE_TARGET")
echo "building the app"
(cd "$ROOT/apps/zgx" && cargo build --release ${CARGO_EXTRA[@]+"${CARGO_EXTRA[@]}"} --target-dir "$APP_TARGET")

for bin in "$APP_BIN" "$ENGINE_BIN"; do
  [[ -x "$bin" ]] || { echo "no binary at $bin" >&2; exit 1; }
done

# The app's own version, which is what the updater compares releases with
# (apps/zgx/build.rs's ZERON_APP_VERSION; `zeron --version` prints it).
VERSION="$("$APP_BIN" --version | sed -n 's/^[a-z-]* //p' | tr -d '\r')"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-beta\.[0-9]+)?$ ]] ||
  { echo "$APP_BIN didn't say its version (it said '$VERSION')" >&2; exit 1; }
echo "version: $VERSION"

APP="$OUT_DIR/Keplorayx.app"
DMG="$OUT_DIR/Keplorayx-$VERSION-macos-$ARCH.dmg"
APP_TARBALL="$OUT_DIR/Keplorayx-$VERSION-macos-$ARCH-app.tar.gz"

rm -rf "$APP" "$DMG" "$APP_TARBALL"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

install -m 755 "$APP_BIN" "$APP/Contents/MacOS/keplorayx"
install -m 755 "$ENGINE_BIN" "$APP/Contents/MacOS/keplorayx-engine"

# The scripts and the built-in plugins, where apps/zgx/src/main.rs's ui_dir()
# reads them from a bundle. (Not zgx's generated types or the tests.)
rsync -a --exclude '.zgx/' --exclude '/tests/' --exclude '.lock' --exclude '.DS_Store' \
  "$ROOT/apps/zgx/ui/" "$APP/Contents/Resources/ui/"
if [[ -d "$ROOT/apps/zgx/plugins-builtin" ]]; then
  rsync -a --exclude '.zgx/' --exclude '.DS_Store' \
    "$ROOT/apps/zgx/plugins-builtin/" "$APP/Contents/Resources/plugins-builtin/"
fi

# Font licences: zr/crates/ui's bundled faces, and each of the app's own OFL
# texts. (zr/crates/ui still holds the fonts' licences even though nothing
# links the crate: three packaging scripts read this folder.)
LICENSES="$APP/Contents/Resources/licenses/fonts"
mkdir -p "$LICENSES"
cp "$ROOT/crates/ui/assets/fonts/licenses/"* "$LICENSES/"
cp "$ROOT/apps/zgx/fonts/"*-OFL.txt "$LICENSES/"
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/LICENSE" "$APP/Contents/Resources/licenses/"
cp "$ROOT/crates/voice/NOTICE.md" "$APP/Contents/Resources/licenses/parakeet-v3.txt"

# The icon, from the app's 1024px source (sips cannot alpha-mask, so a masked
# render goes here if one is ever committed; this is the full-bleed artwork).
ICON_SRC=""
for candidate in "$ROOT/dist/macos/icon-1024.png" \
                 "$REPO/apps/ios/Zeron/Assets.xcassets/AppIcon.appiconset/AppIcon1024.png"; do
  if [[ -f "$candidate" ]]; then
    ICON_SRC="$candidate"
    break
  fi
done
if [[ -n "$ICON_SRC" ]]; then
  ICONSET="$OUT_DIR/keplorayx.iconset"
  rm -rf "$ICONSET" && mkdir -p "$ICONSET"
  for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Keplorayx.icns"
  rm -rf "$ICONSET"
else
  echo "no 1024px icon found: bundling without one" >&2
fi

# zgx's floor is macOS 13 (Ghostty's). The `zeron://` scheme is the one
# crates/notify writes links for and crates/update registers on Windows.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Keplorayx</string>
  <key>CFBundleDisplayName</key><string>Keplorayx</string>
  <key>CFBundleIdentifier</key><string>com.keplorayx.app</string>
  <key>CFBundleExecutable</key><string>keplorayx</string>
  <key>CFBundleIconFile</key><string>Keplorayx</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Keplorayx uses the microphone for voice input.</string>
  <key>NSAppleEventsUsageDescription</key><string>Keplorayx asks other apps for accessibility so it can drive them for its agents.</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>zeron</string>
      <key>CFBundleURLSchemes</key>
      <array><string>zeron</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# Signed with Zeron's entitlements when they are in the tree (dist/macos is
# not, on this branch); ad-hoc otherwise.
SIGN_EXTRA=()
if [[ -f "$ROOT/dist/macos/Zeron.entitlements" ]]; then
  SIGN_EXTRA=(--entitlements "$ROOT/dist/macos/Zeron.entitlements")
fi

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
  # Hardened runtime + secure timestamp are both notarization requirements.
  # Nested code is signed first, inside out: the engine, then the bundle that
  # seals it. (No --deep: Apple deprecated it.)
  codesign "${SIGN_EXTRA[@]+"${SIGN_EXTRA[@]}"}" --force --options runtime --timestamp \
    --sign "$CODESIGN_IDENTITY" --identifier com.keplorayx.app.engine "$APP/Contents/MacOS/keplorayx-engine"
  codesign "${SIGN_EXTRA[@]+"${SIGN_EXTRA[@]}"}" --force --options runtime --timestamp \
    --sign "$CODESIGN_IDENTITY" "$APP"
else
  # Ad-hoc, so the app launches on Apple silicon (Gatekeeper still requires
  # right-click → Open on first launch without notarization).
  codesign "${SIGN_EXTRA[@]+"${SIGN_EXTRA[@]}"}" --force --deep --sign - "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"

# notarize <path>: submit to Apple and wait for the verdict. A rejection may
# still exit 0 depending on the notarytool version — the `stapler staple` that
# follows each call has no ticket to attach then, and fails the build for us.
notarize() {
  xcrun notarytool submit "$1" \
    --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" \
    --issuer "$NOTARY_ISSUER_ID" --wait
}
NOTARIZE=false
if [[ -n "${NOTARY_KEY_PATH:-}" && -n "${NOTARY_KEY_ID:-}" && -n "${NOTARY_ISSUER_ID:-}" ]]; then
  NOTARIZE=true
fi

if $NOTARIZE; then
  # Staple the bundle BEFORE tarring it: the updater swaps the .app with no
  # dmg involved, so the tarball copy must carry its own ticket to pass
  # Gatekeeper offline.
  ZIP="$OUT_DIR/keplorayx-notarize.zip"
  ditto -c -k --keepParent "$APP" "$ZIP"
  notarize "$ZIP"
  rm -f "$ZIP"
  xcrun stapler staple "$APP"
fi

tar -czf "$APP_TARBALL" -C "$OUT_DIR" Keplorayx.app
echo "packaged: $APP_TARBALL"

if [[ "${DMG:-1}" == "0" ]]; then
  echo "size: $(du -sh "$APP" | cut -f1) · app $(du -h "$APP/Contents/MacOS/keplorayx" | cut -f1) · engine $(du -h "$APP/Contents/MacOS/keplorayx-engine" | cut -f1)"
  exit 0
fi

# The dmg: the bundle beside an Applications symlink, so it drags and drops.
STAGE="$OUT_DIR/dmg"
rm -rf "$STAGE" && mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Keplorayx.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "Keplorayx" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"
if $NOTARIZE; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
fi
echo "packaged: $DMG"
echo "size: $(du -sh "$APP" | cut -f1) · app $(du -h "$APP/Contents/MacOS/keplorayx" | cut -f1) · engine $(du -h "$APP/Contents/MacOS/keplorayx-engine" | cut -f1)"
