# wxdata fixtures

- `rap-shear-pair.grib2`: unmodified NOAA RAP 2026-11-04 00Z f00 message 242,
  containing the VUCSH/VVCSH 0–6 km shear siblings; tests distinct component selection.
  Source: `https://nomads.ncep.noaa.gov/pub/data/nccf/com/rap/prod/rap.20261104/rap.t00z.awp130pgrbf00.grib2`,
  byte range `12686079-12834496`; SHA-256
  `0e88a99c09de4cb20bb9be7de1eea2561b38641626a281512b7ccb908e8b9f8c`.

- `g19-c13-meso.nc`: unmodified NOAA GOES-19 ABI-L2-CMIPM C13 granule; source and checksum are
  recorded in `docs/source-contracts.md`.
- `kdmx-one-sweep.bin`: KDMX Archive II fixture trimmed to one complete sweep with the vendored
  `nexrad-data` fixture tool; used to prove archive and realtime-chunk decoding are identical.
- `kfws-velocity-sweep.bin`: NOAA KFWS Archive II volume `KFWS20261103_021529_V06`, trimmed
  with the same `vendor/nexrad-data/examples/trim_fixture.rs` record-selection/compression logic
  to sweep 2 plus metadata records. Gates and instrument constants are unchanged: 720 velocity
  radials at 0.52734375 degrees, Nyquist 33.98 m/s. Used for real raw/corrected velocity worker
  and browser checks (the KDMX fixture has no velocity).
  Source: `https://noaa-nexrad-level2.s3.amazonaws.com/2026/11/03/KFWS/KFWS20261103_021529_V06`;
  input SHA-256 `6e5f0c5c326544e25cd7c0cf1d8cc533e8ed5e5dd67276bf790569894237c0a6`;
  fixture SHA-256 `c5f5c259ab56098da9a87bea5bc25a0cc9b1126c82583fe885e1a2b5ed49b83c`.
- `dwd-boo-tilt*.h5`: DWD polar-radar fixtures used by the ODIM decoder tests.
- `esahr.h5`: unmodified EUMETNET OpenRadarData Alhaurín el Grande Spanish PVOL object,
  `esahr@20261015T0857@0.5_1.5@DBZH_VRADH.h5`, from the public `openradar-24h` bucket;
  SHA-256 `db04be3a870105b6f6bb64fd85616d8bd30d4505b8fe87330da9d9944d3e1e5b`.
  Its floating-point gates check the ODIM quantization path. EUMETNET open data is CC BY 4.0.
