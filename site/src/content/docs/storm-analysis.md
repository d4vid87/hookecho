---
title: Advanced storm analysis
description: Build a four-product workspace, verify the source and scan time, and investigate storm structure with linked views.
order: 2.5
---

Start with one storm and a question. Use **Analyst** to compare products, check their actual source times, and revisit the same feature through successive scans. This workflow uses the existing Radar and Analyst controls; it does not assume every item in the development roadmap has shipped.

## Build the workspace

1. In **Radar**, select the radar site nearest the storm and let the scan load. Choose a live or archive time deliberately.
2. Select **Analyst**, then **Preset → Tornado** for a four-product arrangement. You can also choose four panes and set each product yourself.
3. Compare **reflectivity**, **velocity**, **correlation coefficient (CC)**, and **differential reflectivity (ZDR)**. Select a pane before changing its product or tilt.
4. Enable **Maps** and **Time** in the Analyst toolbar to link the views. Open **Inspector** for values and the actual source time.
5. Keep the same geographic feature in view as you inspect each pane. Return to **Radar** when you want a single-map overview.

![Actual HookEcho four-pane Analyst workspace with the Inspector](/shots/analyst-mode-20260923.webp)

*Archived KTLX example, not current weather. Reflectivity, velocity, CC, and ZDR provide different measurements of the same storm.*

For pane selection, workspace persistence, and phone behavior, see [Radar and Analyst modes](/docs/radar-and-analyst/).

## Verify the scan

Before comparing a pattern, establish what each pane is showing.

| Check | What to record |
| --- | --- |
| Source | Radar site and product. A different site views the storm from a different direction. |
| Scan time | Actual source time in the Inspector and timeline. Linked controls do not remove the need to compare timestamps. |
| Product and units | Read the product name and its legend. A palette color is not a measurement on its own. |
| Tilt and range | Record the selected elevation angle and the storm's distance from the radar. |
| Processing | Note storm-relative velocity, thresholds, smoothing, or other enabled settings when comparing views. |

If the app reports old data, or a pane has no valid scan, resolve that before interpreting apparent changes. Use [Troubleshooting](/docs/troubleshooting/) for loading and freshness problems. **LIVE** returns the timeline to the newest available scan; always check its timestamp.

## Investigate rotation and debris

Begin with storm shape in reflectivity, then examine the corresponding velocity pattern. Compare successive scans rather than relying on one frame. Storm-relative velocity is another view of the same motion with the storm's estimated translation removed; note when it is enabled.

A low-CC area warrants comparison with both rotation and reflectivity at the same location. Low CC alone does not identify tornado debris. A debris signature can be less evident for weaker tornadoes or storms far from the radar. See the [NWS tornado debris example](https://www.weather.gov/lmk/nws_radar_dualpol_tordebris) for the combined interpretation.

> Follow official NWS warnings. Do not wait for a radar signature before acting on a warning, and do not treat a detection label as a confirmed tornado report.

## Examine hail and vertical structure

Compare reflectivity with ZDR and CC. These products describe different properties of the returned signal; use them together instead of treating a single color or cutoff as a hail-size estimate. The [NWS dual-polarization reference](https://www.weather.gov/jan/dualpolupgrade-products) explains what the products measure.

Move through the available tilts and keep the feature geographically aligned. Remember that beam height increases with distance from the radar. Differences across tilts can reflect sampling geometry as well as storm structure.

Open **Overlays → Storm attributes** for the available tracked-cell measurements. Compare those indicators with the radar panes and source times. Read [Reading the radar](/docs/reading-the-radar/) for product definitions and interpretation limits.

## Add context and preserve the view

- Open **Alerts** to read warnings in the map area. Use [Alerts and notifications](/docs/alerts-and-notifications/) to configure saved-place alerts.
- Review neighboring scans on the timeline to distinguish a persistent feature from a momentary one.
- Save a **Workspace** in **Tools** to retain your arrangement across restarts. Record the site, scan time, products, tilts, and relevant settings with your analysis.
- For a historical case, follow [Replay something that already happened](/docs/getting-started/#replay-something-that-already-happened). Check which products exist for that radar and date.

## Where the roadmap goes next

The [U.S. Analyst roadmap](https://github.com/d4vid87/hookecho/blob/main/docs/us-analyst-roadmap.md) describes the longer-term workstation direction: explicit data provenance, synchronized analysis, deeper radar products, satellite and model comparisons, and repeatable historical verification.

That document includes future work. Use the [current issue roadmap](/roadmap/) and [changelog](/changelog/) to distinguish planned capabilities from shipped changes; this guide covers the documented workflow available now.
