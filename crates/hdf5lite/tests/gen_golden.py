#!/usr/bin/env python3
"""Emit the reference values hdf5lite is checked against, straight from h5py.

The point of this file is that the goldens are produced by the real HDF5 library, not by our
reader — otherwise the tests would only prove we're self-consistent. Run it in a venv with h5py
after adding a granule:

    python3 -m venv /tmp/h5venv && /tmp/h5venv/bin/pip install h5py
    /tmp/h5venv/bin/python crates/hdf5lite/tests/gen_golden.py
"""
import json, pathlib, math
import h5py
import numpy as np

HERE = pathlib.Path(__file__).parent / "data"
# The datasets the GLM reader actually needs, plus one contiguous scalar and one large
# multi-chunk array to keep both storage paths covered.
WANT = [
    "flash_lat",
    "flash_lon",
    "flash_energy",
    "flash_time_offset_of_first_event",
    "product_time",
    "event_lat",
]


def summarize(a):
    """A few values plus aggregates — enough to catch a wrong scale, offset or chunk order."""
    v = np.asarray(a, dtype="float64").ravel()
    finite = v[np.isfinite(v)]
    return {
        "len": int(v.size),
        "first": [None if math.isnan(x) else x for x in v[:5].tolist()],
        "last": [None if math.isnan(x) else x for x in v[-5:].tolist()],
        "n_finite": int(finite.size),
        "min": float(finite.min()) if finite.size else None,
        "max": float(finite.max()) if finite.size else None,
        "sum": float(finite.sum()) if finite.size else None,
    }


def attrs_of(g):
    """Attributes as plain JSON — bytes and numpy scalars flattened the way hdf5lite reports them."""
    out = {}
    for k, v in g.attrs.items():
        v = np.asarray(v).ravel()[0]
        if isinstance(v, bytes):
            out[k] = v.decode("utf-8", "replace").rstrip("\0 ")
        elif isinstance(v, str):
            out[k] = v
        else:
            out[k] = float(v)
    return out


for h5 in sorted(HERE.glob("*.h5")):
    # ODIM_H5: what matters is the nested paths, the group attributes the metadata lives in, and
    # that the sweep's byte array comes out of the old-style chunked layout intact.
    with h5py.File(h5, "r") as f:
        out = {"names": sorted(f.keys()), "attrs": {}, "datasets": {}}
        paths = [""] + [
            p
            for top in sorted(f.keys())
            for p in ([top] if not isinstance(f[top], h5py.Group) else [top, *(
                f"{top}/{sub}" for sub in sorted(f[top].keys())
            )])
        ]
        paths += ["dataset1/data1/what"]
        for p in paths:
            g = f[p] if p else f
            if g.attrs:
                out["attrs"][p] = attrs_of(g)
        out["children"] = {p: sorted(f[p].keys()) for p in ["dataset1", "dataset1/data1"]}
        for p in ["dataset1/data1/data", "dataset5/data1/data"]:
            out["datasets"][p] = summarize(np.asarray(f[p][()], dtype="float64"))
    (HERE / (h5.stem + ".golden.json")).write_text(json.dumps(out, indent=1))
    print("wrote", h5.stem + ".golden.json")


for nc in sorted(HERE.glob("*.nc")):
    with h5py.File(nc, "r") as f:
        out = {"names": sorted(f.keys()), "datasets": {}}
        for name in WANT:
            if name not in f:
                continue
            d = f[name]
            # h5py applies scale_factor/add_offset only via netCDF4; do it explicitly so the
            # golden matches what hdf5lite's read_f64 returns.
            raw = np.asarray(d[()], dtype="float64")
            fill = d.attrs.get("_FillValue")
            if fill is not None:
                raw = np.where(raw == float(np.asarray(fill).ravel()[0]), np.nan, raw)
            sf = d.attrs.get("scale_factor")
            ao = d.attrs.get("add_offset")
            if sf is not None:
                raw = raw * float(np.asarray(sf).ravel()[0])
            if ao is not None:
                raw = raw + float(np.asarray(ao).ravel()[0])
            out["datasets"][name] = summarize(raw)
    (HERE / (nc.stem + ".golden.json")).write_text(json.dumps(out, indent=1))
    print("wrote", nc.stem + ".golden.json")
