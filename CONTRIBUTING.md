# Contributing

Bug reports, feature requests and patches are all welcome. This file is the
short version of how the code is put together and what a change is expected to
clear before it lands.

## The workspace

| Crate | What it is |
| --- | --- |
| `crates/hookecho` | The app: egui UI, wgpu render pipelines, settings, the desktop binary and the Android `cdylib`. |
| `crates/wxdata` | Data plumbing: every feed the app decodes, and nothing that draws. See [docs/DATA.md](docs/DATA.md). |
| `crates/nexrad-level3` | From-scratch NEXRAD Level 3 (RPG) product decoder. |
| `crates/hdf5lite` | Minimal read-only HDF5 reader, enough for NOAA's netCDF-4 granules. |
| `vendor/gribberish` | Vendored GRIB2 decoder with local fixes; grep `hookecho patch:`. |

Rule of thumb: if it fetches or decodes, it belongs in `wxdata`; if it draws or
holds UI state, it belongs in `hookecho`.

## The gate

Every commit is expected to pass:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Both are clean on `main`, so a warning that appears is yours.

## Tests

Unit tests live beside the code in `#[cfg(test)] mod tests`. Parsers are tested
against a `const SAMPLE` fixture — a real, trimmed response from the actual
service, not a hand-written ideal.

Anything that touches the network is marked `#[ignore = "network"]` so
`cargo test` stays offline and deterministic:

```rust
#[test]
#[ignore = "network"]
fn live_fetch_works() { … }
```

Run those deliberately with `cargo test -- --ignored`.

## Headless verifiers

Every data-backed feature has a `--headless-*` mode that renders a PNG or prints
a report without opening a window (see the Verification section of the README).
They are the smoke tests: when you add a feed, add its verifier, and when you
change a decoder, run the existing one.

## The Android field-name contract

The Android background alert service
(`android/app/src/main/kotlin/.../AlertService.kt`) parses `settings.json` by
hand, in another language, with no compiler to catch a rename. The test
`kotlin_alert_service_field_names_survive` in `crates/hookecho/src/settings.rs`
is that compiler — if it fails, the Kotlin has to change with your rename.

Settings are also read by that service while the app is running, so per-frame
writes to `settings.json` are avoided; the app coalesces them into a one-second
dirty-diff save.

## Commits

Conventional commits — `feat(scope): …`, `fix(scope): …`, `docs: …`,
`refactor: …`, `build: …`. The body says what was broken or missing and why the
change is shaped the way it is; the diff already says what it does.

## `ponytail:` comments

A deliberate simplification carries a `// ponytail:` comment naming its ceiling
and the upgrade path:

```rust
// ponytail: first station only; a nearest-N picker can follow if the closest is stale.
```

That is a ledger, not an apology — `grep -rn "ponytail:" crates/` is where
[ROADMAP.md](ROADMAP.md) gets restocked. If you lift one of those ceilings, take
the comment with it.

## Data sources

New feeds should be keyless where a keyless source exists; where a key is
required, the layer stays empty and says so rather than nagging. Keys live in
the user's own settings file and never in the repository — including in the
screenshot harness's committed settings template, which `scripts/shots/shoot.sh
check` verifies. Respect each service's terms: some publicly reachable feeds
(Blitzortung, for one) forbid app use, and those stay out no matter how good the
data is.
