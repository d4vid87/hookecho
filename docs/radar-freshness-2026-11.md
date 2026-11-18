# Radar freshness and recovery engineering report

**Investigation date:** November 18, 2026. **Baseline:** stable `0.12.0`, commit
`fb0221e39af3672ff468775c722df0e117837582`. **Candidate:** working tree on
`fix/radar-freshness-20261118`; measurements were collected before publication. Controlled
before/after measurements and native, web-build, GPU, storage and Android
compilation checks are complete, including the final browser workflow checks.

## Executive assessment

HookEcho has confirmed application-side freshness defects, but the evidence does
not establish one universal delay or a measured competitor advantage. The
baseline suppressed incremental Level II ingestion during live-loop playback,
could display a clock unrelated to the selected acquisition, restarted browser
cache freshness, and discarded queued work when a decoder worker failed. Those
are distinct from the radar's scan cadence and provider publication delay.

The three most valuable changes are:

1. Acquire live chunks independently of playback, retaining one mutable head
   while preserving the historical playhead and excluding partial scans from
   completed-object caches and exports.
2. Associate the displayed clock with the selected moment, sweep and recorded
   paint token; keep acquisition, generation, upload and receipt clocks separate.
3. Repair recovery boundaries: worker restart, cached-response age, same-volume
   archive fallback, foreground/reconnect refresh, and Android service timeout.

These changes improve correctness and remove demonstrated delay mechanisms.
They do **not yet establish an end-to-end latency reduction**. The completed
experiments measure source discovery, worker processing and controlled browser
before/after behavior separately. Actual monitor/compositor presentation remains unavailable.

## Architecture and clocks

The shared Rust application serves native desktop, native Android and wasm.
`wxdata` owns acquisition/decoding; `app.rs` coordinates request ownership and
playback; `view.rs` owns pane state and prepared sweeps; wgpu records rendering.
Android uses GameActivity plus Kotlin/Compose integrations. Browser work uses a
second wasm instance in a worker and an allowlisted same-origin proxy. The
standalone server has equivalent proxy policy; there is no new shared ingestion
backend. See [architecture](../ARCHITECTURE.md) and [source inventory](DATA.md).

| Boundary | Identity/time retained | What the evidence can establish |
| --- | --- | --- |
| Instrument acquisition | Level II radial times, moment, elevation/cut; SAILS revisits remain distinct | Selected sweep's oldest/newest acquired sectors; volume start is a separate reference. |
| Provider publication/discovery | Public object/chunk key, chunk sequence, S3 Last-Modified; monitor first-seen | HTTP metadata and first observation by this monitor. Neither proves acquisition or exact publication time. |
| Request/receipt/cache | Request start, download completion, cache result | Local transport/cache stages; first-byte timing is not uniformly instrumented. |
| Decode/assembly | Native queue/start/end; browser job queue, worker interval and round trip | Local processing and contention. Decode is inside round trip; overlapping intervals must not be added. |
| Selection/preparation | Site, source, moment, tilt/cut, appearance revision, upload token | Which decoded acquisition was selected/prepared, including retained-sector ages. |
| GPU/paint | Upload-prepared token and map paint callback token | Draw commands were recorded for that identity; OS/compositor presentation is explicitly unknown. |

Level III now preserves PDB scan and generation clocks separately from message
transmission; products 134/135 use the documented volume-end convention. TDWR
exposes generation time, not individual beam acquisition. MRMS uses GRIB valid
time; it does not establish the age of every mosaic constituent. Missing times
are rejected or labeled unavailable rather than replaced with the current clock.
Derived radar products retain an explicit volume-reference basis.

Local diagnostics retain at most 512 Rust events and 128 worker timings, with
public feed identities rather than credentials, local paths or custom-product
names. Export is user initiated. MRMS has request/cache/decode events, **not yet
per-field upload/paint identity**. Level II paint tokens therefore cannot be
used to claim complete MRMS observation-to-display timing.

Existing optimizations were preserved: shared reads and generation-based
cancellation, native decode limits, worker priority for visible radar work,
transferable buffers, compact scientific payloads, scan/binned/texture caches,
incremental changed-tilt invalidation, bounded loop retention, optional display
decimation, and deferred map refinement during gestures. Scientific source data
remains distinct from display preparation. This work adds no dependency or
second general worker pool.

## Measurements and reproducibility

Durable evidence root:
`/home/dwm/Documents/HookEcho/radar-freshness-2026-11-18/`.
Scripts live under `scripts/web/`; JSON reports retain workload,
timestamps, hashes, environment and limitations. Quantiles below are descriptive
sample summaries. Sample sizes and correlated scan events are insufficient for
a defensible p99 service target.

**Live source monitor.** `web/source-summary.json` covers KFWS from 21:51:40 to
22:01:41 UTC: 41 samples at a requested 15-second interval, 95 listing requests,
28 initially visible objects and 83 newly observed chunks across three volume
prefixes, including two end chunks; no recorded errors. Measured listing
requests: n=82, p50 322 ms, p95 443 ms. Newly seen minus S3 Last-Modified:
p50 7.754 s, p95 12.777 s, maximum 13.788 s. The corresponding absence-to-first-seen
window was about 15.25 s. This is upload-metadata-to-monitor delay under the
monitor's sampling schedule, **not HookEcho latency or radar observation age**.
It does not measure sustained storm-mode behavior or another station.

**Decoder experiment.** `web/worker-report.json` used Chrome 155.0.8059.84,
the deployed wasm, and the committed historical KDMX single-sweep fixture
(1,250,687 bytes). Baseline/candidate JS each ran ten cold-worker, ten warm-worker
and twenty contended jobs: 80 outputs had identical hashes. Cold means a new
worker heap with an already compiled module; it excludes network and compilation.
Contention queued identical archive jobs, not a representative heavy-layer mix.

| Worker round trip | Baseline p50 / p95 | Candidate JS p50 / p95 | Interpretation |
| --- | --- | --- | --- |
| Cold worker, n=10 each | 122.8 / 154.7 ms | 126.3 / 131.5 ms | Mixed small-sample result; no speed claim. |
| Warm worker, n=10 each | 104.5 / 108.4 ms | 103.2 / 110.8 ms | No established improvement beyond noise. |
| Contended, n=20 each | 105.2 / 113.9 ms | 105.1 / 108.2 ms | Queue delay is additional; decode/round trip overlap. |

The decisive result is recovery: an injected worker trap rejected the active job
in both variants. The baseline also rejected its queued peer; the candidate ran
that peer successfully in the replacement worker. Five focused regressions fail
against baseline JS; the final candidate Node suite passes 90 tests.

**Full-application comparison.** `web/app-baseline/report.json` and
`web/app-candidate/report.json` each record ten cold contexts and ten warm reloads
at 1200×900 on Linux, Ryzen 9 5900X, 24 logical CPUs, about 64 GiB RAM, Chrome
155.0.8059.84 with headless SwiftShader. Matched in-memory local servers supplied
uncompressed baseline/candidate release bundles with the same headers and fixture:
one ArchiveOnly pane, historical KDMX data under synthetic current-date names,
empty vector tiles/overlays and deterministic NWS status. Camera state was reset
and asserted for every run. A superseded preliminary run that inherited the
wheel-adjusted camera on reload is excluded.

This isolates application work; it excludes real CDN/mobile networking and live
radar delivery. Service worker was unavailable/bypassed; wasm was refetched in
both modes. “Warm” means a reload in the same browser context, not a fully cached
HTTP payload. User applications remained running on the host.

| Metric | Baseline cold p50 / p95 | Candidate cold p50 / p95 | Baseline warm p50 / p95 | Candidate warm p50 / p95 |
| --- | --- | --- | --- | --- |
| Navigation to first decode completion, n=10 each | 796 / 984 ms | 828 / 1,082 ms | 481 / 524 ms | 510 / 587 ms |
| Navigation to first bin completion, n=10 each | 909 / 1,089 ms | 964 / 1,197 ms | 568 / 612 ms | 608 / 675 ms |
| Wheel event to changed map record, n=120 each | 1.3 / 3.0 ms | 1.4 / 2.1 ms | 1.2 / 2.0 ms | 1.3 / 2.0 ms |
| Map preparation, n=906 / 903 cold, 898 / 865 warm | 0.10 / 0.30 ms | 0.10 / 0.40 ms | 0.10 / 0.20 ms | 0.10 / 0.20 ms |

For the ten startup samples per cohort, p95 equals the observed maximum. Input
maxima were 4.5/3.8 ms baseline/candidate cold and 2.5/4.0 ms warm; map-preparation
maxima were 2.3/5.4 ms cold and 2.1/0.9 ms warm. No page errors or missing input
records occurred. These are preparation/camera-record timings, not physical
input-to-photon latency. Page JS heap medians were 3.14/3.20 MB cold and
3.51/3.55 MB warm; worker, wasm and GPU allocations are excluded. Transfers were
17.03/17.10 MB cold and 15.31/15.37 MB warm. Cold long tasks numbered 21/23 with
maxima 342/322 ms; warm counts were 2/3 with maximum 54 ms in both variants.

**Order-controlled repeat.** Because the first comparison showed slower startup,
`web/app-interleaved/comparison.json` repeats five paired trials, alternating
baseline→candidate and candidate→baseline, with cold and warm pages in one
Chromium process. It ran 22:39:43–22:41:28 UTC without concurrent build/browser
jobs. All 20 pages passed; the fixture and hosting conditions remained matched.
The functional-suite coordinator was suspended after its loop-loading child had
exited; its recorded wall time therefore spans this repeat. The post-run transcript reconstruction
`web/app-interleaved/coordination.json` explains that apparent overlap; no
timestamped full-machine process inventory was saved.

| Interleaved metric | Baseline p50 / max | Candidate p50 / max | Median paired candidate-minus-baseline |
| --- | --- | --- | --- |
| Cold first decode, n=5 each | 857 / 990 ms | 826 / 835 ms | −41 ms (−4.79%) |
| Cold first bin, n=5 each | 977 / 1,160 ms | 942 / 968 ms | −42 ms (−4.31%) |
| Warm first decode, n=5 each | 521 / 593 ms | 547 / 572 ms | +17 ms (+3.49%) |
| Warm first bin, n=5 each | 598 / 666 ms | 619 / 668 ms | +9 ms (+1.42%) |

The initial cold-start regression did not reproduce; paired warm signs were
mixed, but small warm overhead remains plausible. These samples establish no
startup speed improvement. Wheel-to-camera p95/max was 2.0/2.3→2.3/2.9 ms cold
and 2.2/3.6→2.1/2.4 ms warm, with no missing records. Cold map-preparation p95/max
was 0.3/1.9→0.6/5.2 ms; warm was 0.2/0.6→0.2/0.7 ms. Candidate cold preparation
outliers warrant tracking on representative hardware. Cold long-task counts were
14→11 with maxima 305→218 ms; warm counts were two each, maxima 53→55 ms.
Uncompressed transfer medians rose by 66,716 bytes cold and 63,393 bytes warm.
Full per-run values, queue counters, page heap and maxima remain in the JSON.

**Frame association and large-field preservation.** Candidate traces tie the
prepared and paint-recorded token to KDMX reflectivity acquisition
`2022-03-05T23:23:40.627Z`, despite the fixture's synthetic 2026 object name.
The UI correctly labels the observation stale. Actual compositor presentation
remains null. The browser online event is consumed exactly once.

A pinned current MRMS object (valid 22:22:37 UTC) was decoded through both baseline
and candidate wasm, each in a worker and inline. All four 98,500,122-byte outputs
had SHA-256 `194461b89823be692754ca4bb12c72b9ecc83abbd1563858e237dec7f6346736`.
The 1,628,777-byte input and its provenance are retained under `web/mrms-fixture/`.
Page timers advanced 102/100 times during baseline/candidate worker decoding,
versus zero during inline decoding, demonstrating this workload's main-thread
blocking fallback cost. These single runs establish byte equality and worker
responsiveness, not a comparative decode-speed gain. Malformed-job recovery,
inline fallback and bin-output equality checks also passed in both variants.

To reproduce the paired fixture run from this checkout, first start the saved
`web/scripts/hookecho-matched-pair-server-20261118.mjs` helper under the evidence
root. It verifies archived baseline asset hashes and serves both bundles with
matched headers on ports 8767/8766. Stop other test/build jobs, then run from the
repository root, choosing a fresh output directory:

```sh
HOOKECHO_BASE_URL=http://127.0.0.1:8767/ \
HOOKECHO_COMPARE_BASE_URL=http://127.0.0.1:8766/ \
HOOKECHO_FRESHNESS_OUTPUT=/tmp/hookecho-freshness-repeat \
RUNS=5 EXPECT_TRACE=1 CHROME_BIN=/usr/bin/chromium \
node scripts/web/freshness-app.mjs
```

The source-monitor and worker experiments are similarly self-contained in
`scripts/web/source-monitor.mjs` and `scripts/web/freshness.mjs`; their headers list
inputs and output locations. Live monitoring samples a new interval, so its
result should not be treated as an identical replay of this investigation.

These measurements have deliberately different scopes:

| Requested metric | Available evidence / missing observation |
| --- | --- |
| Observation age | Selected Level II oldest/newest radial clocks and Level III generation provenance are tested. The source monitor did not decode every live acquisition. |
| Upstream availability to displayed frame | Discovery windows and local stage traces are available separately; no paired end-to-end live or competitor distribution was collected. |
| First useful/newest radar | Browser decode/bin boundaries and paint-recorded identities are instrumented. They do not establish the compositor's first visible frame. |
| Interaction/frame timing | Controlled wheel-to-camera and map-preparation distributions; no input-to-photon or sustained device FPS claim. |
| Memory/network | Harness transfer counts, page JS heap and application retention/upload counters; no complete process/VRAM or long-session heap profile. |
| CPU/GPU/battery/thermal | Software-renderer correctness checks; no controlled utilization, battery or thermal measurements on representative mobile hardware. |

## Prioritized changes and regression evidence

P1 denotes misleading data, sustained recovery failure or crash risk; P2 denotes
bounded delay or misleading status. Confidence is confirmed by source plus a
fixture/contract unless explicitly marked. Effort/risk describe implementation
and regression exposure, not promised performance gain.

| Priority; platform | File:function and resulting behavior | Effort / risk | Proof or final gate |
| --- | --- | --- | --- |
| P1; all full-app targets | `app.rs:manage_stream/sync_timeline`, `view.rs:receive_live`, `timeline.rs:set_live_head`: ingestion continues during live loops; one mutable head; history selection remains stable. | M / medium state-machine risk | Passing workspace tests cover rollover, stale revisions and Return Latest. |
| P1; all | `view.rs:displayed_time/stage_presentation`, `level2.rs:observation_time_at`, `gpu_metrics.rs:mark_painted`: selected sweep clocks only become displayed after matching paint acknowledgement. | M / medium | Split moments, repeated cuts, retained sectors and old-token rejection tests. No compositor assertion. |
| P1; all | `live.rs:merge_scan/stitch`, `app.rs:poll_messages`: reject delayed VCP/cut rollback; provider recovery compares acquisition with acquisition, allowing a completed same-volume archive to replace partial data. | M / medium | Delayed-chunk and same-volume fallback regressions pass in the final workspace suite. |
| P1; playback/export | `view.rs:receive_live/displays_frame`, `casefile.rs:CasePane::capture`: accumulate skipped tilt invalidations; recognize live identity in linked panes; exclude transient keys from pinned cases/offline packs/loop exports. | M / medium | Review reproduced stale-bin, eight-second linked wait and unreopenable-case sequences; targeted regressions added. |
| P1; browser | `decode-bridge.js:retire`: preserve untransferred queued jobs, reject late retired-worker messages, retain bounded retry/fallback. | S / low | Injected-trap experiment plus baseline-failing/current-passing queue tests. |
| P1/P2; browser/server | `proxy-core.js:handleProxy/cacheSeconds`, `serve.rs:proxy_reply/fetch_capped`: preserve Age, reject unsolicited 206, keep errors uncached, shorten mutable `sn.last` TTL. | S / medium HTTP risk | Body/304 Age, requested ranges, partial objects and TTL regressions; Node suite passes. |
| P1; Level III/TDWR/MRMS | `nexrad-level3:decode`, `level3.rs:product_time`, `mrms.rs:decode_grib2`: preserve typed clocks and reject missing valid time. | S–M / medium format risk | Golden timestamp and invalid-time tests pass. |
| P1; MRMS and shared GRIB decoder | `mrms.rs:fetch_key_display/decode_gz/decode_grib2` and vendored packing implementations: cap transport/expanded bytes, validate actual decoded counts and embedded image dimensions, preserve constant fields and missing values. | M / medium format risk | Gzip boundaries, real-GRIB count mismatch and small malformed packing regressions; 56 vendor and 18 MRMS tests pass. These checks are not a universal codec-heap guarantee. |
| P1/P2; Android | `AlertService:onTimeout/pollOnce`, `RadarWidget:snapshotAge`: stop promptly on OS allowance expiry; cancel subsequent service work; label snapshot save age accurately. | S / low | 15 JVM tests and instrumentation APK compile pass; no executed device timeout test. |
| P2; browser/settings | `platform.rs:network_recovered`, app foreground reconciliation, `Settings:effective_poll_interval_secs`: reset retry clocks on recovery and show effective polling policy. | S / low | Settings tests and final trace verify one-shot online reconciliation; browser lifecycle/reliability checks cover resume and retained offline state. |

The review caught integration defects introduced by the initial live-head change
before release; they are not presented as measured production incidents.

The decoder review also verified constant-field conventions against
[NOAA's simple unpacker](https://raw.githubusercontent.com/NOAA-EMC/NCEPLIBS-g2c/develop/src/simunpack.c)
and [PNG unpacker](https://raw.githubusercontent.com/NOAA-EMC/NCEPLIBS-g2c/develop/src/pngunpack.c).
Zero-bit constants retain the reference value directly; bitmap gaps stay missing.
Malformed decoded lengths are rejected instead of padded with valid zeros.
Previously unsupported row-by-row complex packing now reports an explicit error;
this patch does not claim to add that packing mode.

## Transferable practices and operating budgets

The [dated research table](radar-freshness-research-2026-11.md) separates vendor,
primary, historical and direct evidence. RadarScope documents observation versus
check time and product/source clarity; RadarOmega documents incremental selected
sweeps independent of the beam overlay; Supercell Wx documents updating during
animation and distinguishes update from map-painted completion. NOAA SAILS and
xradar support preserving repeated cuts and missing-sector semantics. RainViewer
mosaic-generation times and Windy's historical timestamp discussion illustrate
why clock labels alone cannot support competitor freshness rankings.

No paid competitor feature, private backend or proprietary push protocol was
tested. Current public NOAA endpoints were checked; the retired archive name was
not copied from old examples. Provider licenses, attribution, rate limits,
support and prices are recorded in the research table. RainViewer's conflicting
pages require conservative transition limits and clarification before expanded
commercial use. No subscription or new service was purchased.

Polling defaults remain 30 seconds, doubled for metering and doubled for saver
(30/60/120 seconds). Shorter mutable-pointer caching can increase upstream
requests, while historical archives retain useful caching. Continuing ingestion
during playback increases network/decode work; one head bounds retained mutable
revisions, not the cost of acquiring them. Streaming still belongs to one active
WSR-88D pane in Automatic/S3 mode while following, after an initial usable volume.
Deliberate historical scrubbing stops it; inactive panes retain existing polling
and linked-pane behavior. This patch does not add independent streams per pane.
MRMS limits are 64 MiB compressed,
512 MiB expanded and 100 million grid cells, preserving the supported
14000×7000 grids. These are ceilings, not mobile memory budgets or proof of
sustained safety. Preserve fidelity and reduce optional visual work before
reducing scientific data or mislabeling lower-resolution products.

The final optimized main wasm is 13,268,783 raw / **4,932,449 gzip bytes**,
15,980 gzip bytes (+0.33%) above the prior recorded 4,916,469-byte build. The
deliberately updated main cap is 4,945,000; Lite/startup caps are unchanged.
This is an explicit first-load cost for correctness and diagnostics, not a
download-size optimization. The final full web build passes its size gates.

## Verification limits, acceptance and release decision

The [platform matrix](radar-freshness-platforms-2026-11.md) records source, CI and
actual commands. Linux/Windows native, experimental Intel macOS, Android ARM64,
full web/PWA, Lite and headless/container paths are distinct. Current GitHub main
and latest published packages point to baseline `fb0221e`; served bridge/worker
JS hashes match that baseline. Wasm filename alone does not prove its revision.
This investigation records pre-publication evidence; consult the release and
deployment workflows for subsequent shipping status.

The open-issue inventory was checked directly and saved as `open-issues.json`:
[small marker icons (#354)](https://github.com/d4vid87/hookecho/issues/354),
[real-device wind fallback (#13)](https://github.com/d4vid87/hookecho/issues/13),
and [macOS hardware validation (#9)](https://github.com/d4vid87/hookecho/issues/9).
None is evidence of a measured radar-freshness delay. Icon-size/accessibility work
and real-device graphics checks remain open; this investigation does not mark
those issues fixed or substitute software-renderer tests for hardware reports.

| Check | Recorded status |
| --- | --- |
| Browser Node suite / baseline failure demonstration | 90 pass including shell-asset checks; five selected new regressions fail on baseline. |
| Android JVM / test APK / native ARM64 | 15 pass; `assembleDebugAndroidTest` and final cargo-ndk build succeed; no unresolved C++ runtime symbols. No connected device. |
| Candidate full-app metrics and browser workflows | Both 20-page cohorts and the 20-page interleaved repeat complete without page errors; acquisition/paint identity and one-shot online reconciliation pass. All 20 browser validation invocations pass, including baseline/candidate MRMS pipelines, default/WebGL/Family Hub smoke, continuity, retries, loop loading, reliability, usability, offline shell, forecast recovery, outlook/site controls, settings, workspace/export, chrome, worldwide and velocity comparison. Supplemental Lite velocity smoke also passes. |
| Final workspace / native build / Clippy | 1,416 tests pass, 98 opt-in tests ignored; all-target build and warnings-denied Clippy pass. |
| Final wasm/Lite, GPU and browser storage | Full web build passes; 28 GPU checks pass on isolated Lavapipe; real IndexedDB offline-weather harness passes all five contracts. |
| Current native MRMS verifier | Fetched/rendered 7000×3500 field valid 22:30:40 UTC; 1,679,810 nonmissing gates, maximum 58.5 dBZ. `mrms-live.log` / `mrms-live.png`; visual inspection confirms radar rendered. This is not a latency comparison. |
| Optional repository-wide formatting | `cargo fmt --all -- --check` fails on widespread formatting differences, including unchanged baseline files. `format-check-result.json` records proof; no bulk reformat was applied. `git diff --check` passes. |
| Physical Android/Family Hub, Windows/macOS runtime, Firefox/Safari, sustained thermal/battery | Not tested in this investigation. Prior deferred checks remain deferred. |

The vendored dependency's own tests are excluded from the application workspace.
The separate 56-test run used a temporary copy with its existing development
dependencies and the repository's real GRIB fixture; no production manifest was
changed. Reproduce from the repository root:

```sh
audit_dir="$(mktemp -d)"
mkdir -p "$audit_dir/vendor" "$audit_dir/crates/wxdata/tests/data"
cp -a vendor/gribberish "$audit_dir/vendor/"
cp crates/wxdata/tests/data/rap-shear-pair.grib2 "$audit_dir/crates/wxdata/tests/data/"
printf '\n[workspace]\n' >> "$audit_dir/vendor/gribberish/Cargo.toml"
cargo test --manifest-path "$audit_dir/vendor/gribberish/Cargo.toml" --lib
```

The actual final command/results list is `final-integration-results.json`; the
vendor log is `data/green-vendor-decoder-constant-corrected-final.log`. Browser
commands, timestamps and logs are indexed by `web/validation/summary.json`;
`web/offline-weather-final.log` records the separately completed storage gate. Keep these
separate from earlier interrupted/failed runs retained for diagnosis.

Correctness and build gates passed. The matched repeat does not reproduce a
cold-start slowdown, but possible warm-start overhead and cold map-preparation
outliers remain explicit tradeoffs. Do not treat these samples as a throughput,
battery, memory or end-to-end freshness acceptance result. Preserve the measured
baseline and repeat on supported hardware rather than setting arbitrary FPS
thresholds. Correctness gates are strict: identical
scientific fixture outputs; no wrong-site/product/time association; no stale
reply rollback; no partial immutable-cache entry; recover after interruption;
preserve deliberate historical selection and explicit unavailable/stale states.
Measure several real scan strategies and longer sessions before assigning
per-source latency or p99 targets. Alert expiry/cancellation/duplicate delivery,
screen-reader use and physical background behavior need their own checks.

Quick follow-ups are MRMS field-to-paint correlation and paired live-source
discovery-to-paint capture over several scan strategies. Medium work is sustained weak-network,
heavy-layer and low-memory testing, then measured workload adaptation. Earlier
audit concerns about IndexedDB closure lifetime and worker-heap retention during
long MRMS loops are **unverified against the current candidate**; reproduce with
retained-object/heap evidence before changing ownership or scheduling.

A shared ingest/SNS fan-out service is a larger option only after public-feed
discovery delay is shown to dominate. It adds queue ordering/deduplication,
reconciliation, availability obligations, request/compute/egress cost and support.
Google Play submission also remains separate: target 35 needs an evaluated
API-36 migration under the [current submission policy](https://support.google.com/googleplay/android-developer/answer/11926878).

Production promotion remains an explicit decision after artifact provenance,
measured tradeoffs and unresolved platform limits are reviewed. Use the existing
CI-gated staging and package workflows; deploy coherent shell/glue/wasm/worker assets together.
Retain baseline packages and the tested browser bundle for rollback. If freshness,
cache correctness or recovery regresses, restore that complete bundle/package
set and re-run source/fixture checks; do not mix worker generations or treat
rollback as a settings reset. The investigation itself performed no push, deployment or public release.
Subsequent publication is handled separately through the existing CI gates.
