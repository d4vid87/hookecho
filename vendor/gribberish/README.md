# gribberish

Read [GRIB 2](https://en.wikipedia.org/wiki/GRIB) files with Rust.

## Getting Started

Add the package

```bash
cargo add gribberish
```

or manually add the package in `Cargo.toml` to `[dependencies]`:

```toml
gribberish = "0.20.1"
```

The following `features` are available:

`png`: Allows unpacking PNG encoded data messages

`jpeg`: Allows unpacking JPEG2000 encoded data messages

By default, both `png` and `jpeg` are enabled.

See [read.rs](tests/read.rs) for example usage for simple reading, or [message-dump](examples/message-dump/main.rs) for an example of dumping grib metadata to stdout.

## Hook Echo-WX local patches

This is a vendored copy carrying two local patches (grep `hookecho patch:`):

1. `src/templates/data_representation/*` — 8-bit PNG unpack fix (MRMS single-byte grids).
2. `src/templates/product/parameters/mrms.rs` — `multiradar_parameter` returns a generic
   placeholder for MRMS categories outside the enumerated set (PrecipFlag, FLASH ARI) instead of
   `None`, so those discipline-209 grids decode. Scoped to MRMS; other disciplines untouched.

## License

[MIT](LICENSE) - 2024 Matthew Iannucci
