//! MRMS (Multi-Radar Multi-Sensor) national reflectivity mosaic from the NOAA AWS PDS bucket.
//!
//! Fetches the latest `MergedReflectivityQCComposite` GRIB2 (gzipped), decodes it with
//! `gribberish`, and returns a plate-carrée dBZ grid + its lat/lon bounds. The renderer warps
//! the regular lat/lon grid onto web-mercator in a shader.

use gribberish::data_message::DataMessage;
use gribberish::message::read_message;

const BUCKET: &str = "https://noaa-mrms-pds.s3.amazonaws.com";

/// National composite reflectivity mosaic (dBZ).
pub const REFLECTIVITY: &str = "CONUS/MergedReflectivityQCComposite_00.50";
/// Cloud-to-ground lightning strike density, 5-minute average (strikes/km²/min).
pub const LIGHTNING: &str = "CONUS/NLDN_CG_005min_AvgDensity_00.00";
/// Max Estimated Size of Hail (mm).
pub const MESH: &str = "CONUS/MESH_00.50";
/// Instantaneous 0–2 km AGL azimuthal shear (s⁻¹).
pub const AZSHEAR: &str = "CONUS/MergedAzShear_0-2kmAGL_00.50";
/// Multi-sensor 1-hour QPE accumulation, Pass-2 gauge-corrected (mm).
pub const QPE_01H: &str = "CONUS/MultiSensor_QPE_01H_Pass2_00.00";
/// Multi-sensor 24-hour QPE accumulation, Pass-2 gauge-corrected (mm; storm-total scale).
pub const QPE_24H: &str = "CONUS/MultiSensor_QPE_24H_Pass2_00.00";
// FLASH flood products (ARI/FFG/streamflow) use a GRIB2 local parameter table the vendored
// gribberish decoder rejects — omitted until that decode path is fixed. QPE accumulation covers
// the flood-suite need for now.

/// Low-level rotation-track (accumulated azimuthal-shear max) product path for `minutes`
/// (30/60/120 supported; other values fall back to 30).
pub fn rotation_track(minutes: u16) -> &'static str {
    match minutes {
        60 => "CONUS/RotationTrack60min_00.50",
        120 => "CONUS/RotationTrack120min_00.50",
        _ => "CONUS/RotationTrack30min_00.50",
    }
}

/// A decoded MRMS reflectivity field: a regular lat/lon grid of dBZ (`NaN` = no data).
pub struct MrmsField {
    /// Row-major `ny × nx` dBZ values; row 0 is the northernmost latitude.
    pub values: Vec<f32>,
    pub nx: usize,
    pub ny: usize,
    /// Grid corner longitudes/latitudes (degrees, lon in −180..180).
    pub lon_west: f64,
    pub lon_east: f64,
    pub lat_north: f64,
    pub lat_south: f64,
    pub time: chrono::DateTime<chrono::Utc>,
}

/// Great-circle distance in km between two lat/lon points (haversine).
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let (dp, dl) = ((lat2 - lat1).to_radians(), (lon2 - lon1).to_radians());
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().asin()
}

impl MrmsField {
    /// Largest non-NaN grid value within `radius_km` of `(lon, lat)`, or 0.0 if none. Scans a
    /// lat/lon window sized to the radius and haversine-filters. Used for point proximity checks
    /// (e.g. lightning density near a saved location) against a density/intensity grid.
    pub fn max_within_km(&self, lon: f64, lat: f64, radius_km: f64) -> f32 {
        if self.nx == 0 || self.ny == 0 {
            return 0.0;
        }
        let dlon = (self.lon_east - self.lon_west) / self.nx as f64;
        let dlat = (self.lat_north - self.lat_south) / self.ny as f64; // rows go north→south
        // Degrees covering the radius (latitude ~111 km/deg; widen longitude by 1/cos lat).
        let dlat_deg = radius_km / 111.0;
        let dlon_deg = radius_km / (111.0 * lat.to_radians().cos().abs().max(0.05));
        let cx = ((lon - self.lon_west) / dlon).round() as isize;
        let cy = ((self.lat_north - lat) / dlat).round() as isize;
        let wx = (dlon_deg / dlon.abs()).ceil() as isize + 1;
        let wy = (dlat_deg / dlat.abs()).ceil() as isize + 1;
        let mut best = f32::NEG_INFINITY;
        for iy in (cy - wy).max(0)..=(cy + wy).min(self.ny as isize - 1) {
            for ix in (cx - wx).max(0)..=(cx + wx).min(self.nx as isize - 1) {
                let v = self.values[iy as usize * self.nx + ix as usize];
                if v.is_nan() || v <= best {
                    continue;
                }
                let clon = self.lon_west + (ix as f64 + 0.5) * dlon;
                let clat = self.lat_north - (iy as f64 + 0.5) * dlat;
                if haversine_km(lat, lon, clat, clon) <= radius_km {
                    best = v;
                }
            }
        }
        if best.is_finite() { best } else { 0.0 }
    }

    /// Max-pool the grid down so both dimensions are `<= max_dim` (GPU texture limits). Some MRMS
    /// products (rotation tracks, AzShear) are 14000×7000 — larger than the 8192 texture cap.
    /// Max-pooling keeps the strongest signal in each block (right for shear/reflectivity).
    pub fn decimated(&self, max_dim: usize) -> MrmsField {
        let factor = (self.nx.max(self.ny) + max_dim - 1) / max_dim;
        if factor <= 1 {
            return MrmsField {
                values: self.values.clone(),
                nx: self.nx,
                ny: self.ny,
                lon_west: self.lon_west,
                lon_east: self.lon_east,
                lat_north: self.lat_north,
                lat_south: self.lat_south,
                time: self.time,
            };
        }
        let nx = self.nx.div_ceil(factor);
        let ny = self.ny.div_ceil(factor);
        let mut values = vec![f32::NAN; nx * ny];
        for oy in 0..ny {
            for ox in 0..nx {
                let mut best = f32::NAN;
                for dy in 0..factor {
                    let sy = oy * factor + dy;
                    if sy >= self.ny {
                        break;
                    }
                    for dx in 0..factor {
                        let sx = ox * factor + dx;
                        if sx >= self.nx {
                            break;
                        }
                        let v = self.values[sy * self.nx + sx];
                        if v.is_finite() && (best.is_nan() || v > best) {
                            best = v;
                        }
                    }
                }
                values[oy * nx + ox] = best;
            }
        }
        MrmsField {
            values,
            nx,
            ny,
            lon_west: self.lon_west,
            lon_east: self.lon_east,
            lat_north: self.lat_north,
            lat_south: self.lat_south,
            time: self.time,
        }
    }
}

/// Fetch + decode the latest CONUS mosaic for `product` (see [`REFLECTIVITY`], [`LIGHTNING`]).
pub async fn fetch_latest(http: &reqwest::Client, product: &str) -> anyhow::Result<MrmsField> {
    let key = latest_key(http, product).await?;
    let url = format!("{BUCKET}/{key}");
    let gz = http.get(&url).send().await?.error_for_status()?.bytes().await?;
    let raw = gunzip(&gz)?;
    // gribberish can panic on some MRMS product packings (a slice off-by-one on rotation-track /
    // AzShear grids). Contain it so a bad product surfaces as an error, never a process abort.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&raw)))
        .unwrap_or_else(|_| anyhow::bail!("grib decode panicked for {product}"))
}

/// Find the newest object key (list today's UTC folder, with a yesterday fallback).
async fn latest_key(http: &reqwest::Client, product: &str) -> anyhow::Result<String> {
    let today = chrono::Utc::now().date_naive();
    for day in [today, today.pred_opt().unwrap_or(today)] {
        let prefix = format!("{product}/{}/", day.format("%Y%m%d"));
        let url = format!("{BUCKET}/?list-type=2&prefix={prefix}&max-keys=2000");
        let Ok(resp) = http.get(&url).send().await else { continue };
        let Ok(xml) = resp.text().await else { continue };
        if let Some(key) = last_key(&xml) {
            return Ok(key);
        }
    }
    anyhow::bail!("no MRMS objects found for today or yesterday")
}

fn gunzip(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// Decode a single-message MRMS GRIB2 into a [`MrmsField`].
fn decode(raw: &[u8]) -> anyhow::Result<MrmsField> {
    let msg = read_message(raw, 0).ok_or_else(|| anyhow::anyhow!("no GRIB2 message"))?;
    let time = msg.forecast_date().unwrap_or_else(|_| chrono::Utc::now());
    let dm = DataMessage::try_from(&msg).map_err(|e| anyhow::anyhow!("grib decode: {e:?}"))?;
    let (ny, nx) = dm.metadata.grid_shape;
    let (lat0, lon0) = dm.metadata.projector.latlng_start();
    let (lat1, lon1) = dm.metadata.projector.latlng_end();

    // MRMS encodes missing as -999 and no-coverage as -99; treat anything very negative as NaN.
    let values: Vec<f32> = dm.data.iter().map(|&v| if v < -90.0 { f32::NAN } else { v as f32 }).collect();
    anyhow::ensure!(values.len() == nx * ny, "grid size {}x{} != {} values", nx, ny, values.len());

    Ok(MrmsField {
        values,
        nx,
        ny,
        lon_west: wrap_lon(lon0.min(lon1)),
        lon_east: wrap_lon(lon0.max(lon1)),
        lat_north: lat0.max(lat1),
        lat_south: lat0.min(lat1),
        time,
    })
}

/// Wrap a 0..360 longitude into −180..180.
fn wrap_lon(lon: f64) -> f64 {
    if lon > 180.0 {
        lon - 360.0
    } else {
        lon
    }
}

/// The last `<Key>` in an S3 list-objects-v2 XML response (ascending sort → newest last).
fn last_key(xml: &str) -> Option<String> {
    xml.rmatch_indices("<Key>").next().and_then(|(i, _)| {
        let rest = &xml[i + 5..];
        rest.find("</Key>").map(|e| rest[..e].to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_and_last_key() {
        assert!((wrap_lon(230.005) - -129.995).abs() < 1e-6);
        assert!((wrap_lon(-60.0) - -60.0).abs() < 1e-6);
        let xml = "<x><Key>a/20260717-000000.grib2.gz</Key><Key>a/20260717-000200.grib2.gz</Key></x>";
        assert_eq!(last_key(xml).unwrap(), "a/20260717-000200.grib2.gz");
    }

    #[test]
    fn max_within_km_haversine_filters() {
        // 3×3 grid over a 2°×2° box centered near 35N; ~0.67° cells (~74 km lat).
        // Values: center cell high, a far corner higher — the corner is outside 30 km.
        let mut vals = vec![0.0f32; 9];
        vals[4] = 5.0; // center cell
        vals[0] = 9.0; // NW corner (far)
        let f = MrmsField {
            values: vals,
            nx: 3,
            ny: 3,
            lon_west: -98.0,
            lon_east: -96.0,
            lat_north: 36.0,
            lat_south: 34.0,
            time: chrono::Utc::now(),
        };
        // Query the center: only the center cell is within 30 km → 5.0, not the far 9.0 corner.
        assert_eq!(f.max_within_km(-97.0, 35.0, 30.0), 5.0);
        // A wide radius reaches the 9.0 corner.
        assert_eq!(f.max_within_km(-97.0, 35.0, 500.0), 9.0);
        // A point far from the grid sees nothing.
        assert_eq!(f.max_within_km(-80.0, 40.0, 20.0), 0.0);
    }

    #[test]
    fn decimate_maxpools_to_fit() {
        // 4×4 grid, cap 2 → factor 2 → 2×2 grid, each cell = max of its 2×2 block.
        let f = MrmsField {
            values: (0..16).map(|i| i as f32).collect(),
            nx: 4,
            ny: 4,
            lon_west: -100.0,
            lon_east: -96.0,
            lat_north: 40.0,
            lat_south: 36.0,
            time: chrono::Utc::now(),
        };
        let d = f.decimated(2);
        assert_eq!((d.nx, d.ny), (2, 2));
        // Top-left block {0,1,4,5} → max 5; bottom-right {10,11,14,15} → max 15.
        assert_eq!(d.values[0], 5.0);
        assert_eq!(d.values[3], 15.0);
        // Corners preserved (no reprojection).
        assert_eq!(d.lon_west, -100.0);
        // Already-small grids pass through unchanged.
        assert_eq!(f.decimated(8192).nx, 4);
    }
}
