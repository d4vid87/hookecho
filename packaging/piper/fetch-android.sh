#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
work="$root/target/piper/android"
assets="$root/android/app/src/main/assets/piper"
archive="$work/espeak-ng.tar.gz"
url="https://github.com/espeak-ng/espeak-ng/archive/refs/tags/1.52.0.tar.gz"
sha="bb4338102ff3b49a81423da8a1a158b420124b055b60fa76cfb4b18677130a23"
mkdir -p "$work" "$assets"
if ! printf '%s  %s\n' "$sha" "$archive" | sha256sum -c --status 2>/dev/null; then
  curl -fL --retry 3 -o "$archive.part" "$url"
  printf '%s  %s\n' "$sha" "$archive.part" | sha256sum -c --status
  mv "$archive.part" "$archive"
fi
rm -rf "$work/espeak-ng"
mkdir "$work/espeak-ng"
tar -xzf "$archive" --strip-components=1 -C "$work/espeak-ng"
# Android packages the release's precompiled espeak-ng-data. The upstream CMake data target tries
# to execute its newly cross-compiled ARM binary on the x86 build host.
sed -i '/include(cmake\/data.cmake)/d' "$work/espeak-ng/CMakeLists.txt"
"$root/packaging/piper/fetch.sh" web "$work/voice"
rm -rf "$assets"
mkdir -p "$assets"
cp "$work/voice/en_US-amy-medium.onnx" "$work/voice/en_US-amy-medium.onnx.json" "$assets/"
cp -a "$work/espeak-ng/espeak-ng-data" "$assets/"
cp "$work/espeak-ng/COPYING" "$assets/eSpeak-NG-LICENSE-GPL-3.0"
printf '%s\n' "eSpeak NG 1.52.0 source: $url" > "$assets/SOURCE"
