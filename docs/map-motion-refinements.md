# Map motion refinements

Mouse-wheel input drives a time-based camera target instead of applying egui's smoothed scroll tail directly. Line and page units identify discrete input; pixel input uses magnitude and cadence to estimate wheel versus precision scrolling. Each original event uses a bounded nonlinear zoom curve before events are summed: a 120-point wheel event requests about 0.334 zoom levels. Horizontal scrolling retains its pan units. Ctrl/cmd wheel and touch pinch remain direct.

Zoom uses an analytic critically damped spring, reaching about 99% of a single request in 220 ms. Repeated input at the same anchor retains its velocity; reversals discard momentum that would pass the new target. Zoom advances by elapsed time through slow frames rather than stretching the animation; stalls longer than 250 ms finish the requested zoom and cancel pan inertia. Wheel zoom keeps its cursor anchor, including when a second wheel input arrives at a different cursor position before the first ease settles; each unfinished impulse retains its original anchor. Released drags coast with exponential deceleration. Touch pinch remains direct. Reduced motion applies wheel changes immediately and disables coasting. Focus loss, pane resizing, projection changes, externally placed cameras, and a new pointer press cancel pending motion. A fresh wheel event after reactive sleep still starts normally; holding a drag still before release does not throw the map. Diagonal scrolling retains both its pan and zoom components.

Raster and vector detail selection have a 0.3-zoom dead band around rounded level boundaries. Large moves still change detail promptly. Cold requests prioritize a small coarse backdrop before fine streets, and the parent/child geography and label fallback remains in use. Globe's existing conservative tile-count cap is applied after detail selection.

Returning labels retain priority during camera movement. Label entry and exit have separate zoom thresholds, and optional geography text has a stable limit of 192 layout candidates per pane. That limit does not shrink when a gesture starts. Nearby duplicate route shields are rejected before layout, and shields can spend only half the allowance so town names retain space. City names remain eligible during movement. Weather warnings retain their separate priority pass. Collision rectangles reset for each clipped pane while frame visibility history is retained, avoiding cross-pane suppression and priority-order assertion failures.

Tile and vector-buffer uploads share a budget across the frame: two items and 2 MiB during movement; eight items and 8 MiB otherwise. One oversized first item may proceed to prevent starvation. Unspent work stays queued, stale vector generations are rejected, and another frame is requested. Gesture upload turns rotate across visible desktop panes; compact layouts can spend the budget on their sole visible pane. MVT decoding/tessellation remains in background workers, and camera motion keeps zoom-driven overlay retessellation deferred until settling. New overlay data still refreshes immediately.

## Measurement

`scripts/web/map-motion.mjs` records original wheel events, camera/frame traces, tile uploads, optional text work, pane UI cost, and WebGL command counts. It uses a historical KDMX sweep under synthetic catalog names and captures OpenFreeMap payloads once for subsequent replay. The scenarios cover a prewarmed route, delayed tile responses, independent and linked quad views, and reduced motion. Separate checks cover drag coasting, direct touch pinch, cursor anchoring, shared upload bounds, and transition screenshots.

Set `globalThis.__hookechoMapTraceEnabled = true` before boot to enable the bounded trace; ordinary sessions do not collect it. The local test buffer can contain camera coordinates. It is separate from user-facing diagnostics and is never automatically transmitted.

Trace frame rows are `[time_ms, pane, frame, zoom, center_x, center_y, width, height, scroll, pinch, moving, tile_zoom, vector_uploads, raster_uploads, label_candidates, preparation_ms]`. Optional cost rows are `[time_ms, pane, frame, geography_layout_candidates, pane_ui_ms]`. Each array is capped at 4096 observations and omissions are counted. Preparation time precedes painter overlays; pane UI time includes them. Neither is GPU completion or display latency.

Whole-window frame intervals include intentional reactive idle sleeps. Use `movingP95` for camera movement; do not interpret whole-window p95 or idle Mapbox render-event gaps as dropped-frame rates. The prewarmed route can still receive tiles. Comparisons are between HookEcho builds on the same machine and recorded scenes, not a reference app speed ranking.

Run the usual web build and serve `web`, then run:

```sh
CHROME_BIN=/usr/bin/chromium HEADFUL=1 EXPECT_MOTION=1 \
HOOKECHO_URL=http://127.0.0.1:8765/ \
HOOKECHO_MOTION_OUTPUT=/tmp/hookecho-motion \
HOOKECHO_MOTION_TILES=/tmp/hookecho-motion-tiles \
node scripts/web/map-motion.mjs
```

After capturing every needed tile, `REPLAY_ONLY=1` rejects any unrecorded payload. Keep compilation and other benchmarking stopped during measurements. Hardware WebGL is verified from the captured renderer string; the default headless software path is useful for functional checks and is not a hardware performance comparison.

## Validation

The final verification record includes native controller, anchor, refresh-rate, idle/release, mixed-input, detail, label, and deferred upload-generation regressions; workspace clippy; the 13 native GPU checks; optimized web size checks; browser traces and screenshots; and PR CI. Measurement results are reported with their workload and timing limits.

## Radar preparation and smoothing

Display cache misses now prepare the selected cut on a native blocking task or the existing browser decode worker. Pending work is deduplicated and bounded; scan revisions discard obsolete results. The previous GPU image stays visible during a scan refresh, while a different product or tilt waits for matching data rather than relabeling the old image. Playback waits for that preparation, and returning to a product already displayed cancels abandoned work so it cannot hold the playhead indefinitely.

MRMS gzip/GRIB decoding, missing-value normalization, and display pooling use the same background path. Full native fields remain available for analysis; categorical grids are subsampled rather than max-pooled. Live radar merging selects borrowed winners before cloning the retained radials, avoiding copies that would immediately be overwritten.

Optional multipass smoothing caches the existing filter's float bits in an R32Uint texture. Camera and palette changes reuse it; gate changes invalidate it. The allocation adds four bytes per gate only after this option is enabled, and counts toward the existing cache budget. Missing/folded gates and precise R16 input are preserved. Native GPU readback also caught and corrected a negative-modulo azimuth seam.

`node scripts/web/radar-pipeline.mjs` checks real worker/inline byte equality, malformed-job recovery, and uploaded radar with an unavailable binning worker. Set `HOOKECHO_MRMS_FIXTURE` to a downloaded gzip MRMS object to exercise its actual packing; the default checks the committed radar fixture without an external download. One local 98.5 MB decoded-response test kept page timers running during the worker job; the equivalent inline call blocked for about half a second. This measures responsiveness, not a throughput speedup, and excludes the Rust caller's response deserialization.

With smoothing and precision enabled, RTX 4080 browser traces remained near the 60 Hz presentation limit: moving-frame p95 was approximately 17.1 ms in both single and four-pane scenes before and after. This does not establish a universal FPS gain. The final browser bundle adds about 8 KB gzip for the worker paths and cache; the explicit main budget is 4,700,000 bytes, with Lite/startup budgets unchanged.

A hosted processed-sweep cache was assessed but is not part of this change: the existing edge Worker caches raw upstream responses, and the snapshot server exposes metadata/images rather than a bounded sweep-data API. It needs a separate decoder/storage deployment and operating limits.
