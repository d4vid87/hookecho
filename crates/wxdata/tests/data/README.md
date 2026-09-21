# wxdata fixtures

- `g19-c13-meso.nc`: unmodified NOAA GOES-19 ABI-L2-CMIPM C13 granule; source and checksum are
  recorded in `docs/source-contracts.md`.
- `kdmx-one-sweep.bin`: KDMX Archive II fixture trimmed to one complete sweep with the vendored
  `nexrad-data` fixture tool; used to prove archive and realtime-chunk decoding are identical.
- `dwd-boo-tilt*.h5`: DWD polar-radar fixtures used by the ODIM decoder tests.
