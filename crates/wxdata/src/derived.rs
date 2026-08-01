//! Products derived locally from a Level 2 volume: VIL, VIL density, echo tops, MEHS/POSH hail.
//!
//! The NWS ships some of these as Level 3 grids (DVL, EET, and per-cell hail attributes in HI),
//! but only for the products and resolutions it chooses to distribute, and only for the current
//! volume. Computing them here from the stacked tilts means they work in any archive replay, at
//! the volume's own resolution, and with thresholds the user can move.
//!
//! Everything integrates the same vertical profile the cross-section panel samples
//! ([`crate::xsection::column_samples`]) over a 0.01° lat/lon grid, matching the grid
//! [`crate::level3::radial_to_field`] projects L3 radials onto.

use crate::level2::BinnedSweep;
use crate::mrms::MrmsField;
use crate::xsection::{column_samples, dist_bearing};
use chrono::{DateTime, Utc};

/// Grid spacing of every derived product, in degrees — same as the L3 radial projection.
const RES_DEG: f64 = 0.01;
/// Reflectivity above this contributes nothing more to VIL/SHI: it is hail, not water, and
/// letting it run free turns one bright core into an implausible column of liquid.
const Z_CAP_DBZ: f32 = 56.0;

/// Knobs for [`derive`].
pub struct DerivedOpts {
    /// Reflectivity defining the echo top. 18.5 dBZ is the NWS EET convention.
    pub etop_dbz: f32,
    /// Stamped onto the output fields — the volume's time, so a scrubbed archive frame doesn't
    /// claim to be current.
    pub time: DateTime<Utc>,
}

impl Default for DerivedOpts {
    fn default() -> Self {
        Self {
            etop_dbz: 18.5,
            time: Utc::now(),
        }
    }
}

/// The three integrated products, on one shared grid.
pub struct Derived {
    /// Vertically integrated liquid, kg/m².
    pub vil: MrmsField,
    /// VIL density, g/m³ (VIL over echo-top height).
    pub vild: MrmsField,
    /// Echo top, kft (matching the L3 EET units the legend already uses).
    pub etop: MrmsField,
}

/// The grid every derived product shares: 0.01° cells covering the volume's range disk.
struct Grid {
    nx: usize,
    ny: usize,
    lon_west: f64,
    lon_east: f64,
    lat_north: f64,
    lat_south: f64,
    lat0: f64,
    lon0: f64,
}

impl Grid {
    fn for_sweeps(sweeps: &[BinnedSweep]) -> Option<Self> {
        let s0 = sweeps.first()?;
        let (lat0, lon0) = (s0.radar_lat as f64, s0.radar_lon as f64);
        // Range disk of the widest sweep; higher tilts are usually shorter.
        let max_range_km = sweeps
            .iter()
            .map(|s| (s.first_gate_km + s.gate_interval_km * s.gate_count as f32) as f64)
            .fold(0.0f64, f64::max);
        if max_range_km <= 0.0 {
            return None;
        }
        let coslat = lat0.to_radians().cos().max(0.05);
        let dlat = max_range_km / 111.0;
        let dlon = max_range_km / (111.0 * coslat);
        Some(Self {
            nx: ((2.0 * dlon / RES_DEG).ceil() as usize).max(1),
            ny: ((2.0 * dlat / RES_DEG).ceil() as usize).max(1),
            lon_west: lon0 - dlon,
            lon_east: lon0 + dlon,
            lat_north: lat0 + dlat,
            lat_south: lat0 - dlat,
            lat0,
            lon0,
        })
    }

    /// Ground range (km) and azimuth (deg from north) of cell `(gx, gy)`.
    fn ground_az(&self, gx: usize, gy: usize) -> (f64, f64) {
        let lat = self.lat_north - (gy as f64 + 0.5) * RES_DEG;
        let lon = self.lon_west + (gx as f64 + 0.5) * RES_DEG;
        dist_bearing(self.lon0, self.lat0, lon, lat)
    }

    fn field(&self, values: Vec<f32>, time: DateTime<Utc>) -> MrmsField {
        MrmsField {
            values,
            nx: self.nx,
            ny: self.ny,
            lon_west: self.lon_west,
            lon_east: self.lon_east,
            lat_north: self.lat_north,
            lat_south: self.lat_south,
            time,
        }
    }
}

/// dBZ → linear reflectivity factor Z (mm⁶/m³), capped at [`Z_CAP_DBZ`].
fn z_linear(dbz: f32) -> f64 {
    10f64.powf(dbz.min(Z_CAP_DBZ) as f64 / 10.0)
}

/// VIL (kg/m²) of one vertical profile: Σ 3.44×10⁻⁶ · Z̄^(4/7) · Δh, over the layers between
/// consecutive beams.
///
/// ponytail: no extrapolation below the lowest beam or above the highest — the classic
/// definition, and inventing a layer under a 0.5° beam 100 km out invents most of the answer.
fn vil_of(samples: &[(f64, f32)]) -> f32 {
    let mut vil = 0.0;
    for w in samples.windows(2) {
        let (h0, v0) = w[0];
        let (h1, v1) = w[1];
        let dh_m = (h1 - h0) * 1000.0;
        if dh_m <= 0.0 {
            continue;
        }
        let z_mean = (z_linear(v0) + z_linear(v1)) / 2.0;
        vil += 3.44e-6 * z_mean.powf(4.0 / 7.0) * dh_m;
    }
    vil as f32
}

/// Echo-top height (km) at `thresh` dBZ: the highest beam at or above the threshold, extended
/// upward toward the first beam below it.
fn etop_of(samples: &[(f64, f32)], thresh: f32) -> Option<f64> {
    let top = samples.iter().rposition(|&(_, v)| v >= thresh)?;
    let (h_top, v_top) = samples[top];
    match samples.get(top + 1) {
        Some(&(h_hi, v_hi)) if v_top > v_hi => {
            let k = ((v_top - thresh) / (v_top - v_hi)) as f64;
            Some(h_top + (h_hi - h_top) * k.clamp(0.0, 1.0))
        }
        _ => Some(h_top),
    }
}

/// Compute VIL, VIL density and echo tops from every reflectivity tilt of one volume.
///
/// `sweeps` must all be the same moment (reflectivity) — pass `Volume::reflectivity_tilts()`.
/// Returns `None` when there is nothing to integrate.
pub fn derive(sweeps: &[BinnedSweep], opts: &DerivedOpts) -> Option<Derived> {
    let g = Grid::for_sweeps(sweeps)?;
    let n = g.nx * g.ny;
    let (mut vil, mut vild, mut etop) = (vec![f32::NAN; n], vec![f32::NAN; n], vec![f32::NAN; n]);
    let mut samples: Vec<(f64, f32)> = Vec::with_capacity(sweeps.len());

    for gy in 0..g.ny {
        for gx in 0..g.nx {
            let (ground_km, az) = g.ground_az(gx, gy);
            column_samples(sweeps, ground_km, az, &mut samples);
            if samples.len() < 2 {
                continue;
            }
            let i = gy * g.nx + gx;
            let v = vil_of(&samples);
            if v > 0.0 {
                vil[i] = v;
            }
            if let Some(top_km) = etop_of(&samples, opts.etop_dbz) {
                etop[i] = (top_km * 3.280_84) as f32; // kft
                if v > 0.0 && top_km > 0.1 {
                    vild[i] = v / top_km as f32; // kg/m² over km = g/m³
                }
            }
        }
    }

    Some(Derived {
        vil: g.field(vil, opts.time),
        vild: g.field(vild, opts.time),
        etop: g.field(etop, opts.time),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level2::Moment;

    /// A uniform column of `dbz` between the tilt beams, sampled straight over the radar's
    /// azimuth ring at `ground_km`.
    fn uniform_sweeps(dbz: f32, elevs: &[f32]) -> Vec<BinnedSweep> {
        elevs
            .iter()
            .map(|&e| BinnedSweep {
                moment: Moment::Reflectivity,
                az_bins: 360,
                gate_count: 200,
                data: vec![((dbz + 32.0) / 127.0 * 253.0 + 2.0) as u8; 360 * 200],
                first_gate_km: 0.0,
                gate_interval_km: 1.0,
                radar_lat: 35.0,
                radar_lon: -97.0,
                elevation_deg: e,
                value_min: -32.0,
                value_max: 95.0,
            })
            .collect()
    }

    fn profile(sweeps: &[BinnedSweep], ground_km: f64) -> Vec<(f64, f32)> {
        let mut s = Vec::new();
        column_samples(sweeps, ground_km, 0.0, &mut s);
        s
    }

    #[test]
    fn vil_matches_the_analytic_integral() {
        let sweeps = uniform_sweeps(46.0, &[0.5, 1.5, 2.4, 3.4, 4.3, 6.0, 9.9, 14.6, 19.5]);
        let s = profile(&sweeps, 30.0);
        assert!(s.len() >= 8, "all tilts reach 30 km");
        let depth_m = (s[s.len() - 1].0 - s[0].0) * 1000.0;
        // Uniform Z: the sum collapses to one slab over the sampled depth.
        let expect = 3.44e-6 * z_linear(s[0].1).powf(4.0 / 7.0) * depth_m;
        let got = vil_of(&s) as f64;
        assert!(
            (got - expect).abs() / expect < 1e-3,
            "vil {got} vs analytic {expect}"
        );
    }

    #[test]
    fn reflectivity_is_capped_at_the_hail_ceiling() {
        let hot = vec![(1.0, 70.0f32), (5.0, 70.0)];
        let capped = vec![(1.0, Z_CAP_DBZ), (5.0, Z_CAP_DBZ)];
        assert_eq!(vil_of(&hot), vil_of(&capped));
    }

    #[test]
    fn echo_top_respects_the_threshold() {
        let s = vec![(1.0, 40.0f32), (5.0, 30.0), (9.0, 10.0)];
        // 30 dBZ lands exactly on the 5 km beam.
        assert!((etop_of(&s, 30.0).unwrap() - 5.0).abs() < 1e-6);
        // 20 dBZ interpolates between the 5 km and 9 km beams.
        let t = etop_of(&s, 20.0).unwrap();
        assert!(t > 5.0 && t < 9.0, "interpolated top {t}");
        // Nothing in the column reaches 60 dBZ.
        assert!(etop_of(&s, 60.0).is_none());
    }

    #[test]
    fn derive_fills_the_grid_and_leaves_gaps_nan() {
        let sweeps = uniform_sweeps(45.0, &[0.5, 2.4, 6.0]);
        let d = derive(&sweeps, &DerivedOpts::default()).unwrap();
        assert_eq!(d.vil.nx * d.vil.ny, d.etop.values.len());
        let sample = |f: &MrmsField, lon: f64, lat: f64| {
            let gx = (((lon - f.lon_west) / RES_DEG) as usize).min(f.nx - 1);
            let gy = (((f.lat_north - lat) / RES_DEG) as usize).min(f.ny - 1);
            f.values[gy * f.nx + gx]
        };
        // 0.3° north of the radar (~33 km) is inside every sweep.
        assert!(sample(&d.vil, -97.0, 35.3) > 0.0);
        assert!(sample(&d.etop, -97.0, 35.3) > 0.0);
        assert!(sample(&d.vild, -97.0, 35.3) > 0.0);
        // The grid corner is past the 200 km range disk.
        assert!(sample(&d.vil, d.lon_west_corner(), d.lat_north_corner()).is_nan());
    }

    impl Derived {
        fn lon_west_corner(&self) -> f64 {
            self.vil.lon_west + 0.005
        }
        fn lat_north_corner(&self) -> f64 {
            self.vil.lat_north - 0.005
        }
    }
}
