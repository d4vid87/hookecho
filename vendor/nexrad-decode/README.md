# NEXRAD Decode

HookEcho vendors upstream `nexrad-decode` 1.0.0-rc.3 (MIT) to pass measured
Nyquist velocity from Message 31 RAD blocks and Message 1 bytes 60–61 into the
patched model. The instrument value is in hundredths of a metre per second;
missing/non-positive metadata stays unknown. Upstream examples, test fixtures
and development dependencies are omitted here. Synthetic binary and worker-wire
regressions live in HookEcho's `wxdata/tests/nyquist_metadata.rs`.

[![Crates.io](https://img.shields.io/crates/v/nexrad-decode)](https://crates.io/crates/nexrad-decode)
[![Docs.rs](https://docs.rs/nexrad-decode/badge.svg)](https://docs.rs/nexrad-decode)
[![Rust CI](https://github.com/danielway/nexrad/actions/workflows/ci.yml/badge.svg)](https://github.com/danielway/nexrad/actions/workflows/ci.yml)

Decoding functions and models for NEXRAD weather radar data. Decoder and struct definitions are in accordance with
NOAA's WSR-88D Interface Control Document for the RDA/RPG "ICD 2620002AA".

## Features

- `nexrad-model`: Provides mappings to a common model for representing NEXRAD radar data.
- `uom`: Use the `uom` crate for type-safe units of measure.
