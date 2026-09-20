//! Native GOES ABI Cloud and Moisture Imagery (CMIP) decoding.

use chrono::{DateTime, TimeZone, Utc};

use crate::field::{
    DataClass, DataStamp, FieldDescriptor, FieldFamily, FieldFrame, FieldId, MissingData,
    QualitySummary, SamplingPolicy, ValueKind,
};

const J2000_UNIX: i64 = 946_728_000;

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
    supports_contours: false,
    supports_difference: false,
};

pub fn descriptor_for_band(band: u8) -> Option<&'static FieldDescriptor> {
    match band {
        2 => Some(&C02_DESCRIPTOR),
        8 => Some(&C08_DESCRIPTOR),
        13 => Some(&C13_DESCRIPTOR),
        _ => None,
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
    compose_rgb(recipe, [&red, &green, &blue])
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
    let pixels = (0..first.values.len())
        .map(|index| {
            let mut pixel = [0, 0, 0, 0];
            let row = index / first.width;
            let col = index % first.width;
            let values = first
                .projection
                .lon_lat(first.x[col], first.y[row])
                .map(|(lon, lat)| {
                    std::array::from_fn(|channel| {
                        let image = channels[channel];
                        if image.x == first.x && image.y == first.y {
                            image.sample_at(index)
                        } else {
                            image.sample_nearest(lon, lat)
                        }
                    })
                });
            if let Some([Some(red), Some(green), Some(blue)]) = values {
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
    let (lon_west, lon_east, lat_south, lat_north) = first
        .bounds()
        .ok_or_else(|| anyhow::anyhow!("ABI RGB image is off Earth"))?;
    Ok(RgbImage {
        width: first.width,
        height: first.height,
        pixels,
        valid_time: channels
            .iter()
            .map(|image| image.valid_time)
            .min()
            .unwrap_or(first.valid_time),
        recipe,
        lon_west,
        lon_east,
        lat_north,
        lat_south,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Conus,
    Mesoscale1,
    Mesoscale2,
}

fn prefix(at: DateTime<Utc>, scene: Scene, band: u8) -> String {
    use chrono::Datelike;
    let (product, stem) = match scene {
        Scene::Conus => ("ABI-L2-CMIPC", "ABI-L2-CMIPC"),
        Scene::Mesoscale1 => ("ABI-L2-CMIPM", "ABI-L2-CMIPM1"),
        Scene::Mesoscale2 => ("ABI-L2-CMIPM", "ABI-L2-CMIPM2"),
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
    let mut image = decode(bytes)?;
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

#[derive(Debug, Clone, Copy, PartialEq)]
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
        if discriminant < 0.0 {
            return None; // scan coordinate is off Earth
        }
        let rs = (-b - discriminant.sqrt()) / (2.0 * a);
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

#[derive(Debug, Clone)]
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
        let descriptor = descriptor_for_band(self.band)
            .ok_or_else(|| anyhow::anyhow!("ABI: C{:02} is not registered", self.band))?;
        let valid_time = self.valid_time;
        let source_identity = self.source_identity.clone();
        let received_time = self.received_time.unwrap_or(received_time);
        let display = self.display_grid(self.width.min(700), self.height.min(700))?;
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
            },
        ))
    }

    fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut points = Vec::with_capacity((self.width + self.height) * 2);
        for &x in &self.x {
            points.extend([
                self.projection.lon_lat(x, self.y[0]),
                self.projection.lon_lat(x, *self.y.last()?),
            ]);
        }
        for &y in &self.y {
            points.extend([
                self.projection.lon_lat(self.x[0], y),
                self.projection.lon_lat(*self.x.last()?, y),
            ]);
        }
        let mut valid = points.into_iter().flatten();
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
    let file = hdf5lite::File::open(bytes).map_err(|e| anyhow::anyhow!("ABI: {e}"))?;
    let dims = file
        .dataset("CMI")
        .map_err(|e| anyhow::anyhow!("ABI CMI: {e}"))?
        .dims;
    let [height, width] = dims.as_slice() else {
        anyhow::bail!("ABI CMI: expected a two-dimensional image");
    };
    let (height, width) = (*height as usize, *width as usize);
    let mut values: Vec<f32> = file
        .read_f64("CMI")
        .map_err(|e| anyhow::anyhow!("ABI CMI: {e}"))?
        .into_iter()
        .map(|value| value as f32)
        .collect();
    let quality: Vec<u8> = file
        .read_f64("DQF")
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
            .map_err(|e| anyhow::anyhow!("ABI x: {e}"))?,
        y: file
            .read_f64("y")
            .map_err(|e| anyhow::anyhow!("ABI y: {e}"))?,
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
    }

    #[test]
    fn nearest_handles_both_coordinate_orders() {
        assert_eq!(nearest(&[0.0, 2.0, 4.0], 2.9), Some(1));
        assert_eq!(nearest(&[4.0, 2.0, 0.0], 2.9), Some(1));
        assert_eq!(nearest(&[0.0, 2.0], -1.0), Some(0));
        assert_eq!(nearest(&[2.0, 0.0], -1.0), Some(1));
    }

    #[test]
    fn declarative_rgb_rejects_bad_pixels_and_mismatched_channels() {
        let base = Image {
            width: 2,
            height: 1,
            band: 2,
            values: vec![0.25, 1.0],
            quality: vec![0, 3],
            x: vec![0.0, 1.0],
            y: vec![0.0],
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
        let rgb = compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).unwrap();
        assert_eq!(rgb.pixels[0], [136, 136, 136, 255]);
        assert_eq!(rgb.pixels[1], [0, 0, 0, 0]);
        blue.width = 3;
        blue.x = vec![0.0, 0.5, 1.0];
        blue.values = vec![0.25; 3];
        blue.quality = vec![0; 3];
        assert!(compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).is_ok());
        blue.valid_time += chrono::Duration::minutes(3);
        assert!(compose_rgb(&TRUE_COLOR, [&base, &green, &blue]).is_err());
    }
}
