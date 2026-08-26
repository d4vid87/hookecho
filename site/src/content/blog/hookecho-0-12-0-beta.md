---
title: "HookEcho v0.12.0 beta: a new interface, top to bottom"
description: "The docked panels are gone, the setup wizard is gone, and the phone got the same rebuild the desktop did. What's in the v0.12.0 beta."
date: 2026-08-26
image: /shots/layers.jpg
---

The **v0.12.0 beta** is out, and it's the largest change HookEcho has had. The
whole interface was rebuilt, the phone app along with it, and about a dozen data
lanes landed on top. It's a prerelease — a couple of things arrive at the final
0.12.0 — but it's the version worth using.

[Get it from Releases](https://github.com/d4vid87/hookecho/releases), or just
[open it in your browser](https://app.hookecho.io).

## The map got the whole window

The old layout was a 264-pixel sidebar down the left, a panel across the bottom,
and the weather in whatever was left. That's gone. The map is full-bleed now,
edge to edge, and the controls float over it — a search pill in one corner, a
control column on the right edge, and the timeline along the bottom.

There's no title bar either: the window is borderless and its buttons float with
the rest of the chrome. Drag the empty strip along the top to move it,
double-click to maximize, resize from any edge. If your window manager disagrees
with any of that, `--decorated` hands the frame back.

The pieces that used to be a dozen floating windows are now pages in one
slide-over drawer, one at a time, with a back arrow. And anything you click *on
the map* — a storm cell, a warning polygon, one of your own markers — answers in
a card next to where you clicked, instead of opening a window somewhere else.

**One panel holds everything**: the radar, the product, the tilt, and every
layer, window and tool, each one described in plain English and filed under
categories that carry their own count. `Ctrl+K` searches it, and Enter runs the
top match. If you learn one thing about the new build, learn that.

## The timeline is a real timeline

The bottom panel became a scrubber: the radar's clock, a play button, a LIVE
badge that snaps back to the newest scan, and a track with one tick per volume,
labelled by the hour — in the radar's own local time, not yours and not Zulu.
Drag it, or click anywhere on it. The shaded stretch at the live end is the loop
the play button cycles. Right-click the badge for the archive day.

## Setup stopped asking questions

The old first-run wizard is deleted. Now the app finds you, picks the nearest
radar and draws it — about ten seconds, no questions. There's a 60-second tour
on offer if you want one, and it's four stops on the live map, two of which you
finish by doing the thing rather than reading about it. Skip it and nothing
nags.

Notification permission is asked for at the moment you turn on something that
notifies, or when a warning lands near you. Never at install.

Labels across the app read in plain language now — "Storm rotation (SRV)",
"Debris detection (CC)" — with the abbreviation kept in parentheses for people
who want it. And there's one help hub behind `?`: glossary, hotkeys, the tour
and what's new, all searchable together.

## The phone got the same rebuild

The persistent bottom sheet and the five-slot toolbar are gone. Android draws
the same floating chrome the desktop does, with content in Material modal
sheets. Long-press the map to inspect what's under your finger, double-tap and
drag to zoom one-handed, swipe sideways between panes. It buzzes as the scrubber
crosses a frame, when a warning lands on you, and when a sheet snaps.

Tablets and landscape phones get a side rail and a docked drawer instead of
sheets. The widgets now show how far the nearest storm is, there's a
battery-saver mode that stretches the polling and throttles repaints, and on a
chase the next radar down the road is prefetched before you need it — then the
whole chase can be replayed against the archive afterwards.

## And a lot of weather

- **GOES** satellite loops run over archive dates alongside the radar timeline,
  with the mid-level water vapour bands selectable.
- **MRMS rotation tracks** at 30, 60 and 120 minutes are field layers now, and
  hail swaths accumulate locally over a window you pick.
- An alert rule can trigger on a **lightning jump** — the rate of change of
  flash density inside a cell, which is one of the things that spikes before a
  storm turns severe.
- **NAM nest and NBM** join the model list; **Synoptic** mesonets join the
  surface observations, with your own API key.
- Cells are **ranked by a composite severity score**, and tapping one opens the
  3D volume already centred on it.
- Panes remember their own thresholds and layers, and an event can be saved as a
  **replay bundle** — time range, site, camera — then replayed with archived
  radar, warnings, reports and outlooks all in sync.

## The browser build grew up

It installs as a **PWA** with the app shell cached offline. Tiles, volumes and
palettes persist between visits. File dialogs, GPS and notifications all work
now. And a share button writes a permalink carrying the site, the camera and the
time, so you can send someone exactly what you're looking at.

## Elsewhere

MQTT publishing, a Home Assistant camera-loop endpoint, and a live dashboard at
the `--serve` index. An update chip when a newer release exists — and no
self-updating, ever. Motion throughout, with a reduce-motion setting and an
automatic degrade when frames get slow. Labels, roles and focus order on all of
the new chrome.

Every screenshot in the README, on this site and in the store listings was
reshot on the new interface.

## What's still coming at 0.12.0

RRFS, and the remaining store submissions — Flathub, Snap, AUR, winget and
Homebrew all have working manifests in the repo, but none are published yet.

Found something broken? [Open an
issue](https://github.com/d4vid87/hookecho/issues) — the site, the product and
the time you were looking at is usually enough to replay it.
