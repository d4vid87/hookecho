# Hook Echo-WX for Android

The Android app is the **same Rust codebase** as the desktop build, compiled to a `cdylib`
(`libhookecho.so`) and loaded by a `GameActivity` — `android_main` in
[`crates/hookecho/src/lib.rs`](../crates/hookecho/src/lib.rs) is the entry point, and Rust draws
every pixel. The Kotlin is deliberately thin: the activity itself (deep links + predictive back),
the background alert service, and the home-screen widget.

GameActivity rather than NativeActivity for one reason — GameTextInput is the real system IME.
Its editor state is bridged into egui events by `platform::pump_ime`; nothing upstream does that
for you. It also requires an AppCompat theme (`res/values/themes.xml`) or the activity throws on
creation.

- **Target:** `arm64-v8a`, minSdk 29 (Android 10), Vulkan.
- **What differs from desktop:** storage routes to the app-private data dir
  ([`paths.rs`](../crates/hookecho/src/paths.rs)); native file dialogs are replaced by a fixed
  `exports/` folder ([`dialog.rs`](../crates/hookecho/src/dialog.rs)); the tray, gpsd GPS, and MP4
  export (ffmpeg) are hidden; the UI is a Material 3 layout (map-first, one persistent bottom sheet with snap points, a docked
  toolbar, full-screen surfaces instead of floating windows) rather than the desktop chrome —
  see [`app/mobile/`](../crates/hookecho/src/app/mobile/) and the tokens in
  [`ui/m3.rs`](../crates/hookecho/src/ui/m3.rs); field grids decimate to
  the device's real texture cap. Everything else — the radar/MRMS/HRRR pipelines and every data
  feature — is shared, unchanged.

## Build locally

Prerequisites:

- Android SDK + NDK (r26+); set `ANDROID_HOME` and `ANDROID_NDK_HOME`
- JDK 17+
- `rustup target add aarch64-linux-android`
- `cargo install cargo-ndk`

Then, from the repo root:

```sh
android/build.sh            # release .so in a debug-signed APK (directly sideloadable)
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

`INSTALL=1 android/build.sh` installs it for you. The Rust library is always
optimised; `build.sh debug` builds an opt-level-0 `.so` with symbols for native crash
hunts, and `build.sh signed` produces a release-signed APK. `FEATURES=profiling
android/build.sh` enables the puffin server (see `crates/hookecho/src/profiling.rs`).

Gradle comes from the committed wrapper (`android/gradlew`), so the Gradle version is identical
locally and in CI. `versionCode`/`versionName` are derived from the workspace `Cargo.toml`
(0.5.0 → `50000`/`0.5.0`) — bump the crate version, not the Gradle file.

## F-Droid

The app qualifies: no proprietary dependencies, no Google Play Services, no telemetry. Store
listing text and screenshots live in
[`fastlane/metadata/android/en-US/`](fastlane/metadata/android/en-US/).

The recipe in `fdroiddata` (F-Droid's repo, not this one) needs a `sudo`/`prebuild` pair that
produces `libhookecho.so` before Gradle runs — verbatim:

```yaml
    sudo:
      - curl -sSf https://sh.rustup.rs -o /tmp/rustup.sh
      - sh /tmp/rustup.sh -y --default-toolchain 1.96.1 --profile minimal
      - . $HOME/.cargo/env
      - rustup target add aarch64-linux-android
      - cargo install cargo-ndk --version 4.1.2 --locked
    prebuild:
      - . $HOME/.cargo/env
      - cargo ndk -t arm64-v8a -o app/src/main/jniLibs --manifest-path ../Cargo.toml build --release --lib -p hookecho
    ndk: r26d
    subdir: app
    gradle:
      - yes
```

Notes for the MR: the toolchain is pinned (`1.96.1`, matching CI) and `cargo-ndk` is version-pinned
for reproducibility (`Cargo.lock` is *not* committed — expect this to be the review round-trip;
`cargo generate-lockfile` in `prebuild` or committing the lockfile are the two answers); the `.so` lands in `app/src/main/jniLibs/` and
a Gradle `preBuild` hook deletes anything in that directory that is not `libhookecho.so`.

## First run

The setup wizard opens (home radar, theme, alerting).

Background alerting is opt-in from Settings. When it is on, a foreground service polls
`api.weather.gov` for your saved markers (60 s while something is warned, 5 min otherwise) and a
15-minute WorkManager job (`AlertWorker`) runs the same poll as a safety net — it survives process
death, and `BootReceiver` re-arms it after a reboot. Deep-Doze worst case is therefore ~15 minutes;
the app does not ask for a battery-optimization exemption or exact alarms. ntfy.sh push (set a
topic in Settings) still works and is the option that does not cost a permanent notification.

## Status

Phases 0–1 of the port (lib split, Android entry, platform gating, storage, touch input, CI) are in
place. On-device polish (soft-keyboard IME, drawer toolbox, JNI GPS, Storage-Access-Framework
import, Play Store) is tracked as deferred work in the port plan. The APK build itself runs in the
release workflow; a physical device is needed to exercise the GUI.
