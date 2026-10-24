#!/usr/bin/env bash
# Verify the assembled payload before publishing a Linux AppImage.
set -euo pipefail
app="$(realpath "${1:?usage: appimage.sh APPDIR}")"
check_dir="$(mktemp -d)"
trap 'rm -rf "$check_dir"' EXIT
export XDG_CONFIG_HOME="$check_dir/config" XDG_DATA_HOME="$check_dir/data" XDG_CACHE_HOME="$check_dir/cache"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET

# A newer build host must not silently raise the supported Linux runtime floor.
while IFS= read -r -d '' binary; do
  readelf -h "$binary" >/dev/null 2>&1 || continue
  maximum="$(readelf --version-info "$binary" | sed -n 's/.*Name: GLIBC_\([0-9.]*\).*/\1/p' | sort -Vu | tail -1)"
  if [[ -n "$maximum" && "$(printf '%s\n' 2.35 "$maximum" | sort -V | tail -1)" != 2.35 ]]; then
    echo "$binary requires glibc $maximum; build on Ubuntu 22.04 (maximum 2.35)" >&2
    exit 1
  fi
done < <(find "$app/usr" -type f \( -perm -100 -o -name '*.so*' \) -print0)

test -s "$app/usr/lib/libasound.so.2"
test -s "$app/usr/share/doc/hookecho/LICENSE"
test -s "$app/usr/share/doc/hookecho/ALSA-copyright"
"$app/AppRun" --version | tee "$check_dir/version"
grep -q 'built .*UTC' "$check_dir/version"
if grep -q '(unknown;' "$check_dir/version"; then
  echo 'Missing build revision; build from a Git checkout with its worktree metadata accessible' >&2
  exit 1
fi
"$app/AppRun" --help > "$check_dir/help"
grep -q 'Usage: hookecho' "$check_dir/help"
if "$app/AppRun" --unknown-option > "$check_dir/invalid" 2>&1; then
  echo 'Unknown option unexpectedly succeeded' >&2; exit 1
else
  test "$?" -eq 2
fi
"$app/AppRun" --headless-icon "$check_dir/icon.png" 64
test -s "$check_dir/icon.png"
voice="$app/usr/lib/hookecho/piper"
printf '%s\n' 'Tornado warning for Tarrant County. Wind 65 miles per hour. El Niño, McKinney, Bois d’Arc. Temperature minus 5 degrees.' |
  "$voice/piper" --model "$voice/en_US-amy-medium.onnx" --output_file "$check_dir/voice.wav"
python3 - "$check_dir/voice.wav" <<'PY'
import sys, wave
with wave.open(sys.argv[1]) as audio:
    assert audio.getnframes() > audio.getframerate(), 'Voice produced less than one second of audio'
PY
printf '%s\n' 'AppDir checks passed: glibc floor, bundled ALSA, diagnostics, icon, offline speech.'
