#!/usr/bin/env bash
# Mobile web capture; never describe these assets as native Android screenshots.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
frames="${1:-/tmp/hookecho-mobile-frames}"
site="$repo/site/public/showcase"
docs="$repo/docs/shots/mobile"
test -s "$frames/09.png"
test -s "$frames/capture.json"
mkdir -p "$site" "$docs"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 \
  -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p -movflags +faststart "$site/phone-current.mp4"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 \
  -c:v libvpx-vp9 -b:v 0 -crf 32 "$site/phone-current.webm"
ffmpeg -y -loglevel error -framerate 2 -i "$frames/%02d.png" -frames:v 10 \
  -filter_complex 'split[a][b];[a]palettegen=max_colors=192[p];[b][p]paletteuse=dither=bayer:bayer_scale=3' \
  -loop 0 "$docs/hero.gif"
magick "$frames/00.png" -quality 88 "$site/phone-current-poster.webp"
ffprobe -v error -select_streams v:0 -show_entries stream=width,height,nb_frames -of json "$site/phone-current.mp4"
