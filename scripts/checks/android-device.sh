#!/usr/bin/env bash
set -euo pipefail
apk=${1:?Usage: android-device.sh APK OUTPUT_DIRECTORY}
out=${2:?Usage: android-device.sh APK OUTPUT_DIRECTORY}
test -s "$apk"
mapfile -t devices < <(adb devices | awk 'NR > 1 && $2 == "device" {print $1}')
test "${#devices[@]}" = 1 || { echo 'Connect exactly one unlocked USB-debugging device.' >&2; exit 1; }
device=${devices[0]}
mkdir -p "$out"
adb -s "$device" shell getprop > "$out/device.txt"
sha256sum "$apk" > "$out/apk.sha256"
adb -s "$device" install -r "$apk"
adb -s "$device" shell am start -W -n io.hookecho.HookEcho/.MainActivity > "$out/launch.txt"
sleep 15
adb -s "$device" shell pidof io.hookecho.HookEcho > "$out/pid-before.txt"
adb -s "$device" shell input keyevent KEYCODE_HOME
sleep 3
adb -s "$device" shell am start -W -n io.hookecho.HookEcho/.MainActivity > "$out/resume.txt"
sleep 10
pid=$(adb -s "$device" shell pidof io.hookecho.HookEcho | tr -d '\r')
test -n "$pid"
adb -s "$device" logcat -d --pid="$pid" -t 1000 > "$out/app.log"
adb -s "$device" shell dumpsys meminfo io.hookecho.HookEcho > "$out/memory.txt"
adb -s "$device" exec-out screencap -p > "$out/resumed.png"
test -s "$out/resumed.png"
if grep -Eq 'FATAL EXCEPTION|Fatal signal|panicked at|UnsatisfiedLinkError' "$out/app.log"; then
  echo "Crash found; inspect $out/app.log" >&2
  exit 1
fi
echo "Launch/resume smoke passed. Complete docs/device-release-checks.md on this hardware."
