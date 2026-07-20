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
import numpy as np
from metpy.io import Level3File


def radial_packet(f):
    """Return the Digital Radial Data Array packet (code 16), if the product has one."""
    for layer in getattr(f, "sym_block", []) or []:
        for pkt in layer:
            if isinstance(pkt, dict) and "start_az" in pkt:
                return pkt
    return None


def main(path: str) -> None:
    f = Level3File(path)
    pd = f.prod_desc
    pkt = radial_packet(f)
    if pkt is not None:
        # Digital radial product (DVL / EET): emit grid shape, thresholds, and sampled values.
        data = pkt["data"]
        # Find the maximum data level and a few representative samples.
        best = (0, 0, 0)
        for ri, row in enumerate(data):
            for bi, lvl in enumerate(row):
                if lvl > best[2]:
                    best = (ri, bi, lvl)

        def decode(lvl):
            m = np.asarray(f.map_data(np.array([lvl]))).ravel()[0]
            return None if (m != m) else round(float(m), 4)  # NaN → None

        samples = []
        for ri, bi in [(best[0], best[1]), (0, 0), (best[0], best[1] + 1)]:
            if ri < len(data) and bi < len(data[ri]):
                samples.append({"rad": ri, "bin": bi, "level": int(data[ri][bi]), "value": decode(data[ri][bi])})
        print(json.dumps({
            "prod_code": pd.prod_code,
            "lat": f.lat,
            "lon": f.lon,
            "nrad": len(data),
            "nbins": len(data[0]),
            "first": pkt["first"],
            "thresholds": [int(t) for t in f.thresholds],
            "max_level": int(best[2]),
            "samples": samples,
        }, indent=2))
        return

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
