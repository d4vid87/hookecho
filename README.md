# Hook Echo-WX

[![CI](https://github.com/d4vid87/hookecho/actions/workflows/ci.yml/badge.svg)](https://github.com/d4vid87/hookecho/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/d4vid87/hookecho?sort=semver)](../../releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux%20%7C%20Android-lightgrey)

Advanced NEXRAD weather radar viewer — an open-source homage to
[supercell-wx](https://github.com/dpaulat/supercell-wx), built from scratch in Rust
with `wgpu` + `egui`. Deep per-site Level 2 / Level 3 analysis plus national
situational awareness, forecast environment overlays, and warning intelligence —
on Windows, Linux, and Android.

![Moore, Oklahoma, 20 May 2013 — KTLX 0.5° reflectivity, replayed from the archive](docs/shots/hero.gif)

<sub>**KTLX 0.5° reflectivity — Moore, Oklahoma, 20 May 2013.** Replayed from the
public archive inside the app, one scan every few seconds, with the tornado
warning that was in force at the time.</sub>

## Install

- **Linux**: download `Hook_Echo-WX-x86_64.AppImage` from
  [Releases](../../releases), `chmod +x`, run.
- **Windows**: grab the installer from [Releases](../../releases) —
  `Hook_Echo-WX-setup-x86_64.exe` (setup wizard) or `Hook_Echo-WX-x86_64.msi`
  (MSI, for scripted/enterprise installs). A portable
  `hookecho-windows-x86_64.zip` is there too: unzip, run `hookecho.exe`.
- **Android** (arm64, Android 10+): sideload `Hook_Echo-WX-arm64-v8a.apk` from
  [Releases](../../releases) (`adb install -r …`, or open it on-device with
  "install unknown apps" enabled). The same Rust app as desktop, as a
  `NativeActivity` — see [`android/README.md`](android/README.md).
- **From source**: `cargo run --release` (needs a Rust toolchain; on Linux also
  ALSA/Wayland/GTK dev headers — see `.github/workflows/ci.yml`). Android builds
  via `android/build.sh` (NDK + `cargo-ndk`).

Versioned `v*` releases are the stable channel. Every push to `main` also
refreshes a [`latest`](../../releases/tag/latest) rolling prerelease carrying the
same five artifacts, if you want the newest work without waiting for a tag.

First launch opens a setup wizard: pick your home radar site, a theme (13 built
in), and how warnings should reach you (chime and/or [ntfy.sh](https://ntfy.sh)
push to your phone). After that, three one-time callouts point at the controls
you'll actually use. Re-run the wizard anytime from **⋯ More → Setup wizard**.

## The interface

Hook Echo-WX is a map with a few floating controls over it — no menu bar, no
docked toolbar, nothing between you and the radar:

- **Bottom-left** names the product you're looking at. Click it to switch
  products or tilts; each one says in plain English what it shows.
- **Bottom-center** is the timeline: site, clock, play, scrub, and a LIVE badge
  that snaps back to the newest scan.
- **Right edge**: Layers, radar Site, Alerts, Advanced toolbox, Settings, and
  **⋯ More**.
- **Top-center** searches for a place. `Ctrl+K` searches every product, layer,
  window and tool in the app — one list, described in plain English.

| Pick a product | Find any layer |
|---|---|
| ![The product picker open over the Joplin supercell](docs/shots/products.jpg) | ![The Layers panel open over the Mayfield supercell](docs/shots/layers.jpg) |
| <sub>Joplin, MO — 22 May 2011</sub> | <sub>Mayfield, KY — 11 Dec 2021</sub> |

Radar times read in **the selected radar's local time**, not Zulu — a KEPZ pane
shows MDT while a KTLX pane shows CDT, side by side. Settings → Units puts it
back to UTC if you'd rather work in Zulu.

Every expert control is still there. The Advanced toolbox (`F7`) docks a panel
with the site, the basemap, the product and tilt, the same searchable layer list,
and a Layer Options section that shows a layer's settings only once that layer is
on — so the forecast-hour slider appears when you turn on future radar, and stays
out of the way when you haven't.

Search a place in the top bar and it flies there, with a **Save marker** button
if you want to keep it. Markers are what the warning, lightning and rain-arrival
alerts watch. Tap one on the map to rename or remove it.

## Walkthrough

*Storm screenshots are historic events replayed through the app's own archive
timeline — the same scrubbing you'd do live. Forecast and national layers are
live data, captured as they came in. Everything is on the Mapbox Satellite
Streets basemap; a dozen others ship too, from a dark vector map to GOES
satellite imagery.*

### Watching a storm

All six Level 2 moments on a GPU polar pipeline, with VCP-aware tilt selection,
velocity dealiasing, storm-relative velocity, and GRLevelX `.pal` color tables.
Reflectivity shows you the storm's structure; velocity shows you what it's doing —
the tight red-against-green couplet below is a tornado on the ground.

| Reflectivity | Velocity (dealiased) |
|---|---|
| ![A classic supercell with a hook echo](docs/shots/reflectivity.jpg) | ![A tornadic velocity couplet](docs/shots/velocity.jpg) |
| <sub>KBMX — Tuscaloosa, AL, 27 Apr 2011</sub> | <sub>KTLX — El Reno, OK, 31 May 2013</sub> |

One action opens the same product at four tilts with the cameras linked, so you
can see how a storm leans with height. Cross-sections slice it vertically in any
product — click two points and you get the storm in profile, core and overhang
and all.

| Four tilts at once | Cross-section |
|---|---|
| ![The same storm at four elevation angles](docs/shots/alltilts.jpg) | ![A vertical slice through the storm core](docs/shots/xsection.jpg) |
| <sub>KTLX — Moore, OK, 20 May 2013</sub> | <sub>KTLX — Moore, OK, 20 May 2013</sub> |

Every screenshot in this section is a replay. The timeline reaches back to 2008,
so any archived storm loads the same way the live one does — a supercell from
fifteen years ago, or a hurricane coming ashore:

![Hurricane Ian's eyewall coming ashore over southwest Florida](docs/shots/tropical.jpg)

<sub>**KTBW 0.5° reflectivity — Hurricane Ian, 28 September 2022.**</sub>

### Deciding which storm matters

Every tracked cell in one sortable table — hail size, tops, VIL, rotation flags —
and clicking a row flies you there. Alongside it, an active-alerts panel sorted
worst-first, with clickable NWS bulletins and parsed storm-motion vectors. Scrub
into the archive and the panel fills with the warnings that were actually in
force at that moment.

| Storm attributes | Active alerts |
|---|---|
| ![The storm attributes table listing tracked cells](docs/shots/stormtable.jpg) | ![The alerts panel listing tornado warnings](docs/shots/alerts.jpg) |
| <sub>KBMX — live</sub> | <sub>KBMX — 27 Apr 2011 outbreak</sub> |

### Looking ahead

HRRR future radar rides the timeline's forecast tail, so scrubbing past the
present keeps going into the model. It is labeled the whole time it is on —
model output should never be mistaken for something a radar actually saw.

![HRRR future radar one hour out, with a banner marking it as model output](docs/shots/hrrr.jpg)

<sub>**HRRR future radar, +1 h.** The banner stays up for as long as the layer is
on: forecast, not observed.</sub>

Any point on the map gives you the plain NWS forecast for that spot, and the WPC
surface analysis draws the national weather map — fronts with their proper pips,
highs and lows with pressures.

| Surface analysis | Point forecast |
|---|---|
| ![Fronts, highs and lows across the country](docs/shots/fronts.jpg) | ![A seven-day forecast for a clicked point](docs/shots/forecast.jpg) |
| <sub>WPC coded surface analysis, live</sub> | <sub>Tap anywhere — KBMX, live</sub> |

Also on the forecast tail: **rotation tracks** (HRRR max updraft-helicity swaths
— where storms are forecast to rotate, accumulated across the hours you scrub
to) and near-surface **wildfire smoke**.

### The national picture

Every radar in the country stitched into one mosaic, updated every couple of
minutes — the view for "what's happening everywhere" rather than "what's
happening here" — plus rainfall accumulated over the last hour or day.

| National mosaic | 24-hour rainfall |
|---|---|
| ![MRMS composite reflectivity across the continental US](docs/shots/mrms.jpg) | ![Accumulated rainfall across the Southeast](docs/shots/qpe.jpg) |
| <sub>MRMS composite reflectivity, live</sub> | <sub>MRMS gauge-corrected precipitation, live</sub> |

Alongside them: individual lightning flashes from the GOES satellite's lightning
mapper (fading as they age), cloud-to-ground lightning density, rotation tracks
and azimuthal shear, MESH hail size and 24-hour hail swaths, surface
precipitation type, and FLASH flash-flood recurrence intervals. Every gridded
layer draws its own scale and units.

## Features

### Warnings and alerting

- Each warning's `eventMotionDescription` is parsed into a storm-motion vector —
  warned-storm dot, projected 15/30/45/60-minute path, and ETA readouts to your
  saved locations.
- Escalation tiers (CONSIDERABLE → DESTRUCTIVE/observed tornado → **Tornado
  Emergency / PDS**) drive a pulsing polygon outline, priority sorting with red
  threat chips, a dedicated emergency siren, and `urgent`-priority phone push.
- Warnings are read aloud through the system voice — the desktop speech engine,
  or Android's own TTS on the phone.
- Watched-location monitoring, a lightning proximity alarm (a strike within
  ~15 km of a saved spot), rain-arrival alerts ("rain in about 20 minutes"), and
  live NWS Local Storm Reports, minutes fresh.

### Storm analysis

- SCIT cell tracks, past tracks and arrival-time cones; hail and mesocyclone
  flags; a sortable attributes table for every tracked cell at once.
- Automatic **tornado debris signature** detection (low correlation coefficient
  collocated with high reflectivity) and client-side azimuthal-shear couplet
  detection on the live volume — a rotation flag that doesn't wait for a Level 3
  product.
- NOAA **ProbSevere** per-storm severe/tornado/hail/wind probabilities.
- Cross-sections in any moment, a CAPPI altitude slicer, and a 3D volume view.

### Environment and forecast

- HRRR CAPE (surface-based or mixed-layer parcel) and storm-relative helicity
  (0–1 or 0–3 km) as map overlays.
- Point soundings — Skew-T and hodograph — with derived **SBCAPE / LCL / SRH /
  SCP / STP / EHI** indices from real parcel ascent and Bunkers storm motion.
- The VAD wind-profile hodograph, plus a time-height panel of wind barbs
  accumulated while the app runs, since the radar only publishes the latest.
- HRRR future radar (0–18 h), forecast rotation tracks, near-surface wildfire
  smoke, and 0–45 minute optical-flow extrapolation of the radar you're watching.
- SPC Day 1–3 categorical outlooks plus Day-1 probabilistic tornado/wind/hail
  grids with the significant-severe hatch, and the WFO's **Area Forecast
  Discussion** in-app.

### Data the app decodes itself

- **Gridded Level 3**: Digital VIL, Enhanced Echo Tops and **Hydrometeor
  Classification** (rain / snow / hail / graupel / biological), via a
  from-scratch packet-16 decoder — BZIP2 symbology blocks, ICD float16
  thresholds, 0.25-km and 1-km bins, golden-tested against MetPy.
- **GOES satellite lightning**: individual flashes from the GLM, ~40 seconds
  behind real time. GLM sees in-cloud flashes a ground network never reports.
  The netCDF-4 granules go through a from-scratch minimal HDF5 reader
  (`crates/hdf5lite`), because the reference C library can't ship in the Android
  build.
- **METAR station plots** with US-convention wind barbs, flight-category colors
  and greedy decluttering; **NHC tropical** storms with forecast cones and
  Saffir–Simpson-colored track points; **SIGMET/AIRMET** hazard polygons.

### Time machine

- Archive playback of any date since 2008, with the warning polygons **and the
  local storm reports** that were actually in effect at the scrubbed instant.
- A curated library of historic events, plus your own bookmarks.
- An in-RAM decode buffer with one-touch instant replay (`R`), and
  screenshot / GIF / MP4 loop export.

### Out in the field

- **Chase mode**: live GPS (gpsd on desktop, the system location service on
  Android), a storm-relative HUD with closest approach and escape bearing, and
  offline "chase packs" of pre-downloaded basemap tiles.
- **Streamer/OBS mode**: chrome-free UI (`F8`) and an auto-tour of active
  warnings (`F9`).
- Click anywhere for historical tornado tracks near that point (SPC 1950–2022)
  with an EF-scale histogram.
- Multi-pane layouts, placefiles with icon sheets and a layer manager, a sensor
  dashboard, range rings, 13 themes, and tray-based background alerting.

On Android the same app wears a touch-first skin: a five-slot labeled dock
(Play · Layers · Products · Site · More), slide-up sheets, a navigation drawer
built from the same described action list as the desktop Layers panel, and
native GPS for chase mode.

## Repository layout

- `crates/hookecho` — the app: egui UI and wgpu render pipelines.
- `crates/wxdata` — data plumbing: Level 2 (AWS), MRMS, HRRR (future radar and
  environment fields), NWS alerts with storm motion and escalation, IEM archived
  warnings and live/archived storm reports, SPC outlooks and climatology, METAR,
  NHC tropical, aviation SIGMETs, Area Forecast Discussions, ProbSevere,
  placefiles, spotters, GOES GLM lightning, WPC surface fronts, NWS point
  forecasts, TDS detection, sounding indices, and CAPPI/cross-section/3D
  resampling.
- `crates/nexrad-level3` — from-scratch NEXRAD Level 3 (RPG) product decoder:
  storm-cell packets (15/19/20/23) and digital radial arrays (packet 16 —
  DVL/EET/HHC), golden-tested against MetPy.
- `crates/hdf5lite` — minimal read-only HDF5 reader, just enough for the
  netCDF-4 files NOAA publishes. Exists because the reference reader is a C
  library the Android build can't take; checked against h5py on real granules.
- `vendor/gribberish` — vendored GRIB2 decoder (PNG-packing and MRMS
  local-parameter fixes; grep `hookecho patch:`).
- `scripts/shots` — the screenshot harness that produced everything above.

## Verification

Every data-backed feature has a headless CLI verifier (renders a PNG or prints a
report without opening a window), e.g.:

```sh
cargo run --release -- --headless out.png KTLX --moment VEL --dealias
cargo run --release -- --headless-mrms mosaic.png
cargo run --release -- --headless-alerts                    # motion + escalation lines
cargo run --release -- --headless-archwarn 2013-05-20T20:00:00Z
cargo run --release -- --headless-env sbcape cape.png       # sbcape|mlcape|srh1|srh3
cargo run --release -- --headless-field preciptype ptype.png
cargo run --release -- --headless-l3grid hhc KTLX hca.png   # dvl|eet|hhc
cargo run --release -- --headless-metar KTLX
cargo run --release -- --headless-tropical
cargo run --release -- --headless-cappi KTLX 3 cappi.png
cargo run --release -- --headless-reports 2013-05-20T19:00Z 2013-05-20T21:00Z
cargo run --release -- --headless-afd KTLX
cargo run --release -- --headless-aviation
cargo run --release -- --headless-indices -97.5 35.3
cargo run --release -- --headless-glm                       # GOES satellite lightning
cargo run --release -- --headless-fronts                    # WPC surface analysis
cargo run --release -- --headless-hrrr uh 3 uh.png          # refc|uh|smoke
```

```sh
cargo test    # 184 offline unit tests
```

The screenshots in this README are regenerated by `scripts/shots/shoot.sh`,
which drives the real binary on a nested X display — see
[`scripts/shots/README.md`](scripts/shots/README.md).

## License

MIT.
