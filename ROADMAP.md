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

- A model comparison / difference view (GFS against ECMWF, or a run against its
  predecessor). Field layers are app-global today and panes carry only radar
  state, so this is an architecture change before it is a feature.
- A packed-RGBA fallback for GPU wind on devices that cannot render to float
  textures — the CPU path already covers them, so this is only worth doing if
  one shows up that needs the speed.
- A headless-browser smoke test in CI. `cargo check` cannot see a runtime panic,
  which is how the web build shipped one.
- Native-level severe profiles: the composites are solved from ten mandatory
  levels, where SPC blends observations into a full-resolution analysis.

## Later

- Per-pane thresholds and field layers, saved with the workspace.
- Colormaps and stroke widths that respond to the high-contrast theme, not just
  the chrome.
- Fetching the effective-layer parameters above 200 hPa, so an uncapped plains
  parcel has an equilibrium level to measure against.

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
  (`HOOKECHO_CPU_WIND=1`).
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
- Animated HRRR wind, the multi-day forecast, fourteen themes including Light,
  interactive vector basemaps, offline chase packs, future radar, archive back
  to 1991, placefiles with a layer manager, settings sync, and a Home Assistant
  integration.
