# Architecture

How Hook Echo-WX is put together, for someone about to change it.
[CONTRIBUTING.md](CONTRIBUTING.md) covers the workflow — the gate, the tests,
the commit style. This file covers the shape of the code.

The one-sentence version: **a frame of UI mutates state, one sync step turns that
state into GPU uploads and background fetches, and results come back over
channels and repaint.** Everything below is a detail of that loop.

## The crates

| Crate | What it is |
| --- | --- |
| `crates/hookecho` | The app. egui UI, wgpu pipelines, settings, desktop binary, Android `cdylib`, wasm entry. |
| `crates/wxdata` | Fetch and decode. One module per feed (see [docs/DATA.md](docs/DATA.md)). Draws nothing, holds no UI state. |
| `crates/nexrad-level3` | From-scratch NEXRAD Level 3 (RPG) product decoder. |
| `crates/hdf5lite` | Minimal read-only HDF5, enough for NOAA's netCDF-4 granules. |
| `crates/hookecho-ffi` | C ABI over the decoders, for embedding elsewhere. |
| `vendor/gribberish` | Vendored GRIB2 decoder with local fixes; grep `hookecho patch:`. |

The `wxdata`/`hookecho` split is the load-bearing boundary. A new feed is a
module in `wxdata` that returns plain data, plus a layer in `hookecho` that
draws it. If a decoder needs to know about the camera, the split is being
violated.

## Entry points

All four platforms build the same `HookEchoApp`; only the launch differs
(`crates/hookecho/src/lib.rs`):

- **Desktop** — `main.rs` dispatches any `--headless-*` verifier or `--serve`, then `run_desktop()`.
- **Android** — `android_main()` takes the `AndroidApp` handle, points `paths` at the app-private dir, stashes the handle in `platform` for insets.
- **Web** — `start(canvas_id)` from `web/index.html`. No filesystem, no live chunk stream, only feeds that allow cross-origin.
- **Headless** — `headless.rs` drives the real pipelines to a PNG with no window. These are the smoke tests.

## The frame loop

`app.rs` is the shell (it is large; the mobile chrome is split into
`app/mobile/`, per-pane state into `view.rs`):

1. **UI runs.** Buttons, hotkeys, the command palette and the mobile sheet all mutate the *active* `MapView` and nothing else. One action, one path — this is why a hotkey and a button never drift apart.
2. **Sync step.** Once per frame, the app diffs that state against what the GPU and the fetchers already have: uploads a changed sweep, requests a missing tilt, kicks off a feed refresh.
3. **Results arrive.** Every fetch returns over an `std::sync::mpsc` channel and asks egui to repaint. `rt.rs` is the whole platform difference: a tokio runtime natively, `spawn_local` on the web.

`platform.rs` gates background work on foreground state — eframe stops calling
`update()` when Android tears down the surface, so workers check that stamp
instead of burning battery behind a dark screen.

## State

- `view.rs` — one map pane: camera, product, tilt, loaded `Volume`, LRU of recent scans.
- `workspace.rs` — a saved arrangement of panes, restored in one command.
- `settings.rs` — JSON at the platform config dir, `#[serde(default)]` so old files stay loadable. Writes are coalesced into a one-second dirty-diff save, because the Android alert service reads the same file while the app runs.
- `paths.rs` — every persistent path in the app. The desktop config/data/cache split and Android's single private dir differ here and nowhere else.

## Rendering

`render/` draws through `egui_wgpu::CallbackTrait`, inside egui's own render
pass:

- `render/mercator.rs` — projection and camera. World coordinates are normalized mercator in `[0,1]`, matching the XYZ tile scheme.
- `render/mod.rs` — the slippy-map raster tile layer and the polar radar layer. Radar is drawn in polar space on the GPU; it is never resampled to a grid first.
- `render/field_ramps.rs` — one table mapping every gridded field's values to colors *and* to its legend. Single source, so a scale cannot drift from its own key.
- `render3d.rs` + `shaders/raymarch.wgsl` — maximum-intensity raymarch of the volume. Cost is the step count in `dims.w`; `ui/volume3d_window.rs` exposes it as the quality preset.
- `colormap.rs` — color tables, including GRLevelX `.pal` import/export.
- `tiles.rs`, `vector_tiles.rs` — basemap fetch, LRU cache, visible-tile computation.
- `wind_gpu.rs`, `stationlayer.rs`, `fronts_draw.rs`, … — the individual overlay layers.

## Extension points, in order of how much they cost you

1. **A `.pal` color table** — no code.
2. **A placefile** — no code; the GRLevelX text format, rendered natively.
3. **A plugin** (`plugins.rs`) — any command that prints a placefile on stdout. Language independent, OS-isolated, no new dependencies. It runs with your privileges: the caps are hygiene, not a sandbox.
4. **A feed** — a `wxdata` module plus a layer, plus a `--headless-*` verifier for it.

## Where the sharp edges are

- **`app.rs` is a god file.** Splitting it is welcome, but state that is genuinely per-pane belongs in `view.rs`, not a new module beside `app.rs`.
- **The Android field-name contract.** `AlertService.kt` parses `settings.json` by hand in another language. The test `kotlin_alert_service_field_names_survive` is the only compiler that sees both sides.
- **`cfg` fences are load-bearing.** Web has no filesystem or process spawning; Android has neither `serve` nor ffmpeg. Check `lib.rs` — the module list documents which is which.
- **`// ponytail:` comments** are the ledger of deliberate ceilings. `grep -rn "ponytail:" crates/` is where [ROADMAP.md](ROADMAP.md) gets restocked.
- **No telemetry, no accounts, no server of ours.** The app talks to NOAA and the NWS directly. A change that adds a hosted dependency is a change to the product, not just the code.
