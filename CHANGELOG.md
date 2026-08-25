# Changelog

Notable changes per release, newest first. Every tagged release's body on GitHub
is this file's matching section, extracted by `.github/workflows/release.yml` —
so **write the section before pushing the tag**, or the release job fails.

The rolling `latest` release tracks `main` and is not listed here.

## 0.11.0 - 2026-08-25

- Basemaps stopped being an afterthought: every pane now draws its own style
  instead of borrowing the active pane's, tiles are fetched at retina
  resolution and overzoomed from the deepest resident ancestor rather than
  going blank, the built-in vector cartography gained buildings, a full road
  ladder and POIs, and the 40-item dropdown became a categorized thumbnail
  grid on both desktop and Android. New sources: keyless hybrid satellite
  (Esri imagery under our own labels), the Esri gray/NatGeo/Oceans canvases,
  an "Auto" entry that follows the theme, and a custom XYZ template on
  desktop and Android.
- The live data path no longer re-merges the whole volume per chunk, which is
  what made a site switch stall and left seams across a partial sweep. Label
  placement, alert rows and the desktop frame loop were profiled and fixed in
  that order; the Android loop cache and site-switch fetches now run in
  parallel.
- Alerts identify a warning by its VTEC event key instead of its message id,
  so a continuation stops re-firing; an outbreak collapses into one rolling
  summary past a threshold; webhooks retry with backoff; Android gained exact
  alarms, a battery-exemption prompt, a watchdog and a delivery-health
  readout; and the spoken script leads with hazard, place and direction
  through an optional local neural voice.
- The rules engine can attach a snapshot, play a per-rule sound, combine
  conditions with one level of AND/OR, and backtest a rule against archived
  volumes.
- Radar accuracy: dealiasing scores region merges by boundary vote and
  anchors on the previous sweep, gate edges stay crisp under smoothing, and
  beam height uses each site's real tower height instead of a flat 20 m.
- Products the app could not show: specific differential phase (KDP, derived
  from the phase field already fetched), single-site composite reflectivity,
  a gate inspector that reads every moment at the gate under the cursor, MRMS
  precipitation rate, nowcast leads out to 120 minutes drawn faintly enough
  to read as a guess, an archive date picker, a wind row on the meteogram,
  trend sparklines in the cells table, NHC advisory and discussion text, TFRs
  and G-AIRMETs, and reflectivity tinted by precipitation type.
- German radar. DWD's open data is the only keyless international volume feed
  there is — seventeen sites, five-minute volumes, reflectivity only. The
  browser build does not offer it; a volume is 4.4 MB and the web build
  fetches through a shared cache.
- A share link can carry the reflectivity threshold (`thr:25`, or `thr:off`),
  which is what an embedded dashboard needs to open at its own threshold
  without changing anyone else's defaults.

## 0.10.0 - 2026-08-14

- The browser build can be embedded in another page's dashboard: `?embed` hides
  all chrome and holds the map at one frame a minute until it is touched, so a
  radar living in someone else's iframe stops costing them a CPU core.
- An embedded map hands its view (site, product, tilt, basemap, camera) to the
  page hosting it once a second, and takes one back through the share link's new
  `bm:<basemap>` and `srv` fields. Browsers partition an iframe's storage, so the
  host is the only place an embedded view can persist — this is what stops
  WeatherDesk's radar resetting on every launch.
- A lost graphics context or a panic after startup now says so instead of leaving
  the last frame on screen forever.
- `cargo run -p wxdata --example sites_json` dumps the radar site registry for
  embedders that want a site picker without the crate.

## 0.9.0 - 2026-08-11

- Three dual-pol signatures the app never computed: three-body scatter spikes
  (near-proof of large hail), ZDR columns (an updraft proxy that deepens before
  a storm intensifies), and the melting-layer bright band. Off by default, and
  none of them makes a sound — the only detection that alarms is still the one
  that means debris is in the air.
- Satellite lightning gains a density field: where the GLM flashes are thickest
  over the last 15 minutes, which is where a lightning jump shows up first.
- Colorblind-safe reflectivity and velocity palettes ship built in, alongside an
  OLED theme, a caption on share cards, a new-scan chime, and beam height on the
  Measure tool.
- Alerting learns restraint and reach: quiet hours with a severity floor,
  rotation near a place you watch, alerting on your own live position, and a
  radar snapshot attached to the push. Pushes held back by quiet hours now come
  back as one summary when the window ends, instead of vanishing.
- The GOES frame follows the radar's clock, so scrubbing back through an event
  takes the satellite with it.
- SPC Day 4-8 outlooks and TAFs on station tooltips.
- A chase breadcrumb log with GPX export, and desktop notifications.
- Android: a home-screen radar widget and a quick-settings tile.
- A small always-on-top mini-loop window on the desktop, and a Debian package
  next to the AppImage so the menu entry, icon and updater are the system's job.
- A crash the app can explain: a panic now leaves a report, and the next start
  offers it back with a Copy button. Nothing in it identifies you.
- Fuzzing the decoders that eat outside bytes found four real bugs — a panic on
  a half-written volume from the live head, two hangs on malformed GRIB2, and a
  parser that died on a mangled hurricane-hunter bulletin. All fixed, all now
  regression-tested, with the fuzz job running nightly.
- First run is four cards and an optional spotlight tour on the real interface,
  down from ten pages of wizard.
- Trackpad pinch zooms and horizontal scroll pans; `hookecho://` links reach an
  already-running instance and carry product and tilt; placefile icon sheets
  cache on disk; a temperature unit the station plots honor.
- Errors you can read, copy and actually see, and a sweep of the unwraps behind
  decoded and user-supplied data.

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
