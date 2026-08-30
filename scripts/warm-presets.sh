#!/usr/bin/env bash
# Warm the origin's disk cache: the national mosaic, then one still per radar on every network.
#
# Serial on purpose — the renderer is one frame at a time anyway, and asking in parallel only
# fills a queue. Run it once after a deploy so no preset is ever "never rendered" (that is the one
# case the server cannot answer from a stale file), and on a slow timer to keep the frames a
# crawler asks for recent rather than absent.
#
# Usage: scripts/warm-presets.sh [origin]   # default http://127.0.0.1:8087
set -u
host="${1:-http://127.0.0.1:8087}"
here="$(cd "$(dirname "$0")/.." && pwd)"

curl -s -o /dev/null -w "national %{http_code} %{time_total}s\n" "$host/national.png"
python3 -c '
import json, sys
sites = json.load(open(sys.argv[1]))
print("\n".join(s["id"] for s in sites))
' "$here/site/src/data/nexrad-sites.json" | while read -r id; do
  curl -s -o /dev/null -w "$id %{http_code} %{time_total}s\n" "$host/snapshot.png?site=$id"
done
