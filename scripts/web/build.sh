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

# `--profile web` is release + opt-level="s", with the decode crates pinned back to 3. See the
# `[profile.web]` block in the workspace Cargo.toml.
cargo build --profile web --target wasm32-unknown-unknown -p hookecho --lib
wasm-bindgen --target web --no-typescript \
  --out-dir web/dist --out-name hookecho \
  target/wasm32-unknown-unknown/web/hookecho.wasm

wasm="web/dist/hookecho_bg.wasm"
glue="web/dist/hookecho.js"

# wasm-opt takes another ~15% off what LTO leaves behind, mostly dead-function and local pruning.
# Optional on purpose: a dev running this on a laptop without binaryen still gets a working bundle,
# just a fatter one. CI installs binaryen, so the deployed bundle is always optimized.
#
# `-all`: rustc emits bulk-memory, sign-ext and friends by default now, and wasm-opt's validator
# rejects the input outright unless those proposals are enabled. It never *introduces* a feature
# the input didn't already use, so this is "accept what rustc produced", not "target the bleeding
# edge" — the smoke test is what proves the result still runs.
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os -all "$wasm" -o "$wasm.opt"
  mv "$wasm.opt" "$wasm"
else
  echo "warning: wasm-opt not found (install binaryen) — bundle is ~15% larger than a CI build" >&2
fi

# Content hashing. The committed sources keep their plain names — only the deployed copies get a
# hash — so `git status` stays clean and `web/_headers` can mark /dist/* immutable for a year.
# Order matters: the glue file names the wasm, so hash the wasm first and rewrite the glue, then
# hash the glue, then rewrite index.html.
hash_of() { sha256sum "$1" | cut -c1-8; }

rm -f web/dist/hookecho_bg-*.wasm web/dist/hookecho-*.js

wasm_hash="$(hash_of "$wasm")"
cp "$wasm" "web/dist/hookecho_bg-$wasm_hash.wasm"
sed -i "s/hookecho_bg\.wasm/hookecho_bg-$wasm_hash.wasm/g" "$glue"

glue_hash="$(hash_of "$glue")"
cp "$glue" "web/dist/hookecho-$glue_hash.js"
# Put the glue back the way git has it; the hashed copy is the one that ships.
sed -i "s/hookecho_bg-$wasm_hash\.wasm/hookecho_bg.wasm/g" "$glue"

# web/index.html is generated (gitignored); web/index.src.html is the committed source. Generating
# it rather than sed-ing in place keeps `git status` clean across builds.
sed \
  -e "s#dist/hookecho\.js#dist/hookecho-$glue_hash.js#g" \
  -e "s#dist/hookecho_bg\.wasm#dist/hookecho_bg-$wasm_hash.wasm#g" \
  web/index.src.html > web/index.html

# web/sw.js is generated the same way, from web/sw.src.js: the shell list has to name the hashed
# assets of *this* build, and the body doubles as the worker's version — a byte-identical script is
# a worker the browser never bothers to install.
shell_json="[\"/\",\"/dist/hookecho-$glue_hash.js\",\"/dist/hookecho_bg-$wasm_hash.wasm\",\"/decode-worker.js\",\"/manifest.webmanifest\",\"/icon-192.png\",\"/icon-512.png\"]"
sed \
  -e "s#__SHELL__#$shell_json#" \
  -e "s#__VERSION__#\"$glue_hash-$wasm_hash\"#" \
  web/sw.src.js > web/sw.js

# A sed that silently matched nothing ships a 404 instead of an app. Assert every dist reference
# in the deployed HTML actually exists on disk.
grep -oh 'dist/[A-Za-z0-9_.-]*' web/index.html web/sw.js | sort -u | while read -r ref; do
  [ -f "web/$ref" ] || { echo "build.sh: a deployed file references missing web/$ref" >&2; exit 1; }
done

# Size gate. The wire cost is the compressed size, so that is what is budgeted. Runs here rather
# than in a workflow so a local build fails the same way CI does.
gz_bytes="$(gzip -9 -c "web/dist/hookecho_bg-$wasm_hash.wasm" | wc -c)"
# ponytail: a regression gate, not an aspiration. It is set just above what the current build
# produces, so a careless new dependency trips it; getting the number meaningfully lower means
# cutting fonts (~1.9 MB of TTF) or a wgpu backend, and neither is free. Raise it deliberately.
budget="${HOOKECHO_WASM_BUDGET:-4850000}"
printf 'wasm: %s raw, %s gzipped (budget %s)\n' \
  "$(stat -c%s "web/dist/hookecho_bg-$wasm_hash.wasm")" "$gz_bytes" "$budget"
if [ "$gz_bytes" -gt "$budget" ]; then
  echo "build.sh: wasm is over the size budget — every byte here is on the critical path for a" >&2
  echo "  first-time visitor. Trim it, or raise HOOKECHO_WASM_BUDGET deliberately." >&2
  exit 1
fi

echo "web/dist ready:"
ls -la web/dist
