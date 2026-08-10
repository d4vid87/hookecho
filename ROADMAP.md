# Roadmap

What's being worked on, what's queued, and what isn't happening. "Later" is a
real list, not a graveyard — most of it is restocked from the `ponytail:`
comments in the source, each of which names a deliberate simplification and the
upgrade path out of it (`grep -rn "ponytail:" crates/`).

## Now

- **macOS**, as an experiment. CI builds and smoke-tests an app bundle, but
  nobody has run it on real hardware, and it is ad-hoc signed.
- **Store submissions.** The Flatpak, Snap, Homebrew and winget manifests are in
  the repo and build; none of them is published, which is an account and a
  review queue away rather than a code change.
- **Placefile `Image:` support**: parsed and skipped today, waiting on a real
  file in the wild to use as a fixture.

## Next

- **GOES satellite imagery** — ABI Level 2 CMIP from the `noaa-goes19`/`-18`
  buckets, CONUS to start, three bands (visible, clean IR, mid-level water
  vapor), animated on the timeline the radar already uses. The GIBS basemap
  layer shows the latest image; this is the archive and the loop.
- Blending surface observations into the effective-layer analysis, which is the
  remaining difference from SPC's mesoanalysis now that the vertical resolution
  is there.
- Per-pane thresholds and field layers, saved with the workspace. The difference
  layer would rather be two panes than one subtraction, but field layers are
  app-global, so that waits on this.

## Later

Restocked from `grep -rn "ponytail:" crates/` — 88 of them at the moment, each
naming its own upgrade path.

- A valid-time alignment for the difference layer: it labels the two cycles
  today rather than interpolating either onto the other's instant.
- Colormaps and stroke widths that respond to the high-contrast theme, not just
  the chrome (`theme.rs`).
- Web persistence: caches live in memory in the browser, so a reload refetches
  everything. IndexedDB or OPFS is the upgrade (`paths.rs`).
- A tablet layout for Android — the phone chrome is what a tablet gets today
  (`ui/m3.rs`).
- Background alerts that test real polygon intersection instead of sampling
  points around a marker's radius (`Nws.kt`).
- Placefile `Image:`, which needs a georeferenced textured quad — and a real
  file in the wild to pin the corner syntax.
- Snap positions for the mobile sheets; they dismiss on a drag today but have no
  half-open state.
- Temperature units for the gridded contour labels, which still do their own
  K → °F while the station plots follow the Units setting.

## Not planned

- **iOS.** No Apple hardware, no signing story, no honest way to test it.
- **Machine-learning nowcasting.** There is already a radar-advection nowcast
  layer; a trained model is a research project, not a feature.
- **Blitzortung lightning.** Their terms forbid application use. See
  [docs/DATA.md](docs/DATA.md).
- **A velocity mosaic.** Stitching velocity from separate radars is physically
  meaningless — each radar measures motion along its own beam.
- **A hosted service, accounts, or telemetry.** The app talks to public feeds
  from your machine; sync goes through your own Google Drive folder, and
  position sharing through your own relay if you set one.

Already shipped, and sometimes mistaken for gaps:

- **The whole app in a browser**, live chunk streaming included, so a web tab
  updates sweep by sweep during a scan.
- **GPU wind particles** — advected in a fragment shader, ping-ponging two
  textures, with the CPU mesh still there as the fallback
  (`HOOKECHO_CPU_WIND=1`). Positions are packed into RGBA already, so a device
  without float render targets is not the blocker it is sometimes taken for.
- **Saved workspaces**, three of them shipped, remembering panes, overlays and
  field layers.
- **Disk caches** for volumes, zones, soundings and snapshots, all capped and
  swept, with a Storage tab that shows and clears them.
- **Effective-layer severe parameters** on both the grid and the clicked
  sounding, and CSV export of the sounding and its indices.
- **Accessibility**: a screen-reader tree (accesskit), a high-contrast theme,
  and the keyboard defaults written down.
- **Android**: real per-device inset queries, and file import through the
  Storage Access Framework.
- **A radar snapshot as a file** (`--snapshot … --every`) for conky and
  wallpaper scripts, with size and zoom on `/snapshot.png` too.
- **True lunar ephemeris** (Meeus), in place of the mean synodic month.
- **A model difference layer** — GFS against ECMWF, HRRR against RAP — drawing
  nothing where the two agree, with a hover readout and a diverging legend.
- **Effective-layer parameters on the full pressure ladder**, up to 100 hPa, so
  the depth-dependent ones exist on the days they describe.
- **A headless-browser smoke test in CI**, which is the check `cargo check`
  structurally cannot perform.
- Animated HRRR wind, the multi-day forecast, fourteen themes including Light,
  interactive vector basemaps, offline chase packs, future radar, archive back
  to 1991, placefiles with a layer manager, settings sync, and a Home Assistant
  integration.
