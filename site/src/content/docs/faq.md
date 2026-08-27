---
title: FAQ and glossary
description: Common questions, common problems, and where to find what all the abbreviations mean.
order: 7
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

**Nothing loads.** The app talks to NOAA and the NWS directly, so this is
usually the network. `--status` prints a per-feed report from the terminal.

**The times look wrong.** Radar times read in *the selected radar's* local time
— not yours and not Zulu — so a Texas pane and an Oklahoma pane can differ side
by side. Settings → Units switches everything to UTC.

**It's slow.** Lower the 3D quality preset, turn off the animated wind
particles, and use fewer panes. On a phone, close station cards you aren't
reading.

**The disk is filling up.** Settings → Storage lists every cache against its
cap, with clear and open buttons. Nothing cached is irreplaceable.

Still stuck? [Open an issue](https://github.com/d4vid87/hookecho/issues) with
the site, the product and the time you were looking at — that's usually enough
to replay it.

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
