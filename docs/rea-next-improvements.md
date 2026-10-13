# Audit follow-up implementation and verification

Scope: the seven recommendations made after the RadarOmega/REA performance audit. Production source is frozen at `c2a8858`; later changes improve tests and evidence. The corresponding browser bundle is `ee5a733c` (4,407,232 gzip bytes under the 4,410,000 gate).

| Requirement | Implementation and evidence |
|---|---|
| Consistent units | Desktop/phone field legends and native field probes share the physical conversion table. Temperature contours and objective-analysis readings use selected temperature units; residuals use scale without an offset. Wind preferences are independent. Exports retain labeled native scientific values. Temperature, wind, hail, visibility and delta tests pass. Browser infrared legends were inspected. |
| Pane independence | Vector managers, label caches and GPU geometry are pane-specific. Native ABI scenes, requested times, request generations, frames, textures, probes and exports are pane-specific; raster satellite selection remains independent. GPU tests prove same-coordinate resources and eviction stay isolated. Browser fixture scenes show distinct identities; basemap cycling changes one pane while peer screenshot pixels remain unchanged. |
| Source/time inspector | Each enabled gridded layer discloses available source identity, data class, valid/issue/run/receipt times, forecast lead, signed age and offset from displayed radar. Alignment-withheld and missing metadata are explicit. Shared weather overlays expose request/receipt health and disclose the absence of a common valid clock. Receipt is not presented as observation time. Native time-format tests and browser inspector checks pass. |
| Notification updates | Linux summaries reuse their daemon-assigned notification ID; browser summaries have a dedicated replacement tag while individual alerts remain separate. Rolling summaries expire old titles/counts instead of accumulating across windows. An isolated Linux bus and a separately compiled browser API contract harness pass. macOS/Windows/Android retain their existing fallback behavior where the current adapter does not support replacement. |
| Stable marker editors | Map popup and manager resolve persistent marker IDs; editor fields and cards use those IDs for widget state. Regression tests cover duplicate names, reorder/removal and actual egui pointer/keyboard editing after an earlier marker is deleted. |
| Sustained-session validation | **Final three-hour run in progress.** It alternates two/four populated reflectivity panes, R8/R16, smoothing and a globe pane; exercises station changes, simulated transport failures and actual background/foreground transitions. It samples browser cadence, heap, aggregate process RSS, overlapping requests and recovery/navigation events. Preliminary evidence is retained separately. |
| Recovery persistence | Separate bounded checkpoints save unsaved layout, appearance, camera, thresholds and playback state. Archive, forecast, replay and cut cursors wait for available listings/decode. Cut restoration validates station/volume/product/tilt; ordinary volume changes still clear old selections. Native edge-case tests and actual Chromium WebGL context-loss checks pass for ordinary panes, forecast lead and cut playback. |

Workspace suites pass: 577 HookEcho tests and 426 wxdata tests, plus other crates. Explicit GPU/platform cases are skipped by ordinary tests. Eleven native GPU checks pass, including the 48-case image-equivalence/performance matrix. Warnings-denied workspace Clippy passes. All seven CI checks passed on `1d28c89`, which replaces a process-wide-counter cache assertion with local retained-allocation checks.

The reusable sustained harness is `scripts/web/soak.mjs`; it requires Puppeteer and a Chromium executable. Serve the built `web/` directory, then run:

```sh
CHROME_BIN=/usr/bin/chromium HOOKECHO_BASE_URL=http://127.0.0.1:8765/ HOOKECHO_SOAK_SECONDS=10800 node scripts/web/soak.mjs
```

`HOOKECHO_SOAK_OUTPUT` selects the result directory (default `outputs/soak`). A short duration validates harness actions; it does not replace sustained testing.

The radar test uses a historical sweep under synthetic timestamp/station request names. Satellite tests use the repository GOES fixture under synthetic scene identities. Empty vector tiles isolate palette rendering. Software-rendered browser cadence and process RSS are scoped fixture measurements, not live-feed latency or GPU draw-time claims. The browser notification harness records API arguments; it does not deliver real OS notifications.

Completion remains unproven until the final sustained run, final CI and requirement-by-requirement evidence review finish.
