# Device checks before a stable release

Record the commit, APK signature/version, device model, OS/browser version, date,
and pass/fail evidence. A changed user agent in desktop Chrome is a simulation,
not a Family Hub hardware result. Keep a failed or untested device marked that way.

## Samsung Android phone

Run `scripts/checks/android-device.sh path/to/HookEcho-arm64-v8a.apk output-dir`
with one unlocked USB-debugging device connected. This installs an update without
clearing app data, checks launch and background/resume, and saves a screenshot,
memory report, and app logs. Never uninstall to work around a signature mismatch.

The real-activity instrumentation tests live in
`android/app/src/androidTest/.../DeviceSmokeTest.kt`. After building the native
library with `android/build.sh`, run `cd android && ./gradlew connectedDebugAndroidTest`
on an arm64 device using the same signing configuration as the installed app.
They exercise every shelf tab, native activity background/resume, offline speech,
and recoverable missing voice data. The native activity intentionally ends its process
on finish, so the runner leaves it open until results are reported. Gradle configures
this automatically; when using `adb shell am instrument`, pass
`-e waitForActivitiesToComplete false`. CI compiles the test APK; executing it
requires compatible hardware.

Then check these flows on the phone:

- Open radar, pan/zoom, play through the full loop, and switch stations while
  offline. The previous scan must keep its original station label until replaced.
- Open every Quiet Shelf tab and close it; verify the map and playback stay usable.
- Switch between two and four panes and Radar, Chase, National, and Analysis.
- Change the system language to German and tap a warning notification and widget.
  The destination must be the saved point; restore the original language afterward.
- Check quiet hours across midnight. With deterministic test alerts, verify that
  warnings respect quiet hours and emergencies retain their existing bypass.
- Disable the foreground app, resume it, and reboot once with background alerts
  enabled. Verify service/worker recovery, notification delivery, and widget state.
- Interrupt connectivity during alert checks. Incomplete checks must be reported;
  received warnings must remain visible, and a complete successful check clears
  the incomplete status. Do not interpret a live day without alerts as a delivery test.
- Run playback for 30 minutes, including screen rotation and background/resume.
  Record crashes, temperature, memory growth, missing sectors, and control glitches.

## Samsung Family Hub

On the actual refrigerator browser, record the full model and browser version:

- Open `app.hookecho.io`, load the saved radar, and run playback for 30 minutes.
- Pan/zoom, switch stations, and exercise two/four panes and every view mode.
- Interrupt and restore Wi-Fi; verify recovery and honest scan-age/station labels.
- Leave the app visible past the appliance's normal sleep interval. Verify the
  screen remains awake where the browser/appliance permits it. Record any device
  setting needed; a mocked wake-lock API is not evidence that the appliance stays on.
- Save photos of the running UI and any startup/render error. Test reload and Retry.

## Release decision

Automated CI is mandatory for the exact commit. Use a preview release until the
target-device checks pass. An unavailable physical device remains an explicit
verification gap; browser simulation and Kotlin unit tests do not close it.

For rollback, download a previous immutable release from GitHub Releases and
verify its `SHA256SUMS`. Android updates must retain the same signing key; Android
may refuse a version-code downgrade, so keep a backup and do not erase app data.
