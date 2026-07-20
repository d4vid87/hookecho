# Hook Echo-WX for Android

The Android app is the **same Rust codebase** as the desktop build, compiled to a `cdylib`
(`libhookecho.so`) and loaded by a `NativeActivity`. There is no Java/Kotlin — `android_main` in
[`crates/hookecho/src/lib.rs`](../crates/hookecho/src/lib.rs) is the entry point.

- **Target:** `arm64-v8a`, minSdk 29 (Android 10), Vulkan.
- **What differs from desktop:** storage routes to the app-private data dir
  ([`paths.rs`](../crates/hookecho/src/paths.rs)); native file dialogs are replaced by a fixed
  `exports/` folder ([`dialog.rs`](../crates/hookecho/src/dialog.rs)); the tray, gpsd GPS, and MP4
  export (ffmpeg) are hidden; touch adds pinch-zoom and fatter tap targets; field grids decimate to
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
android/build.sh            # debug APK (directly sideloadable)
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

`build.sh release` produces an unsigned/CI-signed release APK instead.

## First run

The setup wizard opens (home radar, theme, alerting). Background alerting is **not** a goal on
Android — the OS starves backgrounded processes; phone alerts come from the existing ntfy.sh push
(set an ntfy topic in Settings and install the ntfy app).

## Status

Phases 0–1 of the port (lib split, Android entry, platform gating, storage, touch input, CI) are in
place. On-device polish (soft-keyboard IME, drawer toolbox, JNI GPS, Storage-Access-Framework
import, Play Store) is tracked as deferred work in the port plan. The APK build itself runs in the
release workflow; a physical device is needed to exercise the GUI.
