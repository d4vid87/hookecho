# Clinton Analyst demonstration

The homepage captures are from the real HookEcho web app, using the KEAX Level 2 archive at `2024-04-27T00:29:32Z` (April 26, 7:29 PM CDT), centered near Clinton, Missouri. The single view uses reflectivity. The Analyst preset uses four linked panes at the same scan time: REF (reflectivity), VEL (radial velocity), CC (correlation coefficient), and ZDR (differential reflectivity). The app inspector showed a 0.44° scan. The screenshots and their short playback clips were captured from the app; the website adds the explanatory text and velocity callout. Cropped mobile clips come from the same four-pane capture.

The [NWS Kansas City event report](https://www.weather.gov/eax/2024-04-26-27-Tornadoes) documents a weak radar circulation near Clinton at 7:32 PM CDT, associated with brief EF0 tornadoes southwest and east of Clinton Airport. It reports no injuries or deaths for those brief events. The site's text says *possible rotation* because an individual radar view is not by itself a tornado confirmation; see the [NWS Doppler radar guide](https://www.weather.gov/mlb/Doppler_Dual_Pol_Weather_Radar).

To inspect the source scene in HookEcho, open:

`https://app.hookecho.io/#goto=KEAX,-93.80,38.35,9.4,2024-04-27T00:32:00Z`

Then choose Analyst, four panes, linked maps, and REF / VEL / CC / ZDR. Source video and posters are `public/showcase/clinton-*`. The poster is the 7:29 scan; the five-second clips begin there and step through the next few archived scans. Keep this provenance current if the media is replaced.

## Verification

The homepage was checked in the production Astro preview at 390×844 CSS pixels, device scale 1 and 2, Chromium headless, 150 ms simulated network latency, 200 KB/s download, 75 KB/s upload, and 4× CPU slowdown. Largest Contentful Paint was 1.52–1.53 s; cumulative layout shift was 0.00062. These are local preview measurements, not a field performance guarantee. Run `npm run test:demo` for the desktop/mobile interaction check, then `npm test`, `npm run build`, and `npm run linkcheck` for the existing site gates.
