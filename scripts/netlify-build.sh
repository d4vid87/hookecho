#!/usr/bin/env bash
# Build the browser bundle inside Netlify's build image.
#
# Only used by a git-connected deploy; a CI deploy has already run scripts/web/build.sh and
# uploads web/ as-is. Netlify's image ships rustup but no wasm target and no wasm-bindgen, so
# both get installed here. Expect ~5 minutes cold.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v rustup >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

rustup target add wasm32-unknown-unknown
command -v wasm-bindgen >/dev/null || cargo install wasm-bindgen-cli --locked

scripts/web/build.sh
