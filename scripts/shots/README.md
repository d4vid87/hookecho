# HookEcho media capture

The current media set combines reproducible desktop archive scenes, live weather scenes, a real Android phone running the signed preview APK, and browser captures of the deployed web app. Review each image before committing: a successful screenshot command does not prove that radar data or a panel populated.

```sh
./scripts/shots/shoot.sh archive   # desktop archive stills and hero GIF
./scripts/shots/shoot.sh live      # desktop live-weather stills
./scripts/shots/shoot.sh check     # references and size budget
node scripts/shots/capture-web-showcase.cjs
./scripts/shots/encode-web-showcase.sh
./scripts/shots/encode-android-showcase.sh
```

`shoot.sh` runs the release binary under nested Xvfb with a scratch profile. Historic scenes use `HOOKECHO_GOTO`; live scenes depend on current weather and must be checked for populated data. Its archive set includes reflectivity, velocity, all tilts, products, layers, and a hero animation. The web capture uses archived KTLX for reproducible radar playback and current live data for storm attributes and MRMS. `site/DEMO_CAPTURE.md` records the web and phone source scenes.

The Android set was recorded on a 1440×3120 Samsung device running the signed preview build. `docs/shots/android/` and the Fastlane store screenshots show Signal Deck radar, the Quiet Shelf layers menu, alerts, and more. The encoder crops the phone status bar and builds the phone poster, video, and GIF. It does not clear the phone's app data or saved places. Capture fresh raw frames before running the encoder; its input is `/tmp/hookecho-android-frames/`.

Before committing, inspect the full-size frames and the site at phone and desktop widths. Check that the radar scene is populated, labels and timestamps agree with captions, warning and forecast panels contain real data, and no personal notification or address appears. Then run `./scripts/shots/shoot.sh check`, `npm run build`, and `npm run test:site` from `site/`.
