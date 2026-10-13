# Reliability follow-up

Scope: implement all six recommendations following the completed REA integration audit.

| Requirement | Implementation | Current verification |
| --- | --- | --- |
| Shared identical downloads across panes | Weak shared-flight pools for catalogs, radar volume bytes, map tiles and tile metadata; independent consumer cancellation; bounded bookkeeping; aggregate diagnostic counts | Shared allocation/cancellation unit tests and browser shared-download counters pass |
| Predictable memory release | Closed-pane CPU managers and GPU radar/vector/field/wind/timer resources released; soft encoded-payload caps preserve displayed scans; binned retention bounded across recent volumes; cache eviction/high-water metrics | Eight native GPU close/reopen cycles pass without resource growth; new binned-budget regression passes; browser allocation comparison passes |
| Seamless station switching | Previous scan remains explicitly identified by station and timestamp; generation checks and abort handles for head/list/frame work; dropped panes cancel work; unique generations survive pane recreation; archive-list failures retry with backoff | Station retention, pane-recreation and browser outage/recovery assertions pass; visual inspection caught an occluded caption, now corrected in source and awaiting rebuilt rendering verification |
| Undo workspace actions | Bounded 16-entry/2 MiB local history restores closed panes and replaced workspaces with camera/appearance/playback state; both marker deletion surfaces retain stable identity for Undo; accessible command-palette action | Marker restoration and actual Remove-place pointer-event regression pass; browser closed-pane and workspace Undo assertions pass |
| Explanatory timeline | Per-frame ready/downloading/decoding/unavailable/skipped strip, hover descriptions and selected-frame accessibility label; bounded failure history; successful loads clear stale failures | Frame history, browser ready/unavailable/skipped assertions and rendered availability-strip inspection pass |
| Reproducible diagnostic reports | v2 includes public pane controls, station/product identities, source/playback clocks, bounded 64-event interaction history and aggregate resource/download/Undo counts; private coordinates/names/keys/paths/raw errors excluded | Privacy regression and actual exported-browser JSON/history assertions pass |

The current workspace run passed 584 HookEcho and 428 wxdata tests, plus other workspace suites. The initial native Clippy run passed with warnings denied. A subsequent HookEcho run passed 583 tests including the binned-budget and marker-restoration regressions. Later source refinements require rerunning the complete checks before completion.

The first optimized browser build measured 4,428,056 gzip bytes against the prior 4,410,000-byte budget and failed that gate. Sharing the async payload type reduced a subsequent build to 4,427,028 bytes. Binaryen Oz and similar-function merging measured larger outputs and were not adopted. The follow-up deliberately allocates 30 KB (0.68%) above the prior main-bundle cap: 4,440,000 bytes. The verified build before the final caption correction measured 4,427,108 bytes; lite remains 74,040 bytes under its unchanged 80,000-byte cap, and WebGL startup passed in 3.8 seconds under the unchanged 45-second limit. The first browser harness trial booted and read the pane report, but correctly failed when it encountered the earlier bundle without the latest shared-download counters; it is not completion evidence.

Native GPU regression: `gpu_closed_panes_release_allocations_and_reopen_without_growth` exercises radar textures, upload shadows and vector geometry across eight 4→2→4 cycles. It proves nominal resource release and reuse for those resources; it does not prove browser RSS shrinks or measure driver VRAM.

Browser regression entry point: `scripts/web/reliability.mjs`. Fixtures use historical KDMX sweep bytes under synthetic station/time names. No live-feed latency or real-device performance claim is made.

The previous audit and CI results apply to the earlier implementation, not this follow-up. Completion remains unproven until current-source builds, tests, browser interactions, rendering, download-size gates and external PR checks pass.

The browser assertions passed all eight checks before visual review: shared downloads, ready frames, unavailable/skipped frames, closed-pane allocation reduction, closed-pane Undo, station continuity, workspace Undo and diagnostic privacy/history. Visual review then showed the transition caption was painted before the opaque map callback and was invisible; the corrected source paints it after the callback and replaces the Live badge during a pending station transition. This final correction must be verified in the rebuilt browser before completion.

CI now runs the reusable browser reliability scenario after the existing smoke checks and uploads its report/screenshots even on failure. The harness closes the command menu before sending playback hotkeys, so focused text widgets cannot swallow Space.

Further verification found that decoded scan readiness alone could mask blank panes: the UI had queued uploads without the renderer preparing them. Radar uploads now carry acknowledgement tokens; native and custom-product cache hits require the renderer's matching token, and evicted/clipped uploads are retried in full. A new GPU regression verifies queued-versus-prepared state, acknowledgement removal on pane closure, and full reupload after a LUT-only update cannot restore an evicted texture. The browser scenario now requires matching requested/prepared tokens in every pane before allocation comparisons.

Inspector, probe and diagnostics provenance use the station of the displayed scan rather than a still-pending station selection. Empty panes report no displayed station. The transport's duplicate source label was removed to avoid overlap; each map retains its source caption and pending-station badge.

The renderer-acknowledgement build measured 4,428,702 gzip bytes under the documented 4,440,000-byte cap. Its current browser and complete-suite verification remain pending.
