#!/usr/bin/env bash
# Build the Debian package of HookEcho.
#
# cargo-deb does the packaging; this script only does the two things it can't: build the release
# binary and render the icons out of it (`--headless-icon`), which is why no sized PNG is committed. The
# asset list lives in `crates/hookecho/Cargo.toml` under `[package.metadata.deb]`.
#
# Usage: packaging/build-deb.sh
# Output: target/debian/hookecho_<version>_amd64.deb
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> building release binary"
cargo build --release -p hookecho

echo "==> rendering icons"
mkdir -p target/deb-assets
for px in 128 256; do
  ./target/release/hookecho --headless-icon "target/deb-assets/hookecho-${px}.png" "$px"
done

echo "==> packaging"
# --no-build: we just built it, and cargo-deb's own build would not reuse the profile above.
cargo deb -p hookecho --no-build

ls -l target/debian/*.deb
