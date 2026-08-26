#!/usr/bin/env bash
# Frame-pacing capture. Answers "did frames arrive on time", which a flamegraph does not.
#
#   scripts/perf/capture.sh            # nested Xvfb, drives its own pan/zoom, prints a summary
#   ON_DESKTOP=1 scripts/perf/capture.sh   # your real session; pan it yourself, Ctrl-C when done
#
# The Xvfb run is reproducible and hands-off, but it is not the session that janks: no real
# compositor, and (unless the NVIDIA ICD picks up the X11 surface) no real GPU. Use ON_DESKTOP for
# the number that decides anything; the app prints a `perf:` line every 300 frames either way.
set -euo pipefail
cd "$(dirname "$0")/../.."

DISPLAY_NUM=":78"
W=1600; H=1000
WORK="${TMPDIR:-/tmp}/hookecho-perf"
mkdir -p "$WORK"

echo "building (release + profiling)…"
cargo build --release --features profiling >/dev/null
BIN="target/release/hookecho"

# KTLX, 2013-05-20, the Moore volume — a busy frame, not an empty map.
export HOOKECHO_GOTO="KTLX,-97.28,35.33,8,2013-05-20T20:00:00Z"
export RUST_LOG=info HOOKECHO_PERF_EVERY="${HOOKECHO_PERF_EVERY:-300}"

if [ -n "${ON_DESKTOP:-}" ]; then
  echo "running on your session — pan and zoom for ~30 s, then Ctrl-C"
  exec "$BIN"
fi

pgrep -f "Xvfb $DISPLAY_NUM" >/dev/null || { Xvfb "$DISPLAY_NUM" -screen 0 "${W}x${H}x24" & sleep 1; }
trap 'pkill -x hookecho 2>/dev/null || true' EXIT
( export DISPLAY="$DISPLAY_NUM"; unset WAYLAND_DISPLAY; "$BIN" >"$WORK/app.log" 2>&1 & )

WID=""
for _ in $(seq 60); do
  WID="$(DISPLAY=$DISPLAY_NUM xdotool search --name "HookEcho" 2>/dev/null | tail -1 || true)"
  [ -n "$WID" ] && break
  sleep 0.5
done
[ -n "$WID" ] || { echo "window never appeared; see $WORK/app.log"; exit 1; }
DISPLAY=$DISPLAY_NUM xdotool windowsize "$WID" $W $H windowmove "$WID" 0 0 windowfocus --sync "$WID"
sleep 8   # let the volume land, so the pan is over real data

echo "driving 30 s of pan and zoom…"
export DISPLAY=$DISPLAY_NUM
# A sustained drag, not a burst: the point is to keep frames coming for long enough that the
# pacing summary covers panning rather than the idle cadence either side of it.
end=$(( $(date +%s) + 30 ))
x=400; step=25
xdotool mousemove --sync 800 500 mousedown 1
while [ "$(date +%s)" -lt "$end" ]; do
  x=$(( x + step ))
  if [ "$x" -gt 1200 ] || [ "$x" -lt 400 ]; then step=$(( -step )); x=$(( x + step )); fi
  xdotool mousemove --sync "$x" 500
done
xdotool mouseup 1
sleep 2
pkill -x hookecho 2>/dev/null || true
sleep 1

echo
grep -h "perf:" "$WORK/app.log" || { echo "no perf lines — did the app render 300 frames?"; tail -5 "$WORK/app.log"; }
