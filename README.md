<p align="center"><img src="assets/brand/hookecho-logo.png" alt="HookEcho" width="300"></p>

# HookEcho

**Free weather radar. A complete set of tools to look closer.**

Follow rain and storms with national MRMS radar, Storm Prediction Center outlooks, flexible layouts, and four-tilt comparisons. Use the menu to find weather layers, observations, forecasts, and analysis tools.

No account. No ads. No API key. Free and open source.

[Open live radar](https://app.hookecho.io/) · [Download the app](https://github.com/d4vid87/hookecho/releases) · [Visit the website](https://hookecho.io/) · [Get help](https://github.com/d4vid87/hookecho/issues)

![National to local: current HookEcho menu, MRMS, SPC, layouts, and four tilts](docs/shots/national-to-local/tour.gif)

*An 18-second tour that leads with MRMS and SPC before moving into storm detail. Real interface screenshots captured October 26, 2026. Storm views use the KTLX archive from May 20, 2013; MRMS and SPC show conditions at capture time.*

## Explore the highlights

### Menu & features

**Find the tool. Keep the map.** Search weather layers, observations, forecast models, and analysis tools. Open storm attributes, alerts, and map controls from one organized menu.

![HookEcho menu with searchable categories and severe-weather tools](docs/shots/national-to-local/features.png)

*Current HookEcho menu · captured October 26, 2026 · archived KTLX backdrop.*

### MRMS national radar

**Start with the whole country.** Use the Multi-Radar / Multi-Sensor mosaic to follow rain and storms across the United States, then switch to a radar site for a closer look.

![HookEcho national overview displaying the MRMS reflectivity mosaic](docs/shots/national-to-local/mrms.png)

*MRMS national reflectivity · October 26, 2026 · conditions at capture time.*

### Storm Prediction Center

**Put the outlook on the map.** View SPC Day 1–3 convective outlooks. For Day 1, compare categorical risk with tornado, wind, and hail probabilities.

![HookEcho SPC outlook library with day and hazard controls beside the national map](docs/shots/national-to-local/spc.png)

*SPC Day 1 categorical outlook · October 26, 2026 · conditions at capture time.*

### Flexible layouts

**Choose the view for the job.** Move between Radar, Chase, National overview, and Analysis. Work with one, two, or four panes, and link map positions and timelines when comparing scans.

![HookEcho navigation showing Radar, Chase, National overview, Analysis, and comparison layouts](docs/shots/national-to-local/layouts.png)

*Current HookEcho layout menu · October 26, 2026 · archived KTLX backdrop.*

### Compare four tilts

**Look above the lowest scan.** Show four reflectivity tilts together to examine storm structure at different elevations. Four tilts and four radar products answer different questions.

![Actual Compare 4 tilts view with four populated reflectivity panes](docs/shots/national-to-local/four-tilts.png)

*Compare 4 tilts · KTLX archive, May 20, 2013 · captured October 26, 2026.*

## Radar products and playback

Step through radar scans or play a loop. Compare reflectivity, velocity, correlation coefficient, and differential reflectivity in linked panes. Four products compare different measurements; four tilts compare elevations. Check the scan time and radar geometry when interpreting either view.

[Read the analysis guide](https://hookecho.io/docs/storm-analysis/) · [See the full feature reference](https://hookecho.io/features/)

## On your phone

Keep the map visible and open the controls when you need them. Use the browser app or install the Android build.

<img src="docs/shots/android/map.jpg" alt="HookEcho Signal Deck on Android" width="320">

[Phone instructions](https://hookecho.io/docs/on-your-phone/)

## Start here

The quickest way to use HookEcho is to [open the radar in your browser](https://app.hookecho.io/).
There is nothing to install.

On your first visit:

1. Let HookEcho use your location to choose the closest radar, or search for a place.
2. The newest radar picture opens automatically.
3. Press **Play** to watch the rain and storms move.
4. Press **LIVE** at any time to return to the newest picture.

You can install the website on a phone, tablet, or Chromebook from your browser's **Add to Home
Screen** or **Install app** option.

## Install the app

The installed app is useful when you want a dedicated window, stronger alerts, saved workspaces,
or deeper weather tools.

Open the [releases page](https://github.com/d4vid87/hookecho/releases), then choose the
file for your device.

| Your device | File to choose | What to do |
|---|---|---|
| Windows 10 or 11 | `HookEcho-setup-x86_64.exe` | Open the file and follow the installer. |
| Ubuntu or Debian Linux | `HookEcho-amd64.deb` | Double-click the file, or use the short command below. |
| Other 64-bit Linux computers | `HookEcho-x86_64.AppImage` | Make the file runnable, then open it. |
| Android 10 or newer | `HookEcho-arm64-v8a.apk` | Open the file and allow installation from your browser or Files app when asked. |
| Mac | `HookEcho-macos.zip` | Unzip it and open HookEcho. The Mac version is still experimental. |

For Ubuntu or Debian:

```sh
sudo apt install ./HookEcho-amd64.deb
```

For the AppImage:

```sh
chmod +x HookEcho-x86_64.AppImage
./HookEcho-x86_64.AppImage
```

Windows or Mac may warn that HookEcho is from an unknown developer. On Windows, choose **More
info → Run anyway**. On Mac, open **System Settings → Privacy & Security → Open Anyway**.

HookEcho is currently in beta. Please [report anything that does not work](https://github.com/d4vid87/hookecho/issues/new).

## What you can do

- Watch live rain and storms move across the map.
- See official weather warnings and open the full message.
- View lightning, rainfall, wind, clouds, smoke, and other useful layers.
- Tap anywhere for the local forecast.
- Save important places and receive nearby warning alerts.
- Look ahead with future radar.
- Replay major storms and past radar scans.
- Change the map style, colors, units, and alert sounds.
- Compare several radar views when you want more detail.

Everyday controls stay simple. The deeper weather tools remain available without crowding the
main map.

## Find your way around

- **Radar | Analyst:** choose one map or resume your analysis arrangement.
- **Search or Ctrl+S:** find a place, radar, setting, or weather layer.
- **Play button or Space:** start and stop recent radar animation.
- **Escape:** hide all map controls; press it again to bring them back.
- **Timeline:** move backward through recent scans or forward into future radar.
- **LIVE button:** jump back to current conditions.
- **Radar section:** site, product, tilt, threshold, and custom locations.
- **Overlays section:** storm tracks, MRMS, storm attributes, and SPC outlook.
- **Alerts section:** see warnings covering the area on screen. It stays closed until you open it.
- **Tools section:** search the product catalog and open saved workspaces.
- **Settings gear:** open Settings directly from the fixed header.

![HookEcho showing the current Radar section over a historic storm](docs/shots/radar-mode.jpg)

## Made for different kinds of weather watchers

**For everyday use:** open the map, see whether rain is coming, and read warnings without learning
radar terms.

**For outdoor plans:** save home, work, events, or travel stops and watch the weather near each
place.

**For weather enthusiasts:** compare radar products, split the screen, inspect storm structure,
use forecast layers, and replay historic events. These tools stay tucked away until you ask for
them.

## Alerts

HookEcho can warn you when official weather alerts approach a saved place. Depending on your
device, alerts can appear in the app, browser, phone notification, email, Discord, Telegram, or
another service you connect.

The app explains what the warning is, where it is, and when it expires. It can read warnings in
the visible map area through the bundled Piper voice; emergency speech interrupts routine notices.

## Privacy

HookEcho has no user accounts, ads, or app tracking. Radar, forecasts, and warnings come from
public weather services. Your saved places and preferences stay on your device unless you choose
to connect a syncing or alert service.

## Need help?

- **The map chose the wrong area:** search for your town, postcode, or nearest radar.
- **Radar looks old:** press **LIVE** and check the time shown at the bottom.
- **A layer is missing:** open **Layers** and turn it on.
- **Location was blocked:** search manually; location access is optional.
- **The installed app will not open:** try the browser version first, then report your device and operating system.

Questions are welcome in [GitHub Issues](https://github.com/d4vid87/hookecho/issues) or on
[Discord](https://discord.gg/VNMW2Gyg4V).

## For technical users

The [technical reference](docs/technical-reference.md) covers advanced radar products, data
sources, workspaces, remote control, Home Assistant, MQTT, plugins, command-line options,
development, and testing.

- [User guide](docs/GUIDE.md)
- [Radar and Analyst interface](docs/INTERFACE.md)
- [Weather-data guide](docs/DATA.md)
- [Plugin guide](docs/plugins.md)
- [Sync guide](docs/sync.md)
- [Contributing](CONTRIBUTING.md)
- [Project roadmap](ROADMAP.md)

## License

HookEcho is free and open source under the [GNU GPL v3](LICENSE).
