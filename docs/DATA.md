# Where the data comes from

Public data sources, their delivery paths and timestamp meanings. Native clients
generally read upstream directly. Browser/PWA clients use the allowlisted
same-origin feed proxy where required; `--serve` provides the local equivalent.
Optional provider keys belong to the user.

Observation age is time since acquisition, not time since download. Publication,
first-seen, receipt and product-generation clocks are different. Cadence depends
on scan strategy and upstream conditions; the values below are descriptive, not
latency guarantees. See the [checked primary sources](radar-freshness-research-2026-11.md)
and [measurement report](radar-freshness-2026-11.md).

## Radar

| Feed | Source | Delivery / timing meaning | Key |
| --- | --- | --- | --- |
| Level II archive | `unidata-nexrad-level2` on AWS | Completed volumes; selected moment/tilt time comes from radial acquisition. The old `noaa-nexrad-level2` bucket retired September 1, 2025. | no |
| Level II live chunks | `unidata-nexrad-level2-chunks` | Incremental sweeps, including supplemental low-level revisits. Acquisition bounds, chunk Last-Modified and receipt are tracked separately. Missing or retained sectors are not new observations. | no |
| Level II alternate | Iowa Environmental Mesonet recent volumes | Alternate delivery of the selected station's observations; “Iowa State” identifies the provider, not the radar's location. | no |
| Level III products | Unidata S3 and NWS `tgftp` `sn.last` | PDB generation/volume clocks are distinct from message transmission. DVL/EET generation fields carry volume-end time per the ICD. `sn.last` is mutable and revalidated on a short cache policy. | no |
| TDWR | Unidata S3 Level III tilts TZ0–2 / TV0–2 | Product generation time is available; individual beam acquisition times are unavailable. Separate product/tilt clocks can differ. | no |
| N0B multi-radar mosaic | Unidata S3, nearby sites | Independently timed single-site products; constituent sites can disagree in age. | no |
| MRMS national mosaic and fields | NOAA MRMS on AWS | Product valid time from GRIB metadata; cadence varies by product. A mosaic's valid time does not prove every contributing observation has that age. | no |
| Regional and worldwide radar | National providers, EUMETNET OPERA/ODIM, regional composites and RainViewer fallback | Capability, attribution, source time and geographic availability differ. This is not worldwide equivalent-resolution single-site Level II coverage. See `world_radar.rs`, `opera.rs`, `dwd.rs`, `eccc.rs` and regional adapters. | depends on provider |

Immutable archive objects remain cacheable. Day listings, live chunk discovery
and mutable latest pointers have bounded freshness; the service worker does not
cache radar/proxy responses. User-requested offline packs remain a separate
feature. Upstream Last-Modified is HTTP object metadata, not observation time.

## Warnings, outlooks and reports

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| Active alerts | `api.weather.gov/alerts/active` | polled, ~1 min | seconds after issue | no |
| Archived warning polygons | IEM `sbw.py` GeoJSON | on demand, any past instant | archival | no |
| SPC Day 1–3 outlooks | SPC static GeoJSON | per outlook issue | minutes | no |
| SPC mesoscale discussions | NWS map service | as issued | minutes | no |
| WPC excessive rainfall outlook | WPC map service | per issue | minutes | no |
| Local storm reports (live + archive) | IEM `lsr.geojson` | as filed | minutes to hours (human-filed) | no |
| Tornado climatology (1950–2022) | SPC severe-weather database CSV | annual | archival | no |
| NWS damage surveys | DAT ArcGIS Feature Server | days after the storm | days | no |
| ProbSevere v3 | NOAA/CIMSS | ~2 min | ~2–4 min | no |
| Aviation SIGMET/AIRMET | aviationweather.gov | as issued | minutes | no |
| Winter storm severity index | WPC map service | per day/issue | minutes | no |

## Observations

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| METAR station plots | aviationweather.gov | hourly (plus specials) | minutes | no |
| Nearest-station conditions | `api.weather.gov` observations | ~hourly | minutes | no |
| NDBC buoys and coastal stations | `latest_obs.txt` | ~hourly | ~1 h | no |
| mPING precipitation type | OU/NSSL mPING API | as reported | minutes | **yes** (free) |
| PIREPs | aviationweather.gov | as filed | minutes | no |
| Hurricane-hunter HDOBs | NWS `tgftp` recon bulletins | 30 s in-storm | ~2–5 min | no |
| River flood gauges | NOAA NWPS | ~15 min–1 h | ~1 h | no |
| Air quality (AQI) | AirNow (EPA) | hourly | ~1 h | **yes** (free) |
| Personal weather stations | WeatherFlow Tempest | 1 min | seconds | **yes** (yours) |
| Personal weather stations | Weather Underground | ~5 min | minutes | **yes** (yours) |
| Storm spotter positions | Spotter Network placefile | 1 min | ~1 min | no |
| Radiosonde soundings (RAOB) | University of Wyoming | 00Z / 12Z | 1–2 h | no |
| GOES GLM lightning | GOES-East/West on AWS, netCDF-4 granules | 20 s | **~40 s** | no |
| Snowfall analysis (NOHRSC) | NOHRSC gridded GRIB2 | 4×/day (00/06/12/18Z) | ~1–3 h | no |

## Models and forecasts

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| HRRR future radar (REFC) | HRRR on AWS, byte-range GRIB2 | hourly runs, F00–F18 | **~1–2 h** behind the run hour | no |
| HRRR fields (wind, CAPE, SRH, snow, smoke) | same | hourly runs | ~1–2 h | no |
| RAP mesoanalysis | RAP on AWS | hourly runs | ~1 h | no |
| NWS point forecast | `api.weather.gov` gridpoints | ~hourly | minutes | no |
| Point forecast outside the US | Open-Meteo (ECMWF/GFS/ICON) | hourly | minutes | no |
| Area forecast discussions | `api.weather.gov` products | per issue (~2×/day + updates) | minutes | no |
| Surface fronts | WPC `CODSUS` bulletin | ~4×/day | ~1 h | no |
| Global models (GFS, GEFS mean/spread and members, ECMWF open IFS) | `noaa-gfs-bdp-pds` / `noaa-gefs-pds` on AWS; `data.ecmwf.int` | 6-hourly runs | ~3–5 h behind the run hour | no |
| Tropical cyclones (positions, cones, tracks) | NHC `CurrentStorms.json` + MapServer | per advisory (6 h, plus intermediates) | minutes | no |
| Storm surge | NHC map service | per advisory | minutes | no |

Selected-point temperature history samples native GFS f00 from the three preceding six-hour cycles and f00–f12 from the newest usable cycle. Missing objects leave gaps; run time and valid time remain distinct in the chart and exports.

## Analysis and verification

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| CONUS RTMA / URMA temperature, dewpoint, pressure, 10 m U/V wind, 10 m gust, surface visibility, and one-hour precipitation | [NOAA NOMADS RTMA filter](https://nomads.ncep.noaa.gov/gribfilter.php?ds=rtma2p5) / [NCEP RTMA system](https://emc.ncep.noaa.gov/emc/pages/numerical_forecast_systems/rtma.php) | hourly analysis; precipitation uses separate RTMA/URMA GRIB objects and labels the interval end | valid-data age shown from decoded frame | no |
| Warning verification (POD, FAR, CSI, lead time) | IEM "Cow" (`mesonet.agron.iastate.edu`) | on demand, per office and day | the report database's own lag (days) | no |
| Ionospheric electric field (PPEF) | NOAA/NCEI PPEF model | 5 min | ~1 h ahead (a forecast) | no |
| Ground electric field | a field mill you run and point the app at | yours | yours | no |

## Imagery, maps and cameras

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| Basemap (vector and raster) | OpenMapTiles / Carto | static | — | no |
| Basemap (Mapbox styles) | Mapbox | static | — | **yes** (yours) |
| Basemap (MapTiler styles) | MapTiler | static | — | **yes** (yours) |
| FAA WeatherCams | FAA (US only, thick in Alaska) | ~10 min stills | minutes | no |
| Caltrans highway cameras | Caltrans, 12 districts | live HLS or periodic stills | seconds to minutes | no |
| Windy webcams (global) | Windy | ~10 min | minutes | **yes** (free) |
| Wildfire perimeters and incidents | WFIGS ArcGIS | daily perimeters | ~1 day | no |
| Geocoding | OpenStreetMap Nominatim | on demand | — | no |
| NOAA Weather Radio audio | third-party relays you supply | live | seconds | no |

## Things deliberately absent

- **Blitzortung lightning.** Its terms of service forbid use in an application,
  so it stays out however good the network is. GLM and the MRMS lightning
  density layer cover the gap from public sources.
- **A vendor "premium" radar API.** Everything above is a government or
  volunteer feed; there's no tier of this app that unlocks better data.
- **Any telemetry.** Nothing reports on you, your machine or your usage — there
  is no analytics endpoint and no server of ours to send one to.

  For completeness, the requests the app makes that are not feeds in this table,
  all of them optional or user-initiated: a GitHub Releases check for a newer
  version; the opt-in Anthropic digest, which sends the discussion text you
  asked to be summarized and needs your own API key; Google OAuth and Drive,
  when you turn on settings sync, into your own account's app folder; and the
  ntfy/Discord/Slack/Matrix push you configured, to the address you gave. Each
  is off unless you turn it on.

## Keys

The keyed feeds (Mapbox, MapTiler, Tempest, Weather Underground, Windy, mPING,
AirNow) are entered in Settings and live only in your own settings file. Without
them, those specific layers stay empty and say so; nothing else changes. No key
of any kind ships in this repository.
