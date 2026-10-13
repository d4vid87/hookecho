# Remaining audit recommendations

Scope: complete the seven recommendations made after the performance audit, with current-source verification and runtime evidence where needed. Existing features count only when inspected and verified; a source comment alone does not establish a missing feature.

| Requirement | Current evidence / remaining work |
|---|---|
| Consistent units | Temperature contours already convert through `ContourKind::to_display`; desktop and phone gridded legends now convert temperature and wind labels to selected units without changing native values or LUT colors. Regression test passed. Native field probes and map-hover values now follow the independent velocity setting instead of deriving wind units from the temperature preference; regression test passed. Exports retain explicitly labeled scientific units. Other physical-unit displays remain to audit. |
| Pane independence | Raster satellite selection is per-pane. Vector manager, label cache and GPU geometry are now pane-local; same-coordinate GPU test verified independent colors and that changing one pane leaves the other resident style intact. Native ABI fields still need independent selections and rendering ownership. Browser style-switch verification pending. |
| Layer source/time inspector | Inspector now lists every enabled gridded layer with available scalar/RGB metadata: source identity, observed/analysis/forecast/derived class, full UTC valid time, receipt age, signed valid age, issue time, run and forecast lead, and offset from displayed radar; alignment-withheld and missing metadata are explicit. Pure time-format test passed. Overlay feeds without common stamps and runtime UI verification remain to audit. |
| Notification summary updates | Rollup throttles summaries but native delivery creates new cards. Linux delivery reuses the notification daemon ID; browser summaries use a dedicated replacement tag, while individual alerts are untagged. macOS/Windows/Android retain current fallback behavior. Delivery verification remains. |
| Stable marker editors | Map popup and manager now resolve existing persistent marker IDs each frame. Regression test covers duplicate names, reorder and removal; native test passed. Further UI verification pending. |
| Sustained-session validation | Run long two/four-pane animation, switching, background and interruption scenarios; record memory, dropped frames, network duplication and recovery. Controlled GPU matrix does not prove this requirement. |
| Restore work on recovery | Added a separate bounded crash/reload checkpoint for layout, appearance, archive time, disabled thresholds and playback controls. Native snapshot roundtrip test passed; actual browser reload verification remains. Explicit startup view is honored except graphics recovery reload. Checkpoints update at most once a second and flush on normal exit. |

Implementation remains in progress. The prior PR's green CI does not validate these new changes.

Validation checkpoint: native HookEcho library tests passed (564 passed, 17 explicitly ignored GPU/headless cases). This does not prove browser, OS delivery, or sustained-session behavior.
