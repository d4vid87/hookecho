---
title: FAQ and glossary
description: Common questions, common problems, and where to find what all the abbreviations mean.
order: 8
---

## Questions

**Is it free? Is there an account?**
Yes, and no. HookEcho is open source under the MIT licence. There's no account,
no subscription and no telemetry — the app talks to the public NOAA and National
Weather Service feeds directly from your machine.

**What's a hook echo?**
The signature the app is named after. A supercell's rotating updraft wraps
precipitation around itself, and on reflectivity that wrap draws a hook curling
off the back-right of the storm. It was one of the first radar signatures ever
tied to tornadoes, and it's still one of the first things a forecaster looks for.

**Where does the data come from?**
NEXRAD Level 2 and Level 3 from the radar network, MRMS for the national mosaic
and gridded products, HRRR/NAM/RAP for the model layers, and the NWS for
warnings, forecasts and storm reports. All of it public, all of it free.

**How far back does the archive go?**
June 1991. Any archived storm loads the same way the live one does. Products
that didn't exist yet — CC and ZDR before the dual-pol upgrade around 2012 —
aren't there for old events.

**Is the future radar real?**
No, and the app says so the whole time it's on. HRRR future radar, forecast
rotation tracks and smoke are model output, and they carry a banner for as long
as they're drawn.

**Do I need an API key?**
Not for the weather. A couple of optional basemaps use commercial tile services
and want their own key; a layer that needs one stays empty and says so rather
than nagging. Keys go in Settings and stay on your machine.

## Things that look broken

Use [Troubleshooting](/docs/troubleshooting/) for missing scans, browser problems,
old timestamps, notifications, storage and a checklist for reporting bugs.

## Glossary

Every radar word has its own page now, with the related ones linked from it:

[CAPE](/glossary/cape/),
[CC (correlation coefficient)](/glossary/cc/),
[dBZ](/glossary/dbz/),
[Dealiasing](/glossary/dealiasing/),
[Dual-pol](/glossary/dual-pol/),
[Level 2 / Level 3](/glossary/level-2-and-level-3/),
[MESH](/glossary/mesh/),
[MRMS](/glossary/mrms/),
[NEXRAD](/glossary/nexrad/),
[Reflectivity (Z)](/glossary/reflectivity/),
[SRH (storm-relative helicity)](/glossary/srh/),
[Storm-relative velocity](/glossary/storm-relative-velocity/),
[TDS (tornado debris signature)](/glossary/tds/),
[Tilt](/glossary/tilt/),
[VCP](/glossary/vcp/),
[Velocity (V)](/glossary/velocity/),
[VIL](/glossary/vil/),
[ZDR (differential reflectivity)](/glossary/zdr/), and
[hook echo](/glossary/hook-echo/) itself.

[The whole glossary is here.](/glossary/)

## Still stuck?

Ask in the [Discord](https://discord.gg/VNMW2Gyg4V) — someone is usually looking at the
same storm you are. For a bug or a feature request,
[open an issue](https://github.com/d4vid87/hookecho/issues).
