# U.S. analyst roadmap implementation evidence

Status is conservative: **existing** has repository evidence, **partial** has useful foundations
but misses roadmap acceptance criteria, and **missing** has no accepted implementation. Update a
roadmap checkbox only after its gate passes and link the test or capture here.

| Area | Status | Current evidence | Gate evidence |
|---|---|---|---|
| A1 field registry | Partial | `wxdata::field`; MRMS and derived snow bands, NEXRAD Level III grids, NOHRSC snowfall, plus existing HRRR/RAP/NBM and GFS/ECMWF fields; stable workspace/favorite/recent IDs with legacy-slug reads; descriptor-backed search, native-value sampling, and separate device-sized display grids | `cargo test -p wxdata --lib`; workspace, descriptor coverage, field-ID, history and search tests |
| A2 time alignment | Partial | `timecoord.rs`; class-specific and product-overridden observed/analysis/forecast/derived alignment, visible offsets and tolerance warnings, provenance-backed exact-time model differences, workspace-persisted pane time linking, and native ABI nearest-past archive selection from the radar timeline | Coordinator, radar/MRMS/ABI/GLM alignment, compatibility, nearest-volume pane-link, and ABI object-time parsing tests pass; live cross-source release capture pending |
| A3 requests/cancellation | Partial | hashed request identity, deduplication, one bounded retry inside the overall timeout, abortable fetches, generation checks, and a cancellation boundary before oversized-grid pooling | Retry, shared-consumer, stale-result, and cancellation-boundary tests pass; cancellation inside third-party decoders pending |
| A4 provenance/health | Partial | shared source health records registered fields' `DataStamp` valid time separately from request-success age | Unmigrated feeds pending |
| A5 persistent cache | Partial | native caches, pinned IndexedDB chase packs, automatic MRMS, immutable GOES ABI object, and exact-identity model byte-range caches with preserved receipt times, bounded memory fallback, visible degraded status, and pin-aware eviction/clear behavior; selected ABI bands are resolved across the full saved radar timeline and pinned separately from the automatic quota | Integrity/LRU/range-identity/pin tests and compatible pack-manifest tests pass; HTTP validator revalidation for mutable objects remains pending |
| B operational radar | Partial | AWS live-chunk provider adapter with immediate complete-volume archive fallback, progressive radial-block merge, six moments, VCP/cut/radial/latency/retry telemetry, and native gate inspector | Reordered/duplicate/missing-block and fallback-state tests pass; realtime chunks exactly match the same archived KDMX Level II fixture; native-value and gate-geometry sampling test passes |
| C radar analysis | Partial | derived radar products and detectors; versioned portable product definitions have a live-validating settings editor, are saved/imported/exported/synced with settings, and compile to a typed CPU AST with declared radar/environment inputs and deterministic limits; gate-local products use normal pane rendering and sampling; a bounded, order-independent min/max trail engine applies threshold/window/reset controls, retains contributor age, and exports coordinate/time/value CSV; shared beam-coverage logic reports half-power envelopes, terrain blockage, lowest usable tilt, and the best neighboring radar at a point | DSL parser/type/resource/round-trip/scalar/column/aligned-sweep tests, trail order/window/reset/grid/export tests, and beam envelope/blockage/comparison tests pass; trail rendering, neighboring-radar comparison UI, and GPU expression parity remain pending |
| D satellite/GLM | Partial | GLM flashes and registry-backed flash-extent density; all 16 native ABI CMIP channels plus declarative C02/C03/C01 true-color rendering across selectable CONUS, Mesoscale 1, and Mesoscale 2 scenes with one-minute discovery; catalog channels generate search rows, source dispatch, palettes, timeline following, and offline-pack bands from metadata; fixed-grid navigation, quality masking, native sampling, unequal-channel-grid reprojection, automatic reload cache, and timeline-complete pinned web packs | Real GOES-19 C13 mesoscale fixture passes value, DQF, timestamp, forward/inverse projection, native sample, and display-frame checks; all-channel/scene registry, RGB quality/time/grid, direct-RGBA upload, ABI object-time, cache LRU/pinning, and pack-manifest tests pass; live cross-source release capture pending |
| E MRMS catalog | Partial | registry-backed operational subset in `mrms.rs`, including composite/low/high/super-high reflectivity, 18/30/50/60 dBZ echo tops, high-resolution VIL/VIL density, low/mid-level AzShear, MESH/POSH, gauge-corrected and radar-only 1/3/6/12/24-hour QPE, plus FLASH 30-minute/1/3/6/12/24-hour and maximum ARI; catalog entries generate layer/search rows, acquisition, and palette selection without product-specific UI or renderer branches; public freshness is checked by scheduled CI and sentinel values are normalized before sampling | Descriptor coverage, missing-value, palette coverage, and official feed-contract tests pass; broader catalog pending |
| F model workstation | Partial | shared HRRR/RAP/RRFS/NAM/NBM/GFS/GEFS/ECMWF source and schedule definitions, range-index decoders, registered fields, global fields, and comparisons; GEFS ensemble mean and spread are selectable for global fields; RRFS v1 parallel is selectable for environment fields; REFS >40 dBZ probability is selectable by forecast hour and preserves available-member provenance | Live RRFS CAPE and 14-member REFS probability range reads each cover 1,088,905 finite cells; pending vertical mappings, probabilities, postage stamps, plumes, and ensemble gate |
| G analysis/verification | Partial | observations, soundings, warning verification | Pending objective analysis/forecast verification |
| H workspaces | Partial | workspaces, 1/2/4 pane layouts, camera/time link groups, and a geographic cursor with per-pane native-value readout projected across linked panes | Workspace compatibility and native radar sampling tests pass; resource gate remains pending |
| I GIS | Partial | GeoJSON/placefiles | Pending Shapefile/KML and explicit CRS gate |
| J 3D | Partial | volume view and cross sections | Pending multi-moment/slice/isosurface gate |
| K case/chase | Partial | GPS, packs, sharing, archive replay | Pending portable case manifests/routing exposure |
| L output/API | Partial | snapshots, GIF/MP4, streamer mode | Pending deterministic exports and local API |
| M backtesting | Partial | warning verification | Pending repeatable algorithm/alert backtests |
| N diagnostics | Partial | source health and request status | Continuous |
| O performance | Partial | profiling, smoke tests, bundle budget | Continuous |
| P extensions | Partial | native command plugins | Pending capability-declared IPC/Python |
| Q accessibility | Partial | keyboard/touch foundations | Continuous |
| R research | Missing | — | Pending separately validated research gates |

## Baseline

Baseline commit: `9a0847c6521e9d9354c42cde09d291f4f2b67327`.

| Measure | Baseline | Method |
|---|---:|---|
| Main web WASM | 10,835,167 bytes raw / 3,999,279 gzip | `scripts/web/build.sh`, generated artifact |
| Lite web WASM | 149,346 bytes raw / 72,109 gzip | `scripts/web/build.sh`, generated artifact |
| Linux release executable | 39,041,288 bytes | `cargo build --release -p hookecho` |

The web gate is currently 4,111,000 gzip bytes. The in-app radar-product editor measured 4,109,552
bytes; the ceiling keeps a narrow regression margin while retaining formula validation and input
controls on web as well as native builds.

Startup, resident memory, and interactive frame timing are hardware-dependent. Capture them with
`scripts/perf/capture.sh` and the in-app performance panel on the release-test hardware before the
foundation promotion; do not turn one developer-machine run into a universal threshold.
