# HookEcho media capture · November 11, 2026

The website and GitHub gallery were recaptured from HookEcho commit `94a1ae36` with the current Radar Zeta controls and Quiet Contour warning boundaries. Desktop and scientific-gallery frames came from the current WASM web build at `http://127.0.0.1:8767`, backed by the native local proxy and real radar and weather feeds. Desktop screenshots use a 1600 × 1000 browser viewport; no debug/FPS overlay is shown. Phone media use the responsive web app in a 430 × 932 browser viewport; they are not a recording or validation of a native Android build.

## Scenes and provenance

- Radar playback, four-product analysis and four tilts use archived KTLX scans from the May 20, 2013 Moore, Oklahoma event. Historical warning polygons follow the archive time.
- The scientific gallery includes the Tuscaloosa (April 27, 2011), El Reno (May 31, 2013), Mayfield (December 11, 2021), and Hurricane Ian (September 28, 2022) archives. Each caption identifies its event.
- The menu over the national view, MRMS, SPC, model fields, observations and other current-data views show conditions or source availability at capture time on November 11, 2026.
- The recorded HRRR one-hour forecast is initialized at 17:00 UTC on November 11 and valid at 18:00 UTC. The QPE capture shows 24-hour accumulation at 17:00 UTC over northern Illinois, aligned with its radar frame.
- Phone media show the archived Moore storm through the current mobile web interface. Native Android device testing was skipped; no Galaxy device or signed-APK capture is claimed.

## Outputs

`scripts/shots/capture-web-showcase.cjs` stages the web scenes, and `scripts/shots/encode-web-showcase.sh` encodes the website videos, posters and GitHub GIFs. The older native Xvfb workflow remains available in `scripts/shots/shoot.sh`, but its debug frames were not used for this refresh. Full frames are preserved so controls are not clipped; radar scans are not interpolated or synthesized.

The website build copies the shared JPGs and hero GIF from `docs/shots`. Optimized WebP posters, responsive images, mobile movies and showcase assets are refreshed alongside those source images. Existing media aliases remain available with current images for incoming links. Press links point to the same GitHub gallery used by the README.

All screenshots and animations are recorded examples, not live forecasts. Event dates describe the weather being replayed; the November 11 capture date describes the interface build. Live weather is available only by opening the app.
