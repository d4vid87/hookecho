---
title: Install
description: Download HookEcho for Windows, macOS, Linux or Android — or run it in the browser with nothing to install.
order: 2
---

Every build comes from the same
[Releases page](https://github.com/d4vid87/hookecho/releases/latest), and
[the download page](/download/) picks the right one for your machine. Versioned
`v*` releases are the stable channel; a rolling `latest` prerelease carries the
newest work if you don't want to wait for a tag.

Or don't install anything: [app.hookecho.io](https://app.hookecho.io) is the
whole app running in your browser on live data.

## Windows

Download **`HookEcho-setup-x86_64.exe`** and run it. If you manage machines and
want a scriptable install, **`HookEcho-x86_64.msi`** is the same app as an MSI.
There is also a portable **`hookecho-windows-x86_64.zip`** — unzip it and run
`hookecho.exe`, no installer involved.

## Linux

Download **`HookEcho-x86_64.AppImage`**, make it executable, and run it:

```sh
chmod +x HookEcho-x86_64.AppImage
./HookEcho-x86_64.AppImage
```

On Debian or Ubuntu, **`hookecho_<version>_amd64.deb`** installs it properly
with a menu entry and icon:

```sh
sudo apt install ./hookecho_*.deb
```

Packaging manifests for Flatpak, Snap, the AUR and Homebrew live in the repo and
build today, but none are published to their stores yet.

## Android

Sideload **`HookEcho-arm64-v8a.apk`** (arm64, Android 10 or newer): open it on
the device with "install unknown apps" enabled, or run `adb install -r` from a
computer. It's the same Rust app as the desktop build, with a phone interface —
see [On your phone](/docs/on-your-phone/).

## macOS (experimental)

**`HookEcho-macos.zip`** is built and smoke-tested in CI, but has never been run
on real Apple hardware, and it is ad-hoc signed — Gatekeeper will ask before it
opens. Homebrew users can build it from source instead:

```sh
brew install --HEAD d4vid87/hookecho/hookecho
```

If you run it on a Mac,
[say what happened](https://github.com/d4vid87/hookecho/issues) — that's the
only way this build stops being experimental.

## From source

```sh
cargo run --release
```

Needs a Rust toolchain, and on Linux the ALSA, Wayland and GTK development
headers. Android builds go through `android/build.sh` with the NDK and
`cargo-ndk`.
