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
# Compile the data with a host executable; the tag archive only contains language sources.
cmake -S "$work/espeak-ng" -B "$work/espeak-host" -DBUILD_TESTING=OFF \
  -DUSE_MBROLA=OFF -DUSE_LIBSONIC=OFF -DUSE_LIBPCAUDIO=OFF -DUSE_ASYNC=OFF
cmake --build "$work/espeak-host" --target data --parallel 2
for data in phontab phondata phonindex intonations en_dict; do
  test -s "$work/espeak-host/espeak-ng-data/$data"
done
# Cross-compilation must not try to execute its ARM data compiler on the build host.
sed -i '/include(cmake\/data.cmake)/d' "$work/espeak-ng/CMakeLists.txt"
sed -i '/add_subdirectory(tests)/d' "$work/espeak-ng/CMakeLists.txt"
"$root/packaging/piper/fetch.sh" web "$work/voice"
rm -rf "$assets"
mkdir -p "$assets"
cp "$work/voice/en_US-amy-medium.onnx" "$work/voice/en_US-amy-medium.onnx.json" \
  "$work/voice/Amy-LICENSE-CC-BY-SA-4.0" "$work/voice/ATTRIBUTION" "$assets/"
cp -a "$work/espeak-host/espeak-ng-data" "$assets/"
cp "$work/espeak-ng/COPYING" "$assets/eSpeak-NG-LICENSE-GPL-3.0"
printf '%s\n' "eSpeak NG 1.52.0 source: $url" > "$assets/SOURCE"
