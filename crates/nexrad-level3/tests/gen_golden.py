#!/usr/bin/env python3
"""Generate golden JSON for Level 3 test files using MetPy's Level3File.

The Rust decoder is verified against these goldens (see `tests/golden.rs`). Regenerate with:

    python3 -m venv venv && venv/bin/pip install metpy
    venv/bin/python tests/gen_golden.py tests/data/nst_tlx.l3 > tests/data/nst_tlx.golden.json

Golden shape: product code, radar lat/lon/height, and the storm cells (id + position in km
east/north of the radar) extracted from the symbology block's Storm ID (packet 15) features.
"""
import json
import sys
from metpy.io import Level3File


def main(path: str) -> None:
    f = Level3File(path)
    pd = f.prod_desc
    cells = []
    for layer in getattr(f, "sym_block", []) or []:
        for pkt in layer:
            if isinstance(pkt, dict) and pkt.get("type") == "Storm ID":
                cells.append({"id": pkt["id"], "x": round(pkt["x"], 3), "y": round(pkt["y"], 3)})
    cells.sort(key=lambda c: c["id"])
    print(json.dumps({
        "prod_code": pd.prod_code,
        "lat": f.lat,
        "lon": f.lon,
        "height_m": f.height,
        "cells": cells,
    }, indent=2))


if __name__ == "__main__":
    main(sys.argv[1])
