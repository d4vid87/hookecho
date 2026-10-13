# REA follow-up implementation

Scope: implement all six recommendations following the initial REA integration.

- Performance: cross-platform frame/decode/upload/cache measurements and automatic quality with hysteresis, preserving saved user preferences.
- Freshness: live probe age and optional stale-sector fade, preserving measurement values.
- Timing: visible linked-pane offsets and configurable maximum acceptable offset.
- Presets: saved named appearance presets and copying appearance to another pane.
- Precision: finer custom-product inputs and output, with missing data retained.
- Globe: improved horizon tessellation, alignment and local/global transitions.

Completion requires focused logic tests, native and wasm builds, appropriate GPU/browser checks and an updated reviewable PR. Existing correctness checks are not performance benchmarks.

Implemented locally: custom-product precision path; explicit live probe age; saved named appearance presets and pane copying; linked scan offsets and a configurable inclusive time limit. Offsets use the loaded scan, so a pending fetch cannot disguise an older scan as synchronized.

Verified so far: seven product tests, five appearance tests and native library checks. Full follow-up verification remains pending. Automatic performance tuning, stale-sector fade and globe polish remain incomplete.

Performance work in progress: bounded UI CPU p95, automatic extra-smoothing reduction/recovery with hysteresis, browser/native counters, radar texture byte counts and decode-path wall times. GPU duration, memory readout, adaptive basemap quality and representative performance benchmarks remain to be implemented/verified.
