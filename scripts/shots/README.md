# HookEcho media capture

The desktop media set uses the Radar Zeta layout in reproducible archive scenes, live weather scenes, and browser captures of the deployed web app. The Android phone media shows Signal Deck and is captured separately. Review each image before committing: a successful screenshot command does not prove that radar data or a panel populated.

```sh
./scripts/shots/shoot.sh archive   # desktop archive stills and hero GIF
./scripts/shots/shoot.sh live      # desktop live-weather stills
./scripts/shots/shoot.sh check     # references and size budget
node scripts/shots/capture-web-showcase.cjs
./scripts/shots/encode-web-showcase.sh
HOOKECHO_CAPTURE_ORIGIN=http://127.0.0.1:8767 node scripts/shots/mobile-browser.cjs
./scripts/shots/encode-mobile-showcase.sh
./scripts/shots/encode-android-showcase.sh
```

`shoot.sh` runs the release binary under nested Xvfb with a scratch profile. Historic scenes use `HOOKECHO_GOTO`; live scenes depend on current weather and must be checked for populated data. Its archive set includes reflectivity, velocity, all tilts, products, layers, and a hero animation. The web capture uses archived KTLX for reproducible radar playback and current live data for storm attributes and MRMS. It also writes `warning-detail.png` from the archived Moore warning; convert that frame to `docs/shots/warning-detail.jpg` when refreshing the GitHub gallery. `site/DEMO_CAPTURE.md` records the web and phone source scenes.

The mobile-web set uses a 430×932 browser viewport and the current built app, with the real May 20, 2013 KTLX archive. `mobile-browser.cjs` writes `docs/shots/mobile/` and ten playback frames to `/tmp/hookecho-mobile-frames/`, including capture provenance in `capture.json`. The encoder keeps the entire viewport and writes `phone-current` videos/poster plus the mobile GIF. The mobile browser and native Android layouts differ; captions must identify the actual platform.

The native Android set in `docs/shots/android/` and the Fastlane store screenshots is captured separately from an installed APK. The Android encoder writes `android-current` videos/poster and an Android GIF without overwriting the mobile-web set. Supply fresh raw frames in `/tmp/hookecho-android-frames/` and menu captures in `/tmp/`. Its default crop matches the original 1440×3120 Samsung captures; set `ANDROID_CAPTURE_CROP=width:height:x:y` after checking the current device or emulator insets. Preserve real-device app data and saved places. Record emulator captures as emulator evidence, never as physical-device testing or signed-release verification.

Before committing, inspect the full-size frames and the site at phone and desktop widths. Check that the radar scene is populated, labels and timestamps agree with captions, warning and forecast panels contain real data, and no personal notification or address appears. Then run `./scripts/shots/shoot.sh check`, `npm run build`, and `npm run test:site` from `site/`.
