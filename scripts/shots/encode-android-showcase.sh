#!/usr/bin/env bash
# Encode screenshots recorded from the current signed APK on a connected Android phone.
# Inputs are raw adb screencaps; the crop removes personal status-bar notifications.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
frames="${1:-/tmp/hookecho-android-frames}"
captures="${2:-/tmp}"
site="$repo/site/public"
docs="$repo/docs/shots/android"
store="$repo/android/fastlane/metadata/android/en-US/images/phoneScreenshots"
mkdir -p "$docs" "$store"

filter='crop=1440:2860:0:125,scale=540:-2:flags=lanczos'
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -vf "$filter" \
  -c:v libx264 -preset medium -crf 22 -pix_fmt yuv420p -movflags +faststart \
  "$site/showcase/phone-current.mp4"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -vf "$filter" \
  -c:v libvpx-vp9 -b:v 0 -crf 35 -pix_fmt yuv420p "$site/showcase/phone-current.webm"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" \
  -filter_complex 'crop=1440:2860:0:125,scale=420:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3' \
  -loop 0 "$docs/hero.gif"

for name in map layers alerts more; do
  case "$name" in
    map) raw="$frames/02.png" ;;
    more) raw="$captures/hookecho-phone-menu.png" ;;
    *) raw="$captures/hookecho-phone-$name.png" ;;
  esac
  magick "$raw" -crop 1440x2860+0+125 +repage -resize 50% -quality 82 "$docs/$name.jpg"
done
rm -f "$docs/site.jpg" "$store"/*.jpg
for pair in '1 map' '2 layers' '3 alerts' '4 more'; do
  read -r n name <<<"$pair"
  cp "$docs/$name.jpg" "$store/$n-$name.jpg"
done

cp "$docs/hero.gif" "$site/shots/hookecho-android.gif"
magick "$frames/02.png" -crop 1440x2860+0+125 +repage -resize 420x \
  -quality 84 "$site/showcase/phone-current-poster.webp"
cp "$site/showcase/phone-current-poster.webp" "$site/shots/hookecho-android.webp"
echo 'Encoded current Android showcase, press GIF, and store screenshots.'
