#!/usr/bin/env bash
# Regenerate every derived icon from the one brand master, assets/brand/hookecho-logo.png.
#
# Run it after replacing the master; the outputs are committed, so this is a "make the assets"
# script rather than a build step — nothing in CI depends on ImageMagick being installed.
set -euo pipefail
cd "$(dirname "$0")/../.."
src=assets/brand/hookecho-logo.png
[ -f "$src" ] || { echo "icons.sh: missing $src" >&2; exit 1; }

# The copy compiled into the binary and the wasm. 256 px is the largest size anything asks for
# (the .ico's top entry), and the palette quantise keeps it ~24 KB instead of ~110 KB — real money
# on the web build, where this rides in the wasm on a first visit.
magick "$src" -resize 256x256 -dither FloydSteinberg -colors 128 -strip \
    PNG8:crates/hookecho/data/logo.png

# PWA icons. 192 and 512 are what web/manifest.webmanifest names.
for s in 192 512; do
    magick "$src" -resize "${s}x${s}" -strip "web/icon-$s.png"
done

# Windows installer icon: one .ico carrying every size the shell picks from.
magick "$src" -define icon:auto-resize=256,128,64,48,32,16 -strip packaging/windows/icon.ico

# Android adaptive icon. The foreground is a 108dp canvas whose inner 72dp is the safe zone —
# anything outside it can be masked off by the launcher's shape — so the mark is drawn at 2/3 and
# centred. The same asset is the splash mark (drawable/splash_screen.xml, values-v31/themes.xml).
res=android/app/src/main/res
for d in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
    dens=${d%%:*}; px=${d##*:}; inner=$((px * 2 / 3))
    magick "$src" -resize "${inner}x${inner}" -background none -gravity center \
        -extent "${px}x${px}" -strip "$res/mipmap-$dens/ic_launcher_foreground.png"
done

# Legacy square icon, for launchers older than adaptive icons.
magick "$src" -resize 256x256 -strip "$res/mipmap-xxxhdpi/ic_launcher.png"

# Themed-icon layer: white where the mark is bright, transparent elsewhere, tinted by the OS.
# A silhouette of the alpha channel would be one solid blob (the artwork is a filled badge), so
# the cut is on luminance instead — the scope rings, the storm and the wordmark survive it.
magick "$src" -resize 288x288 -background none -gravity center -extent 432x432 \
    \( +clone -colorspace gray -threshold 55% \) -alpha off -compose copy_opacity -composite \
    -fill white -colorize 100 -strip "$res/mipmap-xxxhdpi/ic_launcher_monochrome.png"

echo "icons.sh: regenerated from $src"
