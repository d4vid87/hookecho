# Analyst UI and application audit

Baseline: `3c1975a` (2026-09-22). This is a reproducible defect ledger, not a claim that passing unit tests proves every live provider or GPU is healthy. The current preview branch fixes the first two navigation issues below; new findings are added with a trigger and evidence before changing code.

| Priority | Area | Trigger and finding | Resolution / gate |
|---|---|---|---|
| P0 | Navigation | Opening the floating panel hid the menu/search pill; the panel was the only path back to that control. A drawer could also hide the panel while `panel_open` remained true. | Keep menu/search above the panel; keep the drawer's Back/Close header visible; test open/close, resize, and workspace restore. |
| P1 | Layout | The floating panel and right-side analysis surfaces could overlap the playback strip, leaving little usable map in multi-pane views. | Analyst docks reserve map width and center the transport in the remaining viewport. Verify 390, 780, and 1440-point layouts. |
| P1 | Settings | Users had to navigate through an optional-settings category to reach the full settings page. | Add a direct Settings action in the workbench; preserve the existing one-click category/footer paths. |
| P1 | Workspace compatibility | Saved chrome uses page titles to reopen drawer pages; title changes can silently drop a restored page. | Read legacy titles, save stable page IDs, and test old/new files before changing page names. |
| P2 | Analyst discovery | Four-pane layouts, linking, probes, provenance, and specialist windows existed but had separate entry points. | Group them in an opt-in docked workbench driven by the existing action registry. |

## Cross-application review matrix

| Domain | Review and regression evidence |
|---|---|
| Radar, MRMS, satellite, models, scientific values | Offline `wxdata` fixtures and native-value/valid-time assertions; inspect missing/categorical handling, projection and accumulation compatibility before promoting a changed adapter. No adapter is changed by this UI series. |
| Requests and caching | Workspace request-generation, cancellation, stale-result, cache integrity and reload tests; preview smoke checks offline/reload and a delayed response while changing pane/product. |
| Renderer and panes | Lavapipe GPU golden, multi-pane source/time checks, zoom/pan and resize captures; ensure a dock never redirects raw map gestures or covers the scrubber. |
| Settings, workspaces, accessibility | Legacy settings/workspace JSON tests, direct Settings action, repeated drawer/panel open-close, keyboard focus, named controls, reduced motion and 48-point touch targets. |
| Platforms | Native Linux/Windows tests, WASM/WebGL smoke, Android arm64 compile, and manual desktop/tablet/phone preview walkthrough. |

## Measured starting point

- Published main web bundle: 11,630,695 bytes raw / 4,338,854 gzip; enforced ceiling 4,345,000 gzip (`scripts/web/build.sh`).
- One cold Chrome run against `https://app.hookecho.io/` on 2026-09-22: app boot 1.34 s, first radar 1.90 s, first volume response 2.00 s, loop playing 4.79 s, 4.09 MB WASM transferred. This is a single software-rendered diagnostic run, not a stable performance threshold. Firefox was unavailable on this host.
- Native frame pacing and resident memory must be captured on release-test hardware via the in-app performance counters before production promotion. The existing `scripts/perf/capture.sh` terminates processes by name, so it is unsuitable while a user-owned HookEcho process may be running.

Promotion blocks on reproducible P0/P1 defects and the existing bundle, CI, preview, and GPU gates. P2 findings remain in this ledger with a trigger and owner rather than becoming speculative changes.
