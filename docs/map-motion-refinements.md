# Map motion refinements

Mouse-wheel input now drives a time-based camera target instead of applying egui's smoothed scroll tail directly. Wheel zoom keeps its cursor anchor; released drags coast with exponential deceleration. Touch pinch remains direct. Reduced motion applies wheel changes immediately and disables coasting. Focus loss, pane resizing, projection changes, externally placed cameras, and a new pointer press cancel pending motion. A fresh wheel event after reactive sleep still starts normally; holding a drag still before release does not throw the map. Diagonal scrolling retains both its pan and zoom components.

Raster and vector detail selection have a 0.3-zoom dead band around rounded level boundaries. Large moves still change detail promptly. Cold requests prioritize a small coarse backdrop before fine streets, and the parent/child geography and label fallback remains in use. Globe's existing conservative tile-count cap is applied after detail selection.

Returning labels retain priority during camera movement. Label entry and exit have separate zoom thresholds, and optional geography text has a stable limit of 192 layout candidates per pane. That limit does not shrink when a gesture starts. Nearby duplicate route shields are rejected before layout, and shields can spend only half the allowance so town names retain space. City names remain eligible during movement. Weather warnings retain their separate priority pass. Collision rectangles reset for each clipped pane while frame visibility history is retained, avoiding cross-pane suppression and priority-order assertion failures.

Tile and vector-buffer uploads share a budget across the frame: two items and 2 MiB during movement; eight items and 8 MiB otherwise. One oversized first item may proceed to prevent starvation. Unspent work stays queued, stale vector generations are rejected, and another frame is requested. Gesture upload turns rotate across visible desktop panes; compact layouts can spend the budget on their sole visible pane. MVT decoding/tessellation remains in background workers, and camera motion keeps zoom-driven overlay retessellation deferred until settling. New overlay data still refreshes immediately.

## Measurement

`scripts/web/map-motion.mjs` records original wheel events, camera/frame traces, tile uploads, optional text work, pane UI cost, and WebGL command counts. It uses a historical KDMX sweep under synthetic catalog names and captures OpenFreeMap payloads once for subsequent replay. The scenarios cover a prewarmed route, delayed tile responses, independent and linked quad views, and reduced motion. Separate checks cover drag coasting, direct touch pinch, cursor anchoring, shared upload bounds, and transition screenshots.

Set `globalThis.__hookechoMapTraceEnabled = true` before boot to enable the bounded trace; ordinary sessions do not collect it. The local test buffer can contain camera coordinates. It is separate from user-facing diagnostics and is never automatically transmitted.

Trace frame rows are `[time_ms, pane, frame, zoom, center_x, center_y, width, height, scroll, pinch, moving, tile_zoom, vector_uploads, raster_uploads, label_candidates, preparation_ms]`. Optional cost rows are `[time_ms, pane, frame, geography_layout_candidates, pane_ui_ms]`. Each array is capped at 4096 observations and omissions are counted. Preparation time precedes painter overlays; pane UI time includes them. Neither is GPU completion or display latency.

Whole-window frame intervals include intentional reactive idle sleeps. Use `movingP95` for camera movement; do not interpret whole-window p95 or idle Mapbox render-event gaps as dropped-frame rates. The prewarmed route can still receive tiles. Comparisons are between HookEcho builds on the same machine and recorded scenes, not a RadarOmega speed ranking.

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
