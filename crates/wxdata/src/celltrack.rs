//! Storm-cell identification and tracking from Level 2 reflectivity, for sites that have no
//! Level 3 SCIT product.
//!
//! The NEXRAD network broadcasts storm-cell tables (the SCIT algorithm's output) as a Level 3
//! product, and the app draws those where they exist. TDWRs, most international radars, and any
//! site whose Level 3 feed is down have no such table — but they all have reflectivity, and the
//! useful half of a cell track is arithmetic on centroids: where the cell is now, where it was,
//! therefore where it will be.
//!
//! Deliberately simpler than SCIT: one threshold instead of seven nested ones, centroids instead
//! of vertically-integrated cell components, nearest-neighbour association instead of a cost
//! matrix. It finds the same big cells and misses the marginal ones, which is the right trade for
//! a display layer that exists so a chaser can see motion at a glance.
//!
//! ponytail: single threshold, greedy association. Multi-threshold splitting matters when cells
//! merge, and the honest fix there is decoding the real SCIT product where it exists.

use crate::level2::BinnedSweep;

/// One contiguous region of reflectivity at or above the threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blob {
    pub lon: f64,
    pub lat: f64,
    /// Strongest gate in the region (dBZ).
    pub dbz_max: f32,
    /// Rough footprint, from gate areas summed over the region.
    pub area_km2: f64,
}

/// Smallest region worth calling a cell. Below this, a handful of gates over threshold is as
/// likely to be a bright patch of a squall line's leading edge as a storm.
const MIN_AREA_KM2: f64 = 8.0;

/// Decode a binned `u8` gate index back to dBZ, or `None` for below-threshold / range-folded
/// gates. Same encoding as [`crate::rotation`] and [`crate::tds`].
fn decode(sweep: &BinnedSweep, idx: u8) -> Option<f32> {
    if idx < 2 {
        return None;
    }
    let (lo, hi) = (sweep.value_min, sweep.value_max);
    Some(lo + (idx as f32 - 2.0) / 253.0 * (hi - lo))
}

/// Great-circle destination point, placing a gate at its azimuth and range.
fn dest(lon: f64, lat: f64, bearing_deg: f64, dist_km: f64) -> (f64, f64) {
    let r = 6371.0;
    let ad = dist_km / r;
    let (br, la1, lo1) = (bearing_deg.to_radians(), lat.to_radians(), lon.to_radians());
    let la2 = (la1.sin() * ad.cos() + la1.cos() * ad.sin() * br.cos()).asin();
    let lo2 = lo1 + (br.sin() * ad.sin() * la1.cos()).atan2(ad.cos() - la1.sin() * la2.sin());
    (lo2.to_degrees(), la2.to_degrees())
}

/// Distance between two points in km.
fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dlon, dlat) = ((b.0 - a.0).to_radians(), (b.1 - a.1).to_radians());
    let h = (dlat / 2.0).sin().powi(2)
        + a.1.to_radians().cos() * b.1.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

/// Bearing from `a` to `b`, degrees clockwise from north.
fn bearing_deg(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.1.to_radians(), b.1.to_radians());
    let dlon = (b.0 - a.0).to_radians();
    let y = dlon.sin() * la2.cos();
    let x = la1.cos() * la2.sin() - la1.sin() * la2.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Find cells in one sweep: flood-fill every region at or above `min_dbz` and return its
/// reflectivity-weighted centroid.
///
/// The fill runs in polar space and wraps in azimuth, so a cell sitting due north of the radar is
/// one region rather than two. Weighting by reflectivity puts the centroid on the core rather than
/// the middle of the anvil-side gradient.
pub fn find_cells(sweep: &BinnedSweep, min_dbz: f32) -> Vec<Blob> {
    let (az_bins, gates) = (sweep.az_bins, sweep.gate_count);
    if az_bins == 0 || gates == 0 {
        return Vec::new();
    }
    let (rlon, rlat) = (sweep.radar_lon as f64, sweep.radar_lat as f64);
    let az_step = 360.0 / az_bins as f64;
    let mut seen = vec![false; az_bins * gates];
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();

    let over = |az: usize, g: usize| -> Option<f32> {
        decode(sweep, sweep.data[az * gates + g]).filter(|&v| v >= min_dbz)
    };

    for az0 in 0..az_bins {
        for g0 in 0..gates {
            if seen[az0 * gates + g0] || over(az0, g0).is_none() {
                continue;
            }
            let (mut wsum, mut wlon, mut wlat) = (0.0f64, 0.0f64, 0.0f64);
            let (mut dbz_max, mut area) = (f32::MIN, 0.0f64);
            seen[az0 * gates + g0] = true;
            stack.push((az0, g0));
            while let Some((az, g)) = stack.pop() {
                let Some(v) = over(az, g) else { continue };
                let range = sweep.first_gate_km as f64 + g as f64 * sweep.gate_interval_km as f64;
                let (lon, lat) = dest(rlon, rlat, (az as f64 + 0.5) * az_step, range);
                // Weight by dBZ above the threshold, not by raw dBZ: a logarithmic unit with an
                // arbitrary zero makes a poor weight, and the offset is what varies here.
                let w = (v - min_dbz + 1.0) as f64;
                wsum += w;
                wlon += lon * w;
                wlat += lat * w;
                dbz_max = dbz_max.max(v);
                // Gate footprint: arc length across the beam times the gate depth.
                area += range * az_step.to_radians() * sweep.gate_interval_km as f64;
                let up = (az + 1) % az_bins;
                let down = (az + az_bins - 1) % az_bins;
                let mut push = |a: usize, gg: usize, stack: &mut Vec<(usize, usize)>| {
                    if gg < gates && !seen[a * gates + gg] {
                        seen[a * gates + gg] = true;
                        stack.push((a, gg));
                    }
                };
                push(up, g, &mut stack);
                push(down, g, &mut stack);
                push(az, g + 1, &mut stack);
                if g > 0 {
                    push(az, g - 1, &mut stack);
                }
            }
            if area >= MIN_AREA_KM2 && wsum > 0.0 {
                out.push(Blob {
                    lon: wlon / wsum,
                    lat: wlat / wsum,
                    dbz_max,
                    area_km2: area,
                });
            }
        }
    }
    out
}

/// One cell followed across volumes, newest point last.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Position and observation time of each volume this cell appeared in.
    pub points: Vec<(f64, f64, chrono::DateTime<chrono::Utc>)>,
    /// Direction of travel, degrees clockwise from north (where it is heading, not where from).
    pub dir_deg: f64,
    pub speed_kt: f64,
}

impl Track {
    /// Where this cell will be `minutes` from its last observation, on its current motion.
    pub fn extrapolate(&self, minutes: f64) -> Option<(f64, f64)> {
        let last = self.points.last()?;
        let km = self.speed_kt * 1.852 / 60.0 * minutes;
        Some(dest(last.0, last.1, self.dir_deg, km))
    }
}

/// Points used to fit motion. Six volumes is 25–30 minutes, long enough to smooth centroid jitter
/// and short enough that a turning storm's fit still points where it is actually going.
const FIT_POINTS: usize = 6;

/// Cell speed a gate allows for, in km per minute — 20 km per 5 minutes, comfortably above the
/// fastest supercell but below the distance between distinct storms in a line.
const GATE_KM_PER_MIN: f64 = 4.0;

/// Extend `prev` with the cells seen at time `t`, and start tracks for the ones that match
/// nothing.
///
/// Greedy nearest-neighbour within a gate that scales with the gap between volumes, so a track
/// survives a skipped scan without letting a five-minute gap match across half a county. Tracks
/// that match nothing this round are returned unchanged — the caller drops stale ones — which is
/// what lets a cell disappear for one noisy volume and be picked up again on the next.
pub fn associate(
    prev: &[Track],
    now: &[Blob],
    t: chrono::DateTime<chrono::Utc>,
    max_km: f64,
) -> Vec<Track> {
    let mut out = prev.to_vec();
    let mut taken = vec![false; out.len()];
    for cell in now {
        let mut best: Option<(usize, f64)> = None;
        for (i, tr) in out.iter().enumerate() {
            if taken[i] {
                continue;
            }
            let Some(&(lon, lat, last_t)) = tr.points.last() else {
                continue;
            };
            let dt_min = (t - last_t).num_seconds() as f64 / 60.0;
            if dt_min <= 0.0 {
                continue; // same volume, or one arriving out of order
            }
            let gate = (dt_min * GATE_KM_PER_MIN).min(max_km);
            let d = haversine_km((lon, lat), (cell.lon, cell.lat));
            if d <= gate && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        match best {
            Some((i, _)) => {
                taken[i] = true;
                out[i].points.push((cell.lon, cell.lat, t));
                let (dir, speed) = fit_motion(&out[i].points);
                out[i].dir_deg = dir;
                out[i].speed_kt = speed;
            }
            None => out.push(Track {
                points: vec![(cell.lon, cell.lat, t)],
                dir_deg: 0.0,
                speed_kt: 0.0,
            }),
        }
    }
    out
}

/// Direction and speed over the last few points.
///
/// Endpoint-to-endpoint over the fit window rather than a per-step average: centroid jitter is
/// the dominant error here, and a straight line between the ends of the window divides that error
/// by the whole elapsed time instead of by one scan interval.
fn fit_motion(points: &[(f64, f64, chrono::DateTime<chrono::Utc>)]) -> (f64, f64) {
    let n = points.len();
    if n < 2 {
        return (0.0, 0.0);
    }
    let first = points[n.saturating_sub(FIT_POINTS)];
    let last = points[n - 1];
    let dt_min = (last.2 - first.2).num_seconds() as f64 / 60.0;
    if dt_min <= 0.0 {
        return (0.0, 0.0);
    }
    let km = haversine_km((first.0, first.1), (last.0, last.1));
    (
        bearing_deg((first.0, first.1), (last.0, last.1)),
        km / dt_min * 60.0 / 1.852,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level2::Moment;

    /// A sweep with one square-ish patch of strong echo, everything else empty.
    fn sweep_with_cell(az_center: usize, gate_center: usize) -> BinnedSweep {
        let (az_bins, gate_count) = (360, 200);
        let mut data = vec![0u8; az_bins * gate_count];
        // 45 dBZ: encoded index for a -32..95 range.
        let idx = ((45.0 + 32.0) / 127.0 * 253.0 + 2.0) as u8;
        for az in 0..az_bins {
            for g in 0..gate_count {
                let daz = ((az + az_bins - az_center) % az_bins)
                    .min((az_center + az_bins - az) % az_bins);
                let dg = g.abs_diff(gate_center);
                if daz <= 4 && dg <= 6 {
                    data[az * gate_count + g] = idx;
                }
            }
        }
        BinnedSweep {
            moment: Moment::Reflectivity,
            az_bins,
            gate_count,
            data,
            first_gate_km: 2.0,
            gate_interval_km: 0.25,
            radar_lat: 35.0,
            radar_lon: -97.0,
            elevation_deg: 0.5,
            value_min: -32.0,
            value_max: 95.0,
        }
    }

    #[test]
    fn one_patch_becomes_one_cell_at_its_centroid() {
        let sweep = sweep_with_cell(90, 120);
        let cells = find_cells(&sweep, 40.0);
        assert_eq!(cells.len(), 1, "one patch, one cell: {cells:?}");
        let c = cells[0];
        // Due east of the radar at ~32 km: same latitude, east in longitude.
        assert!((c.lat - 35.0).abs() < 0.05, "{c:?}");
        assert!(c.lon > -97.0, "{c:?}");
        assert!(c.dbz_max > 44.0 && c.dbz_max < 46.0, "{c:?}");
        assert!(c.area_km2 > MIN_AREA_KM2, "{c:?}");
    }

    #[test]
    fn a_cell_straddling_north_is_not_split_in_two() {
        let cells = find_cells(&sweep_with_cell(0, 120), 40.0);
        assert_eq!(cells.len(), 1, "azimuth wrap keeps it whole: {cells:?}");
    }

    #[test]
    fn weak_echo_is_not_a_cell() {
        assert!(find_cells(&sweep_with_cell(90, 120), 50.0).is_empty());
    }

    fn blob(lon: f64, lat: f64) -> Blob {
        Blob {
            lon,
            lat,
            dbz_max: 55.0,
            area_km2: 50.0,
        }
    }

    #[test]
    fn two_volumes_give_an_eastward_track_at_the_right_speed() {
        let t0 = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let t1 = t0 + chrono::Duration::minutes(5);
        // 4.6 km east in 5 minutes ≈ 30 kt.
        let tracks = associate(&[], &[blob(-97.0, 35.0)], t0, 20.0);
        let tracks = associate(&tracks, &[blob(-96.9494, 35.0)], t1, 20.0);
        assert_eq!(tracks.len(), 1, "the second cell extended the first track");
        let tr = &tracks[0];
        assert!((tr.dir_deg - 90.0).abs() < 1.0, "{tr:?}");
        assert!((tr.speed_kt - 30.0).abs() < 2.0, "{tr:?}");
        let (lon, lat) = tr.extrapolate(5.0).unwrap();
        assert!((lon - (-96.8988)).abs() < 0.01 && (lat - 35.0).abs() < 0.01);
    }

    #[test]
    fn a_far_cell_starts_its_own_track() {
        let t0 = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let tracks = associate(&[], &[blob(-97.0, 35.0)], t0, 20.0);
        let tracks = associate(
            &tracks,
            &[blob(-97.0, 35.0), blob(-95.0, 35.0)],
            t0 + chrono::Duration::minutes(5),
            20.0,
        );
        assert_eq!(tracks.len(), 2, "180 km away is a different storm");
    }

    #[test]
    fn jitter_does_not_swing_the_motion_estimate() {
        let t0 = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut tracks = Vec::new();
        let jitter = [0.0, 0.004, -0.003, 0.002, -0.004];
        for (i, dj) in jitter.iter().enumerate() {
            tracks = associate(
                &tracks,
                &[blob(-97.0 + 0.0506 * i as f64, 35.0 + dj)],
                t0 + chrono::Duration::minutes(5 * i as i64),
                20.0,
            );
        }
        assert_eq!(tracks.len(), 1);
        let tr = &tracks[0];
        assert!((tr.dir_deg - 90.0).abs() < 5.0, "{tr:?}");
        assert!((tr.speed_kt - 30.0).abs() < 3.0, "{tr:?}");
    }
}
