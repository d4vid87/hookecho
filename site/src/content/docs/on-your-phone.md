---
title: On your phone
description: The Android app — the same radar, in your pocket and out in the field.
order: 6
---

The Android build is the same Rust application as the desktop one, with an
interface built for a phone. Install it from
[Releases](https://github.com/d4vid87/hookecho/releases/latest) —
`HookEcho-arm64-v8a.apk`, arm64, Android 10 or newer. See
[Install](/docs/install/) for the two ways to sideload it.

## Getting around

The map opens with controls closed. Tap the radar name above the map to change
its product, site, tilt, or threshold. The timeline stays above the bottom
navigation so you can scrub without opening a menu.

The bottom bar opens one focused sheet at a time. Tap its destination again,
tap **×**, or swipe the handle down to return to the map:

- **Radar** has the site, product, tilt, threshold, and advanced settings.
- **Layers** has storm tracks, MRMS, storm attributes, and the SPC outlook.
  Tap the selected outlook day again to remove it.
- **Alerts** shows the warning count and list. New warnings never open the
  sheet automatically; the map warning switch is inside Alerts.
- **More** has Analyst, saved locations, workspaces, Settings, and the
  walkthrough.

Search and custom locations are one tap from the buttons above the map.
On Android, the system Back gesture closes search or the open sheet before
leaving HookEcho.

- **Long-press** the map to inspect what's under your finger.
- **Double-tap and drag** up or down to zoom with one hand.
- **Open Analyst from More** to use presets and linked panes. Pane tabs show
  one map at a time on a phone. Tap **Back to Radar** to restore your radar view.
- The app buzzes on scrubber ticks, sheet snaps, and new warnings.

## Notifications with the app closed

Turn on the background alert service and the phone tells you about warnings for
your saved locations whether or not HookEcho is running, tiered watch / warning
/ emergency, with a tornado emergency arriving at urgent priority. Tapping a
notification opens the app at that storm. Android is asked for notification
permission only at the moment you switch an alert feature on — never at install.

A **home-screen widget** shows current radar and what's warned at your saved
locations, and there's a **quick-settings tile** for a one-swipe look.

## Out in the field

- **Chase mode** puts your live GPS position on the map as a blue dot, with a
  storm-relative readout: closest approach, and an escape bearing.
- **Offline chase packs** pre-download the basemap tiles for an area before you
  drive into it. Radar still needs a data connection; the map underneath it
  won't.
- **Position sharing** — opt in, and every HookEcho on the same network sees
  everyone's dot. Off-network, point it at a relay URL you host yourself.

## Keeping it fast

Phones are not workstations. Drop the 3D quality preset, turn off the animated
wind particles, and close station cards you aren't reading. There's a
battery-saver mode that slows the polling and repaint cadence when you're
running on what's left of the battery.
