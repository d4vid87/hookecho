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
//! `ponytail: no per-pane thresholds; add them if someone asks for a saved threshold set.`

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
    }
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
            }],
            active: 0,
            link_cameras: true,
            overlays_on: vec!["Alerts".into(), "Cells".into()],
        };
        let json = serde_json::to_string(&ws).unwrap();
        assert_eq!(serde_json::from_str::<Workspace>(&json).unwrap(), ws);
    }
}
