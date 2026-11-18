# Radar freshness: platform and release evidence

Checked 2026-11-18. Source baseline: `fb0221e39af3672ff468775c722df0e117837582`
(`0.12.0`); candidate branch: `fix/radar-freshness-20261118`. This document separates
source inspection, local checks, published artifacts, and physical-device evidence.
The accompanying [research evidence](radar-freshness-research-2026-11.md) covers
providers, competitor practices, source terms, and browser/platform guidance.
The [engineering report](radar-freshness-2026-11.md) contains measured results,
source changes, final gate results and rollout limits.

## Supported surfaces and verification boundaries

| Surface | Actual implementation / release path | Existing automated coverage | Evidence from this investigation |
| --- | --- | --- | --- |
| Linux x86-64 | Shared Rust app, wgpu; AppImage and `.deb` built on Ubuntu 22.04, glibc floor 2.35. | Linux workspace build/tests/clippy; lavapipe golden; release package smoke. | Release API lists both packages at the baseline commit. Final workspace tests pass (1,416 tests), native all-target build and strict Clippy pass, and 28 GPU checks pass using isolated Lavapipe. These are Linux source-build checks, not newly published AppImage/deb packages. |
| Windows x86-64 | Same Rust app; ZIP, MSI and setup EXE. | Windows workspace build/tests/clippy; installer/version checks and release executable smoke. | All three artifacts listed in the current release. No Windows runtime or installer execution here. |
| macOS, experimental | Intel build on `macos-15-intel`, `.app` in ZIP; workflow documents Rosetta for Apple Silicon because its bundled Piper distribution is Intel. | Release version and render smoke. | ZIP listed in current release. No macOS/Apple Silicon runtime execution here; no native ARM performance claim. |
| Android ARM64 | Rust `libhookecho.so` in GameActivity, with Kotlin/Compose quick actions and platform services. `minSdk=29`, `compileSdk=35`, `targetSdk=35`, Vulkan 1.1 manifest requirement; APK and AAB. | Separate cargo-ndk ARM64 build/unresolved-C++-symbol check; Kotlin JVM tests; instrumentation APK compilation. | Local Kotlin checks pass after changes: 15 tests; instrumentation APK assembled. No connected device, so no executed activity, graphics, thermal, standby or service-timeout test. The final ARM64 native library was separately rebuilt successfully and has no unresolved C++ runtime symbols. No refreshed APK was installed or executed. |
| Full browser app / PWA | Rust wasm with WebGPU/WebGL2 paths; Family Hub uses the GL preference. `scripts/web/build.sh`; Cloudflare Pages `demo.yml` preview checks before promotion. | wasm compilation; headless Chrome/WebGL tests, simulated Family Hub and lifecycle/reliability scripts; deployed asset/proxy checks. | Final release bundle builds; 90 Node tests and 20 browser validation invocations pass, including default/WebGL/simulated Family Hub, offline/recovery and user workflows. Controlled baseline/candidate startup and interaction measurements plus an interleaved repeat are complete; see the engineering report for tradeoffs. Real IndexedDB storage checks pass. Browser emulation is not a physical Samsung Family Hub result. |
| Lite browser viewer | Separate `crates/hookecho-lite` target under `/lite/`; not the full app's renderer or feature set. | Built with browser bundle; separate deployed Lite smoke. | Final Lite release build and size gate pass (74,611 gzip bytes; 80,000-byte limit). Supplemental KTLX/N0G velocity fixture smoke renders 542,744 nontransparent pixels, displays frame 2/2 and records no page errors. This is a functional smoke, not a Lite latency benchmark. |
| Headless/container | Native `--serve` runtime packaged by Docker and published to GHCR. Release workflow uses ordinary `docker build` on Ubuntu, not a declared multi-architecture build. | Container `--version` provenance smoke. | Native headless MRMS fetched and rendered a current 7000×3500 field successfully. No container image was built/pushed; no registry deployment claim. |

Sources: [CI](../.github/workflows/ci.yml), [release workflow](../.github/workflows/release.yml),
[demo deployment](../.github/workflows/demo.yml), [Android build configuration](../android/app/build.gradle.kts),
[Android manifest](../android/app/src/main/AndroidManifest.xml),
[device release checks](device-release-checks.md).
The separate `site.yml` builds the marketing site at `hookecho.io`; it is not the
application deployment at `app.hookecho.io`. No native iOS package is defined in
these release paths. An AAB artifact alone is not evidence of a Google Play release.

## Android lifecycle, background work and changes

The radar renderer is native Rust/wgpu, not a WebView wrapper. `MainActivity`
tracks foreground state in `onStart`/`onStop`, hosts the Compose shelf, and retains
the existing process-exit behavior on non-configuration destruction for the
one-shot native event loop. This review did not change that behavior or claim a
device reproduction of the historical rationale.

Rust `platform::activity` detects a frame gap longer than three seconds and
native focus regain. App resume invalidates polling/overlay clocks. Android
metered-network status is cached for 60 seconds; a network change can therefore
leave the old polling policy in effect until that cache expires. This is a source
bound, not a measured 60-second radar delay. No connectivity callback was found in
the reviewed path. The app's base radar poll setting is 30 seconds, multiplied by
two for metered data and two for battery saver; faster polling cannot create a
scan that the provider has not published. Stream and resume changes are recorded
by the main investigation, not inferred from Kotlin checks.

The opt-in background NWS alert watcher is a separate Kotlin pipeline. While
running, it waits 60 seconds during a warning, otherwise 300 seconds (900 with
battery saver), after each network pass. A periodic WorkManager request has a
15-minute interval. A one-shot alarm requests five minutes, or 15 in saver, and
re-arms after its pass. Boot enqueues the worker and alarm rather than starting a
`dataSync` service directly. These are requested schedules, not delivery deadlines.

| Finding and confidence | Candidate action | Verification / remaining limit |
| --- | --- | --- |
| **Confirmed API-contract gap:** target 35 `dataSync` service had no `onTimeout(Int, Int)`. Android 15 allows six background hours per 24 hours and requires prompt `stopSelf()` when the callback arrives. | Added the API-35 override: clear `running`, then `stopSelf()`. The user's enabled setting and worker/alarm schedules remain intact. | Compiled against API 35. No on-device forced-timeout test. [Android timeout contract](https://developer.android.com/develop/background-work/services/fgs/timeout). |
| **Confirmed cleanup gap:** a network pass could continue producing notifications after service shutdown. | Service passes its existing `running` state as a cancellation predicate. Check before subsequent requests and before processing alert notifications; skip widget/health updates after stop. Worker/alarm calls keep their existing behavior. | Existing JVM tests pass; service callback execution needs Android instrumentation. In-flight `HttpURLConnection` is not forcibly disconnected: existing connect/read timeouts are 15 seconds each. Cancellation is cooperative and does not claim an atomic boundary against a notification already being dispatched. |
| **Confirmed timestamp ambiguity:** widget used PNG modification time as an unlabeled age. Rust saves the displayed view every five minutes (15 in saver), even when that view contains old radar; caption file only carries storm distance. | Caption now says `Snapshot saved …` and `Radar may be older`; invalid/future file timestamps are explicitly unknown. No observation time is fabricated. | Two deterministic JVM regression tests cover newly saved/old snapshots and invalid/future clocks. No widget screenshot/device rendering check. |
| **Confirmed documentation drift:** README said there were no exact-alarm/exemption requests and claimed a roughly 15-minute deep-Doze worst case. Alarm comments promised on-time five-minute delivery. | Updated README and comments to match the manifest and implementation. Android limits allow-while-idle alarms, including exact alarms, to at most one per nine minutes per app; WorkManager is deferred in Doze. | Source inspection plus [Android idle-mode guidance](https://developer.android.com/training/monitoring-device-state/doze-standby). No measured standby delivery bound. |
| **Remaining source-bound concern:** metered policy cache lasts 60 seconds; no thermal adaptation was found. | Keep policy-cache age visible in future network-switch tests; measure sustained real hardware before adding thermal-dependent workload changes. | No claim that either currently causes a measured latency regression. [ADPF guidance](https://developer.android.com/games/optimize/adpf). |

Implementation: [AlertService](../android/app/src/main/kotlin/io/hookecho/HookEcho/AlertService.kt),
[AlertAlarm](../android/app/src/main/kotlin/io/hookecho/HookEcho/AlertAlarm.kt),
[AlertWorker](../android/app/src/main/kotlin/io/hookecho/HookEcho/AlertWorker.kt),
[RadarWidget](../android/app/src/main/kotlin/io/hookecho/HookEcho/RadarWidget.kt),
[widget tests](../android/app/src/test/kotlin/io/hookecho/HookEcho/RadarWidgetTest.kt),
[Android README](../android/README.md).

Current Play submission readiness is separate from sideload compatibility:
the official policy checked on this date requires API 36 for new mobile apps and
updates from August 31, 2026 (extension ended November 1). The repository still
targets 35. No SDK migration was folded into this radar fix, and no Play submission
or approval is claimed. [Google Play target requirements](https://support.google.com/googleplay/android-developer/answer/11926878).

## Commands actually run and durable records

Artifacts root: `/home/dwm/Documents/HookEcho/radar-freshness-2026-11-18/platforms/`.
The shell's default Java shim had no selected version. Commands used an explicit
environment without changing global configuration:

```text
JAVA_HOME=/home/dwm/.local/share/mise/installs/java/temurin-17.0.20+101
ANDROID_HOME=/home/dwm/.local/share/android-sdk
ANDROID_SDK_ROOT=/home/dwm/.local/share/android-sdk
ANDROID_NDK_HOME=/home/dwm/.local/share/android-sdk/ndk/26.3.11579264
PATH=$JAVA_HOME/bin:$PATH
```

| Command | Result |
| --- | --- |
| `adb devices -l` | No attached devices. Read-only inventory; no physical test requested or attempted. |
| `bash packaging/piper/fetch-android.sh` (repository root) | Exit 0, 21:50:36–21:50:37 UTC. `android-assets.log`. |
| `./gradlew testDebugUnitTest assembleDebugAndroidTest --no-daemon` (`android/`, before edits) | Exit 0, 21:50:37–21:50:49 UTC. `android-tests.log`; `android-command-results.json`. |
| Same Gradle command after Kotlin changes | Exit 0, 21:55:26–21:55:45 UTC. 15 JVM tests: NWS 3, widget 2, speech 6, transfers 4; no failures/errors/skips. `android-after-fixes.log`, `android-after-fixes.json`, copied `TEST-*.xml`. |
| `git diff --check` | Passed after changes. |

Instrumentation APK: `android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`,
4,137,835 bytes, SHA-256
`2c62c9e32a15e38d254985c74683dee16d18f599f3e21d3aa12ee7f8c6040381`.
It was compiled, not installed or executed. Existing `DeviceSmokeTest` exercises
native menus, activity resume and offline voice when run on ARM64; it does not
measure observation-to-render latency, prolonged Doze, service allowance expiry,
temperature, battery drain or Family Hub hardware. No such result is implied here.
Gradle reports deprecations incompatible with future Gradle 9; current wrapper
8.14.3 completed successfully. This review did not run a signed release build.

## Source, published release, and deployed browser provenance

At 21:51:40 UTC, direct GitHub API reads returned HTTP 200:

- [`commits/main`](https://api.github.com/repos/d4vid87/hookecho/commits/main):
  `fb0221e39af3672ff468775c722df0e117837582`, committed November 11, 2026.
- [`releases/latest`](https://api.github.com/repos/d4vid87/hookecho/releases/latest):
  stable `build-fb0221e39af3672ff468775c722df0e117837582`, published November 13
  at 23:51:08 UTC. API lists Linux `.deb`/AppImage, Android APK/AAB, macOS ZIP,
  Windows ZIP/MSI/EXE and `SHA256SUMS`. This proves publication metadata, not a
  runtime retest or download/hash verification of every release artifact.
- Raw responses and check timestamps are retained in `remote-probes.json`,
  `latest-release.json`, `releases.json`, and `git-main.json` in the artifacts root.

This review's Python `urllib` reads of `https://app.hookecho.io/` and `/build.json`
returned HTTP 403 at 21:51:40 UTC. That method-specific response did not establish
an app outage or an unknown deployment: the parallel browser investigation
successfully retrieved the index and executable assets using Node/curl at
21:53:31 UTC. Its report records URLs, headers, lengths and SHA-256 values:
`../web/worker-report.json` under the durable artifacts directory.

The served `decode-bridge.js` and `decode-worker.js` exactly matched baseline
Git objects (respectively SHA-256
`b6ff15bc16860f11839a227fadeb999c7a66fd8cdc94694e068a273114ff2b27` and
`f25de9750d253061b5e04a838bf5dedbbae6c27a14af52d6b058601cc0ec672e`).
The downloaded glue is `hookecho-ce3b571a.js?boot=2`; wasm is
`hookecho_bg-05fa0666.wasm`. Those asset names alone do not prove the Rust commit.
The wasm revision remains inferred unless matched to build provenance; the JS
content match is directly verified. Baseline/candidate JS decoded 80 fixture
requests with identical output hashes using that same wasm. This supports decode
preservation, not a claim that un-deployed candidate Rust changes are live.

No deployment, external post, subscription, release publication or device setting
change was performed as part of this platform review.
