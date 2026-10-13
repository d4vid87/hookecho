# Remaining audit recommendations

Scope: complete the seven recommendations made after the performance audit, with current-source verification and runtime evidence where needed. Existing features count only when inspected and verified; a source comment alone does not establish a missing feature.

| Requirement | Current evidence / remaining work |
|---|---|
| Consistent units | Temperature contours already convert through `ContourKind::to_display`; gridded legends still expose Kelvin. Audit legends, probes, exports and other physical-unit displays. |
| Pane independence | Raster satellite selection is per-pane. Vector palettes and native ABI fields need independent selections and rendering ownership. |
| Layer source/time inspector | Extend existing source metadata and satellite clock disclosure across visible weather layers; distinguish observed and forecast time, age, and radar offset. |
| Notification summary updates | Rollup throttles summaries but native delivery creates new cards. Implement replacement where supported without grouping urgent events into summaries. |
| Stable marker editors | Map popup and manager now resolve existing persistent marker IDs each frame. Regression test covers duplicate names, reorder and removal; native test passed. Further UI verification pending. |
| Sustained-session validation | Run long two/four-pane animation, switching, background and interruption scenarios; record memory, dropped frames, network duplication and recovery. Controlled GPU matrix does not prove this requirement. |
| Restore work on recovery | Inspect current automatic persistence, preserve unsaved pane state and playback position across graphics reload, verify real reload behavior. |

Implementation remains in progress. The prior PR's green CI does not validate these new changes.
