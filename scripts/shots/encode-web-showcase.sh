#!/usr/bin/env bash
# Encode complete, visually reviewed real-app frames from capture-web-showcase.cjs.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
frames="${1:-/tmp/hookecho-showcase}"
showcase="$repo/site/public/showcase"
shots="$repo/docs/shots"
mkdir -p "$showcase/national-to-local" "$shots/national-to-local"

encode_mp4() {
  local scene="$1" output="$2"
  ffmpeg -y -loglevel error -framerate 2 -i "$frames/$scene/%02d.png" \
    -c:v libx264 -preset medium -crf 21 -pix_fmt yuv420p -movflags +faststart "$showcase/$output.mp4"
}
encode_poster() {
  magick "$frames/$1/02.png" -strip -quality 85 "$showcase/$2.webp"
}
encode_gif() {
  local scene="$1" output="$2" width="$3" fps="${4:-2}"
  ffmpeg -y -loglevel error -framerate "$fps" -i "$frames/$scene/%02d.png" \
    -filter_complex "scale=$width:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3" \
    -loop 0 "$output"
}
still() {
  local input="$1" name="$2"
  for quality in 85 80 75 70 65; do
    magick "$input" -strip -sampling-factor 4:2:0 -quality "$quality" "$shots/$name.jpg"
    [ "$(stat -c%s "$shots/$name.jpg")" -le 409600 ] && break
  done
}

# Preserve the complete 1600×1000 app viewport. Every control remains visible.
encode_mp4 radar moore-2013-single
encode_poster radar moore-2013-single
encode_gif radar "$showcase/moore-2013-single.gif" 900
encode_mp4 radar radar-playback-20261016
encode_mp4 analyst moore-2013-four
encode_poster analyst moore-2013-four
encode_mp4 attributes attributes-20261016
encode_mp4 national mrms-20261016
for product in reflectivity velocity correlation differential; do
  encode_mp4 "product-$product" "moore-2013-$product"
  encode_poster "product-$product" "moore-2013-$product"
done

encode_gif radar "$shots/radar-mode.gif" 900
encode_gif analyst "$shots/analyst-mode.gif" 900
encode_gif radar "$shots/hero.gif" 1100
encode_gif wind "$shots/wind.gif" 700 5
still "$frames/radar/02.png" radar-mode
still "$frames/analyst/02.png" analyst-mode
still "$frames/analyst/02.png" alltilts
still "$frames/attributes/00.png" cellconsole
still "$frames/national/02.png" mrms
for name in reflectivity velocity tropical layers products verify derived fronts hrrr mosaic qpe alerts forecast stormtable warning-detail; do
  still "$frames/$name.png" "$name"
done

# An 18-second sequence of six actual app states, each held for three seconds.
# Individual chapters use the same full-size source still; captions call this a screenshot tour.
tour="$frames/tour-concat.txt"
: > "$tour"
for item in 'mrms mrms' 'features menu' 'spc spc' 'layouts layouts' 'four-tilts tilts'; do
  read -r name clip <<< "$item"
  input="$frames/national-to-local/$name.png"
  cp "$input" "$shots/national-to-local/$name.png"
  cp "$input" "$showcase/national-to-local/$name.png"
  ffmpeg -y -loglevel error -loop 1 -i "$input" -t 3 -r 30 -c:v libx264 -preset medium -crf 21 \
    -pix_fmt yuv420p -movflags +faststart "$showcase/national-to-local/$clip.mp4"
  printf "file '%s'\nduration 3\n" "$input" >> "$tour"
done
printf "file '%s'\nduration 3\nfile '%s'\n" "$frames/radar/02.png" "$frames/radar/02.png" >> "$tour"
ffmpeg -y -loglevel error -safe 0 -f concat -i "$tour" -t 18 -r 30 -c:v libx264 -preset medium -crf 21 \
  -pix_fmt yuv420p -movflags +faststart "$showcase/national-to-local/tour.mp4"
ffmpeg -y -loglevel error -safe 0 -f concat -i "$tour" -t 18 \
  -filter_complex 'fps=2,scale=1100:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128[p];[b][p]paletteuse=dither=bayer:bayer_scale=3' \
  -loop 0 "$shots/national-to-local/tour.gif"
echo 'Encoded current desktop showcase, complete gallery, and screenshot tour.'

# Keep optimized posters and legacy incoming URLs aligned with the source captures.
site_shots="$repo/site/public/shots"
for name in radar-mode analyst-mode; do
  magick "$shots/$name.jpg" -quality 85 "$site_shots/$name-20261111.webp"
  cp "$site_shots/$name-20261111.webp" "$site_shots/$name-20261016.webp"
done
cp "$site_shots/radar-mode-20261111.webp" "$site_shots/hookecho-poster.webp"
for width in 640 1280; do
  magick "$shots/reflectivity.jpg" -resize "${width}x" -quality 84 "$site_shots/reflectivity-$width.webp"
done
