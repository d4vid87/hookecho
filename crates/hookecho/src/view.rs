//! A single map pane's state: camera, selected radar product, and its loaded volume.
//!
//! `HookEchoApp` holds a `Vec<MapView>` (one for now; a grid when multi-pane lands in U9).
//! All per-pane UI state lives here so the app shell stays a thin orchestrator.

use chrono::{DateTime, Utc};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use wxdata::clock::Instant;
use wxdata::level2::{self, BinnedSweep, Moment, Scan};

/// How many binned sweeps one volume keeps. A sweep is ~1.3 MB, and the working set while
/// flipping through moments and tilts is a handful — 12 covers it without the old unbounded
/// map's ~140 MB worst case (7 moments x ~15 tilts, all resident at once).
const BINNED_CACHE: usize = 12;

/// A decoded volume plus lazily-binned sweeps for the moments/tilts the user has viewed.
pub struct Volume {
    /// Shared with the app's decoded-volume LRU: the cache and every pane showing this volume
    /// point at one allocation, so a scrub or a live arrival costs a refcount, not a deep copy.
    pub scan: Arc<Scan>,
    /// AWS object name; used to detect when a newer volume has arrived.
    pub name: String,
    pub time: DateTime<Utc>,
    /// Human-readable VCP label for the toolbox, e.g. "VCP 212 (Precipitation, SZ-2)".
    pub vcp: String,
    /// Sorted, deduped tilt angles; the tilt index selects into this.
    pub elevations: Vec<f32>,
    /// Which moments *this* volume carries (the pane keeps the wider union for its UI rows).
    pub moments: [bool; Moment::ALL.len()],
    binned: LruCache<(Moment, usize, bool), BinnedSweep>,
}

impl Volume {
    pub fn new(scan: Arc<Scan>, name: String, time: DateTime<Utc>) -> Self {
        let vcp = scan.coverage_pattern_number().to_string();
        let elevations = level2::elevation_angles(&scan);
        let moments = level2::available_moments(&scan);
        Self {
            scan,
            name,
            time,
            vcp,
            elevations,
            moments,
            binned: LruCache::new(NonZeroUsize::new(BINNED_CACHE).unwrap()),
        }
    }

    /// Apply a live merged volume: swap in the new scan, recompute tilts, and evict only the
    /// binned sweeps whose tilts changed (a shifted tilt set clears the whole cache to be safe).
    pub fn apply_live(
        &mut self,
        scan: Arc<Scan>,
        name: String,
        time: DateTime<Utc>,
        changed: &[f32],
    ) {
        // The first chunks of a new volume carry the metadata and a sweep with no radials yet, so
        // the merged scan has no elevation angles at all. Applying it emptied the tilt list, blanked
        // the moment rows, and made the next frame's bin fail with "tilt 0 out of range". Keep
        // showing the volume we have until the new one has a tilt in it.
        let new_elev = level2::elevation_angles(&scan);
        if new_elev.is_empty() {
            return;
        }
        self.scan = scan;
        if new_elev != self.elevations {
            self.binned.clear(); // tilt indices may have shifted
        } else {
            for angle in changed {
                if let Some(idx) = new_elev.iter().position(|e| (e - angle).abs() < 0.15) {
                    let stale: Vec<_> = self
                        .binned
                        .iter()
                        .map(|(k, _)| *k)
                        .filter(|(_, t, _)| *t == idx)
                        .collect();
                    for k in stale {
                        self.binned.pop(&k);
                    }
                }
            }
        }
        self.elevations = new_elev;
        self.moments = level2::available_moments(&self.scan);
        self.vcp = self.scan.coverage_pattern_number().to_string();
        self.name = name;
        self.time = time;
    }

    /// Bin (and cache) the sweep for `moment` at tilt index `tilt`.
    pub fn binned(
        &mut self,
        moment: Moment,
        tilt: usize,
        dealias: bool,
    ) -> anyhow::Result<&BinnedSweep> {
        let scan = Arc::clone(&self.scan);
        self.binned.try_get_or_insert((moment, tilt, dealias), || {
            level2::bin_scan_opts(&scan, moment, tilt, dealias)
        })
    }

    /// All reflectivity tilts as owned sweeps (lowest→highest), for vertical cross-sections.
    pub fn reflectivity_tilts(&mut self) -> Vec<BinnedSweep> {
        self.moment_tilts(Moment::Reflectivity)
    }

    /// All tilts of `moment` as owned sweeps (lowest→highest). Cross-sections of velocity show
    /// how a couplet leans with height; CC slices show how deep a debris ball actually is.
    pub fn moment_tilts(&mut self, moment: Moment) -> Vec<BinnedSweep> {
        let n = self.elevations.len();
        (0..n)
            .filter_map(|t| self.binned(moment, t, false).ok().cloned())
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
    pub thresholds: [Option<f32>; Moment::ALL.len()],
    pub threshold_enabled: [bool; Moment::ALL.len()],
    pub volume: Option<Volume>,
    /// The site the current volume/fetch belongs to; drives site-change detection.
    pub loaded_site: Option<String>,
    /// Set when the camera was aimed deliberately (Event Library, a bookmark, `HOOKECHO_GOTO`)
    /// rather than by panning. Switching site normally recenters on the radar, which would throw
    /// away the framing the deep link just asked for; this survives exactly one such recenter.
    pub camera_placed: bool,
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
    /// Every moment seen in any volume from `loaded_site`, cleared when the site changes.
    ///
    /// A single live volume is only as complete as the tilts that have arrived: early in a scan
    /// the merged volume is reflectivity-only, so reading availability off it alone made the
    /// dual-pol rows blink out of the sidebar once a volume. What a radar sends doesn't change
    /// mid-session, so remember it.
    pub moments_seen: [bool; Moment::ALL.len()],
}

impl MapView {
    pub fn new(site: Option<String>, camera: crate::render::mercator::Camera) -> Self {
        Self {
            camera,
            site,
            moment: Moment::Reflectivity,
            tilt: 0,
            thresholds: [None; Moment::ALL.len()],
            threshold_enabled: [false; Moment::ALL.len()],
            volume: None,
            loaded_site: None,
            camera_placed: false,
            timeline: crate::timeline::Timeline::default(),
            smooth: true,
            srv: false,
            storm_dir_deg: 240.0,
            storm_speed_kt: 25.0,
            basemap: crate::tiles::BasemapStyle::default(),
            show_radar: true,
            show_legend: true,
            loading: false,
            last_poll: None,
            error: None,
            moments_seen: [false; Moment::ALL.len()],
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

    /// What this site's radar sends: the union over every volume seen since the site was
    /// selected, or "everything" before the first one lands.
    pub fn moments(&self) -> [bool; Moment::ALL.len()] {
        if self.moments_seen.iter().any(|m| *m) {
            self.moments_seen
        } else {
            [true; Moment::ALL.len()]
        }
    }

    /// Snap the selected product to one this volume actually carries — a TDWR has no dual-pol
    /// moments, and neither does anything from before the 2011-13 upgrade.
    pub fn clamp_moment(&mut self) {
        let Some(v) = &self.volume else { return };
        for (seen, got) in self.moments_seen.iter_mut().zip(v.moments) {
            *seen |= got;
        }
        let have = self.moments();
        if !have[self.moment.index()] {
            if let Some(m) = Moment::ALL.into_iter().find(|m| have[m.index()]) {
                self.moment = m;
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

    use nexrad_model::data::{
        MomentData, PulseWidth, Radial, RadialStatus, Sweep, VolumeCoveragePattern,
    };

    /// A scan with one radial per given elevation, carrying reflectivity only.
    fn scan_at(elevations: &[f32]) -> Arc<Scan> {
        let sweeps: Vec<Sweep> = elevations
            .iter()
            .map(|e| {
                let data = MomentData::from_fixed_point(1, 2125, 250, 8, 2.0, 66.0, vec![106u8]);
                let radial = Radial::new(
                    0,
                    0,
                    0.0,
                    0.5,
                    RadialStatus::ScanStart,
                    1,
                    *e,
                    Some(data),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                Sweep::new(1, vec![radial])
            })
            .collect();
        let vcp = VolumeCoveragePattern::new(
            212,
            0,
            0.5,
            PulseWidth::Short,
            false,
            0,
            false,
            0,
            false,
            false,
            0,
            false,
            false,
            Vec::new(),
        );
        let site = nexrad_model::meta::Site::new(*b"KTLX", 35.33, -97.28, 380, 0);
        Arc::new(Scan::with_site(site, vcp, sweeps))
    }

    /// A live volume that has only just started has no tilts in it yet. Applying it emptied the
    /// tilt list and made the next bin fail with "tilt 0 out of range".
    #[test]
    fn a_tiltless_live_update_does_not_replace_the_volume_on_screen() {
        let now = chrono::Utc::now();
        let mut vol = Volume::new(scan_at(&[0.5, 1.5]), "a".into(), now);
        assert_eq!(vol.elevations, vec![0.5, 1.5]);
        vol.apply_live(scan_at(&[]), "b".into(), now, &[]);
        assert_eq!(vol.elevations, vec![0.5, 1.5], "kept the tilts it had");
        assert_eq!(vol.name, "a", "and the volume they came from");
        // A real volume still applies.
        vol.apply_live(scan_at(&[0.5]), "c".into(), now, &[]);
        assert_eq!(vol.elevations, vec![0.5]);
        assert_eq!(vol.name, "c");
    }

    /// Early in a live volume only reflectivity has arrived; the dual-pol rows must not blink out
    /// of the sidebar and come back once a scan.
    #[test]
    fn moment_availability_is_remembered_across_volumes() {
        let mut v = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 8.0));
        // Nothing loaded yet: assume the radar sends everything rather than hiding rows.
        assert_eq!(v.moments(), [true; Moment::ALL.len()]);
        v.volume = Some(Volume::new(scan_at(&[0.5]), "a".into(), chrono::Utc::now()));
        v.clamp_moment();
        let refl_only = v.moments();
        assert!(refl_only[Moment::Reflectivity.index()]);
        assert!(!refl_only[Moment::Velocity.index()], "REF-only volume");
        // A later volume carrying velocity adds to the union and never subtracts from it.
        v.moments_seen[Moment::Velocity.index()] = true;
        v.volume = Some(Volume::new(scan_at(&[0.5]), "b".into(), chrono::Utc::now()));
        v.clamp_moment();
        assert!(v.moments()[Moment::Velocity.index()], "still velocity");
        // The union is for the UI rows; the volume keeps its own list so the renderer can fall
        // back instead of drawing nothing for a frame that lacks the selected product.
        assert!(!v.volume.as_ref().unwrap().moments[Moment::Velocity.index()]);
    }

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
