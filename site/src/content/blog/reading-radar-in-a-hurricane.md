---
title: "Reading radar in a hurricane"
description: "A landfalling hurricane breaks most of the habits that work on a supercell. What the eyewall, the rainbands and the velocity field are actually telling you."
date: 2026-09-01
---

Most radar advice is written for a supercell on the Plains: find the hook, check
the couplet, watch correlation coefficient for debris. Point those habits at a
landfalling hurricane and they mislead you. The storm is a thousand times
larger, the dangerous parts are not where the brightest colours are, and the
tornadoes — and there will be tornadoes — look nothing like the ones in the
textbook.

Here is what to look at instead.

## The eye is the least interesting part

The eye is the easiest thing to find and tells you the least. What it does tell
you is structural: a small, round, sharply-walled eye is a strong, organised
hurricane. A large ragged one, or an eye that has gone fuzzy, is a storm that is
weakening or reorganising.

The part that matters is the ring around it. The **eyewall** is where the
strongest winds in the storm are, and on reflectivity it is a closed or nearly
closed ring of heavy rain. Watch for two things. First, whether the ring is
complete — an open eyewall on the side facing you is a weaker storm arriving.
Second, whether there are *two* rings. A concentric eyewall means the storm is
going through an eyewall replacement cycle: it will usually weaken slightly at
the peak and then broaden, which spreads damaging wind over a much wider area.
A weakening headline and a worse outcome are entirely compatible.

## Rainbands are where the tornadoes are

Tropical tornadoes almost never come out of the eyewall. They come out of the
outer rainbands, usually in the right-front quadrant relative to the storm's
motion, and often hundreds of kilometres from the centre — frequently well
before the eye arrives, and sometimes in places that never see hurricane-force
sustained wind at all.

They are also nasty to spot. A tropical tornado forms in a small, shallow,
fast-moving cell embedded in a band of rain. It spins up in minutes, it may
never produce a hook, and it is often too shallow to be seen at all beyond
about 60 miles from the radar because the beam has already climbed over it.

So: watch the individual cells inside the bands, not the bands. Look for a cell
that has turned slightly right of the others around it, and check velocity on
the lowest tilt you have.

## Velocity is doing two things at once

This is the part that catches people out. In a supercell, a red-green couplet
sitting side by side means rotation. In a hurricane, the whole velocity field
is already a huge dipole — half the storm rotating towards the radar, half away
— because that is what a cyclone is.

The trick is scale. The storm-scale dipole is enormous and smooth. A tornadic
couplet is small, tight, and sits *inside* one of those halves as a local
anomaly. Zoom in until the storm-scale flow fills the screen and the small
features stop hiding inside it.

Two other velocity readings worth having:

- **Where the strongest inbound winds are.** That is the part of the eyewall
  aimed at you, and radar sees it before it arrives.
- **Range folding and dealiasing failures.** Hurricane winds routinely exceed
  what a single pulse repetition frequency can measure unambiguously, so
  velocity wraps around and a 90-knot wind can display as a 30-knot wind of the
  opposite sign. Dealiasing fixes most of it; where it fails you get a speckled
  patch of impossible-looking extremes next to the real ones. Do not read those
  pixels literally.

## Correlation coefficient still works, differently

CC drops where the beam is full of unlike things. In a supercell that is how you
confirm lofted debris. In a hurricane, low CC over land near the coast is more
often *biological*: birds and insects pulled into the circulation, especially in
and around the eye, where they concentrate. It is a real signature and a
genuinely useful one for finding the exact centre, but it is not damage.

Debris signatures do happen with tropical tornadoes — they are just smaller,
briefer and shallower than the Plains version, so you need the lowest tilt and
some luck on range.

## Do not trust a single radar

Coastal WSR-88Ds get destroyed, lose power, or lose their radome mid-event. It
has happened repeatedly. Beyond that, the beam climbs with range: 100 miles out
you are sampling several thousand feet up, well above the shallow tropical
supercells you are hunting.

So work from more than one site, and use the national mosaic for structure and
motion while you use the nearest single site for the cell-level detail. If the
nearest site drops out mid-storm, that itself is information about conditions
there.

## Try it

Three landfalls worth replaying, all with the warnings that were in force at
the time:

- [Hurricane Ian, 2022](/storms/hurricane-ian-2022/) — a strong, well-defined
  eyewall crossing the southwest Florida coast.
- [Hurricane Harvey, 2017](/storms/hurricane-harvey-2017/) — landfall, then the
  rainfall catastrophe that followed when it stalled.
- [Hurricane Katrina, 2005](/storms/katrina-2005/) — the Gulf coast landfall,
  from a radar that was inside the storm.

Open one, step the timeline through the eyewall, then switch to velocity and go
hunting in the outer bands. That contrast — the smooth enormous dipole, and the
small tight thing hiding inside it — is the whole skill.
