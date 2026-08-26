# Hook Echo-WX

[![CI](https://github.com/d4vid87/hookecho/actions/workflows/ci.yml/badge.svg)](https://github.com/d4vid87/hookecho/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/d4vid87/hookecho?sort=semver)](../../releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20Linux%20%7C%20Android%20%7C%20Web%20%7C%20macOS%20(experimental)-lightgrey)

Advanced NEXRAD weather radar viewer — an open-source homage to
[supercell-wx](https://github.com/dpaulat/supercell-wx), built from scratch in Rust
with `wgpu` + `egui`. Deep per-site Level 2 / Level 3 analysis plus national
situational awareness, forecast environment overlays, and warning intelligence —
on Windows, Linux, and Android.

### **[Try it in your browser →](https://hookecho.pages.dev/)**

The whole app as wasm, on live data, with nothing to install.

![Moore, Oklahoma, 20 May 2013 — KTLX 0.5° reflectivity, replayed from the archive](docs/shots/hero.gif)

<sub>**KTLX 0.5° reflectivity — Moore, Oklahoma, 20 May 2013.** Replayed from the
public archive inside the app, one scan every few seconds, with the tornado
warning that was in force at the time.</sub>

![HRRR 10 m wind drawn as drifting particles across the CONUS](docs/shots/wind.gif)

<sub>**Animated wind.** HRRR 10 m wind as drifting particles coloured by speed —
built from NOAA's free GRIB grids, so it needs no API key.</sub>

## Install

- **Linux**: download `Hook_Echo-WX-x86_64.AppImage` from
  [Releases](../../releases), `chmod +x`, run. Debian/Ubuntu users can install
  `hookecho_<version>_amd64.deb` from the same place
  (`sudo apt install ./hookecho_*.deb`) to get the menu entry and icon.
- **Windows**: grab the installer from [Releases](../../releases) —
  `Hook_Echo-WX-setup-x86_64.exe` (setup wizard) or `Hook_Echo-WX-x86_64.msi`
  (MSI, for scripted/enterprise installs). A portable
  `hookecho-windows-x86_64.zip` is there too: unzip, run `hookecho.exe`.
- **Android** (arm64, Android 10+): sideload `Hook_Echo-WX-arm64-v8a.apk` from
  [Releases](../../releases) (`adb install -r …`, or open it on-device with
  "install unknown apps" enabled). The same Rust app as desktop,
  with a Material 3 phone UI — see [`android/README.md`](android/README.md).
- **macOS** (experimental): `Hook_Echo-WX-macos.zip` from
  [Releases](../../releases). The build is made and smoke-tested in CI but has
  never been run on real Apple hardware, and it is ad-hoc signed, so Gatekeeper
  will ask before it opens. Homebrew users can build it instead:
  `brew install --HEAD d4vid87/hookecho/hookecho` (formula in
  [`packaging/homebrew`](packaging/homebrew)). **macOS testers wanted** — there
  is no Apple hardware behind this project, so an
  [issue](../../issues) saying what did or did not work is the only way this
  build stops being experimental.
- **Package managers**: manifests for
  [Flatpak](packaging/flatpak), [Snap](snap/snapcraft.yaml),
  [Homebrew](packaging/homebrew), [winget](packaging/winget) and the
  [AUR](packaging/aur) live in the repo. None are published to their stores
  yet — building from these files works today; a `flatpak install hookecho` does not.
- **From source**: `cargo run --release` (needs a Rust toolchain; on Linux also
  ALSA/Wayland/GTK dev headers — see `.github/workflows/ci.yml`). Android builds
  via `android/build.sh` (NDK + `cargo-ndk`).

Versioned `v*` releases are the stable channel. Every push to `main` also
refreshes a [`latest`](../../releases/tag/latest) rolling prerelease carrying the
same artifacts, if you want the newest work without waiting for a tag.

First launch asks one question: which radar do you open to. Let it use your
location and it picks the nearest one and gets out of the way; otherwise search
the list. Everything else — map, theme, accent color, how warnings reach you —
has a default and lives in **Settings**. The card also offers a 60-second tour:
four stops spotlighted on the live map, two of which you finish by doing the
thing rather than reading about it. Neither is forced, and both re-run from the
panel's **App** section, `Ctrl+K`, or **Settings → General**.

## The interface

Hook Echo-WX is a full-bleed map with its controls floating over it — no menu
bar, no docked columns eating the weather, and no title bar either: the window
is borderless and its three buttons float with the rest of the chrome. Drag it
by the empty strip along the top edge, double-click that strip to maximize, and
resize from any edge or corner. If your window manager disagrees with any of
that, `--decorated` hands the frame back to it.

- **The panel** holds everything: the site, the current product and its
  tilt, the expert knobs for that product, then every layer, window and tool —
  one icon-led row each, described in plain English, filed under collapsible
  categories that carry their own count, with the layer options, map settings and
  app commands under them. Search it with `Ctrl+K`; Enter runs the top match.
  Drag a row by its icon to pin it above the rest of its category. It opens from
  the search pill in the top-left corner or the layers button on the right edge,
  and closes to nothing — the map runs edge to edge underneath it.
- Its **Alerts tab** lists every alert covering your view, worst first, badged
  with the count. Click a row to fly there and read the bulletin.
- **The scrubber** floats along the bottom: site, clock, play, a LIVE badge that
  snaps back to the newest scan, and a time track marked with one tick per volume
  and labelled by the hour, in the radar's own timezone. Drag it or click anywhere
  on it. The shaded stretch at the live end is the loop the play button cycles.
  Right-click the badge for the archive day, loop and playback speed.
- **The right edge** carries the control column — layers, background map, and an
  alert bell badged with the number of alerts in view — and the color scale for the product each pane is
  showing — a thin bar floating over the map, spanning only the values the color
  table actually paints.

That's the whole of it. Searching for a place is the same box — type a name and
take the **Fly to** row at the bottom of the results.

| Search everything | One panel for everything |
|---|---|
| ![Searching for "hail" over the Joplin supercell](docs/shots/products.jpg) | ![The panel over the Mayfield supercell](docs/shots/layers.jpg) |
| <sub>Joplin, MO — 22 May 2011</sub> | <sub>Mayfield, KY — 11 Dec 2021</sub> |

Radar times read in **the selected radar's local time**, not Zulu — a KEPZ pane
shows MDT while a KTLX pane shows CDT, side by side. Settings → Units puts it
back to UTC if you'd rather work in Zulu.

Every expert control is still there, in the panel's **Layer options** section,
which shows a layer's settings only once that layer is on — so the forecast-hour
slider appears when you turn on future radar, and stays out of the way when you
haven't.

Search a place in the panel and it flies there, with a **Save marker** button
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

Split the window into four panes and give each one its own product, cameras
linked — reflectivity, velocity, correlation coefficient and differential
reflectivity on the same storm at the same second. One action does the same
trick with four tilts instead, so you can see how a storm leans with height.
Cross-sections slice it vertically in any product — click two points and you get
the storm in profile, core and overhang and all.

| Four products at once | Cross-section |
|---|---|
| ![The same storm in reflectivity, velocity, correlation coefficient and differential reflectivity](docs/shots/alltilts.jpg) | ![A vertical slice through the storm core](docs/shots/xsection.jpg) |
| <sub>KTLX — Moore, OK, 20 May 2013</sub> | <sub>KTLX — Moore, OK, 20 May 2013</sub> |

Every screenshot in this section is a replay. The timeline reaches back to June
1991, so any archived storm loads the same way the live one does — a supercell
from fifteen years ago, or a hurricane coming ashore:

![Hurricane Ian's eyewall coming ashore over southwest Florida](docs/shots/tropical.jpg)

<sub>**KTBW 0.5° reflectivity — Hurricane Ian, 28 September 2022.**</sub>

And because the archive is there, you can ask how the warnings did. The
verification lab scores an office's warnings for a day against the storm reports
that came in — POD, FAR, CSI, lead times, and the individual rows behind them,
including the reports nobody warned for (the red dots on the map). The
arbitration is the Iowa Environmental Mesonet's, so the numbers agree with the
published ones instead of being a private opinion.

![Warning verification for Norman, OK on 20 May 2013](docs/shots/verify.jpg)

<sub>**KTLX — 20 May 2013.** OUN's warnings scored against that day's reports.</sub>

### Deciding which storm matters

Every tracked cell in one sortable table — hail size, tops, VIL, rotation flags —
and clicking a row flies you there. Alongside it, an active-alerts panel sorted
worst-first, with clickable NWS bulletins and parsed storm-motion vectors. Scrub
into the archive and the panel fills with the warnings that were actually in
force at that moment.

| Storm attributes | Active alerts |
|---|---|
| ![The storm attributes table listing tracked cells](docs/shots/stormtable.jpg) | ![The alerts panel listing tornado warnings](docs/shots/alerts.jpg) |
| <sub>KTBW — live</sub> | <sub>KBMX — 27 Apr 2011 outbreak</sub> |

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

There is also a **multi-radar mosaic** built the other way round: instead of a
national 1 km product, every radar covering your view contributes its own base
reflectivity at full resolution, composited nearest-radar-wins so neighbouring
sites join without a seam. It shows which radars it used and how old the oldest
scan in it is, because radars scan on their own schedules and a composite is
never one instant.

![Base reflectivity from six radars composited across the southern Plains](docs/shots/mosaic.jpg)

<sub>Six radars, one picture — live N0B composite</sub>

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
- **Home marker with a watch radius**: mark one saved location as home and set
  how close a warning has to come (20 miles by default) — you're alerted when
  the polygon's *edge* reaches that ring, not only when it swallows your house.
  Every marker carries its own radius; home draws its ring on the map.
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
- **Copy CSV or save it to a file** from the cell table, the cross-section, the
  verification report, the sounding — indices and profile both — and the tornado
  climatology, so the numbers can leave the app and be checked somewhere else.

### Environment and forecast

- CAPE (surface-based or mixed-layer parcel) and storm-relative helicity (0–1 or
  0–3 km) as map overlays, from either the **HRRR forecast (3 km)** or the **RAP
  f00 analysis (13 km)** — the observed mesoanalysis, assimilated from obs rather
  than projected forward. Same switch drives the contour fields and the
  STP/SCP/EHI composites.
- Point soundings — Skew-T and hodograph — with derived **SBCAPE / LCL / SRH /
  SCP / STP / EHI** indices from real parcel ascent and Bunkers storm motion,
  and the **observed radiosonde ascent** from the nearest station drawn dashed
  beside the model profile (University of Wyoming archive, so an event replay
  gets that morning's real balloon, back to 1973).
- The VAD wind-profile hodograph, plus a time-height panel of wind barbs
  accumulated while the app runs, since the radar only publishes the latest.
- HRRR future radar (0–18 h), forecast rotation tracks, near-surface wildfire
  smoke, and 0–45 minute optical-flow extrapolation of the radar you're watching.
- A **model difference layer**: GFS minus ECMWF (MSLP, 500 hPa height, 2 m
  temperature, 10 m wind) and HRRR minus RAP (surface CAPE, storm-relative
  helicity), on one map. Where the two models agree it draws nothing, so what
  you see is the disagreement — and the layer names both valid times, because
  the two feeds rarely share a cycle.
- **Effective-layer** bulk shear, SRH and STP as gridded fields, solved per
  column on the HRRR's 25 hPa pressure ladder up to 100 hPa — deep enough that a
  buoyant parcel has an equilibrium level to be measured against, which is what
  the depth-dependent parameters need to exist at all.
- SPC Day 1–3 categorical outlooks plus Day-1 probabilistic tornado/wind/hail
  grids with the significant-severe hatch, and the WFO's **Area Forecast
  Discussion** in-app.

### Products computed from the volume itself

![VIL density over the Mayfield supercell, computed from an archived 2021 volume](docs/shots/derived.jpg)

<sub>VIL density over the Mayfield, KY supercell — computed here from the 11 Dec
2021 volume, four years after it was recorded.</sub>

- **VIL, VIL density and echo tops**, integrated here from the stacked tilts
  rather than read from a Level 3 grid — so they work in **any archive replay**,
  at the volume's own resolution, with an echo-top threshold you can move.
- **Maximum expected hail size and probability of severe hail** (Witt et al.
  1998), with the melting-level and −20 °C heights the algorithm needs sourced
  automatically from HRRR instead of typed in by hand. Live only: those heights
  come from the current analysis, and applying today's freezing level to a storm
  from 2021 would be an answer with no meaning.

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
  and greedy decluttering — extended offshore and over the Great Lakes by the
  **NDBC buoy network**, where the airport network simply has no stations. Buoys
  also print their **wave height and dominant period**, which is the reason to
  look at one.
- **Wildfires**: active perimeters and incident points from the interagency
  **WFIGS** feed, with acres and containment on tap.
- **Air quality**: every **AirNow** monitor in view as an EPA-category dot with
  its AQI (free key in Settings).
- **TDWR terminal radars**: the FAA's airport radars, synthesized into volumes
  from their Level 3 tilt products, so all 44 of them behave like any other site.
- **NHC tropical** storms with forecast cones and Saffir–Simpson-colored track
  points, the **forecast wind field** at 34/50/64 kt, **potential storm surge**,
  and **hurricane-hunter flight tracks** with the measured SFMR surface wind.
- **SIGMET/AIRMET** hazard polygons and **PIREPs** — what pilots actually flew
  through, rather than what was forecast.
- **Winter**: HRRR forecast snowfall, the **NOHRSC observed snowfall analysis**
  (6/24/48/72 h), the **Winter Storm Severity Index** — how disruptive, not just
  how much — and **mPING** crowd reports of what is actually falling.
- **WPC Excessive Rainfall Outlook**, the flood half of a severe-weather day.

### Time machine

- Archive playback of any date since June 1991, with the warning polygons **and
  the local storm reports** that were actually in effect at the scrubbed instant.
  Volumes from before the dual-polarization upgrade simply don't offer ZDR/CC.
- A curated library of historic events, plus your own bookmarks.
- An in-RAM decode buffer with one-touch instant replay (`R`), and
  screenshot / GIF / MP4 loop export.

### Workspaces

A pane layout is a dozen clicks to rebuild — two sites, their products and
tilts, the overlays and field layers that go with them — and it is the same
dozen clicks every time. Save the arrangement from the command palette and
restore it in one.

Three are there on first run: **Chase** (reflectivity beside storm-relative
velocity, cameras locked), **National overview** (the MRMS mosaic under the
warnings), and **Analysis** (one storm at four heights). They adopt whichever
radar is on screen, and deleting them is final.

### What is on your disk

Everything the app downloads is cached and every cache is trimmed to a cap at
startup. Settings → Storage shows each one's size against that cap, with a
button to clear it and a button to open it. Nothing there is irreplaceable —
clearing a cache costs the next fetch and nothing else.

### Across your machines

- **Sign in with Google** (Settings → Sync) and your settings, saved locations,
  placefiles, palettes and API keys follow you to every machine. The data lives
  in your own Drive's hidden per-app folder — no Hook Echo account, no server,
  and a scope that cannot see the rest of your Drive. Screen scale and device
  name stay local. One-time OAuth client setup: [docs/sync.md](docs/sync.md).

### Out in the field

- **Chase mode**: live GPS (gpsd on desktop, the system location service on
  Android) drawn as a blue dot on the radar, a storm-relative HUD with closest
  approach and escape bearing, and offline "chase packs" of pre-downloaded
  basemap tiles.
- **Position sharing**: opt in and every Hook Echo on the same network sees
  everyone else's dot, no account and no configuration (UDP broadcast on
  :41777). For devices that aren't on one network — the phone chasing on
  cellular, the desktop at home — set a relay URL in the same panel: any HTTP
  endpoint you host that takes `POST` of one position and returns `GET` of the
  list. There is no default relay; the endpoint sees your live position.
- **Streamer/OBS mode**: chrome-free UI (`F8`) and an auto-tour of active
  warnings (`F9`).
- Click anywhere for historical tornado tracks near that point (SPC 1950–2022)
  with an EF-scale histogram, plus how often that spot has actually been warned —
  warning counts by type, first year on record, busiest year and worst day.
- **NWS damage surveys**: the EF-rated damage points and surveyed track a survey
  crew filed after the storm, with photos, on the day your timeline is showing.
  Ground truth to lay over the hook you were watching.
- Listen to a **NOAA Weather Radio** relay while you watch (bring your county's
  stream URL — NOAA broadcasts on VHF and streams nothing itself).
- Draw on the map freehand to circle the storm you're talking about, and look
  through webcams to see what the sky actually looks like. The FAA's ~2,600
  airport cameras need no key but stop at the US border; a free **Windy** key in
  Settings adds their global network, which returns the 50 most popular cameras
  in view rather than every one of them.
- **Animated wind**: HRRR 10 m wind drawn as drifting particles, coloured by
  speed — the Windy look, built from NOAA's own free GRIB grids rather than
  licensed, so it needs no key and works offline of any paid API. CONUS only,
  because HRRR is. Model output, not observation, and the layer says so.
- **Minute-by-minute rain** for the spot you tapped: the next hour advected off
  the current scan, above the hourly forecast — when it starts, when it stops.
- **Share the view**: the palette copies a `hookecho://` link carrying the site,
  place, zoom and (when you're scrubbed back) the time. Opens the app right
  there — on Android, straight from the link.
- **Android home-screen widget**: what's warned at your saved locations, tap to
  open the map on it.
- **Per-layer opacity** in the Layer Manager, for field layers as well as
  placefiles, and the scrub bar now shades its forecast tail so a model hour
  never reads as radar.
- **Open this view in Windy** from the command palette when you want their
  models next to ours — same place, same zoom, matching overlay.
- **Live station cards**: click a surface station and get a floating,
  draggable card — a live highway camera on top, then the station's local clock
  and how stale its reading is, temperature with humidity and dewpoint, a wind
  dial with the trailing-gust ladder (10 s through 24 h), and the electric field.
  Open as many as you want, one per station.
  Airport METARs need no key; a **WeatherFlow Tempest** token or a **Weather
  Underground** key adds personal weather stations. Camera video is MJPEG decoded
  in-app, or HLS if you have `ffmpeg` installed — the card says so plainly when
  you don't, and everything else on it keeps working.
  Cameras come from the FAA and from **Caltrans**, whose twelve districts publish
  ~3,300 cameras keyless and stream video from most of them. No other state DOT
  publishes an open camera API, so that is the coverage, not a national map.
  The electric field is NOAA's **PPEF** model: the equatorial *ionospheric* field
  in mV/m, predicted from solar wind. That is space weather, not the storm over
  your head — for the kV/m a chaser means, point the card at a ground field
  mill's JSON in Settings and it charts that instead.
  On Android the cards work the same, minus live video: the camera shows its
  newest still instead, refreshed on the poll clock (a phone can't spawn ffmpeg,
  and a video decoder per open card is not what its battery is for).
- Multi-pane layouts, placefiles with icon sheets and a layer manager, a sensor
  dashboard, range rings, 7 themes with a custom accent color, and tray-based background alerting.

### On your phone

![Hook Echo-WX on Android replaying the Moore, Oklahoma tornado of 20 May 2013](docs/shots/android/hero.gif)

<sub>**The same Rust app, rebuilt around the phone.** KTLX 0.5° reflectivity,
20 May 2013, playing out of the archive at half speed with the tornado warnings
of the day counting up on the alert badge.</sub>

Android is not the desktop squeezed onto a smaller screen — it is a Material 3
layout over the same renderer, the same data paths and the same action registry:

| | |
|---|---|
| ![The map with the sheet at peek](docs/shots/android/map.jpg) | ![The sheet at half height, showing the scrubber and product chips](docs/shots/android/sheet.jpg) |
| **Map first.** The radar owns the screen; one chip names the site and VCP. | **One sheet, three snaps.** Drag it up for the scrubber, speed, products and tilts; drag it to full for the archive. |
| ![The layers and tools sheet](docs/shots/android/layers.jpg) | ![Active alerts listed in a modal sheet](docs/shots/android/alerts.jpg) |
| **Everything else is a sheet.** The same described, categorized action registry the desktop panel uses. | **Alerts in view**, tap to fly there and read the full text. |

- A docked five-action toolbar along the sheet's bottom edge — no second bar
  stacked under it.
- Every tool window (soundings, cross-sections, settings, the site picker) is a
  full-screen surface, because that is what the compact width class is for.
- Real `WindowInsets`, including the keyboard, so a focused field rises above it.
- The real system IME through GameActivity: autocorrect, suggestions, and every
  language the phone has, instead of NativeActivity's raw ASCII key events.
- Predictive back: back dismisses the innermost surface, and with nothing open
  Android draws its own home-screen preview as you drag.
- In landscape the sheet becomes a full-height side rail.
- Native GPS for chase mode, and opt-in background alerting: a foreground
  service watches your saved locations and notifies you with the app closed,
  tiered by watch / warning / emergency, tapping through to the storm.

### Without opening the app

`hookecho --status` prints what's happening at your saved locations — current
conditions from the nearest station and any active alert whose polygon comes
within that location's watch radius:

```
$ hookecho --status
Home: 74°F 62%rh SW 12kt (KOKC)
  ‼ Tornado Warning until 14:30
Cabin: 71°F 70%rh calm (KSWO)
```

Pass `LAT,LON` to report on somewhere you haven't saved, `--line` for a single
line about home (status bars), or `--json` for the whole report:

```sh
hookecho --status 35.3,-97.5
hookecho --status --line     # 74°F 62%rh SW 12kt · ‼ Tornado Warning until 14:30
hookecho --status --json
```

Home Assistant, without leaving the machine hookecho runs on:

```yaml
command_line:
  - sensor:
      name: Home weather alerts
      command: "hookecho --status --json"
      value_template: "{{ (value_json | selectattr('home') | first).alerts | count }}"
      scan_interval: 300
```

polybar:

```ini
[module/hookecho]
type = custom/script
exec = hookecho --status --line
interval = 300
```

tmux:

```
set -g status-right '#(hookecho --status --line) | %H:%M'
```

### As a service

`hookecho --serve` answers the same questions over HTTP, for the machine with no
display attached:

```sh
hookecho --serve             # http://127.0.0.1:8080
hookecho --serve 9000 --bind 0.0.0.0
```

| Endpoint | What it returns |
| --- | --- |
| `/status.json` | conditions and nearby alerts for every saved location |
| `/alerts.json` | alerts only |
| `/obs.json` | conditions only |
| `/cells.json?site=KTLX` | every storm cell the radar's algorithms track — hail size, tops, VIL, TVS, forecast track |
| `/health.json` | version, uptime and how stale the answers are, for a container health check |
| `/snapshot.png?site=KTLX` | a radar render — `&product=VEL`, `&basemap=none`, `&size=512` (256–2048), `&zoom=6.5`, `&tilt=1` |
| `/loop.gif?site=KTLX` | the last half hour animating — same render knobs, plus `&frames=6` (2–12) and `&fps=2` |
| `/loop.mp4?site=KTLX` | the same loop as H.264, if ffmpeg is installed |

JSON answers are cached for a minute, snapshots and loops for five, so polling it
every 30 seconds costs the upstream services nothing extra. A loop reuses the
frames it already rendered, so a poll five minutes later renders one new frame
rather than six — and two steps landing on the same volume become one frame, not
a stutter.

The server binds loopback and answers anyone who asks. Putting it on a network
(`--bind 0.0.0.0`, or a container port that isn't `127.0.0.1:`) means publishing
where you live, so set a token first — `serve_token` in settings.json, or
`--serve-token` on the command line, which wins:

```sh
curl -H 'Authorization: Bearer hunter2' http://boxname:8080/status.json
curl 'http://boxname:8080/snapshot.png?site=KTLX&token=hunter2'
```

Every route is behind it, `/metrics` and the CORS proxy included. The
`?token=` form is there for dashboards that can only fetch a URL and have no
place to put a header.

For a desktop widget rather than a server, `--snapshot` writes the same render
straight to a file:

```sh
hookecho --snapshot ~/.cache/radar.png KTLX --size 480 --zoom 7 --every 120
```

It renders, renames the finished file into place (so a widget polling on its own
clock never catches a half-written PNG), and repeats. Point conky, `feh
--reload`, or a wallpaper script at the file.

There's a container for it, if that's easier than a systemd unit:

```sh
docker run -d -p 127.0.0.1:8080:8080 \
  -v ~/.config/hookecho:/root/.config/hookecho \
  -v hookecho-cache:/root/.cache/hookecho \
  ghcr.io/d4vid87/hookecho:latest
```

`:latest` is rebuilt from `main` on every push; released versions are tagged
(`:0.7.0`). `docker build -t hookecho .` still works if you'd rather build it.

Mount your settings so it reports on your saved locations. The image is ~660 MB
because it carries Mesa's lavapipe software Vulkan driver — that's what renders
`/snapshot.png` with no GPU and no display attached.

**It binds loopback unless you tell it otherwise.** This endpoint says where you
live and what is warned there; `--bind 0.0.0.0` exposes that to everything on
your network, and there is no authentication in front of it. Put it behind a
reverse proxy if it needs to leave the machine.

### In Home Assistant

The `command_line` sensor above is the no-install path. For something richer
there's a custom component in [`custom_components/hookecho`](custom_components/hookecho):
point it at a machine running `--serve` and you get, per saved location, a
device carrying temperature, dewpoint, humidity, wind, gust and pressure
sensors, an **alert count** sensor and an **alert active** binary sensor
(attributes: event, expiry, distance, escalation tier), a **nearest storm**
sensor giving the distance to the closest cell the radar is tracking
(attributes: cell id, bearing, max dBZ, hail size, TVS), plus one **radar
camera** entity showing the live snapshot.

A `generic` camera pointed at `/loop.gif` gives a dashboard card that animates:

```yaml
camera:
  - platform: generic
    name: Radar loop
    still_image_url: http://boxname:8080/snapshot.png?site=KTLX&token=hunter2
    stream_source: http://boxname:8080/loop.mp4?site=KTLX&token=hunter2
```

The nearest-storm sensor is the one to automate on: it answers "is a storm
coming" minutes before anybody issues a warning, and goes unavailable — never
zero — when nothing is being tracked.

Install via HACS as a custom repository, or copy the `custom_components/hookecho`
directory into your Home Assistant `config/custom_components/` and restart. Then
add the integration and give it the host and port. Polling is once a minute,
which is the server's own cache window.

Remember the server has to be reachable from Home Assistant — that means
`--bind 0.0.0.0` (or the container), and a network you're willing to expose your
saved locations on. Set `serve_token` if that network is not one you control.

### Over MQTT

If the rest of the house already talks to a broker, hookecho can talk to it too.
Set the broker under **Settings → Alerts → MQTT** (host, port, optional TLS and
credentials, and a topic prefix), restart, and three topics appear:

| Topic | Retained | What it carries |
|---|---|---|
| `<prefix>/status` | yes | the whole `--status` report as JSON, every five minutes |
| `<prefix>/nearest` | yes | the closest tracked cell to your home location, or `null` |
| `<prefix>/alerts` | no | one message per alert as it is delivered — title, body, urgency, time |

Alerts are not retained on purpose: a warning is an event, and a subscriber that
connects tomorrow must not be handed yesterday's tornado. `null` on `nearest` is
the honest answer for "nothing is being tracked" — a retained topic that simply
stopped updating looks the same as a dead app.

Publishing only; nothing subscribes, so a compromised broker cannot drive the
app. The password is a secret like the rest — it stays in your settings file.
`hookecho --serve` publishes too, which is usually the process you want doing it:
it is the one that is always up.

### In a browser

A hosted build of `main` runs at **<https://hookecho.pages.dev/>** if you only
want to look at it.

The app also builds for wasm and runs on a canvas — the same `HookEchoApp`, not
a cut-down viewer. Build it and serve it:

```sh
cargo install wasm-bindgen-cli   # once
./scripts/web/build.sh
cargo run --release -- --serve 8080 --web-root web
```

Then open <http://localhost:8080>. Radar, the vector basemap, alerts and point
forecasts all work: the Level 2 buckets and `api.weather.gov` allow cross-origin
requests. **Live chunk streaming works too**, so a browser tab updates sweep by
sweep during a scan rather than waiting out the whole volume. Settings persist to
`localStorage`, and alert sounds and spoken warnings play through the browser's
own audio and `speechSynthesis`. "Use my location" and chase follow-me work too,
off the browser's own Geolocation, and "Post alerts to the desktop" posts real
browser notifications. Both permissions are asked for at the moment you turn the
thing on and never before. It is also **installable**: a manifest and a
service worker put it on the home screen or in its own window, and precache the
app shell so a launch with no signal still opens the map instead of a browser
error. Basemap tiles are cached too, so a second visit draws the map without
re-downloading geography that has not moved. Radar, satellite and everything else
through the proxy stay on the network, because a cached radar frame is a wrong
radar frame.

This is a **core viewer**, though, not parity.
Settings bundles, color tables and CSV/GPX exports import and export through the
browser's own file picker and downloads — an imported `.pal` has no path to live
at, so its content rides in the settings and survives a reload like everything
else there. There is still no filesystem, though, so volume caches are
memory-only and marker icons and custom alert sounds (both of which are files the
app reopens later) stay off; there is no camera or plugin support; and any feed whose host refuses
cross-origin requests simply doesn't load. Anything that needs those is on the
desktop and Android builds.

#### Hosting it yourself

The bundle is static files plus one CORS proxy — the feeds NOAA serves without
`Access-Control-Allow-Origin` need an origin of your own in front of them, or the
page loads and draws nothing. The proxy's rules (exact-match allowlist, GET only,
64 MB cap, narrowed content types) live once in `web/_worker.js/proxy-core.js`;
each host is a few lines of wiring around it.

The Worker also answers `/geo.json` with the visitor's rough position from
Cloudflare's own geo-IP, which is how the demo opens on the radar nearest you
instead of the configured default site. No third-party lookup and no browser
permission prompt; a `#goto=` deep link still wins. A host without it just opens
on the default.

The address bar is a permalink: the `#goto=` fragment follows the active pane as
you pan, zoom and scrub, so copying the URL sends someone to the frame you were
looking at, archive time included. The share button in the control column does
the same thing through the platform's share sheet where there is one.

**Cloudflare Pages** — `web/_worker.js/`, deployed by
`.github/workflows/demo.yml`. That's what `hookecho.pages.dev` is, and it is the
only host this repo deploys to.

Any other host works the same way if it can run one small function on
`/proxy/*`. A purely static host cannot: without the proxy the app opens, and
then sits empty.

### Extending it

Placefiles work as they do everywhere else — URLs, icon sheets, a layer manager with per-file
opacity and paint order. Beyond that, anything that can print a placefile can be a plugin: the app
runs your command on a cadence, hands it the current site, view box, product and **the instant on
screen** (so it works during an archive replay too), and draws what comes back. No SDK, no build
step, no language requirement — the shipped examples are a Python script and twenty lines of
shell. See [docs/plugins.md](docs/plugins.md).

### Deep links

`hookecho://goto/…` opens the app at a place and, optionally, a time. The
desktop registers the scheme on install (Linux `.desktop`, Windows registry,
macOS bundle); on Android a notification tap uses the same path.

```
hookecho://goto/KTLX                                   # a radar site
hookecho://goto/KTLX,-97.28,35.33,9                    # site, lon, lat, zoom
hookecho://goto/,-97.28,35.33,9                        # no site: just the view
hookecho://goto/KTLX,-97.28,35.33,9,2013-05-20T20:00:00Z   # and an instant
hookecho://goto/KTLX,-97.28,35.33,9,VEL,1              # and a product and tilt
hookecho://goto/KTLX,-97.28,35.33,9,thr:25             # and a threshold, in dBZ
```

The timestamp is RFC 3339 and puts the timeline where it was, so a link can
point at a moment in the archive rather than at "now". The trailing fields are
recognized by shape, not position: a timestamp, a product code (`VEL`, `ZDR`,
`CC`, …), a tilt index, a basemap (`bm:dark`), a threshold (`thr:25`, or
`thr:off` to turn one off) and `srv` can appear in any order, and any you leave
out keep whatever the person opening the link already had.

The threshold is in the product's own unit — dBZ for reflectivity, m/s for
velocity — so a link means the same thing whichever Units the recipient uses. It
applies to the product the link names, which is why `VEL,thr:15` thresholds
velocity rather than whatever happened to be showing.

The browser build takes the same thing in the URL fragment, which never leaves
the client — no server sees where you are looking:

```
https://hookecho.pages.dev/#goto=KTLX,-97.28,35.33,9,VEL
```

"Copy link to this view" in the command palette writes the right form for
whichever build you are running.

## Keyboard

Every action in the command palette is bindable, and the defaults are listed in
Settings → Hotkeys. `?` opens the cheat sheet over whatever is on screen. The
ones worth knowing before you look:

| Key | What it does |
| --- | --- |
| `←` `→` | step one frame |
| `R` | instant replay from the in-RAM buffer |
| `Page Up` / `Page Down` | tilt up / down |
| `1`–`6` | products: reflectivity, velocity, spectrum width, ZDR, PHI, CC |
| `Ctrl+K` | command palette |
| `L` | the panel |
| `A` | the alert panel |
| `Z` | cycle the basemap |
| `M` | mute |
| `F3` | change site |
| `?` | cheat sheet |

The whole interface is reachable from the keyboard, and the widget tree is
published to screen readers (AT-SPI on Linux, UI Automation on Windows). There
is a **High contrast** theme in Settings → General for low vision and for
reading the screen in direct sun, an **OLED black** theme for night use, and
colorblind-safe reflectivity and velocity color tables in Settings → Palettes
(a viridis ramp whose brightness rises with dBZ, and a blue/orange diverging
velocity table — red/green diverging tables do not survive protan or deutan
vision).

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

Also worth reading: [docs/GUIDE.md](docs/GUIDE.md) — the task-by-task user
guide, "how do I actually do this"; [docs/DATA.md](docs/DATA.md) — every feed the
app decodes, with its cadence, latency and whether it needs a key;
[ROADMAP.md](ROADMAP.md) — what's next and what isn't planned;
[ARCHITECTURE.md](ARCHITECTURE.md) — how the code is shaped;
[CONTRIBUTING.md](CONTRIBUTING.md) — how the
workspace fits together and what a patch has to clear;
[CHANGELOG.md](CHANGELOG.md) — what changed per release;
[docs/promotion.md](docs/promotion.md) — how a release is cut and announced.

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
cargo run --release -- --headless-rules KTLX                # would your alert rules fire?
```

```sh
cargo test    # 303 offline unit tests
```

The screenshots in this README are regenerated by `scripts/shots/shoot.sh`,
which drives the real binary on a nested X display — see
[`scripts/shots/README.md`](scripts/shots/README.md).

## License

MIT — see [LICENSE](LICENSE).
