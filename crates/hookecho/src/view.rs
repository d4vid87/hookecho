//! A single map pane's state: camera, selected radar product, and its loaded volume.
//!
//! `HookEchoApp` holds a `Vec<MapView>` (one for now; a grid when multi-pane lands in U9).
//! All per-pane UI state lives here so the app shell stays a thin orchestrator.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Instant;
use wxdata::level2::{self, BinnedSweep, Moment, Scan};

/// A decoded volume plus lazily-binned sweeps for the moments/tilts the user has viewed.
pub struct Volume {
    pub scan: Scan,
    /// AWS object name; used to detect when a newer volume has arrived.
    pub name: String,
    pub time: DateTime<Utc>,
    /// Human-readable VCP label for the toolbox, e.g. "VCP 212 (Precipitation, SZ-2)".
    pub vcp: String,
    /// Sorted, deduped tilt angles; the tilt index selects into this.
    pub elevations: Vec<f32>,
    // ponytail: unbounded per-volume cache; ~1.3 MB/sweep, cleared whenever the volume
    // changes. Worst case a user cycles all 6 moments × ~15 tilts ≈ 120 MB; add an LRU
    // only if that ever bites.
    binned: HashMap<(Moment, usize, bool), BinnedSweep>,
}

impl Volume {
    pub fn new(scan: Scan, name: String, time: DateTime<Utc>) -> Self {
        let vcp = scan.coverage_pattern_number().to_string();
        let elevations = level2::elevation_angles(&scan);
        Self { scan, name, time, vcp, elevations, binned: HashMap::new() }
    }

    /// Apply a live merged volume: swap in the new scan, recompute tilts, and evict only the
    /// binned sweeps whose tilts changed (a shifted tilt set clears the whole cache to be safe).
    pub fn apply_live(&mut self, scan: Scan, name: String, time: DateTime<Utc>, changed: &[f32]) {
        self.scan = scan;
        let new_elev = level2::elevation_angles(&self.scan);
        if new_elev != self.elevations {
            self.binned.clear(); // tilt indices may have shifted
        } else {
            for angle in changed {
                if let Some(idx) = new_elev.iter().position(|e| (e - angle).abs() < 0.15) {
                    self.binned.retain(|(_, t, _), _| *t != idx);
                }
            }
        }
        self.elevations = new_elev;
        self.vcp = self.scan.coverage_pattern_number().to_string();
        self.name = name;
        self.time = time;
    }

    /// Bin (and cache) the sweep for `moment` at tilt index `tilt`.
    pub fn binned(&mut self, moment: Moment, tilt: usize, dealias: bool) -> anyhow::Result<&BinnedSweep> {
        use std::collections::hash_map::Entry;
        match self.binned.entry((moment, tilt, dealias)) {
            Entry::Occupied(e) => Ok(e.into_mut()),
            Entry::Vacant(e) => {
                let sweep = level2::bin_scan_opts(&self.scan, moment, tilt, dealias)?;
                Ok(e.insert(sweep))
            }
        }
    }

    /// All reflectivity tilts as owned sweeps (lowest→highest), for vertical cross-sections.
    pub fn reflectivity_tilts(&mut self) -> Vec<BinnedSweep> {
        let n = self.elevations.len();
        (0..n)
            .filter_map(|t| self.binned(Moment::Reflectivity, t, false).ok().cloned())
            .collect()
    }
}

/// One map pane.
pub struct MapView {
    pub camera: crate::render::mercator::Camera,
    /// Selected radar site (`None` = Supercell's cleared "None" state).
    pub site: Option<String>,
    pub moment: Moment,
    pub tilt: usize,
    /// Per-moment display threshold (physical units), indexed by [`Moment::index`].
    pub thresholds: [Option<f32>; 6],
    pub threshold_enabled: [bool; 6],
    pub volume: Option<Volume>,
    /// The site the current volume/fetch belongs to; drives site-change detection.
    pub loaded_site: Option<String>,
    /// Archive/live playback state; `timeline.following` is the live auto-update flag.
    pub timeline: crate::timeline::Timeline,
    pub smooth: bool,
    /// Storm-relative velocity (velocity moment only); session state, not persisted.
    pub srv: bool,
    /// Storm motion the SRV subtracts: direction toward (deg from north) and speed (knots).
    pub storm_dir_deg: f32,
    pub storm_speed_kt: f32,
    /// Basemap source under the radar (`None` = off).
    pub basemap: crate::tiles::BasemapStyle,
    pub show_radar: bool,
    pub show_legend: bool,
    pub loading: bool,
    pub last_poll: Option<Instant>,
    pub error: Option<String>,
}

impl MapView {
    pub fn new(site: Option<String>, camera: crate::render::mercator::Camera) -> Self {
        Self {
            camera,
            site,
            moment: Moment::Reflectivity,
            tilt: 0,
            thresholds: [None; 6],
            threshold_enabled: [false; 6],
            volume: None,
            loaded_site: None,
            timeline: crate::timeline::Timeline::default(),
            smooth: false,
            srv: false,
            storm_dir_deg: 240.0,
            storm_speed_kt: 25.0,
            basemap: crate::tiles::BasemapStyle::default(),
            show_radar: true,
            show_legend: true,
            loading: false,
            last_poll: None,
            error: None,
        }
    }

    /// The active threshold for the current moment, if enabled.
    pub fn active_threshold(&self) -> Option<f32> {
        let i = self.moment.index();
        if self.threshold_enabled[i] {
            self.thresholds[i]
        } else {
            None
        }
    }

    /// Storm motion as (east, north) components in m/s, from the toolbox dir/speed (knots).
    /// `None` unless SRV is on and the velocity moment is active (SRV is velocity-only).
    pub fn storm_motion_uv(&self) -> Option<(f32, f32)> {
        if !self.srv || self.moment != Moment::Velocity {
            return None;
        }
        let speed_ms = self.storm_speed_kt / 1.943_844; // knots -> m/s
        let r = self.storm_dir_deg.to_radians();
        Some((speed_ms * r.sin(), speed_ms * r.cos())) // east = sin(bearing), north = cos
    }

    /// Clamp the tilt index to the loaded volume's elevation list.
    pub fn clamp_tilt(&mut self) {
        if let Some(v) = &self.volume {
            if !v.elevations.is_empty() && self.tilt >= v.elevations.len() {
                self.tilt = v.elevations.len() - 1;
            }
        }
    }

    /// Number of tilts in this pane's own loaded volume (0 if none).
    pub fn elevation_count(&self) -> usize {
        self.volume.as_ref().map_or(0, |v| v.elevations.len())
    }

    /// Clamp the tilt index to `count` tilts (used when a pane binds another pane's volume).
    pub fn clamp_tilt_to(&mut self, count: &usize) {
        if *count > 0 && self.tilt >= *count {
            self.tilt = *count - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::mercator::Camera;

    #[test]
    fn storm_motion_uv_is_velocity_only_and_directional() {
        let mut v = MapView::new(None, Camera::at_lonlat(-97.0, 35.0, 8.0));
        v.moment = Moment::Velocity;
        // Off by default.
        assert_eq!(v.storm_motion_uv(), None);
        // Due east (090°) at ~19.4 kt = 10 m/s: east component ~10, north ~0.
        v.srv = true;
        v.storm_dir_deg = 90.0;
        v.storm_speed_kt = 19.438_44;
        let (e, n) = v.storm_motion_uv().unwrap();
        assert!((e - 10.0).abs() < 0.05, "east {e}");
        assert!(n.abs() < 0.05, "north {n}");
        // SRV never applies to non-velocity moments.
        v.moment = Moment::Reflectivity;
        assert_eq!(v.storm_motion_uv(), None);
    }
}
