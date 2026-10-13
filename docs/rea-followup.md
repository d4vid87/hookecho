# REA follow-up implementation

These are independent HookEcho changes informed by the earlier architectural comparison. No RadarOmega implementation, private service adapter or credentials are included.

## Implemented locally

1. **Performance:** bounded UI CPU and gesture-cadence p95 observations, hysteresis for temporarily disabling extra smoothing and simplifying vector maps in Auto quality, browser/native counters, decode-path wall times, radar texture upload bytes and resident binned-gate buffer memory. Saved preferences are preserved. Only consecutive moving gestures contribute cadence pressure; idle intervals and stationary pointer holds are excluded. Observations are capped at 500 ms. Both CPU work and gesture cadence can trigger reduction. GPU execution time is not measured; cadence includes drawing and scheduling effects. Decode timings include worker scheduling/serialization. Memory is scoped to binned gate buffers rather than whole-process allocation.
2. **Freshness:** live probes show gate age. Optional live-sector fading draws retained prior-scan sectors at 35% opacity; current sectors retain full opacity. Custom products combine freshness across required inputs conservatively. Missing inputs hide the sector. Native measurements and archive playback are unchanged.
3. **Timing:** linked radar panes disclose signed scan offsets and offer an inclusive maximum difference. The check uses the loaded scan/cut rather than a pending playhead. Radar outside the limit is withheld; the pane reports the mismatch.
4. **Presets:** named appearance presets are saved in app settings, support deletion and can be copied to another pane. They include portable palettes, smoothing, ranges and projection, preserving station, product and playhead.
5. **Precision:** gate-local custom products use finer input encodings and produce finer output when precision is enabled. Missing/folded inputs remain missing for product arithmetic. No claim of additional physical radar resolution is made.
6. **Globe:** global views use 16 tile subdivisions per axis, with 8 retained for local views. Both projections support global zoom, avoiding a forced zoom jump when leaving the globe. Picking, overlays and radar continue to use geographic projection. Provider coverage remains about ±85° latitude.

## Verification

- 552 enabled HookEcho library tests passed, including projection, cadence, presets and timing checks.
- 426 enabled wxdata tests passed, including fine custom-product values and missing data.
- Six hardware GPU tests passed: radar golden image, partial/full upload equality, precision switching, globe field/picking alignment and sky clipping, stroke width, and wind.
- The expanded sector-opacity GPU check passed: sectors are independently classified, zero opacity matches a hidden radar, and clearing opacity restores the original pixels.
- A synthetic sparse update submitted 9,272 texture bytes versus 75,073 for a full upload, about 88% fewer bytes. This is a byte-count benchmark, not a wall-clock speed or live-feed latency result.
- Native library checks and workspace Clippy with warnings denied pass. Full workspace tests pass. The most recent optimized bundle before the final cadence/timing refinements measured 4,365,401 bytes gzip; the explicit cap is 4,380,000. Lite remains 74,040 bytes gzip against an unchanged 80,000-byte cap. Final browser verification requires the latest rebuilt bundle.

Final production bundle size, browser interaction checks and PR update remain pending. The six-item goal is not yet verified complete.
