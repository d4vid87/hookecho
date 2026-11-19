"""Regenerate the sparse ABI allocation regression fixture (requires h5py/numpy).

Use the real 0.5 km full-disk shape without storing or allocating its 470 million
cells. The two written samples check striding and NOAA quality-mask alignment.
"""
from pathlib import Path

import h5py
import numpy as np

with h5py.File(Path(__file__).with_name("abi-full-disk-sparse.nc"), "w", libver="earliest") as f:
    size = 21696
    cmi = f.create_dataset("CMI", (size, size), dtype="i2", chunks=(64, 64),
                           compression="gzip", fillvalue=-1)
    cmi.attrs["_FillValue"] = np.int16(-1)
    cmi.attrs["scale_factor"] = 0.01
    quality = f.create_dataset("DQF", (size, size), dtype="u1", chunks=(64, 64),
                               compression="gzip", fillvalue=3)
    quality.attrs["_FillValue"] = np.uint8(3)
    cmi[19 * 400, 19 * 600] = 50
    quality[19 * 400, 19 * 600] = 0
    cmi[19 * 400, 19 * 601] = 75
    quality[19 * 400, 19 * 601] = 2
    f.create_dataset("x", data=np.linspace(-0.151844, 0.151844, size), compression="gzip", shuffle=True)
    f.create_dataset("y", data=np.linspace(0.151844, -0.151844, size), compression="gzip", shuffle=True)
    projection = f.create_dataset("goes_imager_projection", data=0)
    projection.attrs["longitude_of_projection_origin"] = -137.0
    projection.attrs["perspective_point_height"] = 35786023.0
    projection.attrs["semi_major_axis"] = 6378137.0
    projection.attrs["semi_minor_axis"] = 6356752.31414
    f.create_dataset("t", data=797580031.0)
    f.create_dataset("band_id", data=2)
