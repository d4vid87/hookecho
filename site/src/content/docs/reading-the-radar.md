---
title: Reading the radar
description: What the colours actually mean — reflectivity, velocity and the dual-pol products, in plain language.
order: 3
---

A radar sends out a pulse and listens for what comes back. Everything on the
screen is some measurement of that echo. There are only a few you need.

## Reflectivity (Z) — where the rain is

The one everybody knows. It measures how much is coming back, in dBZ. Bigger
drops, and more of them, send back more.

- **Blue and green, under about 30 dBZ** — light rain, or snow.
- **Yellow, 35–45 dBZ** — proper rain. Enough to hear on the roof.
- **Red and beyond, over 50 dBZ** — a downpour, and a candidate for hail.

Reflectivity shows you a storm's *shape*. A neat blob is a shower. A long line
is a squall. And a curl on the back edge of a supercell is the hook that gives
this app its name — see [What's a hook echo?](/docs/faq/).

## Velocity (V) — what the storm is doing

Velocity measures whether the echo is moving toward the radar or away from it,
along the beam. Green is toward, red is away. The radar site is the centre of
that logic, so the same wind reads differently on either side of it.

What you're hunting for is **green and red tight against each other** over a
short distance. Air coming at the radar right beside air going away from it is
air spinning: a rotating storm.

HookEcho dealiases velocity by default, so a fast couplet reads as red against
green instead of folding back into nonsense.

**Storm-relative velocity** subtracts the storm's own travel across the ground,
which otherwise hides the rotation inside it. Turn it on in the panel's Layer
options when a couplet looks marginal.

## Correlation coefficient (CC) — what the echo is made of

CC asks whether everything in the beam looks alike. Rain is uniform, so CC is
high — near 1.0. Anything mixed pulls it down: hail, melting snow, birds, and
crucially **debris**. A tornado on the ground lofts fence posts and roof
shingles, and a hole punched in CC where the reflectivity is high is one of the
strongest confirmations there is.

## Differential reflectivity (ZDR)

Falling raindrops flatten out, so they're wider than they are tall, and ZDR is
positive. Hailstones tumble, so they average round, and ZDR goes to zero even
where the reflectivity is enormous. High Z with low ZDR is a hail signature.

## Three questions, three answers

**Is it rotating?** Velocity, 0.5° tilt: look for a tight inbound/outbound pair
over a few gates. Then turn on storm-relative velocity to be sure.

**Is it hail?** Reflectivity over about 50 dBZ is the candidate; confirm with CC
dropping and ZDR going flat. The **storm attributes** table
(<kbd>Ctrl</kbd>+<kbd>K</kbd> → "cells") lists every tracked cell with its hail
size, tops and VIL, sorted — click a row to fly there.

**Is it a tornado?** The debris signature is all three together at low tilt: a
velocity couplet, high reflectivity, and a hole in CC. The app flags candidates,
but those three panels are the reason.

## Tilts, and why they matter

A radar doesn't take one picture — it sweeps a full circle, steps its beam up,
and sweeps again, until it has a stack. Each of those sweeps is a **tilt**. The
beam also climbs as it travels, so far from the radar even 0.5° is well above
your head.

Climbing the tilts tells you how a storm leans. A rotating column that stands
straight up through several tilts is a much bigger deal than one that falls
apart at the second sweep. One action in <kbd>Ctrl</kbd>+<kbd>K</kbd> puts four
tilts side by side to make that easy to see.

## A caveat about old storms

Dual-polarization — CC and ZDR — arrived across the network around 2012. Replay
a storm from before your site was upgraded and those products simply aren't
there. That's the data, not a bug.
