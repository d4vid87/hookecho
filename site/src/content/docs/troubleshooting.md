---
title: Troubleshooting
description: Step-by-step checks for missing data, slow playback, notification problems and useful bug reports.
order: 7
---

## The map opens, but radar is missing

1. Check the selected radar, product and time. Press **LIVE** to leave archive
   playback, and try **Reflectivity (Z)** at the lowest available tilt.
2. Read the scan timestamp. No coloured echoes with a recent timestamp can mean
   there is no precipitation; it does not necessarily mean the app is broken.
3. Try a neighbouring radar. A single radar can have missing or delayed scans.
4. Check your connection. HookEcho needs network access to public weather feeds.
   If several sites fail, try again after checking your network or VPN settings.

For an old event, not every radar, product or time is available. CC and ZDR are
not present in volumes recorded before that radar's dual-polarization upgrade.

## The browser app is blank or slow

Reload the page and try a current browser. Close extra panes and turn off 3D or
animated wind particles before adding more layers. If the full app still cannot
run on your device, try [HookEcho Lite](https://app.hookecho.io/lite/).

Lite offers a smaller set of radar features. It does not provide archive replay;
see [the browser guide](/docs/in-your-browser/) before switching.

## The radar is showing an old time

Press **LIVE** on the timeline. Archive playback and a manually selected scan
show past conditions. If the newest scan itself is old, try another radar and
check your connection.

Radar times use the selected radar's local time by default. **Settings → Units**
can switch them to UTC. When reporting a problem, include the date and time zone
as well as the clock time.

## Notifications are not arriving

1. Confirm that the place is saved as a marker and that your alert rules cover it.
2. Check the channels selected in **Settings → Alerts**.
3. Check notification permission and sound settings in your operating system.
4. Keep the browser tab open for browser alerts. For Android alerts with the app
   closed, enable the background alert service and check Android notification
   and battery restrictions for HookEcho.
5. For an external channel such as ntfy or a webhook, check its destination and
   the receiving app's permissions too.

Delivery depends on the device, network and receiving service. Urgent priority
does not guarantee that a phone will sound through every quiet-hours setting.
See [Alerts and notifications](/docs/alerts-and-notifications/) for setup.

## Storage use keeps growing

Open **Settings → Storage** to inspect caches and their limits. Clear the cache
that is taking up space using its own control. Cached weather data may need to
be downloaded again. Avoid deleting the entire app profile or clearing browser
site data as a first step: those can also remove saved settings.

## Report a problem we can reproduce

[Open a GitHub issue](https://github.com/d4vid87/hookecho/issues/new) and include:

- HookEcho version or build, operating system, and browser version if applicable.
- Radar ID, product and tilt, and whether you were viewing live or archived data.
- Date, time and time zone of the scan.
- The steps you took, what you expected, and what happened instead.
- A screenshot or the relevant error text, if available.

Remove API keys, webhook tokens and private saved-location details before
posting screenshots or logs. For a usage question, ask in
[Discord](https://discord.gg/VNMW2Gyg4V).
