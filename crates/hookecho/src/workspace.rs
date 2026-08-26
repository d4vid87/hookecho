//! Saved pane layouts.
//!
//! Rebuilding "KTLX reflectivity beside KDMX velocity, storm-relative, with these overlays on"
//! costs a dozen clicks, and it's the layout people rebuild every time they sit down. A workspace
//! is a snapshot of that arrangement, restored in one command.
//!
//! It stores state, not actions: the command palette's actions all target the active pane, so a
//! recorded action list has no way to say "and pane two looks like this". What it deliberately
//! doesn't capture is anything tied to the moment rather than the arrangement — the archive
//! playhead, open windows, per-moment thresholds.
//!
//! Per-pane display thresholds and field layers ride along, since a pane that shows one field
//! above 50 dBZ beside another that shows a different one is exactly the arrangement worth saving.

use crate::view::MapView;

/// One saved pane arrangement.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Workspace {
    pub name: String,
    pub panes: Vec<PaneSnap>,
    /// Which pane was focused.
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub link_cameras: bool,
    /// Overlay toggles that were on, by slug — the same names `Settings::overlays_on` uses, so an
    /// unknown one from a newer build is skipped rather than fatal.
    #[serde(default)]
    pub overlays_on: Vec<String>,
    /// Fill panes that have no site from whatever site is on screen when the workspace is
    /// applied. Set on the starter workspaces, which describe an arrangement rather than a place:
    /// "two panes of the same radar" has to mean *your* radar, not one shipped in a default file.
    #[serde(default)]
    pub adopt_site: bool,
    /// National field layers that were on, by slug. Same forward-compatibility rule as the
    /// overlays; `default` so a workspace written before this field existed still loads.
    #[serde(default)]
    pub fields_on: Vec<String>,
    /// What the chrome looked like: which panel was showing, which drawer page was open. `None`
    /// on a workspace written before this field existed, and on the starters — both mean "leave
    /// the chrome where the user left it" rather than "close everything", which is what a
    /// defaulted struct would have meant.
    #[serde(default)]
    pub chrome: Option<Chrome>,
}

/// The floating chrome's state, as far as it is worth restoring: which surface was showing, not
/// how far it was scrolled.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Chrome {
    #[serde(default)]
    pub panel_open: bool,
    /// The panel's Alerts tab rather than Data.
    #[serde(default)]
    pub alerts_tab: bool,
    #[serde(default)]
    pub basemap_open: bool,
    /// Title of the drawer page on top, if any — matched back to a window by
    /// `app::chrome::window_for_page`. A title this build no longer has is skipped, same rule as
    /// the overlay slugs.
    #[serde(default)]
    pub drawer: Option<String>,
}

/// One pane's state. Camera as lon/lat/zoom, basemap as its slug: both survive a file written by
/// a different build.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaneSnap {
    pub site: Option<String>,
    pub moment: wxdata::level2::Moment,
    pub tilt: usize,
    #[serde(default)]
    pub srv: bool,
    pub basemap: String,
    pub lon: f64,
    pub lat: f64,
    pub zoom: f64,
    /// Field layers this pane drew, by slug. Same forward-compatibility rule as the overlays: a
    /// slug this build does not have is skipped. Empty on a snapshot written before per-pane
    /// layers existed, which is read as "fall back to the workspace-wide list".
    #[serde(default)]
    pub fields_on: Vec<String>,
    /// Enabled display thresholds, as `(moment, physical value)`. Only the enabled ones are
    /// stored: a threshold that was set but switched off is not part of the arrangement.
    #[serde(default)]
    pub thresholds: Vec<(wxdata::level2::Moment, f32)>,
}

impl PaneSnap {
    /// Snapshot a live pane.
    pub fn capture(v: &MapView) -> Self {
        let (lon, lat) = crate::render::mercator::world_to_lonlat(v.camera.center.0, v.camera.center.1);
        Self {
            site: v.site.clone(),
            moment: v.moment,
            tilt: v.tilt,
            srv: v.srv,
            basemap: v.basemap.slug().to_string(),
            lon,
            lat,
            zoom: v.camera.zoom,
            fields_on: crate::render::FieldLayer::DRAW_ORDER
                .iter()
                .filter(|l| v.fields_on.contains(l))
                .map(|l| l.slug().to_string())
                .collect(),
            thresholds: wxdata::level2::Moment::ALL
                .iter()
                .enumerate()
                .filter(|&(i, _)| v.threshold_enabled[i])
                .filter_map(|(i, m)| v.thresholds[i].map(|t| (*m, t)))
                .collect(),
        }
    }

    /// Apply this snapshot to a pane. The volume itself isn't restored — a pane with a site and no
    /// data fetches through the normal poll path, which is also what a fresh pane does.
    pub fn apply(&self, v: &mut MapView) {
        v.site = self.site.clone();
        v.moment = self.moment;
        v.tilt = self.tilt;
        v.srv = self.srv;
        v.basemap = crate::tiles::BasemapStyle::from_slug(&self.basemap);
        v.camera = crate::render::mercator::Camera::at_lonlat(self.lon, self.lat, self.zoom);
        // The camera came from the saved layout, not from a site recenter — hold it through the
        // site change the restore just triggered.
        v.camera_placed = true;
        // Thresholds are a per-moment array; clear it first so restoring an arrangement without
        // one does not leave the previous pane's filter in place.
        v.thresholds = Default::default();
        v.threshold_enabled = Default::default();
        for (m, t) in &self.thresholds {
            v.thresholds[m.index()] = Some(*t);
            v.threshold_enabled[m.index()] = true;
        }
    }
}

/// Pane at a site-less default camera: the starters describe arrangements, and `adopt_site`
/// fills the site in when one is applied.
fn pane(moment: wxdata::level2::Moment, tilt: usize, srv: bool) -> PaneSnap {
    PaneSnap {
        site: None,
        moment,
        tilt,
        srv,
        basemap: "dark".into(),
        // Centered on the southern plains at a single-radar zoom; overwritten the moment the
        // pane adopts a site and recenters on it.
        lon: -97.5,
        lat: 35.5,
        zoom: 7.5,
        fields_on: Vec::new(),
        thresholds: Vec::new(),
    }
}

/// The three arrangements worth having before you have built any of your own. Seeded once, on
/// first run; deleting them is final (see `Settings::seeded_workspaces`).
pub fn starters() -> Vec<Workspace> {
    use wxdata::level2::Moment;
    vec![
        Workspace {
            name: "Chase".into(),
            // Reflectivity beside storm-relative velocity, same radar, cameras locked: the
            // couplet and the hook in one glance.
            panes: vec![
                pane(Moment::Reflectivity, 0, false),
                pane(Moment::Velocity, 0, true),
            ],
            active: 0,
            link_cameras: true,
            overlays_on: vec![
                "Alerts".into(),
                "Cells".into(),
                "Spotters".into(),
                "StormReports".into(),
                "Tds".into(),
            ],
            adopt_site: true,
            fields_on: Vec::new(),
            chrome: None,
        },
        Workspace {
            name: "National overview".into(),
            // One pane, no site, the MRMS mosaic under the warnings — what is happening anywhere.
            panes: vec![PaneSnap {
                site: None,
                moment: Moment::Reflectivity,
                tilt: 0,
                srv: false,
                basemap: "dark".into(),
                lon: -97.0,
                lat: 38.5,
                zoom: 4.0,
                fields_on: vec!["mrms".into()],
                thresholds: Vec::new(),
            }],
            active: 0,
            link_cameras: false,
            overlays_on: vec!["Alerts".into(), "StormReports".into(), "Fronts".into()],
            adopt_site: false,
            fields_on: vec!["mrms".into()],
            chrome: None,
        },
        Workspace {
            name: "Analysis".into(),
            // The same storm at four heights: how a couplet leans with height, which is the
            // layout people rebuild by hand every time.
            panes: (0..4).map(|t| pane(Moment::Reflectivity, t, false)).collect(),
            active: 0,
            link_cameras: true,
            overlays_on: vec!["Alerts".into(), "Cells".into(), "RangeRings".into()],
            adopt_site: true,
            fields_on: Vec::new(),
            chrome: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_roundtrips_through_a_view() {
        let mut v = MapView::new(
            Some("KTLX".into()),
            crate::render::mercator::Camera::at_lonlat(-97.28, 35.33, 8.5),
        );
        v.moment = wxdata::level2::Moment::Velocity;
        v.tilt = 2;
        v.srv = true;
        v.basemap = crate::tiles::BasemapStyle::from_slug("satellite");

        let snap = PaneSnap::capture(&v);
        let mut fresh = MapView::new(None, crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0));
        snap.apply(&mut fresh);

        assert_eq!(fresh.site.as_deref(), Some("KTLX"));
        assert_eq!(fresh.moment, wxdata::level2::Moment::Velocity);
        assert_eq!(fresh.tilt, 2);
        assert!(fresh.srv && fresh.camera_placed);
        assert_eq!(fresh.basemap, v.basemap);
        // Camera survives the lon/lat round trip to within a pixel at this zoom.
        assert!((fresh.camera.center.0 - v.camera.center.0).abs() < 1e-9);
        assert!((fresh.camera.center.1 - v.camera.center.1).abs() < 1e-9);
        assert_eq!(fresh.camera.zoom, 8.5);
    }

    #[test]
    fn per_pane_thresholds_and_layers_survive_the_round_trip() {
        use wxdata::level2::Moment;
        let mut v = MapView::new(None, crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0));
        v.fields_on.insert(crate::render::FieldLayer::Mrms);
        v.thresholds[Moment::Reflectivity.index()] = Some(35.0);
        v.threshold_enabled[Moment::Reflectivity.index()] = true;
        // Set but switched off: part of the session, not part of the arrangement.
        v.thresholds[Moment::Velocity.index()] = Some(20.0);

        let snap = PaneSnap::capture(&v);
        assert_eq!(snap.fields_on, vec!["mrms"]);
        assert_eq!(snap.thresholds, vec![(Moment::Reflectivity, 35.0)]);

        let mut fresh = MapView::new(None, crate::render::mercator::Camera::at_lonlat(0.0, 0.0, 3.0));
        // A leftover filter from whatever this pane was showing before must not survive a restore.
        fresh.thresholds[Moment::SpectrumWidth.index()] = Some(4.0);
        fresh.threshold_enabled[Moment::SpectrumWidth.index()] = true;
        snap.apply(&mut fresh);
        assert_eq!(fresh.thresholds[Moment::Reflectivity.index()], Some(35.0));
        assert!(fresh.threshold_enabled[Moment::Reflectivity.index()]);
        assert!(!fresh.threshold_enabled[Moment::SpectrumWidth.index()]);
        assert_eq!(fresh.thresholds[Moment::Velocity.index()], None);
    }

    #[test]
    fn a_pane_written_before_per_pane_layers_still_loads() {
        // No `fields_on`, no `thresholds` — exactly what an older build wrote.
        let snap: PaneSnap = serde_json::from_str(
            r#"{"site":"KTLX","moment":"Reflectivity","tilt":0,"basemap":"dark",
                "lon":-97.3,"lat":35.3,"zoom":8.0}"#,
        )
        .expect("old pane snapshots still parse");
        assert!(snap.fields_on.is_empty() && snap.thresholds.is_empty());
    }

    #[test]
    fn workspace_roundtrips_through_json() {
        let ws = Workspace {
            name: "Two-site chase".into(),
            panes: vec![PaneSnap {
                site: Some("KDMX".into()),
                moment: wxdata::level2::Moment::CorrelationCoefficient,
                tilt: 1,
                srv: false,
                basemap: "dark".into(),
                lon: -93.72,
                lat: 41.73,
                zoom: 7.25,
                fields_on: Vec::new(),
                thresholds: Vec::new(),
            }],
            active: 0,
            link_cameras: true,
            overlays_on: vec!["Alerts".into(), "Cells".into()],
            adopt_site: false,
            fields_on: vec!["mrms".into()],
            chrome: Some(Chrome {
                panel_open: true,
                alerts_tab: false,
                basemap_open: false,
                drawer: Some("Settings".into()),
            }),
        };
        let json = serde_json::to_string(&ws).unwrap();
        assert_eq!(serde_json::from_str::<Workspace>(&json).unwrap(), ws);
    }

    #[test]
    fn old_workspace_files_still_load() {
        // Written before `fields_on` and `adopt_site` existed.
        let json = r#"{"name":"old","panes":[],"active":0,"link_cameras":false,
            "overlays_on":["Alerts"]}"#;
        let ws: Workspace = serde_json::from_str(json).unwrap();
        assert!(ws.fields_on.is_empty() && !ws.adopt_site && ws.chrome.is_none());
    }

    #[test]
    fn every_starter_names_things_this_build_has() {
        for ws in starters() {
            assert!(!ws.panes.is_empty(), "{} has no panes", ws.name);
            assert!(ws.active < ws.panes.len());
            for slug in &ws.fields_on {
                assert!(
                    crate::render::FieldLayer::from_slug(slug).is_some(),
                    "{}: unknown field layer {slug}",
                    ws.name
                );
            }
            for p in &ws.panes {
                // `from_slug` falls back to Dark on an unknown name, so compare round trips.
                let style = crate::tiles::BasemapStyle::from_slug(&p.basemap);
                assert_eq!(style.slug(), p.basemap, "{}: bad basemap", ws.name);
            }
        }
    }
}
