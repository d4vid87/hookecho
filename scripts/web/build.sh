#!/usr/bin/env bash
# Build the browser bundle into web/dist, then serve it with:
#
#   cargo run --release -- --serve 8080 --web-root web
#
# ponytail: plain wasm-bindgen CLI, no trunk — `--serve` already covers the dev-server role, and
# this is one command with no extra config file. Bring in trunk if a hot-reload loop is ever wanted.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

if ! command -v wasm-bindgen >/dev/null; then
  echo "wasm-bindgen not found. Install it with:" >&2
  echo "  cargo install wasm-bindgen-cli" >&2
  exit 1
fi

# getrandom 0.3 needs this to pick its browser backend; the feature alone isn't enough.
export RUSTFLAGS="${RUSTFLAGS:-} --cfg getrandom_backend=\"wasm_js\""

cargo build --release --target wasm32-unknown-unknown -p hookecho --lib
wasm-bindgen --target web --no-typescript \
  --out-dir web/dist --out-name hookecho \
  target/wasm32-unknown-unknown/release/hookecho.wasm

echo "web/dist ready:"
ls -la web/dist
