# Hook Echo-WX

Advanced NEXRAD weather radar viewer — an open-source homage to
[supercell-wx](https://github.com/dpaulat/supercell-wx), built from scratch in Rust
with `wgpu` + `egui`. Deep per-site Level 2 / Level 3 analysis plus national
situational awareness (MRMS), on Windows and Linux.

## Install

- **Linux**: download `Hook_Echo-WX-x86_64.AppImage` from
  [Releases](../../releases), `chmod +x`, run.
- **Windows**: grab the installer from [Releases](../../releases) —
  `Hook_Echo-WX-setup-x86_64.exe` (setup wizard) or `Hook_Echo-WX-x86_64.msi`
  (MSI, for scripted/enterprise installs). A portable
  `hookecho-windows-x86_64.zip` is there too: unzip, run `hookecho.exe`.
- **From source**: `cargo run --release` (needs a Rust toolchain; on Linux also
  ALSA/Wayland/GTK dev headers — see `.github/workflows/ci.yml`).

First launch opens a three-step setup wizard: pick your home radar site, a theme
(13 built in), and how warnings should reach you (chime and/or
[ntfy.sh](https://ntfy.sh) push to your phone). Re-run it anytime from
**Help → Setup wizard**.

## Walkthrough

**Level 2 base data** — all six moments on a GPU polar pipeline over vector/raster
basemaps, with VCP-aware tilt selection, velocity dealiasing, storm-relative
velocity, and GRLevelX `.pal` color tables (editable in-app):

| Reflectivity | Velocity (dealiased) | Correlation coefficient |
|---|---|---|
| ![REF](docs/shots/reflectivity.jpg) | ![VEL](docs/shots/velocity.jpg) | ![CC](docs/shots/cc.jpg) |

**National view (MRMS)** — CONUS composite reflectivity, cloud-to-ground lightning
density, rotation tracks / azimuthal shear, MESH hail size, and storm-total QPE
flood layers:

| Composite | Lightning | 1-h QPE |
|---|---|---|
| ![MRMS](docs/shots/mrms.jpg) | ![Lightning](docs/shots/lightning.jpg) | ![QPE](docs/shots/qpe.jpg) |

**Forecast & analysis** — HRRR future radar with an observed→forecast timeline
scrub, 3D volume raymarching, vertical cross-sections, VAD hodograph, soundings:

| HRRR future radar | 3D volume | Cross-section |
|---|---|---|
| ![HRRR](docs/shots/hrrr.jpg) | ![3D](docs/shots/storm3d.jpg) | ![Cross-section](docs/shots/xsection.jpg) |

**Warnings & alerts** — clickable NWS bulletins with polygon overlays, an
active-alerts panel, audible cues, and ntfy push the moment a warning covers one
of your saved locations:

![Alerts](docs/shots/alerts.jpg)

## Feature highlights

- **Storm analysis**: SCIT cell tracks + forecast cones, hail/mesocyclone flags,
  auto **tornado debris signature (TDS)** detection (low CC + high Z → chime/push),
  NOAA **ProbSevere** per-storm severe/tor/hail/wind probabilities.
- **Nowcast**: 0–45 min optical-flow radar extrapolation from storm motion,
  alongside hourly HRRR model future radar.
- **Safety**: My-Locations warning monitoring, lightning proximity alarm
  (strike within ~15 km of a saved spot → chime/push), storm reports and
  Spotter Network overlay (contact info stripped at parse).
- **Climatology**: click anywhere → historical tornado tracks near that point
  (SPC 1950–2022 database) with EF-scale histogram.
- **Radar DVR**: deep in-RAM decode buffer with one-touch instant replay (`R`).
- **Streamer/OBS mode**: chrome-free UI (`F8`) + auto-tour of active warnings (`F9`).
- **Time machine**: archive playback of any date since 2008, curated historic
  events library, bookmarks.
- Multi-pane layouts, placefiles, sensor dashboard, cross-sections/CAPPI,
  13 themes, tray + background alerting, screenshot/loop export.

## Workspace

- `crates/nexrad-level3` — from-scratch NEXRAD Level 3 (RPG) product decoder.
- `crates/wxdata` — data plumbing: Level 2 (AWS), MRMS, HRRR, NWS alerts, SPC,
  ProbSevere, placefiles, spotters, climatology, TDS detection.
- `crates/hookecho` — the app: egui UI + wgpu render pipelines.
- `vendor/gribberish` — vendored GRIB2 decoder (PNG-packing fix for MRMS).

## Verification

Every data-backed feature has a headless CLI verifier (renders a PNG or prints a
report without opening a window), e.g.:

```sh
cargo run --release -- --headless out.png KTLX --moment VEL --dealias
cargo run --release -- --headless-mrms mosaic.png
cargo run --release -- --headless-tds KTLX
cargo run --release -- --headless-climatology -97.5 35.3
```

```sh
cargo test    # 82 offline unit tests
```

License: MIT.
