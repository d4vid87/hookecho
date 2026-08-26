#!/usr/bin/env bash
# Fill the per-release checksums into the store manifests.
#
# The winget, Homebrew and AUR manifests all name a version and a hash of something that only
# exists once the release artifacts are built — so they sit in the repo with placeholders and get
# stamped here, from the artifacts themselves. The stamped copies are submission inputs (a PR to
# winget-pkgs, a push to the tap and to the AUR); nothing commits them back to this repo.
#
# Usage: scripts/release/stamp-manifests.sh <version> [setup-exe]
#   e.g. scripts/release/stamp-manifests.sh 0.8.0 dist/HookEcho-setup-x86_64.exe
set -euo pipefail

VER="${1:?usage: stamp-manifests.sh <version> [setup-exe]}"
EXE="${2:-dist/HookEcho-setup-x86_64.exe}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

sha() { sha256sum "$1" | cut -d' ' -f1; }

# winget: the hash is of the installer we just built, not of anything downloadable yet.
EXE_SHA="$(sha "$EXE")"
W=packaging/winget
sed -i \
  -e "s|^PackageVersion: .*|PackageVersion: $VER|" \
  -e "s|releases/download/v[^/]*/|releases/download/v$VER/|" \
  -e "s|^\( *InstallerSha256: \).*|\1${EXE_SHA^^}|" \
  "$W/zip.batman.hookecho.installer.yaml"
sed -i "s|^PackageVersion: .*|PackageVersion: $VER|" \
  "$W/zip.batman.hookecho.yaml" "$W/zip.batman.hookecho.locale.en-US.yaml"

# Homebrew and the AUR both build from the GitHub tag tarball, so both want its hash. GitHub
# generates that tarball on demand but it is byte-stable for a given tag.
TARBALL="$(mktemp -t hookecho-src-XXXX.tar.gz)"
trap 'rm -f "$TARBALL"' EXIT
curl -fsSL -o "$TARBALL" "https://github.com/d4vid87/hookecho/archive/refs/tags/v$VER.tar.gz"
SRC_SHA="$(sha "$TARBALL")"

sed -i \
  -e "s|archive/refs/tags/v[^\"]*\.tar\.gz|archive/refs/tags/v$VER.tar.gz|" \
  -e "s|^  sha256 .*|  sha256 \"$SRC_SHA\"|" \
  packaging/homebrew/hookecho.rb

sed -i \
  -e "s|^pkgver=.*|pkgver=$VER|" \
  -e "s|^pkgrel=.*|pkgrel=1|" \
  -e "s|^sha256sums=.*|sha256sums=('$SRC_SHA')|" \
  packaging/aur/PKGBUILD

echo "stamped $VER: installer $EXE_SHA, source $SRC_SHA"
