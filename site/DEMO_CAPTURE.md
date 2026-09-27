# Moore tornado radar demonstration

Recorded from the current HookEcho web app (source `d27cd74`, web module `hookecho_bg-91e87b20.wasm`) using genuine KTLX Level 2 archive scans from May 20, 2013. The [NWS event report](https://www.weather.gov/oun/events-20130520) documents the Moore tornado. Warning geometries are the app’s timestamped [Iowa Environmental Mesonet warning archive](https://mesonet.agron.iastate.edu/geojson/sbw.py), rendered with the current Field Atlas styling. No synthetic radar or warning geometry was added.

Seven scans: **20:08:11, 20:12:29, 20:16:43, 20:20:58, 20:25:11, 20:29:28, 20:33:46 UTC** (3:08–3:33 PM CDT). Each is held for one second in a seven-second loop. The recordings show the compact Quick Launch navigation and cobalt playback controls. The four-pane recording links map positions and timelines and shows REF, VEL, CC, and ZDR. Product sweep acquisition times can differ within a volume.

Source scene:

`https://app.hookecho.io/#goto=KTLX,-97.47,35.36,10.8,2013-05-20T20:08:11Z,REF`

## Assets

`public/showcase/moore-2013-*`: 1280×720 single and four-pane MP4 recordings, WebP first-frame posters, four 630×350 mobile product crops, and a 960×540 single-radar GIF. The website uses smaller MP4 loops and lazy-loaded still fallbacks. All media was assembled with FFmpeg from screenshots captured through the browser UI; scan frames are repeated, not interpolated. Mobile crops come from the same synchronized four-pane frames. The previous static velocity marker was removed because it did not track the moving storm.

## Verification

Verified September 27, 2026: `npm test` (7 passing), `npm run build` (392 pages), and `npm run test:site` (386 pages checked). Browser checks confirmed four-product playback, Pause / Replay, and all mobile product tabs at 390×844 without horizontal page overflow. FFprobe confirmed 1280×720, 210 frames, and seven seconds for the main recording; mobile clips decoded at 630×350. Earlier Clinton capture performance measurements do not apply to these replacement assets.
