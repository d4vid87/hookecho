# README screenshots

`shoot.sh` regenerates everything in `docs/shots/`. It runs the real release binary on a nested
Xvfb display, stages each scene with the `HOOKECHO_GOTO` deep link, drives the UI with xdotool,
and captures with ImageMagick.

```sh
./scripts/shots/shoot.sh archive       # historic events — reproducible on any day
./scripts/shots/shoot.sh scout         # find a radar with storms on it right now
./scripts/shots/shoot.sh live          # the three shots that need real weather
./scripts/shots/shoot.sh check         # cross-reference + size budget, no display needed
./scripts/shots/shoot.sh velocity      # re-shoot one scene
```

## Why it is built this way

**Nested Xvfb, not your desktop.** xdotool key injection does not reach the window under KDE
Wayland/XWayland — the mouse works and the keys silently do not. There is also no window manager
on Xvfb, so `windowsize`, `windowmove` and `windowfocus --sync` all have to be called by hand;
skip the focus call and every keystroke goes nowhere.

**`HOOKECHO_GOTO`, not clicking the Event Library.** Staging a historic storm by driving the UI
means hard-coding the pixel coordinates of a window's rows. That breaks the first time a layout
changes, and it fails by shooting a plausible-looking wrong frame rather than by erroring.

**Hash-stabilisation, not sleeps.** Archive volumes come off S3 in anything from two seconds to a
minute. `wait_settle` captures the frame every few seconds and moves on once two consecutive
captures are identical, with a floor and a cap.

**A scratch profile.** `XDG_CONFIG_HOME` and `XDG_DATA_HOME` point into `$TMPDIR`, so the shoot
neither reads your real settings nor gets coloured by them. The tile cache is deliberately the
real one — basemap tiles are already paid for. The Mapbox token is copied out of your config at
run time and never leaves the scratch profile; the committed template has an empty string, and
`check` fails if that ever stops being true.

## The scene list

| Scene | Data | What it has to show |
|---|---|---|
| `hero.gif` | Moore, OK — May 20 2013 | The hook echo organising, warning polygon on |
| `reflectivity` | Tuscaloosa — Apr 27 2011 | A textbook supercell, centred |
| `velocity` | El Reno — May 31 2013 | The velocity couplet, unmistakable |
| `alltilts` | Moore | A 60+ dBZ core in all four panes |
| `xsection` | Moore | The vault, in a panel that is not empty |
| `alerts` | Tuscaloosa | The real archived tornado-warning stack |
| `products` | Joplin — May 22 2011 | The picker over a storm, not over black |
| `layers` | Mayfield — Dec 11 2021 | Whole rows, opaque card |
| `forecast` | Moore | A populated point forecast |
| `tropical` | Ian — Sep 28 2022 | A full eyewall |
| `stormtable` | **live** | A populated table — no fallback, needs a storm day |
| `fronts` | **live** | Fronts crossing the frame |
| `glm` | **live** | Lightning on an active storm |

Storm anatomy comes from the archive on purpose. The previous set was shot in one session at a
coastal radar on a quiet day, and it showed: empty panels, half-empty panes, and a lot of ocean.

## Before committing a re-shoot

Look at every frame. The script cannot tell a good screenshot from a bad one.

- The subject is centred and fills roughly 60% of the frame.
- No panel is showing an empty state — no "No alerts in view.", no blank cross-section.
- The legend card is fully inside the frame and nothing reads through it.
- No `T+15m`/`T+30m` track labels unless the shot is about forecast tracks.
- The product and timestamp in the chrome match what the README caption claims.
- It still reads at README thumbnail width (~420 px), not just at full size.

Then `./shoot.sh check` for the mechanical part, and confirm the whole set shares one basemap.
The theme is pinned to Dark in `settings.template.json` — the shoot no longer inherits whatever
theme your own config happens to be on, so the set can't drift a shot at a time.
