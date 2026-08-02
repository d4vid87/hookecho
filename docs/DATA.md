# Where the data comes from

Every feed Hook Echo-WX decodes, what it costs you, and how fresh it is. All of
it is public; nothing is proxied through a server of ours, because there isn't
one — the app talks to NOAA, the NWS and the rest directly from your machine.

"Latency" is how old the newest thing you can see already is when it arrives.
Cadence is how often a new one appears. Both are the services' numbers, not
ours, and both move around during busy weather.

## Radar

| Feed | Source | Cadence | Latency | Key |
| --- | --- | --- | --- | --- |
| Level 2 archive | NEXRAD Archive II on AWS (`noaa-nexrad-level2`) | one volume, ~4–6 min | minutes after the volume closes | no |
| Level 2 live chunks | `unidata-nexrad-level2-chunks` on AWS | sweep by sweep, seconds | ~30 s behind the beam | no |
| Level 3 products (NST, NMD, DVL, EET, HHC) | Unidata S3 bucket | per volume | ~1–2 min | no |
| Level 3 storm structure / hail (SS, HI) | NWS `tgftp` (`sn.last`) | per volume | ~1–2 min | no |
| N0B mosaic (multi-radar stitch) | Unidata S3, six nearest sites | per volume, per site | ~1–2 min; sites disagree by a scan | no |
| TDWR (44 airport radars) | Unidata S3 Level 3 tilts (TZ0–2, TV0–2) | ~1 min | ~1–2 min | no |
| MRMS national mosaic and fields | NOAA MRMS on AWS | ~2 min | ~2–4 min | no |

Archive coverage runs back to **June 1991** for Level 2, and every WSR-88D plus
the 44 TDWRs are addressable.

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
| Area forecast discussions | `api.weather.gov` products | per issue (~2×/day + updates) | minutes | no |
| Surface fronts | WPC `CODSUS` bulletin | ~4×/day | ~1 h | no |
| Tropical cyclones (positions, cones, tracks) | NHC `CurrentStorms.json` + MapServer | per advisory (6 h, plus intermediates) | minutes | no |
| Storm surge | NHC map service | per advisory | minutes | no |

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
- **Any telemetry.** The app makes no request that isn't a data feed you can see
  in this table.

## Keys

The keyed feeds (Mapbox, MapTiler, Tempest, Weather Underground, Windy, mPING,
AirNow) are entered in Settings and live only in your own settings file. Without
them, those specific layers stay empty and say so; nothing else changes. No key
of any kind ships in this repository.
