#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
platform="${1:?usage: fetch.sh PLATFORM [OUTPUT_DIR]}"
out="${2:-$root/target/piper/$platform}"
manifest="$root/packaging/piper/assets.json"
mkdir -p "$out"
fetch() {
  local url="$1" digest="$2" path="$3"
  if [ -f "$path" ] && [ "$(sha256sum "$path" | awk '{print $1}')" = "$digest" ]; then return; fi
  curl -fL --retry 3 -o "$path.part" "$url"
  [ "$(sha256sum "$path.part" | awk '{print $1}')" = "$digest" ] || { echo "checksum mismatch: $url" >&2; exit 1; }
  mv "$path.part" "$path"
}
model=(); while IFS= read -r line; do model+=("$line"); done < <(jq -r '.voice.model[]' "$manifest" | tr -d '\r')
config=(); while IFS= read -r line; do config+=("$line"); done < <(jq -r '.voice.config[]' "$manifest" | tr -d '\r')
fetch "${model[0]}" "${model[1]}" "$out/en_US-amy-medium.onnx"
fetch "${config[0]}" "${config[1]}" "$out/en_US-amy-medium.onnx.json"
if [ "$platform" = web ]; then exit 0; fi
archive=(); while IFS= read -r line; do archive+=("$line"); done < <(jq -r --arg p "$platform" '.piper.archives[$p][]' "$manifest" | tr -d '\r')
[ "${#archive[@]}" -eq 2 ] || { echo "unsupported Piper platform: $platform" >&2; exit 1; }
case "$platform" in windows-*) ext=zip;; *) ext=tar.gz;; esac
fetch "${archive[0]}" "${archive[1]}" "$out/piper.$ext"
mkdir -p "$out/runtime"
case "$ext" in zip) unzip -qo "$out/piper.zip" -d "$out/runtime";; tar.gz) tar -xzf "$out/piper.tar.gz" -C "$out/runtime";; esac
if [ -d "$out/runtime/piper" ]; then cp -a "$out/runtime/piper/." "$out/"; fi
find "$out/runtime" -mindepth 1 -delete
rmdir "$out/runtime"
find "$out" -maxdepth 1 -type f \( -name 'piper.zip' -o -name 'piper.tar.gz' \) -delete
curl -fsSL 'https://raw.githubusercontent.com/rhasspy/piper/master/LICENSE.md' -o "$out/Piper-LICENSE-MIT"
curl -fsSL 'https://raw.githubusercontent.com/MycroftAI/mimic3-voices/master/LICENSE' -o "$out/Amy-LICENSE-CC-BY-SA-4.0"
printf '%s\n' 'Amy voice source: https://github.com/MycroftAI/mimic3-voices' > "$out/ATTRIBUTION"
