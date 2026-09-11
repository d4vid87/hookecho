---
title: Alerts and notifications
description: Get told when weather is coming to a place you care about, without watching the screen.
order: 4
---

Alerting is built around **markers** — the places you've saved. Search a place in
the panel, and take **Save marker** when the map flies there. Mark one of them as
**home** and it gets a watch ring on the map.

## Choosing how you're told

**Settings → Alerts** picks the channels. You can use as many as you like:

- a **chime** from the app,
- a **desktop notification**,
- a phone push via [ntfy.sh](https://ntfy.sh),
- a webhook into **Discord, Slack or Matrix**,
- the warning **read aloud** through your system or browser voice.

Spoken alerts are scoped to warnings visible on the map, independently of saved-place push
notifications. Open **Settings → Alerts**, enable spoken alerts, and run the test warning. Browsers
require this activation once per page session and cannot speak after the page closes. HookEcho
shows a recovery message if a device voice is missing or playback fails. Emergency speech
interrupts routine warning speech; **Stop speech** and the master mute remain available.

## Choosing what you're told about

- **Warnings** covering a marker — or coming within its watch radius. Home
  defaults to 20 miles, and you're alerted when the polygon's *edge* reaches
  that ring, not only when it swallows your house.
- **Lightning** striking within about 15 km of a saved spot.
- **Rain arrival** — "rain in about 20 minutes", from the radar's own motion.
- **Rotation** and **debris signatures** detected on the live volume.

Severe warnings aren't all equal, and neither are the alerts. HookEcho reads the
NWS escalation tiers — CONSIDERABLE, then DESTRUCTIVE or an observed tornado,
then a **Tornado Emergency** — and a higher tier gets a pulsing polygon, a red
threat chip at the top of the alert list, a dedicated siren, and an
urgent-priority push. Whether it sounds during quiet hours depends on your
phone and receiving app settings.

## Where the storm actually is

Every warning carries the office's own storm-motion description, which HookEcho
parses into a vector: the warned storm as a dot and its projected path up to 30 minutes. When the
radar product publishes an error estimate, HookEcho uses it for the corridor and presents arrival
as a range. Without that support, it says the arrival cannot be estimated reliably. Tracks are
estimates, not official warnings or forecasts, and stale projections are suppressed.

## On your phone

Opt into the background service and Android notifies you with the app closed,
tiered watch / warning / emergency, and tapping through takes you to the storm.
A home-screen widget shows what's warned at your saved locations. See
[On your phone](/docs/on-your-phone/).

## On a machine you leave running

The desktop build keeps alerting from the tray. If you want the map itself
available elsewhere on your own network, `--serve` publishes it as a local HTTP
endpoint.

## Check your setup

Save a marker, select the channels you want, and check notification permission
in your operating system. Browser alerts need an open tab; Android background
alerts need the background service enabled. If delivery fails, follow the
[notification checklist](/docs/troubleshooting/#notifications-are-not-arriving).
