# U.S. analyst roadmap implementation evidence

Status is conservative: **existing** has repository evidence, **partial** has useful foundations
but misses roadmap acceptance criteria, and **missing** has no accepted implementation. Update a
roadmap checkbox only after its gate passes and link the test or capture here.

| Area | Status | Current evidence | Gate evidence |
|---|---|---|---|
| A1 field registry | Partial | `wxdata::field`; MRMS and derived snow bands, NEXRAD Level III grids, NOHRSC snowfall, plus existing HRRR/RAP/NBM and GFS/ECMWF fields; stable workspace/favorite/recent IDs with legacy-slug reads; descriptor-backed search, native-value sampling, and separate device-sized display grids | `cargo test -p wxdata --lib`; workspace, descriptor coverage, field-ID, history and search tests |
| A2 time alignment | Partial | `timecoord.rs`; class-specific observed/analysis/forecast/derived alignment, visible offsets and tolerance warnings, provenance-backed exact-time model differences, workspace-persisted pane time linking, and native ABI nearest-past archive selection from the radar timeline | Coordinator, compatibility, nearest-volume pane-link, and ABI object-time parsing tests pass; full cross-source release capture pending |
| A3 requests/cancellation | Partial | hashed request identity, deduplication, one bounded retry inside the overall timeout, abortable fetches, generation checks, and a cancellation boundary before oversized-grid pooling | Retry, shared-consumer, stale-result, and cancellation-boundary tests pass; cancellation inside third-party decoders pending |
| A4 provenance/health | Partial | shared source health records registered fields' `DataStamp` valid time separately from request-success age | Unmigrated feeds pending |
| A5 persistent cache | Partial | native caches, pinned IndexedDB chase packs, automatic MRMS and immutable GOES ABI object caches with preserved receipt times, bounded memory fallback, visible degraded status, and pin-aware eviction/clear behavior; selected ABI bands are resolved across the full saved radar timeline and pinned separately from the automatic quota | Integrity/LRU/pin tests and compatible pack-manifest tests pass; model byte ranges pending milestone 4 |
| B operational radar | Partial | AWS live-chunk provider adapter with immediate complete-volume archive fallback, progressive radial-block merge, six moments, VCP/cut/radial/latency/retry telemetry, and native gate inspector | Reordered/duplicate/missing-block and fallback-state tests pass; realtime chunks exactly match the same archived KDMX Level II fixture; native-value and gate-geometry sampling test passes |
| C radar analysis | Partial | derived radar products and detectors | Pending bounded product DSL |
| D satellite/GLM | Partial | GLM flashes and registry-backed flash-extent density; native ABI C13 clean IR, C08 water vapor, C02 red-visible, and declarative C02/C03/C01 true-color rendering; fixed-grid navigation, quality masking, native sampling, unequal-channel-grid reprojection, radar-timeline archive following, automatic reload cache, and timeline-complete pinned web packs | Real GOES-19 C13 mesoscale fixture passes value, DQF, timestamp, forward/inverse projection, native sample, and display-frame checks; RGB quality/time/grid, direct-RGBA upload, ABI object-time, cache LRU/pinning, and pack-manifest tests pass; live cross-source release capture pending |
| E MRMS catalog | Partial | registry-backed operational subset in `mrms.rs`, including gauge-corrected 1/3/6/12/24-hour QPE windows whose public prefixes are checked by scheduled CI | Descriptor coverage and official-prefix contract tests pass; broader catalog pending |
| F model workstation | Partial | HRRR/RAP/global fields and comparisons | Pending generic engine and ensemble gate |
| G analysis/verification | Partial | observations, soundings, warning verification | Pending objective analysis/forecast verification |
| H workspaces | Existing | workspaces and 1/2/4 pane layouts | Pending linked probes/resource gate |
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

Startup, resident memory, and interactive frame timing are hardware-dependent. Capture them with
`scripts/perf/capture.sh` and the in-app performance panel on the release-test hardware before the
foundation promotion; do not turn one developer-machine run into a universal threshold.
