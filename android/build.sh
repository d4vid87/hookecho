#!/usr/bin/env bash
# Build the Hook Echo-WX Android APK (arm64-v8a, directly sideloadable).
#
# Prerequisites (see android/README.md):
#   - Android SDK + NDK (r26+), with ANDROID_HOME / ANDROID_NDK_HOME set
#   - a JDK 17+
#   - `rustup target add aarch64-linux-android`
#   - `cargo install cargo-ndk`
#
# The Rust `.so` is built by cargo-ndk into app/src/main/jniLibs/, then Gradle packages the APK.
set -euo pipefail

ABI="${ABI:-arm64-v8a}"
# The Rust library is always built --release: an opt-level-0 `.so` is several times slower on
# device and there is no reason to ship one to a phone. Pass `debug` to get symbols back for a
# native crash hunt. The APK itself stays debug-signed (Gradle `assembleDebug`) so it sideloads
# without a keystore — release lib inside a debug-signed package, which is what CI produces too.
BUILD_TYPE="${1:-release}" # release (default) | debug (opt-level 0, symbols) | signed (release APK)
FEATURES="${FEATURES:-}"   # e.g. FEATURES=profiling

cd "$(dirname "$0")/.."   # repo root

echo "== cargo-ndk: building libhookecho.so ($ABI, $BUILD_TYPE) =="
# --lib: only the cdylib matters on Android (the bin target is desktop-only).
NDK_FLAGS=(-t "$ABI" -o android/app/src/main/jniLibs build --lib -p hookecho)
[ "$BUILD_TYPE" != "debug" ] && NDK_FLAGS+=(--release)
[ -n "$FEATURES" ] && NDK_FLAGS+=(--features "$FEATURES")
cargo ndk "${NDK_FLAGS[@]}"

echo "== gradle: assembling APK =="
cd android
GRADLE="${GRADLE:-./gradlew}"
command -v "$GRADLE" >/dev/null 2>&1 || GRADLE="gradle"  # fall back to a system Gradle
if [ "$BUILD_TYPE" = "signed" ]; then
    "$GRADLE" assembleRelease
    APK="app/build/outputs/apk/release/app-release.apk"
else
    "$GRADLE" assembleDebug
    APK="app/build/outputs/apk/debug/app-debug.apk"
fi

cd ..
echo
echo "APK: android/$APK"
if [ "${INSTALL:-0}" = "1" ]; then
    echo "== adb install =="
    adb install -r "android/$APK"
else
    echo "Install: adb install -r android/$APK   (or re-run with INSTALL=1)"
fi
