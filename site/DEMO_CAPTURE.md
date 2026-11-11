# HookEcho media capture · November 11, 2026

The website and GitHub gallery were recaptured from HookEcho commit `94a1ae36` with the current Radar Zeta controls and Quiet Contour warning boundaries. Desktop and scientific-gallery frames came from the current WASM web build at `http://127.0.0.1:8767`, backed by the native local proxy and real radar and weather feeds. Desktop screenshots use a 1600 × 1000 browser viewport; no debug/FPS overlay is shown. The primary phone showcase uses the responsive web app in a 430 × 932 browser viewport. A separate native Android gallery was recorded from the current app in an Android emulator.

## Scenes and provenance

- Radar playback, four-product analysis and four tilts use archived KTLX scans from the May 20, 2013 Moore, Oklahoma event. Historical warning polygons follow the archive time.
- The scientific gallery includes the Tuscaloosa (April 27, 2011), El Reno (May 31, 2013), Mayfield (December 11, 2021), and Hurricane Ian (September 28, 2022) archives. Each caption identifies its event.
- The menu over the national view, MRMS, SPC, model fields, observations and other current-data views show conditions or source availability at capture time on November 11, 2026.
- The recorded HRRR one-hour forecast is initialized at 17:00 UTC on November 11 and valid at 18:00 UTC. The QPE capture shows 24-hour accumulation at 17:00 UTC over northern Illinois, aligned with its radar frame.
- `phone-current` media and `docs/shots/mobile` show the archived Moore storm through the current mobile web interface.
- `android-current` media, `docs/shots/android`, and the Fastlane phone screenshots show the native Radar Focus layout and Quiet Shelf menus replaying the same Moore archive. They were recorded at 18:41 UTC on November 11 in an Android 15 / API 35 Pixel 7 AVD, using an x86_64 debug-signed APK with release-optimized Rust 1.98.0 code. The 1080 × 2400 screencaps were cropped to the 1080 × 2208 app area and resized to 540 × 1104 for stills and movies, or 420 × 859 for the GIF and poster. These are emulator captures; physical-device and signed-release validation were not performed.

## Outputs

`scripts/shots/capture-web-showcase.cjs` stages the web scenes, and `scripts/shots/encode-web-showcase.sh` encodes the website videos, posters and GitHub GIFs. The older native Xvfb workflow remains available in `scripts/shots/shoot.sh`, but its debug frames were not used for this refresh. Full frames are preserved so controls are not clipped; radar scans are not interpolated or synthesized.

The website build copies the shared JPGs and hero GIF from `docs/shots`. Optimized WebP posters, responsive images, mobile movies and showcase assets are refreshed alongside those source images. `encode-mobile-showcase.sh` and `encode-android-showcase.sh` keep the mobile-browser and native Android media separate. Existing media aliases remain available with current images for incoming links. Press links point to the same GitHub gallery used by the README.

All screenshots and animations are recorded examples, not live forecasts. Event dates describe the weather being replayed; the November 11 capture date describes the interface build. Live weather is available only by opening the app.
