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
- the warning **read aloud** through your system voice.

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
urgent-priority push that gets through your phone's quiet hours.

## Where the storm actually is

Every warning carries the office's own storm-motion description, which HookEcho
parses into a vector: the warned storm as a dot, its projected path at 15, 30,
45 and 60 minutes, and an ETA to each of your saved locations. That turns
"there's a warning near you" into "it's 22 minutes from home."

## On your phone

Opt into the background service and Android notifies you with the app closed,
tiered watch / warning / emergency, and tapping through takes you to the storm.
A home-screen widget shows what's warned at your saved locations. See
[On your phone](/docs/on-your-phone/).

## On a machine you leave running

The desktop build keeps alerting from the tray. If you want the map itself
available elsewhere on your own network, `--serve` publishes it as a local HTTP
endpoint.
