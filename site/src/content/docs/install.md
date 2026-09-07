---
title: Install
description: Download HookEcho for Windows, macOS, Linux or Android — or run it in the browser with nothing to install.
order: 2
---

[The download page](/download/) helps you choose the right build for your machine.
Use [the rolling beta build](https://github.com/d4vid87/hookecho/releases/tag/latest)
for the current filenames below, or choose a versioned release from
[all releases](https://github.com/d4vid87/hookecho/releases). Versioned
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

On Debian or Ubuntu, **`HookEcho-amd64.deb`** installs it properly
with a menu entry and icon:

```sh
sudo apt install ./HookEcho-amd64.deb
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

Install Git and a [Rust toolchain](https://rustup.rs/) first. Then clone the repository
and run from its root:

```sh
git clone https://github.com/d4vid87/hookecho.git
cd hookecho
cargo run --release
```

Needs a Rust toolchain, and on Linux the ALSA, Wayland and GTK development
headers. Android builds go through `android/build.sh` with the NDK and
`cargo-ndk`.

## Update an existing installation

Download the new build for the same platform from the [download page](/download/).
Close HookEcho before running an installer or replacing a portable build. On
Android, install the new APK over the existing app; uninstalling first can remove
app data.

After opening the update, check your saved places and alert settings. If something
fails, include the build version and operating system in a
[bug report](https://github.com/d4vid87/hookecho/issues/new).

## Next step

[Open your first radar](/docs/getting-started/) or check
[installation and loading problems](/docs/troubleshooting/).
