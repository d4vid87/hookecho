# wxdata fixtures

- `g19-c13-meso.nc`: unmodified NOAA GOES-19 ABI-L2-CMIPM C13 granule; source and checksum are
  recorded in `docs/source-contracts.md`.
- `kdmx-one-sweep.bin`: KDMX Archive II fixture trimmed to one complete sweep with the vendored
  `nexrad-data` fixture tool; used to prove archive and realtime-chunk decoding are identical.
- `dwd-boo-tilt*.h5`: DWD polar-radar fixtures used by the ODIM decoder tests.
- `esahr.h5`: unmodified EUMETNET OpenRadarData Alhaurín el Grande Spanish PVOL object,
  `esahr@20261015T0857@0.5_1.5@DBZH_VRADH.h5`, from the public `openradar-24h` bucket;
  SHA-256 `db04be3a870105b6f6bb64fd85616d8bd30d4505b8fe87330da9d9944d3e1e5b`.
  Its floating-point gates check the ODIM quantization path. EUMETNET open data is CC BY 4.0.
