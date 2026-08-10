# User guide

Task by task: how to actually do the thing you opened the app for. The
[README](../README.md) is the tour and the feature list; this is the "how do I…"
The [browser demo](https://hookecho.pages.dev/) runs everything below except the
parts that need a filesystem or GPS.

## Getting oriented

First launch runs a four-card setup: home radar site, map and theme, how
warnings reach you. Its last card offers a 60-second tour of the live map; take
it or skip it, and re-run either from the sidebar's **App** section, `Ctrl+K`,
or **Settings → General**. After that the whole app is four regions — **left sidebar** (site, product,
tilt, every layer, window, tool and setting), **bottom bar** (the timeline),
**right edge** (the color scale for what you're looking at), and the map.

**If you remember one thing, remember `Ctrl+K`.** It searches the sidebar, and
Enter runs the top match. Every action in the app is in there, described in
plain English — products, layers, windows, workspaces, and "Fly to" for any
place name. You do not need to learn where anything lives.

## Watch a storm right now

1. Sidebar → site → pick the radar nearest the storm (or `Ctrl+K`, type the ID).
2. Product **Reflectivity (Z)** shows structure — where the rain and hail are.
3. Product **Velocity (V)** shows motion. Dealiasing is on by default, so a
   couplet reads as red against green rather than folding into nonsense.
4. Tilt: the sidebar's tilt row walks the VCP's elevation angles. 0.5° is what's
   near the ground; climb the tilts to see whether a storm leans.
5. The **LIVE** badge on the bottom bar means you're on the newest scan. Anything
   that moves you off it turns it off; click it to snap back.

Moving the map: drag to pan, scroll to zoom. On a trackpad, pinch to zoom and
swipe sideways to pan; on a touchscreen, two fingers do both.

**Is it rotating?** Velocity, 0.5°, look for tight inbound (green) next to
outbound (red) over a few gates. Then turn on **storm-relative velocity** in
Layer options — it subtracts the storm's own motion, so rotation stops hiding
inside the storm's translation.

**Is it hail?** Reflectivity over ~50 dBZ is a candidate; confirm with
correlation coefficient (CC) — hail is non-uniform, so CC drops. The **storm
attributes** table (`Ctrl+K` → "cells") lists every tracked cell with hail size,
tops and VIL, sorted; click a row to fly there.

**Is it a tornado?** The tornado-debris signature is the three together at low
tilt: a velocity couplet, high reflectivity, and a *hole* in CC where debris is
lofted. The app flags candidates, but the three panels are the reason.

## Look at four things at once

Split into panes and give each its own product with cameras linked — Z, V, CC
and ZDR on the same storm at the same second, or one product at four tilts. It's
one action in `Ctrl+K` ("four products" / "four tilts").

**Cross-section**: click two points on the map and get the storm in profile —
core, overhang, and how high the echo goes. Any product.

**3D**: the volume as an orbitable raymarch. Drag to orbit, scroll or pinch to
zoom, set
a dBZ floor ("Only above" → *Hail core*) so cores stand alone. Drop **Quality**
to Low on a phone or an integrated GPU; it only changes sampling, not the data.

## Replay something that already happened

The timeline reaches back to **June 1991**.

1. Right-click the **LIVE** badge → pick the archive day.
2. Scrub. Warning polygons and local storm reports come along — what was
   actually in force at the instant you're parked on, not today's.
3. `R` replays the in-RAM decode buffer instantly.
4. Export what you're watching: screenshot, GIF, or MP4 loop.

Pre-dual-pol volumes (before ~2012 at your site) simply have no ZDR/CC. That's
the data, not a bug.

The **event library** has curated historic events; your own bookmarks sit beside
them. And the **verification lab** scores an office's warnings for a day against
the reports that came in — POD, FAR, CSI, lead times, and the reports nobody
warned for.

## Get told without watching

- **Markers** are what alerts watch. Search a place in the sidebar → **Save marker**.
- Settings → Alerts: chime, desktop notification, [ntfy.sh](https://ntfy.sh)
  push, Discord/Slack/Matrix webhook. Triggers include warnings, lightning
  distance, rain arrival, debris signature and rotation.
- **Android**: opt into the background service and your phone notifies you with
  the app closed, tiered watch / warning / emergency, tapping through to the storm.
  The home-screen widget shows what's warned at your saved locations.
- **Desktop**: tray-based background alerting, and `--serve` if you want the map
  as an HTTP endpoint.

## Make it yours

- **Color tables**: Settings → Palettes imports GRLevelX `.pal` and `.pal3`
  files, per product, and exports what you've built. A v3 table loads as the
  part v2 shares — its colors and stops — so anything v3-only is ignored rather
  than refused.
- **Themes**: 13 built in.
- **Keyboard**: every binding is remappable in Settings.
- **Workspaces**: save a pane arrangement (sites, products, tilts, overlays) and
  restore it in one command. Three ship — Chase, National overview, Analysis.
- **Placefiles**: the GRLevelX format, rendered natively, with per-layer opacity.
- **Plugins**: any command that prints a placefile on stdout. See
  [plugins.md](plugins.md).
- **Sync** (optional): sign in with Google and settings, locations, placefiles,
  palettes and keys follow you to your other machines, in your own Drive.
  Setup: [sync.md](sync.md).

## Out in the field

- **Chase mode**: live GPS as a blue dot, a storm-relative HUD with closest
  approach and escape bearing.
- **Offline chase packs**: pre-download basemap tiles for the area you're
  driving into, before you lose signal. Radar still needs data; the map won't.
- **Position sharing**: opt in and every Hook Echo on the same network sees
  everyone's dot. Off-network, point it at a relay URL you host.
- **Streamer mode**: `F8` hides the chrome, `F9` auto-tours active warnings.

## When something looks wrong

- **Nothing loads.** The app talks to NOAA/NWS directly — check the network
  first. `--status` prints a per-feed report from the terminal.
- **A layer is empty and says it needs a key.** That's by design; it stays empty
  rather than nagging. Keys go in Settings, and stay on your machine.
- **The map is slow.** Lower the 3D quality preset, turn off animated wind
  (particles), and reduce the number of panes. On Android, close station cards
  you aren't reading.
- **Times look wrong.** Radar times read in *the selected radar's* local time,
  not yours and not Zulu. Settings → Units switches to UTC.
- **A forecast layer looks too good.** Check for the model banner — HRRR future
  radar, rotation tracks and smoke are model output, and the app labels them for
  as long as they're on.
- **Disk filling up.** Settings → Storage lists every cache against its cap,
  with clear and open buttons. Nothing cached is irreplaceable.

Still stuck, or something's wrong that isn't here — open an
[issue](../../../issues). Include the site, the product and the time you were
looking at; that's usually enough to replay it.
