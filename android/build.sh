#!/usr/bin/env bash
# Build the Hook Echo-WX Android APK (arm64-v8a, debug — directly sideloadable).
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
BUILD_TYPE="${1:-debug}" # debug (default, sideloadable) | release (needs signing)

cd "$(dirname "$0")/.."   # repo root

echo "== cargo-ndk: building libhookecho.so ($ABI, $BUILD_TYPE) =="
# --lib: only the cdylib matters on Android (the bin target is desktop-only).
NDK_FLAGS=(-t "$ABI" -o android/app/src/main/jniLibs build --lib -p hookecho)
[ "$BUILD_TYPE" = "release" ] && NDK_FLAGS+=(--release)
cargo ndk "${NDK_FLAGS[@]}"

echo "== gradle: assembling APK =="
cd android
GRADLE="${GRADLE:-./gradlew}"
command -v "$GRADLE" >/dev/null 2>&1 || GRADLE="gradle"  # fall back to a system Gradle
if [ "$BUILD_TYPE" = "release" ]; then
    "$GRADLE" assembleRelease
    APK="app/build/outputs/apk/release/app-release.apk"
else
    "$GRADLE" assembleDebug
    APK="app/build/outputs/apk/debug/app-debug.apk"
fi

echo
echo "APK: android/$APK"
echo "Install: adb install -r android/$APK"
