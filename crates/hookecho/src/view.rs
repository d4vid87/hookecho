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

/// Whether `new` is `old` with tilts only added on the end, so every existing tilt index still
/// points at the same elevation.
///
/// This is the ordinary shape of a live volume: chunks arrive every ten or twenty seconds and each
/// one appends the next sweep. Treating that as "the tilt set changed" threw away every binned
/// sweep in the volume, and the UI thread re-binned all of them on the next frame — a visible hitch
/// once per chunk, for the whole life of the volume. A VCP restart or a re-ordered merge really
/// does shift the indices, and still clears.
///
/// Compared with the same 0.15 degree tolerance the caller uses to match a changed tilt: the angles
/// are recomputed from the merged scan each time and need not be bit-identical.
fn tilts_only_grew(old: &[f32], new: &[f32]) -> bool {
    new.len() >= old.len() && old.iter().zip(new).all(|(a, b)| (a - b).abs() < 0.15)
}

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

type BinnedKey = (Moment, usize, bool, Option<i64>, bool);

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
    /// Acquisition order, including repeated low-level cuts at the same display tilt.
    pub cuts: Vec<level2::CutTime>,
    /// Which moments *this* volume carries (the pane keeps the wider union for its UI rows).
    pub moments: [bool; Moment::ALL.len()],
    binned: LruCache<BinnedKey, Arc<BinnedSweep>>,
    /// Set once a live chunk has been merged in. Such a volume is still being written — half its
    /// tilts may not have arrived — so it must never be kept and shown again later in place of
    /// the complete archived volume of the same name.
    live: bool,
    pub live_status: Option<wxdata::live::ScanStatus>,
}

impl Volume {
    fn binned_bytes(&self) -> usize {
        self.binned.iter().map(|(_, s)| s.data.len() + s.data16.as_ref().map_or(0, |v| v.len() * 2)).sum()
    }

    pub fn new(scan: Arc<Scan>, name: String, time: DateTime<Utc>) -> Self {
        let vcp = scan.coverage_pattern_number().to_string();
        let elevations = level2::elevation_angles(&scan);
        let cuts = level2::cut_chronology(&scan);
        let moments = level2::available_moments(&scan);
        Self {
            scan,
            name,
            time,
            vcp,
            elevations,
            cuts,
            moments,
            binned: LruCache::new(NonZeroUsize::new(BINNED_CACHE).unwrap()),
            live: false,
            live_status: None,
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
        status: wxdata::live::ScanStatus,
    ) {
        self.live_status = Some(status);
        // The first chunks of a new volume carry the metadata and a sweep with no radials yet, so
        // the merged scan has no elevation angles at all. Applying it emptied the tilt list, blanked
        // the moment rows, and made the next frame's bin fail with "tilt 0 out of range". Keep
        // showing the volume we have until the new one has a tilt in it.
        let new_elev = level2::elevation_angles(&scan);
        if new_elev.is_empty() {
            return;
        }
        self.scan = scan;
        if !tilts_only_grew(&self.elevations, &new_elev) {
            self.binned.clear(); // tilt indices may have shifted
        } else {
            for angle in changed {
                if let Some(idx) = new_elev.iter().position(|e| (e - angle).abs() < 0.15) {
                    let stale: Vec<_> = self
                        .binned
                        .iter()
                        .map(|(k, _)| *k)
                        .filter(|(_, t, _, _, _)| *t == idx)
                        .collect();
                    for k in stale {
                        self.binned.pop(&k);
                    }
                }
            }
        }
        self.elevations = new_elev;
        self.cuts = level2::cut_chronology(&self.scan);
        self.moments = level2::available_moments(&self.scan);
        self.vcp = self.scan.coverage_pattern_number().to_string();
        self.name = name;
        self.time = time;
        self.live = true;
    }

    pub fn end_live(&mut self) {
        if let Some(status) = &mut self.live_status {
            status.stream_active = false;
        }
    }

    /// Bin (and cache) the sweep for `moment` at tilt index `tilt`.
    pub fn binned(
        &mut self,
        moment: Moment,
        tilt: usize,
        dealias: bool,
    ) -> anyhow::Result<&BinnedSweep> {
        self.binned_at(moment, tilt, dealias, None)
    }

    pub fn binned_at(
        &mut self,
        moment: Moment,
        tilt: usize,
        dealias: bool,
        cut_end_ms: Option<i64>,
    ) -> anyhow::Result<&BinnedSweep> {
        self.binned_display_at(moment, tilt, dealias, cut_end_ms, false)
    }

    pub fn binned_display_at(&mut self, moment: Moment, tilt: usize, dealias: bool, cut_end_ms: Option<i64>, precision: bool) -> anyhow::Result<&BinnedSweep> {
        let key = (moment, tilt, dealias, cut_end_ms, precision);
        if !self.binned.contains(&key) {
            let sweep = level2::bin_scan_display_at(&self.scan, moment, tilt, dealias, cut_end_ms, precision)?;
            let bytes = |s: &BinnedSweep| s.data.len() + s.data16.as_ref().map_or(0, |d| d.len() * 2);
            let budget = BINNED_CACHE * 720 * 1840;
            let mut retained: usize = self.binned.iter().map(|(_, s)| bytes(s)).sum();
            while !self.binned.is_empty() && retained + bytes(&sweep) > budget {
                if let Some((_, old)) = self.binned.pop_lru() { retained -= bytes(&old); }
            }
            self.binned.put(key, Arc::new(sweep));
        }
        Ok(self.binned.get(&key).expect("inserted display sweep"))
    }

    /// Immutable shared input for custom products: retaining a moment never copies its gate arrays.
    pub fn shared_binned_display_at(&mut self, moment: Moment, tilt: usize, cut_end_ms: Option<i64>, precision: bool) -> anyhow::Result<Arc<BinnedSweep>> {
        self.binned_display_at(moment, tilt, false, cut_end_ms, precision)?;
        Ok(self.binned.get(&(moment, tilt, false, cut_end_ms, precision)).expect("inserted shared display sweep").clone())
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
pub(crate) struct RecoveryCut {
    pub(crate) site: Option<String>,
    pub(crate) volume_ms: i64,
    pub(crate) selection: (i64, Moment, usize),
}

pub struct MapView {
    pub camera: crate::render::mercator::Camera,
    pub camera_motion: crate::camera_motion::Motion,
    pub tile_detail: crate::tiles::DetailLevel,
    /// Selected radar site (`None` = Supercell's cleared "None" state).
    pub site: Option<String>,
    pub moment: Moment,
    /// Name of a saved user-defined radar product, or `None` for a native moment.
    pub custom_product: Option<String>,
    pub tilt: usize,
    /// Selected cut within the shown volume; `None` follows its newest matching sweep.
    pub cut_selection: Option<(i64, Moment, usize)>,
    pub(crate) recovery_cut: Option<RecoveryCut>,
    /// Opt-in live mode: keep this pane on the newest available 0.5° revisit.
    pub follow_low_cut: bool,
    /// Per-moment display threshold (physical units), indexed by [`Moment::index`].
    pub thresholds: [Option<f32>; Moment::ALL.len()],
    pub threshold_enabled: [bool; Moment::ALL.len()],
    /// Pane-local display controls; never alter native gate measurements.
    pub appearance: crate::radar_appearance::RadarAppearance,
    pub satellite: crate::satellite_time::Selection,
    pub frame_request: crate::frame_request::Request,
    pub head_request: crate::frame_request::Request,
    pub list_request: crate::frame_request::Request,
    pub last_list_failure: Option<Instant>,
    pub load_phase: Option<level2::LoadPhase>,
    pub buffering: bool,
    pub failed_frame: Option<String>,
    pub frame_history: std::collections::VecDeque<(String, crate::timeline::FrameStatus)>,
    pub skipped_frames: u64,
    pub last_skipped_at: Option<wxdata::clock::Instant>,
    pub volume: Option<Volume>,
    /// Volumes the playhead has moved off, with their binned sweeps intact. See
    /// [`RECENT_VOLUMES`]; use [`MapView::show_volume`] rather than touching this.
    recent: LruCache<String, Volume>,
    /// The site the current volume/fetch belongs to; drives site-change detection.
    pub loaded_site: Option<String>,
    /// Source of the previous scan kept visible while the selected station loads.
    pub retained_station: Option<String>,
    pub recenter_on_ready: bool,
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
    pub abi_scene: Option<wxdata::abi::Scene>,
    pub show_radar: bool,
    pub show_legend: bool,
    pub loading: bool,
    pub last_poll: Option<Instant>,
    pub error: Option<String>,
    pub fetch_failures: u8,
    pub failover_from: Option<String>,
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
    /// Public radar state for reproducible reports. Omit camera, user product names, paths,
    /// error messages and provider selectors: those can contain private locations or keys.
    pub(crate) fn diagnostic_state(&self) -> serde_json::Value {
        let public_site = |site: &Option<String>| {
            site.as_deref().filter(|id| wxdata::sites::site_by_id(id).is_some())
                .map(str::to_owned)
        };
        serde_json::json!({
            "selected_station": public_site(&self.site),
            "loaded_station": public_site(&self.loaded_site),
            "displayed_station": public_site(&self.displayed_station().map(str::to_owned)),
            "station_transition": self.retained_station.is_some(),
            "moment": self.moment,
            "custom_product_selected": self.custom_product.is_some(),
            "tilt_index": self.tilt,
            "appearance": {
                "blend_width": self.appearance.blend_width,
                "precision": self.appearance.precision,
                "multipass": self.appearance.multipass,
                "fade_old_sectors": self.appearance.fade_old_sectors,
                "upper_bounds": self.appearance.upper_bounds,
                "pane_palette_count": self.appearance.palettes.len(),
                "smooth": self.smooth,
                "globe": self.camera.globe,
                "pitch_deg": self.camera.pitch_deg,
                "bearing_deg": self.camera.bearing_deg,
            },
            "thresholds": self.thresholds,
            "threshold_enabled": self.threshold_enabled,
            "volume_time": self.volume.as_ref().map(|volume| volume.time),
            "archive_date": self.timeline.date,
            "playhead": self.timeline.playhead,
            "frame_count": self.timeline.frames.len(),
            "playing": self.timeline.playing,
            "following_live": self.timeline.following,
            "speed": self.timeline.speed,
            "loop_enabled": self.timeline.loop_enabled,
            "cut_playback": self.timeline.cut_playback,
            "cut_index": self.timeline.cut_index,
            "listing": self.timeline.listing,
            "loading": self.loading,
            "buffering": self.buffering,
            "skipped_frames": self.skipped_frames,
            "load_failed": self.error.is_some(),
        })
    }
    /// Resident display gate buffers, including this pane's recent-volume cache.
    pub fn retained_scans(&self) -> impl Iterator<Item = &Arc<Scan>> {
        self.volume.iter().chain(self.recent.iter().map(|(_, volume)| volume)).map(|volume| &volume.scan)
    }

    pub fn binned_cache_bytes(&self) -> usize {
        self.volume.as_ref().map_or(0, Volume::binned_bytes)
            + self.recent.iter().map(|(_, volume)| volume.binned_bytes()).sum::<usize>()
    }

    /// Bound binned retention across the displayed and recent volumes, preserving the newest
    /// display bin. GPU upload shadows are accounted separately by the renderer.
    pub fn trim_binned_memory(&mut self, cap: usize) {
        let mut bytes = self.binned_cache_bytes();
        if bytes <= cap { return; }
        for (_, volume) in self.recent.iter_mut().rev() {
            while bytes > cap {
                let Some((_, sweep)) = volume.binned.pop_lru() else { break; };
                bytes = bytes.saturating_sub(sweep.data.len() + sweep.data16.as_ref().map_or(0, |data| data.len() * 2));
            }
        }
        if let Some(volume) = &mut self.volume {
            while bytes > cap && volume.binned.len() > 1 {
                if let Some((_, sweep)) = volume.binned.pop_lru() {
                    bytes = bytes.saturating_sub(sweep.data.len() + sweep.data16.as_ref().map_or(0, |data| data.len() * 2));
                }
            }
        }
    }

    /// Station belonging to the scan on screen, distinct from a pending selection.
    pub fn displayed_station(&self) -> Option<&str> {
        self.volume.as_ref()?;
        self.retained_station.as_deref().or(self.loaded_site.as_deref()).or(self.site.as_deref())
    }

    pub fn displayed_time(&self) -> Option<DateTime<Utc>> {
        let volume = self.volume.as_ref()?;
        Some(self.selected_cut_ms().and_then(chrono::DateTime::from_timestamp_millis).unwrap_or(volume.time))
    }

    /// Compare the scan actually loaded, including a selected cut, with the linked target.
    /// A pending playhead fetch must not make the previously painted scan appear synchronized.
    pub fn alignment_offset_seconds(&self) -> Option<i64> {
        let target = self.timeline.alignment_target?;
        Some((self.displayed_time()? - target).num_seconds())
    }

    pub fn alignment_offset_milliseconds(&self) -> Option<i64> {
        Some((self.displayed_time()? - self.timeline.alignment_target?).num_milliseconds())
    }

    pub fn alignment_acceptable(&self, limit: Option<u32>) -> bool {
        match (self.timeline.alignment_target, limit) {
            (Some(target), Some(limit)) => self.displayed_time().is_some_and(|time| (time - target).abs() <= chrono::Duration::seconds(i64::from(limit))),
            _ => true,
        }
    }

    pub fn new(site: Option<String>, camera: crate::render::mercator::Camera) -> Self {
        let mut thresholds = [None; Moment::ALL.len()];
        let mut threshold_enabled = [false; Moment::ALL.len()];
        thresholds[Moment::Reflectivity.index()] = Some(16.0);
        threshold_enabled[Moment::Reflectivity.index()] = true;
        Self {
            camera,
            camera_motion: crate::camera_motion::Motion::default(),
            tile_detail: crate::tiles::DetailLevel::default(),
            site,
            moment: Moment::Reflectivity,
            custom_product: None,
            tilt: 0,
            cut_selection: None,
            recovery_cut: None,
            follow_low_cut: false,
            thresholds,
            threshold_enabled,
            appearance: Default::default(),
            satellite: Default::default(),
            frame_request: Default::default(),
            head_request: Default::default(),
            list_request: Default::default(),
            last_list_failure: None,
            load_phase: None,
            buffering: false,
            failed_frame: None,
            frame_history: Default::default(),
            skipped_frames: 0,
            last_skipped_at: None,
            volume: None,
            recent: LruCache::new(NonZeroUsize::new(RECENT_VOLUMES).unwrap()),
            loaded_site: None,
            retained_station: None,
            recenter_on_ready: false,
            camera_placed: false,
            timeline: crate::timeline::Timeline::default(),
            smooth: true,
            srv: false,
            storm_dir_deg: 240.0,
            storm_speed_kt: 25.0,
            basemap: crate::tiles::BasemapStyle::default(),
            abi_scene: None,
            show_radar: true,
            show_legend: true,
            loading: false,
            last_poll: None,
            error: None,
            fetch_failures: 0,
            failover_from: None,
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
    pub fn note_frame_status(&mut self, name: String, state: crate::timeline::FrameStatus) {
        self.frame_history.retain(|(previous, _)| previous != &name);
        if self.frame_history.len() == 64 { self.frame_history.pop_front(); }
        self.frame_history.push_back((name, state));
    }

    pub fn has_decoded_frame(&self, name: &str) -> bool {
        self.volume.as_ref().is_some_and(|volume| volume.name == name) || self.recent.contains(&name.to_owned())
    }

    pub fn station_ready(&mut self) {
        self.retained_station = None;
        if std::mem::take(&mut self.recenter_on_ready) {
            if let Some(site) = self.site.as_deref() {
                crate::ui::site_dialog::center_on_site(&mut self.camera, site);
            }
        }
    }

    /// Retain at most one previous scan, with its original station identity. Never use that
    /// volume as a live-merge base for the new station or put it in the new station's recent LRU.
    pub fn begin_station_switch(&mut self) {
        if self.site.is_none() {
            self.volume = None;
            self.retained_station = None;
        } else if self.volume.is_some() && self.retained_station.is_none() {
            self.retained_station = self.loaded_site.clone();
        }
        self.loaded_site = self.site.clone();
        self.head_request.cancel();
        self.list_request.cancel();
        self.timeline.listing = false;
        self.last_list_failure = None;
        self.recenter_on_ready = false;
        self.timeline.frames_key = None;
        self.frame_history.clear();
        self.forget_recent();
        self.cut_selection = None;
        self.frame_request.cancel();
        self.load_phase = None;
        self.failed_frame = None;
        self.buffering = false;
    }

    pub fn show_volume(&mut self, scan: Arc<Scan>, name: String, time: DateTime<Utc>) {
        self.frame_history.retain(|(previous, _)| previous != &name);
        let was_retained = self.retained_station.is_some();
        self.station_ready();
        self.frame_request.cancel();
        self.load_phase = None;
        self.failed_frame = None;
        let restored_cut = self.recovery_cut.take();
        self.cut_selection = None;
        let vol = self
            .recent
            .pop(&name)
            .unwrap_or_else(|| Volume::new(scan, name, time));
        if let Some(old) = self.volume.replace(vol) {
            // Not a volume still being written, and not a stale copy of the one just shown.
            if !was_retained && !old.live && old.name != self.volume.as_ref().expect("just set").name {
                self.recent.put(old.name.clone(), old);
            }
        }
        self.cut_selection = restored_cut.filter(|pending| {
            pending.site == self.site && self.volume.as_ref().is_some_and(|volume| volume.time.timestamp_millis() == pending.volume_ms)
        }).map(|pending| pending.selection).filter(|(ended, moment, tilt)| {
            *moment == self.moment && *tilt == self.tilt && self.volume.as_ref().is_some_and(|volume| {
                volume.cuts.iter().any(|cut| cut.ended_at_ms == *ended
                    && level2::cut_has_moment(&volume.scan, *cut, *moment)
                    && volume.elevations.get(*tilt).is_some_and(|elevation| (*elevation - cut.elevation_deg).abs() < 0.15))
            })
        });
        self.pin_low_cut();
    }

    /// Follow the lowest standard NEXRAD cut while live. Repeated cuts share the same tilt
    /// index; the decoder selects the newest sweep at that angle even before a volume completes.
    pub fn pin_low_cut(&mut self) {
        if !self.follow_low_cut || !self.timeline.following {
            return;
        }
        if let Some(index) = self.volume.as_ref().and_then(|volume| {
            volume.elevations.iter().position(|angle| (*angle - 0.5).abs() < 0.15)
        }) {
            self.tilt = index;
        }
    }

    pub fn selected_cut_ms(&self) -> Option<i64> {
        self.cut_selection
            .filter(|(_, moment, tilt)| {
                self.custom_product.is_none() && *moment == self.moment && *tilt == self.tilt
            })
            .map(|(end, _, _)| end)
    }

    /// Resolve the cut cursor against the displayed completed volume. The timeline keeps volume
    /// identifiers for loading; this supplies its ordered subframes only once that volume arrives.
    pub fn sync_cut_playback(&mut self) {
        if !self.timeline.cut_playback {
            return;
        }
        if self.custom_product.is_some() {
            self.timeline.enable_cut_playback(false);
            self.cut_selection = None;
            return;
        }
        let cuts = self.volume.as_ref().and_then(|volume| {
            (volume.live_status.is_none()
                && self.timeline.current().is_some_and(|id| id.name() == volume.name))
            .then(|| {
                volume
                    .cuts
                    .iter()
                    .copied()
                    .filter(|cut| level2::cut_has_moment(&volume.scan, *cut, self.moment))
                    .collect::<Vec<_>>()
            })
        });
        let Some(cuts) = cuts else {
            self.timeline.set_cut_count(0);
            self.cut_selection = None;
            return;
        };
        self.timeline.set_cut_count(cuts.len());
        let Some(cut) = cuts.get(self.timeline.cut_index) else {
            self.cut_selection = None;
            return;
        };
        if let Some(tilt) = self.volume.as_ref().and_then(|volume| {
            volume
                .elevations
                .iter()
                .position(|angle| (*angle - cut.elevation_deg).abs() < 0.15)
        }) {
            self.tilt = tilt;
            self.cut_selection = Some((cut.ended_at_ms, self.moment, tilt));
        } else {
            self.cut_selection = None;
        }
    }

    /// Forget every kept volume. The site changed, so none of them is of anywhere being looked at.
    pub fn forget_recent(&mut self) {
        self.recent.clear();
    }

    pub fn live_stream_ended(&mut self) {
        self.last_poll = None;
        if let Some(volume) = &mut self.volume {
            volume.end_live();
        }
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
    #[test]
    fn manual_cut_survives_loading_its_volume_after_recovery() {
        let scan = std::sync::Arc::new(wxdata::level2::decode_volume(include_bytes!("../../wxdata/tests/data/kdmx-one-sweep.bin").to_vec()).unwrap());
        let mut view = super::MapView::new(Some("KDMX".into()), crate::render::mercator::Camera::at_lonlat(-93.7, 41.7, 6.0));
        let time = chrono::Utc::now();
        view.show_volume(scan.clone(), "fixture".into(), time);
        let cut = view.volume.as_ref().unwrap().cuts.iter().find(|cut| wxdata::level2::cut_has_moment(&scan, **cut, wxdata::level2::Moment::Reflectivity)).copied().unwrap();
        let tilt = view.volume.as_ref().unwrap().elevations.iter().position(|angle| (*angle - cut.elevation_deg).abs() < 0.15).unwrap();
        view.volume = None;
        view.tilt = tilt;
        view.recovery_cut = Some(super::RecoveryCut { site: view.site.clone(), volume_ms: time.timestamp_millis(), selection: (cut.ended_at_ms, wxdata::level2::Moment::Reflectivity, tilt) });
        view.show_volume(scan.clone(), "fixture".into(), time);
        assert_eq!(view.selected_cut_ms(), Some(cut.ended_at_ms));
        view.recovery_cut = Some(super::RecoveryCut { site: view.site.clone(), volume_ms: time.timestamp_millis(), selection: (cut.ended_at_ms + 1, wxdata::level2::Moment::Reflectivity, tilt) });
        view.show_volume(scan, "fixture".into(), time);
        assert_eq!(view.cut_selection, None);
    }

    #[test]
    fn a_growing_live_volume_keeps_its_binned_tilts() {
        use super::tilts_only_grew;
        // The ordinary case: a chunk lands and appends the next sweep. Every index that already
        // has a binned sweep still means the same elevation, so the cache survives.
        assert!(tilts_only_grew(&[0.5, 0.5, 0.9], &[0.5, 0.5, 0.9, 1.3]));
        // Nothing new yet is still "only grew".
        assert!(tilts_only_grew(&[0.5, 0.9], &[0.5, 0.9]));
        // A fresh volume from an empty list.
        assert!(tilts_only_grew(&[], &[0.5]));
        // Recomputed angles wobble slightly; that is not a re-order.
        assert!(tilts_only_grew(&[0.5, 0.9], &[0.52, 0.88, 1.3]));
        // A VCP change re-orders the prefix — index 1 now means a different tilt, so every binned
        // sweep at an index is suspect and the cache has to go.
        assert!(!tilts_only_grew(&[0.5, 0.9, 1.3], &[0.5, 1.3, 0.9]));
        // Tilts disappearing is not growth either.
        assert!(!tilts_only_grew(&[0.5, 0.9, 1.3], &[0.5, 0.9]));
    }

    use super::*;
    use crate::render::mercator::Camera;

    use nexrad_model::data::{
        MomentData, PulseWidth, Radial, RadialStatus, Sweep, VolumeCoveragePattern,
    };

    fn live_status() -> wxdata::live::ScanStatus {
        wxdata::live::ScanStatus {
            provider: "test",
            volume_start_ms: Some(0),
            vcp: 212,
            cuts_received: 1,
            cuts_expected: 14,
            radials_received: 360,
            current_elevation_deg: Some(0.5),
            latency: std::time::Duration::ZERO,
            oldest_radial_age: Some(std::time::Duration::ZERO),
            sails_cuts: 0,
            mrle_cuts: 0,
            retries: 0,
            stream_active: true,
        }
    }

    /// A scan with one radial per given elevation, carrying reflectivity only.
    fn scan_at(elevations: &[f32]) -> Arc<Scan> {
        let sweeps: Vec<Sweep> = elevations
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let data = MomentData::from_fixed_point(1, 2125, 250, 8, 2.0, 66.0, vec![106u8]);
                let radial = Radial::new(
                    (i as i64 + 1) * 1_000,
                    0,
                    0.0,
                    0.5,
                    RadialStatus::ScanStart,
                    (i + 1) as u8,
                    *e,
                    Some(data),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                );
                Sweep::new((i + 1) as u8, vec![radial])
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

    #[test]
    fn custom_product_inputs_share_gate_storage_and_survive_cache_eviction() {
        let mut volume = Volume::new(scan_at(&[0.5]), "fixture".into(), Utc::now());
        let first = volume.shared_binned_display_at(Moment::Reflectivity, 0, None, false).unwrap();
        let second = volume.shared_binned_display_at(Moment::Reflectivity, 0, None, false).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let bytes = first.data.clone();
        volume.binned.clear();
        assert_eq!(first.data, bytes);
        assert_eq!(second.data, bytes);
    }

    #[test]
    fn memory_inventory_counts_shared_scan_payloads_once() {
        let first = scan_at(&[0.5, 1.5]);
        let second = scan_at(&[0.5]);
        let scans = [first.clone(), first.clone(), second.clone()];
        let mut inventory = crate::radar_memory::Inventory::default();
        assert_eq!(inventory.bytes(scans.iter()), 3);
        assert_eq!(inventory.bytes([first].iter()), 2);
    }

    #[test]
    fn low_cut_follow_is_live_only_and_leaves_missing_tilts_alone() {
        let mut view = MapView::new(
            Some("KTLX".into()),
            Camera::at_lonlat(-97.28, 35.33, 7.0),
        );
        view.follow_low_cut = true;
        view.tilt = 1;
        view.show_volume(scan_at(&[0.5, 1.5]), "live".into(), Utc::now());
        assert_eq!(view.tilt, 0);

        view.timeline.following = false;
        view.tilt = 1;
        view.pin_low_cut();
        assert_eq!(view.tilt, 1, "archive browsing keeps its selected tilt");

        view.timeline.following = true;
        view.show_volume(scan_at(&[1.5, 2.5]), "other".into(), Utc::now());
        assert_eq!(view.tilt, 1, "a radar without a 0.5° cut keeps its tilt");
    }

    #[test]
    fn linked_comparison_uses_loaded_scan_instead_of_pending_playhead() {
        let mut view = MapView::new(None, Camera::at_lonlat(0.0, 0.0, 3.0));
        let target = chrono::DateTime::from_timestamp_millis(600_000).unwrap();
        view.timeline.alignment_target = Some(target);
        assert!(!view.alignment_acceptable(Some(120)));
        view.volume = Some(Volume::new(scan_at(&[0.5]), "old".into(), target - chrono::Duration::seconds(180)));
        assert_eq!(view.displayed_time(), Some(target - chrono::Duration::seconds(180)));
        assert_eq!(view.alignment_offset_seconds(), Some(-180));
        assert!(!view.alignment_acceptable(Some(120)));
        view.volume.as_mut().unwrap().time = target - chrono::Duration::seconds(120);
        assert!(view.alignment_acceptable(Some(120)));
        view.volume.as_mut().unwrap().time -= chrono::Duration::milliseconds(1);
        assert_eq!(view.alignment_offset_milliseconds(), Some(-120_001));
        assert!(!view.alignment_acceptable(Some(120)), "a fractional second beyond the limit is outside the window");
        view.timeline.alignment_target = None;
        assert!(view.alignment_acceptable(Some(0)));
    }

    #[test]
    fn cut_selection_tracks_product_and_resets_on_volume_change() {
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.28, 35.33, 7.0));
        view.show_volume(scan_at(&[0.5]), "first".into(), Utc::now());
        view.cut_selection = Some((1_000, Moment::Reflectivity, 0));
        assert_eq!(view.selected_cut_ms(), Some(1_000));
        view.moment = Moment::Velocity;
        assert_eq!(view.selected_cut_ms(), None);
        view.moment = Moment::Reflectivity;
        view.show_volume(scan_at(&[0.5]), "second".into(), Utc::now());
        assert_eq!(view.selected_cut_ms(), None);
    }

    #[test]
    fn cut_playback_uses_decoded_collection_order_and_tilt() {
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.28, 35.33, 7.0));
        let id = wxdata::level2::Identifier::new("KTLX20260819_000000_V06".into());
        let next = wxdata::level2::Identifier::new("KTLX20260819_000500_V06".into());
        view.timeline.set_frames(vec![id.clone(), next.clone()], ("KTLX".into(), view.timeline.date));
        view.show_volume(scan_at(&[0.5, 1.5]), id.name().into(), Utc::now());
        view.timeline.enable_cut_playback(true);
        view.sync_cut_playback();
        assert_eq!((view.timeline.cut_count, view.selected_cut_ms(), view.tilt), (2, Some(1_000), 0));
        view.timeline.step_cut(1);
        view.sync_cut_playback();
        assert_eq!((view.selected_cut_ms(), view.tilt), (Some(2_000), 1));
        view.timeline.step_cut(1);
        view.sync_cut_playback();
        assert_eq!(view.selected_cut_ms(), None, "old volume is not the new playhead");
        view.show_volume(scan_at(&[0.5, 1.5]), next.name().into(), Utc::now());
        view.sync_cut_playback();
        assert_eq!((view.timeline.cut_count, view.selected_cut_ms()), (2, Some(1_000)));
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
        let sample = |volume: &mut Volume| volume.shared_binned_display_at(Moment::Reflectivity, 0, None, false).unwrap();
        view.show_volume(scan_at(&[0.5, 1.5]), "a".into(), now);
        assert!(view.volume.as_ref().unwrap().binned.is_empty());
        let first_a = sample(view.volume.as_mut().unwrap());
        assert_eq!(view.volume.as_ref().unwrap().binned.len(), 1);
        view.show_volume(scan_at(&[0.5, 1.5]), "b".into(), now);
        let first_b = sample(view.volume.as_mut().unwrap());
        view.show_volume(scan_at(&[0.5, 1.5]), "a".into(), now);
        assert_eq!(view.volume.as_ref().unwrap().name, "a");
        assert!(Arc::ptr_eq(&first_a, &sample(view.volume.as_mut().unwrap())), "second lap must retain the actual binned allocation");
        view.forget_recent();
        view.show_volume(scan_at(&[0.5, 1.5]), "b".into(), now);
        assert!(!Arc::ptr_eq(&first_b, &sample(view.volume.as_mut().unwrap())), "site reset must discard the old allocation");

        // A volume still being written must never be kept: half its tilts may not have arrived,
        // and showing it again later in place of the complete archive volume loses them.
        view.forget_recent();
        view.show_volume(scan_at(&[0.5, 1.5]), "live".into(), now);
        view.volume.as_mut().unwrap().apply_live(
            scan_at(&[0.5]),
            "live".into(),
            now,
            &[],
            live_status(),
        );
        view.show_volume(scan_at(&[0.5, 1.5]), "next".into(), now);
        view.show_volume(scan_at(&[0.5, 1.5]), "live".into(), now);
        assert_eq!(
            view.volume.as_ref().unwrap().elevations,
            vec![0.5, 1.5],
            "the half-written live volume was kept and shown again"
        );

        // Retain allocations so pointer reuse cannot hide a re-bin. These assertions are
        // local to this test, unlike process-wide telemetry shared by parallel test threads.
        let names: Vec<String> = (0..10).map(|i| format!("frame-{i}")).collect();
        let mut fresh = Vec::new();
        for _ in 0..3 {
            for name in &names {
                let mut volume = Volume::new(scan_at(&[0.5, 1.5]), name.clone(), now);
                fresh.push(sample(&mut volume));
            }
        }
        let unique: std::collections::HashSet<_> = fresh.iter().map(Arc::as_ptr).collect();
        assert_eq!(unique.len(), 30, "fresh volumes allocate on every lap");
        let mut view = MapView::new(Some("KTLX".into()), crate::render::mercator::Camera::at_lonlat(-97.28, 35.33, 7.0));
        let mut retained = std::collections::HashMap::new();
        for _ in 0..3 {
            for name in &names {
                view.show_volume(scan_at(&[0.5, 1.5]), name.clone(), now);
                let sweep = sample(view.volume.as_mut().unwrap());
                if let Some(previous) = retained.get(name) { assert!(Arc::ptr_eq(previous, &sweep)); }
                else { retained.insert(name.clone(), sweep); }
            }
        }
        assert_eq!(retained.len(), 10, "one allocation per volume, then two free laps");
    }

    /// A live volume that has only just started has no tilts in it yet. Applying it emptied the
    /// tilt list and made the next bin fail with "tilt 0 out of range".
    #[test]
    fn a_tiltless_live_update_does_not_replace_the_volume_on_screen() {
        let now = chrono::Utc::now();
        let mut vol = Volume::new(scan_at(&[0.5, 1.5]), "a".into(), now);
        assert_eq!(vol.elevations, vec![0.5, 1.5]);
        vol.apply_live(scan_at(&[]), "b".into(), now, &[], live_status());
        assert_eq!(vol.elevations, vec![0.5, 1.5], "kept the tilts it had");
        assert_eq!(vol.name, "a", "and the volume they came from");
        // A real volume still applies.
        vol.apply_live(scan_at(&[0.5]), "c".into(), now, &[], live_status());
        assert_eq!(vol.elevations, vec![0.5]);
        assert_eq!(vol.name, "c");
        vol.end_live();
        assert!(!vol.live_status.as_ref().unwrap().stream_active);
    }

    #[test]
    fn ending_a_live_stream_requests_archive_fallback_immediately() {
        let now = chrono::Utc::now();
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 8.0));
        let mut volume = Volume::new(scan_at(&[0.5]), "live".into(), now);
        volume.live_status = Some(live_status());
        view.volume = Some(volume);
        view.last_poll = Some(Instant::now());
        view.live_stream_ended();
        assert!(view.last_poll.is_none());
        assert!(!view.volume.unwrap().live_status.unwrap().stream_active);
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

    #[test]
    fn station_switch_retains_one_identified_scan_and_replacement_discards_old_site_cache() {
        let old = scan_at(&[0.5]);
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 8.0));
        view.loaded_site = view.site.clone();
        view.show_volume(Arc::clone(&old), "old".into(), Utc::now());
        view.site = Some("KDMX".into());
        view.begin_station_switch();
        assert!(Arc::ptr_eq(&view.volume.as_ref().unwrap().scan, &old));
        assert_eq!(view.retained_station.as_deref(), Some("KTLX"));
        assert_eq!(view.displayed_station(), Some("KTLX"));
        view.site = Some("KOUN".into());
        view.begin_station_switch();
        assert_eq!(view.retained_station.as_deref(), Some("KTLX"));
        view.show_volume(scan_at(&[0.5]), "replacement".into(), Utc::now());
        assert!(view.retained_station.is_none());
        assert_eq!(view.displayed_station(), Some("KOUN"));
        assert!(!view.has_decoded_frame("old"), "old station must not enter new station's recent LRU");
        view.site = None;
        view.begin_station_switch();
        assert!(view.volume.is_none());
        assert_eq!(view.displayed_station(), None);
    }

    #[test]
    fn binned_budget_evicts_old_tilts_but_preserves_the_newest_display_bin() {
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 8.0));
        view.show_volume(scan_at(&[0.5, 1.5]), "fixture".into(), Utc::now());
        let volume = view.volume.as_mut().unwrap();
        volume.binned_display_at(Moment::Reflectivity, 0, false, None, false).unwrap();
        volume.binned_display_at(Moment::Reflectivity, 1, false, None, false).unwrap();
        let bytes = view.binned_cache_bytes();
        view.trim_binned_memory(1);
        let volume = view.volume.as_ref().unwrap();
        assert_eq!(volume.binned.len(), 1);
        assert_eq!(volume.binned.iter().next().unwrap().0.1, 1);
        assert!(view.binned_cache_bytes() < bytes);
    }

    #[test]
    fn unavailable_frame_history_is_bounded_and_success_removes_failure() {
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.0, 35.0, 8.0));
        for frame in 0..100 { view.note_frame_status(frame.to_string(), crate::timeline::FrameStatus::Unavailable); }
        assert_eq!(view.frame_history.len(), 64);
        view.note_frame_status("99".into(), crate::timeline::FrameStatus::Skipped);
        assert_eq!(view.frame_history.len(), 64);
        view.show_volume(scan_at(&[0.5]), "99".into(), Utc::now());
        assert!(!view.frame_history.iter().any(|(name, _)| name == "99"));
    }

    #[test]
    fn new_views_ship_with_the_reflectivity_floor() {
        let v = MapView::new(None, Camera::at_lonlat(-97.0, 35.0, 8.0));
        assert_eq!(v.active_threshold(), Some(16.0));
    }

    #[test]
    fn diagnostic_state_preserves_public_reproduction_without_private_selectors() {
        let mut view = MapView::new(Some("KTLX".into()), Camera::at_lonlat(-97.123456, 35.654321, 8.0));
        view.loaded_site = Some("private-provider-token".into());
        view.custom_product = Some("private-product-name".into());
        view.error = Some("https://private.example/?key=secret".into());
        view.timeline.playing = true;
        let report = view.diagnostic_state();
        assert_eq!(report["selected_station"], "KTLX");
        assert!(report["loaded_station"].is_null());
        assert_eq!(report["playing"], true);
        assert_eq!(report["custom_product_selected"], true);
        let text = report.to_string();
        for private in ["private-provider-token", "private-product-name", "private.example", "secret", "97.123456", "35.654321"] {
            assert!(!text.contains(private), "diagnostic leaked {private}");
        }
    }
}
