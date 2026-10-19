#!/usr/bin/env bash
# Dove-<version>-<arch>.dmg: Dove.app (from scripts/macos/bundle.sh) beside an
# Applications link, to drag across. The app is ad-hoc signed, not notarized:
# the first launch needs right-click > Open (or
# `xattr -dr com.apple.quarantine /Applications/Dove.app`).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$ROOT/packaging/out}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ARCH="$(uname -m)"
[[ "$ARCH" == "arm64" ]] || ARCH="x86_64"

if [[ -z "${SKIP_BUNDLE:-}" ]]; then
  "$ROOT/scripts/macos/bundle.sh"
fi
APP="$ROOT/target/release/Dove.app"
[[ -d "$APP" ]] || { echo "missing $APP" >&2; exit 1; }

mkdir -p "$OUT"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cp -R "$APP" "$WORK/Dove.app"
ln -s /Applications "$WORK/Applications"
DMG="$OUT/Dove-${VERSION}-${ARCH}.dmg"
rm -f "$DMG"
hdiutil create -volname "Dove" -srcfolder "$WORK" -ov -format UDZO "$DMG"
echo "dmg: $DMG"
