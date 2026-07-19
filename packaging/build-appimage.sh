#!/usr/bin/env bash
# Build a single-file Linux AppImage of Hook Echo-WX.
#
# The binary is nearly self-contained: TLS is static rustls (no libssl), D-Bus/tray/file-dialogs
# ride the D-Bus/portal socket (no libdbus/GTK link), X11/Wayland client libs are dlopened. The
# only bundled shared lib concern is libasound (ALSA); audio degrades gracefully if absent. We
# assemble a bare AppDir and package it with appimagetool — no linuxdeploy, since everything we
# would auto-bundle is on the AppImage excludelist anyway.
#
# Usage: packaging/build-appimage.sh
# Output: dist/Hook_Echo-WX-x86_64.AppImage
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ARCH=x86_64
APPDIR="$ROOT/target/appimage/HookEcho.AppDir"
OUT="$ROOT/dist/Hook_Echo-WX-${ARCH}.AppImage"
TOOL="$ROOT/packaging/appimagetool-${ARCH}.AppImage"

echo "==> building release binary"
cargo build --release -p hookecho

echo "==> assembling AppDir at $APPDIR"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
cp "$ROOT/target/release/hookecho" "$APPDIR/usr/bin/hookecho"

# Desktop entry (top level + the canonical applications dir).
cp "$ROOT/packaging/hookecho.desktop" "$APPDIR/hookecho.desktop"
mkdir -p "$APPDIR/usr/share/applications"
cp "$ROOT/packaging/hookecho.desktop" "$APPDIR/usr/share/applications/hookecho.desktop"

# Icon: our procedural logo (256x256), rendered by the binary we just built. Both the top-level
# icon (required by appimagetool) and the hicolor theme path.
echo "==> rendering icon"
"$APPDIR/usr/bin/hookecho" --headless-icon "$APPDIR/hookecho.png"
mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$APPDIR/hookecho.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/hookecho.png"

# AppRun -> the binary.
ln -sf usr/bin/hookecho "$APPDIR/AppRun"

# Fetch appimagetool if we don't have it.
if [ ! -x "$TOOL" ]; then
  echo "==> fetching appimagetool"
  curl -fL -o "$TOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
  chmod +x "$TOOL"
fi

echo "==> packaging"
mkdir -p "$ROOT/dist"
# APPIMAGE_EXTRACT_AND_RUN lets appimagetool run without FUSE (CI, containers).
ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" "$APPDIR" "$OUT"

echo "==> done: $OUT"
