---
title: "Hello, HookEcho"
description: "A free, open-source weather radar app that talks straight to the public NOAA feeds — why it exists, and who it's for."
date: 2026-08-26
image: /shots/reflectivity.jpg
---

Every radar app on your phone is somebody's business model. The data underneath
them is free — the United States operates 150-odd Doppler radars and publishes
every sweep of every one of them, for nothing, to anyone — and yet the way most
people see that data is through an app that wants a subscription, an account, or
a look at where you've been.

HookEcho is what happens when you cut all of that out. It's a radar viewer that
talks to the NOAA and National Weather Service feeds directly, from your own
machine. No account. No subscription. No telemetry. It's MIT-licensed, and the
source is all there.

## Weather radar for everyone

There are two kinds of radar software. The consumer apps show you a blob of
green and a chance of rain. The professional ones — the ones storm chasers and
broadcast meteorologists use — show you everything, and expect you to already
know what a correlation coefficient is.

HookEcho tries to be one program that's both, by putting the depth where you go
looking for it instead of in your way.

Open it and it finds your nearest radar and starts the loop. That's the whole of
setup: one question, and it answers that one itself if you let it use your
location. If all you want is to know whether the rain is going to reach you
before you get home, you are already done.

Go looking, and underneath is the full thing: Level 2 volumes at every tilt,
velocity with dealiasing and storm-relative motion, the dual-pol products,
cross-sections, a 3D volume you can orbit, the national MRMS mosaic, HRRR and
NAM model fields, Skew-T soundings with the severe composite parameters, warning
verification scored against the storm reports that actually came in, and an
archive that reaches back to **June 1991**.

Same app. Same map. You just have to want it.

## What it runs on

Windows, macOS, Linux, Android — and the browser, where the whole thing runs as
WebAssembly on live data with nothing to install. Try it at
[app.hookecho.io](https://app.hookecho.io) before you download anything.

## Where the data comes from

NEXRAD Level 2 and Level 3 from the radar network. MRMS for the national mosaic
and the gridded products. HRRR, NAM and RAP for the model layers. The NWS for
warnings, forecasts and storm reports. The University of Wyoming's archive for
the real balloon soundings.

All public, all free, all fetched by your machine rather than by a server in the
middle that gets to see what you're looking at.

## Start here

- [Getting started](/docs/getting-started/) — the ten-second version.
- [Reading the radar](/docs/reading-the-radar/) — what the colours mean.
- [The source](https://github.com/d4vid87/hookecho) — it's all there.

If something's broken or missing, [say
so](https://github.com/d4vid87/hookecho/issues). That's the whole feedback
mechanism, and it works.
