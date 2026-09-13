#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/web/voice/runtime"
work="$root/target/piper/web-runtime"
mkdir -p "$out" "$work"
fetch() {
  local url="$1" digest="$2" path="$3"
  curl -fL --retry 3 -o "$path" "$url"
  printf '%s  %s\n' "$digest" "$path" | sha256sum -c --status || { echo "checksum mismatch: $url" >&2; exit 1; }
}
fetch 'https://registry.npmjs.org/@mintplex-labs/piper-tts-web/-/piper-tts-web-1.0.5.tgz' \
  b15c4b29bf958a93977e6eb950be186bd31cefe1a866a465ea5afde637246649 "$work/piper.tgz"
fetch 'https://registry.npmjs.org/onnxruntime-web/-/onnxruntime-web-1.18.0.tgz' \
  b4940dec2f4f63cd98d48597336809d45fdaa1c4ddc6d2e6e61183772a266e07 "$work/ort.tgz"
mkdir -p "$work/piper" "$work/ort"
tar -xzf "$work/piper.tgz" -C "$work/piper"
tar -xzf "$work/ort.tgz" -C "$work/ort"
cp "$work/piper/package/dist/piper-tts-web.js" "$out/piper-tts-web.js"
cp "$work/piper/package/dist/piper-o91UDS6e.js" "$out/piper-o91UDS6e.js"
cp "$work/ort/package/dist/ort.wasm.min.js" "$out/ort.wasm.min.js"
cp "$work/ort/package/dist/ort-wasm.wasm" "$work/ort/package/dist/ort-wasm-simd.wasm" \
  "$work/ort/package/dist/ort-wasm-simd-threaded.wasm" "$out/"
fetch 'https://cdn.jsdelivr.net/npm/@diffusionstudio/piper-wasm@1.0.0/build/piper_phonemize.wasm' \
  b777cd107a91d2bcc6a1ea46f2c26a662a7407394fe84589198aeaa83dd7a9d6 "$out/piper_phonemize.wasm"
fetch 'https://cdn.jsdelivr.net/npm/@diffusionstudio/piper-wasm@1.0.0/build/piper_phonemize.data' \
  29f1025eb23a5b5c192cd14a6efbce4509402ff265405072ee6f7d1a09b78f8c "$out/piper_phonemize.data"
# The published wrapper uses a bare npm import and remote defaults. Point its already-bundled
# code at the verified files beside it; the model itself is populated in OPFS by amy-bootstrap.
sed -i \
  -e 's#await import("onnxruntime-web/wasm")#await import("./ort.wasm.min.js")#' \
  -e 's#const ort = ortModule.default || ortModule;#const ort = globalThis.ort || ortModule.default || ortModule;#' \
  -e 's#https://cdnjs.cloudflare.com/ajax/libs/onnxruntime-web/1.18.0/#./#' \
  -e 's#https://cdn.jsdelivr.net/npm/@diffusionstudio/piper-wasm@1.0.0/build/piper_phonemize#./piper_phonemize#' \
  -e 's#ort.env.wasm.numThreads = navigator.hardwareConcurrency#ort.env.wasm.numThreads = 1#' \
  -e '/if (!url.match("https:\/\/huggingface.co")) return;/d' \
  "$out/piper-tts-web.js"
