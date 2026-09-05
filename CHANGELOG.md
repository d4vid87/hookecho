# Changelog

Notable changes per release, newest first. Every tagged release's body on GitHub
is this file's matching section, extracted by `.github/workflows/release.yml` —
so **write the section before pushing the tag**, or the release job fails.

The rolling `latest` release tracks `main` and is not listed here.

## Unreleased

### Warnings say where, and what to do

- Spoken warnings are on by default and now say something the tone cannot.
  The script leads with the hazard, then where the warning sits against a
  place you saved ("covering Home", "12 miles northeast of Home"), then the
  counties with the state spoken in full, the towns from the bulletin's own
  "Locations impacted include..." list, the motion, and the office's call to
  action. Previously a warning that covered a saved marker was announced as
  "Tornado warning for covers Home" and never named a county at all.
- The tone plays first and the voice follows it, one announcement at a time.
  Every cue used to open its own audio stream on its own thread, so the alert
  tone played over the opening words and a squall line warning four counties
  in one refresh produced four voices at once.
- An emergency now runs immediately after the sentence already being spoken,
  discarding queued ordinary warnings and chase updates. Warnings delivered
  together are spoken from highest to lowest escalation.
- Spoken warnings honour quiet hours the way the tone always has. Escalated
  warnings still go past it. The voice previously ignored quiet hours
  entirely.
- Speech follows the alert volume slider. Piper's output ignored it, so the
  voice was louder than the tone introducing it.
- Settings grows a "Speak a test warning" button that runs the whole chain on
  a made-up warning, and tells you when a configured Piper cannot run — on
  Arch the text-to-speech Piper is `piper-tts-bin` in the AUR, while
  `extra/piper` is a mouse-configuration tool that installs the same binary
  name.
- An audio device unplugged mid-playback no longer strands the thread waiting
  on it forever.

## 0.12.0-beta.2 - 2026-08-30

Third R18 checkpoint, and the biggest one: the app stops being a US radar
viewer. Germany, Europe and Canada get their own radars, composites and
warnings; the whole render and data path got measured and made faster; and the
site, the Lite viewer and the headless server all grew up. Still a prerelease —
0.12.0 waits on RRFS and the remaining store submissions.

### The world, not just the States

- German DWD radars decode natively, velocity and dual-pol moments included,
  and the DWD national composites are a basemap layer.
- The OPERA network via EUMETNET OpenRadarData puts European radars on the
  map, with a generic WMS bridge behind them for composite layers.
- Canada: ECCC GeoMet radar composites and ECCC public alerts.
- MeteoAlarm European warnings, ranked by their own severity scale rather than
  forced through the US one — Red now outranks Yellow, which it did not.
- GIBS Himawari and IMERG basemaps for the rest of the planet.
- Distances read in the units of the region on screen. A radar in Germany
  measures in kilometres without being asked.
- Site pages for the German and European networks, and a radar-data
  attribution surface that names whose data you are looking at.

### Faster

A measured pass end to end, not a guess:

- The decode path stops allocating per sweep, binned sweeps survive the
  playhead, and the wasm build lost weight (fonts fetched, rayon and gif
  degated).
- The renderer reuses radar GPU state, wind particle bind groups and a
  palette-only LUT; field textures get evicted; the tile cache weighs bytes
  instead of counting entries (it was 512 MB while claiming 134 MB).
- Overlay re-tessellation comes off the gesture, one janitor thread replaces
  several, and an idle window slows its own heartbeat.
- Every data feed asks before downloading (conditional requests), the proxy
  caches, and neither cache can grow forever.
- The Lite viewer fetches frames in parallel, decodes them as they arrive, and
  a refresh only fetches what it does not already have.

### Serve and headless

- Every network renders headlessly, with warnings and chrome in the output.
- `/national.png`, mp4 loops and velocity presets; the national mosaic moves.
- A busy renderer serves a stale image rather than queueing, and the image
  cache prunes itself.
- One command stands the image origin back up (`scripts/img-origin`).

### Analysis and data (R18 batch)

- Dealiasing solves the whole sweep at once — a maximum spanning tree over
  region-boundary votes — so a couplet folded twice comes back right.
- Derived products extrapolate down to the surface under a low beam, so VIL
  near the radar stops reading low.
- 2 m dewpoint joins the global layers and the model-diff readout.
- SPC watch boxes, tornado and severe thunderstorm, drawn under the warnings.
- Local cell tracking computed from reflectivity, with 15- and 30-minute
  extrapolation and a closest-approach ETA — for sites and networks with no
  Level 3 storm-cell table.
- Level 3 Digital Base Velocity (N0G) decodes.

### Integrations and quality of life

- MQTT gained a command topic (point the app at a site, a product, or mute it
  from an automation) and optional Home Assistant discovery, so HA creates the
  device itself.
- A lightning-strikes layer fed from your own broker, plus a standalone relay
  in `scripts/strikes-relay` that fills it. The app never connects to a strike
  network itself.
- A curated Piper voice picker instead of one hardcoded voice.
- `?palette=` on snapshot renders.
- Web retries actually back off now, and the quiet-hours queue is written as it
  changes rather than only on exit.

### The web build

- Alert-rule backtests run in the browser.
- Offline chase packs: save a loop into the browser and play it with no signal.
- Zoom controls, full screen, a warnings overlay and neighbouring-radar
  switching in the Lite viewer, whose app bundle is now content-hashed.

### The site

hookecho.io grew from a landing page into the product's front door: docs with
search, a blog with RSS, per-site and per-state radar pages, TDWR and
international pages, historic storm pages, a glossary, an honest comparison
section, a live roadmap, an embed generator, comments, and OG cards built at
build time. Live radar renders on the pages themselves.

### Fixes

- Archive-less sites (TDWR, DWD) no longer claim "(no volumes)" while showing a
  live volume.
- 1-degree sweeps filled every other azimuth bin.
- Crash reports are honest: a caught decode panic is not a crash.
- The live stream survives a network drop instead of restarting its backfill
  every minute.

## 0.12.0-beta.1 - 2026-08-26

Second R18 checkpoint. The phone gets the same chrome the desktop got in
alpha.1, onboarding stops asking questions, and the data lanes land. Still a
prerelease: a few lanes (RRFS, the remaining store submissions) come at 0.12.0.

### The phone

- The persistent bottom sheet and the five-slot toolbar are gone. The phone
  draws the same floating chrome the desktop does — search pill, control
  column, scrubber — with content in Material modal sheets instead of a
  drawer. Predictive back still works.
- Long-press the map to inspect what is under your finger, double-tap and drag
  to zoom with one hand, swipe sideways between panes.
- Haptics: ticks as the scrubber crosses a frame, a buzz when a warning lands
  on you, a bump when a sheet snaps.
- Tablets and landscape phones get a side rail and a docked drawer instead of
  sheets.
- Widgets show how far the nearest storm is. A battery-saver mode stretches
  the poll cadence and throttles repaints. On a chase, the next radar down the
  road is prefetched before you need it, and a chase can be replayed against
  the archive afterwards.

### Getting started

- The setup wizard is gone. First run finds you, picks the nearest radar and
  draws it — about ten seconds, no questions.
- Notification permission is asked for when you turn on something that
  notifies, or when a warning lands near you. Never at install.
- One help hub behind `?`: glossary, hotkeys, the tour and what's new, all
  searchable in one place. Labels across the app read in plain language, with
  the abbreviation kept in parentheses for anyone who wants it.
- Text products — area forecast discussions, tropical advisories — render in a
  real webview with real typography, on both desktop and Android.

### Weather

- GOES loops run over archive dates alongside the radar timeline, and the
  mid-level water vapor bands are selectable.
- MRMS rotation tracks (30/60/120 minutes) are field layers now, and hail
  swaths accumulate locally over a window you choose.
- Snow squalls and banding get their own emphasis in winter.
- An alert rule can trigger on a lightning jump — the rate of change of flash
  density inside a cell.
- NAM nest and NBM join the model list. Synoptic mesonets join the surface
  obs, with your own API key.
- Cells are ranked by a composite severity score, and tapping one opens the 3D
  volume already centered on it.
- Panes remember their own thresholds and field layers. Events can be saved as
  replay bundles — time range, site, camera — and replayed with archived
  radar, warnings, reports and outlooks in sync.

### The web build

- Installable as a PWA, with the app shell cached offline.
- Tiles, volumes and palettes persist between visits.
- File dialogs, GPS and notifications all work in the browser now.
- A share button writes a permalink that carries the site, camera and time.

### Elsewhere

- MQTT publishing, a Home Assistant camera loop endpoint and a live dashboard
  at the `--serve` index.
- An update chip appears when a newer release exists. No self-updating.
- Motion throughout, with `reduce_motion` and an automatic degrade on slow
  frames. Labels, roles and focus order on all of the new chrome.
- The application id is `io.hookecho.HookEcho`, and every screenshot in the
  README, the store listings and this repo was reshot on the new chrome.

## 0.12.0-alpha.1 - 2026-08-25

First R18 checkpoint: the desktop and web chrome, rebuilt. A prerelease — the
mobile chrome is still the old sheet-and-toolbar layout, and the data lanes
land later in the cycle.

- The docked sidebar and the docked timeline bar are gone. The map runs edge
  to edge, and everything that used to eat it now floats over it: a search
  pill top-left, a control column down the right edge beside the color scale,
  and a scrubber pill along the bottom with time ticks, play, a LIVE badge and
  the loop range.
- Tools you browse rather than watch — settings, the event library, alert
  rules — open as pages in one slide-over drawer with a back-stack, one page
  at a time, on every platform. Anything you click *on the map* answers in a
  card anchored next to the click instead of in a window wherever egui last
  left one.
- The window is borderless: its three buttons float with the rest of the
  chrome, the empty strip along the top edge drags it, double-clicking that
  strip maximizes, and any edge or corner resizes. `--decorated` hands the
  frame back to your window manager if it disagrees.
- Design tokens: one metrics module with a Comfortable default and a Compact
  density, the theme list curated to six, a free accent color, and Inter
  bundled on every platform including web.
- The 60-second tour now points at the chrome that exists; the keyboard cheat
  sheet and every hotkey work unchanged; workspaces round-trip and now also
  remember which panel and drawer page were open.
- Streamer (OBS) mode and `?embed=1` strip all of the new chrome, same as
  before.

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
