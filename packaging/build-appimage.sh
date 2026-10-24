#!/usr/bin/env bash
# Build a single-file Linux AppImage of HookEcho.
#
# The binary is nearly self-contained: TLS is static rustls (no libssl), D-Bus/tray/file-dialogs
# ride the D-Bus/portal socket (no libdbus/GTK link), X11/Wayland client libs are dlopened. The
# audio backend links ALSA directly, so bundle it before the loader starts the app. Build on
# Ubuntu 22.04 (glibc 2.35); verify every bundled ELF against that floor before packaging.
#
# Usage: packaging/build-appimage.sh [--radar-workspace]
# Output: dist/HookEcho-x86_64.AppImage
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ARCH=x86_64
variant=HookEcho
desktop=hookecho
metadata=io.hookecho.HookEcho
assets="$ROOT/packaging"
if [[ "${1:-}" == --radar-workspace ]]; then
  variant=HookEcho-Radar-Workspace
  desktop=hookecho-radar-workspace
  metadata=io.hookecho.RadarWorkspace
  assets="$ROOT/packaging/radar-workspace"
elif [[ $# -ne 0 ]]; then
  echo "Usage: $0 [--radar-workspace]" >&2
  exit 2
fi
APPDIR="$ROOT/target/appimage/$variant.AppDir"
OUT="$ROOT/dist/${variant}-${ARCH}.AppImage"
TOOL="$ROOT/packaging/appimagetool-${ARCH}.AppImage"

echo "==> building release binary"
cargo build --release -p hookecho

echo "==> assembling AppDir at $APPDIR"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
cp "${CARGO_TARGET_DIR:-$ROOT/target}/release/hookecho" "$APPDIR/usr/bin/hookecho"
mkdir -p "$APPDIR/usr/lib"
alsa="$(ldd "$APPDIR/usr/bin/hookecho" | awk '$1 == "libasound.so.2" {print $3}')"
[[ -f "$alsa" ]] || { echo "Missing libasound.so.2; install the supported build dependencies" >&2; exit 1; }
cp -L "$alsa" "$APPDIR/usr/lib/libasound.so.2"

"$ROOT/packaging/piper/fetch.sh" linux-x86_64
mkdir -p "$APPDIR/usr/lib/hookecho/piper"
cp -a "$ROOT/target/piper/linux-x86_64/." "$APPDIR/usr/lib/hookecho/piper/"
# Keep language data for the voice picker and custom models; only standalone tools are unused.
voice="$APPDIR/usr/lib/hookecho/piper"
rm -f "$voice/piper_phonemize" "$voice/espeak-ng"
mkdir -p "$APPDIR/usr/share/doc/hookecho"
cp "$ROOT/LICENSE" "$APPDIR/usr/share/doc/hookecho/LICENSE"
cp "$ROOT/packaging/THIRD-PARTY-NOTICES.txt" "$APPDIR/usr/share/doc/hookecho/"
cp "$ROOT/LICENSE" "$voice/ESpeak-NG-LICENSE-GPL-3.0"
curl -fsSL https://raw.githubusercontent.com/microsoft/onnxruntime/v1.14.1/LICENSE -o "$voice/ONNXRuntime-LICENSE-MIT"
curl -fsSL https://raw.githubusercontent.com/rhasspy/piper-phonemize/2023.11.14-2/LICENSE.md -o "$voice/Piper-Phonemize-LICENSE-MIT"
curl -fsSL https://raw.githubusercontent.com/rhasspy/piper-phonemize/2023.11.14-2/licenses/uni-algo/LICENSE.md -o "$voice/Uni-Algo-LICENSE"
cp /usr/share/doc/libasound2/copyright "$APPDIR/usr/share/doc/hookecho/ALSA-copyright"

# Desktop entry (top level + the canonical applications dir).
cp "$assets/$desktop.desktop" "$APPDIR/$desktop.desktop"
mkdir -p "$APPDIR/usr/share/applications"
cp "$assets/$desktop.desktop" "$APPDIR/usr/share/applications/$desktop.desktop"
if [[ "$desktop" == hookecho-radar-workspace ]]; then
  cp "$assets/launcher" "$APPDIR/usr/bin/$desktop"
  chmod +x "$APPDIR/usr/bin/$desktop"
fi

# Icon: the app logo, rendered by the binary we just built. Every hicolor size gets its own file
# so the desktop picks one rather than scaling a 256 px PNG down to a 48 px slot itself.
echo "==> rendering icons"
"$APPDIR/usr/bin/hookecho" --headless-icon "$APPDIR/$desktop.png" 256
for px in 48 64 128 256 512; do
  dir="$APPDIR/usr/share/icons/hicolor/${px}x${px}/apps"
  mkdir -p "$dir"
  "$APPDIR/usr/bin/hookecho" --headless-icon "$dir/$desktop.png" "$px"
done

# AppStream metadata, so software centres have something to show.
mkdir -p "$APPDIR/usr/share/metainfo"
cp "$assets/$metadata.metainfo.xml" "$APPDIR/usr/share/metainfo/"

# AppRun adds the bundled voice engine to the process environment.
cp "$assets/AppRun" "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun"

# Fetch appimagetool if we don't have it.
if [ ! -x "$TOOL" ]; then
  echo "==> fetching appimagetool"
  curl -fL -o "$TOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
  chmod +x "$TOOL"
fi

echo "==> verifying AppDir"
bash "$ROOT/scripts/checks/appimage.sh" "$APPDIR"
echo "==> packaging"
mkdir -p "$ROOT/dist"
# APPIMAGE_EXTRACT_AND_RUN lets appimagetool run without FUSE (CI, containers).
ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" "$APPDIR" "$OUT"

echo "==> done: $OUT"
