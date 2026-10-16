# Everyday radar workflows

This follow-up implements the eight recommendations from the reference app interface comparison. It refines HookEcho's existing Radar and Analyst surfaces, action registry, glossary, favorites, recovery and optional tour.

1. The browser installation suggestion has a separate dismissal control and 44-pixel buttons. On narrow screens it sits above playback and navigation, centered away from the color legend, rather than covering a navigation destination. It remains a temporary browser-install suggestion.
2. Station, city, full radar product name and elevation stay together on desktop and phone. Station selection uses the existing picker; product selection uses the shared action path; elevation selection preserves the existing manual-cut behavior. During a station transition the header identifies the prior station whose scan remains displayed.
3. Playback status is explanatory. History has a visible button, and Return to live appears when browsing older data. Both reuse the existing calendar and live-head behavior, including the Android Radar surface.
4. Navigation consistently uses Radar, Layers, Alerts and Tools across desktop, phone, the Android shelf and the tour. Internal action IDs and saved workspace formats remain compatible.
5. Catalog rows have independent favorite and Details buttons. Details exposes descriptions, shortcuts, glossary explanations and source freshness without toggling the layer. Native radar moments, including storm-relative velocity, can also be favorited in the product selector. Favorites use stable keys in the existing persisted collection; the selector groups them before other available products.
6. Pane selection shows product and station names. Editing controls identify the active pane and map/time linking. Products and elevations remain pane-local; station selection explicitly identifies its existing linked-quad scope.
7. An unresolved radar error remains accessible until recovery. The issue panel identifies the requested station and pane, offers Retry and Change station, and retains eight recent events in session memory. Repeated unchanged errors do not fill the history. Resolved history can be cleared; old history entries cannot target a closed or replaced pane.
8. Location-based first-run setup offers a small optional quick-guide prompt after selecting the nearest radar. Start quick guide uses the existing tour; Not now or closing the prompt dismisses it. The prompt does not force a tour or request additional location access.

## Verification

Use the historical-radar browser fixtures to verify favorite persistence without product changes, independent Details actions, archive/live transitions, issue persistence and retry, descriptive panes, optional onboarding, and phone installation geometry. Inspect the rendered desktop and compact layouts; viewport simulation is not a physical-device or human usability study.

The main compressed WebAssembly allowance increases deliberately from 4,440,000 to 4,450,000 bytes for these controls. The optimized build checks this allowance before publishing its bundle. The lite allowance and startup timeout remain unchanged. The local native suite passes 604 tests (22 ignored), and warnings-denied workspace Clippy passes. Browser interaction results and screenshots are recorded with the PR.

Browser verification covers install placement and dismissal at 390×844 and 320×568, favorite persistence without selecting another product, independent Details actions, historical playback and Return to live, descriptive panes, retained-scan recovery, and the optional location-based guide. In the controlled fixture, Retry started a fresh listing 8 ms after a click and fetched a replacement volume when connectivity returned; this is not a live-service latency measurement. Older archive days remain archive when playback, stepping or scrubbing reaches their last frame. The category rail and tool drawers follow the measured toolbar height, including its retained-scan explanation. Drawer headers, filter fields and close controls stay below the grouped navigation on desktop and browser-phone layouts.

The repeatable phone check is `scripts/web/usability.mjs`, configured with `HOOKECHO_BASE_URL`, `CHROME_BIN` and `HOOKECHO_USABILITY_OUTPUT`. CI runs this alongside browser smoke and reliability checks and retains both reports and screenshots.
