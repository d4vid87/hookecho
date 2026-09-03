#!/usr/bin/env bash
# What is actually in the browser bundle, by item and by generic instantiation.
#
# `build.sh`'s size budget says when the bundle got bigger. It cannot say what got bigger, and
# guessing has been wrong: `env_logger` looked like the obvious win and costs nothing (it is only
# named from `main.rs`, which is not in the cdylib), while `chrono-tz`, which nobody suspected,
# was 119403 bytes gzipped of IANA database for the 25 zones this app names.
#
# This builds its own wasm rather than reading `web/dist`, because the shipped profile has
# `strip = true` and twiggy on a stripped module can only report `code[2199]`. The absolute
# numbers here are therefore larger than what ships; the ranking is the point.
#
# Usage:  scripts/web/bloat.sh [top-n]
# Needs:  cargo install twiggy
set -euo pipefail
cd "$(dirname "$0")/../.."
n="${1:-40}"

if ! command -v twiggy >/dev/null; then
  echo "twiggy not found — cargo install twiggy" >&2
  exit 1
fi

out=target/bloat
rm -rf "$out"
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
  CARGO_PROFILE_WEB_STRIP=false \
  CARGO_PROFILE_WEB_DEBUG=1 \
  cargo build --profile web --target wasm32-unknown-unknown -p hookecho --lib
wasm-bindgen --target web --no-typescript \
  --out-dir "$out" target/wasm32-unknown-unknown/web/hookecho.wasm
wasm="$out/hookecho_bg.wasm"

echo
echo "== $wasm ($(stat -c%s "$wasm") bytes unstripped)"
echo
echo "== biggest items"
twiggy top -n "$n" "$wasm"
echo
# `monos` groups the generic instantiations, which is where one over-generic helper becomes forty
# copies of itself and no single copy looks big enough to notice.
echo "== generic bloat (one function, many instantiations)"
twiggy monos -n 20 "$wasm"
