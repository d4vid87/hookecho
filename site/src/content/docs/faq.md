---
title: FAQ and glossary
description: Common questions, common problems, and what all the abbreviations mean.
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

**CAPE** — how much energy is available for a thunderstorm to rise. More CAPE,
taller storms.

**CC (correlation coefficient)** — whether everything in the beam looks alike.
High for rain, low for a mix — hail, melting snow, or tornado debris.

**dBZ** — the unit of reflectivity. Roughly: how much is coming back.

**Dealiasing** — undoing the fold that happens when wind is faster than the
radar can measure directly, so a fast couplet reads correctly.

**Dual-pol** — the radar sending both horizontal and vertical pulses, which is
what makes CC and ZDR possible. Rolled out across the network around 2012.

**Level 2 / Level 3** — Level 2 is the raw volume, every gate of every sweep.
Level 3 is the smaller set of processed products the NWS publishes from it.

**MESH** — maximum estimated hail size, computed from the storm's vertical
structure.

**MRMS** — the national grid that stitches every radar in the country into one
picture, updated every couple of minutes.

**NEXRAD** — the US network of 150-odd weather radars. Each has a four-letter
ID like KTLX.

**Reflectivity (Z)** — how much energy the echo sends back. Where the rain is.

**SRH (storm-relative helicity)** — how much spin the low-level wind has for a
storm to use.

**Storm-relative velocity** — velocity with the storm's own travel subtracted,
so rotation stops hiding inside the storm's motion.

**TDS (tornado debris signature)** — high reflectivity, a velocity couplet, and
a hole in CC in the same place: the radar seeing lofted debris.

**Tilt** — one sweep of the radar at one elevation angle. A volume is a stack of
them.

**VCP** — the scanning pattern the radar is running. Storm modes sweep faster
and use more tilts than clear-air modes.

**Velocity (V)** — motion toward or away from the radar along the beam. Green
toward, red away.

**VIL** — how much water the storm is holding, integrated through its depth.

**ZDR (differential reflectivity)** — how flat the things in the beam are.
Positive for raindrops, near zero for tumbling hail.
