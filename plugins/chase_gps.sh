#!/usr/bin/env bash
# Draw the chase vehicle's track from a GPS log — the "my own data on the radar" case.
#
# Reads a CSV of `iso8601,lat,lon` (the shape gpsd's `gpspipe -w` output reduces to, and what
# most phone logger apps export) and prints it as a placefile trail with a marker at the head.
#
#     Name:     chase
#     Command:  bash /path/to/plugins/chase_gps.sh /path/to/track.csv
#
# A plugin is any program that prints a placefile, so this one is twenty lines of shell.
set -euo pipefail

LOG="${1:-$HOME/.local/share/hookecho/track.csv}"
[ -r "$LOG" ] || { echo "no GPS log at $LOG" >&2; exit 1; }

# Keep the last hour of fixes: the whole log would repaint the state you drove across last week.
POINTS=$(tail -n 720 "$LOG")

echo "Title: Chase track"
echo "RefreshSeconds: 15"
echo "Color: 255 200 0"
echo "Line: 3, 0"
echo "$POINTS" | awk -F, 'NF>=3 { printf " %s, %s\n", $2, $3 }'
echo "End:"

# Head marker, drawn in screen pixels so it stays the same size as you zoom.
read -r _ LAT LON _ <<<"$(echo "$POINTS" | tail -1 | tr ',' ' ')"
echo "Color: 255 255 255"
echo "Object: $LAT, $LON"
echo " Triangles:"
echo "  0, 10, 255, 255, 255"
echo "  -7, -7, 255, 200, 0"
echo "  7, -7, 255, 200, 0"
echo " End:"
echo "End:"
