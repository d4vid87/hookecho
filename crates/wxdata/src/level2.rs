//! Level 2 acquisition and sweep binning.
//!
//! Pipeline: AWS archive listing -> download an Archive II volume -> decode to a
//! [`Scan`] -> bin a chosen moment of one sweep into a dense polar grid the GPU can
//! sample. The binned grid is deliberately simple (fixed azimuth bins, `u8` values)
//! so the render side is a single texture upload.

use nexrad_model::data::{DataMoment, MomentData, MomentValue, Sweep};

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
    CorrelationCoefficient,
}

impl Moment {
    /// All moments, in toolbox/hotkey display order (REF..RHO).
    pub const ALL: [Moment; 6] = [
        Moment::Reflectivity,
        Moment::Velocity,
        Moment::SpectrumWidth,
        Moment::DifferentialReflectivity,
        Moment::DifferentialPhase,
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
            Moment::DifferentialPhase => radial.differential_phase(),
            Moment::CorrelationCoefficient => radial.correlation_coefficient(),
        }
    }

    /// Physical value range used to normalize gate values into the 2..=255 `u8` band.
    /// 0 = below-threshold (transparent), 1 = range-folded.
    pub fn value_range(&self) -> (f32, f32) {
        match self {
            Moment::Reflectivity => (-32.0, 95.0),           // dBZ
            Moment::Velocity => (-127.0, 127.0),             // m/s (pre-dealias)
            Moment::SpectrumWidth => (0.0, 63.0),            // m/s
            Moment::DifferentialReflectivity => (-7.9, 7.9), // dB
            Moment::DifferentialPhase => (0.0, 360.0),       // deg
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
    use nexrad_data::aws::archive;

    let today = chrono::Utc::now().date_naive();
    let mut out: Vec<Identifier> = Vec::new();
    for day in [today, today.pred_opt().unwrap_or(today)] {
        let mut ids = archive::list_files(site, &day)
            .await
            .map_err(|e| anyhow::anyhow!("list_files({site}, {day}): {e}"))?;
        ids.sort_by_key(|id| id.date_time());
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

/// List every volume for `site` on a specific UTC `date`, oldest first.
pub async fn list_volumes(site: &str, date: chrono::NaiveDate) -> anyhow::Result<Vec<Identifier>> {
    use nexrad_data::aws::archive;
    let mut ids = archive::list_files(site, &date)
        .await
        .map_err(|e| anyhow::anyhow!("list_files({site}, {date}): {e}"))?;
    ids.sort_by_key(|id| id.date_time());
    Ok(ids)
}

/// Download and decode a specific volume to a [`Scan`].
pub async fn download_scan(id: Identifier) -> anyhow::Result<Scan> {
    use nexrad_data::aws::archive;
    let name = id.name().to_string();
    let file = archive::download_file(id)
        .await
        .map_err(|e| anyhow::anyhow!("download_file: {e}"))?;
    // bzip2 decompression plus the message decode is tens of MB of pure CPU. On the async worker
    // it blocks every other fetch sharing that thread for as long as it runs; the `parallel`
    // feature of nexrad-data then spreads the decode itself across rayon.
    let scan = tokio::task::spawn_blocking(move || {
        let file = if file.compressed() {
            file.decompress()
                .map_err(|e| anyhow::anyhow!("decompress: {e}"))?
        } else {
            file
        };
        file.scan().map_err(|e| anyhow::anyhow!("scan: {e}"))
    })
    .await??;
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
    download_scan(latest).await
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
pub fn available_moments(scan: &Scan) -> [bool; 6] {
    let mut got = [false; 6];
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
    // legacy 1-degree radials just fill two adjacent bins.
    const AZ_BINS: usize = 720;

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

    let mut data = vec![0u8; AZ_BINS * gate_count];
    if dealias && moment == Moment::Velocity {
        // Gather the raw velocity field (m/s) into the az×gate grid, unfold it region-based,
        // then normalize. Below-threshold/range-folded gates stay None → code 0.
        let mut vel = vec![None; AZ_BINS * gate_count];
        for radial in radials {
            let Some(m) = moment.select(radial) else {
                continue;
            };
            if m.gate_count() as usize != gate_count {
                continue;
            }
            let az = radial.azimuth_angle_degrees().rem_euclid(360.0);
            let bin = ((az / 360.0 * AZ_BINS as f32) as usize) % AZ_BINS;
            for (g, value) in m.iter().enumerate().take(gate_count) {
                if let MomentValue::Value(v) = value {
                    vel[bin * gate_count + g] = Some(v);
                }
            }
        }
        let nyq = crate::dealias::estimate_nyquist(&vel);
        let unfolded = crate::dealias::dealias(&vel, AZ_BINS, gate_count, nyq);
        for (i, v) in unfolded.iter().enumerate() {
            if let Some(v) = v {
                data[i] = normalize(*v);
            }
        }
    } else {
        for radial in radials {
            let Some(m) = moment.select(radial) else {
                continue;
            };
            if m.gate_count() as usize != gate_count {
                continue; // skip radials with mismatched geometry (rare split-cut edge)
            }
            let az = radial.azimuth_angle_degrees().rem_euclid(360.0);
            let bin = ((az / 360.0 * AZ_BINS as f32) as usize) % AZ_BINS;
            let row = &mut data[bin * gate_count..(bin + 1) * gate_count];
            for (g, value) in m.iter().enumerate().take(gate_count) {
                row[g] = match value {
                    MomentValue::BelowThreshold => 0,
                    MomentValue::RangeFolded => 1,
                    MomentValue::Value(v) => normalize(v),
                };
            }
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
}
