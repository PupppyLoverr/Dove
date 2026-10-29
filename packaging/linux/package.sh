#!/usr/bin/env bash
# Linux packages from release binaries already built:
#   dove_<version>_amd64.deb         (apt install ./dove_*.deb)
#   Keplorayx-<version>-x86_64.AppImage   (chmod +x and run; uses the system's
#                                     GTK/WebKit, Vulkan and Wayland/X11 libs)
#   dove-<version>-linux-x86_64.tar.gz
# Needs: dpkg-deb; APPIMAGETOOL pointing at appimagetool for the AppImage.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${BIN:-$ROOT/target/release}"
MESH_BIN="${MESH_BIN:-$ROOT/mesh/target/release}"
OUT="${OUT:-$ROOT/packaging/out}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ICON="$ROOT/apps/dove/assets/keplorayx-icon.png"
mkdir -p "$OUT"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# One install tree, reused by every format: /usr/lib/dove holds the binaries
# (sidecars are found beside dove), /usr/bin/dove links to it.
stage() {
  local root="$1"
  mkdir -p "$root/usr/lib/dove" "$root/usr/bin" "$root/usr/share/applications" \
    "$root/usr/share/icons/hicolor/256x256/apps" "$root/usr/share/icons/hicolor/1024x1024/apps"
  install -m 755 "$BIN/dove" "$root/usr/lib/dove/dove"
  if [[ -f "$MESH_BIN/dove-mesh-host" ]]; then
    install -m 755 "$MESH_BIN/dove-mesh-host" "$root/usr/lib/dove/dove-mesh-host"
  else
    echo "warning: dove-mesh-host not built; remote control will be unavailable" >&2
  fi
  ln -sf ../lib/dove/dove "$root/usr/bin/dove"
  install -m 644 "$ROOT/packaging/linux/dove.desktop" "$root/usr/share/applications/dove.desktop"
  install -m 644 "$ICON" "$root/usr/share/icons/hicolor/1024x1024/apps/dove.png"
  if command -v convert >/dev/null; then
    convert "$ICON" -resize 256x256 "$root/usr/share/icons/hicolor/256x256/apps/dove.png"
  else
    install -m 644 "$ICON" "$root/usr/share/icons/hicolor/256x256/apps/dove.png"
  fi
}

# .deb
DEB="$WORK/deb"
stage "$DEB"
mkdir -p "$DEB/DEBIAN"
SIZE_KB="$(du -sk "$DEB/usr" | cut -f1)"
cat > "$DEB/DEBIAN/control" <<CONTROL
Package: dove
Version: ${VERSION}
Section: devel
Priority: optional
Architecture: amd64
Maintainer: Keplorayx <noreply@dove.sh>
Installed-Size: ${SIZE_KB}
Depends: libc6 (>= 2.38), libgcc-s1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libx11-xcb1, libxcb1, libfontconfig1, libfreetype6, libssl3 | libssl3t64, libasound2t64 | libasound2, libwebkit2gtk-4.1-0, libgtk-3-0t64 | libgtk-3-0, libjson-glib-1.0-0
Recommends: git, mesa-vulkan-drivers
Homepage: https://github.com/PupppyLoverr/Dove
Description: Keplorayx (K22), a desktop environment for coding agents
 Runs Claude Code, Codex, Gemini and other agent CLIs side by side, with
 threads, diffs, a terminal and a built-in browser.
CONTROL
dpkg-deb --root-owner-group --build "$DEB" "$OUT/dove_${VERSION}_amd64.deb"

# tar.gz: the same tree, relative to a prefix.
TAR="$WORK/tar/dove-${VERSION}"
stage "$TAR"
tar -C "$WORK/tar" -czf "$OUT/dove-${VERSION}-linux-x86_64.tar.gz" "dove-${VERSION}"

# AppImage
if [[ -n "${APPIMAGETOOL:-}" ]]; then
  APPDIR="$WORK/Dove.AppDir"
  stage "$APPDIR"
  install -m 644 "$ROOT/packaging/linux/dove.desktop" "$APPDIR/dove.desktop"
  install -m 644 "$ICON" "$APPDIR/dove.png"
  cat > "$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/lib/dove/dove" "$@"
APPRUN
  chmod 755 "$APPDIR/AppRun"
  ARCH=x86_64 "$APPIMAGETOOL" --no-appstream "$APPDIR" "$OUT/Keplorayx-${VERSION}-x86_64.AppImage"
else
  echo "APPIMAGETOOL not set; skipping the AppImage" >&2
fi
ls -la "$OUT"
