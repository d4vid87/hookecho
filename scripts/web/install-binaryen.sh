#!/usr/bin/env bash
# Pin the optimizer: Ubuntu's Binaryen 108 misexports the externref table emitted by wasm-bindgen.
set -euo pipefail
install_dir="${RUNNER_TEMP:?}/hookecho-binaryen"
mkdir -p "$install_dir"
curl -fsSL --retry 3 \
  https://github.com/WebAssembly/binaryen/releases/download/version_130/binaryen-version_130-x86_64-linux.tar.gz \
  -o "$install_dir/binaryen.tar.gz"
echo "0a18362361ad05465118cd8eeb72edaeec89de6894bc283576ef4e07aa3babcc  $install_dir/binaryen.tar.gz" | sha256sum -c -
tar -xzf "$install_dir/binaryen.tar.gz" -C "$install_dir"
echo "$install_dir/binaryen-version_130/bin" >> "${GITHUB_PATH:?}"
"$install_dir/binaryen-version_130/bin/wasm-opt" --version
