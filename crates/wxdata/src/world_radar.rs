//! Public Asian radar observations. These are composites, not native Doppler volumes.

use anyhow::{bail, ensure, Context};
use chrono::{DateTime, NaiveDateTime, Utc};

pub const JMA_CATALOG_URL: &str =
    "https://www.jma.go.jp/bosai/jmatile/data/nowc/targetTimes_N1.json";
pub const CWA_GRID_URL: &str =
    "https://cwaopendata.s3.ap-northeast-1.amazonaws.com/Observation/O-A0059-001.xml";
pub const CWA_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const KMA_MAX_BYTES: usize = 48 * 1024 * 1024;

/// Native clients use the same credential-holding gateway as the web app. No API key is shipped.
pub fn regional_service_url(path: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    if let Some(origin) = web_sys::window().and_then(|window| window.location().origin().ok()) {
        return format!("{origin}/regional-radar/{path}");
    }
    format!("https://app.hookecho.io/regional-radar/{path}")
}

/// KMA filenames use Korean Standard Time. Only actual HSR files advertised by the service
/// become playable times; the adapter does not invent a five-minute history.
pub fn parse_kma_frames(bytes: &[u8]) -> anyhow::Result<Vec<DateTime<Utc>>> {
    ensure!(bytes.len() <= 1024 * 1024, "KMA catalog exceeds size limit");
    let text = std::str::from_utf8(bytes).context("KMA catalog is not UTF-8")?;
    let mut times = Vec::new();
    for segment in text.split("RDR_CMP_HSR_").skip(1) {
        let filename = segment
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            .next()
            .unwrap_or("");
        let Some(stem) = filename
            .strip_suffix(".bin.gz")
            .or_else(|| filename.strip_suffix(".bin"))
        else {
            continue;
        };
        let stamp = stem.rsplit('_').next().unwrap_or("");
        if stamp.len() != 12 || !stamp.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(time) = NaiveDateTime::parse_from_str(stamp, "%Y%m%d%H%M") {
            times.push(time.and_utc() - chrono::Duration::hours(9));
        }
    }
    times.sort_unstable();
    times.dedup();
    ensure!(times.len() <= 512, "KMA catalog has too many frames");
    Ok(times)
}

pub fn kma_time_parameter(time: DateTime<Utc>) -> String {
    (time + chrono::Duration::hours(9))
        .format("%Y%m%d%H%M")
        .to_string()
}

/// KMA's HSR ASCII API, with rows proceeding west to east then south to north. i16 storage
/// retains the original dBZ * 100 values and keeps one full composite around 13 MB.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct KmaGrid {
    pub time: DateTime<Utc>,
    values: Vec<i16>,
    #[serde(skip, default = "kma_geographic")]
    geographic: proj4rs::Proj,
    #[serde(skip, default = "kma_lambert")]
    lambert: proj4rs::Proj,
}

fn kma_geographic() -> proj4rs::Proj {
    proj4rs::Proj::from_proj_string("+proj=longlat +R=6371008.77 +no_defs")
        .expect("constant KMA sphere")
}

fn kma_lambert() -> proj4rs::Proj {
    proj4rs::Proj::from_proj_string(
        "+proj=lcc +lat_1=30 +lat_2=60 +lat_0=38 +lon_0=126 +R=6371008.77 +units=m +no_defs",
    )
    .expect("constant KMA projection")
}

impl KmaGrid {
    pub fn sample_wgs84(&self, lon: f64, lat: f64) -> RadarSample {
        if !lon.is_finite() || !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
            return RadarSample::Outside;
        }
        let mut point = (lon.to_radians(), lat.to_radians(), 0.0);
        if proj4rs::transform::transform(&self.geographic, &self.lambert, &mut point).is_err() {
            return RadarSample::Outside;
        }
        // Official HB diagram puts 126E/38N 560 km east and 840 km north of the lower-left.
        let x = (point.0 / 500.0 + 1120.0).round();
        let y = (point.1 / 500.0 + 1680.0).round();
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..2305.0).contains(&x)
            || !(0.0..2881.0).contains(&y)
        {
            return RadarSample::Outside;
        }
        kma_sample(self.values[y as usize * 2305 + x as usize])
    }
}

fn kma_sample(value: i16) -> RadarSample {
    match value {
        -30000 => RadarSample::Outside,
        -25000 => RadarSample::Missing,
        -20000 => RadarSample::NoEcho,
        value => RadarSample::Echo(value as f32 / 100.0),
    }
}

/// `time` must come from the advertised KMA file list and be requested explicitly from the
/// gateway. The API's undated "latest" response is deliberately unsupported.
pub fn parse_kma_grid(bytes: &[u8], time: DateTime<Utc>) -> anyhow::Result<KmaGrid> {
    ensure!(bytes.len() <= KMA_MAX_BYTES, "KMA grid exceeds size limit");
    let text = std::str::from_utf8(bytes).context("KMA grid is not UTF-8")?;
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    let dimensions = lines.next().context("KMA grid is empty")?;
    let mut dimensions = dimensions
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "=");
    ensure!(
        dimensions.next() == Some("2305")
            && dimensions.next() == Some("2881")
            && dimensions.next().is_none(),
        "unsupported KMA grid dimensions"
    );
    let count = 2305 * 2881;
    let mut values = Vec::with_capacity(count);
    for value in lines
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "=")
    {
        ensure!(values.len() < count, "too many KMA values");
        let value: i16 = value.parse().context("invalid KMA reflectivity")?;
        if !(-4000..=10000).contains(&value) && ![-30000, -25000, -20000].contains(&value) {
            bail!("KMA reflectivity out of range");
        }
        values.push(value);
    }
    ensure!(values.len() == count, "incomplete KMA grid");
    // KMA's operational LCC uses a sphere, true latitudes 30/60 and reference 38N/126E.
    // https://apihub.kma.go.kr/apiList.do?seqApi=5 (HSR HB grid / format specification).
    Ok(KmaGrid {
        time,
        values,
        geographic: kma_geographic(),
        lambert: kma_lambert(),
    })
}

/// Keep only advertised observation frames. Forecasts use a different valid time and must never
/// quietly enter the live radar loop. JMA's website feed is not a guaranteed stable API.
pub fn parse_jma_frames(bytes: &[u8]) -> anyhow::Result<Vec<DateTime<Utc>>> {
    ensure!(bytes.len() <= 64 * 1024, "JMA catalog exceeds size limit");
    #[derive(serde::Deserialize)]
    struct Entry {
        basetime: String,
        validtime: String,
        elements: Vec<String>,
    }
    let entries: Vec<Entry> = serde_json::from_slice(bytes).context("invalid JMA catalog")?;
    ensure!(entries.len() <= 512, "JMA catalog has too many frames");
    let mut times = entries
        .into_iter()
        .filter(|entry| {
            entry.basetime == entry.validtime
                && entry.elements.iter().any(|element| element == "hrpns")
                && entry.validtime.len() == 14
                && entry.validtime.bytes().all(|byte| byte.is_ascii_digit())
        })
        .filter_map(|entry| {
            NaiveDateTime::parse_from_str(&entry.validtime, "%Y%m%d%H%M%S")
                .ok()
                .map(|time| time.and_utc())
        })
        .collect::<Vec<_>>();
    times.sort_unstable();
    times.dedup();
    Ok(times)
}

/// Call only with a time obtained from `parse_jma_frames`; unavailable times are not guessed.
pub fn jma_tile_url(time: DateTime<Utc>, z: u8, x: u32, y: u32) -> Option<String> {
    if z > 10 || x >= 1_u32 << z || y >= 1_u32 << z {
        return None;
    }
    let stamp = time.format("%Y%m%d%H%M%S");
    Some(format!("https://www.jma.go.jp/bosai/jmatile/data/nowc/{stamp}/none/{stamp}/surf/hrpns/{z}/{x}/{y}.png"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadarSample {
    Echo(f32),
    NoEcho,
    Missing,
    Outside,
}

/// CWA O-A0059-001: real dBZ on a regular TWD67 grid, southwest-first. The timestamp identifies
/// this immutable snapshot; the public URL itself always changes to the newest observation.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CwaGrid {
    pub time: DateTime<Utc>,
    pub nx: usize,
    pub ny: usize,
    west: f64,
    south: f64,
    step: f64,
    values: Vec<f32>,
    #[serde(skip, default = "cwa_wgs84")]
    wgs84: proj4rs::Proj,
    #[serde(skip, default = "cwa_twd67")]
    twd67: proj4rs::Proj,
}

fn cwa_wgs84() -> proj4rs::Proj {
    proj4rs::Proj::from_proj_string("+proj=longlat +datum=WGS84 +no_defs")
        .expect("constant WGS84 projection")
}

fn cwa_twd67() -> proj4rs::Proj {
    // Published metre translations avoid ambiguous rotation/scale units in older examples.
    // https://www.ttsb.gov.tw/media/1311/asc94.pdf
    // shortcut: regional datum approximation; use a surveyed transform for sub-100m data.
    proj4rs::Proj::from_proj_string(
        "+proj=longlat +a=6378160 +rf=298.25 +towgs84=-752,-358,-179 +no_defs",
    )
    .expect("constant TWD67 projection")
}

impl CwaGrid {
    pub fn sample_wgs84(&self, lon: f64, lat: f64) -> RadarSample {
        if !lon.is_finite() || !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
            return RadarSample::Outside;
        }
        let mut point = (lon.to_radians(), lat.to_radians(), 0.0);
        if proj4rs::transform::transform(&self.wgs84, &self.twd67, &mut point).is_err() {
            return RadarSample::Outside;
        }
        self.sample_twd67(point.0.to_degrees(), point.1.to_degrees())
    }

    fn sample_twd67(&self, lon: f64, lat: f64) -> RadarSample {
        let x = ((lon - self.west) / self.step).round();
        let y = ((lat - self.south) / self.step).round();
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.nx as f64
            || y >= self.ny as f64
        {
            return RadarSample::Outside;
        }
        match self.values[y as usize * self.nx + x as usize] {
            // CWA combines outside-coverage and quality-control removal in -999; neither
            // sentinel proves that no rain is present.
            -999.0 | -99.0 => RadarSample::Missing,
            value => RadarSample::Echo(value),
        }
    }
}

fn xml_value<'a>(xml: &'a str, name: &str) -> anyhow::Result<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let (_, tail) = xml
        .split_once(&open)
        .with_context(|| format!("CWA is missing {name}"))?;
    let (value, rest) = tail
        .split_once(&close)
        .with_context(|| format!("CWA has unclosed {name}"))?;
    ensure!(!rest.contains(&open), "CWA has duplicate {name}");
    Ok(value.trim())
}

pub fn parse_cwa_grid(bytes: &[u8]) -> anyhow::Result<CwaGrid> {
    ensure!(bytes.len() <= CWA_MAX_BYTES, "CWA grid exceeds size limit");
    let xml = std::str::from_utf8(bytes).context("CWA grid is not UTF-8")?;
    ensure!(
        !xml.contains("<!DOCTYPE") && !xml.contains("<!ENTITY"),
        "CWA XML entities are not supported"
    );
    ensure!(
        xml_value(xml, "dataid")? == "O-A0059-001",
        "unexpected CWA product"
    );
    ensure!(
        xml_value(xml, "Reflectivity")? == "dBZ",
        "unexpected CWA units"
    );
    ensure!(
        xml_value(xml, "contentDescription")?.contains("TWD67"),
        "unsupported CWA datum"
    );
    let time = DateTime::parse_from_rfc3339(xml_value(xml, "DateTime")?)?.with_timezone(&Utc);
    let nx: usize = xml_value(xml, "GridDimensionX")?.parse()?;
    let ny: usize = xml_value(xml, "GridDimensionY")?.parse()?;
    ensure!(
        (2..=2048).contains(&nx) && (2..=2048).contains(&ny) && nx * ny <= 2_000_000,
        "invalid CWA dimensions"
    );
    let west: f64 = xml_value(xml, "StartPointLongitude")?.parse()?;
    let south: f64 = xml_value(xml, "StartPointLatitude")?.parse()?;
    let step: f64 = xml_value(xml, "GridResolution")?.parse()?;
    ensure!(
        west.is_finite()
            && south.is_finite()
            && step.is_finite()
            && (100.0..=130.0).contains(&west)
            && (0.0..=40.0).contains(&south)
            && (0.001..=0.1).contains(&step)
            && west + (nx - 1) as f64 * step <= 140.0
            && south + (ny - 1) as f64 * step <= 50.0,
        "invalid CWA grid bounds"
    );
    let mut values = Vec::with_capacity(nx * ny);
    for value in xml_value(xml, "content")?.split(',') {
        ensure!(values.len() < nx * ny, "too many CWA values");
        let value: f32 = value.trim().parse().context("invalid CWA reflectivity")?;
        ensure!(
            value == -99.0 || value == -999.0 || (-40.0..=100.0).contains(&value),
            "CWA reflectivity out of range"
        );
        values.push(value);
    }
    ensure!(values.len() == nx * ny, "incomplete CWA grid");
    Ok(CwaGrid {
        time,
        nx,
        ny,
        west,
        south,
        step,
        values,
        wgs84: cwa_wgs84(),
        twd67: cwa_twd67(),
    })
}

/// Keep large text-grid parsing off the map thread using the existing radar decode worker.
pub async fn decode_cwa_grid(bytes: std::sync::Arc<Vec<u8>>) -> anyhow::Result<CwaGrid> {
    #[cfg(target_arch = "wasm32")]
    match crate::wasm_worker::decode_cwa(bytes.as_ref().clone()).await {
        Ok(wire) => return postcard::from_bytes(&wire).map_err(Into::into),
        Err(crate::wasm_worker::Error::Unavailable) => {}
        Err(error) => anyhow::bail!("{error}"),
    }
    crate::task::blocking(move || parse_cwa_grid(&bytes)).await?
}

pub async fn decode_kma_grid(
    bytes: std::sync::Arc<Vec<u8>>,
    time: DateTime<Utc>,
) -> anyhow::Result<KmaGrid> {
    #[cfg(target_arch = "wasm32")]
    match crate::wasm_worker::decode_kma(postcard::to_allocvec(&(time, bytes.as_slice()))?).await {
        Ok(wire) => return postcard::from_bytes(&wire).map_err(Into::into),
        Err(crate::wasm_worker::Error::Unavailable) => {}
        Err(error) => anyhow::bail!("{error}"),
    }
    crate::task::blocking(move || parse_kma_grid(&bytes, time)).await?
}

#[doc(hidden)]
#[cfg(any(target_arch = "wasm32", test))]
pub fn decode_cwa_worker(payload: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    postcard::to_allocvec(&parse_cwa_grid(&payload)?).map_err(Into::into)
}

#[doc(hidden)]
#[cfg(any(target_arch = "wasm32", test))]
pub fn decode_kma_worker(payload: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let (time, bytes): (DateTime<Utc>, &[u8]) = postcard::from_bytes(&payload)?;
    postcard::to_allocvec(&parse_kma_grid(bytes, time)?).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwa_fixture(values: &str) -> String {
        format!(
            r#"<cwaopendata><dataid>O-A0059-001</dataid><Reflectivity>dBZ</Reflectivity>
        <DateTime>2026-11-05T07:40:00+08:00</DateTime><GridDimensionX>2</GridDimensionX>
        <GridDimensionY>2</GridDimensionY><StartPointLongitude>121</StartPointLongitude>
        <StartPointLatitude>24</StartPointLatitude><GridResolution>0.0125</GridResolution>
        <contentDescription>Coordinate system TWD67</contentDescription><content>{values}</content></cwaopendata>"#
        )
    }

    #[test]
    fn jma_observations_exclude_forecast_wrong_product_and_malformed_times() {
        let frames = parse_jma_frames(
            br#"[
            {"basetime":"20261104234000","validtime":"20261104234000","elements":["hrpns"]},
            {"basetime":"20261104234000","validtime":"20261104234500","elements":["hrpns"]},
            {"basetime":"20261104233500","validtime":"20261104233500","elements":["hrpns"]},
            {"basetime":"20261104234000","validtime":"20261104234000","elements":["hrpns"]},
            {"basetime":"20261104233000","validtime":"20261104233000","elements":["thns"]},
            {"basetime":"../../bad","validtime":"../../bad","elements":["hrpns"]}
        ]"#,
        )
        .unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[0] < frames[1]);
        assert!(jma_tile_url(frames[1], 10, 909, 403)
            .unwrap()
            .contains("/20261104234000/none/20261104234000/"));
        assert!(jma_tile_url(frames[0], 11, 0, 0).is_none());
        assert!(jma_tile_url(frames[0], 4, 16, 0).is_none());
    }

    #[test]
    fn cwa_preserves_timestamp_orientation_and_missing_data() {
        let grid = parse_cwa_grid(cwa_fixture("10,20,-99,-999").as_bytes()).unwrap();
        assert_eq!(grid.time.to_rfc3339(), "2026-11-04T23:40:00+00:00");
        assert_eq!(grid.sample_twd67(121.0, 24.0), RadarSample::Echo(10.0));
        assert_eq!(grid.sample_twd67(121.0125, 24.0), RadarSample::Echo(20.0));
        assert_eq!(grid.sample_twd67(121.0, 24.0125), RadarSample::Missing);
        assert_eq!(grid.sample_twd67(121.0125, 24.0125), RadarSample::Missing);
        let mut point = (121.0_f64.to_radians(), 24.0_f64.to_radians(), 0.0);
        proj4rs::transform::transform(&grid.twd67, &grid.wgs84, &mut point).unwrap();
        assert!((point.0.to_degrees() - 121.0).abs() > 0.005);
        assert_eq!(
            grid.sample_wgs84(point.0.to_degrees(), point.1.to_degrees()),
            RadarSample::Echo(10.0)
        );
        assert_eq!(grid.sample_wgs84(f64::NAN, 24.0), RadarSample::Outside);
    }

    #[test]
    fn cwa_rejects_truncated_oversized_nonfinite_or_changed_grids() {
        for values in ["1,2,3", "1,2,3,4,5", "1,2,3,NaN", "1,2,3,Inf", "1,2,3,-200"] {
            assert!(parse_cwa_grid(cwa_fixture(values).as_bytes()).is_err());
        }
        assert!(parse_cwa_grid(
            cwa_fixture("1,2,3,4")
                .replace("TWD67", "unknown")
                .as_bytes()
        )
        .is_err());
        assert!(parse_cwa_grid(
            cwa_fixture("1,2,3,4")
                .replace("<GridDimensionX>2", "<GridDimensionX>999999999999")
                .as_bytes()
        )
        .is_err());
        assert!(parse_cwa_grid(cwa_fixture("1,2,3,4").replace("dBZ", "mm/h").as_bytes()).is_err());
    }

    #[test]
    fn cwa_datum_matches_published_taichung_city_hall_landmark() {
        // Taichung GIS publishes TWD67 TM2 (213291, 2673189) and WGS84
        // (120.646910, 24.161768) for City Hall. This catches an omitted datum translation.
        // https://giscloud.taichung.gov.tw/ (map coordinate systems documentation).
        let grid = parse_cwa_grid(cwa_fixture("1,2,3,4").as_bytes()).unwrap();
        let tm2 = proj4rs::Proj::from_proj_string("+proj=tmerc +lat_0=0 +lon_0=121 +k=0.9999 +x_0=250000 +y_0=0 +a=6378160 +rf=298.25 +towgs84=-752,-358,-179 +units=m +no_defs").unwrap();
        let mut point = (213291.0, 2673189.0, 0.0);
        proj4rs::transform::transform(&tm2, &grid.wgs84, &mut point).unwrap();
        assert!((point.0.to_degrees() - 120.646910).abs() < 0.0005);
        assert!((point.1.to_degrees() - 24.161768).abs() < 0.0005);
    }

    #[test]
    fn cwa_worker_preserves_time_values_and_datum() {
        let fixture = cwa_fixture("12,-99,-999,45");
        let native = parse_cwa_grid(fixture.as_bytes()).unwrap();
        let wire = decode_cwa_worker(fixture.into_bytes()).unwrap();
        let worker: CwaGrid = postcard::from_bytes(&wire).unwrap();
        assert_eq!(worker.time, native.time);
        assert_eq!(worker.values, native.values);
        for (lon, lat) in [
            (120.64691, 24.161768),
            (native.west, native.south),
            (0.0, 0.0),
        ] {
            assert_eq!(worker.sample_wgs84(lon, lat), native.sample_wgs84(lon, lat));
        }
    }

    #[test]
    fn kma_catalog_converts_advertised_korean_times_and_ignores_other_products() {
        let times = parse_kma_frames(b"# files\nRDR_CMP_HSR_PUB_202611050840.bin.gz 100\nRDR_CMP_CPP_PUB_202611050845.bin.gz 200\nhttps://example.test/RDR_CMP_HSR_EXT_202611050835.bin.gz\nRDR_CMP_HSR_PUB_bad.bin.gz\nRDR_CMP_HSR_PUB_202611050840.bin.gz").unwrap();
        assert_eq!(times.len(), 2);
        assert_eq!(times[0].to_rfc3339(), "2026-11-04T23:35:00+00:00");
        assert_eq!(kma_time_parameter(times[1]), "202611050840");
        assert_eq!(kma_sample(3215), RadarSample::Echo(32.15));
        assert_eq!(kma_sample(-30000), RadarSample::Outside);
        assert_eq!(kma_sample(-25000), RadarSample::Missing);
        assert_eq!(kma_sample(-20000), RadarSample::NoEcho);
    }

    #[test]
    fn kma_rejects_error_pages_truncated_and_unsupported_grids() {
        let time = DateTime::from_timestamp(1793835600, 0).unwrap();
        for bytes in [
            b"<html>Invalid API key</html>".as_slice(),
            b"2305,2881,=\n1,2,3,=",
            b"9999999,9999999,=\n",
        ] {
            assert!(parse_kma_grid(bytes, time).is_err());
        }
    }

    #[test]
    fn kma_reference_cell_matches_the_official_hb_origin() {
        let mut data = String::from("# KMA HSR\n2305,2881,=\n");
        for index in 0..2305 * 2881 {
            data.push_str(if index == 1680 * 2305 + 1120 {
                "1234,"
            } else {
                "-30000,"
            });
        }
        data.push('=');
        let time = DateTime::from_timestamp(1793835600, 0).unwrap();
        let grid = parse_kma_grid(data.as_bytes(), time).unwrap();
        assert_eq!(grid.sample_wgs84(126.0, 38.0), RadarSample::Echo(12.34));
        assert_eq!(grid.sample_wgs84(-97.0, 33.0), RadarSample::Outside);
        let payload = postcard::to_allocvec(&(time, data.as_bytes())).unwrap();
        let wire = decode_kma_worker(payload).unwrap();
        let worker: KmaGrid = postcard::from_bytes(&wire).unwrap();
        assert_eq!(worker.time, time);
        assert_eq!(worker.sample_wgs84(126.0, 38.0), RadarSample::Echo(12.34));
        assert_eq!(worker.sample_wgs84(-97.0, 33.0), RadarSample::Outside);
    }
}
