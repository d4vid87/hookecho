# Radar reliability implementation progress

This records the implementation and the acceptance checks that require external hardware.

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
- Shared-worker scheduling places radar ahead of queued vector refinement. The basemap requests a
  coarse backdrop before full streets, adapts while moving, prefetches a small margin only after
  the view settles, and bounds GPU upload batches.
- Map storage is capped at 250 MB combined, and the basemap panel offers Auto, Performance, Full
  quality and Clear map cache controls.
- SCIT projections are aligned to the displayed scan, limited to 30 minutes, and suppressed when
  stale, malformed, or missing source error information. Source error drives corridors and arrival ranges.
- Visible-map speech uses exact polygon/viewport intersection and is disabled during archive playback.
- The AppImage bundles espeak-ng and its voice data; Debian and AUR declare it as a dependency.

## Acceptance checks not reproducible on this host

- Audible output on physical Windows, macOS and Android devices. Automated builds exercise their
  code paths, but this host cannot hear those devices.
- Native desktop emergency interruption occurs at sentence boundaries because system command-line
  speech APIs expose no portable cancellation handle. Browser and Android active utterances cancel.
- Quantitative accuracy comparison against a curated historical SCIT archive. No such fixture is
  stored in the repository; deterministic unit and scan-time tests cover the implemented invariants.
- Two-second startup and 30 FPS targets on the Chromebook named below.

## Deferred by the user

The Acer Chromebook Spin 311 CP311-2H-C679 (Celeron N4020, 4 GB RAM, ChromeOS)
benchmark is deferred. Neither two-second startup nor 30 FPS on that hardware has been verified.

## Checks

- `cargo test --workspace`: 397 HookEcho tests passed, 10 ignored; all workspace suites passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Browser worker tests: 17 passed.
- Production web build: main WASM 3,977,688 bytes gzip (4,085,000-byte budget); lite WASM
  72,109 bytes gzip (80,000-byte budget).
- Website build and the 738-link advisory check passed.
- The x86_64 AppImage release build passed with bundled espeak-ng.
- Public deployment checks are recorded by the Demo and Site GitHub Actions runs.
