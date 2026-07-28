# Plugins

A plugin is any program that prints a [placefile](https://www.grlevelx.com/manuals/gis/files_places.htm)
on stdout. That's the whole interface: no library to link, no SDK, no build step, and no language
requirement — the two examples in `plugins/` are a Python script and twenty lines of shell.

Add one in **Placefile Manager → Plugins**: a name, and the command to run. The app runs it on
your chosen cadence and draws whatever it prints, through exactly the same pipeline network
placefiles use — thresholds, `TimeRange` gating, icons and all.

Desktop only. Android can't execute a program you dropped in app storage.

## What your plugin is told

Four environment variables, set before every run:

| Variable | Example | Meaning |
|---|---|---|
| `HOOKECHO_SITE` | `KTLX` | The radar the active pane is on. |
| `HOOKECHO_BBOX` | `-98.4,34.6,-96.1,36.2` | The visible map, `min_lon,min_lat,max_lon,max_lat`. |
| `HOOKECHO_TIME` | `2013-05-20T20:15:00+00:00` | The instant on screen. **Archive-aware** — scrub back and your plugin is asked about then, not now. |
| `HOOKECHO_PRODUCT` | `REF` | The moment being displayed. |

Answering for `HOOKECHO_TIME` rather than for "now" is what makes a plugin work during an event
replay, which is most of the point.

## What your plugin returns

Standard placefile text on stdout, exit 0. Everything the app's parser supports is available:
`Color`, `Threshold`, `TimeRange`, `Line`, `Polygon`, `Text`, `Icon`/`IconFile`, `Object`
(screen-anchored, pixel-sized symbols) and `Triangles` (per-vertex-coloured mesh).

```
Title: My overlay
RefreshSeconds: 60
Color: 255 200 0
Line: 3, 0
 35.20, -97.40
 35.30, -97.30
End:
```

Anything on stderr from a failing plugin is shown in the Placefile Manager, so a plugin that dies
tells you why instead of quietly not appearing.

## Rules of the road

- **Output cap:** 10 MB. Past that the read stops.
- **Timeout:** 10 s, then the process is killed. Refreshes are seconds to minutes apart; a plugin
  that can't answer in ten seconds is hung, not slow.
- **Empty output is an error.** A plugin that runs cleanly and draws nothing is nearly always
  broken, and saying so beats a silently missing overlay.
- **The command is split on whitespace, not run through a shell.** No quoting rules to get wrong.
  If you want a pipeline, ask for one on purpose: `sh -c "…"`.

## Trust

Plugins run as you, with your privileges, because you typed the command — the same trust you give
a shell alias. The cap and the timeout are hygiene against a plugin that wedges or floods, **not**
a sandbox: a hostile plugin is not contained. Don't add a plugin you wouldn't run by hand.

## Examples

- [`plugins/metar_flightcat.py`](../plugins/metar_flightcat.py) — colours every airport in view by
  flight category from live METARs, using `Object:` so the symbols stay the same size at any zoom.
- [`plugins/chase_gps.sh`](../plugins/chase_gps.sh) — draws your own GPS track from a CSV log, with
  a `Triangles:` arrowhead at the head of the trail.
