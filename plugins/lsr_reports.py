#!/usr/bin/env python3
"""Plot local storm reports — hail, wind, tornadoes — for the time on screen.

An example Hook Echo-WX plugin: read the view from the environment, print a placefile on stdout,
exit 0. This one is archive-aware on purpose: it asks the IEM for the six hours ending at
HOOKECHO_TIME, so scrubbing back through an event shows the reports as they came in.

    Name:     lsr
    Command:  python3 /path/to/plugins/lsr_reports.py
"""

import datetime as dt
import json
import os
import sys
import urllib.parse
import urllib.request

BBOX = os.environ.get("HOOKECHO_BBOX", "-100,32,-94,38")
TIME = os.environ.get("HOOKECHO_TIME", "")
UA = "hookecho-plugin-example (github.com/d4vid87/hookecho)"

# How far back from the instant on screen to ask for.
WINDOW_HOURS = 6

# Colour by report type code (the label comes from the feed's own `typetext`). The codes are the
# NWS ones: T tornado, V funnel, D thunderstorm wind damage, G measured gust, H hail, and the
# water ones F / E / R.
COLORS = {
    "T": "255 0 0",
    "V": "255 120 0",
    "D": "255 180 0",
    "G": "0 200 90",
    "H": "0 220 255",
    "F": "80 120 255",
    "E": "80 120 255",
    "R": "80 160 200",
}
DEFAULT_COLOR = "200 200 200"


def when():
    """The instant on screen, or now. The app sends RFC 3339; Python wants no trailing Z."""
    if not TIME:
        return dt.datetime.now(dt.timezone.utc)
    return dt.datetime.fromisoformat(TIME.replace("Z", "+00:00"))


def main():
    west, south, east, north = (float(v) for v in BBOX.split(","))
    end = when()
    start = end - dt.timedelta(hours=WINDOW_HOURS)
    query = urllib.parse.urlencode(
        {
            "sts": start.strftime("%Y-%m-%dT%H:%M"),
            "ets": end.strftime("%Y-%m-%dT%H:%M"),
            "west": west,
            "south": south,
            "east": east,
            "north": north,
            "fmt": "geojson",
        }
    )
    url = f"https://mesonet.agron.iastate.edu/geojson/lsr.geojson?{query}"
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=8) as resp:
        data = json.load(resp)

    print(f"Title: Storm reports ({WINDOW_HOURS} h)")
    print("RefreshSeconds: 120")
    print("Threshold: 999")
    for feature in data.get("features", []):
        geom = feature.get("geometry") or {}
        if geom.get("type") != "Point":
            continue
        lon, lat = geom["coordinates"][:2]
        props = feature.get("properties") or {}
        code = props.get("type")
        label = (props.get("typetext") or "REPORT").title()
        magnitude = props.get("magnitude")
        if magnitude not in (None, ""):
            label = f"{label} {magnitude} {props.get('unit') or ''}".strip()
        remark = (props.get("remark") or props.get("city") or "").replace('"', "'")

        print(f"Color: {COLORS.get(code, DEFAULT_COLOR)}")
        # A diamond for a tornado or a funnel, a square for everything else — the shape carries
        # the severity at a glance, and `Object:` keeps both readable at any zoom.
        corners = (
            ((0, -7), (7, 0), (0, 7), (-7, 0), (0, -7))
            if code in ("T", "V")
            else ((-5, -5), (5, -5), (5, 5), (-5, 5), (-5, -5))
        )
        print(f"Object: {lat}, {lon}")
        print(" Line: 2, 0")
        for x, y in corners:
            print(f"  {x}, {y}")
        print(" End:")
        print(f' Text: 0, 14, 1, "{label}", "{label} — {remark}"')
        print("End:")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # a plugin's stderr is shown in the Placefile Manager
        print(f"lsr plugin: {exc}", file=sys.stderr)
        sys.exit(1)
