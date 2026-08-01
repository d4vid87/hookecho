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
/// 24-hour running max of MESH (mm) — hail swaths / damage tracks.
pub const MESH_1440: &str = "CONUS/MESH_Max_1440min_00.50";
/// Instantaneous 0–2 km AGL azimuthal shear (s⁻¹).
pub const AZSHEAR: &str = "CONUS/MergedAzShear_0-2kmAGL_00.50";
/// Multi-sensor 1-hour QPE accumulation, Pass-2 gauge-corrected (mm).
pub const QPE_01H: &str = "CONUS/MultiSensor_QPE_01H_Pass2_00.00";
/// Multi-sensor 24-hour QPE accumulation, Pass-2 gauge-corrected (mm; storm-total scale).
pub const QPE_24H: &str = "CONUS/MultiSensor_QPE_24H_Pass2_00.00";
/// Surface precipitation type flag (categorical: rain/snow/hail/convective).
pub const PRECIP_TYPE: &str = "CONUS/PrecipFlag_00.00";
/// FLASH flash-flood average recurrence interval over the 30-min QPE window (years).
pub const FLASH_ARI30: &str = "CONUS/FLASH_QPE_ARI30M_00.00";

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
#[derive(Clone)]
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
        if best.is_finite() {
            best
        } else {
            0.0
        }
    }

    /// Value at `(lon, lat)` by bilinear interpolation of the four surrounding cell centres, or
    /// `None` outside the grid. Point sampling for things that walk the field continuously rather
    /// than drawing it as pixels (wind advection); [`max_within_km`](Self::max_within_km) answers a
    /// different question and answers it over a radius.
    ///
    /// NaN-aware: HRRR's scatter regrid leaves holes, so the weights of whichever corners are
    /// finite are renormalised over the corners that survive. That makes an isolated empty cell
    /// invisible and softens the domain edge by half a cell, instead of punching a hole through
    /// every sample that touches one. All four NaN → `None`, same as being off the grid.
    pub fn sample_bilinear(&self, lon: f64, lat: f64) -> Option<f32> {
        if self.nx == 0 || self.ny == 0 {
            return None;
        }
        if lon < self.lon_west || lon > self.lon_east || lat < self.lat_south || lat > self.lat_north
        {
            return None;
        }
        let dlon = (self.lon_east - self.lon_west) / self.nx as f64;
        let dlat = (self.lat_north - self.lat_south) / self.ny as f64; // rows go north→south
                                                                       // Cell *centres* sit half a cell in from the corners, as max_within_km also assumes.
        let fx = (lon - self.lon_west) / dlon - 0.5;
        let fy = (self.lat_north - lat) / dlat - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let mut acc = 0.0f32;
        let mut wsum = 0.0f32;
        for (dx, dy, w) in [
            (0, 0, (1.0 - tx) * (1.0 - ty)),
            (1, 0, tx * (1.0 - ty)),
            (0, 1, (1.0 - tx) * ty),
            (1, 1, tx * ty),
        ] {
            let (x, y) = (x0 + dx, y0 + dy);
            if x < 0 || y < 0 || x >= self.nx as isize || y >= self.ny as isize {
                continue;
            }
            let v = self.values[y as usize * self.nx + x as usize];
            if !v.is_finite() {
                continue;
            }
            acc += v * w;
            wsum += w;
        }
        (wsum > 0.0).then(|| acc / wsum)
    }

    /// Max-pool the grid down so both dimensions are `<= max_dim` (GPU texture limits). Some MRMS
    /// products (rotation tracks, AzShear) are 14000×7000 — larger than the 8192 texture cap.
    /// Max-pooling keeps the strongest signal in each block (right for shear/reflectivity).
    pub fn decimated(self, max_dim: usize) -> MrmsField {
        let factor = self.nx.max(self.ny).div_ceil(max_dim);
        if factor <= 1 {
            return self; // already fits — hand the grid straight back, no copy
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
    let gz = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let raw = gunzip(&gz)?;
    // gribberish can panic on some MRMS product packings (a slice off-by-one on rotation-track /
    // AzShear grids). Contain it so a bad product surfaces as an error, never a process abort.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&raw)))
        .unwrap_or_else(|_| anyhow::bail!("grib decode panicked for {product}"))
}

/// Newest key seen per product, so refreshes can ask S3 only for what came after it.
static LAST_SEEN: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

/// Find the newest object key (list today's UTC folder, with a yesterday fallback).
///
/// MRMS writes a file every couple of minutes, so a day folder holds hundreds of keys and the
/// first listing of a product is a few hundred kilobytes of XML — once every refresh, per layer.
/// After that first call we remember the newest key and pass it as `start-after`, which is
/// lexicographically ordered the same way the timestamps in the names are: the refresh listing
/// then contains only the handful of files written since. An empty result means nothing new, and
/// the remembered key is still the answer.
async fn latest_key(http: &reqwest::Client, product: &str) -> anyhow::Result<String> {
    let known = LAST_SEEN
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(product).cloned()));
    let today = chrono::Utc::now().date_naive();
    for day in [today, today.pred_opt().unwrap_or(today)] {
        let prefix = format!("{product}/{}/", day.format("%Y%m%d"));
        // `start-after` only helps within the folder the known key belongs to.
        let after = match &known {
            Some(k) if k.starts_with(&prefix) => format!("&start-after={k}"),
            _ => String::new(),
        };
        let url = format!("{BUCKET}/?list-type=2&prefix={prefix}&max-keys=2000{after}");
        let Ok(resp) = http.get(&url).send().await else {
            continue;
        };
        let Ok(xml) = resp.text().await else { continue };
        if let Some(key) = last_key(&xml) {
            if let Ok(mut g) = LAST_SEEN.lock() {
                g.get_or_insert_with(Default::default)
                    .insert(product.to_string(), key.clone());
            }
            return Ok(key);
        }
        // Nothing newer than what we already have.
        if !after.is_empty() {
            if let Some(k) = known {
                return Ok(k);
            }
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
    let values: Vec<f32> = dm
        .data
        .iter()
        .map(|&v| if v < -90.0 { f32::NAN } else { v as f32 })
        .collect();
    anyhow::ensure!(
        values.len() == nx * ny,
        "grid size {}x{} != {} values",
        nx,
        ny,
        values.len()
    );

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
        let xml =
            "<x><Key>a/20260717-000000.grib2.gz</Key><Key>a/20260717-000200.grib2.gz</Key></x>";
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

    /// 4×4 grid of 1° cells; each cell holds its own centre longitude, so the field is exactly
    /// linear and bilinear interpolation has an analytic answer everywhere.
    fn linear_field() -> MrmsField {
        let mut values = vec![0.0f32; 16];
        for j in 0..4 {
            for i in 0..4 {
                values[j * 4 + i] = -100.0 + i as f32 + 0.5;
            }
        }
        MrmsField {
            values,
            nx: 4,
            ny: 4,
            lon_west: -100.0,
            lon_east: -96.0,
            lat_north: 40.0,
            lat_south: 36.0,
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn sample_bilinear_is_exact_on_a_linear_field() {
        let f = linear_field();
        // On a cell centre, and halfway between two — both reproduce the underlying line.
        assert!((f.sample_bilinear(-98.5, 38.5).unwrap() - -98.5).abs() < 1e-4);
        assert!((f.sample_bilinear(-98.0, 38.0).unwrap() - -98.0).abs() < 1e-4);
        // Outside the box in either axis is None, not a clamped edge value.
        assert_eq!(f.sample_bilinear(-90.0, 38.0), None);
        assert_eq!(f.sample_bilinear(-98.0, 10.0), None);
    }

    #[test]
    fn sample_bilinear_renormalises_over_nan_corners() {
        let mut f = linear_field();
        // Midpoint of the four centres around (-98.0, 38.0): all corners weigh 0.25.
        f.values[5] = f32::NAN; // row 1, col 1 — one of the four; the other three carry the sample
        let v = f.sample_bilinear(-98.0, 38.0).unwrap();
        // Surviving corners are -97.5 (twice, at x=2) and -98.5 (once), weights 0.25 each.
        assert!((v - (-97.5 * 2.0 + -98.5) / 3.0).abs() < 1e-4, "got {v}");
        // Every corner gone → None, indistinguishable from off-grid, which is what callers want.
        for v in f.values.iter_mut() {
            *v = f32::NAN;
        }
        assert_eq!(f.sample_bilinear(-98.0, 38.0), None);
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
        let d = f.clone().decimated(2);
        assert_eq!((d.nx, d.ny), (2, 2));
        // Top-left block {0,1,4,5} → max 5; bottom-right {10,11,14,15} → max 15.
        assert_eq!(d.values[0], 5.0);
        assert_eq!(d.values[3], 15.0);
        // Corners preserved (no reprojection).
        assert_eq!(d.lon_west, -100.0);
        // Already-small grids pass straight back with no copy: same allocation, same values.
        let ptr = f.values.as_ptr();
        let passthrough = f.decimated(8192);
        assert_eq!(passthrough.nx, 4);
        assert_eq!(passthrough.values.as_ptr(), ptr);
    }
}
