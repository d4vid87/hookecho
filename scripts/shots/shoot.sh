#!/usr/bin/env bash
# Regenerate the README screenshots.
#
#   ./scripts/shots/shoot.sh archive     # the historic-event set — reproducible any day
#   ./scripts/shots/shoot.sh live        # needs real weather; run ./shoot.sh scout first
#   ./scripts/shots/shoot.sh scout       # which radar has storms right now?
#   ./scripts/shots/shoot.sh check       # README <-> docs/shots cross-reference + size budget
#   ./scripts/shots/shoot.sh hero        # the animated hero loop
#   ./scripts/shots/shoot.sh reflectivity velocity ...   # named scenes
#
# The app runs on a nested Xvfb, not your desktop: xdotool key injection does not reach the
# window under KDE Wayland/XWayland (mouse does, keys don't), and a nested server also keeps the
# shoot from stealing your pointer for ten minutes.
#
# Scenes are staged with HOOKECHO_GOTO=SITE,lon,lat,zoom[,RFC3339] (see app.rs) rather than by
# clicking through the Event Library — the moment a window's rows move, coordinate-driven staging
# shoots the wrong frame and looks fine doing it.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$REPO/docs/shots"
BIN="$REPO/target/release/hookecho"
DISPLAY_NUM=":77"
W=1600
H=1000
WORK="${TMPDIR:-/tmp}/hookecho-shots"
PROFILE="$WORK/profile"

# Registry labels typed into the command palette. These are matched against the source in
# preflight — a renamed action silently shoots the wrong scene otherwise.
L_ALLTILTS="Compare 4 tilts"
L_XSECTION="Tool: Cross-section"
L_FORECAST="Tool: Point forecast"
L_STORMTABLE="Storm attributes…"
L_FRONTS="Surface fronts (H/L)"
L_GLM="Satellite lightning (GLM)"

log() { printf '\033[36m▸\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------- setup

preflight() {
  for t in Xvfb xdotool import magick jq ffmpeg; do
    command -v "$t" >/dev/null || die "need $t"
  done
  # A stale binary is the classic failure: the palette answers "No matches" for anything added
  # since the last build, and the scene silently shoots without its layer.
  log "building release"
  (cd "$REPO" && cargo build --release --quiet)
  for l in "$L_ALLTILTS" "$L_XSECTION" "$L_FORECAST" "$L_STORMTABLE" "$L_FRONTS" "$L_GLM"; do
    grep -qF "$l" "$REPO/crates/hookecho/src/app.rs" \
      || die "palette label '$l' is not in app.rs any more — update shoot.sh"
  done
}

start_xvfb() {
  pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null && return
  Xvfb "$DISPLAY_NUM" -screen 0 "${W}x${H}x24" >/dev/null 2>&1 &
  sleep 2
}

# Scratch profile so the shoot never touches (or is coloured by) the real one. The Mapbox token
# is copied in at run time from the user's config and never leaves this directory — the committed
# template carries an empty string.
write_profile() {
  local extra="${1:-{\}}"
  mkdir -p "$PROFILE/hookecho"
  local key theme
  key="$(jq -r '.mapbox_key // ""' "$HOME/.config/hookecho/settings.json" 2>/dev/null || echo "")"
  theme="$(jq -r '.theme // "Dark"' "$HOME/.config/hookecho/settings.json" 2>/dev/null || echo Dark)"
  [ -n "$key" ] || log "WARNING: no mapbox_key found — shots will fall back to the dark vector basemap"
  jq -n --argjson base "$(cat "$REPO/scripts/shots/settings.template.json")" \
        --argjson extra "$extra" \
        --arg key "$key" --arg theme "$theme" \
        '$base * $extra * {mapbox_key: $key, theme: $theme}' \
    > "$PROFILE/hookecho/settings.json"
}

# ---------------------------------------------------------------- driving

WID=""

launch() { # launch SITE,lon,lat,zoom[,RFC3339]  [settings-overrides-json]
  stop_app
  write_profile "${2:-{\}}"
  log "launch @ $1"
  ( export DISPLAY="$DISPLAY_NUM" \
           XDG_CONFIG_HOME="$PROFILE" XDG_DATA_HOME="$PROFILE/data" \
           HOOKECHO_GOTO="$1" RUST_LOG=warn
    # winit picks Wayland over X11 whenever WAYLAND_DISPLAY is set, so on a Wayland desktop the
    # app cheerfully ignores DISPLAY and opens a window on the user's actual screen — where the
    # harness cannot see it and every keystroke lands in whatever they were doing.
    unset WAYLAND_DISPLAY
    # The tile cache is the real one on purpose: basemap tiles are already paid for, and a cold
    # cache means every scene waits on Mapbox before it can settle.
    "$BIN" >"$WORK/app.log" 2>&1 & )
  for _ in $(seq 40); do
    WID="$(DISPLAY="$DISPLAY_NUM" xdotool search --name "Hook Echo" 2>/dev/null | tail -1 || true)"
    [ -n "$WID" ] && break
    sleep 0.5
  done
  [ -n "$WID" ] || die "window never appeared (see $WORK/app.log)"
  # No window manager on Xvfb, so size/position/focus are all manual — and without an explicit
  # windowfocus, synthetic keys go nowhere at all.
  DISPLAY="$DISPLAY_NUM" xdotool windowsize "$WID" "$W" "$H" windowmove "$WID" 0 0
  DISPLAY="$DISPLAY_NUM" xdotool windowfocus --sync "$WID"
  sleep 1
}

stop_app() { pkill -x hookecho 2>/dev/null || true; sleep 0.6; }

key()   { DISPLAY="$DISPLAY_NUM" xdotool key --window "$WID" "$@"; sleep 0.4; }
click() { DISPLAY="$DISPLAY_NUM" xdotool mousemove --sync "$1" "$2" click 1; sleep 0.8; }

palette() { # palette "Exact Registry Label"
  DISPLAY="$DISPLAY_NUM" xdotool key --clearmodifiers ctrl+k; sleep 0.6
  DISPLAY="$DISPLAY_NUM" xdotool type --delay 45 -- "$1"; sleep 0.8
  DISPLAY="$DISPLAY_NUM" xdotool key Return; sleep 1.2
}

# Wait for the frame to stop changing rather than sleeping a guessed number of seconds: archive
# volumes come off S3 in anything from two seconds to a minute, and a fixed sleep either wastes
# time or shoots a half-loaded map.
wait_settle() { # wait_settle [floor_secs] [cap_secs]
  local floor="${1:-10}" cap="${2:-100}" last="" now="" stable=0 t=0
  sleep "$floor"; t=$floor
  while [ "$t" -lt "$cap" ]; do
    now="$(import -display "$DISPLAY_NUM" -window root png:- 2>/dev/null | md5sum | cut -c1-16)"
    if [ "$now" = "$last" ]; then
      stable=$((stable + 1))
      [ "$stable" -ge 2 ] && { log "settled after ${t}s"; return 0; }
    else
      stable=0
    fi
    last="$now"; sleep 3; t=$((t + 3))
  done
  log "WARNING: never settled in ${cap}s — check the frame before shipping it"
}

snap() { # snap NAME
  mkdir -p "$OUT"
  local f="$OUT/$1.jpg"
  import -display "$DISPLAY_NUM" -window root png:- | magick - -quality 88 "$f"
  local kb=$(( $(stat -c%s "$f") / 1024 ))
  if [ "$kb" -gt 400 ]; then
    import -display "$DISPLAY_NUM" -window root png:- | magick - -quality 80 "$f"
    kb=$(( $(stat -c%s "$f") / 1024 ))
  fi
  log "saved $1.jpg (${kb}K)"
}

# ---------------------------------------------------------------- scenes
#
# Historic events, from crates/hookecho/src/events.rs. Framing is tighter than the Event Library's
# own zoom: these are portraits of one storm, not a regional view.

MOORE="KTLX,-97.47,35.32,9.6,2013-05-20T20:15:00Z"
ELRENO="KTLX,-97.96,35.53,9.4,2013-05-31T23:10:00Z"
TUSCALOOSA="KBMX,-87.55,33.20,9.3,2011-04-27T22:10:00Z"
JOPLIN="KSGF,-94.50,37.06,9.5,2011-05-22T22:40:00Z"
MAYFIELD="KPAH,-88.64,36.74,9.3,2021-12-11T03:30:00Z"
IAN="KTBW,-82.20,26.70,7.4,2022-09-28T19:00:00Z"

scene_reflectivity() { launch "$TUSCALOOSA"; wait_settle 14; key 1; wait_settle 6; snap reflectivity; }
scene_velocity()     { launch "$ELRENO";     wait_settle 14; key 2; wait_settle 8; snap velocity; }
scene_layers()       { launch "$MAYFIELD";   wait_settle 14; key 1; sleep 1; key l; sleep 1.5; snap layers; }
scene_tropical()     { launch "$IAN";        wait_settle 16; key 1; wait_settle 6; snap tropical; }

scene_alltilts() {
  launch "$MOORE"; wait_settle 14; key 1
  palette "$L_ALLTILTS"
  wait_settle 20 140   # four tilts = four bins off one volume, plus three more panes of tiles
  snap alltilts
}

scene_xsection() {
  launch "$MOORE"; wait_settle 14; key 1
  palette "$L_XSECTION"
  # Cut southwest-to-northeast through the mesocyclone, so the panel shows the vault and the
  # overhang rather than a slice of clear air. Screen coords hold because the camera is pinned.
  click 690 620
  click 900 430
  wait_settle 8
  snap xsection
}

scene_alerts() {
  # Archived warnings render whenever the timeline is scrubbed (app.rs sync_archive_warnings), so
  # the panel lists the real April 27 2011 warning stack instead of "No alerts in view."
  launch "$TUSCALOOSA"; wait_settle 16; key 1; sleep 1
  key a; sleep 2
  snap alerts
}

scene_products() {
  launch "$JOPLIN"; wait_settle 14; key 1; sleep 1
  click 68 938   # the product pill, bottom-left
  sleep 1.5
  snap products
}

scene_forecast() {
  launch "$MOORE"; wait_settle 14; key 1
  palette "$L_FORECAST"
  click 640 560
  wait_settle 8
  snap forecast
}

# --- live-dependent: these have no archive path, so they need real weather -------------------

scene_stormtable() { # SCIT cells are fetched for today/yesterday only
  launch "${LIVE_SITE:-KTLX},${LIVE_LON:--97.3},${LIVE_LAT:-35.3},8.4"
  wait_settle 20
  palette "$L_STORMTABLE"
  wait_settle 8
  snap stormtable
}

scene_fronts() {
  launch "KTLX,-95.0,38.5,4.6"; wait_settle 16
  palette "$L_FRONTS"
  wait_settle 12
  snap fronts
}

scene_glm() {
  launch "${LIVE_SITE:-KTLX},${LIVE_LON:--97.3},${LIVE_LAT:-35.3},7.2"
  wait_settle 16
  palette "$L_GLM"
  wait_settle 14
  snap glm
}

# --- hero loop --------------------------------------------------------------------------------

scene_hero() {
  launch "$MOORE"; wait_settle 16; key 1
  local dir="$WORK/hero"; rm -rf "$dir"; mkdir -p "$dir"
  # Step back through the volumes leading up to the tornado, then replay forwards, so the loop
  # shows the hook organising rather than a single frozen frame. 10 frames = the scan cache's
  # depth, past which every step re-downloads.
  for _ in $(seq 9); do key Left; sleep 2; done
  wait_settle 10 120
  for i in $(seq 0 9); do
    import -display "$DISPLAY_NUM" -window root "$dir/$(printf '%02d' "$i").png"
    key Right; sleep 3
  done
  ffmpeg -y -loglevel error -framerate 4 -pattern_type glob -i "$dir/*.png" \
    -vf "scale=1100:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=160[p];[b][p]paletteuse=dither=bayer:bayer_scale=3" \
    -loop 0 "$OUT/hero.gif"
  log "hero.gif $(( $(stat -c%s "$OUT/hero.gif") / 1024 ))K"
}

# ---------------------------------------------------------------- utility subcommands

scout() {
  log "looking for a radar with storms on it…"
  for s in KTLX KFWS KDDC KLSX KBMX KICT KAMA KSHV KLZK KGWX; do
    n="$("$BIN" --headless-cells "$s" 2>/dev/null | grep -c '^' || true)"
    printf '  %s  %s cells\n' "$s" "$n"
  done
  echo "Pick the busiest, then: LIVE_SITE=KXXX LIVE_LON=-97.3 LIVE_LAT=35.3 ./shoot.sh live"
}

check() {
  local fail=0
  # Every referenced file exists, and every file is referenced — an orphan shot is a shot nobody
  # noticed going stale.
  while read -r f; do
    [ -f "$REPO/$f" ] || { echo "MISSING: $f (referenced by README)"; fail=1; }
  done < <(grep -o 'docs/shots/[a-z0-9]*\.\(jpg\|gif\)' "$REPO/README.md" | sort -u)
  for f in "$OUT"/*.jpg "$OUT"/*.gif; do
    [ -e "$f" ] || continue
    grep -q "docs/shots/$(basename "$f")" "$REPO/README.md" \
      || { echo "ORPHAN: $(basename "$f") is not in the README"; fail=1; }
  done
  for f in "$OUT"/*.jpg; do
    [ -e "$f" ] || continue
    kb=$(( $(stat -c%s "$f") / 1024 ))
    [ "$kb" -le 400 ] || { echo "OVERSIZE: $(basename "$f") ${kb}K > 400K"; fail=1; }
  done
  grep -q '"mapbox_key": ""' "$REPO/scripts/shots/settings.template.json" \
    || { echo "LEAK: settings.template.json has a non-empty mapbox_key"; fail=1; }
  du -ch "$OUT"/* | tail -1
  [ "$fail" = 0 ] && log "check passed" || die "check failed"
}

ARCHIVE_SCENES=(reflectivity velocity alltilts xsection alerts products layers forecast tropical)
LIVE_SCENES=(stormtable fronts glm)

main() {
  mkdir -p "$WORK"
  case "${1:-}" in
    check) check; return ;;
    scout) preflight; scout; return ;;
  esac
  preflight; start_xvfb
  local want=()
  case "${1:-all}" in
    all)     want=("${ARCHIVE_SCENES[@]}" hero "${LIVE_SCENES[@]}") ;;
    archive) want=("${ARCHIVE_SCENES[@]}" hero) ;;
    live)    want=("${LIVE_SCENES[@]}") ;;
    *)       want=("$@") ;;
  esac
  for s in "${want[@]}"; do
    log "=== $s ==="
    "scene_$s" || log "SCENE FAILED: $s (continuing)"
  done
  stop_app
  log "done — review every frame before committing (see README.md in this directory)"
}

main "$@"
