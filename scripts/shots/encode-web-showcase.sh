#!/usr/bin/env bash
# Encode the current app captures from capture-web-showcase.cjs.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
frames="${1:-/tmp/hookecho-showcase}"
showcase="$repo/site/public/showcase"
shots="$repo/docs/shots"

encode_mp4() {
  local scene="$1" output="$2" filter="$3"
  ffmpeg -y -loglevel error -framerate 2 -i "$frames/$scene/%02d.png" \
    -vf "$filter" -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p \
    -movflags +faststart "$showcase/$output.mp4"
}
encode_poster() {
  local scene="$1" output="$2" filter="$3"
  ffmpeg -y -loglevel error -i "$frames/$scene/02.png" -vf "$filter" -frames:v 1 \
    "$showcase/$output.webp"
}
encode_gif() {
  local scene="$1" output="$2" filter="$3"
  ffmpeg -y -loglevel error -framerate 2 -i "$frames/$scene/%02d.png" \
    -filter_complex "$filter,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3" \
    -loop 0 "$output"
}

full='crop=1280:720:0:40'
encode_mp4 radar moore-2013-single "$full"
encode_poster radar moore-2013-single "$full"
encode_gif radar "$showcase/moore-2013-single.gif" 'scale=900:-1'
encode_mp4 radar radar-playback-20261004 "$full"
encode_mp4 analyst moore-2013-four "$full"
encode_poster analyst moore-2013-four "$full"
encode_mp4 attributes attributes-20261004 "$full"
encode_mp4 national mrms-20261004 "$full"

for item in 'reflectivity 10 65' 'velocity 500 65' 'correlation 10 420' 'differential 500 420'; do
  read -r product x y <<<"$item"
  filter="crop=480:270:$x:$y,scale=640:360"
  encode_mp4 analyst "moore-2013-$product" "$filter"
  encode_poster analyst "moore-2013-$product" "$filter"
done

# The README and press kit use short, current UI loops too.
encode_gif radar "$shots/radar-mode.gif" 'scale=900:-1'
encode_gif analyst "$shots/analyst-mode.gif" 'scale=900:-1'
magick "$frames/radar/02.png" -quality 82 "$shots/radar-mode.jpg"
magick "$frames/analyst/02.png" -quality 80 "$shots/analyst-mode.jpg"
magick "$frames/national/02.png" -quality 82 "$shots/mrms.jpg"
magick "$frames/attributes/02.png" -quality 82 "$shots/stormtable.jpg"

echo 'Encoded current web showcase and README media.'
