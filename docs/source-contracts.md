# External source contracts

Adapters in this roadmap are added only after their public contract is checked against an official
provider source. Live network checks belong in scheduled CI; ordinary tests use committed fixtures.

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
