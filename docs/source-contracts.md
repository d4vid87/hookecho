# External source contracts

Adapters in this roadmap are added only after their public contract is checked against an official
provider source. Live network checks belong in scheduled CI; ordinary tests use committed fixtures.
The scheduled `Feed contracts` workflow runs `scripts/feed-contracts.sh` against the public NOAA
bucket listings. It requires each selected MRMS feed to publish today or yesterday and GOES-19
ABI/GLM families to publish in the current or previous hour. Missing or stale feeds fail that
workflow without making pull-request tests depend on the network.
RRFS plus GEFS mean, spread, control, and edge perturbed members must expose a current or
previous-day index.

Model schedule metadata follows NOAA's current HRRR/RAP/GFS product inventories and ECMWF's
Cycle 50r1 open-data contract. Extended HRRR and RAP forecast hours are restricted to their
documented cycles; ECMWF 00/12 UTC runs extend to F240 while 06/18 UTC runs stop at F90.

- HRRR/RAP: <https://registry.opendata.aws/noaa-rap/> and
  <https://registry.opendata.aws/dynamical-noaa-hrrr/>
- GFS: <https://www.nco.ncep.noaa.gov/pmb/products/gfs/>
- GEFS: <https://registry.opendata.aws/noaa-gefs/>. HookEcho reads `geavg` mean, `gespr`
  standard deviation, `gec00` control, and `gep01` through `gep30` perturbed fields, using the
  0.25-degree surface product and 0.5-degree pressure-level product. NOAA's operational inventory
  documents one control plus 30 perturbed forecasts; aggregate provenance therefore reports 31
  available members:
  <https://www.nco.ncep.noaa.gov/pmb/products/gens/>.
- ECMWF IFS: <https://confluence.ecmwf.int/spaces/DAC/pages/272310539/ECMWF+open+data+real-time+forecasts+from+IFS+and+AIFS>

## NOAA RRFS v1 parallel

- Provider: NOAA's `noaa-rrfs-ops-pds` public bucket, using the operational directory contract
  during pre-implementation parallel: <https://registry.opendata.aws/noaa-rrfs-ops/>.
- Adapter: CONUS 3 km `2dfld` GRIB2 plus `.idx` range reads. Hourly cycles extend to F18;
  00/06/12/18 UTC cycles extend to F84.
- REFS: combined `ensprod` probability files are range-read from their `.idx` sidecars. Published
  neighborhood probabilities above 10, 20, 30, 40, and 50 dBZ are selectable; each index-provided
  available-member count is preserved in field provenance.
- Status: label the source as parallel/experimental until NOAA's announced operational promotion.
  Scheduled CI requires a current or previous-day index object so naming drift fails visibly.

## NOAA GOES ABI CMIP

- Provider: NOAA Open Data Dissemination on AWS. GOES-19 is operational GOES-East and GOES-18 is
  operational GOES-West. Buckets are public and require no AWS account:
  <https://registry.opendata.aws/noaa-goes/>.
- Products: `ABI-L2-CMIPC` (CONUS) and `ABI-L2-CMIPM` (mesoscale), netCDF-4/HDF5. The CMIP product
  carries `CMI`, `DQF`, fixed-grid `x`/`y`, time, band, and `goes_imager_projection` metadata.
- Navigation: use the file's geostationary projection parameters and scan coordinates; do not
  assume a latitude/longitude grid. NOAA's fixed-grid definition is in the GOES-R PUG:
  <https://goes-r.noaa.gov/users/docs/PUG-GRB-vol4.pdf>.
- Quality: DQF 0 is good, 1 conditionally usable, 2 out of range, 3 missing, and 4 focal-plane
  temperature threshold exceeded. Values with DQF 2–4 are kept distinct in the quality mask and
  treated as missing for numeric sampling. NOAA documents these states at
  <https://goes-r.noaa.gov/users/GOES-17-ABI-Performance.html>.
- Fixture: `g19-c13-meso.nc`, an unmodified public GOES-19 C13 mesoscale granule from
  `noaa-goes19/ABI-L2-CMIPM/2025/100/18/`, SHA-256
  `f8c25303bcb27d5bec342cae2be858cf825b0960e89de35c5f7bed75280d677a`.

## NOAA MRMS

- Provider: NOAA Open Data Dissemination on AWS, public bucket
  <https://registry.opendata.aws/noaa-mrms-pds/>.
- Selected catalog prefixes and their current object dates are checked by `scripts/feed-contracts.sh`.
- The public bucket listing currently publishes the 0, -10, and -20 °C isothermal reflectivity
  grids used by the metadata-driven catalog; their prefixes and recent objects are contract-tested.
- The NWS Warning Decision Training Division MRMS product guide defines 18 dBZ echo tops as the
  maximum height AGL of that reflectivity surface and VIL as vertically integrated liquid water:
  <https://training.weather.gov/wdtd/courses/MRMS/index.php>.
- Live objects confirm native echo-top values are kilometres AGL and VIL is kg/m²; HookEcho keeps
  those native values for sampling and applies display palettes only during upload.
- NOAA's operational GRIB2 table defines FLASH QPE-to-FFG 1/3/6-hour and maximum products as
  unitless ratios at two-minute cadence; HookEcho displays the native 0–5 ratio scale:
  <https://www.nssl.noaa.gov/projects/mrms/operational/tables.php>.
- The same table defines CREST maximum unit streamflow as m³/s/km² and maximum soil saturation
  as percent; both public prefixes and recent objects are contract-tested.
- It also defines `HeightCompositeReflectivity` and `HeightLowLevelCompositeReflectivity` in
  metres MSL and `SeamlessHSRHeight` in kilometres AGL; all three public prefixes and recent
  objects are contract-tested.

## NOAA RTMA / URMA

- RTMA v2.10 immutable analysis objects and text indexes are read from NOAA NOMADS:
  <https://nomads.ncep.noaa.gov/pub/data/nccf/com/rtma/v2.10/>.
- URMA objects are read from <https://nomads.ncep.noaa.gov/pub/data/nccf/com/urma/prod/>. NOAA
  publishes no usable sidecar index there, so HookEcho walks the self-sized GRIB message headers
  with 512-byte range requests and downloads only the selected field message.
