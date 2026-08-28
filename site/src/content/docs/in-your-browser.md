---
title: In your browser
description: HookEcho runs as a web app at app.hookecho.io — live data, nothing to install.
order: 6
---

[**app.hookecho.io**](https://app.hookecho.io) is the whole application compiled
to WebAssembly, running on live data, with nothing to install and no account.
It's the fastest way to try it, and it's a perfectly good way to *use* it.

## What's the same

Nearly everything. Radar products and tilts, velocity dealiasing, the panel and
<kbd>Ctrl</kbd>+<kbd>K</kbd>, warnings and storm reports, the national mosaic,
model layers, soundings, cross-sections, the 3D volume, and archive playback all
the way back to 1991.

## On an old machine

[**app.hookecho.io/lite/**](https://app.hookecho.io/lite/) is a second, much
smaller page for hardware the full app asks too much of. It needs no WebGL and
downloads about seventy kilobytes instead of five megabytes, because it draws
the radar on the processor rather than the graphics card.

You get an animated loop of the last six scans, reflectivity or velocity, a
picker for every NEXRAD site, three fixed zoom levels, and the tornado, severe
thunderstorm and flash flood warnings currently in view. There's no panning, no
archive and no other product — for those, use the full app or the desktop build.

## What's different

The browser doesn't hand a web page everything a native app gets:

- **Files.** Importing colour tables or placefiles from disk, and exporting GIFs
  and MP4s, work best on the desktop build.
- **Location.** The browser asks before it shares it, and there's no background
  GPS — chase mode is a desktop and Android feature.
- **Background alerting.** A tab that isn't open can't warn you. For alerts that
  reach you when the app is closed, use the [phone
  app](/docs/on-your-phone/) or leave the desktop build running.
- **Speed.** WebAssembly is fast, but a native build with your own GPU is
  faster. On a modest laptop, keep the pane count down.

Settings you change in the browser stay in that browser, on that machine.

## Putting a radar on your own page

Adding `?embed` to the URL strips the chrome and throttles the app when it
isn't visible, which is what you want inside an `<iframe>` on a dashboard. The
same `#goto` deep links the app uses for sharing work in the browser too, so you
can link straight to a site, a product and a moment in time.
