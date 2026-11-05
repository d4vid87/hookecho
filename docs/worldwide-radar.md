# Worldwide radar

Worldwide radar uses regional observations where they are published. It does not turn satellite
rainfall estimates into ground radar, or imply that every country publishes Doppler volumes.
Saved map positions are retained; regional overview selects an appropriate composite instead of
moving everyone to the United States. Single-site products remain limited to fields the selected
radar actually provides.

## Sources and capabilities

| Region | Source | Data and limitations |
| --- | --- | --- |
| United States | NOAA MRMS / NEXRAD / TDWR | Existing national composite and detailed site products. |
| Europe | [EUMETNET OPERA](https://eumetnet.github.io/openradardata-documentation/1-ORD-API-overview/) and DWD | Regional reflectivity plus available site volumes. The 1 km OPERA composite is sampled at 2 km for bounded overview memory. Available products and history differ by site; station registry membership is not proof of current availability. |
| Canada | [ECCC GeoMet](https://eccc-msc.github.io/open-data/msc-data/obs_radar/readme_radar_geomet_en/) | Published rain/snow composite imagery and advertised frame times. Not native velocity or elevation volumes. |
| Japan | [JMA observed high-resolution precipitation](https://www.jma.go.jp/bosai/nowc/) | Five-minute rainfall analysis tiles, not dBZ. Only `targetTimes_N1.json` entries advertising `hrpns` with equal base and valid times enter playback; forecast frames are excluded. The public website tile route is not a guaranteed stable API. |
| Taiwan | [CWA O-A0059-001](https://opendata.cwa.gov.tw/dataset/observation/O-A0059-001) | Numeric composite dBZ, normally every ten minutes. Public CWA S3 distribution needs no embedded key. Coordinates are TWD67 and transformed for the WGS84 map. The latest-file URL cannot retrieve arbitrary history: HookEcho retains up to two actual downloaded frames, identified by the XML observation time. |
| South Korea | [KMA HSR](https://apihub.kma.go.kr/apiList.do?seqApi=5) | Authenticated 500 m composite dBZ. Requires a server-side KMA API Hub key with radar access. Implemented but not live-verified without that key; configured does not mean the subscription or upstream feed has been verified. |
| Other supported radar regions | [RainViewer](https://www.rainviewer.com/api/weather-maps-api.html) | Public composite fallback, with coverage gaps. Current public limits include zoom 7, ten-minute frames, and 100 requests/IP/minute. Shared per-provider request budgeting applies across panes. |
| Satellite precipitation | [NASA GPM IMERG](https://gpm.nasa.gov/data/directory) | Separate precipitation estimate layer. It has substantial processing latency and is not radar reflectivity or Doppler velocity. |

Source attribution remains visible. JMA's [use conditions](https://www.jma.go.jp/jma/kishou/info/coment.html)
require source identification and disclosure of processing. CWA's [product specification](https://www.cwa.gov.tw/Data/data_catalog/3-6-1.pdf)
identifies the official dataset and download products. Regional feeds retain their own attribution
and licensing requirements.

The CWA `-99` and `-999` sentinels never become zero reflectivity. `-999` combines areas outside
radar coverage and values removed by quality control, so it cannot prove clear weather. The
published three-translation TWD67 datum approximation is tested against Taichung City's paired
City Hall coordinates; it is suitable for this roughly 1.25 km grid, not surveying.

## Configure South Korea without shipping a key

Obtain a KMA API Hub account and enable access to the radar file list and HSR composite APIs.
Set `KMA_API_KEY` in the **server** environment:

- Cloudflare Pages: configure a secret binding named `KMA_API_KEY` for the application project.
- Native `hookecho --serve`: supply `KMA_API_KEY` through the process supervisor's secret or
  environment mechanism. Existing server bearer-token/public-route restrictions still apply.

Never put the key in a web build, settings shared with clients, source control, a public URL, or a
sample command. Unconfigured native clients can use the fixed `app.hookecho.io` gateway when that
deployment is configured. No key is included in desktop packages.

The gateway exposes only:

| Route | Response |
| --- | --- |
| `/regional-radar/status` | `{"kma":true}` when a server credential is configured; this is configuration status, not a successful data-feed probe. |
| `/regional-radar/kma/catalog?day=YYYYMMDD` | The official HSR file listing for a recent Korean calendar day. |
| `/regional-radar/kma/grid?time=YYYYMMDDHHMM` | The fixed HSR/MSK/ECHO/HB ASCII composite for an explicitly selected observation time, within the last 48 hours. |

Times in gateway parameters use Korean Standard Time (UTC+9); application playback timestamps
use UTC. Catalog times come from actual advertised HSR filenames. Requests cannot choose an
upstream host, product, or authentication key. Redirects are refused, bodies are bounded, and
provider errors do not return the key or private upstream URL. Cache keys contain only the public
route and timestamp. Data access still depends on the KMA account's permissions and quota.

The KMA grid has 2305 × 2881 cells, so each decoded frame retains approximately 13 MB of `i16`
values. A full ASCII response can approach 48 MB before compression. Avoid loading historical
frames that are not selected; use composites at overview zoom and only a selected site's volume
for detailed analysis. The European, Taiwanese, and Korean composites do not expose invented
elevations, velocity, or dual-polarization products.

Decoded composites retain at most two frames per provider. Advertised frame lists are bounded
and shared across panes without replacing another pane's historical selection. Weather tiles
share an 8 MiB memory cache, four concurrent upstream reads, and an 80-request/minute RainViewer
budget; cached reads do not spend that budget. Tile loads wait briefly for concurrency slots.
The normal browser decode worker handles the large OPERA/CWA/KMA grids; browsers without a
working worker fall back to inline decoding and may pause during a frame change. Raster tile
projection yields between row batches in browser builds.

The optimized main web bundle measured 4,877,777 gzip bytes after these additions, about 85 KB
(1.8%) above the previous measured bundle. Its size cap is 4,890,000 bytes; Lite and startup
performance limits are unchanged.

## Verification

```sh
cargo test -p wxdata world_radar --lib
cargo test -p hookecho regional_radar_tests --lib
node --test web/_worker.js/regional-radar.test.mjs
```

Provider tests cover observed-versus-forecast times, UTC+8/UTC+9 conversion, datum and projection
orientation, malformed or oversized grids, and missing-data sentinels. Gateway tests cover
credential containment, request validation, redirects, upstream failures, and streaming headers.

On 2026-11-04, JMA's live observation catalog and 256 px tiles at zooms 4, 8, and 10 returned HTTP
200. CWA's live numeric XML and transparent image products also returned HTTP 200. These checks
do not establish continuous feed availability. KMA authenticated delivery, provider quotas, and
physical-device behavior still require deployment credentials and real-device testing.
