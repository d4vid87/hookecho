# Remaining REA findings: implementation and verification

These are independent HookEcho improvements following the RadarOmega architectural research and the subsequent HookEcho source audit. The research did not establish RadarOmega performance superiority. No proprietary renderer, private service adapter, credentials or palettes are included.

## Custom products

Compiled recipes are retained per pane, with a full definition comparison. Derived sweeps are reused when their scan allocation, recipe, tilt, selected cut and precision remain unchanged. Input identity holds a weak scan reference, preventing decoded-volume retention and allocation-address reuse. Binned inputs are immutable shared `Arc` values, so evaluation no longer clones their gate arrays. Palette-table changes reuse calculated values. Live source replacement invalidates output; upload invalidation still permits sector-opacity refreshes. Current outputs have a combined 32 MiB retained-buffer ceiling and disappear from probes after evaluation errors. Unavailable custom samples cannot fall through to an unrelated native-moment probe. Compilation, evaluation and cache-hit counters are available in diagnostics.

## Satellite timing

Each pane owns its satellite selection and frame catalog. Manual selection is an absolute timestamp, preserved in workspace files. Follow-radar mode uses the loaded scan/cut and rejects imagery more than thirty minutes away. Manual unavailable frames remain unavailable rather than silently substituting latest imagery. An explicit setting links satellite selections across panes.

Tile identities include style and exact timestamp; switching pane focus no longer flushes another pane's imagery or confuses late tile results with current images. Disk frame tags include seconds. Catalogs refresh at a bounded interval, with old-window receivers discarded and existing catalogs retained through transient empty refreshes. Failed imagery is labeled incomplete/retrying. Per-pane readouts disclose selected/displayed satellite time and radar difference; loading imagery is distinguished from fully resident imagery. The active satellite controls sit above the radar transport. Vector palettes and native ABI field layers retain their existing global behavior; they are separate from pane-local raster satellite selection.

## Measurements

Map draw GPU timestamps are optional, requested only when the adapter exposes the required capabilities. Readback is asynchronous and bounded; unsupported backends say so explicitly. Draw timing excludes uploads and wind advection. CPU and gesture observations keep their existing meanings; scheduled playback/idle intervals are not treated as gesture pressure.

Diagnostics now separately report retained encoded scan payloads (shared allocations counted once), binned buffers, derived buffers, CPU upload shadows and nominal GPU resource sizes. These exclude scan metadata, allocator/driver overhead and framework targets; they are not whole-process memory usage. The public **Save diagnostics** search action exports these measurements.

A repeatable native test matrix covers one/two/four pane resources, Mercator/globe, R8/R16 and smoothing, with full-sweep and one-row arrivals. Five warm-up frames precede ten measured frames. Every case checks the final incremental image against a fresh render. Each pane renders to the same 256-pixel test target sequentially; this is a controlled workload, not a full desktop viewport or network benchmark.

```sh
HOOKECHO_PERFORMANCE_REPORT=/tmp/hookecho-matrix.json cargo test -p hookecho gpu_performance_matrix -- --ignored --test-threads=1
HOOKECHO_WIND_REPORT=/tmp/hookecho-wind.json cargo test -p hookecho gpu_globe_wind -- --ignored --test-threads=1
```

The initial local RX 580 / RADV Vulkan run verified all 48 matrix cases and produced timestamps in all of them. Synchronized frame p95 ranged approximately 0.98–6.70 ms in the native test profile. These numbers include polling for GPU completion, omit map tiles/network traffic, and establish no comparative or live-feed speedup.

## Globe rendering

Wind now projects particles onto the same orthographic globe as radar, clips the far hemisphere and sky, and clears screen-space trails when the camera/projection changes. GPU wind resources are pane-local, avoiding shared per-frame uniforms clobbering different pane projections or opacities. The CPU fallback remains available.

Tile tessellation uses a projected chord-error estimate, quantized to bounded subdivisions (maximum 16 per axis), rather than fixed 8/16 geometry for every tile. Draw ranges support different vertex counts per tile. A conservative angular-cone test excludes tiles entirely behind the horizon; the existing vertex ceiling remains enforced. Tests check that culling does not discard potentially visible samples, and GPU checks cover globe alignment and sky clipping.

The separate wind test measures CPU update/mesh generation, GPU command enqueue and GPU advection/trails. It also checks visible front-side particles, empty sky, an empty far hemisphere and removal of previous-camera trails. Different particle counts/rendering methods mean these measurements do not constitute an equivalent-work speedup comparison.

## Interruptions and feedback

Selected archive requests own their loading state through a generation and frame identity. Superseded requests are aborted, including when a cached frame completes the selection. Already dispatched blocking decode work may finish, but cannot publish an obsolete result. Late success, failure and phase messages cannot replace a newer selection, a cache hit or a completed request. Revisiting a frame does not revive its previous request. Changing selections can start new work while the previous frame is still loading. Live head/poll results are prevented from overriding unrelated archive selections or clearing selected-frame loading state.

Existing stream-generation, partial-scan retention, retry and vector-generation safeguards remain covered by their tests. Selected downloads report downloading versus decoding; the transport uses loaded scan/cut time, with requested/displayed timestamps in its tooltip. Buffering and skipped unavailable frames have separate feedback and skipped-frame counts.

Browser device loss and actual WebGL context loss share one recovery handler: one automatic reload, then an actionable manual reload panel if failure repeats. Native unexpected device loss logs the error and sends a best-effort OS notification to restart. The in-app message cannot itself restore a failed GPU. Physical native driver removal/reset is not induced by the tests. Android's existing notification shim does not provide a desktop OS notification.

Performance metrics live inside the existing expandable disclosure, with collecting/unsupported states, so new samples do not insert rows above the pane controls.

## Verification record

The task outputs contain the workspace-test and Clippy logs, GPU-test log, 48-case matrix, separate wind timings/image, production bundle build log, and browser persistence/recovery report and screenshots. Browser tests use repository radar bytes and a synthetic satellite catalog/tile response, not live upstream weather. Production headless startup checks and current-revision CI are tracked on [draft PR #367](https://github.com/d4vid87/hookecho/pull/367).

The app bundle cap is deliberately 4,410,000 gzip bytes for the added controls and instrumentation; the lite cap remains 80,000. Verify actual measured sizes in the build log and CI rather than inferring them from the cap.
