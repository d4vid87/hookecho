//! Native GOES ABI Cloud and Moisture Imagery (CMIP) decoding.

use chrono::{DateTime, TimeZone, Utc};

use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};

const J2000_UNIX: i64 = 946_728_000;
const DISPLAY_MAX_DIM: usize = 1200;

#[cfg(not(target_arch = "wasm32"))]
static CPU_SLOTS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(if cfg!(target_os = "android") { 1 } else { 2 });

#[cfg(not(target_arch = "wasm32"))]
async fn process<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    crate::task::blocking_limited(&CPU_SLOTS, work).await?
}

#[cfg(not(target_arch = "wasm32"))]
async fn decode_for_display(bytes: Vec<u8>) -> anyhow::Result<Image> {
    process(move || decode_with_max_dim(bytes, DISPLAY_MAX_DIM)).await
}

pub static C13_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.c13"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES clean infrared",
    short_name: "GOES C13",
    search_aliases: &["satellite", "infrared", "cloud top temperature"],
    units: "K",
    value_kind: ValueKind::Scalar,
    palette_key: "infrared",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: true,
    supports_difference: true,
};

pub static C08_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.c08"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES upper-level water vapor",
    short_name: "GOES C08",
    search_aliases: &["satellite", "water vapor", "upper level"],
    units: "K",
    value_kind: ValueKind::Scalar,
    palette_key: "water-vapor",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: true,
    supports_difference: true,
};

pub static C09_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.c09"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES mid-level water vapor",
    short_name: "GOES C09",
    search_aliases: &["satellite", "water vapor", "mid level"],
    units: "K",
    value_kind: ValueKind::Scalar,
    palette_key: "water-vapor",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: true,
    supports_difference: true,
};

pub static C14_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.c14"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES longwave infrared",
    short_name: "GOES C14",
    search_aliases: &["satellite", "infrared", "cloud top temperature"],
    units: "K",
    value_kind: ValueKind::Scalar,
    palette_key: "infrared",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: true,
    supports_difference: true,
};

pub static C02_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.c02"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES red visible",
    short_name: "GOES C02",
    search_aliases: &["satellite", "visible", "red"],
    units: "reflectance",
    value_kind: ValueKind::Scalar,
    palette_key: "visible",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: false,
    supports_difference: true,
};

pub static TRUE_COLOR_DESCRIPTOR: FieldDescriptor = FieldDescriptor {
    id: FieldId("satellite.goes.abi.true-color"),
    source: "NOAA GOES ABI",
    family: FieldFamily::Satellite,
    display_name: "GOES true color",
    short_name: "GOES True Color",
    search_aliases: &["satellite", "visible", "rgb", "geocolor"],
    units: "RGB",
    value_kind: ValueKind::Vector,
    palette_key: "true-color",
    sampling: SamplingPolicy::Nearest,
    missing: MissingData::Nan,
    time_policy: None,
    supports_contours: false,
    supports_difference: false,
};

macro_rules! channel_descriptor {
    ($name:ident, $id:literal, $display:literal, $short:literal, $units:literal, $palette:literal, $contours:literal, [$($alias:literal),* $(,)?]) => {
        pub static $name: FieldDescriptor = FieldDescriptor {
            id: FieldId($id), source: "NOAA GOES ABI", family: FieldFamily::Satellite,
            display_name: $display, short_name: $short, search_aliases: &[$($alias),*],
            units: $units, value_kind: ValueKind::Scalar, palette_key: $palette,
            sampling: SamplingPolicy::Nearest, missing: MissingData::Nan, time_policy: None,
            supports_contours: $contours, supports_difference: true,
        };
    };
}

channel_descriptor!(C01_DESCRIPTOR, "satellite.goes.abi.c01", "GOES blue visible", "GOES C01", "reflectance", "visible", false, ["satellite", "visible", "blue"]);
channel_descriptor!(C03_DESCRIPTOR, "satellite.goes.abi.c03", "GOES veggie near-infrared", "GOES C03", "reflectance", "visible", false, ["satellite", "vegetation", "near infrared"]);
channel_descriptor!(C04_DESCRIPTOR, "satellite.goes.abi.c04", "GOES cirrus near-infrared", "GOES C04", "reflectance", "visible", false, ["satellite", "cirrus", "near infrared"]);
channel_descriptor!(C05_DESCRIPTOR, "satellite.goes.abi.c05", "GOES snow and ice near-infrared", "GOES C05", "reflectance", "visible", false, ["satellite", "snow", "ice"]);
channel_descriptor!(C06_DESCRIPTOR, "satellite.goes.abi.c06", "GOES cloud-particle near-infrared", "GOES C06", "reflectance", "visible", false, ["satellite", "cloud particle", "near infrared"]);
channel_descriptor!(C07_DESCRIPTOR, "satellite.goes.abi.c07", "GOES shortwave infrared", "GOES C07", "K", "infrared", true, ["satellite", "shortwave infrared", "fog"]);
channel_descriptor!(C10_DESCRIPTOR, "satellite.goes.abi.c10", "GOES lower-level water vapor", "GOES C10", "K", "water-vapor", true, ["satellite", "water vapor", "lower level"]);
channel_descriptor!(C11_DESCRIPTOR, "satellite.goes.abi.c11", "GOES cloud-top phase infrared", "GOES C11", "K", "infrared", true, ["satellite", "cloud phase", "infrared"]);
channel_descriptor!(C12_DESCRIPTOR, "satellite.goes.abi.c12", "GOES ozone infrared", "GOES C12", "K", "infrared", true, ["satellite", "ozone", "infrared"]);
channel_descriptor!(C15_DESCRIPTOR, "satellite.goes.abi.c15", "GOES dirty longwave infrared", "GOES C15", "K", "infrared", true, ["satellite", "dirty window", "infrared"]);
channel_descriptor!(C16_DESCRIPTOR, "satellite.goes.abi.c16", "GOES carbon-dioxide infrared", "GOES C16", "K", "infrared", true, ["satellite", "carbon dioxide", "infrared"]);

pub struct CatalogBand {
    pub band: u8,
    pub descriptor: &'static FieldDescriptor,
    pub slug: &'static str,
}

pub static CATALOG: [CatalogBand; 11] = [
    CatalogBand { band: 1, descriptor: &C01_DESCRIPTOR, slug: "goes-c01" },
    CatalogBand { band: 3, descriptor: &C03_DESCRIPTOR, slug: "goes-c03" },
    CatalogBand { band: 4, descriptor: &C04_DESCRIPTOR, slug: "goes-c04" },
    CatalogBand { band: 5, descriptor: &C05_DESCRIPTOR, slug: "goes-c05" },
    CatalogBand { band: 6, descriptor: &C06_DESCRIPTOR, slug: "goes-c06" },
    CatalogBand { band: 7, descriptor: &C07_DESCRIPTOR, slug: "goes-c07" },
    CatalogBand { band: 10, descriptor: &C10_DESCRIPTOR, slug: "goes-c10" },
    CatalogBand { band: 11, descriptor: &C11_DESCRIPTOR, slug: "goes-c11" },
    CatalogBand { band: 12, descriptor: &C12_DESCRIPTOR, slug: "goes-c12" },
    CatalogBand { band: 15, descriptor: &C15_DESCRIPTOR, slug: "goes-c15" },
    CatalogBand { band: 16, descriptor: &C16_DESCRIPTOR, slug: "goes-c16" },
];

pub fn descriptor_for_band(band: u8) -> Option<&'static FieldDescriptor> {
    match band {
        2 => Some(&C02_DESCRIPTOR),
        8 => Some(&C08_DESCRIPTOR),
        9 => Some(&C09_DESCRIPTOR),
        13 => Some(&C13_DESCRIPTOR),
        14 => Some(&C14_DESCRIPTOR),
        _ => CATALOG.iter().find(|entry| entry.band == band).map(|entry| entry.descriptor),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RgbRecipe {
    pub name: &'static str,
    pub bands: [u8; 3],
    pub ranges: [(f32, f32); 3],
    pub gamma: [f32; 3],
}

pub const TRUE_COLOR: RgbRecipe = RgbRecipe {
    name: "True color",
    bands: [2, 3, 1],
    ranges: [(0.0, 1.0); 3],
    gamma: [2.2; 3],
};

#[derive(Debug, Clone)]
pub struct RgbImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[u8; 4]>,
    pub valid_time: DateTime<Utc>,
    pub recipe: &'static RgbRecipe,
    pub lon_west: f64,
    pub lon_east: f64,
    pub lat_north: f64,
    pub lat_south: f64,
    pub source_identity: String,
    pub received_time: Option<DateTime<Utc>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Serialize, serde::Deserialize)]
struct RgbWire {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
    valid_time: DateTime<Utc>,
    lon_west: f64,
    lon_east: f64,
    lat_north: f64,
    lat_south: f64,
    source_identity: String,
    received_time: Option<DateTime<Utc>>,
}

#[cfg(target_arch = "wasm32")]
impl From<RgbImage> for RgbWire {
    fn from(image: RgbImage) -> Self {
        Self { width: image.width, height: image.height, pixels: image.pixels,
            valid_time: image.valid_time, lon_west: image.lon_west, lon_east: image.lon_east,
            lat_north: image.lat_north, lat_south: image.lat_south,
            source_identity: image.source_identity, received_time: image.received_time }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<RgbWire> for RgbImage {
    fn from(image: RgbWire) -> Self {
        Self { width: image.width, height: image.height, pixels: image.pixels,
            valid_time: image.valid_time, recipe: &TRUE_COLOR,
            lon_west: image.lon_west, lon_east: image.lon_east,
            lat_north: image.lat_north, lat_south: image.lat_south,
            source_identity: image.source_identity, received_time: image.received_time }
    }
}

/// Worker entry points use a bounded display grid and keep HDF5 decoding off the map thread.
#[cfg(target_arch = "wasm32")]
pub fn decode_worker(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    Ok(postcard::to_allocvec(&decode_with_max_dim(bytes, DISPLAY_MAX_DIM)?)?)
}

#[cfg(target_arch = "wasm32")]
pub fn compose_worker(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let (red, green, blue): (Image, Image, Image) = postcard::from_bytes(&bytes)?;
    let image = compose_rgb(&TRUE_COLOR, [&red, &green, &blue])?;
    Ok(postcard::to_allocvec(&RgbWire::from(image))?)
}

#[cfg(target_arch = "wasm32")]
pub fn project_worker(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let image: Image = postcard::from_bytes(&bytes)?;
    Ok(postcard::to_allocvec(&image.display_grid(image.width.min(700), image.height.min(700))?)?)
}

pub async fn fetch_rgb(
    http: &reqwest::Client,
    satellite: Satellite,
    scene: Scene,
    recipe: &'static RgbRecipe,
) -> anyhow::Result<RgbImage> {
    fetch_rgb_at(http, satellite, scene, recipe, Utc::now()).await
}

pub async fn fetch_rgb_at(
    http: &reqwest::Client,
    satellite: Satellite,
    scene: Scene,
    recipe: &'static RgbRecipe,
    at: DateTime<Utc>,
) -> anyhow::Result<RgbImage> {
    let (red, green, blue) = futures_util::future::try_join3(
        fetch_at(http, satellite, scene, recipe.bands[0], at),
        fetch_at(http, satellite, scene, recipe.bands[1], at),
        fetch_at(http, satellite, scene, recipe.bands[2], at),
    )
    .await?;
    #[cfg(target_arch = "wasm32")]
    {
        let payload = postcard::to_allocvec(&(red, green, blue))?;
        let encoded = crate::wasm_worker::compose_abi(payload).await
            .map_err(|e| anyhow::anyhow!("GOES worker: {e}"))?;
        let image: RgbWire = postcard::from_bytes(&encoded)?;
        return Ok(image.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    process(move || compose_rgb(recipe, [&red, &green, &blue])).await
}

pub fn compose_rgb(recipe: &'static RgbRecipe, channels: [&Image; 3]) -> anyhow::Result<RgbImage> {
    let first = channels[0];
    anyhow::ensure!(
        channels
            .iter()
            .enumerate()
            .all(|(index, image)| image.band == recipe.bands[index]
                && (image.valid_time - first.valid_time).num_seconds().abs() <= 120),
        "ABI RGB channels differ in band or valid time"
    );
    let display = first.display_grid(first.width.min(700), first.height.min(700))?;
    let pixels = display.values.iter().enumerate()
        .map(|(index, &red)| {
            let mut pixel = [0, 0, 0, 0];
            let row = index / display.nx;
            let col = index % display.nx;
            let lon = display.lon_west + (display.lon_east - display.lon_west)
                * col as f64 / (display.nx - 1) as f64;
            let lat = display.lat_north - (display.lat_north - display.lat_south)
                * row as f64 / (display.ny - 1) as f64;
            if let [Some(red), Some(green), Some(blue)] = [
                red.is_finite().then_some(red),
                channels[1].sample_nearest(lon, lat),
                channels[2].sample_nearest(lon, lat),
            ] {
                let values = [red, green, blue];
                for channel in 0..3 {
                    let (low, high) = recipe.ranges[channel];
                    let normalized = ((values[channel] - low) / (high - low))
                        .clamp(0.0, 1.0)
                        .powf(1.0 / recipe.gamma[channel]);
                    pixel[channel] = (normalized * 255.0).round() as u8;
                }
                pixel[3] = 255;
            }
            pixel
        })
        .collect();
    Ok(RgbImage {
        width: display.nx,
        height: display.ny,
        pixels,
        valid_time: channels
            .iter()
            .map(|image| image.valid_time)
            .min()
            .unwrap_or(first.valid_time),
        recipe,
        lon_west: display.lon_west,
        lon_east: display.lon_east,
        lat_north: display.lat_north,
        lat_south: display.lat_south,
        source_identity: channels
            .iter()
            .map(|image| image.source_identity.as_str())
            .collect::<Vec<_>>()
            .join(" + "),
        received_time: channels
            .iter()
            .filter_map(|image| image.received_time)
            .max(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Satellite {
    East,
    West,
}

impl Satellite {
    fn bucket(self) -> &'static str {
        match self {
            Self::East => crate::glm::EAST,
            Self::West => crate::glm::WEST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Scene {
    #[default]
    Conus,
    Mesoscale1,
    Mesoscale2,
    FullDisk,
}

fn prefix(at: DateTime<Utc>, scene: Scene, band: u8) -> String {
    use chrono::Datelike;
    let (product, stem) = match scene {
        Scene::Conus => ("ABI-L2-CMIPC", "ABI-L2-CMIPC"),
        Scene::Mesoscale1 => ("ABI-L2-CMIPM", "ABI-L2-CMIPM1"),
        Scene::Mesoscale2 => ("ABI-L2-CMIPM", "ABI-L2-CMIPM2"),
        Scene::FullDisk => ("ABI-L2-CMIPF", "ABI-L2-CMIPF"),
    };
    format!(
        "{product}/{:04}/{:03}/{:02}/OR_{stem}-M6C{band:02}",
        at.year(),
        at.ordinal(),
        at.format("%H")
    )
}

async fn latest_key(
    http: &reqwest::Client,
    satellite: Satellite,
    scene: Scene,
    band: u8,
    at: DateTime<Utc>,
) -> anyhow::Result<String> {
    for hour in [at, at - chrono::Duration::hours(1)] {
        let prefix = prefix(hour, scene, band);
        let url = format!(
            "{}/?list-type=2&prefix={prefix}&max-keys=1000",
            satellite.bucket()
        );
        let xml = http
            .get(crate::net::fetch_url(&url))
            .timeout(crate::net::FEED_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        if let Some(key) = xml
            .split("<Key>")
            .skip(1)
            .filter_map(|part| part.split_once("</Key>").map(|(key, _)| key))
            .filter_map(|key| {
                key_time(key)
                    .filter(|time| *time <= at)
                    .map(|time| (time, key))
            })
            .max_by_key(|(time, _)| *time)
            .map(|(_, key)| key)
        {
            return Ok(key.to_string());
        }
    }
    anyhow::bail!("GOES ABI: no recent C{band:02} image")
}

fn key_time(key: &str) -> Option<DateTime<Utc>> {
    let value = key.split("_s").nth(1)?.get(..13)?;
    let number = |range: std::ops::Range<usize>| value.get(range)?.parse::<u32>().ok();
    chrono::NaiveDate::from_yo_opt(number(0..4)? as i32, number(4..7)?)?
        .and_hms_opt(number(7..9)?, number(9..11)?, number(11..13)?)
        .map(|time| time.and_utc())
}

pub async fn fetch_latest(
    http: &reqwest::Client,
    satellite: Satellite,
    scene: Scene,
    band: u8,
) -> anyhow::Result<Image> {
    fetch_at(http, satellite, scene, band, Utc::now()).await
}

pub async fn fetch_at(
    http: &reqwest::Client,
    satellite: Satellite,
    scene: Scene,
    band: u8,
    at: DateTime<Utc>,
) -> anyhow::Result<Image> {
    if !(1..=16).contains(&band) {
        anyhow::bail!("GOES ABI: band must be 1 through 16");
    }
    let key = latest_key(http, satellite, scene, band, at).await?;
    let (bytes, received_time) = match crate::object_cache::get("satellite", &key).await {
        Some(cached) => (cached.bytes, cached.received_at),
        None => {
            let bytes = http
                .get(crate::net::fetch_url(&format!(
                    "{}/{}",
                    satellite.bucket(),
                    key
                )))
                .timeout(crate::net::FEED_TIMEOUT)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?
                .to_vec();
            let received = Utc::now();
            if let Err(error) = crate::object_cache::put("satellite", &key, &bytes, received).await
            {
                log::warn!("GOES ABI browser cache write failed: {error}");
            }
            (bytes, received)
        }
    };
    #[cfg(target_arch = "wasm32")]
    let mut image: Image = {
        let encoded = crate::wasm_worker::decode_abi(bytes).await
            .map_err(|e| anyhow::anyhow!("GOES worker: {e}"))?;
        postcard::from_bytes(&encoded)?
    };
    #[cfg(not(target_arch = "wasm32"))]
    let mut image = decode_for_display(bytes).await?;
    image.source_identity = key;
    image.received_time = Some(received_time);
    if image.band != band {
        anyhow::bail!(
            "GOES ABI: requested C{band:02}, received C{:02}",
            image.band
        );
    }
    Ok(image)
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Projection {
    pub longitude_origin_deg: f64,
    pub perspective_height_m: f64,
    pub semi_major_m: f64,
    pub semi_minor_m: f64,
}

impl Projection {
    /// Convert ABI fixed-grid scan angles (radians) to geodetic longitude/latitude.
    pub fn lon_lat(self, x: f64, y: f64) -> Option<(f64, f64)> {
        let h = self.perspective_height_m + self.semi_major_m;
        let (sin_x, cos_x) = x.sin_cos();
        let (sin_y, cos_y) = y.sin_cos();
        let ratio = self.semi_major_m.powi(2) / self.semi_minor_m.powi(2);
        let a = sin_x.powi(2) + cos_x.powi(2) * (cos_y.powi(2) + ratio * sin_y.powi(2));
        let b = -2.0 * h * cos_x * cos_y;
        let c = h.powi(2) - self.semi_major_m.powi(2);
        let discriminant = b * b - 4.0 * a * c;
        let rounding = 16.0 * f64::EPSILON * (b * b + (4.0 * a * c).abs());
        if !discriminant.is_finite() || discriminant < -rounding {
            return None; // scan coordinate is off Earth
        }
        let rs = (-b - discriminant.max(0.0).sqrt()) / (2.0 * a);
        let sx = rs * cos_x * cos_y;
        let sy = -rs * sin_x;
        let sz = rs * cos_x * sin_y;
        let lat = (ratio * sz / ((h - sx).powi(2) + sy.powi(2)).sqrt()).atan();
        let lon = self.longitude_origin_deg.to_radians() - (sy / (h - sx)).atan();
        Some((lon.to_degrees(), lat.to_degrees()))
    }

    /// Convert a geodetic point to ABI fixed-grid scan angles (radians).
    pub fn scan_angles(self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let req = self.semi_major_m;
        let rpol = self.semi_minor_m;
        let h = self.perspective_height_m + req;
        let lon = lon.to_radians();
        let lat = lat.to_radians();
        let lon0 = self.longitude_origin_deg.to_radians();
        let phi = ((rpol * rpol / (req * req)) * lat.tan()).atan();
        let rc = rpol / (1.0 - (req * req - rpol * rpol) / (req * req) * phi.cos().powi(2)).sqrt();
        let sx = h - rc * phi.cos() * (lon - lon0).cos();
        let sy = -rc * phi.cos() * (lon - lon0).sin();
        let sz = rc * phi.sin();
        if h * (h - sx) < sy * sy + req * req / (rpol * rpol) * sz * sz {
            return None;
        }
        let range = (sx * sx + sy * sy + sz * sz).sqrt();
        Some(((-sy / range).asin(), (sz / sx).atan()))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub band: u8,
    pub values: Vec<f32>,
    /// NOAA DQF: 0 good, 1 conditionally usable, 2 out of range, 3 missing, 4 focal-plane limit.
    pub quality: Vec<u8>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub projection: Projection,
    pub valid_time: DateTime<Utc>,
    pub source_identity: String,
    pub received_time: Option<DateTime<Utc>>,
}

impl Image {
    fn sample_at(&self, index: usize) -> Option<f32> {
        let value = *self.values.get(index)?;
        (self.quality.get(index).copied().unwrap_or(3) < 2 && value.is_finite()).then_some(value)
    }

    pub fn sample_nearest(&self, lon: f64, lat: f64) -> Option<f32> {
        let (x, y) = self.projection.scan_angles(lon, lat)?;
        let col = nearest(&self.x, x)?;
        let row = nearest(&self.y, y)?;
        let index = row.checked_mul(self.width)?.checked_add(col)?;
        self.sample_at(index)
    }

    /// Regrid native ABI values only for the existing display-texture pipeline.
    pub fn display_grid(
        &self,
        width: usize,
        height: usize,
    ) -> anyhow::Result<crate::mrms::MrmsField> {
        if width < 2 || height < 2 {
            anyhow::bail!("ABI display grid must be at least 2 by 2");
        }
        let bounds = self
            .bounds()
            .ok_or_else(|| anyhow::anyhow!("ABI image is off Earth"))?;
        let (lon_west, lon_east, lat_south, lat_north) = bounds;
        let mut values = Vec::with_capacity(width * height);
        for row in 0..height {
            let lat = lat_north - (lat_north - lat_south) * row as f64 / (height - 1) as f64;
            for col in 0..width {
                let lon = lon_west + (lon_east - lon_west) * col as f64 / (width - 1) as f64;
                values.push(self.sample_nearest(lon, lat).unwrap_or(f32::NAN));
            }
        }
        Ok(crate::mrms::MrmsField {
            values,
            nx: width,
            ny: height,
            lon_west,
            lon_east,
            lat_north,
            lat_south,
            time: self.valid_time,
        })
    }

    pub fn into_frame(self, received_time: DateTime<Utc>) -> anyhow::Result<FieldFrame> {
        let display = self.display_grid(self.width.min(700), self.height.min(700))?;
        self.into_frame_with_display(received_time, display)
    }

    /// Project display pixels without occupying a native async-runtime thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn into_frame_async(self, received_time: DateTime<Utc>) -> anyhow::Result<FieldFrame> {
        process(move || self.into_frame(received_time)).await
    }

    pub fn into_frame_with_display(
        self,
        received_time: DateTime<Utc>,
        display: crate::mrms::MrmsField,
    ) -> anyhow::Result<FieldFrame> {
        let descriptor = descriptor_for_band(self.band)
            .ok_or_else(|| anyhow::anyhow!("ABI: C{:02} is not registered", self.band))?;
        let valid_time = self.valid_time;
        let source_identity = self.source_identity.clone();
        let received_time = self.received_time.unwrap_or(received_time);
        Ok(FieldFrame::from_abi(
            descriptor,
            self,
            display,
            DataStamp {
                source_identity,
                issue_time: None,
                run_time: None,
                valid_time,
                received_time,
                class: DataClass::Observed,
                quality: QualitySummary::Unknown,
                available_members: None,
            },
        ))
    }

    fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let p = self.projection;
        let h = p.perspective_height_m + p.semi_major_m;
        if ![h, p.longitude_origin_deg, p.semi_major_m, p.semi_minor_m].into_iter().all(f64::is_finite)
            || p.semi_minor_m <= 0.0 || p.semi_major_m < p.semi_minor_m || h <= p.semi_major_m
            || !self.x.iter().chain(&self.y).all(|angle| angle.is_finite() && angle.abs() < std::f64::consts::FRAC_PI_2) {
            return None;
        }
        let (xmin, xmax) = ((*self.x.first()?).min(*self.x.last()?), (*self.x.first()?).max(*self.x.last()?));
        let (ymin, ymax) = ((*self.y.first()?).min(*self.y.last()?), (*self.y.first()?).max(*self.y.last()?));
        let q = p.semi_major_m.powi(2) / (h * h - p.semi_major_m.powi(2));
        let ratio = p.semi_major_m.powi(2) / p.semi_minor_m.powi(2);
        let mut points = Vec::with_capacity(12);
        // The full-disk rectangle extends into space. Clip its edges/axes to the Earth limb,
        // whose ray discriminant is zero: tan²(x) = q cos²(y) - ratio sin²(y).
        for x in [xmin, xmax, 0.0f64.clamp(xmin, xmax)] {
            let sin2_y = (q - x.tan().powi(2)) / (q + ratio);
            if sin2_y < 0.0 { continue; }
            let limit = sin2_y.min(1.0).sqrt().asin();
            let (south, north) = (ymin.max(-limit), ymax.min(limit));
            if south <= north { points.extend([p.lon_lat(x, south), p.lon_lat(x, north)]); }
        }
        for y in [ymin, ymax, 0.0f64.clamp(ymin, ymax)] {
            let tan2_x = q * y.cos().powi(2) - ratio * y.sin().powi(2);
            if tan2_x < 0.0 { continue; }
            let limit = tan2_x.sqrt().atan();
            let (west, east) = (xmin.max(-limit), xmax.min(limit));
            if west <= east { points.extend([p.lon_lat(west, y), p.lon_lat(east, y)]); }
        }
        // Keep longitude continuous around the satellite meridian, including across ±180°.
        let mut valid = points.into_iter().flatten().filter(|(lon, lat)| lon.is_finite() && lat.is_finite());
        let (mut west, mut south) = valid.next()?;
        let (mut east, mut north) = (west, south);
        for (lon, lat) in valid {
            west = west.min(lon);
            east = east.max(lon);
            south = south.min(lat);
            north = north.max(lat);
        }
        Some((west, east, south, north))
    }
}

fn nearest(values: &[f64], target: f64) -> Option<usize> {
    let first = *values.first()?;
    let ascending = first <= *values.last()?;
    let split = values.partition_point(|value| {
        if ascending {
            *value < target
        } else {
            *value > target
        }
    });
    match split {
        0 => Some(0),
        n if n == values.len() => Some(n - 1),
        n => ((values[n - 1] - target).abs() <= (values[n] - target).abs())
            .then_some(n - 1)
            .or(Some(n)),
    }
}

pub fn decode(bytes: Vec<u8>) -> anyhow::Result<Image> {
    decode_with_max_dim(bytes, usize::MAX)
}

/// Keep only display-resolution points before HDF5 expands a full CONUS band into memory.
pub fn decode_with_max_dim(bytes: Vec<u8>, max_dim: usize) -> anyhow::Result<Image> {
    let file = hdf5lite::File::open(bytes).map_err(|e| anyhow::anyhow!("ABI: {e}"))?;
    let dims = file
        .dataset("CMI")
        .map_err(|e| anyhow::anyhow!("ABI CMI: {e}"))?
        .dims;
    let [height, width] = dims.as_slice() else {
        anyhow::bail!("ABI CMI: expected a two-dimensional image");
    };
    anyhow::ensure!(
        file.dataset("DQF").map_err(|e| anyhow::anyhow!("ABI DQF: {e}"))?.dims == dims,
        "ABI: image and quality dimensions disagree"
    );
    let (native_height, native_width) = (*height as usize, *width as usize);
    let stride = native_width.max(native_height).div_ceil(max_dim.max(1)).max(1);
    let (height, width) = (native_height.div_ceil(stride), native_width.div_ceil(stride));
    let mut values: Vec<f32> = file
        .read_f64_strided_2d("CMI", stride)
        .map_err(|e| anyhow::anyhow!("ABI CMI: {e}"))?
        .into_iter()
        .map(|value| value as f32)
        .collect();
    let quality: Vec<u8> = file
        .read_f64_strided_2d("DQF", stride)
        .map_err(|e| anyhow::anyhow!("ABI DQF: {e}"))?
        .into_iter()
        .map(|value| if value.is_finite() { value as u8 } else { 3 })
        .collect();
    if values.len() != width * height || quality.len() != values.len() {
        anyhow::bail!("ABI: image and quality dimensions disagree");
    }
    for (value, &flag) in values.iter_mut().zip(&quality) {
        if flag >= 2 {
            *value = f32::NAN;
        }
    }
    let attrs = file
        .attributes("goes_imager_projection")
        .map_err(|e| anyhow::anyhow!("ABI projection: {e}"))?;
    let number = |name: &str| {
        attrs
            .get(name)
            .and_then(hdf5lite::Value::as_f64)
            .ok_or_else(|| anyhow::anyhow!("ABI projection: missing {name}"))
    };
    let projection = Projection {
        longitude_origin_deg: number("longitude_of_projection_origin")?,
        perspective_height_m: number("perspective_point_height")?,
        semi_major_m: number("semi_major_axis")?,
        semi_minor_m: number("semi_minor_axis")?,
    };
    let t = file
        .read_f64("t")
        .ok()
        .and_then(|times| times.first().copied())
        .and_then(|seconds| Utc.timestamp_opt(J2000_UNIX + seconds as i64, 0).single())
        .ok_or_else(|| anyhow::anyhow!("ABI: missing valid time"))?;
    let root = file.attributes("").unwrap_or_default();
    Ok(Image {
        width,
        height,
        band: file
            .read_f64("band_id")
            .ok()
            .and_then(|bands| bands.first().copied())
            .unwrap_or_default() as u8,
        values,
        quality,
        x: file
            .read_f64("x")
            .map_err(|e| anyhow::anyhow!("ABI x: {e}"))?
            .into_iter().step_by(stride).collect(),
        y: file
            .read_f64("y")
            .map_err(|e| anyhow::anyhow!("ABI y: {e}"))?
            .into_iter().step_by(stride).collect(),
        projection,
        valid_time: t,
        source_identity: root
            .get("dataset_name")
            .and_then(hdf5lite::Value::as_str)
            .unwrap_or("NOAA GOES ABI CMIP")
            .to_string(),
        received_time: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test(flavor = "current_thread")]
    async fn full_resolution_disk_is_bounded_before_decode_and_keeps_quality_and_navigation() {
        // A 21,696² C02 disk would require 3.77 GB for just the expanded f64 CMI array.
        let bytes = include_bytes!("../tests/data/abi-full-disk-sparse.nc").to_vec();
        assert!(decode(bytes.clone()).is_err(), "full decode should exceed the HDF5 safety ceiling");
        let image = decode_for_display(bytes).await.unwrap();
        assert_eq!((image.width, image.height, image.band), (1142, 1142, 2));
        assert!(image.width <= DISPLAY_MAX_DIM && image.height <= DISPLAY_MAX_DIM);
        assert_eq!(image.values.len(), image.width * image.height);
        assert_eq!(image.quality.len(), image.values.len());
        assert_eq!((image.x.len(), image.y.len()), (image.width, image.height));
        assert_eq!(image.values[400 * image.width + 600], 0.5);
        assert_eq!(image.quality[400 * image.width + 600], 0);
        assert!(image.values[400 * image.width + 601].is_nan());
        assert_eq!(image.quality[400 * image.width + 601], 2);
        assert!(image.values[0].is_nan());
        let x = -0.151844 + 0.303688 * (19 * 600) as f64 / 21695.0;
        let y = 0.151844 - 0.303688 * (19 * 400) as f64 / 21695.0;
        assert!((image.x[600] - x).abs() < 1e-12);
        assert!((image.y[400] - y).abs() < 1e-12);
        let (lon, lat) = image.projection.lon_lat(x, y).unwrap();
        assert_eq!(image.sample_nearest(lon, lat), Some(0.5));
        let valid_time = image.valid_time;
        let frame = image.into_frame_async(Utc::now()).await.unwrap();
        assert_eq!(frame.stamp.valid_time, valid_time);
        assert_eq!((frame.field().nx, frame.field().ny), (700, 700));
        assert_eq!(frame.sample(lon, lat).value, Some(0.5));
        assert!(frame.field().lat_north > 80.0 && frame.field().lat_south < -80.0);
        assert!(frame.field().lon_west < -180.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test(flavor = "current_thread")]
    async fn native_abi_cpu_work_leaves_the_async_scheduler_responsive() {
        let scheduler_thread = std::thread::current().id();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let work = tokio::spawn(process(move || {
            assert_ne!(std::thread::current().id(), scheduler_thread);
            started_tx.send(()).unwrap();
            // The current-thread runtime must keep polling to release this worker.
            finish_rx.recv_timeout(std::time::Duration::from_secs(5))?;
            Ok(17)
        }));
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx).await.unwrap().unwrap();
        tokio::task::yield_now().await;
        finish_tx.send(()).unwrap();
        assert_eq!(work.await.unwrap().unwrap(), 17);
    }

    #[test]
    fn full_disk_bounds_include_the_limb_and_cross_the_antimeridian_continuously() {
        let mut image = decode_with_max_dim(include_bytes!("../tests/data/g19-c13-meso.nc").to_vec(), 3).unwrap();
        image.x = vec![-0.151844, 0.0, 0.151844];
        image.y = vec![0.151844, 0.0, -0.151844];
        image.values.fill(1.0);
        image.quality.fill(0);
        for meridian in [-75.0, -137.0, 140.0] {
            image.projection.longitude_origin_deg = meridian;
            let (west, east, south, north) = image.bounds().unwrap();
            assert!(south < -80.0 && north > 80.0, "full disk lost high latitudes: {south}..{north}");
            assert!(west < meridian - 80.0 && east > meridian + 80.0);
            assert!(east - west < 180.0, "antimeridian crossing expanded the image into a world map");
            let display = image.display_grid(65, 65).unwrap();
            assert_eq!(display.sample_nearest(meridian, 60.0), Some(1.0));
            assert_eq!(display.sample_nearest(meridian, -60.0), Some(1.0));
            assert!(display.values[0].is_nan(), "off-Earth corners must stay transparent");
        }
    }

    #[test]
    fn clipped_scene_bounds_enclose_every_visible_sample() {
        let mut image = decode_with_max_dim(include_bytes!("../tests/data/g19-c13-meso.nc").to_vec(), 3).unwrap();
        for (xmin, xmax, ymin, ymax) in [
            (-0.16, 0.16, -0.16, 0.16), (-0.1, 0.04, 0.01, 0.13),
            (-0.06, -0.04, 0.09, 0.1), (-0.01, 0.01, 0.14, 0.16),
            (-0.16, -0.14, -0.05, 0.05), (0.14, 0.16, 0.03, 0.08),
        ] {
            image.x = vec![xmin, (xmin + xmax) / 2.0, xmax];
            image.y = vec![ymax, (ymin + ymax) / 2.0, ymin];
            let (west, east, south, north) = image.bounds().unwrap();
            for col in 0..=100 {
                for row in 0..=100 {
                    let x = xmin + (xmax - xmin) * col as f64 / 100.0;
                    let y = ymin + (ymax - ymin) * row as f64 / 100.0;
                    if let Some((lon, lat)) = image.projection.lon_lat(x, y) {
                        assert!(lon >= west - 1e-6 && lon <= east + 1e-6 && lat >= south - 1e-6 && lat <= north + 1e-6,
                            "sample {lon},{lat} outside {west},{east},{south},{north}");
                    }
                }
            }
        }
        image.x = vec![0.2, 0.25, 0.3];
        assert!(image.bounds().is_none());
        image.x.clear();
        assert!(image.bounds().is_none());
        image.x = vec![f64::NAN];
        assert!(image.bounds().is_none());
    }

    #[test]
    fn decodes_real_goes19_c13_mesoscale_with_quality_and_navigation() {
        let image = decode(include_bytes!("../tests/data/g19-c13-meso.nc").to_vec()).unwrap();
        assert_eq!((image.width, image.height, image.band), (500, 500, 13));
        assert_eq!(image.values.len(), 250_000);
        assert!(image.values.iter().any(|value| value.is_finite()));
        assert!(image
            .values
            .iter()
            .zip(&image.quality)
            .all(|(value, flag)| { *flag < 2 || value.is_nan() }));
        let (lon, lat) = image
            .projection
            .lon_lat(image.x[250], image.y[250])
            .unwrap();
        assert!((lon - -87.857_76).abs() < 0.001, "longitude {lon}");
        assert!((lat - 36.059_45).abs() < 0.001, "latitude {lat}");
        assert!(image.projection.lon_lat(0.3, 0.3).is_none());
        assert_eq!(image.valid_time.timestamp(), 1_744_308_031);

        let (x, y) = image.projection.scan_angles(lon, lat).unwrap();
        assert!((x - image.x[250]).abs() < 1e-8, "x {x} != {}", image.x[250]);
        assert!((y - image.y[250]).abs() < 1e-8, "y {y} != {}", image.y[250]);
        assert_eq!(
            image.sample_nearest(lon, lat),
            Some(image.values[250 * 500 + 250])
        );
        let frame = image.into_frame(Utc::now()).unwrap();
        assert_eq!(frame.descriptor.id, C13_DESCRIPTOR.id);
        assert_eq!(
            frame.sample(lon, lat).value,
            Some(frame.native_abi().unwrap().values[250 * 500 + 250])
        );
    }

    #[test]
    fn bounded_decode_samples_the_same_scientific_cells() {
        let bytes = include_bytes!("../tests/data/g19-c13-meso.nc").to_vec();
        let full = decode(bytes.clone()).unwrap();
        let small = decode_with_max_dim(bytes, 100).unwrap();
        assert_eq!((small.width, small.height), (100, 100));
        for (x, y) in [(0, 0), (31, 47), (99, 99)] {
            let at = y * small.width + x;
            let source = (y * 5) * full.width + x * 5;
            assert_eq!(small.quality[at], full.quality[source]);
            assert_eq!(small.values[at].to_bits(), full.values[source].to_bits());
            assert_eq!(small.x[x], full.x[x * 5]);
            assert_eq!(small.y[y], full.y[y * 5]);
        }
    }

    #[test]
    fn official_bucket_prefixes_name_each_supported_scene() {
        let at = Utc.with_ymd_and_hms(2025, 4, 10, 18, 0, 0).unwrap();
        assert_eq!(
            prefix(at, Scene::Conus, 13),
            "ABI-L2-CMIPC/2025/100/18/OR_ABI-L2-CMIPC-M6C13"
        );
        assert_eq!(
            key_time("OR_ABI-L2-CMIPC-M6C13_G19_s20251001846231_e.nc")
                .unwrap()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
            "2025-04-10 18:46:23"
        );
        assert_eq!(
            prefix(at, Scene::Mesoscale2, 9),
            "ABI-L2-CMIPM/2025/100/18/OR_ABI-L2-CMIPM2-M6C09"
        );
        assert_eq!(
            prefix(at, Scene::Mesoscale1, 2),
            "ABI-L2-CMIPM/2025/100/18/OR_ABI-L2-CMIPM1-M6C02"
        );
        assert_eq!(
            prefix(at, Scene::FullDisk, 13),
            "ABI-L2-CMIPF/2025/100/18/OR_ABI-L2-CMIPF-M6C13"
        );
        for band in 1..=16 {
            assert_eq!(
                descriptor_for_band(band).unwrap().id.0,
                format!("satellite.goes.abi.c{band:02}")
            );
        }
        assert!(descriptor_for_band(99).is_none());
        let mut ids: Vec<_> = CATALOG.iter().map(|entry| entry.descriptor.id.0).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
    }

    #[test]
    fn nearest_handles_both_coordinate_orders() {
        assert_eq!(nearest(&[0.0, 2.0, 4.0], 2.9), Some(1));
        assert_eq!(nearest(&[4.0, 2.0, 0.0], 2.9), Some(1));
        assert_eq!(nearest(&[0.0, 2.0], -1.0), Some(0));
        assert_eq!(nearest(&[2.0, 0.0], -1.0), Some(1));
    }

    #[test]
    fn rgb_places_native_fixture_pixels_on_the_geographic_display_grid() {
        let mut red = decode(include_bytes!("../tests/data/g19-c13-meso.nc").to_vec()).unwrap();
        red.band = 2;
        red.values.fill(0.0);
        red.quality.fill(0);
        let mut green = red.clone();
        green.band = 3;
        green.values.fill(0.25);
        let mut blue = green.clone();
        blue.band = 1;
        for row in 247..=253 {
            for col in 247..=253 {
                red.values[row * red.width + col] = 1.0;
            }
        }
        let rgb = compose_rgb(&TRUE_COLOR, [&red, &green, &blue]).unwrap();
        let (lon, lat) = red.projection.lon_lat(red.x[250], red.y[250]).unwrap();
        let col = ((lon - rgb.lon_west) / (rgb.lon_east - rgb.lon_west)
            * (rgb.width - 1) as f64)
            .round() as usize;
        let row = ((rgb.lat_north - lat) / (rgb.lat_north - rgb.lat_south)
            * (rgb.height - 1) as f64)
            .round() as usize;
        assert_eq!(rgb.pixels[row * rgb.width + col], [255, 136, 136, 255]);
        assert_eq!(rgb.pixels[250 * rgb.width + 250], [0, 136, 136, 255]);
    }

    #[test]
    fn rgb_bounds_output_and_keeps_off_earth_pixels_transparent() {
        let mut red = decode_with_max_dim(
            include_bytes!("../tests/data/g19-c13-meso.nc").to_vec(),
            3,
        ).unwrap();
        red.band = 2;
        red.x = vec![-0.15, 0.0, 0.15];
        red.y = vec![0.15, 0.0, -0.15];
        red.values.fill(0.25);
        red.quality.fill(0);
        let mut green = red.clone();
        green.band = 3;
        let mut blue = red.clone();
        blue.band = 1;
        let rgb = compose_rgb(&TRUE_COLOR, [&red, &green, &blue]).unwrap();
        assert_eq!(rgb.pixels[4], [136, 136, 136, 255]);
        for corner in [0, 2, 6, 8] {
            assert_eq!(rgb.pixels[corner], [0, 0, 0, 0]);
        }
        red.x.fill(0.3);
        assert!(compose_rgb(&TRUE_COLOR, [&red, &green, &blue]).is_err());

        red.width = 701;
        red.x = (0..701).map(|col| (col as f64 - 350.0) * 0.00001).collect();
        red.y = vec![0.001, 0.0, -0.001];
        red.values = vec![0.25; red.width * red.height];
        red.quality = vec![0; red.values.len()];
        green = red.clone();
        green.band = 3;
        blue = red.clone();
        blue.band = 1;
        let rgb = compose_rgb(&TRUE_COLOR, [&red, &green, &blue]).unwrap();
        assert_eq!((rgb.width, rgb.height, rgb.pixels.len()), (700, 3, 2100));
        assert!(rgb.pixels.iter().all(|pixel| *pixel == [136, 136, 136, 255]));
    }

    #[test]
    fn declarative_rgb_rejects_bad_pixels_and_mismatched_channels() {
        let base = Image {
            width: 2,
            height: 2,
            band: 2,
            values: vec![0.25, 1.0, 0.25, 1.0],
            quality: vec![0, 3, 0, 0],
            x: vec![0.0, 0.01],
            y: vec![0.01, 0.0],
            projection: Projection {
                longitude_origin_deg: -75.0,
                perspective_height_m: 35_786_023.0,
                semi_major_m: 6_378_137.0,
                semi_minor_m: 6_356_752.314_14,
            },
            valid_time: Utc::now(),
            source_identity: "fixture".into(),
            received_time: None,
        };
        let mut green = base.clone();
        green.band = 3;
        let mut blue = base.clone();
        blue.band = 1;
        green.valid_time += chrono::Duration::seconds(30);
        blue.received_time = Some(base.valid_time + chrono::Duration::minutes(1));
        let rgb = compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).unwrap();
        assert_eq!(rgb.pixels[0], [136, 136, 136, 255]);
        assert_eq!(rgb.pixels[1], [0, 0, 0, 0]);
        assert_eq!(rgb.valid_time, base.valid_time);
        assert_eq!(rgb.received_time, blue.received_time);
        assert_eq!(rgb.source_identity, "fixture + fixture + fixture");
        blue.width = 3;
        blue.height = 3;
        blue.x = vec![0.0, 0.005, 0.01];
        blue.y = vec![0.01, 0.005, 0.0];
        blue.values = vec![1.0, 0.5, 0.25, 0.5, 0.5, 0.5, 0.0, 0.5, 1.0];
        blue.quality = vec![0; 9];
        let rgb = compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).unwrap();
        assert_eq!(rgb.pixels, vec![
            [136, 136, 255, 255],
            [0, 0, 0, 0],
            [136, 136, 0, 255],
            [255, 255, 255, 255],
        ]);
        blue.quality[6] = 2;
        green.values[0] = f32::NAN;
        let rgb = compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).unwrap();
        assert_eq!(rgb.pixels[0], [0, 0, 0, 0]);
        assert_eq!(rgb.pixels[2], [0, 0, 0, 0]);
        blue.valid_time += chrono::Duration::minutes(3);
        assert!(compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).is_err());
        blue.valid_time = base.valid_time;
        blue.band = 2;
        assert!(compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).is_err());
    }
}
