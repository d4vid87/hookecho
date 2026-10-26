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

# wasm-bindgen names inline snippets by crate, so their URLs stay the same even when their
# exports change. Give each snippet a content-addressed URL before hashing the glue; otherwise
# a browser can pair a new glue file with an old, immutable inline0.js.
find web/dist/snippets -type f -name 'inline*-*.js' -delete
find web/dist/snippets -type f -name 'inline*.js' | while read -r snippet; do
  hash="$(sha256sum "$snippet" | cut -c1-8)"
  hashed="${snippet%.js}-$hash.js"
  cp "$snippet" "$hashed"
  # Only the JS import URL changes. The wasm import-map key below it is part of the ABI.
  sed -i "/^import .* from /s#${snippet#web/dist/}#${hashed#web/dist/}#" "$glue"
done

# wasm-opt takes another ~15% off what LTO leaves behind. Convergence saves another ~6 KB gzip
# on the main build, keeping room under the existing cap for analyst controls.
# Optional on purpose: a dev running this on a laptop without binaryen still gets a working bundle,
# just a fatter one. CI installs binaryen, so the deployed bundle is always optimized.
#
# `-all`: rustc emits bulk-memory, sign-ext and friends by default now, and wasm-opt's validator
# rejects the input outright unless those proposals are enabled. It never *introduces* a feature
# the input didn't already use, so this is "accept what rustc produced", not "target the bleeding
# edge" — the smoke test is what proves the result still runs.
if command -v wasm-opt >/dev/null; then
  wasm-opt -Os --converge -all "$wasm" -o "$wasm.opt"
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

# The browser video recorder is optional, but it must update with the shell rather than stay in
# an old offline cache when only its JavaScript changes.
rm -f web/dist/video-export-*.js
video_export_hash="$(hash_of web/video-export.js)"
cp web/video-export.js "web/dist/video-export-$video_export_hash.js"

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

# Amy is deliberately outside the radar wasm and below Cloudflare Pages' 25 MiB file ceiling.
# Start-up JS fetches these ordered pieces only after the map is running.
"$root/packaging/piper/fetch.sh" web "$root/target/piper/web"
"$root/scripts/web/piper-runtime.sh"
voice_dir="web/voice/amy-medium-39ab474b"
mkdir -p "$voice_dir"
find "$voice_dir" -type f -delete
split -b 20M -d -a 2 "$root/target/piper/web/en_US-amy-medium.onnx" "$voice_dir/model.part-"
cp "$root/target/piper/web/en_US-amy-medium.onnx.json" "$voice_dir/en_US-amy-medium.onnx.json"
chunks="$(find "$voice_dir" -name 'model.part-*' -printf '%f\n' | sort | jq -R . | jq -sc .)"
jq -n --argjson chunks "$chunks" '{voice:"en_US-amy-medium", chunks:$chunks}' > "$voice_dir/manifest.json"

# web/index.html is generated (gitignored); web/index.src.html is the committed source. Generating
# it rather than sed-ing in place keeps `git status` clean across builds.
sed \
  -e "s#dist/hookecho\.js#dist/hookecho-$glue_hash.js#g" \
  -e "s#dist/hookecho_bg\.wasm#dist/hookecho_bg-$wasm_hash.wasm#g" \
  -e "s#video-export\.js#dist/video-export-$video_export_hash.js#g" \
  -e "s#__FONT_URLS__#$font_urls#" \
  web/index.src.html > web/index.html

# web/sw.js is generated the same way, from web/sw.src.js: the shell list has to name the hashed
# assets of *this* build, and the body doubles as the worker's version — a byte-identical script is
# a worker the browser never bothers to install.
font_shell=""
for f in web/dist/font-*.ttf; do font_shell="$font_shell,\"/${f#web/}\""; done
shell_json="[\"/\",\"/dist/hookecho-$glue_hash.js?boot=2\",\"/dist/hookecho_bg-$wasm_hash.wasm\",\"/dist/video-export-$video_export_hash.js\",\"/decode-worker.js?v=radar-fields\",\"/decode-bridge.js\",\"/manifest.webmanifest\",\"/icon-192.png\",\"/icon-512.png\"$font_shell]"
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
# Raised deliberately for the offline chase packs (IndexedDB via web-sys) and the detailed dark
# street-map labels shipped in the default view.
# Vector-tile worker serialization adds ~2.4 KB gzip while moving tessellation off the UI thread.
# The station-analysis timeline measured 4,244,093 bytes gzip with binaryen; retain a 0.21%
# regression allowance rather than silently failing the new preview on a 4,240,000-byte gate.
# Native point/station RTMA sampling for the objective sounding measured 4,257,369 bytes after
# sharing the existing GRIB decoder and removing the old map-grid sounding path. The remaining
# 2,631-byte allowance is 0.06% of the measured bundle.
# Native θe contours compare GRIB grid definitions before deriving, avoiding two duplicate
# coordinate arrays (local test-process peak: 353.5 → 293.1 MiB). This measured 4,261,133
# gzip bytes; the ceiling keeps a 2,867-byte (0.07%) allowance.
# Contour GeoJSON export measured 4,335,841 gzip bytes; keep a 9,159-byte margin.
# Pane appearance, native precision and globe projection add about 17 KB gzip
# (0.4%) to the prior baseline. Keep a small margin for the complete feature set.
# Follow-up presets, sector freshness, timing limits and adaptive-quality counters measured
# 4,365,401 bytes gzip before the final cadence refinement (~0.3% above the prior integration).
# Retain a small explicit margin; the lite and startup budgets remain unchanged.
# Shared downloads, bounded retention, transition identity, workspace Undo and diagnostic
# replay metadata measured 4,427,028 gzip bytes before the final timeline export. Allocate
# 30 KB (0.68%) over the prior cap for the complete reliability follow-up; lite and startup
# limits remain unchanged. Alternative binaryen shrinking passes measured larger outputs.
# Radar Workspace, perspective camera, source catalog and public-feed availability measured
# 4,489,754 gzip bytes with binaryen 130. Allocate 50 KB (1.12%) over the former cap;
# the lite and startup limits remain unchanged.
# Public CPC and SPC fire-weather polygons measured 4,503,536 gzip bytes with binaryen 130,
# 3,536 bytes above the previous cap (0.08%). Allow 11,464 bytes of headroom; the lite and
# startup limits remain unchanged.
# Timestamped TDWR archive playback measured 4,523,808 gzip bytes with binaryen 130.
# Allow 6,192 bytes of margin above that measured result; lite/startup caps are unchanged.
# Public Level III DAA/DSA accumulation adds two real radar products to the first-load bundle.
# The measured optimized main WASM is 4,531,260 bytes gzip; retain under 0.2% headroom.
# NOAA SPC fire-weather discussion adds 1,305 gzip bytes to the measured 4,539,943-byte
# main WASM. Keep a narrow 8,752-byte margin for this public-data parity feature.
# The manual storm-track controls add a few kilobytes to the interactive map bundle.
# Browser PNG capture reuses the existing screenshot command and download helper but includes
# the PNG encoder in the main bundle. Its measured size is 4,555,906 gzip bytes; allow 4,094
# bytes of margin (under 0.1%) while keeping the lite and startup limits unchanged.
# Browser GIF export adds the codec and in-memory loop capture. The optimized main bundle measured
# 4,583,740 gzip bytes; retain about 6 KB of margin (0.13%) for this verified four-frame export.
# Public global-radar tiles and frame discovery measure 4,590,891 bytes gzip with binaryen 130.
# Keep roughly 5 KB of regression headroom while leaving the lite and startup caps unchanged.
# Dublin's complete velocity volume and product-specific tilt picker measure 4,628,168 gzip bytes.
# Keep 832 bytes of headroom without changing the lite or startup caps.
# NOAA's matching NDFD legend images are fetched only when a forecast map is selected. The
# desktop loader and compact forecast controls measured 4,632,979 bytes gzip; retain 3,021 bytes
# of margin (0.07%) for the main bundle without changing lite/startup budgets.
# NOAA's exact raster-point readout adds 5,248 gzip bytes to the previous 4,632,160-byte build.
# Allow 2,592 bytes of margin for the completed forecast inspection path; lite/startup stay put.
# Alaska/Hawaii NDFD regions measured 4,639,631 gzip bytes; retain 4,369 bytes of margin.
# Six-frame regional MRMS replay measured 4,650,476 gzip bytes with binaryen 130. Allow 4,524
# bytes of headroom for the new source-backed transport; lite and startup budgets stay unchanged.
# UTC archive-day and hour navigation adds a source-backed historical loop without storing days of
# grids in memory. The optimized main bundle measured 4,653,859 gzip bytes; retain under 0.2%
# headroom while preserving the Lite and startup gates.
# Public-camera dashboard adds a small amount of UI code; measured at 4,664,691 bytes gzip.
# Complete raster fallbacks and faster label lookup measured 4,676,680 gzip bytes in CI.
# Allow 10 KB (0.21%) more than the previous cap; Lite/startup gates remain unchanged.
# Radar/MRMS worker preparation and the cached smoothing pass add about 8 KB gzip.
# Keep the measured feature increase explicit; Lite/startup budgets stay unchanged.
# Signal rail category navigation measured 4,703,917 gzip bytes (about 0.08% over the prior bundle).
# Allow a small main-bundle margin; Lite and startup budgets remain unchanged.
budget="${HOOKECHO_WASM_BUDGET:-4710000}"
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
