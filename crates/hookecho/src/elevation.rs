//! Offline terrain elevation (DEM) and the beam-vs-terrain blockage raster.
//!
//! Heights come from the AWS Open Data *terrarium* tiles — free, keyless PNGs where the height in
//! metres is packed as `R*256 + G + B/256 - 32768`. They ride the same read-through disk cache the
//! basemap uses ([`crate::tiles::load_tile_bytes`]), so a chase pack that pre-downloaded them
//! works with the radio off.
//!
//! The product on top is the blockage raster: for each cell of a world-space grid, how far the
//! radar beam clears the ground under it. Below the ground means the radar cannot see there.

use crate::render::mercator::world_to_lonlat;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};

/// Default DEM zoom. z10 is ~150 m/px at mid-latitudes — finer than the beam is wide, and coarse
/// enough that a chase-pack bbox is a few dozen tiles.
pub const DEM_ZOOM: u8 = 10;

/// z12 is ~40 m/px: sixteen times the tiles for terrain you can read a draw in. Offered for chase
/// packs, where the download is deliberate and the detail is the point.
pub const DEM_ZOOM_HIRES: u8 = 12;

/// The zoom this session actually fetches and samples. Set once from settings at startup.
static ZOOM: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(DEM_ZOOM);

/// The DEM zoom in force.
pub fn zoom() -> u8 {
    ZOOM.load(std::sync::atomic::Ordering::Relaxed)
}

/// Choose the DEM resolution. Clears the decoded-tile cache, whose keys are zoom-specific.
pub fn set_hires(on: bool) {
    let z = if on { DEM_ZOOM_HIRES } else { DEM_ZOOM };
    if ZOOM.swap(z, std::sync::atomic::Ordering::Relaxed) != z {
        if let Ok(mut c) = cache().lock() {
            c.clear();
        }
    }
}

/// Cache subfolder (and chase-pack folder) for DEM tiles, alongside the basemap providers'.
pub const DEM_PROVIDER: &str = "terrarium";

/// Decoded tiles kept in memory. Each is 256 KB of `f32`. `None` = the tile failed
/// (negative-cached, or a raster loop re-requests a missing tile 65 000 times).
type DemCache = LruCache<(u32, u32), Option<Arc<Vec<f32>>>>;

/// Sized by how [`occultation`] scans, not by how much memory is spare: it sweeps radial by
/// radial, so the working set is one radial's tiles (~10 at 250 km) plus the overlap with the
/// neighbouring azimuths. 64 holds that comfortably at ~16 MB.
/// ponytail: a 250 km disk is ~230 z10 tiles, so a pan re-decodes rather than re-reading memory —
/// bump this (and pack the samples as `i16`) if the rebuild ever feels slow on a phone.
const CACHE_TILES: usize = 64;

fn cache() -> &'static Mutex<DemCache> {
    static C: OnceLock<Mutex<DemCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(LruCache::new(NonZeroUsize::new(CACHE_TILES).unwrap())))
}

/// Terrarium height (metres) from one RGB pixel.
pub fn decode_height(rgb: [u8; 3]) -> f32 {
    rgb[0] as f32 * 256.0 + rgb[1] as f32 + rgb[2] as f32 / 256.0 - 32768.0
}

pub fn tile_url(x: u32, y: u32) -> String {
    let z = zoom();
    format!("https://s3.amazonaws.com/elevation-tiles-prod/{DEM_PROVIDER}/{z}/{x}/{y}.png")
}

/// Disk path a DEM tile caches at. Mirrors the basemap layout (`<root>/<provider>/default/z/x/y`)
/// so `sweep_tile_cache` and the chase-pack downloader need no special case.
pub fn tile_path(root: &std::path::Path, x: u32, y: u32) -> std::path::PathBuf {
    root.join(DEM_PROVIDER)
        .join("default")
        .join(format!("{}/{x}/{y}", zoom()))
}

/// Slippy tile and in-tile pixel for a lon/lat at [`DEM_ZOOM`], or `None` outside the Mercator
/// latitude band.
fn tile_of(lon: f64, lat: f64) -> Option<(u32, u32, f64, f64)> {
    if !(-85.05..=85.05).contains(&lat) || !lon.is_finite() {
        return None;
    }
    let n = (1u32 << zoom()) as f64;
    let fx = (lon + 180.0) / 360.0 * n;
    let s = lat.to_radians().tan().asinh(); // ln(tan+sec), spelled the stable way
    let fy = (1.0 - s / std::f64::consts::PI) / 2.0 * n;
    if !fx.is_finite() || !fy.is_finite() || fy < 0.0 || fy >= n {
        return None;
    }
    let (tx, ty) = (fx.floor(), fy.floor());
    // Pixel coordinates kept fractional: the caller interpolates between the four neighbours.
    let px = ((fx - tx) * 256.0).clamp(0.0, 255.0);
    let py = ((fy - ty) * 256.0).clamp(0.0, 255.0);
    Some((
        (tx as i64).rem_euclid(1i64 << zoom()) as u32,
        ty as u32,
        px,
        py,
    ))
}

/// Bilinear height at fractional pixel `(px, py)` in a 256x256 Terrarium tile.
///
/// A DEM pixel at z10 is ~150 m across, and taking the nearest one quantized every beam-blockage
/// profile into terraces that were an artifact of the sampling, not the terrain.
///
/// ponytail: neighbours are clamped to this tile, so the outermost half-pixel of a tile edge is
/// nearest-neighbour. Blend across tiles only if a seam ever shows.
fn bilinear(heights: &[f32], px: f64, py: f64) -> f32 {
    let (x0, y0) = (px.floor(), py.floor());
    let (fx, fy) = ((px - x0) as f32, (py - y0) as f32);
    let (x0, y0) = (x0 as usize, y0 as usize);
    let (x1, y1) = ((x0 + 1).min(255), (y0 + 1).min(255));
    let at = |x: usize, y: usize| heights[y * 256 + x];
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Terrain height above sea level (metres) at `(lat, lon)`, or `None` if the covering DEM tile is
/// unavailable (offline and not packed, or ocean-edge).
pub async fn elevation_m(client: &reqwest::Client, lat: f64, lon: f64) -> Option<f32> {
    let (tx, ty, px, py) = tile_of(lon, lat)?;
    let sample = |t: &Option<Arc<Vec<f32>>>| t.as_ref().map(|v| bilinear(v, px, py));
    // Guard dropped before the await below — a fetch must not hold the whole cache.
    if let Some(hit) = cache()
        .lock()
        .ok()
        .and_then(|mut c| c.get(&(tx, ty)).cloned())
    {
        return sample(&hit);
    }
    let path = crate::paths::cache_dir().map(|d| tile_path(&d.join("tiles"), tx, ty));
    let decoded = match crate::tiles::load_tile_bytes(client, &tile_url(tx, ty), path.as_deref())
        .await
        .ok()
        .and_then(|b| image::load_from_memory(&b).ok())
    {
        Some(img) => {
            let rgb = img.to_rgb8();
            // Terrarium tiles are 256x256; anything else is a proxy/error page, not a DEM.
            (rgb.dimensions() == (256, 256)).then(|| {
                Arc::new(
                    rgb.pixels()
                        .map(|p| decode_height([p[0], p[1], p[2]]))
                        .collect::<Vec<f32>>(),
                )
            })
        }
        None => None,
    };
    if let Ok(mut c) = cache().lock() {
        c.put((tx, ty), decoded.clone());
    }
    sample(&decoded)
}

/// Half-power half-beamwidth of the WSR-88D (0.95° full width).
const HALF_BEAMWIDTH_DEG: f64 = 0.475;

/// Fraction of the beam (0 = clear, 1 = fully occulted) hidden when the highest terrain along the
/// ray subtends `occult_deg` and the beam is centred on `tilt_deg`.
///
/// ponytail: linear in the angle, i.e. the beam is treated as uniformly bright across its width.
/// A Gaussian power weighting is the upgrade path if the shading is ever read quantitatively.
pub fn blockage_fraction(occult_deg: f64, tilt_deg: f64) -> f32 {
    let bottom = tilt_deg - HALF_BEAMWIDTH_DEG;
    (((occult_deg - bottom) / (2.0 * HALF_BEAMWIDTH_DEG)).clamp(0.0, 1.0)) as f32
}

/// Elevation angle (degrees) from the radar to a terrain point `ground_km` away and
/// `terrain_msl_m` high, under the same 4/3-earth refraction the beam model uses. This is the
/// exact inverse of [`wxdata::xsection::beam_height_km`] for small angles: terrain whose angle
/// equals the tilt is terrain the beam centre grazes.
pub fn terrain_angle_deg(radar_msl_m: f64, terrain_msl_m: f64, ground_km: f64) -> f64 {
    if ground_km <= 0.0 {
        return -90.0;
    }
    let rise_km = (terrain_msl_m - radar_msl_m) / 1000.0;
    // Curvature drop: the earth falls away by r²/2R over the path.
    let drop_km = ground_km * ground_km / (2.0 * wxdata::xsection::R_EFF_KM);
    ((rise_km - drop_km) / ground_km).atan().to_degrees()
}

/// The radar this raster is drawn for.
#[derive(Debug, Clone, Copy)]
pub struct BeamSite {
    pub lon: f64,
    pub lat: f64,
    /// Ground elevation MSL from the site registry, in metres.
    pub ground_m: f64,
    /// Elevation angle of the tilt being displayed, in degrees.
    pub tilt_deg: f64,
}

/// Antenna height above the site's registry ground elevation. The registry carries the ground, not
/// the feedhorn, and WSR-88D towers are all in this neighbourhood.
/// ponytail: one constant for every site; per-site tower heights if a shading check needs them.
pub const TOWER_M: f64 = 20.0;

/// Beam is only useful out to about the unambiguous range; past it there is nothing to shade.
const MAX_RANGE_KM: f64 = 250.0;

/// Radials in the occultation scan (0.5°, half the beamwidth).
const N_AZ: usize = 720;
/// Range gates in the occultation scan (1 km out to [`MAX_RANGE_KM`]).
const N_RANGE: usize = MAX_RANGE_KM as usize;
/// How far out the scan steps finely (km).
const NEAR_KM: usize = 10;

/// Grid resolution of the painted raster (per side).
pub const RASTER_N: usize = 256;

/// Cumulative terrain occultation angle per (azimuth, range) around one site — the highest
/// elevation angle any terrain along that ray has reached so far. This is what makes the product a
/// *shadow* map rather than a "beam is underground here" map: a ridge keeps blocking everything
/// behind it.
async fn occultation(client: &reqwest::Client, site: BeamSite, radar_msl: f64) -> Vec<f32> {
    let mut polar = vec![f32::MIN; N_AZ * N_RANGE];
    for a in 0..N_AZ {
        let az = a as f64 * 360.0 / N_AZ as f64;
        let mut highest = f64::MIN;
        // 250 m steps through the near field, then 1 km. The biggest blocker at most sites is the
        // ridge the tower stands on, and at 1 km steps the scan walks straight over it.
        for r in (1..=4 * NEAR_KM)
            .map(|i| i as f64 * 0.25)
            .chain(((NEAR_KM + 1)..=N_RANGE).map(|r| r as f64))
        {
            let p = crate::geo::destination_point([site.lon, site.lat], az, r);
            if let Some(h) = elevation_m(client, p[1], p[0]).await {
                highest = highest.max(terrain_angle_deg(radar_msl, h as f64, r));
            }
            // Every gate 0..N_RANGE is covered: the fine steps all land in the first NEAR_KM
            // gates, the coarse ones write exactly one gate each.
            let gate = (r.ceil() as usize).saturating_sub(1).min(N_RANGE - 1);
            polar[a * N_RANGE + gate] = highest as f32;
        }
    }
    polar
}

/// Build the beam-blockage raster covering the world-space rect `[wx0, wy0, wx1, wy1]`.
///
/// The rect is Mercator world space, which maps linearly to screen, so the caller paints the
/// result as one image with no reprojection.
pub async fn blockage_image(
    client: &reqwest::Client,
    site: BeamSite,
    world: [f64; 4],
) -> egui::ColorImage {
    let radar_msl = site.ground_m + TOWER_M;
    let polar = occultation(client, site, radar_msl).await;
    let mut px = vec![egui::Color32::TRANSPARENT; RASTER_N * RASTER_N];
    let (wx0, wy0, wx1, wy1) = (world[0], world[1], world[2], world[3]);
    for row in 0..RASTER_N {
        let wy = wy0 + (wy1 - wy0) * (row as f64 + 0.5) / RASTER_N as f64;
        for col in 0..RASTER_N {
            let wx = wx0 + (wx1 - wx0) * (col as f64 + 0.5) / RASTER_N as f64;
            let (lon, lat) = world_to_lonlat(wx, wy);
            let (km, az) = crate::geo::great_circle([site.lon, site.lat], [lon, lat]);
            if km >= MAX_RANGE_KM {
                continue;
            }
            let a = ((az / 360.0 * N_AZ as f64) as usize).min(N_AZ - 1);
            let occult = polar[a * N_RANGE + (km as usize).min(N_RANGE - 1)] as f64;
            let frac = blockage_fraction(occult, site.tilt_deg);
            // Below a tenth of the beam the bias is not worth painting over the radar.
            if frac < 0.1 {
                continue;
            }
            // Amber where the beam is clipped, red where it is gone. Alpha tracks the fraction so
            // the edge of a shadow fades instead of stepping.
            px[row * RASTER_N + col] = if frac < 0.5 {
                egui::Color32::from_rgba_unmultiplied(230, 170, 40, (frac * 130.0) as u8)
            } else {
                egui::Color32::from_rgba_unmultiplied(200, 40, 40, (frac * 150.0) as u8)
            };
        }
    }
    egui::ColorImage {
        size: [RASTER_N, RASTER_N],
        pixels: px,
        source_size: egui::vec2(RASTER_N as f32, RASTER_N as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_decodes_signed_metres() {
        // The zero offset: 128*256 - 32768 = 0.
        assert_eq!(decode_height([128, 0, 0]), 0.0);
        // Death Valley depth and a Rockies summit.
        assert_eq!(decode_height([127, 170, 0]), -86.0);
        assert!((decode_height([140, 108, 128]) - 3180.5).abs() < 0.01);
        // The B channel is the 1/256 m fraction.
        assert!((decode_height([128, 0, 128]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_ridge_blocks_the_beam_and_flat_ground_does_not() {
        // KMAX (Medford OR) sits at ~2290 m on Mount Ashland and scans at 0.5°.
        let radar_msl = 2290.0 + TOWER_M;
        let tilt = 0.5;
        // A valley floor 40 km out is far below the beam: no occultation at all.
        let valley = terrain_angle_deg(radar_msl, 500.0, 40.0);
        assert!(valley < -1.0, "valley angle {valley}");
        assert_eq!(blockage_fraction(valley, tilt), 0.0);
        // The beam's own centre height at that range, back through the model it came from — a
        // ridge exactly that tall hides half the beam.
        let beam_msl = radar_msl + wxdata::xsection::beam_height_km(40.0, tilt) * 1000.0;
        let grazing = terrain_angle_deg(radar_msl, beam_msl, 40.0);
        assert!((grazing - tilt).abs() < 0.02, "grazing angle {grazing}");
        assert!((blockage_fraction(grazing, tilt) - 0.5).abs() < 0.02);
        // A 4000 m wall at 40 km is a full block, and it stays blocked further out because the
        // scan carries the running maximum forward.
        let wall = terrain_angle_deg(radar_msl, 4000.0, 40.0);
        assert_eq!(blockage_fraction(wall, tilt), 1.0);
        assert!(wall > grazing);
    }

    /// The DEM zoom is process-global, and cargo runs tests in parallel — the tests that flip it
    /// and the tests that read it have to take turns or they read each other's zoom.
    static ZOOM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn hires_switches_both_url_and_cache_path() {
        let _g = ZOOM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::path::Path::new("/tmp/hookecho-test");
        set_hires(false);
        assert_eq!(zoom(), DEM_ZOOM);
        assert!(tile_url(3, 4).contains("/10/3/4.png"));
        assert!(tile_path(root, 3, 4).ends_with("10/3/4"));
        set_hires(true);
        assert_eq!(zoom(), DEM_ZOOM_HIRES);
        assert!(tile_url(3, 4).contains("/12/3/4.png"));
        assert!(tile_path(root, 3, 4).ends_with("12/3/4"));
        // A hi-res pack must not be read as if it were z10, so the two live in separate folders.
        set_hires(false);
        assert_ne!(tile_path(root, 3, 4), {
            set_hires(true);
            let p = tile_path(root, 3, 4);
            set_hires(false);
            p
        });
    }

    #[test]
    fn bilinear_reads_between_the_pixels() {
        // A tile whose height is its column index: the midpoint between two pixels is their mean,
        // and a whole-pixel coordinate still reads that pixel exactly.
        let heights: Vec<f32> = (0..256 * 256).map(|i| (i % 256) as f32).collect();
        assert_eq!(bilinear(&heights, 10.0, 4.0), 10.0);
        assert_eq!(bilinear(&heights, 10.5, 4.0), 10.5);
        assert_eq!(bilinear(&heights, 10.25, 200.0), 10.25);
        // The last half-pixel clamps to the edge rather than reading off the end.
        assert_eq!(bilinear(&heights, 255.5, 255.5), 255.0);
    }

    #[test]
    fn tile_of_lands_in_range() {
        let _g = ZOOM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (x, y, px, py) = tile_of(-97.28, 35.33).unwrap();
        assert!(x < 1 << zoom() && y < 1 << zoom());
        assert!((0.0..=255.0).contains(&px) && (0.0..=255.0).contains(&py));
        // Antimeridian wraps rather than panicking; poles are outside the Mercator band.
        assert!(tile_of(180.0, 0.0).is_some());
        assert!(tile_of(0.0, 89.0).is_none());
    }

    /// Live check against the classic terrain-blocked site: KMAX is walled in by the Cascades.
    /// Fetches real DEM tiles, so it stays out of the default run.
    #[tokio::test]
    #[ignore = "network"]
    async fn kmax_is_shadowed_where_the_terrain_is() {
        let http = reqwest::Client::new();
        let summit = elevation_m(&http, 42.081, -122.717).await.unwrap();
        assert!((2000.0..2600.0).contains(&summit), "KMAX summit {summit} m");
        let valley = elevation_m(&http, 42.33, -122.87).await.unwrap();
        assert!((300.0..600.0).contains(&valley), "Medford {valley} m");

        let site = BeamSite {
            lon: -122.717,
            lat: 42.081,
            ground_m: summit as f64,
            tilt_deg: 0.5,
        };
        let (cx, cy) = crate::render::mercator::lonlat_to_world(site.lon, site.lat);
        // ~250 km across, so the whole shadowed disk is in frame.
        let half = 0.0042;
        let img = blockage_image(&http, site, [cx - half, cy - half, cx + half, cy + half]).await;
        let shaded = img.pixels.iter().filter(|p| p.a() > 0).count();
        let total = img.pixels.len();
        // Real answer for this site is a percent or two of a 125 km window: the Cascade peaks
        // east and the Siskiyous south throw the shadows, the Rogue Valley is wide open.
        assert!(
            (200..total / 2).contains(&shaded),
            "{shaded} of {total} pixels shaded at 0.5\u{b0}"
        );
        // Same terrain, a tilt that clears all of it: nothing at all should be shaded. Without
        // this, a raster that shaded everything would pass the check above.
        let high = BeamSite {
            tilt_deg: 6.0,
            ..site
        };
        let img = blockage_image(&http, high, [cx - half, cy - half, cx + half, cy + half]).await;
        assert_eq!(img.pixels.iter().filter(|p| p.a() > 0).count(), 0);
    }
}
