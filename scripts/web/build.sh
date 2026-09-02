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

# The four default font faces, which the web build fetches instead of compiling in (see
# crates/hookecho/src/fonts.rs and web/fonts/README.md). Hashed into /dist/ like everything else,
# so the year-long immutable `_headers` rule covers them and a face is downloaded once, ever.
rm -f web/dist/font-*.ttf
font_urls=""
for face in Hack-Regular NotoEmoji-Regular Ubuntu-Light emoji-icon-font; do
  src="web/fonts/$face.ttf"
  [ -f "$src" ] || { echo "build.sh: missing $src" >&2; exit 1; }
  h="$(hash_of "$src")"
  cp "$src" "web/dist/font-$face-$h.ttf"
  font_urls="$font_urls\"$face\":\"/dist/font-$face-$h.ttf\","
done
# Trailing comma trimmed: this is pasted into the page as a JS object literal.
font_urls="{${font_urls%,}}"

# web/index.html is generated (gitignored); web/index.src.html is the committed source. Generating
# it rather than sed-ing in place keeps `git status` clean across builds.
sed \
  -e "s#dist/hookecho\.js#dist/hookecho-$glue_hash.js#g" \
  -e "s#dist/hookecho_bg\.wasm#dist/hookecho_bg-$wasm_hash.wasm#g" \
  -e "s#__FONT_URLS__#$font_urls#" \
  web/index.src.html > web/index.html

# web/sw.js is generated the same way, from web/sw.src.js: the shell list has to name the hashed
# assets of *this* build, and the body doubles as the worker's version — a byte-identical script is
# a worker the browser never bothers to install.
font_shell=""
for f in web/dist/font-*.ttf; do font_shell="$font_shell,\"/${f#web/}\""; done
shell_json="[\"/\",\"/dist/hookecho-$glue_hash.js\",\"/dist/hookecho_bg-$wasm_hash.wasm\",\"/decode-worker.js\",\"/decode-bridge.js\",\"/manifest.webmanifest\",\"/icon-192.png\",\"/icon-512.png\"$font_shell]"
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
# cutting a wgpu backend, which is not free. The fonts already went (they are fetched at runtime
# now — see crates/hookecho/src/fonts.rs), which is what this number dropped by. Raise it
# deliberately.
# The number tracks CI's build, and a local build without binaryen does not reproduce it: wasm-opt
# leaves a SMALLER raw module that GZIPS LARGER (11.8 MB raw / 4.15 MB gz in CI against 12.9 MB
# raw / 4.01 MB gz here), so skipping it makes a local build look ~130 KB under the gate while CI
# is over it. Install binaryen and the two agree; the warning above is not cosmetic.
#
# Raised deliberately for the offline chase packs (IndexedDB via web-sys) — the one budget raise
# the R18 batch reserved for itself.
budget="${HOOKECHO_WASM_BUDGET:-4166000}"
printf 'wasm: %s raw, %s gzipped (budget %s)\n' \
  "$(stat -c%s "web/dist/hookecho_bg-$wasm_hash.wasm")" "$gz_bytes" "$budget"
if [ "$gz_bytes" -gt "$budget" ]; then
  echo "build.sh: wasm is over the size budget — every byte here is on the critical path for a" >&2
  echo "  first-time visitor. Trim it, or raise HOOKECHO_WASM_BUDGET deliberately." >&2
  exit 1
fi

# --- The lite viewer (web/lite, deployed at /lite/) -------------------------------------------
#
# A second, much smaller wasm: canvas-2D radar for machines that cannot run the app above. It ships
# from the same origin so it can use the same /proxy, and it is hashed into web/dist so the
# `_headers` immutable rule covers it with no new entry.
cargo build --profile web --target wasm32-unknown-unknown -p hookecho-lite --lib
wasm-bindgen --target web --no-typescript \
  --out-dir web/dist --out-name lite \
  target/wasm32-unknown-unknown/web/hookecho_lite.wasm

lite_wasm="web/dist/lite_bg.wasm"
lite_glue="web/dist/lite.js"
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os -all "$lite_wasm" -o "$lite_wasm.opt"
  mv "$lite_wasm.opt" "$lite_wasm"
fi

rm -f web/dist/lite_bg-*.wasm web/dist/lite-*.js

lite_wasm_hash="$(hash_of "$lite_wasm")"
cp "$lite_wasm" "web/dist/lite_bg-$lite_wasm_hash.wasm"
sed -i "s/lite_bg\.wasm/lite_bg-$lite_wasm_hash.wasm/g" "$lite_glue"

lite_glue_hash="$(hash_of "$lite_glue")"
cp "$lite_glue" "web/dist/lite-$lite_glue_hash.js"
sed -i "s/lite_bg-$lite_wasm_hash\.wasm/lite_bg.wasm/g" "$lite_glue"

# app.js is hashed like the glue rather than served under its own name. It is generated HTML that
# names it, and the two are a matched pair: a browser holding a cached app.js against a newer page
# runs code that expects controls the page no longer has. Pages serves /lite/* with a four-hour
# max-age, so that window was real.
rm -f web/dist/lite-app-*.js
lite_app_hash="$(hash_of web/lite/app.js)"
cp web/lite/app.js "web/dist/lite-app-$lite_app_hash.js"

# Generated like web/index.html: the committed source keeps plain names, the deployed copy names
# the hashed glue and the hashed page script.
sed -e "s#/dist/lite\.js#/dist/lite-$lite_glue_hash.js#g" \
  -e "s#\./app\.js#/dist/lite-app-$lite_app_hash.js#g" \
  web/lite/index.src.html > web/lite/index.html

# The site picker's data, derived from the one committed registry (site/src/data/nexrad-sites.json,
# itself CI drift-checked against wxdata) — not a second source of truth. TDWR sites are dropped:
# the bucket carries no super-res digital products for them.
if ! command -v jq >/dev/null; then
  echo "build.sh: jq not found — it generates web/lite/sites.json" >&2
  exit 1
fi
jq -c '[.[] | select(.network == "nexrad") | {id, city, state, lat, lon}]' \
  site/src/data/nexrad-sites.json > web/lite/sites.json

grep -oh 'dist/[A-Za-z0-9_.-]*' web/lite/index.html | sort -u | while read -r ref; do
  [ -f "web/$ref" ] || { echo "build.sh: web/lite/index.html references missing web/$ref" >&2; exit 1; }
done

lite_gz="$(gzip -9 -c "web/dist/lite_bg-$lite_wasm_hash.wasm" | wc -c)"
# Its own budget: the lite bundle exists to be small, and sharing the app's budget would hide a
# tenfold regression here inside the app's margin.
lite_budget="${HOOKECHO_LITE_WASM_BUDGET:-80000}"
printf 'lite wasm: %s raw, %s gzipped (budget %s)\n' \
  "$(stat -c%s "web/dist/lite_bg-$lite_wasm_hash.wasm")" "$lite_gz" "$lite_budget"
if [ "$lite_gz" -gt "$lite_budget" ]; then
  echo "build.sh: the lite wasm is over its size budget — it is the whole point of that page." >&2
  exit 1
fi

echo "web/dist ready:"
ls -la web/dist
