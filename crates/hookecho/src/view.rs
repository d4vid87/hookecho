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

/// How many binned sweeps one volume keeps. A sweep is ~1.3 MB.
///
/// Twelve was below the number of tilts in a volume, which is the one size it must not be: VCP
/// 212 has 14 unique elevations and VCP 215 has 15, and anything that walks the tilts in order —
/// the derived products, CAPPI, a cross-section, the three dual-pol passes — evicted each entry
/// just before coming back to it. A sequential scan through an LRU shorter than the scan is the
/// textbook miss-every-time case, and it ran on the UI thread once per volume.
///
/// 32 holds two moments' worth of a full volume, which is what flipping between reflectivity and
/// velocity actually asks for. The browser keeps half that: its heap is 32-bit and it is already
/// holding the volumes themselves.
const BINNED_CACHE: usize = if cfg!(target_arch = "wasm32") { 16 } else { 32 };

/// How many volumes a pane keeps after the playhead has moved off them.
///
/// `Volume::new` is called every time the playhead lands on a frame, and a fresh `Volume` starts
/// with an empty [`BINNED_CACHE`] — so a ten-frame loop re-binned every sweep it showed, every
/// lap, forever. Keeping the volumes themselves means each (moment, tilt) is binned once per
/// volume instead of four times a second.
///
/// Sized to the loop window, because that is the working set: what a lap comes back to. The scan
/// behind each one is an `Arc` shared with the app's decoded-volume cache, so what this actually
/// holds is the binned sweeps that were looked at — about 1.3 MB per sweep per volume.
const RECENT_VOLUMES: usize = if cfg!(target_os = "android") {
    8
} else if cfg!(target_arch = "wasm32") {
    // Two, not six. These overlap the app's own scan cache by design — both hold the same
    // `Arc<Scan>` — but the binned sweeps hanging off them are this pane's alone, and in a
    // 32-bit heap that never shrinks, six volumes' worth of them is the difference between a
    // tab that runs all day and one that has to be closed.
    2
} else {
    12
};

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
    /// Set once a live chunk has been merged in. Such a volume is still being written — half its
    /// tilts may not have arrived — so it must never be kept and shown again later in place of
    /// the complete archived volume of the same name.
    live: bool,
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
            live: false,
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
        self.live = true;
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
    /// Volumes the playhead has moved off, with their binned sweeps intact. See
    /// [`RECENT_VOLUMES`]; use [`MapView::show_volume`] rather than touching this.
    recent: LruCache<String, Volume>,
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
    /// National field layers drawn in this pane. Per-pane rather than app-wide: two panes is how
    /// you compare two fields, and the model-difference layer would rather be a pair of panes
    /// than a subtraction. The grids themselves stay in one shared cache — only the choice of
    /// what to draw lives here.
    pub fields_on: std::collections::HashSet<crate::render::FieldLayer>,
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
            recent: LruCache::new(NonZeroUsize::new(RECENT_VOLUMES).unwrap()),
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
            fields_on: Default::default(),
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
    /// Show the volume named `name`, reusing the one this pane already binned if it has it.
    ///
    /// The playhead moving off a volume is not the same as being done with it — a loop comes back
    /// round. The displaced volume goes into [`Self::recent`] with its binned sweeps, and coming
    /// back to it is a hash lookup rather than a full az x gate rebin of every sweep on display.
    ///
    /// `scan` is only used when this pane has never binned that volume; it is an `Arc` from the
    /// app's decoded-volume cache either way, so the two paths hold the same allocation.
    pub fn show_volume(&mut self, scan: Arc<Scan>, name: String, time: DateTime<Utc>) {
        let vol = self
            .recent
            .pop(&name)
            .unwrap_or_else(|| Volume::new(scan, name, time));
        if let Some(old) = self.volume.replace(vol) {
            // Not a volume still being written, and not a stale copy of the one just shown.
            if !old.live && old.name != self.volume.as_ref().expect("just set").name {
                self.recent.put(old.name.clone(), old);
            }
        }
    }

    /// Forget every kept volume. The site changed, so none of them is of anywhere being looked at.
    pub fn forget_recent(&mut self) {
        self.recent.clear();
    }

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

    /// A lap of a loop must not re-bin what it binned last lap. This is the whole wave: before it,
    /// `Volume::new` on every playhead move meant the binned cache started empty every frame.
    #[test]
    fn coming_back_to_a_volume_reuses_its_binned_sweeps() {
        let now = chrono::Utc::now();
        let mut view = MapView::new(
            Some("KTLX".into()),
            crate::render::mercator::Camera::at_lonlat(-97.28, 35.33, 7.0),
        );
        let binned = || {
            wxdata::stats::snapshot()
                .into_iter()
                .find(|(l, _)| *l == "sweeps_binned")
                .unwrap()
                .1
        };

        view.show_volume(scan_at(&[0.5, 1.5]), "a".into(), now);
        let before = binned();
        view.volume
            .as_mut()
            .unwrap()
            .binned(Moment::Reflectivity, 0, false)
            .unwrap();
        assert_eq!(binned() - before, 1, "the first look has to do the work");

        // The playhead moves on, then comes back round.
        view.show_volume(scan_at(&[0.5, 1.5]), "b".into(), now);
        view.volume
            .as_mut()
            .unwrap()
            .binned(Moment::Reflectivity, 0, false)
            .unwrap();
        let after_b = binned();
        view.show_volume(scan_at(&[0.5, 1.5]), "a".into(), now);
        assert_eq!(view.volume.as_ref().unwrap().name, "a");
        view.volume
            .as_mut()
            .unwrap()
            .binned(Moment::Reflectivity, 0, false)
            .unwrap();
        assert_eq!(
            binned(),
            after_b,
            "the second lap re-binned a sweep it already had"
        );

        // A site change is a different piece of sky: nothing kept is worth keeping.
        view.forget_recent();
        view.show_volume(scan_at(&[0.5, 1.5]), "b".into(), now);
        let before = binned();
        view.volume
            .as_mut()
            .unwrap()
            .binned(Moment::Reflectivity, 0, false)
            .unwrap();
        assert_eq!(binned() - before, 1);

        // A volume still being written must never be kept: half its tilts may not have arrived,
        // and showing it again later in place of the complete archive volume loses them.
        view.forget_recent();
        view.show_volume(scan_at(&[0.5, 1.5]), "live".into(), now);
        view.volume
            .as_mut()
            .unwrap()
            .apply_live(scan_at(&[0.5]), "live".into(), now, &[]);
        view.show_volume(scan_at(&[0.5, 1.5]), "next".into(), now);
        view.show_volume(scan_at(&[0.5, 1.5]), "live".into(), now);
        assert_eq!(
            view.volume.as_ref().unwrap().elevations,
            vec![0.5, 1.5],
            "the half-written live volume was kept and shown again"
        );

        // The size of it, both paths measured against each other: ten frames, three laps. The old
        // path called `Volume::new` on every playhead move, so every frame of every lap re-ran
        // `bin_scan_opts`. One test, not two: the counter is process-wide and the test threads
        // share it, so a sibling reading it in parallel would see this one's work.
        let names: Vec<String> = (0..10).map(|i| format!("frame-{i}")).collect();
        let before = binned();
        for _ in 0..3 {
            for name in &names {
                let mut vol = Volume::new(scan_at(&[0.5, 1.5]), name.clone(), now);
                vol.binned(Moment::Reflectivity, 0, false).unwrap();
            }
        }
        assert_eq!(
            binned() - before,
            30,
            "the old path re-bins every frame, every lap"
        );

        let mut view = MapView::new(
            Some("KTLX".into()),
            crate::render::mercator::Camera::at_lonlat(-97.28, 35.33, 7.0),
        );
        let before = binned();
        for _ in 0..3 {
            for name in &names {
                view.show_volume(scan_at(&[0.5, 1.5]), name.clone(), now);
                view.volume
                    .as_mut()
                    .unwrap()
                    .binned(Moment::Reflectivity, 0, false)
                    .unwrap();
            }
        }
        assert_eq!(binned() - before, 10, "one per volume, then two free laps");
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
