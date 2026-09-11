# Radar reliability implementation progress

This is an implementation checkpoint, not a release-completion report.

## Implemented locally

- Local cell association no longer indexes beyond matching flags when multiple new cells arrive.
- Local motion extrapolation rejects invalid coordinates, motion values and single-observation tracks.
- Local-track cache keys include available historical frames, so later decoded history refreshes estimates.
- Vector uploads are drained in batches of two; remaining uploads request another frame.
- Debug logs distinguish tile fetch/cache time from processing time.
- Tropical public advisories are the initial product; text windows use the shared glass frame,
  bounded immediately visible scrolling, and a narrow-screen sheet. Changing products clears old text.
- Visible-map speech has a separate session ledger, expiration checks, archive suppression, and
  material-update deduplication. Existing banner and push location selection remains independent.
- Browser speech supports explicit activation, volume, error reporting, utterance retention,
  emergency cancellation and a stop control. Official warning cards offer full-bulletin speech.
- Debian and AUR metadata require espeak-ng; Piper remains optional.

## Remaining before claiming the agreed plan complete

- Historical SCIT/local-track accuracy evaluation, time-aligned source fallback, stale history,
  evidence-derived uncertainty corridors and arrival ranges.
- Progressive coarse-to-fine basemap scheduling, radar-first worker prioritization, adaptive quality
  controls, obsolete-job cancellation, adjacent prefetch and combined 250 MB browser/native cache.
- Full startup/network/cache/decode/tessellation/GPU/label timing separation and cold/warm benchmarks.
- Visual verification of every tropical surface, narrow screens, and long official products.
- Cancellation of expired/superseded speech already queued, device speech verification, native
  process interruption, and remaining voice readiness/recovery checks. Current native stop is
  between sentences; browser stop cancels the active utterance.
- Speech scope currently uses the same bounding-box overlap as the existing alert panel; exact
  polygon intersection and antimeridian handling still need validation.
- Packaged AppImage voice handling and audible tests on Windows, macOS, Linux and Android.
- Website documentation, final production builds, GitHub publishing and public-deployment checks.

## Deferred by the user

The Acer Chromebook Spin 311 CP311-2H-C679 (Celeron N4020, 4 GB RAM, ChromeOS)
benchmark is deferred. Neither two-second startup nor 30 FPS on that hardware has been verified.

## Checks at this checkpoint

- HookEcho library suite: 394 passed, 10 ignored before the final browser callback changes.
- Cell tracking suite: 7 passed.
- Native and WASM library checks passed; WASM reports existing unused-code warnings.
- These checks do not establish audible output, historical forecast accuracy or performance targets.
