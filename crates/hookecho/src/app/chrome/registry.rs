//! The one action registry every surface searches: palette entries and their build.

use super::*;

impl HookEchoApp {
    /// Every layer/product/tool/window as a searchable, categorized row. Consumed by the layers
    /// panel (desktop slide-in + mobile sheet) and the Ctrl+K command palette.
    /// The action registry, rebuilt at most once a frame.
    ///
    /// Building it allocates ~150 owned strings, and up to five places render from it in the same
    /// frame (drawer, mobile sheets, legend, palette). Nothing can change it mid-frame — an
    /// action dispatched from one of those lists takes effect on the next one — so a per-frame
    /// memo is both free and honest.
    /// The command-palette action registry for this frame. Shared, not copied: the built list is
    /// a few hundred owned Strings, and several call sites want it in the same frame.
    pub(crate) fn palette_entries(&mut self) -> std::sync::Arc<[PaletteEntry]> {
        if let Some((frame, entries)) = &self.palette_cache {
            if *frame == self.frame_nr {
                return std::sync::Arc::clone(entries);
            }
        }
        let entries: std::sync::Arc<[PaletteEntry]> = self.palette_entries_build().into();
        self.palette_cache = Some((self.frame_nr, std::sync::Arc::clone(&entries)));
        entries
    }

    pub(crate) fn palette_entries_build(&mut self) -> Vec<PaletteEntry> {
        crate::prof_scope!("palette_entries");
        use crate::render::FieldLayer as FL;
        use AppWindow as W;
        use OverlayToggle as T;
        let mut out = Vec::new();
        // Reverse index from the live bindings, so a rebind relabels every row that shows a chip.
        let keys: Vec<(PaletteAction, String)> = crate::hotkeys::active(&self.settings)
            .iter()
            .filter_map(|b| match b.action {
                crate::hotkeys::BindableAction::Palette(p) => {
                    Some((p, crate::hotkeys::pretty(&b.shortcut)))
                }
                _ => None,
            })
            .collect();
        let mut push = |label: &str, category, desc, common, action, on| {
            // The panel groups rows by category, one collapsible per name in `CATEGORIES`. A row
            // with a category that isn't in that list draws nowhere: reachable from Ctrl+K, gone
            // from the panel. Cheaper to catch here than to notice a missing layer months later.
            debug_assert!(
                crate::ui::layers_panel::CATEGORIES.contains(&category),
                "registry row {label:?} has category {category:?}, which the panel does not draw"
            );
            out.push(PaletteEntry {
                label: label.to_string(),
                category,
                action,
                on,
                desc,
                common,
                key: keys
                    .iter()
                    .find(|(a, _)| *a == action)
                    .map(|(_, k)| k.clone()),
            })
        };

        // --- Radar products (the active pane's moment). ---
        let (cur_moment, cur_srv) = {
            let v = &self.views[self.active];
            (v.moment, v.srv)
        };
        // Every product, plus the storm-relative variant of velocity.
        let rows = crate::products::PRODUCTS
            .iter()
            .map(|p| (p.moment, false, p.name, p.blurb))
            .chain([(
                Moment::Velocity,
                true,
                "Storm-Relative Velocity",
                "Velocity with the storm's own motion subtracted out",
            )]);
        let have = self.available_moments();
        for (m, srv, label, desc) in rows {
            // A product this radar doesn't send is absent, not a row that paints nothing.
            if !have[m.index()] {
                continue;
            }
            let on = cur_moment == m && (m != Moment::Velocity || cur_srv == srv);
            push(
                label,
                "Radar",
                desc,
                true,
                PaletteAction::SetMoment(m, srv),
                Some(on),
            );
        }

        // --- National / model grids. ---
        for (layer, category, label, desc, common) in [
            (
                FL::Mrms,
                "National",
                "MRMS Mosaic",
                "Every radar in the country stitched into one picture",
                true,
            ),
            (
                FL::Mosaic,
                "National",
                "Radar Mosaic",
                "Every nearby radar's own base reflectivity, stitched seamlessly",
                true,
            ),
            (
                FL::Rotation,
                "National",
                "Rotation tracks",
                "Where rotation has passed over the last hour — the tornado-track map",
                true,
            ),
            (
                FL::Mesh,
                "National",
                "MESH hail",
                "Estimated largest hail size each storm is producing",
                true,
            ),
            (
                FL::Lightning,
                "National",
                "Lightning density (CG)",
                "Ground strikes only: NLDN cloud-to-ground density, averaged over a window you \
                 pick. Pair it with satellite lightning (GLM) to see total vs cloud-to-ground.",
                true,
            ),
            (
                FL::AzShear,
                "National",
                "AzShear (0–2 km)",
                "Low-level rotation strength, right now",
                false,
            ),
            (
                FL::PrecipRate,
                "National",
                "Rain rate",
                "How hard it is coming down right now, rather than how much has fallen",
                false,
            ),
            (
                FL::Qpe1h,
                "National",
                "QPE 1-hour",
                "How much rain has fallen in the last hour",
                false,
            ),
            (
                FL::Qpe24h,
                "National",
                "QPE 24-hour",
                "How much rain has fallen in the last day",
                false,
            ),
            (
                FL::PrecipType,
                "National",
                "Precip type",
                "Rain, snow, sleet or freezing rain at the surface",
                false,
            ),
            (
                FL::FlashFlood,
                "National",
                "FLASH flood ARI",
                "How rare this much rain is here — flash-flood risk",
                false,
            ),
            (
                FL::HailSwath,
                "National",
                "Hail swaths (24 h)",
                "Where hail has fallen over the past day",
                false,
            ),
            (
                FL::Vil,
                "National",
                "Digital VIL (L3)",
                "How much water the storm is holding aloft",
                false,
            ),
            (
                FL::EchoTops,
                "National",
                "Echo tops (L3)",
                "How tall the storm is",
                false,
            ),
            (
                FL::Hca,
                "National",
                "Hydrometeor class (L3)",
                "What the radar thinks it's seeing: rain, hail, debris",
                false,
            ),
            (
                FL::GlmFed,
                "National",
                "Flash density (GLM)",
                "Where the satellite flashes are densest \u{2014} the total-lightning field                  behind the individual dots, and where a lightning jump shows up first.",
                false,
            ),
            (
                FL::CompositeLocal,
                "Radar",
                "Composite reflectivity",
                "The strongest echo anywhere above each point, not just what this tilt cuts through",
                false,
            ),
            (
                FL::VilLocal,
                "Radar",
                "VIL (derived)",
                "Water held aloft, computed from this volume \u{2014} works in archive replay",
                false,
            ),
            (
                FL::VilDensity,
                "Radar",
                "VIL density (derived)",
                "Water aloft per unit storm depth \u{2014} high values mean large hail",
                false,
            ),
            (
                FL::EtopLocal,
                "Radar",
                "Echo tops (derived)",
                "Storm-top height at a threshold you pick, from this volume",
                false,
            ),
            (
                FL::HailMehs,
                "Radar",
                "Max hail size (derived)",
                "Largest hail this storm can be making, from the volume aloft (live only)",
                false,
            ),
            (
                FL::HailPosh,
                "Radar",
                "Severe hail probability (derived)",
                "Odds this storm is producing hail an inch or larger (live only)",
                false,
            ),
            (
                FL::GlobalMslp,
                "Models",
                "Global MSLP",
                "Surface pressure worldwide — GFS or ECMWF, your pick in Layer options",
                false,
            ),
            (
                FL::GlobalHeight500,
                "Models",
                "Global 500 hPa height",
                "The steering flow: where the troughs and ridges are, worldwide",
                false,
            ),
            (
                FL::GlobalTemp2m,
                "Models",
                "Global 2 m temp",
                "Surface temperature worldwide",
                false,
            ),
            (
                FL::GlobalWind10m,
                "Models",
                "Global 10 m wind",
                "Surface wind worldwide",
                false,
            ),
            (
                FL::GlobalPrecip,
                "Models",
                "Global moisture",
                "Precipitable water (GFS) or total precipitation (ECMWF)",
                false,
            ),
            (
                FL::ModelDiff,
                "Models",
                "Model difference",
                "Where two models disagree \u{2014} pick the field in layer options",
                false,
            ),
            (
                FL::Hrrr,
                "Models",
                "HRRR future radar",
                "Forecast radar picture for the next 18 hours (not observed)",
                true,
            ),
            (
                FL::UpdraftHelicity,
                "Models",
                "Future rotation tracks",
                "Where storms are forecast to rotate \u{2014} scrub the timeline to extend the swath",
                true,
            ),
            (
                FL::SnowAnalysis,
                "National",
                "Snowfall analysis",
                "How much snow actually fell \u{2014} pick the window in layer options",
                false,
            ),
            (
                FL::Snowfall,
                "Models",
                "Forecast snowfall",
                "How much snow is forecast to pile up \u{2014} scrub the timeline to add hours",
                false,
            ),
            (
                FL::Smoke,
                "Models",
                "Wildfire smoke",
                "Forecast smoke near the ground, from active fires",
                false,
            ),
            (
                FL::Cape,
                "Models",
                "CAPE",
                "How much fuel the atmosphere has for storms",
                false,
            ),
            (
                FL::Srh,
                "Models",
                "Storm-relative helicity",
                "How much spin the wind profile can feed a storm",
                false,
            ),
        ] {
            let on = self.fields.get(&layer).is_some_and(|s| s.show);
            push(
                label,
                category,
                desc,
                common,
                PaletteAction::ToggleField(layer),
                Some(on),
            );
        }
        for k in ContourKind::ALL {
            let label = format!("Contours: {}", k.label());
            let on = self.contour_kind == k;
            push(
                &label,
                "Models",
                "Draw this forecast field as labeled contour lines",
                false,
                PaletteAction::SetContours(k),
                Some(on),
            );
        }

        // --- Severe / obs / reference toggles. ---
        for (t, category, label, desc, common) in [
            (
                T::Cells,
                "Severe",
                "Storm cells",
                "Mark each storm the radar is tracking",
                true,
            ),
            (
                T::Alerts,
                "Severe",
                "NWS alerts",
                "Official warning and watch polygons",
                true,
            ),
            (
                T::Couplets,
                "Severe",
                "Rotation couplets",
                "Flag tight rotation that could produce a tornado",
                true,
            ),
            (
                T::Tbss,
                "Severe",
                "Hail spikes (TBSS)",
                "Flag three-body scatter spikes \u{2014} near-proof of large hail in the core \
                 they point away from",
                false,
            ),
            (
                T::ZdrColumns,
                "Severe",
                "ZDR columns",
                "Flag rain carried above the freezing level \u{2014} an updraft proxy that \
                 deepens before a storm intensifies",
                false,
            ),
            (
                T::Tds,
                "Severe",
                "TDS detection",
                "Flag lofted debris — a tornado is likely on the ground",
                true,
            ),
            (
                T::StormReports,
                "Severe",
                "Storm reports (LSR)",
                "What people on the ground actually reported today",
                true,
            ),
            (
                T::AlertPanel,
                "Severe",
                "Active alerts list",
                "Every alert in view, worst first (the sidebar's Alerts tab)",
                true,
            ),
            (
                T::Tracks,
                "Severe",
                "SCIT forecast tracks",
                "Where each tracked storm is projected to go",
                false,
            ),
            (
                T::ArrivalCones,
                "Severe",
                "Arrival-time cones",
                "When a storm is expected to reach points downstream",
                false,
            ),
            (
                T::Nowcast,
                "Severe",
                "Nowcast (echo extrapolation)",
                "Short-range radar forecast by sliding echoes forward",
                false,
            ),
            (
                T::Mds,
                "Severe",
                "Mesoscale discussions",
                "SPC's notes on where watches may be issued next",
                false,
            ),
            (
                T::ProbSevere,
                "Severe",
                "ProbSevere",
                "Per-storm probability of severe weather, from NOAA/CIMSS",
                false,
            ),
            (
                T::Metar,
                "Obs",
                "Surface obs (METAR)",
                "Temperature, dewpoint and wind at airports",
                true,
            ),
            (
                T::Webcams,
                "Obs",
                "Webcams (FAA + Windy)",
                "Look at the sky through a real camera \u{2014} FAA airports, plus the Windy \
                 network worldwide with a key in Settings",
                false,
            ),
            (
                T::Fires,
                "Severe",
                "Wildfires (WFIGS)",
                "Active fire perimeters and incident points from the interagency fire feed",
                false,
            ),
            (
                T::Aqi,
                "Obs",
                "Air quality (AirNow)",
                "EPA AQI at every monitor in view \u{2014} needs a free AirNow key in Settings",
                false,
            ),
            (
                T::Stations,
                "Obs",
                "Live station cards",
                "Cameras and live telemetry from surface stations, one floating card each",
                false,
            ),
            (
                T::Dat,
                "Obs",
                "Damage surveys (NWS DAT)",
                "What the survey crews found on the ground, rated point by point",
                false,
            ),
            (
                T::Spotters,
                "Obs",
                "Spotter Network",
                "Live positions of storm spotters near the radar",
                true,
            ),
            (
                T::Gauges,
                "Obs",
                "River gauges (NWPS)",
                "River levels and flood stage",
                false,
            ),
            (
                T::Sensors,
                "Obs",
                "Sensor dashboard",
                "Current conditions and 24-hour trends at the nearest station",
                false,
            ),
            (
                T::Hodo,
                "Obs",
                "VAD hodograph",
                "How the wind turns with height above the radar",
                false,
            ),
            (
                T::Tropical,
                "Obs",
                "Tropical (NHC)",
                "Hurricane tracks and forecast cones",
                false,
            ),
            (
                T::Mping,
                "Obs",
                "Crowd reports (mPING)",
                "What people outside say is falling: rain, snow, sleet, freezing rain",
                false,
            ),
            (
                T::Pireps,
                "Obs",
                "Pilot reports (PIREPs)",
                "What pilots actually flew through: turbulence, icing, cloud tops",
                false,
            ),
            (
                T::Recon,
                "Obs",
                "Recon flight track",
                "Hurricane-hunter observations: flight-level and surface wind, measured",
                false,
            ),
            (
                T::Aviation,
                "Obs",
                "Aviation (SIGMET/AIRMET)",
                "Hazard areas for pilots: turbulence, icing, low ceilings",
                false,
            ),
            (
                T::Tfr,
                "Reference",
                "Flight restrictions (TFR)",
                "Airspace you may not fly through: fires, stadiums, VIP movements, launches",
                false,
            ),
            (
                T::Blockage,
                "Reference",
                "Beam blockage (terrain)",
                "Shade where terrain cuts into this tilt's beam \u{2014} the radar's blind spots",
                false,
            ),
            (
                T::RadarSites,
                "Reference",
                "Radar sites",
                "Show every NEXRAD site; click one to switch radars",
                true,
            ),
            (
                T::Wind,
                "Models",
                "Wind (animated)",
                "HRRR 10 m wind as drifting particles \u{2014} forecast output, CONUS only",
                true,
            ),
            (
                T::GlmLightning,
                "Severe",
                "Satellite lightning (GLM)",
                "Total lightning (optical) — every flash the GOES mapper sees, in-cloud included, \
                 fading as it ages. The CG density layer is the ground-strike half.",
                true,
            ),
            (
                T::Fronts,
                "Reference",
                "Surface fronts (H/L)",
                "The cold, warm and stationary fronts from the national weather map",
                true,
            ),
            (
                T::RangeRings,
                "Reference",
                "Range rings",
                "Distance rings around the radar, every 50 km",
                true,
            ),
            (
                T::LinkCameras,
                "Reference",
                "Link pane cameras",
                "Pan and zoom every pane together",
                false,
            ),
            (
                T::MiniLoop,
                "Reference",
                "Mini loop window",
                // Honest about the caveat rather than promising something the compositor will
                // not do — see `mini_loop_viewport`.
                if cfg!(unix) && std::env::var_os("WAYLAND_DISPLAY").is_some() {
                    "A small window showing the active pane (Wayland cannot keep it on top)"
                } else {
                    "A small always-on-top window showing the active pane"
                },
                false,
            ),
        ] {
            let on = *self.overlay_flag(t);
            push(
                label,
                category,
                desc,
                common,
                PaletteAction::ToggleOverlay(t),
                Some(on),
            );
        }
        push(
            "Cycle basemap",
            "Reference",
            "Switch the map underneath the radar",
            true,
            PaletteAction::CycleBasemap,
            None,
        );
        push(
            "Open this view in Windy",
            "Reference",
            "Open windy.com in your browser, looking at the same place",
            true,
            PaletteAction::OpenInWindy,
            None,
        );
        push(
            "Copy link to this view",
            "Reference",
            "A link to this site, place, zoom, time and product \u{2014} opens Hook Echo here",
            true,
            PaletteAction::CopyViewLink,
            None,
        );
        push(
            "Save workspace",
            "Reference",
            "Remember this pane layout \u{2014} sites, products, tilts, overlays \u{2014} to restore later",
            true,
            PaletteAction::SaveWorkspace,
            None,
        );
        for (i, ws) in self.settings.workspaces.iter().enumerate() {
            push(
                &format!("Workspace: {}", ws.name),
                "Reference",
                "Restore this saved pane layout",
                true,
                PaletteAction::ApplyWorkspace(i),
                None,
            );
        }
        push(
            "Mute audio alerts",
            "Alerts",
            "Silence every chime and spoken warning without changing your sound choices",
            true,
            PaletteAction::ToggleMute,
            Some(self.settings.mute_alerts),
        );
        push(
            "Layers panel",
            "Reference",
            "The floating panel \u{2014} close it and nothing covers the map",
            false,
            PaletteAction::TogglePanel,
            Some(self.panel_open),
        );
        push(
            "About Hook Echo-WX",
            "Reference",
            "Version, links, and whether a newer release is out",
            false,
            PaletteAction::OpenWindow(AppWindow::About),
            None,
        );

        // --- Tools, windows, panes. ---
        let tool = self.tool;
        for (t, label, desc, common) in [
            (
                MapTool::Interrogate,
                "Tool: Interrogate",
                "Click anywhere to read the exact radar value",
                true,
            ),
            (
                MapTool::Measure,
                "Tool: Measure",
                "Drag to measure distance and bearing",
                true,
            ),
            (
                MapTool::Marker,
                "Tool: Drop marker",
                "Save a place — home, work, where you're headed",
                true,
            ),
            (
                MapTool::Forecast,
                "Tool: Point forecast",
                "Tap anywhere for that spot's 7-day and hourly forecast",
                true,
            ),
            (
                MapTool::Sounding,
                "Tool: Sounding",
                "Click a point for the model profile plus the nearest balloon sounding",
                false,
            ),
            (
                MapTool::CrossSection,
                "Tool: Cross-section",
                "Drag a line to slice the storm vertically",
                false,
            ),
            (
                MapTool::Chase,
                "Tool: Set chase location",
                "Tell the app where you are, for the chase readout",
                false,
            ),
            (
                MapTool::Climatology,
                "Tool: Tornado climatology",
                "How often tornadoes have hit this spot historically",
                false,
            ),
            (
                MapTool::AlertZone,
                "Tool: Watch zone",
                "Draw an area that alerts when a warning polygon touches it",
                false,
            ),
            (
                MapTool::Draw,
                "Tool: Draw",
                "Scribble on the map — circle the storm you're talking about",
                false,
            ),
        ] {
            push(
                label,
                "Tools",
                desc,
                common,
                PaletteAction::Tool(t),
                Some(tool == t),
            );
        }
        for (w, label, desc, common) in [
            (
                W::Site,
                "Radar site…",
                "Pick which radar you're watching",
                true,
            ),
            (
                W::Settings,
                "Settings…",
                "Theme, units, time display, alert sounds",
                true,
            ),
            (
                W::Markers,
                "Location markers…",
                "Manage your saved places and their alerts",
                false,
            ),
            (
                W::Events,
                "Event library…",
                "Jump to a famous storm and watch it replay",
                false,
            ),
            (
                W::Digest,
                "Storm digest…",
                "A plain-language summary of what's happening now",
                false,
            ),
            (
                W::Afd,
                "Forecast discussion (AFD)…",
                "What the local forecast office is writing",
                false,
            ),
            (
                W::Placefiles,
                "Placefile manager…",
                "Add GRLevelX placefile overlays",
                false,
            ),
            (
                W::LayerManager,
                "Layer manager…",
                "Reorder and set opacity for every layer",
                false,
            ),
            (
                W::Palettes,
                "Color-table editor…",
                "Change the colors a product is drawn with",
                false,
            ),
            (
                W::StormTable,
                "Storm attributes…",
                "Every tracked storm in one sortable table \u{2014} hail size, tops, rotation",
                true,
            ),
            (
                W::AlertRules,
                "Alert rules\u{2026}",
                "Tell the app what is worth interrupting you for",
                false,
            ),
            (
                W::Help,
                "Help \u{2014} shortcuts, glossary, tour\u{2026}",
                "What a TDS, a hail spike or a ZDR column actually is, and every keyboard shortcut",
                false,
            ),
            (
                W::Verify,
                "Warning verification…",
                "Score an office's warnings against what actually happened",
                false,
            ),
            (
                W::Cappi,
                "CAPPI slice…",
                "See the storm at one constant altitude",
                false,
            ),
            (
                W::Volume3d,
                "3D volume…",
                "Rotate the storm in three dimensions",
                false,
            ),
            (
                W::Climatology,
                "Tornado climatology…",
                "Historical tornado tracks for this area",
                false,
            ),
            (W::Setup, "Set up again…", "Pick your home radar again", false),
            (
                W::Tour,
                "Take the tour…",
                "A 60-second walk through the app's controls",
                false,
            ),
        ] {
            // The raymarch samples a `texture_3d<u32>`, which WebGL2 does not guarantee; on wasm
            // the entry would open a black window.
            // ponytail: revisit when the web build is webgpu-only.
            if cfg!(target_arch = "wasm32") && w == W::Volume3d {
                continue;
            }
            let on = None;
            push(
                label,
                "Tools",
                desc,
                common,
                PaletteAction::OpenWindow(w),
                on,
            );
        }
        push(
            "Compare 4 tilts",
            "Tools",
            "Four panes of this product at four heights, cameras linked",
            true,
            PaletteAction::AllTilts,
            None,
        );
        let panes = self.views.len();
        for n in [1usize, 2, 4] {
            push(
                &format!("{n} pane{}", if n == 1 { "" } else { "s" }),
                "Tools",
                "Split the window to watch several radars or products at once",
                false,
                PaletteAction::SetPanes(n),
                Some(panes == n),
            );
        }
        push(
            "Reload",
            "Tools",
            "Fetch the latest data again",
            true,
            PaletteAction::Reload,
            None,
        );
        push(
            "Jump to live",
            "Tools",
            "Snap back to the newest scan",
            true,
            PaletteAction::GoLive,
            None,
        );
        push(
            "Instant replay (DVR)",
            "Tools",
            "Replay the scans already in memory",
            false,
            PaletteAction::InstantReplay,
            None,
        );
        out
    }
}
