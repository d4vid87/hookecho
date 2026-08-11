# Fuzz targets

Five libFuzzer targets over the decoders that eat bytes from the outside world: Level 2 volumes,
Level 3 products, placefiles, HDOB bulletins, and `.pal` colour tables.

GRIB2 is deliberately **not** a target. Production decodes it inside `catch_unwind` because
gribberish's templates panic freely on malformed input, and libFuzzer aborts on any panic no
matter what the target catches — so the job would report upstream template bugs nightly and teach
everyone to ignore it. A throwaway GRIB target was still worth running once: it found the two
hangs fixed in this same change (see below), which no `catch_unwind` would have saved us from.

```
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run level3_decode -- -max_total_time=60
```

Not a workspace member — cargo-fuzz needs nightly and its own profile, and `cargo test
--workspace` should not try to build it. Because an excluded crate is its own workspace root, this
manifest **duplicates the root `[patch.crates-io]`**; without that it would fuzz upstream
gribberish and nexrad-data instead of the vendored forks the app actually ships.

`seeds/` holds the starting inputs (the committed Level 3 fixtures and colour tables); copy them
into `corpus/<target>/` before a run, as CI does. `corpus/` and `artifacts/` are run output and
are not committed. `.github/workflows/
fuzz.yml` runs every target for two minutes nightly and uploads any crashing input.

Found so far:

* `level2::decode_volume` panicked on any buffer shorter than the 24-byte volume header, which the
  live head really does serve when an object is half-written. Fixed with a length guard.
* `mrms::decode_grib2` ground for minutes on a 28-byte message that declared a huge length. Fixed
  by checking the declared length against what we hold.
* `recon::parse_hdob` panicked on a byte slice across a multi-byte character, twice — in the wind
  group and in the coordinate split. Both now slice by character.
* `vendor/gribberish`'s section iterator spun forever on a zero-length section (the offset never
  advanced). Fixed there, marked `hookecho patch:` like the other vendor fixes.
