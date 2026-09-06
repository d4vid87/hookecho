#!/usr/bin/env bash
# Turn an existing real product image/loop into one platform-safe vertical clip.
set -euo pipefail

campaign=${1:?campaign json required}
output=${2:-social.mp4}
for tool in jq ffmpeg ffprobe fold; do command -v "$tool" >/dev/null || { echo "need $tool" >&2; exit 1; }; done

media=$(jq -er .media "$campaign")
product=$(jq -er .product "$campaign")
label=$(jq -er .label "$campaign")
test -f "$media" || { echo "media not found: $media" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
jq -r .title "$campaign" | fold -s -w 27 > "$work/title.txt"
jq -r .body "$campaign" | fold -s -w 43 > "$work/body.txt"
if [ "$product" = hookecho ]; then
  name=HookEcho
  accent=0x20d9ff
  url=hookecho.io
else
  name=WeatherDesk
  accent=0x58a6ff
  url=hookecho.io/weatherdesk
fi
printf '%s' "$name" > "$work/product.txt"
printf '%s' "$label" > "$work/label.txt"
printf '%s' "$url" > "$work/url.txt"

if [ -f /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf ]; then
  font=/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf
  bold=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf
else
  font=/usr/share/fonts/liberation/LiberationSans-Regular.ttf
  bold=/usr/share/fonts/liberation/LiberationSans-Bold.ttf
fi
test -f "$font" && test -f "$bold" || { echo "a DejaVu or Liberation Sans font is required" >&2; exit 1; }

case "$media" in
  *.gif|*.mp4|*.webm) media_input=(-stream_loop -1 -i "$media");;
  *) media_input=(-loop 1 -framerate 30 -i "$media");;
esac

ffmpeg -y -loglevel error \
  -f lavfi -i "color=c=0x07111f:s=1080x1920:r=30:d=20" \
  "${media_input[@]}" \
  -f lavfi -i "anullsrc=channel_layout=stereo:sample_rate=48000" \
  -filter_complex "
    [1:v]scale=1000:650:force_original_aspect_ratio=decrease:flags=lanczos[ui];
    [0:v][ui]overlay=(W-w)/2:470:shortest=1,
    drawtext=fontfile=${bold}:textfile=${work}/product.txt:fontcolor=0xffffff:fontsize=42:x=60:y=58,
    drawbox=x=60:y=132:w=230:h=54:color=${accent}@0.18:t=fill,
    drawtext=fontfile=${bold}:textfile=${work}/label.txt:fontcolor=${accent}:fontsize=27:x=82:y=143,
    drawtext=fontfile=${bold}:textfile=${work}/title.txt:fontcolor=0xffffff:fontsize=65:line_spacing=16:x=60:y=230,
    drawtext=fontfile=${font}:textfile=${work}/body.txt:fontcolor=0xc8d7e8:fontsize=38:line_spacing=12:x=60:y=1190,
    drawtext=fontfile=${font}:textfile=${work}/url.txt:fontcolor=0xc8d7e8:fontsize=34:x=(w-text_w)/2:y=1800[v]" \
  -map '[v]' -map 2:a -t 20 -r 30 -c:v libx264 -preset medium -crf 21 \
  -pix_fmt yuv420p -c:a aac -b:a 96k -ar 48000 -movflags +faststart -shortest "$output"

test "$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$output")" = 1080
test "$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$output")" = 1920
test "$(ffprobe -v error -select_streams v:0 -show_entries stream=pix_fmt -of csv=p=0 "$output")" = yuv420p
duration=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$output")
awk -v d="$duration" 'BEGIN { exit !(d >= 15 && d <= 35) }'
echo "$output: 1080x1920, ${duration}s"
