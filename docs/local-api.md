# Local HTTP API

Start `hookecho --serve 8080` to serve on `127.0.0.1`. The port is optional. Use
`--bind ADDRESS` only when the server must be reachable from another machine.
Set a bearer token in Settings or pass `--serve-token TOKEN`; then send
`Authorization: Bearer TOKEN` with each request. The server accepts `?token=`
for clients that cannot set headers, but URLs can enter logs and browser history.
`--public` permits only the fixed, known-site image presets without a token.

All routes below use `GET` and return JSON except the image/video routes. The
`/v1` prefix is the versioned contract; legacy `.json` and image paths remain
available. Feed failures return a JSON `error` body. Times are UTC.

| Route | Result / query |
|---|---|
| `/v1/status` | Saved-location observations and alerts; refreshed at most once per minute. |
| `/v1/alerts` | Saved-location warning summaries from the same cached report. |
| `/v1/health` | Process uptime, feed success, report age, and cached snapshot count. |
| `/v1/products` | Radar sites, radar moments, and MRMS field identities and units. |
| `/v1/cells?site=KTLX` | Radar-reported storm cells and projected tracks. |
| `/v1/frame?site=KTLX&product=REF` | Selected radar object, valid time, units, and tilt; optional `date=YYYY-MM-DD` and `time=RFC3339` select an archived scan. |
| `/v1/probe?site=KTLX&product=REF&lon=-97.3&lat=35.3` | Native radial/gate sample with source object, time, range, and beam height. Accepts the same archive selectors as `/v1/frame`. |
| `/v1/snapshot.png?site=KTLX&product=REF` | Rendered radar still. Optional `size` (256–2048), `zoom`, `tilt`, `basemap`, and built-in `palette` select framing and display. |
| `/v1/loop.gif` and `/v1/loop.mp4` | Radar loop. Accepts still options plus `frames` (2–12), `fps` (1–10), `timing=fixed|source`, `date`, and `time`. |

For example:

```sh
curl -H 'Authorization: Bearer TOKEN' 'http://127.0.0.1:8080/v1/frame?site=KTLX&product=REF'
```

The server is a separate headless process. These routes describe its configured
locations and requested radar frame; they do not expose the desktop app's live
camera, panes, or open workspace. A shared desktop view-state endpoint remains
part of the M4 roadmap gate.
