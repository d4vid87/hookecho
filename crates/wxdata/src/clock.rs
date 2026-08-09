//! A monotonic clock that also exists in a browser.
//!
//! `std::time::Instant::now()` panics on `wasm32-unknown-unknown` — the target has no clock, and
//! the standard library's answer is "time not implemented". That is a runtime panic, so
//! `cargo check --target wasm32-unknown-unknown` never sees it; the web build died on frame one
//! because a frame counter called `Instant::now()`.
//!
//! `web-time` is already in the lockfile (eframe pulls it) and re-exports the standard library
//! types verbatim on every native target, so this is a compile-time alias and nothing else: on
//! desktop and Android the generated code is identical to what it replaced.
//!
//! Use these in anything the web build compiles. `Duration` is pure arithmetic and needs no
//! substitute; `SystemTime` from `std` is still the right type for filesystem timestamps, which
//! only exist where there is a filesystem.

#[cfg(target_arch = "wasm32")]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(target_arch = "wasm32"))]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};
