# U.S. analyst roadmap implementation evidence

Status is conservative: **existing** has repository evidence, **partial** has useful foundations
but misses roadmap acceptance criteria, and **missing** has no accepted implementation. Update a
roadmap checkbox only after its gate passes and link the test or capture here.

| Area | Status | Current evidence | Gate evidence |
|---|---|---|---|
| A1 field registry | Partial | `wxdata::field`; MRMS descriptors; stable workspace IDs with legacy-slug reads; descriptor-backed search and native-value map sampling | `cargo test -p wxdata --lib`; workspace, descriptor coverage, field-ID and search tests |
| A2 time alignment | Partial | `timecoord.rs`; observed MRMS draw alignment; exact-time model differences | Coordinator and compatibility tests pass; HRRR/RAP registry migration pending |
| A3 requests/cancellation | Partial | hashed request identity, deduplication, abortable fetches, generation checks | Shared-consumer and stale-result tests pass; decode-stage cancellation pending |
| A4 provenance/health | Partial | source health plus reflectivity `DataStamp` inspector; valid-data age is distinct from request-success age | Pending all migrated families |
| A5 persistent cache | Partial | native caches, pinned IndexedDB chase packs, automatic MRMS object cache | Integrity/LRU tests and web storage controls pass; model byte ranges pending milestone 4 |
| B operational radar | Partial | Level II stream/archive, progressive merge, six moments | Pending provider/failover fixture gate |
| C radar analysis | Partial | derived radar products and detectors | Pending bounded product DSL |
| D satellite/GLM | Partial | GLM support | Pending native ABI ingest and projection gate |
| E MRMS catalog | Partial | operational subset in `mrms.rs` | Pending descriptor-driven supported catalog |
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
