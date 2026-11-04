# NEXRAD Model

HookEcho vendors upstream `nexrad-model` 1.0.0-rc.2 (MIT) to retain measured
Nyquist velocity on each radial, including serde transfers to the decode worker.
The only runtime change is the optional metadata field and its validated accessor/builder.
Upstream examples, test fixtures and development dependencies are omitted here;
HookEcho's `wxdata/tests/nyquist_metadata.rs` checks the patched binary-to-model path.

[![Crates.io](https://img.shields.io/crates/v/nexrad-model)](https://crates.io/crates/nexrad-model)
[![Docs.rs](https://docs.rs/nexrad-model/badge.svg)](https://docs.rs/nexrad-model)
[![Rust CI](https://github.com/danielway/nexrad/actions/workflows/ci.yml/badge.svg)](https://github.com/danielway/nexrad/actions/workflows/ci.yml)

A common model for representing NEXRAD weather radar data. Provides an ergonomic API which is documented for an audience
who is not necessarily familiar with the NOAA Archive II format.

## Features

- `uom`: Use the `uom` crate for type-safe units of measure.
- `serde`: Implement `serde::Serialize` and `serde::Deserialize` for all models.
- `chrono`: Use the `chrono` crate for date and time types.
