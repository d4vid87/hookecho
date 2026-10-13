# Remaining audit recommendations

Scope: complete the seven recommendations made after the performance audit, with current-source verification and runtime evidence where needed. Existing features count only when inspected and verified; a source comment alone does not establish a missing feature.

| Requirement | Current evidence / remaining work |
|---|---|
| Consistent units | Temperature contours already convert through `ContourKind::to_display`; desktop and phone gridded legends now convert temperature and wind labels to selected units without changing native values or LUT colors. Regression test passed. Audit probes, exports and other physical-unit displays remains. |
| Pane independence | Raster satellite selection is per-pane. Vector palettes and native ABI fields need independent selections and rendering ownership. |
| Layer source/time inspector | Extend existing source metadata and satellite clock disclosure across visible weather layers; distinguish observed and forecast time, age, and radar offset. |
| Notification summary updates | Rollup throttles summaries but native delivery creates new cards. Linux delivery reuses the notification daemon ID; browser summaries use a dedicated replacement tag, while individual alerts are untagged. macOS/Windows/Android retain current fallback behavior. Delivery verification remains. |
| Stable marker editors | Map popup and manager now resolve existing persistent marker IDs each frame. Regression test covers duplicate names, reorder and removal; native test passed. Further UI verification pending. |
| Sustained-session validation | Run long two/four-pane animation, switching, background and interruption scenarios; record memory, dropped frames, network duplication and recovery. Controlled GPU matrix does not prove this requirement. |
| Restore work on recovery | Added a separate bounded crash/reload checkpoint for layout, appearance, archive time, disabled thresholds and playback controls. Native snapshot roundtrip test passed; actual browser reload verification remains. Explicit startup view is honored except graphics recovery reload. Checkpoints update at most once a second and flush on normal exit. |

Implementation remains in progress. The prior PR's green CI does not validate these new changes.

Validation checkpoint: native HookEcho library tests passed (564 passed, 17 explicitly ignored GPU/headless cases). This does not prove browser, OS delivery, or sustained-session behavior.
