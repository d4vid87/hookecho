# Roadmap

What's being worked on, what's queued, and what isn't happening. "Later" is a
real list, not a graveyard — most of it is restocked from the `ponytail:`
comments in the source, each of which names a deliberate simplification and the
upgrade path out of it (`grep -rn "ponytail:" crates/`).

## Now

- **Saved workspaces** — a pane layout (sites, products, tilts, overlays) saved
  and restored from the command palette.
- **Disk caches that survive a restart** — archived radar volumes, zone
  geometry, and the last alert fetch, so a scrub back through an event is a file
  read and a restart mid-outbreak doesn't re-announce what's already on screen.
- **Web build (WASM)** — the viewer in a browser: radar, basemap, alerts and
  forecasts. Feeds that block cross-origin requests degrade rather than break,
  and native-only subsystems (audio, GPS, plugins, camera video) are compiled
  out.
- **HTTP server mode (`--serve`)** and a **Docker image**, so a machine on the
  shelf can answer questions about the weather without a display attached.
- **Home Assistant integration** — a custom component polling the server, with
  per-location sensors, an alert binary sensor and a radar camera.

## Next

- Live polling on the web where the platform allows it. Settings, audio and
  spoken warnings already work there.
- GPU-side wind particle advection. The current layer rebuilds a CPU mesh each
  frame because that also works on WebGL2 — the note in `wind_draw.rs` marks the
  ceiling and says explicitly that the upgrade is *not* a compute shader.
- A radar snapshot as a desktop widget / conky-style output, reusing the same
  off-screen render the server's `/snapshot.png` uses.
- Placefile `Image:` support: it is parsed and skipped today, waiting on a real
  file in the wild to use as a fixture.

## Later

- Accessibility: a screen-reader tree (accesskit), a high-contrast theme, and
  keyboard-only navigation documented as a first-class path.
- Flatpak, Snap, Homebrew and winget manifests, so installing is a package
  manager line rather than a download.
- True lunar ephemeris (Meeus) in place of the mean synodic phase, which is
  currently good to about half a day.
- Effective-layer severe parameters on the clicked sounding. The gridded layers
  already solve a column per cell; the sounding panel still shows the
  fixed-layer forms, so the two disagree in method.
- Android settings import through the Storage Access Framework.

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

Already shipped, and sometimes mistaken for gaps: gridded effective-layer STP,
ESRH and EBWD, real per-device inset queries on Android, animated HRRR wind particles,
the multi-day forecast, thirteen themes including Light, interactive vector
basemaps, offline chase packs, future radar, archive back to 1991, placefiles
with a layer manager, and settings sync.
