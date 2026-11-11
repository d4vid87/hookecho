#!/usr/bin/env bash
# Encode native Android screenshots, keeping mobile-web media separate.
# Inputs are raw adb screencaps; set ANDROID_CAPTURE_CROP for the captured screen insets.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
frames="${1:-/tmp/hookecho-android-frames}"
captures="${2:-/tmp}"
site="$repo/site/public"
docs="$repo/docs/shots/android"
store="$repo/android/fastlane/metadata/android/en-US/images/phoneScreenshots"
crop="${ANDROID_CAPTURE_CROP:-1080:2208:0:132}"
[[ "$crop" =~ ^[0-9]+:[0-9]+:[0-9]+:[0-9]+$ ]]
IFS=: read -r crop_width crop_height crop_x crop_y <<< "$crop"
geometry="${crop_width}x${crop_height}+${crop_x}+${crop_y}"
prefix="android-current"
test -s "$frames/09.png"
test -s "$frames/capture.json"
mkdir -p "$docs" "$store"

filter="crop=$crop,scale=540:-2:flags=lanczos"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 -vf "$filter" \
  -c:v libx264 -preset medium -crf 22 -pix_fmt yuv420p -movflags +faststart \
  "$site/showcase/$prefix.mp4"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 -vf "$filter" \
  -c:v libvpx-vp9 -b:v 0 -crf 35 -pix_fmt yuv420p "$site/showcase/$prefix.webm"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 \
  -filter_complex "crop=$crop,scale=420:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3" \
  -loop 0 "$docs/hero.gif"

for name in map layers alerts more; do
  case "$name" in
    map) raw="$frames/02.png" ;;
    more) raw="$captures/hookecho-phone-menu.png" ;;
    *) raw="$captures/hookecho-phone-$name.png" ;;
  esac
  magick "$raw" -crop "$geometry" +repage -resize 50% -quality 82 "$docs/$name.jpg"
done
rm -f "$docs/site.jpg" "$store"/*.jpg
for pair in '1 map' '2 layers' '3 alerts' '4 more'; do
  read -r n name <<<"$pair"
  cp "$docs/$name.jpg" "$store/$n-$name.jpg"
done

cp "$docs/hero.gif" "$site/shots/hookecho-android.gif"
magick "$frames/02.png" -crop "$geometry" +repage -resize 420x \
  -quality 84 "$site/showcase/$prefix-poster.webp"
cp "$site/showcase/$prefix-poster.webp" "$site/shots/hookecho-android.webp"
ffprobe -v error -select_streams v:0 -show_entries stream=width,height,nb_frames \
  -of json "$site/showcase/$prefix.mp4"
echo 'Encoded current Android showcase, press GIF, and store screenshots.'
