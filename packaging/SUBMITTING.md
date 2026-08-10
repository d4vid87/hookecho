# Submitting to the stores

The manifests in this directory all build. None of them is published, and
publishing is the part that cannot be automated from here: every store wants an
account, a pull request against somebody else's repository, or both, and most
want a human to look at it.

Do these in the order below. The first has the longest queue, so it goes first
even though it is the most work.

Everything here assumes the release artifacts exist and
`scripts/release/stamp-manifests.sh <version>` has been run — that is what fills
in the versions and hashes the manifests carry as placeholders. Nothing it
stamps is committed back to this repo; the stamped copies are submission inputs.

## 1. Flathub — weeks

Longest review, so start here.

- Input: `packaging/flatpak/zip.batman.hookecho.yml` and
  `packaging/flatpak/cargo-sources.json`.
- Regenerate the sources file after any `Cargo.lock` change:
  `python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json`.
  The `flatpak_sources` test in `crates/hookecho/tests/` fails when it is stale.
- Build it locally first — `flatpak-builder --force-clean build
  packaging/flatpak/zip.batman.hookecho.yml` — because a build failure in their
  CI costs another round trip through the queue.
- Submit: a pull request to
  [flathub/flathub](https://github.com/flathub/flathub) on the `new-pr` branch,
  one app per PR. Expect review comments about the `finish-args` sandbox holes
  specifically; each one needs a justification in the thread.

## 2. AUR — same day

- Input: `packaging/aur/PKGBUILD`, stamped.
- Needs an AUR account with an SSH key registered.
- `makepkg --printsrcinfo > .SRCINFO` in the package checkout — the AUR rejects
  a push whose `.SRCINFO` does not match the `PKGBUILD`, and this is the step
  everyone forgets.
- `git push` to `ssh://aur@aur.archlinux.org/hookecho.git`. No review.

## 3. Homebrew tap — same day

- Input: `packaging/homebrew/hookecho.rb`, stamped.
- A tap is its own GitHub repository named `homebrew-hookecho`; the formula goes
  in `Formula/`. Users then run `brew tap d4vid87/hookecho && brew install
  hookecho`.
- Not homebrew-core: that wants a notable user base and no `HEAD` builds, and
  the tap is the honest place for this until the former is true.

## 4. winget — days

- Inputs: the three files in `packaging/winget/`, stamped — the installer
  manifest carries the SHA256 of the built setup `.exe`.
- Validate first: `winget validate --manifest packaging/winget` and, on a
  Windows machine, `winget install --manifest packaging/winget`.
- Submit: a pull request to
  [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under
  `manifests/z/zip.batman/hookecho/<version>/`. Their bot does most of the
  review; the usual failure is a hash that does not match the URL's contents.

## 5. Snap — needs a name registration first

- Input: `snap/snapcraft.yaml` (repo root, not this directory).
- Register the name at [snapcraft.io/register-snap](https://snapcraft.io/register-snap)
  first: it can be taken, and finding that out after a build is the expensive
  order.
- `snapcraft` to build, `snapcraft upload --release=edge hookecho_*.snap` to
  push. Promote to `stable` once it has been installed from `edge` on a machine
  that is not the build machine.

## After any of them lands

Update the install section of the README with the real command, and say in the
release notes which channel it went to. A store entry nobody is told about is
the same as no store entry.
