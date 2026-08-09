# Changelog

Notable changes per release, newest first. Every tagged release's body on GitHub
is this file's matching section, extracted by `.github/workflows/release.yml` —
so **write the section before pushing the tag**, or the release job fails.

The rolling `latest` release tracks `main` and is not listed here.

## 0.8.0 - 2026-08-09

- A model difference layer: GFS−ECMWF and HRRR−RAP, resampled onto a common grid
  and drawing nothing where the two models agree.
- Effective-layer severe parameters on the full pressure ladder, up to 100 hPa,
  so the depth-dependent ones exist on the days they describe.
- Soundings fetch the 150 and 100 hPa levels, so a parcel has an equilibrium
  level instead of running off the top of the profile.
- CI opens the built web bundle in a real headless browser — the check
  `cargo check --target wasm32` structurally cannot perform.
- Saved workspaces appear in the command palette without "Show all"; CSV export
  writes a file, not just the clipboard.
- Android: the dialog shim can reach the activity handle again.

## 0.7.0 - 2026-08-09

- GPU wind particles, advected in a fragment shader with the CPU mesh kept as a
  fallback (`HOOKECHO_CPU_WIND=1`).
- Packaging: Flatpak, Snap, Homebrew and winget manifests, AppStream metadata,
  a full icon theme, and an experimental macOS app bundle built in CI.
- Android imports files through the Storage Access Framework.
- Workspaces remember field layers, and three starter layouts ship with the app.
- `--snapshot` writes a radar PNG for desktop widgets; `/snapshot.png` takes
  size and zoom.
- A Storage tab that shows and clears every disk cache.
- Live chunk streaming in the browser, so a web tab updates sweep by sweep.
- Accessibility: the widget tree is published to screen readers, plus a
  high-contrast palette.
- True lunar phase (Meeus) instead of the mean synodic month.
- Soundings: effective-layer parameters on the clicked profile, and CSV export
  of the profile and its indices.

## 0.6.0 - 2026-08-09

- Saved workspaces: pane layouts stored and restored.
- Archived Level 2 volumes and zone geometry kept on disk across restarts.
- GFS and ECMWF global model fields.
- Forecast-hour soundings with a full parcel, lapse rates and effective-inflow
  parameters.
- User-drawn watch zones that fire when a warning polygon enters them.
- 3D: reflectivity threshold, axis slicing, and off-thread volume building.
- Chase pack street tiles beside raster imagery; wildfire feeds fully paged.
- Container image published to ghcr.io; `hookecho://` URL scheme registered.
- Performance: parallel site switches, faster live-stream start, fewer per-frame
  allocations.

## 0.5.0 - 2026-07-25

- Time-height wind profile (VWP) panel.
- WPC surface analysis fronts overlay.
- Rain-arrival ETA for saved locations and your chase position.
- Tap anywhere for the NWS point forecast.
- New warnings read aloud.
- Cross-sections slice velocity and correlation coefficient, not just
  reflectivity; compare-4-tilts pane preset.
- First-run tutorial rebuilt around the app as it is now.

## 0.4.0 - 2026-07-20

- Android port: the same codebase as a NativeActivity APK (arm64-v8a).
- Per-pane product picker for multi-pane layouts.

## 0.3.0 - 2026-07-19

- Hydrometeor classification, live and archived local storm reports, an AFD
  viewer, and hail sizing.

## 0.2.0 - 2026-07-19

- Warning intelligence, archived warnings, probabilistic outlooks, and
  CAPE/SRH environment overlays.
- Live-loop playback, selectable alert sounds, marker icons, more basemaps.

## 0.1.0 - 2026-07-18

- First release: Level 2 / Level 3 NEXRAD viewing on a `wgpu` + `egui` map,
  with archive replay and NWS alerts.
