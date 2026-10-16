# HookEcho Radar Workspace

Radar Zeta is the default desktop layout for fresh HookEcho profiles and a separately launched HookEcho profile, built from observed reference app interactions and HookEcho's existing data/rendering systems. It is an independent partial implementation. It is **not a completed one-to-one clone**, and the standalone package does not add unavailable provider features.

## Use it inside HookEcho

Fresh website, AppImage, and desktop installs open with Radar Zeta controls. Existing saved layout choices are respected. Open **Settings → Appearance → Desktop workspace → HookEcho layout** to change the desktop layout. Radar Zeta adds a source chooser, station/projection card, grouped drawer, Display Settings and compact playback controls. Compact phone layouts remain HookEcho's existing responsive UI.

The new **Presentation** vector map uses independently chosen colors over the existing public vector-map pipeline. Display Settings exposes vector state/county visibility, colors and widths; city/road visibility; radar station/range-ring controls; projection with 0–60° Mercator tilt and bearing; radar opacity; time format; and pause while panning. Raster maps cannot expose editable individual vector layers. **Show TDWR sites** is saved independently (enabled for older settings by default) and hides only TDWR map markers and their hit targets. Selected TDWR data and the complete site picker remain available; the same control is under Settings → Radar & map → Station markers.

Product/elevation and History controls share the ordinary UI implementation, including favorites, custom radar fields, cut playback, archive dates and browser offline packs. Workspace buttons invoke existing tools; no placeholder service buttons claim unavailable behavior. Source choices select one source product; Layer manager remains available to combine layers. Forecasts are labeled as guidance rather than observed radar.

## Run the separate desktop app

From a checkout with the normal Rust/native build prerequisites:

```sh
cargo run --release -p hookecho --bin hookecho -- --radar-workspace
```

To package the separate Linux x86_64 AppImage:

```sh
packaging/build-appimage.sh --radar-workspace
```

The packaging script produces `dist/HookEcho-Radar-Workspace-x86_64.AppImage` and a desktop entry named **HookEcho Radar Workspace**. Its launcher passes `--radar-workspace` before settings or background services initialize. The script builds the normal binary and fetches its packaging/voice dependencies; a source command here is not proof that the final AppImage has been built or tested.

The standalone app uses a separate profile identifier, `hookecho-radar-workspace`. Ordinary HookEcho continues to use `hookecho`. Under default Linux XDG locations:

| Storage | Ordinary HookEcho | Radar Workspace |
| --- | --- | --- |
| Settings/config | `~/.config/hookecho` | `~/.config/hookecho-radar-workspace` |
| Application data | `~/.local/share/hookecho` | `~/.local/share/hookecho-radar-workspace` |
| Cache | `~/.cache/hookecho` | `~/.cache/hookecho-radar-workspace` |

XDG overrides and other operating systems use their normal `ProjectDirs` locations. A fresh standalone profile defaults to Radar workspace, Presentation map and a separate MQTT prefix. Existing saved preferences win, including a user's choice to disable the layout or change the map/prefix. Switching the layout inside ordinary HookEcho does not change its storage profile. Android's existing app-private directory layout remains unchanged.

## What remains unfinished

The machine-readable [61-group parity ledger](reference-parity.json) records implementation scope, source references, acceptance criteria and remaining gaps for every observed group. **Implemented-related** means a related capability exists; it does not mean that its provider, algorithm, UI or behavior matches.

- **Map rendering:** Mercator tilt/bearing now share a CPU/GPU ground-plane projection, with persisted workspace orientation. 3D building extrusion, a radar-value surface and exact Mapbox style/label equivalence remain absent. The orthographic globe and separate radar-volume window are different capabilities.
- **Radar and playback:** the browser live-loop ceiling adapts from four to twelve frames based on measured scan payload; a two-minute TDFW run decoded ten consecutive frames. Native 7–250 choices are requested limits. The decoded cache can hold 250 small scans plus head/in-flight slots, subject to a 512 MiB encoded-payload budget. A repeatable native stress run retained 250 independent copies of one live TDFW scan at about 514 MB process RSS; it did not fetch or render 250 distinct historical frames, so actual long-loop playback remains unmeasured. Public incremental radar chunks are not the RapidSweep backend. Complete international/private-network coverage and provider-derived product algorithms are absent or partial.
- **Provider integrations:** CyclonePORT accounts/devices/telemetry/control/video and spotter-linked live video are not implemented equivalents. Generic sensors, cameras and video playback do not supply those associations. Commercial lightning/outage coverage, provider-specific forecasts and some model products require independent authorized services/data; existing public alternatives have different coverage.
- **Accounts/backend:** vendor authentication, subscription, billing, entitlement, cloud synchronization and proprietary ingestion/processing services were not recovered or recreated.
- **Workflow coverage:** complete satellite/MRMS/model catalogs, all overlay settings, hail-history controls, notification behavior, export codecs and mobile behavior remain partially mapped or unverified. A menu entry alone is not completion.
- **Performance:** hardware browser trials now provide measured callback/task baselines, but different scenes, engines and weather data prevent a cross-app speed ranking or equal-smoothness claim. Matched-data and presented-frame measurements are still needed.

The next completion gates are: expand source/settings/transport acceptance beyond the tested paths; exercise the remaining live feeds and stale/error recovery; resolve each missing product/provider integration; finish the remaining 3D surface/building modes; then compare screenshots, numerical values and interaction/performance traces under matched conditions. Every accepted exception must remain explicit in the ledger.

The native retention check is reproducible with `cargo run -p hookecho --example tdwr_cache_profile -- TDFW 250`. It fetches one live volume, clones its decoded geometry into independent allocations, and reports encoded payload, RSS and cache access timing. The recorded run is `outputs/ro-parity/native-loop-profile/results.json` in the research workspace. It does not time 250 historical downloads or render frames.

## Accepted public-data alternatives

The user selected public-data alternatives for device telemetry, chaser video and international feeds. The [provider capability matrix](public-data-alternatives.md) records existing NWS/METAR stations, FAA/Caltrans/optional Windy cameras, German DWD radar and Canadian ECCC composites, with current access checks, representative live renders and explicit remaining UI validation. Fixed cameras do not establish chaser-video association. Australian embedded radar remains absent; an official viewer link is available in the Weather map picker. This accepted scope change does not mark the original feature groups complete.

## REA evidence and limits

The current capture set records the running desktop viewer at a 920×1028 viewport on October 14, 2026. Screenshots prove the visible state only; the hosted viewer can change independently of the AppImage version.

| State | Retained REA evidence ID |
| --- | --- |
| Main map | `ev_bf9d7760e0d04ac53c2594d5532a17946fd6eb8c36e92f554dee85df5b8bc7f0` |
| Sidebar | `ev_42f005059240a45abaad37405a4cc2bfd3bab25488a9c3e3809ee01ffdfee7ee` |
| Display settings | `ev_f4fc167477152e72bba97f1fc648599ce7ee50df9ea6e8c65cbb7dbb38c05b9e` |
| Spotter settings | `ev_445a6809cb1df1e744164211c79536c7f01140c2edab5c1a8146c7c616e827be` |

The JSON ledger includes the other capture IDs, hashes and times. Research manifests are referenced relative to the original research workspace (`outputs/ro-parity/live-evidence-index.json`, `feature-inventory.json`, `hookecho-coverage.json`, and `research findings (external to this repository)`); they are not implied to ship with this repository. No vendor implementation, authenticated URLs, credentials or keys are included here.

## Validation status

The shared transport changes passed native and wasm library compilation. Two settings-profile tests prove fresh standalone defaults, unchanged ordinary defaults and preservation of saved preferences; two path tests prove separate native roots and unchanged Android override layout without mutating global profile state.

The full library suites passed **635 HookEcho tests and 431 wxdata tests** (24 and 38 ignored). Focused follow-up checks passed all fifteen workspace tests, including radar/legend visibility persistence, and all eight MQTT tests. Final native Clippy passed with warnings denied. The five affected WGSL shaders validated and translated to GLSL ES300/WebGL2; explicit hardware GPU checks passed for pitched radar, imagery, field and stroke alignment plus wind advection.

Browser candidate checks verified source search/selection, a scrollable Display Settings modal at 800×600, layer visibility/opacity, separate source animation speeds, tilt/bearing persistence and Escape returning to the map without hiding its controls. These used local radar fixtures, not a live-data equivalence comparison. A packaged standalone candidate launched with live KEAX radar and created only its separate application profile under isolated test XDG directories.

The final optimized web build passed its explicitly revised 4,500,000-byte budget at **4,488,523 gzip bytes** (12,112,454 raw); Lite passed its unchanged 80,000-byte budget at **74,040 gzip bytes**. Final browser checks verified search-caret arrow keys, numeric tilt/bearing input, MRMS selection with radar/legend hidden across reload, saved TDWR filtering, and a History speed change restored at 7 fps. The older local test server stopped during a later reload; after restarting it, state restored. The test-injection localStorage error on the browser's network-error document is retained in the validation evidence.

REA-wrapped runtime trials completed at 1280×720 on the same Radeon RX 580: three idle, three wheel and three pan trials per application. HookEcho’s median trial p95 `requestAnimationFrame` interval was **16.8 ms** for all three conditions, with zero observed long tasks during the input windows. The reference app’s corresponding p95 values were **16.8 / 83.3 / 50.0 ms**. These describe different live/fixture weather scenes, engines and GL contexts. Actual wheel-delivery windows also differed (about 8.13 seconds versus 3.90 seconds), and HookEcho camera response was not directly sampled. They are **not a speed ranking, presented FPS, input latency or performance-parity result**. The earlier SwiftShader attempt produced insufficient samples and timed out; it was retained separately and excluded. Evidence IDs: reference app `ev_c44344280cedd8a5616f140cae54c6df58e51b601dc0a414b3f3a6adff943338`; hardware HookEcho `ev_415308d773bfafe35ae519c6ecab212286c4be0fa39087664ce95268b10dde38`. Raw records and limitations are in `outputs/ro-parity/performance/`, relative to the research workspace. The measured web artifact predates the subsequent hidden-radar/imagery status wording, imagery/alerts placement and narrow-desktop install-prompt corrections.

A final-source raw WASM check passed at **1120×900 and 800×600**: alerts, imagery controls and transport stayed separate, the duplicate timestamp was absent and the install suggestion stayed clear. Live ECCC rain navigation changed 17:24 → 17:18 → 17:24 UTC through Previous/Latest; one capabilities request and 42 PNG requests succeeded with no page/request errors. This validates those layouts and rain-frame navigation, not continuous animation or error recovery. A later optimized browser check verified ECCC rain Play advancing through six-minute frames and Pause holding the selected frame; source-tagged imagery selection prevents an old DWD manual time from leaving ECCC blank on switch. Snow and error recovery remain unverified. A separate optimized browser check verified GOES-East Band 13 infrared Play/Pause over north Texas, including ten-minute frame progression and wraparound; other bands, sectors and archives remain unverified.

The final optimized browser artifact also booted and displayed live 17:30 UTC ECCC imagery at both sizes. Its loaded JS/WASM/snippet files all returned HTTP 200 and matched the built file hashes. The optional edge-provided `/geo.json` route returned HTTP 404 on the local server; the selected KBUF view and imagery still loaded. Evidence: `outputs/ro-parity/web-delivery-smoke/`. This short smoke covered startup, imagery and control placement; complete basemap loading was not an acceptance gate.

The final standalone **0.12.0-beta.2 AppImage** built successfully (**101,935,608 bytes**). Its SHA256 is `3d3fe1d2ed1d09847159a2ec1ee334f21fb906cf0e0e62afa4352d2201e523a5`. One isolated launch verified the HookEcho Radar Workspace window/class, live KEAX reflectivity over labeled geographic maps, VCP 212 and live transport updates on AMD RX 580/Vulkan. Settings, recovery and cache appeared only under the standalone profile in the test XDG roots. The owned process closed normally with exit code 0, and its artifact hash stayed unchanged. Evidence: `outputs/ro-parity/native-final/launch-evidence.json`. An unrelated desktop crash notification partly obscures the screenshot; it is not a native application crash. This test does not establish all interactive paths or audit writes outside the isolated test roots.

Delivery source/browser/native hashes are retained in `outputs/ro-parity/delivery-{source,web,native}-manifest.json`, relative to the research workspace. The final optimized WASM SHA256 is `2ae3bd460a53a0d2379826407deb21c09d2bd55675a0d65bd996374b09f1bdb8`. These bounded checks do not establish complete feature parity.
