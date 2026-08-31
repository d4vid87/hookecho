//! Level 2 acquisition and sweep binning.
//!
//! Pipeline: AWS archive listing -> download an Archive II volume -> decode to a
//! [`Scan`] -> bin a chosen moment of one sweep into a dense polar grid the GPU can
//! sample. The binned grid is deliberately simple (fixed azimuth bins, `u8` values)
//! so the render side is a single texture upload.

use nexrad_model::data::{DataMoment, MomentData, MomentValue, Sweep};
use std::path::PathBuf;

/// Re-exported so the app can name decoded volumes without depending on `nexrad-model`.
pub use nexrad_model::data::Scan;

/// Which radar moment to extract from a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Moment {
    Reflectivity,
    Velocity,
    SpectrumWidth,
    DifferentialReflectivity,
    DifferentialPhase,
    /// Specific differential phase — not transmitted; derived from [`Moment::DifferentialPhase`]
    /// at bin time by [`crate::kdp`].
    SpecificDifferentialPhase,
    CorrelationCoefficient,
}

impl Moment {
    /// All moments, in toolbox/hotkey display order (REF..RHO).
    pub const ALL: [Moment; 7] = [
        Moment::Reflectivity,
        Moment::Velocity,
        Moment::SpectrumWidth,
        Moment::DifferentialReflectivity,
        Moment::DifferentialPhase,
        Moment::SpecificDifferentialPhase,
        Moment::CorrelationCoefficient,
    ];

    /// Short product code shown on toolbox buttons and accepted on the CLI.
    pub fn short_name(&self) -> &'static str {
        match self {
            Moment::Reflectivity => "REF",
            Moment::Velocity => "VEL",
            Moment::SpectrumWidth => "SW",
            Moment::DifferentialReflectivity => "ZDR",
            Moment::DifferentialPhase => "PHI",
            Moment::SpecificDifferentialPhase => "KDP",
            Moment::CorrelationCoefficient => "CC",
        }
    }

    /// Physical units label for the legend and threshold slider.
    pub fn units(&self) -> &'static str {
        match self {
            Moment::Reflectivity => "dBZ",
            Moment::Velocity | Moment::SpectrumWidth => "m/s",
            Moment::DifferentialReflectivity => "dB",
            Moment::DifferentialPhase => "deg",
            Moment::SpecificDifferentialPhase => "deg/km",
            Moment::CorrelationCoefficient => "",
        }
    }

    /// Parse a short code (case-insensitive); `None` if unrecognized.
    pub fn from_code(code: &str) -> Option<Moment> {
        if code.eq_ignore_ascii_case("RHO") {
            return Some(Moment::CorrelationCoefficient); // legacy code, pre-CC rename
        }
        Moment::ALL
            .iter()
            .copied()
            .find(|m| m.short_name().eq_ignore_ascii_case(code))
    }

    /// Position in [`Moment::ALL`] — a stable index for per-moment arrays.
    pub fn index(self) -> usize {
        Moment::ALL.iter().position(|m| *m == self).unwrap()
    }

    pub(crate) fn select<'a>(
        &self,
        radial: &'a nexrad_model::data::Radial,
    ) -> Option<&'a MomentData> {
        match self {
            Moment::Reflectivity => radial.reflectivity(),
            Moment::Velocity => radial.velocity(),
            Moment::SpectrumWidth => radial.spectrum_width(),
            Moment::DifferentialReflectivity => radial.differential_reflectivity(),
            // KDP rides on the phase field: it is what the derivative is taken of, so sweep
            // selection and gate geometry are decided by ΦDP's presence.
            Moment::DifferentialPhase | Moment::SpecificDifferentialPhase => {
                radial.differential_phase()
            }
            Moment::CorrelationCoefficient => radial.correlation_coefficient(),
        }
    }

    /// Physical value range used to normalize gate values into the 2..=255 `u8` band.
    /// 0 = below-threshold (transparent), 1 = range-folded.
    pub fn value_range(&self) -> (f32, f32) {
        match self {
            Moment::Reflectivity => (-32.0, 95.0),            // dBZ
            Moment::Velocity => (-127.0, 127.0),              // m/s (pre-dealias)
            Moment::SpectrumWidth => (0.0, 63.0),             // m/s
            Moment::DifferentialReflectivity => (-7.9, 7.9),  // dB
            Moment::DifferentialPhase => (0.0, 360.0),        // deg
            Moment::SpecificDifferentialPhase => (-2.0, 8.0), // deg/km
            Moment::CorrelationCoefficient => (0.0, 1.05),
        }
    }
}

/// A sweep resampled onto a fixed azimuth grid, ready for GPU upload.
///
/// `data` is row-major `[az_bin][gate]`, one `u8` per gate:
/// `0` below threshold, `1` range folded, `2..=255` linearly maps the moment's
/// [`Moment::value_range`].
#[derive(Debug, Clone)]
pub struct BinnedSweep {
    pub moment: Moment,
    pub az_bins: usize,
    pub gate_count: usize,
    pub data: Vec<u8>,
    pub first_gate_km: f32,
    pub gate_interval_km: f32,
    pub radar_lat: f32,
    pub radar_lon: f32,
    pub elevation_deg: f32,
    /// Inverse of `value_range`, for the shader/legend to recover physical units.
    pub value_min: f32,
    pub value_max: f32,
}

/// The first UTC day with volumes in the AWS NEXRAD archive.
///
/// Nothing before this exists to be asked for, so it is where a date picker has to stop. The
/// early years are legacy pre-dual-pol Type-1 messages, which decode but carry reflectivity
/// and velocity only.
pub const ARCHIVE_START: chrono::NaiveDate = match chrono::NaiveDate::from_ymd_opt(1991, 6, 5) {
    Some(d) => d,
    None => panic!("1991-06-05 is a real date"),
};

/// What a [`BinnedSweep`] holds at one gate, and where that gate is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateSample {
    /// Physical value in the moment's units; `None` when the gate is below threshold or the
    /// value is range-folded (see [`GateSample::folded`]).
    pub value: Option<f32>,
    /// The gate is range-folded — the radar saw a target it cannot place in range, which is a
    /// different statement from "nothing there".
    pub folded: bool,
    /// Azimuth from the radar, degrees clockwise from north.
    pub azimuth_deg: f32,
    /// Slant range from the radar along the beam, km.
    pub range_km: f32,
    /// Gate index along the radial — what a range-resolution argument is actually about.
    pub gate: usize,
}

impl BinnedSweep {
    /// The gate under a ground position, or `None` when it falls outside this sweep.
    ///
    /// This is the inverse of the binning: ground distance and bearing from the radar, the
    /// slant range that ground distance implies at this tilt, and then the same azimuth-bin and
    /// gate-index arithmetic [`bin_sweep_opts`] used on the way in. Deliberately no
    /// interpolation — the question a readout answers is "what did the radar record *here*",
    /// and a blend of neighbouring gates is a number the radar never produced.
    pub fn sample_at(&self, lon: f64, lat: f64) -> Option<GateSample> {
        let (ground_km, bearing) =
            crate::xsection::dist_bearing(self.radar_lon as f64, self.radar_lat as f64, lon, lat);
        let slant = crate::xsection::slant_from_ground_km(ground_km, self.elevation_deg as f64);
        let gate =
            ((slant - self.first_gate_km as f64) / self.gate_interval_km.max(1e-6) as f64).floor();
        if gate < 0.0 || gate >= self.gate_count as f64 {
            return None;
        }
        let gate = gate as usize;
        let az = bearing.rem_euclid(360.0);
        let bin = ((az / 360.0 * self.az_bins as f64) as usize) % self.az_bins;
        let code = *self.data.get(bin * self.gate_count + gate)?;
        // 0 and 1 are the two sentinel codes the binner writes; 2..=255 is the value band.
        let value = (code >= 2).then(|| {
            let t = (code - 2) as f32 / 253.0;
            self.value_min + t * (self.value_max - self.value_min)
        });
        Some(GateSample {
            value,
            folded: code == 1,
            azimuth_deg: az as f32,
            range_km: slant as f32,
            gate,
        })
    }

    /// Height of the beam centre above the radar at `range_km`, in feet — the number that says
    /// whether a reading is near the ground or three miles up.
    pub fn beam_height_ft(&self, range_km: f32) -> f64 {
        crate::xsection::beam_height_km(range_km as f64, self.elevation_deg as f64) * 3280.84
    }
}

/// An AWS archive volume identifier (re-exported so callers needn't depend on `nexrad-data`).
pub use nexrad_data::aws::archive::Identifier;

/// The most recent volume identifier for `site`, checking today then yesterday.
///
/// The yesterday fallback covers the window just after 00Z when today's UTC day has
/// no volumes yet (e.g. evening in the US). `site` is the 4-letter ICAO id.
pub async fn latest_identifier(site: &str) -> anyhow::Result<Identifier> {
    Ok(latest_identifiers(site, 1).await?.remove(0))
}

/// The newest `n` volumes for `site`, newest first.
///
/// Callers want more than one because the newest volume on S3 is usually still being uploaded —
/// the radar writes it as it scans. A volume caught early enough can be missing the metadata
/// record that carries its scan strategy, and the only cure is to fall back a volume.
pub async fn latest_identifiers(site: &str, n: usize) -> anyhow::Result<Vec<Identifier>> {
    let today = chrono::Utc::now().date_naive();
    let mut out: Vec<Identifier> = Vec::new();
    for day in [today, today.pred_opt().unwrap_or(today)] {
        let mut ids = list_day(site, day).await?;
        while out.len() < n {
            match ids.pop() {
                Some(id) => out.push(id),
                None => break,
            }
        }
        if out.len() >= n {
            break;
        }
    }
    if out.is_empty() {
        anyhow::bail!("no volumes for {site} today or yesterday");
    }
    Ok(out)
}

/// Volumes ending on a specific UTC `date`, oldest first — enough of them to fill a loop.
///
/// The archive is bucketed by UTC day, so a naive listing of "today" is nearly empty just after
/// 00Z: for the first half hour of every UTC day the newest few volumes are all there is, and a
/// loop that wants the last fifteen minutes cannot be built from them. When the day is young this
/// borrows the tail of the previous day, which is where those minutes actually live.
///
/// Only for the current day, and only when it is short: a deliberate scrub back to a past date
/// should show that date, not a few hours of the one before it.
pub async fn list_volumes(site: &str, date: chrono::NaiveDate) -> anyhow::Result<Vec<Identifier>> {
    /// Roughly two hours at a severe-weather VCP — comfortably more than any loop window, and
    /// still one extra listing at most.
    const MIN_FRAMES: usize = 24;

    let ids = list_day(site, date).await?;
    if ids.len() >= MIN_FRAMES || date != chrono::Utc::now().date_naive() {
        return Ok(ids);
    }
    let Some(prev) = date.pred_opt() else {
        return Ok(ids);
    };
    // A failed listing for yesterday is not a failure: today's frames are still perfectly good.
    let Ok(older) = list_day(site, prev).await else {
        return Ok(ids);
    };
    Ok(with_previous_tail(older, ids, MIN_FRAMES))
}

/// Take just enough of `older` (oldest first) to bring `today` up to `min`, and put it in front.
fn with_previous_tail(
    mut older: Vec<Identifier>,
    today: Vec<Identifier>,
    min: usize,
) -> Vec<Identifier> {
    let keep = min.saturating_sub(today.len());
    older.drain(..older.len().saturating_sub(keep));
    older.extend(today);
    older
}

/// Recent day listings, so a site switch doesn't LIST the same S3 prefix two or three times over
/// (the head poll and the timeline listing both want today's).
///
/// ponytail: a flat map with a short TTL, swept of expired entries on insert — a session touches
/// a handful of site/day pairs at a time. Bound it by count if that ever stops being true.
type DayListCache = std::collections::HashMap<
    (String, chrono::NaiveDate),
    (crate::clock::Instant, Vec<Identifier>),
>;
static DAY_LIST_CACHE: std::sync::OnceLock<std::sync::Mutex<DayListCache>> =
    std::sync::OnceLock::new();

/// Listing TTL. Volumes land every 4-6 minutes; 20 s is short enough that the head poll still
/// sees a new one promptly and long enough to collapse the burst a site switch makes.
const DAY_LIST_TTL: std::time::Duration = std::time::Duration::from_secs(20);

/// Whether an archive object name is a radar volume rather than one of the sidecar objects the
/// bucket carries alongside them.
///
/// `..._V06_MDM` is a Metadata Message file: the volume's metadata records with none of the
/// radials. It sorts in among the volumes and, taken for one, becomes a "newest volume" that is
/// really a few minutes older and has nothing to draw — which is how a site whose feed had
/// stopped ended up showing 09:55 when its last actual volume was 10:12.
fn is_volume(name: &str) -> bool {
    !name.ends_with("_MDM")
}

async fn list_day(site: &str, date: chrono::NaiveDate) -> anyhow::Result<Vec<Identifier>> {
    use nexrad_data::aws::archive;
    let key = (site.to_string(), date);
    let cache = DAY_LIST_CACHE.get_or_init(Default::default);
    if let Ok(map) = cache.lock() {
        if let Some((at, ids)) = map.get(&key) {
            if at.elapsed() < DAY_LIST_TTL {
                crate::stats::bump(crate::stats::Counter::DayListHits);
                return Ok(ids.clone());
            }
        }
    }
    crate::stats::bump(crate::stats::Counter::DayListMisses);
    let mut ids = archive::list_files(site, &date)
        .await
        .map_err(|e| anyhow::anyhow!("list_files({site}, {date}): {e}"))?;
    ids.retain(|id| is_volume(id.name()));
    ids.sort_by_key(|id| id.date_time());
    if let Ok(mut map) = cache.lock() {
        // Scrubbing a week of dates across a few sites left every one of those listings here for
        // the life of the process; an expired entry can never be read, so drop it as we pass.
        map.retain(|_, (at, _)| at.elapsed() < DAY_LIST_TTL);
        map.insert(key, (crate::clock::Instant::now(), ids.clone()));
    }
    Ok(ids)
}

/// Decode raw Archive II bytes to a [`Scan`], decompressing first if they are bzip2-packed.
///
/// The one decode path: the download, the on-disk cache and the fuzz target all come through here,
/// so a volume that decodes in one place decodes the same way in the others.
pub fn decode_volume(data: Vec<u8>) -> anyhow::Result<Scan> {
    decode_file(nexrad_data::volume::File::new(data))
}

/// bzip2 decompression plus the message decode is tens of MB of pure CPU — callers on an async
/// worker must run this on a blocking thread, or it stalls every other fetch sharing that thread.
fn decode_file(file: nexrad_data::volume::File) -> anyhow::Result<Scan> {
    // Anything shorter than the 24-byte Archive II volume header is not a volume. The decoder
    // slices past the header unconditionally and panics on a short buffer (found by
    // fuzz/fuzz_targets/level2_decode.rs), and the live head really does serve half-written
    // objects — so this is a guard on real input, not a fuzz-only nicety.
    const VOLUME_HEADER_LEN: usize = 24;
    if file.data().len() < VOLUME_HEADER_LEN {
        anyhow::bail!("volume is {} bytes, too short to be one", file.data().len());
    }
    let file = if file.compressed() {
        file.decompress()
            .map_err(|e| anyhow::anyhow!("decompress: {e}"))?
    } else {
        file
    };
    file.scan().map_err(|e| anyhow::anyhow!("scan: {e}"))
}

/// Decode raw Archive II bytes and re-encode the [`Scan`] as postcard, for the Web Worker.
///
/// The worker runs a second instance of this same wasm module, so both sides agree on the format
/// by construction — there is no version to negotiate. Exported to JS by `hookecho::decode_archive2`.
#[cfg(target_arch = "wasm32")]
pub fn decode_and_encode(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let scan = decode_volume(bytes)?;
    postcard::to_allocvec(&scan).map_err(|e| anyhow::anyhow!("encode scan: {e}"))
}

/// The other half of [`decode_and_encode`], run on the main thread.
#[cfg(target_arch = "wasm32")]
pub fn scan_from_wire(bytes: &[u8]) -> anyhow::Result<Scan> {
    postcard::from_bytes(bytes).map_err(|e| anyhow::anyhow!("decode scan wire: {e}"))
}

/// Raw Archive II bytes for one volume, straight from the bucket.
pub async fn volume_bytes(id: Identifier) -> anyhow::Result<Vec<u8>> {
    let f = nexrad_data::aws::archive::download_file(id)
        .await
        .map_err(|e| anyhow::anyhow!("download_file: {e}"))?;
    crate::stats::net(f.data().len());
    Ok(f.data().to_vec())
}

/// Decode raw Archive II bytes that came from somewhere other than the bucket — a browser's
/// offline pack, say. Same decode path (and the same Web Worker hop) `download_scan` uses.
pub async fn scan_from_volume_bytes(name: &str, bytes: Vec<u8>) -> anyhow::Result<Scan> {
    let file = nexrad_data::volume::File::new(bytes);
    #[cfg(not(target_arch = "wasm32"))]
    let scan = crate::task::blocking(move || decode_file(file)).await??;
    #[cfg(target_arch = "wasm32")]
    let scan = match crate::wasm_worker::decode_volume(file.data().to_vec()).await {
        Ok(wire) => scan_from_wire(&wire)?,
        Err(crate::wasm_worker::Error::Unavailable) => decode_file(file)?,
        Err(e) => anyhow::bail!("{e}"),
    };
    Ok(match scan.site() {
        Some(_) => scan,
        None => with_registry_site(scan, &name[..4.min(name.len())]),
    })
}

/// Download and decode a specific volume to a [`Scan`].
///
/// With `cache_dir` set the raw Archive II bytes are kept on disk, exactly as the bucket served
/// them: an archived volume never changes, so a scrub back through yesterday's outbreak is a file
/// read instead of a download. Raw bytes rather than a decoded scan because they're smaller, need
/// no serialization of their own, and go back through the same decode path either way. Pass `None`
/// at the live head, where the newest object may still be mid-write.
pub async fn download_scan(id: Identifier, cache_dir: Option<PathBuf>) -> anyhow::Result<Scan> {
    use nexrad_data::aws::archive;
    let name = id.name().to_string();
    let cache_file = cache_dir.map(|d| d.join("volumes").join(&name));
    let cached = cache_file
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .map(nexrad_data::volume::File::new);
    let file = match cached {
        Some(f) => f,
        None => {
            let f = archive::download_file(id)
                .await
                .map_err(|e| anyhow::anyhow!("download_file: {e}"))?;
            crate::stats::net(f.data().len());
            if let Some(p) = &cache_file {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, f.data());
            }
            f
        }
    };
    // bzip2 decompression plus the message decode is tens of MB of pure CPU. On the async worker
    // it blocks every other fetch sharing that thread for as long as it runs; the `parallel`
    // feature of nexrad-data then spreads the decode itself across rayon.
    #[cfg(not(target_arch = "wasm32"))]
    let scan = crate::task::blocking(move || decode_file(file)).await??;
    // In the browser that "async worker" is the thread drawing the map, so try the Web Worker
    // first. ponytail: no worker (old browser, `file://`, a worker that already trapped) means
    // inline decode, jank and all — a frozen map still beats no radar.
    #[cfg(target_arch = "wasm32")]
    let scan = match crate::wasm_worker::decode_volume(file.data().to_vec()).await {
        Ok(wire) => scan_from_wire(&wire)?,
        Err(crate::wasm_worker::Error::Unavailable) => decode_file(file)?,
        Err(e) => anyhow::bail!("{e}"),
    };
    // Legacy (pre-2008) volumes carry no volume data block, so the decoder can't name the radar.
    // The volume's own filename can: "KTLX19910605_162126".
    Ok(match scan.site() {
        Some(_) => scan,
        None => with_registry_site(scan, &name[..4.min(name.len())]),
    })
}

/// Attach registry site metadata to a scan that arrived without any — everything downstream
/// (binning, cross-sections, derived products) needs the radar's position.
fn with_registry_site(scan: Scan, id: &str) -> Scan {
    match crate::sites::site_by_id(id) {
        Some(entry) => Scan::with_site(
            entry.to_site(),
            scan.coverage_pattern().clone(),
            scan.sweeps().to_vec(),
        ),
        None => scan,
    }
}

/// List and download the most recent volume for `site` on `date`, decoding it to a [`Scan`].
///
/// Retained for the headless harness and tests. `date` a UTC calendar day.
pub async fn download_latest_scan(site: &str, date: chrono::NaiveDate) -> anyhow::Result<Scan> {
    let latest = list_volumes(site, date)
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("no volumes for {site} on {date}"))?;
    download_scan(latest, None).await
}

/// Sorted, deduplicated elevation angles (degrees) of a scan's sweeps.
///
/// Split cuts and SAILS revisits at (nearly) the same angle collapse to one entry, so
/// the index into this list is the tilt the toolbox selects.
pub fn elevation_angles(scan: &Scan) -> Vec<f32> {
    let mut angles: Vec<f32> = scan
        .sweeps()
        .iter()
        .filter_map(|s| s.elevation_angle_degrees())
        .collect();
    angles.sort_by(f32::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 0.15);
    angles
}

/// Which moments this volume actually carries, indexed by [`Moment::index`].
///
/// Not every radar sends everything: a TDWR has only reflectivity and velocity, and volumes from
/// before the 2011-13 dual-polarization upgrade have no ZDR/PHI/CC. The UI reads this so those
/// products are absent rather than selectable-and-blank.
pub fn available_moments(scan: &Scan) -> [bool; Moment::ALL.len()] {
    let mut got = [false; Moment::ALL.len()];
    for sweep in scan.sweeps() {
        for radial in sweep.radials() {
            for m in Moment::ALL {
                got[m.index()] |= m.select(radial).is_some();
            }
        }
        if got.iter().all(|g| *g) {
            break;
        }
    }
    got
}

/// Bin the sweep at tilt index `tilt` (into [`elevation_angles`]) for `moment`.
///
/// Split-cut aware: among the sweeps at that elevation it picks the first whose radials
/// actually carry `moment` — the lowest tilt often has a reflectivity-only surveillance
/// cut alongside a Doppler cut, so naively taking the first sweep can miss VEL/SW.
pub fn bin_scan(scan: &Scan, moment: Moment, tilt: usize) -> anyhow::Result<BinnedSweep> {
    bin_scan_opts(scan, moment, tilt, false)
}

/// Like [`bin_scan`] but `dealias` unfolds aliased Doppler velocity (ignored for other moments).
pub fn bin_scan_opts(
    scan: &Scan,
    moment: Moment,
    tilt: usize,
    dealias: bool,
) -> anyhow::Result<BinnedSweep> {
    crate::stats::bump(crate::stats::Counter::SweepsBinned);
    let target = *elevation_angles(scan)
        .get(tilt)
        .ok_or_else(|| anyhow::anyhow!("tilt {tilt} out of range"))?;

    let sweep = scan
        .sweeps()
        .iter()
        .filter(|s| {
            s.elevation_angle_degrees()
                .is_some_and(|e| (e - target).abs() < 0.15)
        })
        .find(|s| s.radials().iter().any(|r| moment.select(r).is_some()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no sweep at tilt {tilt} ({target:.2}deg) carries {}",
                moment.short_name()
            )
        })?;

    let (lat, lon) = scan
        .site()
        .map(|s| (s.latitude(), s.longitude()))
        .ok_or_else(|| anyhow::anyhow!("scan has no site metadata"))?;

    bin_sweep_opts(sweep, moment, lat, lon, dealias)
}

/// Bin the lowest-elevation sweep of `scan` for `moment`.
pub fn bin_lowest_sweep(scan: &Scan, moment: Moment) -> anyhow::Result<BinnedSweep> {
    bin_scan(scan, moment, 0)
}

/// Which sweep a dealias continuity reference belongs to. The site is in the key so two panes on
/// two radars never hand each other a reference; the gate count is in it because a reference of a
/// different shape is unusable.
#[derive(PartialEq, Eq, Clone, Copy)]
struct DealiasKey {
    lat_e2: i32,
    lon_e2: i32,
    elevation_number: u8,
    gate_count: usize,
}

/// How far back a reference may be. A volume lands every 4-6 minutes; past about three of them
/// the wind field has moved on and anchoring to it is worse than anchoring to zero.
const DEALIAS_REF_MAX_AGE_MS: i64 = 15 * 60 * 1000;

/// ponytail: exactly one reference is kept, not one per tilt. The dealiased view is a tilt at a
/// time, so the entry that matters is nearly always the one that is here; changing tilt just
/// costs the first frame its reference, which is what the code did everywhere before. It is a
/// ~7 MB field, and one of them is cheap where six would not be. Key it into a small map if
/// flipping tilts on a folded storm turns out to matter.
#[allow(clippy::type_complexity)]
// The field is shared, never edited: an `Arc` so keeping it and handing it back both cost a
// pointer instead of a ~10 MB copy each.
static DEALIAS_REF: std::sync::Mutex<Option<(DealiasKey, i64, DealiasField)>> =
    std::sync::Mutex::new(None);

type DealiasField = std::sync::Arc<Vec<Option<f32>>>;

/// The previous pass over this tilt, if there is one and it is recent enough to mean anything.
/// A reference from *later* than this sweep is a scrub backwards through the timeline: skipped,
/// because "previous" is the only relationship the dealiaser can use.
fn take_dealias_reference(key: &DealiasKey, at: i64) -> Option<DealiasField> {
    let guard = DEALIAS_REF.lock().ok()?;
    let (k, t, field) = guard.as_ref()?;
    (k == key && at > *t && at - *t <= DEALIAS_REF_MAX_AGE_MS).then(|| field.clone())
}

fn put_dealias_reference(key: DealiasKey, at: i64, field: DealiasField) {
    if let Ok(mut guard) = DEALIAS_REF.lock() {
        *guard = Some((key, at, field));
    }
}

/// Bin one sweep's `moment` into a fixed azimuth grid.
pub fn bin_sweep(
    sweep: &Sweep,
    moment: Moment,
    radar_lat: f32,
    radar_lon: f32,
) -> anyhow::Result<BinnedSweep> {
    bin_sweep_opts(sweep, moment, radar_lat, radar_lon, false)
}

/// Like [`bin_sweep`] but `dealias` unfolds aliased Doppler velocity (ignored for other moments).
pub fn bin_sweep_opts(
    sweep: &Sweep,
    moment: Moment,
    radar_lat: f32,
    radar_lon: f32,
    dealias: bool,
) -> anyhow::Result<BinnedSweep> {
    let radials = sweep.radials();
    // Azimuth resolution: 0.5-degree (720 bins) covers both super-res and legacy;
    // legacy 1-degree radials fill the two adjacent bins they span.
    const AZ_BINS: usize = 720;
    const BIN_DEG: f32 = 360.0 / AZ_BINS as f32;

    // Gate geometry from the first radial that carries this moment.
    let sample = radials
        .iter()
        .find_map(|r| moment.select(r))
        .ok_or_else(|| anyhow::anyhow!("no radial carries the requested moment"))?;
    let gate_count = sample.gate_count() as usize;
    let first_gate_km = sample.first_gate_range_km() as f32;
    let gate_interval_km = sample.gate_interval_km() as f32;

    let (value_min, value_max) = moment.value_range();
    let span = (value_max - value_min).max(f32::EPSILON);

    let normalize = |v: f32| -> u8 {
        let t = ((v - value_min) / span).clamp(0.0, 1.0);
        2 + (t * 253.0) as u8
    };

    // Bucket the radials by azimuth bin first so each bin's output row can be filled
    // independently. Radials stay in scan order within a bin, so overlapping radials still
    // overwrite in the same order the single serial pass used — identical output.
    // ponytail: cache-miss fill now N-cores faster, but it still runs on the UI thread; async
    // fill via task::blocking is the upgrade path if hitches persist.
    let mut by_bin: Vec<Vec<&_>> = vec![Vec::new(); AZ_BINS];
    for radial in radials {
        let Some(m) = moment.select(radial) else {
            continue;
        };
        // Skip radials with mismatched geometry (rare split-cut edge).
        if m.gate_count() as usize != gate_count {
            continue;
        }
        // A radial covers its azimuth spacing, not a point. At 720 bins a 0.5-degree super-res
        // radial lands in one bin, but a 1-degree legacy or ODIM radial spans two — filling only
        // the bin under its centre leaves every other row transparent, which draws a sweep of
        // stripes rather than a field.
        let az = radial.azimuth_angle_degrees().rem_euclid(360.0);
        // Divide by the bin width rather than scaling by 720/360: BIN_DEG is a power of two, so
        // this is exact, and a centre landing a bit-width short of its own bin was leaving gaps.
        let bin = ((az / BIN_DEG) as usize) % AZ_BINS;
        // The bins below `bin` are the rest of the beam: an azimuth is the radial's centre, so a
        // 1-degree radial reaches back half a degree into the preceding bin.
        let bins = ((radial.azimuth_spacing_degrees() / BIN_DEG).round() as usize).max(1);
        for k in 0..bins {
            by_bin[(bin + AZ_BINS - k) % AZ_BINS].push(radial);
        }
    }

    // Gather one radial's worth of raw f32 values into `row`. Below-threshold and range-folded
    // gates stay `None` — derived moments must not read them as measurements.
    let gather_row = |bin: usize, row: &mut [Option<f32>], by_bin: &Vec<Vec<&_>>| {
        for radial in &by_bin[bin] {
            let Some(m) = moment.select(radial) else {
                continue;
            };
            for (g, value) in m.iter().enumerate().take(gate_count) {
                if let MomentValue::Value(v) = value {
                    row[g] = Some(v);
                }
            }
        }
    };

    let mut data = vec![0u8; AZ_BINS * gate_count];
    if moment == Moment::SpecificDifferentialPhase {
        // KDP is the range derivative of ΦDP, so it has to be taken on the physical field:
        // the u8 band quantizes 0..360 deg into 253 steps (~1.4 deg), which is the same order
        // as the per-gate phase change being measured. Differentiating that reads mostly
        // quantization staircase.
        let mut phi = vec![None; AZ_BINS * gate_count];
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            phi.par_chunks_mut(gate_count)
                .enumerate()
                .for_each(|(bin, row)| gather_row(bin, row, &by_bin));
        }
        #[cfg(target_arch = "wasm32")]
        for (bin, row) in phi.chunks_mut(gate_count).enumerate() {
            gather_row(bin, row, &by_bin);
        }
        let kdp = crate::kdp::from_differential_phase(&phi, AZ_BINS, gate_count, gate_interval_km);
        for (i, v) in kdp.iter().enumerate() {
            if let Some(v) = v {
                data[i] = normalize(*v);
            }
        }
    } else if dealias && moment == Moment::Velocity {
        // Gather the raw velocity field (m/s) into the az×gate grid, unfold it region-based,
        // then normalize. Below-threshold/range-folded gates stay None → code 0.
        let mut vel = vec![None; AZ_BINS * gate_count];
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            vel.par_chunks_mut(gate_count)
                .enumerate()
                .for_each(|(bin, row)| gather_row(bin, row, &by_bin));
        }
        #[cfg(target_arch = "wasm32")]
        for (bin, row) in vel.chunks_mut(gate_count).enumerate() {
            gather_row(bin, row, &by_bin);
        }
        let nyq = crate::dealias::estimate_nyquist(&vel);
        // Continuity: hand the previous pass over this same tilt to the dealiaser, so a storm
        // whose fastest air genuinely sits past the Nyquist velocity stays unfolded from volume
        // to volume instead of snapping to zero whenever the fast region becomes the biggest one.
        let key = DealiasKey {
            lat_e2: (radar_lat * 100.0) as i32,
            lon_e2: (radar_lon * 100.0) as i32,
            elevation_number: sweep.elevation_number(),
            gate_count,
        };
        let at = radials
            .iter()
            .map(|r| r.collection_timestamp())
            .min()
            .unwrap_or(0);
        let reference = take_dealias_reference(&key, at);
        let unfolded = crate::dealias::dealias_with_reference(
            &vel,
            AZ_BINS,
            gate_count,
            nyq,
            reference.as_ref().map(|r| r.as_slice()),
        );
        let unfolded: DealiasField = std::sync::Arc::new(unfolded);
        put_dealias_reference(key, at, unfolded.clone());
        for (i, v) in unfolded.iter().enumerate() {
            if let Some(v) = v {
                data[i] = normalize(*v);
            }
        }
    } else {
        let fill_row = |bin: usize, row: &mut [u8]| {
            for radial in &by_bin[bin] {
                let Some(m) = moment.select(radial) else {
                    continue;
                };
                for (g, value) in m.iter().enumerate().take(gate_count) {
                    row[g] = match value {
                        MomentValue::BelowThreshold => 0,
                        MomentValue::RangeFolded => 1,
                        MomentValue::Value(v) => normalize(v),
                    };
                }
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            data.par_chunks_mut(gate_count)
                .enumerate()
                .for_each(|(bin, row)| fill_row(bin, row));
        }
        #[cfg(target_arch = "wasm32")]
        for (bin, row) in data.chunks_mut(gate_count).enumerate() {
            fill_row(bin, row);
        }
    }

    Ok(BinnedSweep {
        moment,
        az_bins: AZ_BINS,
        gate_count,
        data,
        first_gate_km,
        gate_interval_km,
        radar_lat,
        radar_lon,
        elevation_deg: sweep.elevation_angle_degrees().unwrap_or(0.0),
        value_min,
        value_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexrad_model::data::{MomentData, Radial};

    /// The bucket carries a metadata-message sidecar next to the volumes. It parses as an
    /// identifier and sorts in among them, so nothing downstream notices it is not a volume.
    /// A sweep sampled at a point built from a known azimuth and range must hand back that
    /// azimuth, that range, and the value written into that gate. This is the whole inspector:
    /// if the inverse mapping is off, every number it shows is off with it.
    #[test]
    fn sampling_a_gate_inverts_the_binning() {
        let (az_bins, gate_count) = (720, 200);
        let mut sweep = BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins,
            gate_count,
            data: vec![0u8; az_bins * gate_count],
            first_gate_km: 0.0,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -32.0,
            value_max: 95.0,
        };
        // Write a known value into one gate: azimuth bin 180 (= 90 deg, due east), gate 100.
        let (want_az, want_gate) = (90.0_f64, 100usize);
        let code = 200u8;
        sweep.data[(want_az / 360.0 * az_bins as f64) as usize * gate_count + want_gate] = code;

        // Where is that gate on the ground? Slant range of its centre, converted back to ground.
        let slant = (want_gate as f64 + 0.5) * 0.25;
        let ground = crate::xsection::ground_from_slant_km(slant, 0.5);
        let (lon, lat) = destination(-97.0, 35.0, want_az, ground);

        let got = sweep
            .sample_at(lon, lat)
            .expect("point is inside the sweep");
        assert_eq!(got.gate, want_gate, "gate index");
        assert!(
            (got.azimuth_deg as f64 - want_az).abs() < 0.5,
            "azimuth {got:?}"
        );
        let want_value = -32.0 + (code - 2) as f32 / 253.0 * (95.0 - -32.0);
        assert!(
            (got.value.unwrap() - want_value).abs() < 0.01,
            "value {got:?}"
        );
    }

    /// The two sentinel codes are not values, and must not be reported as one.
    #[test]
    fn below_threshold_and_range_folded_are_not_values() {
        let mut sweep = BinnedSweep {
            moment: Moment::Velocity,
            az_bins: 720,
            gate_count: 10,
            data: vec![0u8; 720 * 10],
            first_gate_km: 0.0,
            gate_interval_km: 1.0,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -127.0,
            value_max: 127.0,
        };
        sweep.data[1] = 1; // range folded, due north, gate 1
        let (lon, lat) = destination(-97.0, 35.0, 0.0, 1.5);
        let got = sweep.sample_at(lon, lat).unwrap();
        assert!(got.folded && got.value.is_none());

        let (lon, lat) = destination(-97.0, 35.0, 0.0, 3.5);
        let got = sweep.sample_at(lon, lat).unwrap();
        assert!(
            !got.folded && got.value.is_none(),
            "below threshold is not folded"
        );
    }

    /// Past the last gate there is no reading — not a clamped one at the edge of the sweep.
    #[test]
    fn a_point_beyond_the_last_gate_has_no_sample() {
        let sweep = BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins: 720,
            gate_count: 10,
            data: vec![9u8; 720 * 10],
            first_gate_km: 0.0,
            gate_interval_km: 1.0,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -32.0,
            value_max: 95.0,
        };
        let (lon, lat) = destination(-97.0, 35.0, 45.0, 400.0);
        assert!(sweep.sample_at(lon, lat).is_none());
    }

    /// Walk `km` from a point along `bearing`, for placing test points at a known gate.
    fn destination(lon: f64, lat: f64, bearing_deg: f64, km: f64) -> (f64, f64) {
        let r = 6371.0088_f64;
        let (b, d) = (bearing_deg.to_radians(), km / r);
        let (p0, l0) = (lat.to_radians(), lon.to_radians());
        let p = (p0.sin() * d.cos() + p0.cos() * d.sin() * b.cos()).asin();
        let l = l0 + (b.sin() * d.sin() * p0.cos()).atan2(d.cos() - p0.sin() * p.sin());
        (l.to_degrees(), p.to_degrees())
    }

    #[test]
    fn a_metadata_sidecar_is_not_a_volume() {
        assert!(is_volume("KTLH20260819_101257_V06"));
        assert!(!is_volume("KTLH20260819_095548_V06_MDM"));
        // The newest object for a site whose feed had stopped was the sidecar, so "latest"
        // resolved to 09:55 while the last real volume was 10:12.
        let newest = ["KTLH20260819_095548_V06_MDM", "KTLH20260819_101257_V06"]
            .into_iter()
            .filter(|n| is_volume(n))
            .max();
        assert_eq!(newest, Some("KTLH20260819_101257_V06"));
    }

    /// Just after 00Z the current UTC day holds only a volume or two, which is not a loop. The
    /// missing minutes are at the end of yesterday, so they get borrowed — but only as many as
    /// are needed, and only ever in front.
    #[test]
    fn short_day_borrows_the_previous_evening() {
        let ids = |names: &[&str]| -> Vec<Identifier> {
            names
                .iter()
                .map(|n| Identifier::new(n.to_string()))
                .collect()
        };
        let older = ids(&["a1", "a2", "a3", "a4"]);
        let today = ids(&["b1", "b2"]);

        let merged = with_previous_tail(older.clone(), today.clone(), 4);
        let names: Vec<&str> = merged.iter().map(|i| i.name()).collect();
        assert_eq!(
            names,
            ["a3", "a4", "b1", "b2"],
            "newest of yesterday, then today"
        );

        // Enough frames already: yesterday contributes nothing.
        let merged = with_previous_tail(older, today, 2);
        let names: Vec<&str> = merged.iter().map(|i| i.name()).collect();
        assert_eq!(names, ["b1", "b2"]);
    }

    /// The live head serves half-written objects, and anything shorter than the 24-byte volume
    /// header used to panic inside the decoder rather than come back as an error (found by
    /// fuzzing).
    #[test]
    fn a_volume_too_short_to_be_one_is_an_error_not_a_panic() {
        assert!(decode_volume(Vec::new()).is_err());
        assert!(decode_volume(vec![0u8; 23]).is_err());
        assert!(decode_volume(b"AR2V0006.".to_vec()).is_err());
    }

    // Build a one-radial sweep with a known reflectivity ramp and confirm binning
    // places it in the right azimuth row and normalizes values as documented.
    #[test]
    fn bins_reflectivity_into_correct_azimuth_row() {
        // 3 gates: below-threshold, range-folded, then a numeric 20 dBZ value.
        // from_fixed_point stores raw u8 and decodes as (raw - offset)/scale.
        // Pick scale/offset so raw 2..? map cleanly: value = (raw - offset)/scale.
        // offset=66, scale=2 -> raw=106 => (106-66)/2 = 20 dBZ.
        let raw = vec![0u8, 1u8, 106u8];
        let moment = MomentData::from_fixed_point(3, 2125, 250, 8, 2.0, 66.0, raw);
        let radial = Radial::new(
            0,    // collection_timestamp
            90,   // azimuth_number
            90.0, // azimuth_angle_degrees -> bin 180
            0.5,  // azimuth_spacing_degrees
            nexrad_model::data::RadialStatus::ScanStart,
            1,   // elevation_number
            0.5, // elevation_angle_degrees
            Some(moment),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let sweep = Sweep::new(1, vec![radial]);
        let binned = bin_sweep(&sweep, Moment::Reflectivity, 35.33, -97.28).unwrap();

        assert_eq!(binned.az_bins, 720);
        assert_eq!(binned.gate_count, 3);
        let row = 180 * 3; // az 90deg -> bin 180
        assert_eq!(binned.data[row], 0, "below threshold -> 0");
        assert_eq!(binned.data[row + 1], 1, "range folded -> 1");
        // 20 dBZ in range [-32, 95]: t = (20+32)/127 = 0.409 -> 2 + 0.409*253 = 105
        let expected = 2 + (((20.0f32 + 32.0) / 127.0) * 253.0) as u8;
        assert_eq!(binned.data[row + 2], expected, "20 dBZ normalization");
    }

    /// A 1-degree sweep (ODIM, and legacy WSR-88D cuts) must fill all 720 bins, not every other
    /// one. Half-empty azimuth rows drew DWD volumes as a spiral of stripes.
    #[test]
    fn one_degree_radials_leave_no_empty_azimuth_rows() {
        let radials = (0..360)
            .map(|i| {
                let raw = vec![106u8];
                let data = MomentData::from_fixed_point(1, 2125, 250, 8, 2.0, 66.0, raw);
                Radial::new(
                    0,
                    i as u16,
                    (i as f32 + 0.5) * 1.0,
                    1.0,
                    nexrad_model::data::RadialStatus::ScanStart,
                    1,
                    0.5,
                    Some(data),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            })
            .collect();
        let binned =
            bin_sweep(&Sweep::new(1, radials), Moment::Reflectivity, 35.33, -97.28).unwrap();
        assert_eq!(binned.gate_count, 1);
        let empty = binned.data.iter().filter(|&&c| c == 0).count();
        assert_eq!(empty, 0, "every azimuth bin should carry the 20 dBZ value");
    }

    // A radial carrying only the given moment (others None).
    fn radial_with(moment: Moment, elevation: f32) -> Radial {
        let raw = vec![106u8];
        let data = MomentData::from_fixed_point(1, 2125, 250, 8, 2.0, 66.0, raw);
        let (refl, vel) = match moment {
            Moment::Reflectivity => (Some(data), None),
            Moment::Velocity => (None, Some(data)),
            _ => (Some(data), None),
        };
        Radial::new(
            0,
            0,
            0.0,
            0.5,
            nexrad_model::data::RadialStatus::ScanStart,
            1,
            elevation,
            refl,
            vel,
            None,
            None,
            None,
            None,
            None,
        )
    }

    fn minimal_vcp() -> nexrad_model::data::VolumeCoveragePattern {
        use nexrad_model::data::{PulseWidth, VolumeCoveragePattern};
        VolumeCoveragePattern::new(
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
        )
    }

    // Two sweeps at ~0.5deg: a reflectivity-only surveillance cut and a velocity-only
    // Doppler cut (the classic split cut). elevation_angles collapses them to one tilt,
    // and bin_scan for VEL must pick the Doppler cut, not error on the surveillance cut.
    #[test]
    fn bin_scan_picks_split_cut_sweep_carrying_moment() {
        let surveillance = Sweep::new(1, vec![radial_with(Moment::Reflectivity, 0.48)]);
        let doppler = Sweep::new(1, vec![radial_with(Moment::Velocity, 0.52)]);
        let site = nexrad_model::meta::Site::new(*b"KTLX", 35.33, -97.28, 380, 0);
        let scan = Scan::with_site(site, minimal_vcp(), vec![surveillance, doppler]);

        assert_eq!(
            elevation_angles(&scan),
            vec![0.48],
            "split cut collapses to one tilt"
        );
        assert!(
            bin_scan(&scan, Moment::Velocity, 0).is_ok(),
            "VEL found on Doppler cut"
        );
        assert!(
            bin_scan(&scan, Moment::Reflectivity, 0).is_ok(),
            "REF found on surveillance cut"
        );
    }

    /// The AWS archive reaches back to June 1991 — a decade earlier than the app used to claim.
    /// Those volumes are gzip files of legacy (pre-2008, pre-dual-pol) Type-1 messages, so this
    /// is really a test that the whole legacy path still decodes.
    #[tokio::test]
    #[ignore = "network"]
    async fn caches_a_volume_to_disk_and_reads_it_back() {
        let dir = std::env::temp_dir().join(format!("hookecho-vol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let day = chrono::NaiveDate::from_ymd_opt(2013, 5, 20).unwrap();
        let id = list_volumes("KTLX", day).await.unwrap().pop().unwrap();
        let name = id.name().to_string();

        let first = download_scan(id.clone(), Some(dir.clone())).await.unwrap();
        let path = dir.join("volumes").join(&name);
        assert!(path.exists(), "the raw volume lands on disk");

        // Second time through must decode the same scan from the file; unplug the network by
        // trusting the byte count — a re-download would have to write the same bytes anyway, so
        // the real check is that the cached path produces an identical scan.
        let second = download_scan(id, Some(dir.clone())).await.unwrap();
        assert_eq!(first.sweeps().len(), second.sweeps().len());
        assert_eq!(elevation_angles(&first), elevation_angles(&second));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn decodes_a_1990s_legacy_volume() {
        let day = chrono::NaiveDate::from_ymd_opt(1991, 6, 5).unwrap();
        let scan = download_latest_scan("KTLX", day)
            .await
            .expect("1991 volume");
        let tilts = elevation_angles(&scan);
        assert!(!tilts.is_empty(), "legacy volume has tilts");
        let have = available_moments(&scan);
        assert!(have[Moment::Reflectivity.index()], "legacy REF");
        // Dual-polarization did not exist yet; the UI hides those products via available_moments.
        assert!(
            !have[Moment::CorrelationCoefficient.index()],
            "no dual-pol in 1991"
        );
        assert!(bin_scan_opts(&scan, Moment::Reflectivity, 0, false).is_ok());
    }

    /// The continuity reference is only handed back for the same tilt of the same radar, moving
    /// forward in time, within the age limit. Every other case has to come back `None` — a
    /// reference from the wrong sweep is worse than no reference at all.
    #[test]
    fn a_dealias_reference_is_only_reused_for_the_next_pass_over_the_same_tilt() {
        let key = DealiasKey {
            lat_e2: 3_552,
            lon_e2: -9_727,
            elevation_number: 1,
            gate_count: 2,
        };
        let field = vec![Some(1.0), Some(2.0)];
        let field = std::sync::Arc::new(field);
        put_dealias_reference(key, 100_000, field.clone());

        assert_eq!(take_dealias_reference(&key, 400_000), Some(field.clone()));
        assert_eq!(take_dealias_reference(&key, 100_000), None, "same instant");
        assert_eq!(take_dealias_reference(&key, 50_000), None, "scrubbed back");
        assert_eq!(
            take_dealias_reference(&key, 100_000 + DEALIAS_REF_MAX_AGE_MS + 1),
            None,
            "too old to mean anything"
        );
        let other_tilt = DealiasKey {
            elevation_number: 2,
            ..key
        };
        assert_eq!(take_dealias_reference(&other_tilt, 400_000), None);
        let other_site = DealiasKey {
            lat_e2: 4_100,
            ..key
        };
        assert_eq!(take_dealias_reference(&other_site, 400_000), None);
    }
}
