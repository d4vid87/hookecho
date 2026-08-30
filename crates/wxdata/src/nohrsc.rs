//! NOHRSC snowfall analysis — how much snow actually fell, over the last 6/24/48/72 hours.
//!
//! This is the observed counterpart to the HRRR snowfall forecast: the National Operational
//! Hydrologic Remote Sensing Center's gridded analysis, reissued four times a day at 00/06/12/18Z.
//!
//! Files live at `nohrsc.noaa.gov/snowfall_v2/data/{YYYYMM}/sfav2_CONUS_{N}h_{YYYYMMDDHH}_grid184.grb2`
//! and are plain single-message GRIB2, so they decode through the same reader the MRMS grids use.
//! There is no "latest" symlink, so the fetch walks back through recent issue times until one
//! answers — in the warm season that can be the whole retention window, and no snow analysis is
//! the correct answer.

use crate::mrms::MrmsField;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use gribberish::message::read_message;

const BASE: &str = "https://www.nohrsc.noaa.gov/snowfall_v2/data";

/// Accumulation windows the analysis is published for.
pub const DURATIONS: [u16; 4] = [6, 24, 48, 72];

/// Candidate URLs for the `hours`-hour analysis, newest issue first.
///
/// Issues land at 00/06/12/18Z; `now` is rounded down to the last one and walked backwards.
fn candidate_urls(hours: u16, now: DateTime<Utc>, back: usize) -> Vec<String> {
    let mut t = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    t -= Duration::hours(i64::from(t.hour() % 6));
    (0..back)
        .map(|i| {
            let issue = t - Duration::hours(6 * i as i64);
            format!(
                "{BASE}/{:04}{:02}/sfav2_CONUS_{hours}h_{}_grid184.grb2",
                issue.year(),
                issue.month(),
                issue.format("%Y%m%d%H")
            )
        })
        .collect()
}

/// Fetch the newest available `hours`-hour snowfall analysis.
pub async fn fetch(http: &reqwest::Client, hours: u16) -> anyhow::Result<MrmsField> {
    // Eight issues is two days: enough to ride out a late posting, short enough that an
    // out-of-season request gives up quickly instead of crawling the archive.
    for url in candidate_urls(hours, Utc::now(), 8) {
        let Ok(resp) = http.get(crate::net::fetch_url(&url)).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        let Ok(raw) = resp.bytes().await else {
            continue;
        };
        // Same containment as the MRMS decode: a bad packing must not abort the process.
        let decoded = crate::task::guarded(|| decode(&raw));
        match decoded {
            Ok(Ok(f)) => return Ok(f),
            Ok(Err(e)) => log::warn!("nohrsc decode {url}: {e}"),
            Err(_) => log::warn!("nohrsc decode panicked for {url}"),
        }
    }
    anyhow::bail!("no {hours}h snowfall analysis in the last two days")
}

/// Lambert conformal conic grid parameters, read from GRIB2 grid definition template 3.30.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lambert {
    nx: usize,
    ny: usize,
    /// First grid point (degrees).
    lat1: f64,
    lon1: f64,
    /// Standard parallel and central meridian (degrees).
    lat_d: f64,
    lon_v: f64,
    /// Grid spacing, metres.
    dx: f64,
    dy: f64,
}

/// Earth radius the NOHRSC grid is defined on (spherical, from the file's own proj string).
const R_M: f64 = 6_371_229.0;

impl Lambert {
    /// Forward projection: `(lon, lat)` in degrees → grid indices `(x, y)`, fractional.
    ///
    /// Spherical LCC with both standard parallels equal, which is what this grid uses — the
    /// two-parallel cone constant collapses to `sin(lat_d)`.
    fn xy(&self, lon: f64, lat: f64) -> (f64, f64) {
        let n = self.lat_d.to_radians().sin();
        let t = |p: f64| (std::f64::consts::FRAC_PI_4 + p.to_radians() / 2.0).tan();
        let f = self.lat_d.to_radians().cos() * t(self.lat_d).powf(n) / n;
        let rho = |p: f64| R_M * f / t(p).powf(n);
        let dl = (lon - self.lon_v + 540.0).rem_euclid(360.0) - 180.0;
        let theta = n * dl.to_radians();
        let (r, r0) = (rho(lat), rho(self.lat_d));
        let (mx, my) = (r * theta.sin(), r0 - r * theta.cos());
        // Grid origin is the first point; +x is east and +y is north along the projection axes.
        let (ox, oy) = {
            let dl0 = (self.lon1 - self.lon_v + 540.0).rem_euclid(360.0) - 180.0;
            let th0 = n * dl0.to_radians();
            let r1 = rho(self.lat1);
            (r1 * th0.sin(), r0 - r1 * th0.cos())
        };
        ((mx - ox) / self.dx, (my - oy) / self.dy)
    }
}

/// Read grid definition template 3.30 (Lambert conformal) out of a GRIB2 message.
fn lambert_grid(raw: &[u8]) -> anyhow::Result<Lambert> {
    let be32 = |o: usize| -> anyhow::Result<u32> {
        raw.get(o..o + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| anyhow::anyhow!("grib truncated at {o}"))
    };
    // Signed lat/lon fields use a sign bit rather than two's complement.
    let signed = |v: u32| -> f64 {
        let m = (v & 0x7fff_ffff) as f64 / 1e6;
        if v & 0x8000_0000 != 0 {
            -m
        } else {
            m
        }
    };
    // Sections start after the 16-byte indicator section.
    let mut p = 16usize;
    while p + 5 <= raw.len() {
        let len = be32(p)? as usize;
        if len == 0 {
            break;
        }
        if raw[p + 4] == 3 {
            anyhow::ensure!(
                u16::from_be_bytes([raw[p + 12], raw[p + 13]]) == 30,
                "not a Lambert conformal grid"
            );
            return Ok(Lambert {
                nx: be32(p + 30)? as usize,
                ny: be32(p + 34)? as usize,
                lat1: signed(be32(p + 38)?),
                lon1: signed(be32(p + 42)?),
                lat_d: signed(be32(p + 47)?),
                lon_v: signed(be32(p + 51)?),
                dx: be32(p + 55)? as f64 / 1000.0,
                dy: be32(p + 59)? as f64 / 1000.0,
            });
        }
        p += len;
    }
    anyhow::bail!("no grid definition section")
}

/// Decode a NOHRSC snowfall GRIB2 and resample it onto a plate-carrée grid.
///
/// The analysis is published on a Lambert conformal grid, which the app's field pipeline (and
/// gribberish's own projector) can't take, so this projects the output lat/lon cells forward into
/// grid space and samples nearest — cheap, and the grid is finer than the display anyway.
pub(crate) fn decode(raw: &[u8]) -> anyhow::Result<MrmsField> {
    let grid = lambert_grid(raw)?;
    let msg = read_message(raw, 0).ok_or_else(|| anyhow::anyhow!("no GRIB2 message"))?;
    let time = msg.forecast_date().unwrap_or_else(|_| Utc::now());
    let data = msg
        .data()
        .map_err(|e| anyhow::anyhow!("grib decode: {e:?}"))?;
    anyhow::ensure!(
        data.len() == grid.nx * grid.ny,
        "grid {}x{} != {} values",
        grid.nx,
        grid.ny,
        data.len()
    );

    // CONUS at ~2.5 km, which is about the native spacing of the analysis.
    const RES_DEG: f64 = 0.025;
    let (lon_west, lon_east, lat_south, lat_north) = (-126.0, -66.0, 23.0, 51.0);
    let nx = ((lon_east - lon_west) / RES_DEG) as usize;
    let ny = ((lat_north - lat_south) / RES_DEG) as usize;
    let mut values = vec![f32::NAN; nx * ny];
    for gy in 0..ny {
        let lat = lat_north - (gy as f64 + 0.5) * RES_DEG;
        for gx in 0..nx {
            let lon = lon_west + (gx as f64 + 0.5) * RES_DEG;
            let (x, y) = grid.xy(lon, lat);
            let (xi, yi) = (x.round(), y.round());
            if xi < 0.0 || yi < 0.0 || xi >= grid.nx as f64 || yi >= grid.ny as f64 {
                continue;
            }
            let v = data[yi as usize * grid.nx + xi as usize];
            if v.is_finite() && v >= 0.0 {
                values[gy * nx + gx] = v as f32;
            }
        }
    }

    Ok(MrmsField {
        values,
        nx,
        ny,
        lon_west,
        lon_east,
        lat_north,
        lat_south,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_walk_back_through_the_issue_times() {
        let now = "2026-02-15T14:37:00Z".parse::<DateTime<Utc>>().unwrap();
        let urls = candidate_urls(24, now, 3);
        assert_eq!(
            urls[0],
            format!("{BASE}/202602/sfav2_CONUS_24h_2026021512_grid184.grb2"),
            "14:37Z rounds down to the 12Z issue"
        );
        assert!(urls[1].ends_with("sfav2_CONUS_24h_2026021506_grid184.grb2"));
        assert!(urls[2].ends_with("sfav2_CONUS_24h_2026021500_grid184.grb2"));
    }

    #[test]
    fn walking_back_rolls_the_month_folder() {
        let now = "2026-03-01T02:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let urls = candidate_urls(72, now, 2);
        assert!(urls[0].contains("/202603/sfav2_CONUS_72h_2026030100_"));
        assert!(
            urls[1].contains("/202602/sfav2_CONUS_72h_2026022818_"),
            "{}",
            urls[1]
        );
    }

    /// The projection has to put cities where they belong, or the whole grid is silently shifted.
    #[test]
    fn lambert_projection_places_cities_on_the_grid() {
        // The parameters the real CONUS files carry.
        let g = Lambert {
            nx: 2145,
            ny: 1377,
            lat1: 20.191_999,
            lon1: 238.446,
            lat_d: 25.0,
            lon_v: 265.0,
            dx: 2539.703,
            dy: 2539.703,
        };
        let (x, y) = g.xy(g.lon1, g.lat1);
        assert!(x.abs() < 1e-6 && y.abs() < 1e-6, "origin maps to (0,0)");
        let inside = |lon, lat| {
            let (x, y) = g.xy(lon, lat);
            x > 0.0 && y > 0.0 && x < g.nx as f64 && y < g.ny as f64
        };
        assert!(inside(-104.99, 39.74), "Denver");
        assert!(inside(-71.06, 42.36), "Boston");
        assert!(inside(-122.33, 47.60), "Seattle");
        // Seattle is north-west of Miami: smaller x, larger y.
        let (sx, sy) = g.xy(-122.33, 47.60);
        let (mx, my) = g.xy(-80.19, 25.76);
        assert!(sx < mx && sy > my, "grid orientation");
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn decodes_a_real_analysis() {
        // Out of season this legitimately finds nothing; only a decode failure is a bug.
        let http = reqwest::Client::new();
        if let Ok(f) = fetch(&http, 24).await {
            assert!(f.nx > 100 && f.ny > 100, "{}x{}", f.nx, f.ny);
            assert_eq!(f.values.len(), f.nx * f.ny);
        }
    }
}
