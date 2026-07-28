#!/usr/bin/env python3
"""Colour every airport in view by flight category (VFR / MVFR / IFR / LIFR).

An example Hook Echo-WX plugin: read the view from the environment, print a placefile on stdout,
exit 0. Nothing else is required of a plugin — no library, no registration, no build step.

    Name:     metar
    Command:  python3 /path/to/plugins/metar_flightcat.py
"""

import json
import os
import sys
import urllib.request

# The app sets these before every run. BBOX is min_lon,min_lat,max_lon,max_lat.
BBOX = os.environ.get("HOOKECHO_BBOX", "-100,32,-94,38")
UA = "hookecho-plugin-example (github.com/d4vid87/hookecho)"

# Ceiling (ft) and visibility (statute miles) thresholds, and the colour for each category.
CATEGORIES = [
    ("LIFR", 500, 1.0, "255 0 255"),
    ("IFR", 1000, 3.0, "255 0 0"),
    ("MVFR", 3000, 5.0, "0 128 255"),
    ("VFR", 99999, 99.0, "0 255 0"),
]


def category(ceiling, visibility):
    for name, ceil_max, vis_max, color in CATEGORIES:
        if ceiling < ceil_max or visibility < vis_max:
            return name, color
    return CATEGORIES[-1][0], CATEGORIES[-1][3]


def main():
    west, south, east, north = (float(v) for v in BBOX.split(","))
    url = (
        "https://aviationweather.gov/api/data/metar?format=json"
        f"&bbox={south},{west},{north},{east}"
    )
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=8) as resp:
        obs = json.load(resp)

    print("Title: Flight categories")
    print("RefreshSeconds: 300")
    print("Threshold: 400")
    for ob in obs:
        lat, lon = ob.get("lat"), ob.get("lon")
        if lat is None or lon is None:
            continue
        ceiling = min(
            (c["cover"] and c.get("base") or 99999)
            for c in (ob.get("clouds") or [{"cover": None}])
        ) or 99999
        visibility = ob.get("visib")
        visibility = 99.0 if not isinstance(visibility, (int, float)) else float(visibility)
        name, color = category(float(ceiling), visibility)
        print(f"Color: {color}")
        # An Object draws a fixed-size symbol: a ring that stays readable at every zoom.
        print(f"Object: {lat}, {lon}")
        print(" Line: 2, 0")
        for x, y in ((-5, -5), (5, -5), (5, 5), (-5, 5), (-5, -5)):
            print(f"  {x}, {y}")
        print(" End:")
        print(f' Text: 0, 12, 1, "{ob.get("icaoId", "")}", "{name} — {ob.get("rawOb", "")}"')
        print("End:")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # a plugin's stderr is shown in the Placefile Manager
        print(f"metar plugin: {exc}", file=sys.stderr)
        sys.exit(1)
